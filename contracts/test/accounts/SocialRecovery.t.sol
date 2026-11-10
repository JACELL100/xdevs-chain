// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {IAccountEvents, IAccountExtension} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {
    Call,
    InitParams,
    Signer,
    SessionKeyConfig,
    SessionPermission,
    SessionTokenCap
} from "../../src/accounts/libraries/AccountTypes.sol";

contract SocialRecoveryTest is AccountTestBase {
    IXAccount internal acct;
    Signer[] internal newSigners;

    function setUp() public override {
        super.setUp();
        acct = _vaultAccount(); // owner ECDSA, guardians 1..3, threshold 2, 48 h delay
        newSigners.push(_passkeySigner()); // the owner lost their key; recover to a new passkey
    }

    function test_recovery_happyPath() public {
        vm.prank(guardian1);
        acct.approveRecovery(newSigners);
        (, uint8 approvals, uint64 readyAt) = acct.getRecoveryProposal(newSigners);
        assertEq(approvals, 1);
        assertEq(readyAt, 0, "timer starts only at threshold");

        vm.prank(guardian3);
        acct.approveRecovery(newSigners);
        (, approvals, readyAt) = acct.getRecoveryProposal(newSigners);
        assertEq(approvals, 2);
        assertEq(readyAt, block.timestamp + 48 hours);

        vm.warp(readyAt - 1);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        acct.executeRecovery(newSigners);

        vm.warp(readyAt);
        vm.prank(makeAddr("anyone"));
        acct.executeRecovery(newSigners);

        Signer[] memory s = acct.getSigners();
        assertEq(s.length, 1);
        assertEq(s[0].x, passkeyX);
        // old key is out
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(owner, 1, "");
        // replay of the same proposal is impossible (nonce bumped)
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        acct.executeRecovery(newSigners);
    }

    function test_recovery_onlyGuardians_noDoubleVote() public {
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        acct.approveRecovery(newSigners);
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        acct.approveRecovery(newSigners);

        vm.startPrank(guardian1);
        acct.approveRecovery(newSigners);
        vm.expectRevert(IAccountEvents.AlreadyApproved.selector);
        acct.approveRecovery(newSigners);
        vm.stopPrank();

        vm.warp(block.timestamp + 365 days);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        acct.executeRecovery(newSigners);
    }

    function test_recovery_votesForDifferentSignerSetsDoNotCombine() public {
        Signer[] memory other = _one(_ecdsaSigner(attacker));
        vm.prank(guardian1);
        acct.approveRecovery(newSigners);
        vm.prank(guardian2);
        acct.approveRecovery(other);
        vm.warp(block.timestamp + 30 days);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        acct.executeRecovery(newSigners);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        acct.executeRecovery(other);
    }

    function test_recovery_ownerCancelsDuringDelay() public {
        vm.prank(guardian1);
        acct.approveRecovery(newSigners);
        vm.prank(guardian2);
        acct.approveRecovery(newSigners);
        vm.warp(block.timestamp + 47 hours);
        vm.prank(owner);
        acct.cancelRecovery();
        vm.warp(block.timestamp + 10 days);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        acct.executeRecovery(newSigners);
        // a stranger cannot cancel
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.cancelRecovery();
    }

    function test_recovery_guardianChangeInvalidatesApprovals() public {
        IXAccount plain = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(3)));
        vm.prank(guardian1);
        plain.approveRecovery(newSigners);
        vm.prank(guardian2);
        plain.approveRecovery(newSigners);
        address[] memory g = new address[](2);
        g[0] = guardian1;
        g[1] = guardian2;
        vm.prank(owner);
        plain.setGuardians(g, 2); // vault disabled -> instant
        vm.warp(block.timestamp + 3 days);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        plain.executeRecovery(newSigners);
        assertFalse(plain.isGuardian(guardian3));
    }

    function test_recovery_withoutGuardians_isImpossible() public {
        IXAccount lonely = _createAccount(_paramsNoGuardians(), bytes32(uint256(4)));
        vm.prank(guardian1);
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        lonely.approveRecovery(newSigners);
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        lonely.executeRecovery(newSigners);
    }

    function _paramsNoGuardians() internal view returns (InitParams memory p) {
        p = _params(_one(_ecdsaSigner(owner)), false);
        p.guardians = new address[](0);
        p.guardianThreshold = 0;
    }

    function test_recovery_invalidatesQueuedActions_andSessions() public {
        // the thief (with the old key) queued a drain and registered a session key
        vm.startPrank(owner);
        uint256 drainId = acct.execute(attacker, 50 ether, "");
        SessionKeyConfig memory cfg;
        cfg.key = attacker;
        cfg.validUntil = uint48(block.timestamp + 365 days);
        cfg.nativeCap = 1 ether;
        cfg.permissions = new SessionPermission[](1);
        cfg.permissions[0] = SessionPermission(attacker, bytes4(0));
        cfg.tokenCaps = new SessionTokenCap[](0);
        acct.addSessionKey(cfg);
        vm.stopPrank();

        vm.prank(guardian1);
        acct.approveRecovery(newSigners);
        vm.prank(guardian2);
        acct.approveRecovery(newSigners);
        vm.warp(block.timestamp + 48 hours);
        acct.executeRecovery(newSigners);

        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionInvalidated.selector, drainId));
        acct.execute(drainId);
        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.SessionNotActive.selector, attacker));
        acct.executeWithSession(_calls1(_call(attacker, 0.1 ether, "")));
        assertEq(attacker.balance, 0);
    }

    /// @dev Fuzz: whatever subset of guardians approves and whenever execution is attempted, recovery succeeds
    ///      iff approvals >= threshold and the delay since the threshold was reached has elapsed.
    function testFuzz_recovery_thresholdAndDelay(uint8 voterMask, uint32 waitAfter) public {
        address[3] memory gs = [guardian1, guardian2, guardian3];
        uint256 votes;
        uint256 thresholdTime;
        for (uint256 i; i < 3; ++i) {
            if (voterMask & (1 << i) != 0) {
                vm.warp(block.timestamp + 1 hours);
                vm.prank(gs[i]);
                acct.approveRecovery(newSigners);
                if (++votes == 2) thresholdTime = block.timestamp;
            }
        }
        vm.warp(block.timestamp + waitAfter);
        bool shouldWork = votes >= 2 && block.timestamp >= thresholdTime + 48 hours;
        Signer[] memory ns = newSigners;
        (bool ok,) = address(acct).call(abi.encodeCall(IAccountExtension.executeRecovery, (ns)));
        assertEq(ok, shouldWork);
        assertEq(acct.getSigners()[0].x, shouldWork ? passkeyX : bytes32(uint256(uint160(owner))));
    }
}

