// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {CanonicalTokenFactory} from "../../src/periphery/token/CanonicalTokenFactory.sol";
import {XDRC20Bridged} from "../../src/periphery/token/XDRC20Bridged.sol";
import {XDRC20} from "../../src/periphery/token/XDRC20.sol";
import {IXDRC20} from "../../src/periphery/token/IXDRC20.sol";

contract CanonicalTokenFactoryTest is Test {
    CanonicalTokenFactory internal factory;
    address internal gov = makeAddr("gov");
    address internal bridge = makeAddr("bridge");
    address internal usdcL1 = makeAddr("usdcL1");
    address internal alice = makeAddr("alice");

    function setUp() public {
        vm.warp(1_800_000_000);
        factory = new CanonicalTokenFactory(gov, 30 days, 365 days);
    }

    function _create() internal returns (XDRC20Bridged t) {
        vm.prank(gov);
        t = XDRC20Bridged(factory.createCanonicalToken(1, usdcL1, "USD Coin (bridged)", "USDC", 6, bridge));
    }

    function test_createAndVerify() public {
        XDRC20Bridged t = _create();
        assertTrue(factory.isCanonical(address(t)));
        assertEq(factory.canonicalToken(1, usdcL1), address(t));
        (uint256 cid, address ot) = factory.originOf(address(t));
        assertEq(cid, 1);
        assertEq(ot, usdcL1);
        assertEq(factory.allTokensLength(), 1);
        assertEq(t.decimals(), 6);
        assertEq(t.symbol(), "USDC");
        assertEq(t.bridge(), bridge);
        assertEq(t.totalSupply(), 0);
        assertTrue(t.supportsInterface(type(IXDRC20).interfaceId));
    }

    function test_lookalikeIsNotCanonical() public {
        _create();
        // Anyone can deploy a token called "USDC"; it is simply not canonical.
        XDRC20 fake = new XDRC20("USD Coin (bridged)", "USDC", alice, 1e12, 30 days, 365 days);
        assertFalse(factory.isCanonical(address(fake)));
    }

    function test_onlyOnePerOrigin() public {
        XDRC20Bridged t = _create();
        vm.prank(gov);
        vm.expectRevert(abi.encodeWithSelector(CanonicalTokenFactory.AlreadyExists.selector, address(t)));
        factory.createCanonicalToken(1, usdcL1, "Other", "USDC2", 6, bridge);
    }

    function test_onlyGovernance() public {
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(CanonicalTokenFactory.NotGovernance.selector, alice));
        factory.createCanonicalToken(1, usdcL1, "x", "x", 6, bridge);
        vm.prank(gov);
        factory.setGovernance(alice);
        assertEq(factory.governance(), alice);
    }

    function test_bridgeMintBurnAndExpiringApprovals() public {
        XDRC20Bridged t = _create();
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XDRC20Bridged.OnlyBridge.selector, alice));
        t.mint(alice, 1);

        vm.prank(bridge);
        t.mint(alice, 100e6);
        assertEq(t.balanceOf(alice), 100e6);

        vm.prank(alice);
        t.approve(address(this), 10e6);
        vm.warp(block.timestamp + 30 days);
        assertEq(t.allowance(alice, address(this)), 0);

        vm.prank(bridge);
        t.burn(alice, 40e6);
        assertEq(t.totalSupply(), 60e6);
    }
}
