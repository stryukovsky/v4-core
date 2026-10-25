# How Uniswap v4 swaps tokens

This explanation follows this repository's implementation, especially [`MySwapper.sol`](../src/MySwapper.sol) and the [swap scenario walkthrough](../.scenarios_of_codebase/swap-of-tokens.txt). It describes the checked-in code rather than a particular deployed router.

The central idea: **a swap first changes pool state and records debts and credits; the callback then pays those debts and withdraws those credits.** Everything must balance before the manager finishes the unlock operation, or the transaction reverts.

## 1. The contracts and their roles

| Participant | Role |
| --- | --- |
| User | Chooses a pool and swap parameters, supplies ETH or approves ERC20 spending, and receives output. |
| `MySwapper` | Connects the user to the manager, implements the callback, pays input, and sends output to the user. |
| `PoolManager` | Holds state for all pools and custody of their underlying currencies; tracks temporary obligations during an unlock. |
| `Pool` / `SwapMath` libraries | Calculate price movement, token amounts, liquidity transitions, and fees. |
| Hook contract | Optionally customizes behavior before and after a swap, according to its permissions. |

V4 uses a singleton manager: individual pools are entries in `PoolManager._pools`, rather than separate contracts holding each pair's assets. Accounting keeps pools distinct even though underlying custody is shared.

## 2. Identify a pool and prepare the trade

[`PoolKey`](../src/types/PoolKey.sol) contains five fields:

```solidity
Currency currency0;
Currency currency1;
uint24 fee;
int24 tickSpacing;
IHooks hooks;
```

Currencies are ordered numerically by address: `currency0 < currency1`. Address zero represents native ETH. The complete key identifies the pool; the same pair can have multiple pools with different fees, tick spacing, or hooks. A dynamic-fee key uses the dynamic-fee flag rather than a fixed current fee.

Before trading, the pool must be initialized with a starting price, and liquidity providers must add and settle liquidity. Initialization alone does not supply assets or useful trading liquidity.

The user supplies [`SwapParams`](../src/types/PoolOperation.sol):

| Parameter | Meaning |
| --- | --- |
| `zeroForOne = true` | Sell currency0, receive currency1. The pool price moves down. |
| `zeroForOne = false` | Sell currency1, receive currency0. The pool price moves up. |
| `amountSpecified < 0` | Exact input: the magnitude is the requested input budget, including swap fees. |
| `amountSpecified > 0` | Exact output: the value is the requested output amount. |
| `sqrtPriceLimitX96` | The price boundary at which execution stops. |

Amounts use each token's smallest units. For a token with six decimals, `1_000_000` means one token. `amountSpecified == 0` is rejected by the manager.

The encoded price is:

```text
sqrtPriceX96 = sqrt(P_raw) × 2^96
P_raw = currency1 raw units per currency0 raw unit
P_human = P_raw × 10^(decimals0 - decimals1)
```

For currency0 → currency1, the limit must be strictly below the current price and strictly above `MIN_SQRT_PRICE`. For the reverse direction, it must be strictly above the current price and strictly below `MAX_SQRT_PRICE`.

**A price boundary is not a minimum-output or maximum-input check.** Execution can stop at that boundary before filling the requested amount, including an exact-output request.

## 3. The complete `MySwapper` call sequence

```mermaid
sequenceDiagram
    participant U as User
    participant R as MySwapper
    participant M as PoolManager
    participant T as Input ERC20
    U->>R: swap(key, params, hookData), optional ETH
    R->>M: unlock(encoded callback data)
    M->>R: unlockCallback(data)
    R->>M: swap(key, params, hookData)
    Note over M: Run hooks and swap math; record deltas
    M-->>R: Final BalanceDelta
    R->>M: sync(input currency)
    alt ERC20 input
        R->>T: transferFrom(user, manager, debt)
        R->>M: settle()
    else Native ETH input
        R->>M: settle{value: debt}()
    end
    R->>M: take(output currency, user, credit)
    R-->>M: Encoded delta and native spending
    Note over M: Require every outstanding delta to be zero; lock
    M-->>R: Callback result
    R-->>U: Return delta; refund unused supplied ETH
```

The diagram shows a normal swap with one input and one output. The implementation checks both currencies' signs independently.

