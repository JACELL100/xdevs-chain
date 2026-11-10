// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {IGuardianVault} from "../../src/accounts/interfaces/IGuardianVault.sol";
import {IAccountEvents} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "../../src/accounts/libraries/AccountConstants.sol";
import {
    Call,
    InitParams,
    Signer,
    SessionKeyConfig,
    SessionPermission,
    SessionTokenCap
} from "../../src/accounts/libraries/AccountTypes.sol";

/// @notice Guardian freeze + guardian-threshold cancels: the fix for the "owner vs thief" recovery stalemate.
contract GuardianFreezeTest is AccountTestBase {
    IXAccount internal acct;
    address internal friend = makeAddr("friend");
    address internal sessionKey = makeAddr("session");

    function setUp() public override {
        super.setUp();
        acct = _vaultAccount(); // owner ECDSA, guardians 1..3 (2-of-3), vault on (1 ether / 5 ether per day, 24 h)
    }

    function _freeze() internal {
        vm.prank(guardian1);
        acct.freeze();
        vm.prank(guardian2);
        acct.freeze();
    }

    function _addSession() internal {
        SessionKeyConfig memory cfg;
        cfg.key = sessionKey;
        cfg.validUntil = uint48(block.timestamp + 365 days);
        cfg.nativeCap = 1 ether;
        cfg.permissions = new SessionPermission[](1);
        cfg.permissions[0] = SessionPermission(friend, bytes4(0));
        cfg.tokenCaps = new SessionTokenCap[](0);
        vm.prank(owner);
        acct.addSessionKey(cfg);
    }

    // ------------------------------------------------------------------------------------------------------------
    // Freeze: threshold
    // ------------------------------------------------------------------------------------------------------------

    function test_freeze_requiresGuardianThreshold() public {
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        acct.freeze();
        vm.prank(owner); // signers cannot freeze (nor unfreeze) — guardian power only
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        acct.freeze();

        vm.prank(guardian1);
        acct.freeze();
        assertFalse(acct.isFrozen(), "one guardian below threshold cannot freeze");
        (, uint8 votes) = acct.guardianVotes(0, 0);
        assertEq(votes, 1);
        vm.prank(guardian1);
        vm.expectRevert(IAccountEvents.AlreadyApproved.selector);
        acct.freeze();

        uint64 expectedUntil = uint64(block.timestamp) + AccountConstants.DEFAULT_FREEZE_DURATION;
        vm.expectEmit(address(acct));
        emit IAccountEvents.AccountFrozen(expectedUntil);
        vm.prank(guardian3);
        acct.freeze();
        assertTrue(acct.isFrozen());
        (uint64 until, uint64 duration,) = acct.freezeState();
        assertEq(until, expectedUntil);
        assertEq(duration, 30 days);

        vm.prank(guardian2);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.freeze();
    }

    // ------------------------------------------------------------------------------------------------------------
    // While frozen nothing executes
    // ------------------------------------------------------------------------------------------------------------

    function test_frozen_blocksEverySignerAndSessionAction() public {
        _addSession();
        // a matured queued action and a pre-approved ERC-1271 hash exist before the freeze
        vm.startPrank(owner);
        uint256 queued = acct.execute(friend, 2 ether, "");
        bytes32 h = keccak256("order");
        acct.approveHash(h);
        uint256 approveId = acct.lastActionId();
        vm.stopPrank();
        vm.warp(block.timestamp + 24 hours);
        acct.execute(approveId);
        assertEq(acct.isValidSignature(h, ""), IERC1271.isValidSignature.selector);

        _freeze();

        vm.startPrank(owner);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.execute(friend, 0.1 ether, ""); // would be instant (below threshold / daily limit)
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.execute(friend, 50 ether, ""); // would be queued
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.executeBatch(_calls1(_call(friend, 0.1 ether, "")));
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.addSigner(_ecdsaSigner(makeAddr("newDevice")));
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.revokeSessionKey(sessionKey);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.cancelRecovery();
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.cancel(queued);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.setWhitelisted(friend, true); // delayed function: not even queued
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.removeSigner(bytes32(uint256(uint160(owner))));
        vm.stopPrank();

        // matured queued action: not executable by anyone while frozen
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.execute(queued);
        // session key
        vm.prank(sessionKey);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.executeWithSession(_calls1(_call(friend, 0.1 ether, "")));
        // ERC-1271: even pre-approved hashes are refused
        assertEq(acct.isValidSignature(h, ""), bytes4(0xffffffff));

        // ERC-4337 owner op: validAfter = frozenUntil - 1 => the EntryPoint refuses it (no TIMESTAMP in validation)
        PackedUserOperation memory op =
            _signOpEcdsa(_userOp(address(acct), _execData(friend, 0.1 ether, "")), ownerPk);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, "AA22 expired or not due"));
        ep.handleOps(ops, payable(bundler));

        // views keep working for signers
        assertEq(acct.getSigners().length, 1);
        assertTrue(acct.vaultPolicy().enabled);
        assertEq(friend.balance, 0);

        // once the freeze has elapsed everything works again
        (uint64 until,,) = acct.freezeState();
        vm.warp(until);
        assertFalse(acct.isFrozen());
        vm.prank(owner);
        acct.execute(friend, 0.1 ether, "");
        _handle(_signOpEcdsa(_userOp(address(acct), _execData(friend, 0.1 ether, "")), ownerPk));
        assertEq(friend.balance, 0.2 ether);
        assertEq(acct.isValidSignature(h, ""), IERC1271.isValidSignature.selector);
    }

    function test_frozen_sessionUserOp_refusedByEntryPoint() public {
        _addSession();
        _freeze();
        Call[] memory calls = _calls1(_call(friend, 0.1 ether, ""));
        PackedUserOperation memory op = _userOp(address(acct), _executeUserOpData(calls));
        op.signature =
            abi.encodePacked(uint8(0x03), sessionKey, _ecdsaSign(_sessionPk(), ep.getUserOpHash(op)));
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, "AA22 expired or not due"));
        ep.handleOps(ops, payable(bundler));
    }

    function _sessionPk() internal returns (uint256 pk) {
        (, pk) = makeAddrAndKey("session");
    }

    function test_frozen_guardiansStillCancelAndRecover() public {
        vm.prank(owner);
        uint256 id = acct.execute(friend, 50 ether, "");
        _freeze();
        // guardians cancel with the threshold while frozen
        vm.prank(guardian1);
        acct.cancel(id);
        vm.prank(guardian3);
        acct.cancel(id);
        (, uint8 status,,) = acct.getAction(id);
        assertEq(status, 2);
        // and run a recovery the signers cannot cancel
        Signer[] memory ns = _one(_passkeySigner());
        vm.prank(guardian1);
        acct.approveRecovery(ns);
        vm.prank(guardian2);
        acct.approveRecovery(ns);
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.cancelRecovery();
        vm.warp(block.timestamp + 48 hours);
        vm.expectEmit(address(acct));
        emit IAccountEvents.AccountUnfrozen();
        acct.executeRecovery(ns);
        assertFalse(acct.isFrozen());
        (uint64 until,, uint64 next) = acct.freezeState();
        assertEq(until, 0);
        assertEq(next, 0, "a completed recovery resets the freeze cooldown");
    }

    // ------------------------------------------------------------------------------------------------------------
    // Unfreeze / expiry / cooldown / duration
    // ------------------------------------------------------------------------------------------------------------

    function test_unfreeze_requiresGuardianThreshold() public {
        vm.prank(guardian1);
        vm.expectRevert(IAccountEvents.NotFrozen.selector);
        acct.unfreeze();
        _freeze();
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        acct.unfreeze();
        vm.prank(guardian3);
        acct.unfreeze();
        assertTrue(acct.isFrozen(), "one guardian cannot unfreeze");
        vm.warp(block.timestamp + 1 days);
        vm.prank(guardian1);
        acct.unfreeze();
        assertFalse(acct.isFrozen());
        vm.prank(owner);
        acct.execute(friend, 0.1 ether, "");
        assertEq(friend.balance, 0.1 ether);
        // anti-hostage cooldown = time spent frozen (1 day)
        (,, uint64 next) = acct.freezeState();
        assertEq(next, block.timestamp + 1 days);
        vm.prank(guardian1);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.FreezeCooldown.selector, next));
        acct.freeze();
        vm.warp(next);
        _freeze();
        assertTrue(acct.isFrozen());
    }

    function test_freeze_expiresAutomatically_thenCooldown() public {
        uint256 t0 = block.timestamp;
        _freeze();
        vm.warp(t0 + 30 days - 1);
        assertTrue(acct.isFrozen());
        vm.warp(t0 + 30 days);
        assertFalse(acct.isFrozen(), "freeze never outlives its duration");
        vm.prank(owner);
        acct.cancelRecovery(); // signers are back
        // a guardian majority cannot chain freezes: the account stays unfrozen for as long as it was frozen
        vm.prank(guardian1);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.FreezeCooldown.selector, uint64(t0 + 60 days)));
        acct.freeze();
        vm.warp(t0 + 60 days);
        _freeze();
        assertTrue(acct.isFrozen());
    }

    function test_freezeDuration_boundsDelayAndRecoveryMargin() public {
        // vault on: the change is queued like any policy change
        vm.prank(owner);
        acct.setFreezeDuration(3 days);
        uint256 id = acct.lastActionId();
        (uint64 until, uint64 duration,) = acct.freezeState();
        assertEq(duration, 30 days);
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);
        (, duration,) = acct.freezeState();
        assertEq(duration, 3 days);
        // a freeze always outlasts the recovery delay (48 h) + 3 days of grace
        _freeze();
        (until,,) = acct.freezeState();
        assertEq(until, block.timestamp + 48 hours + 3 days);

        IXAccount plain = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(9)));
        vm.startPrank(owner);
        vm.expectRevert(IAccountEvents.InvalidParameter.selector);
        plain.setFreezeDuration(3 days - 1);
        vm.expectRevert(IAccountEvents.InvalidParameter.selector);
        plain.setFreezeDuration(60 days + 1);
        plain.setFreezeDuration(60 days);
        vm.stopPrank();
    }

    function test_guardianSetChange_invalidatesPendingVotes() public {
        IXAccount plain = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(5)));
        vm.prank(guardian1);
        plain.freeze();
        vm.prank(owner);
        plain.setGuardians(_guardians(), 2); // same set, new guardian epoch (instant: vault off)
        vm.prank(guardian2);
        plain.freeze();
        assertFalse(plain.isFrozen(), "votes from the previous guardian epoch do not count");
        vm.prank(guardian1);
        plain.freeze();
        assertTrue(plain.isFrozen());
    }

    function test_guardianCancelVotes_notResetBySignerCancelRecovery() public {
        // The thief (a signer) cannot wipe guardian votes by bumping the recovery nonce.
        vm.prank(owner);
        uint256 id = acct.execute(friend, 50 ether, "");
        vm.prank(guardian1);
        acct.cancel(id);
        vm.prank(owner);
        acct.cancelRecovery();
        vm.prank(guardian2);
        acct.cancel(id);
        (, uint8 status,,) = acct.getAction(id);
        assertEq(status, 2);
    }

    /// @dev Fuzz: whatever the configured durations, a freeze lasts at least recoveryDelay + grace (so a recovery
    ///      started at freeze time completes while frozen) and never more than MAX_FREEZE_DURATION.
    function testFuzz_freezeBounded(uint64 recoveryDelay, uint64 freezeDuration, uint32 startOffset) public {
        recoveryDelay = uint64(bound(recoveryDelay, 1 hours, 30 days));
        freezeDuration = uint64(bound(freezeDuration, 3 days, 60 days));
        IXAccount plain = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(7)));
        vm.startPrank(owner);
        plain.setRecoveryDelay(recoveryDelay);
        plain.setFreezeDuration(freezeDuration);
        vm.stopPrank();
        vm.warp(block.timestamp + startOffset);
        vm.prank(guardian2);
        plain.freeze();
        vm.prank(guardian3);
        plain.freeze();
        (uint64 until,,) = plain.freezeState();
        uint256 len = until - block.timestamp;
        assertLe(len, AccountConstants.MAX_FREEZE_DURATION);
        assertGe(len, freezeDuration);
        assertGe(len, uint256(recoveryDelay) + AccountConstants.FREEZE_RECOVERY_GRACE);
        vm.warp(until);
        assertFalse(plain.isFrozen());
    }

    // ------------------------------------------------------------------------------------------------------------
    // Demo: thief and owner both hold signer keys
    // ------------------------------------------------------------------------------------------------------------

    function test_demo_ownerVsThief_freezeBreaksStalemate() public {
        // Owner and thief both hold registered signer keys (e.g. the thief copied one device's key).
        InitParams memory p = _params(_one(_ecdsaSigner(owner)), true);
        p.signers = new Signer[](2);
        p.signers[0] = _ecdsaSigner(owner);
        p.signers[1] = _ecdsaSigner(attacker);
        IXAccount a = _createAccount(p, bytes32(uint256(42)));
        uint256 start = address(a).balance;
        Signer[] memory ns = _one(_passkeySigner()); // owner's fresh passkey

        // 1. Before the fix: the thief cancels every recovery the guardians start (the old stalemate).
        vm.prank(guardian1);
        a.approveRecovery(ns);
        vm.prank(guardian2);
        a.approveRecovery(ns);
        vm.prank(attacker);
        a.cancelRecovery();
        vm.warp(block.timestamp + 48 hours);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        a.executeRecovery(ns);

        // ... and queues a drain + tries to remove the owner's key.
        vm.startPrank(attacker);
        uint256 drainId = a.execute(attacker, 90 ether, "");
        a.removeSigner(bytes32(uint256(uint160(owner))));
        vm.stopPrank();

        // 2. Guardians (2-of-3) freeze the account.
        vm.prank(guardian1);
        a.freeze();
        vm.prank(guardian3);
        a.freeze();
        assertTrue(a.isFrozen());

        // 3. The thief can no longer move anything, cancel the recovery, or cancel the owner's actions.
        vm.startPrank(attacker);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        a.execute(attacker, 0.5 ether, "");
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        a.cancelRecovery();
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        a.addSigner(_ecdsaSigner(makeAddr("thief2")));
        vm.stopPrank();
        vm.warp(block.timestamp + 24 hours);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        a.execute(drainId); // matured, but frozen

        // 4. Guardians run the recovery; it cannot be cancelled by any signer.
        vm.prank(guardian1);
        a.approveRecovery(ns);
        vm.prank(guardian2);
        a.approveRecovery(ns);
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        a.cancelRecovery();
        vm.warp(block.timestamp + 48 hours);
        a.executeRecovery(ns);

        // 5. Recovery unfroze the account, revoked the thief's key and invalidated its queued actions.
        assertFalse(a.isFrozen());
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        a.execute(attacker, 0.5 ether, "");
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionInvalidated.selector, drainId));
        a.execute(drainId);
        assertEq(attacker.balance, 0);
        assertEq(address(a).balance, start);

        // 6. The owner (new passkey) is back in control.
        _handle(_signOpPasskey(_userOp(address(a), _execData(friend, 0.5 ether, ""))));
        assertEq(friend.balance, 0.5 ether);
        assertEq(a.getSigners().length, 1);
    }
}
