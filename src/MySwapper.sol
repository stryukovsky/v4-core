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

        if (delta.amount0() > 0) poolManager.take(data.key.currency0, data.sender, uint128(delta.amount0()));
        if (delta.amount1() > 0) poolManager.take(data.key.currency1, data.sender, uint128(delta.amount1()));

        return abi.encode(delta, nativeSpent);
    }

    function _settle(Currency currency, address payer, int128 delta, uint256 nativeBudget)
        private
        returns (uint256 nativeSpent)
    {
        if (delta >= 0) return 0;
        uint256 amount = uint256(-int256(delta));
        poolManager.sync(currency);
        if (currency.isAddressZero()) {
            require(amount <= nativeBudget, "Insufficient ETH input");
            poolManager.settle{value: amount}();
            return amount;
        }

        ERC20(Currency.unwrap(currency)).safeTransferFrom(payer, address(poolManager), amount);
        poolManager.settle();
    }
}