### Entering the callback

The user calls `MySwapper.swap()`. For ERC20 input, the user must have a sufficient balance and approve **MySwapper** as spender. For native input, the user supplies `msg.value`.

The swapper encodes the original caller, pool key, parameters, hook data, and this call's ETH budget. It passes these to `poolManager.unlock()`.

The manager marks itself unlocked and calls `unlockCallback()` on its immediate caller: `MySwapper`. The callback verifies that its caller is the configured manager before decoding the data.

When `MySwapper` calls `PoolManager.swap()`, the manager sees **MySwapper as `msg.sender`**. Its accounting obligations belong to the swapper. The original user is retained in callback data as payer and recipient.

Calling the manager's `swap()` while locked fails with `ManagerLocked`. Starting another `unlock()` while already unlocked fails with `AlreadyUnlocked`; this does not prohibit all nested manager operations during a callback.

## 4. What the manager computes

The manager selects the pool using the full key, verifies initialization, and runs permitted `beforeSwap` behavior. It then calls the core [`Pool.swap()`](../src/libraries/Pool.sol) calculation.

V4 uses concentrated liquidity. Liquidity providers choose tick ranges, and only positions active at the current price contribute to the current liquidity. A tick represents a discrete price level; tick spacing restricts where position boundaries can be placed.

The core calculation proceeds in steps:

1. Load current square-root price, tick, active liquidity, and fees.
2. Search the tick bitmap for the next boundary in the trade direction. A bitmap search boundary is not necessarily an initialized liquidity boundary.
3. Choose the nearer target: that boundary or the user's price limit.
4. Use [`SwapMath.computeSwapStep()`](../src/libraries/SwapMath.sol) to calculate input, output, fees, and the new price for the current liquidity.
5. Update remaining requested amount and calculated counter-amount.
6. If an initialized tick is crossed, update fee accounting and active liquidity for the next range.
7. Continue until the requested amount is consumed or the price limit is reached.

Within a range, liquidity determines how much input moves the price and how much output can be obtained. Crossing a range boundary can change that liquidity. Empty ranges can be traversed without exchanging tokens.

Input requirements round upward and output rounds downward where required by the math. Actual integer amounts therefore include rounding effects.

The calculation persists the resulting price, tick, liquidity, and fee growth. It does not transfer the user's ERC20 input or pay their output. Hooks invoked around this calculation may perform additional calls and transfers of their own.

### Fees

Swap fees are charged in the input currency. Fee rates use millionths: `3000` means `0.3%`. When protocol fees are enabled, the total rate combines protocol and LP rates sequentially:

```text
totalFee = protocolFee + lpFee
           - floor(protocolFee × lpFee / 1_000_000)
```

The protocol portion accrues in protocol accounting. The LP portion updates fee growth for liquidity active in each step; LPs do not receive wallet transfers during the swap. Exact-output execution with a total swap fee of 100% is rejected.

## 5. Understand `BalanceDelta`

The returned [`BalanceDelta`](../src/types/BalanceDelta.sol) contains two signed `int128` values, always ordered by currency:

| Component | Negative | Positive |
| --- | --- | --- |
| `amount0()` | MySwapper owes currency0 to the manager. | The manager owes currency0 to MySwapper. |
| `amount1()` | MySwapper owes currency1 to the manager. | The manager owes currency1 to MySwapper. |

For an ordinary currency0 → currency1 trade, suppose the actual result is:

```text
amount0 = -100 raw units
amount1 =  +95 raw units
```

These are illustrative amounts, not a price quote. The swapper must pay 100 units of currency0 and may withdraw 95 units of currency1. For the reverse direction, the usual signs reverse.

The manager adds these deltas to temporary balances keyed by **account and currency**, using transient storage. This is flash accounting: operations can accumulate and offset obligations during the unlock, with actual transfers needed only for the remaining amounts. More elaborate routers can use this to net multiple swaps; `MySwapper` performs one swap and immediately settles its result.

The swapper checks signs rather than comparing the magnitudes of the two components. Different currencies have different units, and hooks may adjust the final amounts.

## 6. How input is paid and output is withdrawn

`MySwapper._settle()` ignores a nonnegative component. For a negative component, it converts its magnitude to the amount owed.

