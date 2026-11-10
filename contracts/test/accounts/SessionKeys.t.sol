// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {Counter} from "./utils/Mocks.sol";
import {IAccountEvents} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {
    Call,
    InitParams,
    SessionKeyConfig,
    SessionPermission,
    SessionTokenCap
} from "../../src/accounts/libraries/AccountTypes.sol";

contract SessionKeysTest is AccountTestBase {
    IXAccount internal acct; // vault disabled
    IXAccount internal vaultAcct; // vault enabled
    address internal sessionKey;
    uint256 internal sessionPk;
    address internal shop = makeAddr("shop");

    function setUp() public override {
        super.setUp();
        (sessionKey, sessionPk) = makeAddrAndKey("session");
        acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(1)));
        vaultAcct = _createAccount(_params(_one(_ecdsaSigner(owner)), true), bytes32(uint256(2)));
        usd.mint(address(acct), 1_000e6);
        usd.mint(address(vaultAcct), 1_000e6);
        vm.startPrank(owner);
        acct.addSessionKey(_cfg());
        vaultAcct.addSessionKey(_cfg());
        vm.stopPrank();
    }

    /// @dev counter.increment + counter.pay (any amount up to 1 ether total) + usd.transfer (cap 100 USD), 1 day.
    function _cfg() internal view returns (SessionKeyConfig memory cfg) {
        cfg.key = sessionKey;
        cfg.validAfter = uint48(block.timestamp);
        cfg.validUntil = uint48(block.timestamp + 1 days);
        cfg.nativeCap = 1 ether;
        cfg.permissions = new SessionPermission[](4);
        cfg.permissions[0] = SessionPermission(address(counter), counter.increment.selector);
        cfg.permissions[1] = SessionPermission(address(counter), counter.pay.selector);
        cfg.permissions[2] = SessionPermission(address(usd), usd.transfer.selector);
        cfg.permissions[3] = SessionPermission(shop, bytes4(0)); // plain value transfers to the shop
        cfg.tokenCaps = new SessionTokenCap[](1);
        cfg.tokenCaps[0] = SessionTokenCap(address(usd), 100e6);
    }

    function _session(IXAccount a, Call memory c) internal {
        vm.prank(sessionKey);
        a.executeWithSession(_calls1(c));
    }

    function test_session_allowedCalls() public {
        _session(acct, _call(address(counter), 0, abi.encodeCall(counter.increment, ())));
        _session(acct, _call(address(counter), 0.4 ether, abi.encodeCall(counter.pay, ())));
        _session(acct, _call(shop, 0.5 ether, ""));
        _session(acct, _call(address(usd), 0, abi.encodeCall(usd.transfer, (shop, 60e6))));
        assertEq(counter.count(), 1);
        assertEq(shop.balance, 0.5 ether);
        assertEq(usd.balanceOf(shop), 60e6);
        (,,,,, uint128 nativeSpent) = acct.getSession(sessionKey);
        assertEq(nativeSpent, 0.9 ether);
    }

    function test_session_disallowedTargetOrSelector() public {
        vm.startPrank(sessionKey);
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccountEvents.SessionCallNotAllowed.selector, address(counter), counter.other.selector
            )
        );
        acct.executeWithSession(_calls1(_call(address(counter), 0, abi.encodeCall(counter.other, ()))));
        vm.expectRevert(
            abi.encodeWithSelector(IAccountEvents.SessionCallNotAllowed.selector, attacker, bytes4(0))
        );
        acct.executeWithSession(_calls1(_call(attacker, 1, "")));
        // management of the account itself is never allowed
        bytes memory addSigner = abi.encodeCall(acct.addSigner, (_ecdsaSigner(sessionKey)));
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccountEvents.SessionCallNotAllowed.selector, address(acct), acct.addSigner.selector
            )
        );
        acct.executeWithSession(_calls1(_call(address(acct), 0, addSigner)));
        // owner-only entry points stay closed
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(shop, 1, "");
        vm.stopPrank();
    }

    function test_session_nativeCap() public {
        _session(acct, _call(shop, 0.7 ether, ""));
        vm.prank(sessionKey);
        vm.expectRevert(IAccountEvents.SessionNativeCapExceeded.selector);
        acct.executeWithSession(_calls1(_call(shop, 0.31 ether, "")));
        _session(acct, _call(shop, 0.3 ether, ""));
        assertEq(shop.balance, 1 ether);
    }

    function test_session_tokenCap_andApprovalsCount() public {
        _session(acct, _call(address(usd), 0, abi.encodeCall(usd.transfer, (shop, 100e6))));
        vm.prank(sessionKey);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.SessionTokenCapExceeded.selector, address(usd)));
        acct.executeWithSession(_calls1(_call(address(usd), 0, abi.encodeCall(usd.transfer, (shop, 1)))));
    }

    function test_session_expiry_and_notYetValid() public {
        vm.warp(block.timestamp + 1 days + 1);
        vm.prank(sessionKey);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.SessionNotActive.selector, sessionKey));
        acct.executeWithSession(_calls1(_call(shop, 1, "")));

        SessionKeyConfig memory cfg = _cfg();
        cfg.validAfter = uint48(block.timestamp + 1 hours);
        cfg.validUntil = uint48(block.timestamp + 2 hours);
        vm.prank(owner);
        acct.addSessionKey(cfg);
        vm.prank(sessionKey);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.SessionNotActive.selector, sessionKey));
        acct.executeWithSession(_calls1(_call(shop, 1, "")));
        vm.warp(block.timestamp + 1 hours);
        _session(acct, _call(shop, 1, ""));
    }

    function test_session_revoke() public {
        vm.prank(owner);
        acct.revokeSessionKey(sessionKey);
        vm.prank(sessionKey);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.SessionNotActive.selector, sessionKey));
        acct.executeWithSession(_calls1(_call(shop, 1, "")));
    }

    function test_session_reAddResetsPermissionsAndSpend() public {
        _session(acct, _call(shop, 1 ether, ""));
        SessionKeyConfig memory cfg = _cfg();
        cfg.permissions = new SessionPermission[](1);
        cfg.permissions[0] = SessionPermission(address(counter), counter.increment.selector);
        vm.prank(owner);
        acct.addSessionKey(cfg);
        // old permission (shop) is gone, spend counter reset
        vm.prank(sessionKey);
        vm.expectRevert(
            abi.encodeWithSelector(IAccountEvents.SessionCallNotAllowed.selector, shop, bytes4(0))
        );
        acct.executeWithSession(_calls1(_call(shop, 1, "")));
        assertFalse(acct.isSessionPermitted(sessionKey, shop, bytes4(0)));
    }

    function test_session_cannotQueueVaultActions() public {
        // vault: 1 ether per tx; a session can never create queued actions
        vm.prank(sessionKey);
        vm.expectRevert(IAccountEvents.SessionActionNeedsDelay.selector);
        vaultAcct.executeWithSession(
            _calls1(_call(address(usd), 0, abi.encodeCall(usd.transfer, (shop, 1e6))))
        );
        // instant-eligible native spend works and counts against the vault's daily limit
        _session(vaultAcct, _call(shop, 0.5 ether, ""));
        assertEq(vaultAcct.spentToday(), 0.5 ether);
        assertEq(vaultAcct.lastActionId(), 0);
    }

    function test_session_viaUserOp() public {
        Call[] memory calls = new Call[](2);
        calls[0] = _call(address(counter), 0, abi.encodeCall(counter.increment, ()));
        calls[1] = _call(shop, 0.2 ether, "");
        PackedUserOperation memory op = _userOp(address(acct), _executeUserOpData(calls));
        op.signature = abi.encodePacked(uint8(3), sessionKey, _ecdsaSign(sessionPk, ep.getUserOpHash(op)));
        _handle(op);
        assertEq(counter.count(), 1);
        assertEq(shop.balance, 0.2 ether);
    }

    function test_session_userOp_cannotUseOwnerEntryPoints() public {
        // session signature on `execute(...)` callData -> rejected at validation
        PackedUserOperation memory op = _userOp(address(acct), _execData(shop, 0.2 ether, ""));
        op.signature = abi.encodePacked(uint8(3), sessionKey, _ecdsaSign(sessionPk, ep.getUserOpHash(op)));
        _expectFailedOp(op, "AA24 signature error");
        // non-permitted call inside executeUserOp -> rejected at validation
        PackedUserOperation memory op2 =
            _userOp(address(acct), _executeUserOpData(_calls1(_call(attacker, 1, ""))));
        op2.signature = abi.encodePacked(uint8(3), sessionKey, _ecdsaSign(sessionPk, ep.getUserOpHash(op2)));
        _expectFailedOp(op2, "AA24 signature error");
        // signature by a different key than the one claimed
        PackedUserOperation memory op3 =
            _userOp(address(acct), _executeUserOpData(_calls1(_call(shop, 1, ""))));
        op3.signature = abi.encodePacked(uint8(3), sessionKey, _ecdsaSign(attackerPk, ep.getUserOpHash(op3)));
        _expectFailedOp(op3, "AA24 signature error");
    }

    function test_session_userOp_expired() public {
        vm.warp(block.timestamp + 2 days);
        PackedUserOperation memory op =
            _userOp(address(acct), _executeUserOpData(_calls1(_call(shop, 1, ""))));
        op.signature = abi.encodePacked(uint8(3), sessionKey, _ecdsaSign(sessionPk, ep.getUserOpHash(op)));
        _expectFailedOp(op, "AA22 expired or not due");
    }

    function _expectFailedOp(PackedUserOperation memory op, string memory reason) internal {
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, reason));
        ep.handleOps(ops, payable(bundler));
    }

    /// @dev Fuzz: a sequence of session payments never exceeds the native cap.
    function testFuzz_session_neverExceedsNativeCap(uint96[8] memory amounts) public {
        uint256 paid;
        for (uint256 i; i < amounts.length; ++i) {
            uint256 amt = uint256(amounts[i]) % 0.6 ether;
            vm.prank(sessionKey);
            try acct.executeWithSession(_calls1(_call(shop, amt, ""))) {
                paid += amt;
            } catch {}
            assertLe(shop.balance, 1 ether);
        }
        assertEq(shop.balance, paid);
    }
}
