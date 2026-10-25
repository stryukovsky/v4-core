// SPDX-License-Identifier: MIT

pragma solidity ^0.8.24;

import {IPoolManager} from "./interfaces/IPoolManager.sol";
import {IUnlockCallback} from "./interfaces/callback/IUnlockCallback.sol";
import {Currency, CurrencyLibrary} from "./types/Currency.sol";
import {BalanceDelta} from "./types/BalanceDelta.sol";
import {ERC20} from "solmate/src/tokens/ERC20.sol";
import {SafeTransferLib} from "solmate/src/utils/SafeTransferLib.sol";
import {PoolKey} from "./types/PoolKey.sol";
import {SwapParams} from "./types/PoolOperation.sol";

contract MySwapper is IUnlockCallback {
    using SafeTransferLib for ERC20;

    IPoolManager public immutable poolManager;

    struct CallbackData {
        address sender;
        PoolKey key;
        SwapParams params;
        bytes hookData;
        uint256 nativeBudget;
    }

    constructor(IPoolManager manager) {
        poolManager = manager;
    }

    /// @notice ERC20 input requires allowance to this contract; native input is supplied as msg.value.
    function swap(PoolKey memory key, SwapParams memory params, bytes calldata hookData)
        external
        payable
        returns (BalanceDelta delta)
    {
        uint256 nativeSpent;
        (delta, nativeSpent) = abi.decode(
            poolManager.unlock(abi.encode(CallbackData(msg.sender, key, params, hookData, msg.value))),
            (BalanceDelta, uint256)
        );
        // Refund only this call's unused value, rather than the contract's entire balance.
        if (msg.value > nativeSpent) CurrencyLibrary.ADDRESS_ZERO.transfer(msg.sender, msg.value - nativeSpent);
    }

    function unlockCallback(bytes calldata rawData) external override returns (bytes memory) {
        require(msg.sender == address(poolManager), "Only manager");
        CallbackData memory data = abi.decode(rawData, (CallbackData));
        BalanceDelta delta = poolManager.swap(data.key, data.params, data.hookData);

        uint256 nativeSpent = _settle(data.key.currency0, data.sender, delta.amount0(), data.nativeBudget);
        nativeSpent += _settle(data.key.currency1, data.sender, delta.amount1(), data.nativeBudget - nativeSpent);

        // Each delta is this swap's balance change for MySwapper: negative means we owe
        // the manager input; positive means the manager owes us output. swap() records
        // these amounts but does not transfer the tokens. The calls above pay the input;
        // take() sends the output directly to the user and clears our corresponding credit.
        // For a normal zeroForOne swap, amount0 is negative and amount1 positive; swapping
        // in the other direction reverses those signs. We check each sign, not which token
        // has a larger amount: the currencies have different units, and hooks may adjust
        // the returned amounts. These checks withdraw credits; they do not enforce slippage.
        // All outstanding deltas must be zero before the unlock callback finishes.
        if (delta.amount0() > 0) poolManager.take(data.key.currency0, data.sender, uint128(delta.amount0()));
        if (delta.amount1() > 0) poolManager.take(data.key.currency1, data.sender, uint128(delta.amount1()));

        return abi.encode(delta, nativeSpent);
    }

    /// @dev Pay this contract's debt for one currency. Nonnegative deltas require no payment.
    /// Settling credits the payment against our negative delta in the manager's accounting.
    /// @return nativeSpent ETH consumed from this swap's budget; zero for ERC20 payments.
    function _settle(Currency currency, address payer, int128 delta, uint256 nativeBudget)
        private
        returns (uint256 nativeSpent)
    {
        if (delta >= 0) return 0;
        // Widen before negating so even the minimum int128 value can be converted safely.
        uint256 amount = uint256(-int256(delta));
        // For ERC20s, checkpoint the manager's balance before sending tokens so settle()
        // can measure the received amount. For ETH, sync resets any ERC20 checkpoint;
        // settle() instead measures the payment using msg.value.
        poolManager.sync(currency);
        if (currency.isAddressZero()) {
            // Only spend ETH supplied for this call; report usage so swap() can refund the rest.
            require(amount <= nativeBudget, "Insufficient ETH input");
            poolManager.settle{value: amount}();
            return amount;
        }

        // The user approved MySwapper, so pull their tokens directly into the manager.
        // The transfer moves tokens; settle() records the payment against MySwapper's debt.
        ERC20(Currency.unwrap(currency)).safeTransferFrom(payer, address(poolManager), amount);
        poolManager.settle();
    }
}