For ERC20 debt, the sequence is:

```solidity
poolManager.sync(currency);
ERC20(Currency.unwrap(currency)).safeTransferFrom(payer, address(poolManager), amount);
poolManager.settle();
```

`sync()` snapshots the manager's token balance **before** the transfer. `transferFrom()` moves tokens directly from the user to the manager. `settle()` measures the balance increase and credits it against MySwapper's debt.

The transfer and settlement are distinct: transferring tokens alone does not clear the debt. If the received amount is insufficient, the remaining obligation prevents unlock completion.

For native ETH debt, the swapper calls `sync(address(0))` to reset the synced currency and then `settle{value: amount}()`. The manager credits `msg.value`. MySwapper checks the amount against this call's ETH budget and reports how much it spent.

For each positive component, the callback calls:

```solidity
poolManager.take(currency, originalUser, credit);
```

`take()` reduces MySwapper's credit and transfers the actual currency directly to the user. In the example:

| Stage | Currency0 delta | Currency1 delta |
| --- | ---: | ---: |
| After swap | -100 | +95 |
| After input settlement | 0 | +95 |
| After output withdrawal | 0 | 0 |

## 7. Hooks and final accounting

Permitted hooks can run before and after the core swap. Depending on permissions and pool configuration, they can adjust the amount sent into core math, override the LP fee for a dynamic-fee pool, and return deltas that change the caller's result. A before-swap amount adjustment cannot reverse the exact-input/exact-output sign, although reducing the core amount to zero is allowed.

The returned delta includes hook adjustments. Hook deltas are accounted separately to the hook address, so settling MySwapper's obligations does not automatically clear a hook's obligations.

The manager emits its `Swap` event before `afterSwap`; event amounts describe the core swap and may differ from the final returned caller delta. The event sender is MySwapper.

When the callback returns, the manager requires **every outstanding account/currency delta to be zero**, including hook balances. Otherwise `CurrencyNotSettled` reverts the unlock and its nested effects. On success it locks again and returns the callback result.

MySwapper then refunds `msg.value - nativeSpent` to the original caller. It refunds only unused ETH supplied for this call. A failing output transfer or refund reverts the enclosing transaction.

## 8. What callers must account for in this swapper

`MySwapper` demonstrates the swap-and-settlement mechanics, but its interface does not enforce several common trading policies:

- **Minimum output:** it has no `amountOutMinimum` check for exact input.
- **Maximum input:** it has no explicit ERC20 `amountInMaximum` check for exact output. Native spending is bounded by this call's supplied ETH.
- **Full fill:** it accepts the actual result even if execution stops at the price limit before consuming all input or delivering all requested output.
- **Deadline:** it does not reject execution after a user-selected expiry time.

ERC20 approval permits spending; it does not by itself express the desired trade price or a per-trade maximum. Such protections should be enforced atomically by the integration using final caller amounts, with hook behavior taken into account.

The [test router](../src/test/PoolSwapTest.sol) also supports settling through ERC-6909 claims and receiving claims instead of underlying assets, via its [settlement helper](../test/utils/CurrencySettler.sol). MySwapper uses underlying transfers only. Claims represent a right to underlying currency held by the manager; minting claims is not an immediate underlying-token payout.

## Source map

| File | What to read it for |
| --- | --- |
| [`src/MySwapper.sol`](../src/MySwapper.sol) | User entry point, callback, input settlement, output withdrawal, and ETH refund. |
| [`.scenarios_of_codebase/swap-of-tokens.txt`](../.scenarios_of_codebase/swap-of-tokens.txt) | Ordered walkthrough of the wider swap path. |
| [`src/PoolManager.sol`](../src/PoolManager.sol) | Unlock lifecycle, hooks, delta accounting, `sync`, `settle`, and `take`. |
| [`src/libraries/Pool.sol`](../src/libraries/Pool.sol) | Swap loop, tick crossings, fee growth, and final amounts. |
| [`src/libraries/SwapMath.sol`](../src/libraries/SwapMath.sol) | Input/output and fee calculations for one price step. |
| [`src/libraries/Hooks.sol`](../src/libraries/Hooks.sol) | Hook dispatch, permissions, and returned-delta adjustments. |
