// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Deployers} from "./utils/Deployers.sol";
import {MySwapper} from "../src/MySwapper.sol";
import {IHooks} from "../src/interfaces/IHooks.sol";
import {IERC20Minimal} from "../src/interfaces/external/IERC20Minimal.sol";
import {Currency} from "../src/types/Currency.sol";
import {BalanceDelta} from "../src/types/BalanceDelta.sol";
import {SwapParams} from "../src/types/PoolOperation.sol";
import {Vm} from "forge-std/Vm.sol";

contract MySwapperTest is Deployers {
    MySwapper internal swapper;

    function setUp() public {
        vm.deal(address(this), 100 ether);
        initializeManagerRoutersAndPoolsWithLiq(IHooks(address(0)));
        swapper = new MySwapper(manager);
        IERC20Minimal(Currency.unwrap(currency0)).approve(address(swapper), type(uint256).max);
        IERC20Minimal(Currency.unwrap(currency1)).approve(address(swapper), type(uint256).max);
    }

    function testNativeInputAndRefund() public {
        vm.deal(address(swapper), 1 ether);
        uint256 ethBefore = address(this).balance;
        uint256 tokenBefore = currency1.balanceOf(address(this));
        vm.recordLogs();
        BalanceDelta delta = swapper.swap{value: 0.1 ether}(nativeKey, SwapParams(true, -1e12, MIN_PRICE_LIMIT), "");
        assertReceiptDelta(delta);
        assertLt(delta.amount0(), 0);
        assertGt(delta.amount1(), 0);
        assertEq(ethBefore - address(this).balance, uint256(-int256(delta.amount0())));
        assertEq(currency1.balanceOf(address(this)) - tokenBefore, uint128(delta.amount1()));
        assertEq(address(swapper).balance, 1 ether);
    }

    function testErc20InputAndNativeOutput() public {
        uint256 ethBefore = address(this).balance;
        uint256 tokenBefore = currency1.balanceOf(address(this));
        vm.recordLogs();
        BalanceDelta delta = swapper.swap(nativeKey, SwapParams(false, -1e12, MAX_PRICE_LIMIT), "");
        assertReceiptDelta(delta);
        assertGt(delta.amount0(), 0);
        assertLt(delta.amount1(), 0);
        assertEq(address(this).balance - ethBefore, uint128(delta.amount0()));
        assertEq(tokenBefore - currency1.balanceOf(address(this)), uint256(-int256(delta.amount1())));
    }

    function testRejectUnauthorizedCallback() public {
        vm.expectRevert("Only manager");
        swapper.unlockCallback("");
    }

    function assertReceiptDelta(BalanceDelta delta) internal {
        Vm.Log[] memory logs = vm.getRecordedLogs();
        Vm.Log memory execution = logs[logs.length - 1];
        assertEq(execution.emitter, address(swapper));
        assertEq(execution.topics[0], keccak256("SwapExecuted(address,int256)"));
        assertEq(execution.topics[1], bytes32(uint256(uint160(address(this)))));
        assertEq(abi.decode(execution.data, (int256)), BalanceDelta.unwrap(delta));
    }

    function testCannotSpendExistingEthBalance() public {
        vm.deal(address(swapper), 1 ether);
        vm.expectRevert("Insufficient ETH input");
        swapper.swap(nativeKey, SwapParams(true, -1e12, MIN_PRICE_LIMIT), "");
        assertEq(address(swapper).balance, 1 ether);
    }
}
