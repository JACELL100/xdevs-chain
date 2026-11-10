// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {SystemFixture} from "./utils/SystemFixture.sol";
import {Governance} from "../../src/system/Governance.sol";
import {Treasury} from "../../src/system/Treasury.sol";
import {FeeConfig} from "../../src/system/FeeConfig.sol";
import {IGovernance} from "../../src/system/interfaces/IGovernance.sol";
import {IStaking} from "../../src/system/interfaces/IStaking.sol";

contract Target {
    uint256 public value;
    uint256 public received;

    function set(uint256 v) external payable {
        value = v;
        received += msg.value;
    }

    function boom() external pure {
        revert("boom");
    }
}

contract GovernanceInitTest is SystemFixture {
    function setUp() public {
        _etch(address(gov), "Governance.sol:Governance");
    }

    function test_initialize() public {
        vm.expectEmit(address(gov));
        emit IGovernance.Initialized(address(staking), guardian);
        vm.prank(SYS);
        gov.initialize(address(staking), guardian, _defaultGovParams());
        assertTrue(gov.initialized());
        assertEq(gov.staking(), address(staking));
        assertEq(gov.guardian(), guardian);
        assertEq(gov.params().votingPeriod, 3 days);
    }

    function test_initialize_reverts() public {
        IGovernance.Params memory p = _defaultGovParams();
        vm.expectRevert(Governance.OnlySystem.selector);
        gov.initialize(address(staking), guardian, p);

        vm.prank(SYS);
        vm.expectRevert(abi.encodeWithSelector(Governance.InvalidParam.selector, "staking"));
        gov.initialize(address(0), guardian, p);

        vm.prank(SYS);
        gov.initialize(address(staking), guardian, p);
        vm.prank(SYS);
        vm.expectRevert(Governance.AlreadyInitialized.selector);
        gov.initialize(address(staking), guardian, p);
    }

    function test_propose_revertsBeforeInit() public {
        address[] memory t = new address[](1);
        vm.expectRevert(Governance.NotInitialized.selector);
        gov.propose(t, new uint256[](1), new bytes[](1), "");
    }

    function test_params_validation() public {
        IGovernance.Params memory p = _defaultGovParams();
        p.votingPeriod = 1 hours - 1;
        _expectInvalid(p, "votingPeriod");
        p.votingPeriod = 30 days + 1;
        _expectInvalid(p, "votingPeriod");
        p = _defaultGovParams();
        p.timelockDelay = 1 hours - 1;
        _expectInvalid(p, "timelockDelay");
        p.timelockDelay = 30 days + 1;
        _expectInvalid(p, "timelockDelay");
        p = _defaultGovParams();
        p.gracePeriod = 1 days - 1;
        _expectInvalid(p, "gracePeriod");
        p.gracePeriod = 60 days + 1;
        _expectInvalid(p, "gracePeriod");
        p = _defaultGovParams();
        p.pauseDuration = 14 days + 1;
        _expectInvalid(p, "pauseDuration");
        p = _defaultGovParams();
        p.quorumBps = 0;
        _expectInvalid(p, "quorumBps");
        p.quorumBps = 10_001;
        _expectInvalid(p, "quorumBps");
        p = _defaultGovParams();
        p.thresholdBps = 4_999;
        _expectInvalid(p, "thresholdBps");
        p.thresholdBps = 10_000;
        _expectInvalid(p, "thresholdBps");
    }

    function _expectInvalid(IGovernance.Params memory p, string memory name) internal {
        vm.prank(SYS);
        vm.expectRevert(abi.encodeWithSelector(Governance.InvalidParam.selector, name));
        gov.initialize(address(staking), guardian, p);
    }
}

contract GovernanceTest is SystemFixture {
    Target internal target;
    address internal v0;
    address internal v1;
    address internal v2;
    address internal v3;
    address internal alice = makeAddr("alice");

    function setUp() public {
        _deployGenesis();
        (address[] memory ops,) = staking.activeValidators();
        (v0, v1, v2, v3) = (ops[0], ops[1], ops[2], ops[3]);
        target = new Target();
        _nextBlock();
    }

    function _one(address t, uint256 value, bytes memory data)
        internal
        pure
        returns (address[] memory ts, uint256[] memory vs, bytes[] memory ds)
    {
        ts = new address[](1);
        vs = new uint256[](1);
        ds = new bytes[](1);
        ts[0] = t;
        vs[0] = value;
        ds[0] = data;
    }

    function _propose(address t, uint256 value, bytes memory data) internal returns (uint256 id) {
        (address[] memory ts, uint256[] memory vs, bytes[] memory ds) = _one(t, value, data);
        vm.prank(v0);
        id = gov.propose(ts, vs, ds, "test");
    }

    function _vote(uint256 id, address who, uint8 support) internal {
        vm.prank(who);
        gov.castVote(id, support);
    }

    function _passAndQueue(uint256 id) internal {
        _vote(id, v0, 1);
        _vote(id, v1, 1);
        _vote(id, v2, 1);
        vm.warp(block.timestamp + 3 days);
        gov.queue(id);
    }

    function _execute(address t, uint256 value, bytes memory data) internal returns (uint256 id) {
        id = _propose(t, value, data);
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days);
        gov.execute(id);
    }

    // ------------------------------------------------------------------ propose

    function test_propose() public {
        (address[] memory ts, uint256[] memory vs, bytes[] memory ds) =
            _one(address(target), 0, abi.encodeCall(Target.set, (7)));
        vm.expectEmit(address(gov));
        emit IGovernance.ProposalCreated(
            1, v0, ts, vs, ds, block.number - 1, uint64(block.timestamp + 3 days), "desc"
        );
        vm.prank(v0);
        uint256 id = gov.propose(ts, vs, ds, "desc");
        assertEq(id, 1);
        assertEq(gov.proposalCount(), 1);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Active));
        (address proposer, uint256 snap, uint64 voteEnd,, uint256 quorum,,,) = gov.getProposal(id);
        assertEq(proposer, v0);
        assertEq(snap, block.number - 1);
        assertEq(voteEnd, block.timestamp + 3 days);
        assertEq(quorum, 4 * GENESIS_STAKE * 3_340 / 10_000);
        (address[] memory t2, uint256[] memory v2_, bytes[] memory d2) = gov.getActions(id);
        assertEq(t2[0], address(target));
        assertEq(v2_[0], 0);
        assertEq(d2[0], ds[0]);
    }

    function test_propose_reverts() public {
        vm.startPrank(v0);
        vm.expectRevert(abi.encodeWithSelector(Governance.InvalidProposal.selector, "actions"));
        gov.propose(new address[](0), new uint256[](0), new bytes[](0), "");
        vm.expectRevert(abi.encodeWithSelector(Governance.InvalidProposal.selector, "actions"));
        gov.propose(new address[](17), new uint256[](17), new bytes[](17), "");
        vm.expectRevert(abi.encodeWithSelector(Governance.InvalidProposal.selector, "length mismatch"));
        gov.propose(new address[](1), new uint256[](2), new bytes[](1), "");
        vm.expectRevert(abi.encodeWithSelector(Governance.InvalidProposal.selector, "length mismatch"));
        gov.propose(new address[](1), new uint256[](1), new bytes[](2), "");
        vm.stopPrank();

        vm.prank(alice);
        vm.expectRevert(Governance.BelowProposalThreshold.selector);
        gov.propose(new address[](1), new uint256[](1), new bytes[](1), "");
    }

    function test_propose_usesSnapshotOfPreviousBlock() public {
        // stake acquired in the proposal's own block does not count
        _delegate(alice, v0, 20_000 ether);
        vm.prank(alice);
        vm.expectRevert(Governance.BelowProposalThreshold.selector);
        gov.propose(new address[](1), new uint256[](1), new bytes[](1), "");
        _nextBlock();
        vm.prank(alice);
        gov.propose(new address[](1), new uint256[](1), new bytes[](1), "");
    }

    // ------------------------------------------------------------------ voting

    function test_castVote_tallies() public {
        uint256 id = _propose(address(target), 0, "");
        vm.expectEmit(address(gov));
        emit IGovernance.VoteCast(v0, id, 1, GENESIS_STAKE);
        _vote(id, v0, 1);
        _vote(id, v1, 0);
        _vote(id, v2, 2);
        (,,,,, uint256 f, uint256 a, uint256 ab) = gov.getProposal(id);
        assertEq(f, GENESIS_STAKE);
        assertEq(a, GENESIS_STAKE);
        assertEq(ab, GENESIS_STAKE);
        assertTrue(gov.hasVoted(id, v0));
    }

    function test_castVote_reverts() public {
        uint256 id = _propose(address(target), 0, "");
        vm.expectRevert(Governance.UnknownProposal.selector);
        gov.castVote(99, 1);
        vm.prank(v0);
        vm.expectRevert(Governance.InvalidVoteType.selector);
        gov.castVote(id, 3);
        _vote(id, v0, 1);
        vm.prank(v0);
        vm.expectRevert(Governance.AlreadyVoted.selector);
        gov.castVote(id, 1);
        vm.prank(alice);
        vm.expectRevert(Governance.NoVotingPower.selector);
        gov.castVote(id, 1);

        // stake added after the snapshot does not give voting power
        _delegate(alice, v0, 1_000 ether);
        _nextBlock();
        vm.prank(alice);
        vm.expectRevert(Governance.NoVotingPower.selector);
        gov.castVote(id, 1);

        vm.warp(block.timestamp + 3 days);
        vm.prank(v1);
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Defeated)
        );
        gov.castVote(id, 1);
    }

    function test_defeated_byQuorum() public {
        uint256 id = _propose(address(target), 0, "");
        _vote(id, v0, 1); // 25% < 33.4%
        vm.warp(block.timestamp + 3 days);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Defeated));
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Defeated)
        );
        gov.queue(id);
    }

    function test_quorum_countsAbstain() public {
        uint256 id = _propose(address(target), 0, "");
        _vote(id, v0, 1);
        _vote(id, v1, 2);
        vm.warp(block.timestamp + 3 days);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Succeeded));
    }

    function test_defeated_byThreshold() public {
        uint256 id = _propose(address(target), 0, "");
        _vote(id, v0, 1);
        _vote(id, v1, 1);
        _vote(id, v2, 0);
        _vote(id, v3, 0); // exactly 50% is not a majority
        vm.warp(block.timestamp + 3 days);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Defeated));
    }

    function test_defeated_noForVotes() public {
        uint256 id = _propose(address(target), 0, "");
        _vote(id, v0, 2);
        _vote(id, v1, 2);
        vm.warp(block.timestamp + 3 days);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Defeated));
    }

    // ------------------------------------------------------------------ queue / execute

    function test_fullLifecycle() public {
        vm.deal(address(gov), 1 ether);
        uint256 id = _propose(address(target), 1 ether, abi.encodeCall(Target.set, (42)));
        _vote(id, v0, 1);
        _vote(id, v1, 1);
        _vote(id, v2, 1);
        vm.warp(block.timestamp + 3 days - 1);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Active));
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Active)
        );
        gov.queue(id);
        vm.warp(block.timestamp + 1);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Succeeded));

        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Succeeded)
        );
        gov.execute(id);

        uint64 eta = uint64(block.timestamp + 1 days);
        vm.expectEmit(address(gov));
        emit IGovernance.ProposalQueued(id, eta);
        gov.queue(id);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Queued));

        vm.warp(eta - 1);
        vm.expectRevert(abi.encodeWithSelector(Governance.TimelockNotExpired.selector, eta));
        gov.execute(id);

        vm.warp(eta);
        vm.expectEmit(address(gov));
        emit IGovernance.ProposalExecuted(id);
        gov.execute(id);
        assertEq(target.value(), 42);
        assertEq(target.received(), 1 ether);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Executed));

        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Executed)
        );
        gov.execute(id);
    }

    function test_execute_forwardsMsgValue() public {
        uint256 id = _propose(address(target), 2 ether, abi.encodeCall(Target.set, (1)));
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days);
        vm.deal(alice, 2 ether);
        vm.prank(alice);
        gov.execute{value: 2 ether}(id);
        assertEq(target.received(), 2 ether);
    }

    function test_execute_callFailureKeepsQueued() public {
        uint256 id = _propose(address(target), 0, abi.encodeCall(Target.boom, ()));
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days);
        vm.expectRevert(
            abi.encodeWithSelector(
                Governance.CallFailed.selector, 0, abi.encodeWithSignature("Error(string)", "boom")
            )
        );
        gov.execute(id);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Queued));
    }

    function test_expired() public {
        uint256 id = _propose(address(target), 0, abi.encodeCall(Target.set, (1)));
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days + 14 days + 1);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Expired));
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Expired)
        );
        gov.execute(id);
    }

    // ------------------------------------------------------------------ cancel

    function test_cancel() public {
        uint256 id = _propose(address(target), 0, "");
        vm.prank(alice);
        vm.expectRevert(Governance.OnlyProposer.selector);
        gov.cancel(id);
        vm.expectEmit(address(gov));
        emit IGovernance.ProposalCanceled(id);
        vm.prank(v0);
        gov.cancel(id);
        assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Canceled));
        vm.prank(v0);
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Canceled)
        );
        gov.cancel(id);
    }

    function test_cancel_queued() public {
        uint256 id = _propose(address(target), 0, abi.encodeCall(Target.set, (1)));
        _passAndQueue(id);
        vm.prank(v0);
        gov.cancel(id);
        vm.warp(block.timestamp + 1 days);
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Canceled)
        );
        gov.execute(id);
    }

    function test_cancel_succeededAndDefeated() public {
        uint256 id = _propose(address(target), 0, "");
        _vote(id, v0, 1);
        _vote(id, v1, 1);
        uint256 id2 = _propose(address(target), 0, "");
        vm.warp(block.timestamp + 3 days);
        vm.prank(v0);
        gov.cancel(id); // Succeeded
        vm.prank(v0);
        vm.expectRevert(
            abi.encodeWithSelector(Governance.InvalidState.selector, IGovernance.ProposalState.Defeated)
        );
        gov.cancel(id2);
    }

    // ------------------------------------------------------------------ emergency pause

    function test_pause_blocksOnlyNewProposals() public {
        uint256 id = _propose(address(target), 0, abi.encodeCall(Target.set, (5)));
        vm.expectEmit(address(gov));
        emit IGovernance.Paused(guardian, uint64(block.timestamp + 3 days));
        vm.prank(guardian);
        gov.pause();
        assertTrue(gov.paused());

        (address[] memory ts, uint256[] memory vs, bytes[] memory ds) = _one(address(target), 0, "");
        vm.prank(v0);
        vm.expectRevert(Governance.ProposalsPaused.selector);
        gov.propose(ts, vs, ds, "");

        // existing proposals keep going
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days);
        gov.execute(id);
        assertEq(target.value(), 5);
    }

    function test_pause_cooldownAndExpiry() public {
        vm.prank(alice);
        vm.expectRevert(Governance.OnlyGuardian.selector);
        gov.pause();

        vm.prank(guardian);
        gov.pause();
        vm.prank(guardian);
        vm.expectRevert(Governance.PauseCooldown.selector);
        gov.pause();

        vm.warp(block.timestamp + 3 days);
        assertFalse(gov.paused());
        vm.prank(guardian);
        vm.expectRevert(Governance.PauseCooldown.selector);
        gov.pause(); // cooldown: governance gets a guaranteed window

        _propose(address(target), 0, ""); // works again
        vm.warp(block.timestamp + 3 days);
        vm.prank(guardian);
        gov.pause();
        assertTrue(gov.paused());
    }

    function test_unpause() public {
        vm.prank(guardian);
        vm.expectRevert(Governance.NotPaused.selector);
        gov.unpause();
        vm.prank(guardian);
        gov.pause();
        vm.prank(alice);
        vm.expectRevert(Governance.OnlyGuardian.selector);
        gov.unpause();
        vm.expectEmit(address(gov));
        emit IGovernance.Unpaused(guardian);
        vm.prank(guardian);
        gov.unpause();
        assertFalse(gov.paused());
    }

    function test_unpause_bySelf() public {
        IGovernance.Params memory p = _defaultGovParams();
        p.pauseDuration = 14 days; // longer than vote + timelock
        _execute(address(gov), 0, abi.encodeCall(Governance.setParams, (p)));
        _nextBlock();
        uint256 id = _propose(address(gov), 0, abi.encodeCall(Governance.unpause, ()));
        vm.prank(guardian);
        gov.pause();
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days);
        assertTrue(gov.paused());
        gov.execute(id);
        assertFalse(gov.paused());
    }

    function test_noGuardianCannotPause() public {
        _execute(address(gov), 0, abi.encodeCall(Governance.setGuardian, (address(0))));
        assertEq(gov.guardian(), address(0));
        vm.prank(address(0));
        vm.expectRevert(Governance.OnlyGuardian.selector);
        gov.pause();
    }

    // ------------------------------------------------------------------ self-administration

    function test_selfAdmin_onlySelf() public {
        IGovernance.Params memory p = _defaultGovParams();
        vm.expectRevert(Governance.OnlySelf.selector);
        gov.setParams(p);
        vm.expectRevert(Governance.OnlySelf.selector);
        gov.setGuardian(alice);
    }

    function test_setParams_viaProposal() public {
        IGovernance.Params memory p = _defaultGovParams();
        p.votingPeriod = 1 days;
        _execute(address(gov), 0, abi.encodeCall(Governance.setParams, (p)));
        assertEq(gov.params().votingPeriod, 1 days);
    }

    function test_setGuardian_viaProposal() public {
        uint256 id = _propose(address(gov), 0, abi.encodeCall(Governance.setGuardian, (alice)));
        _passAndQueue(id);
        vm.warp(block.timestamp + 1 days);
        vm.expectEmit(address(gov));
        emit IGovernance.GuardianUpdated(guardian, alice);
        gov.execute(id);
        assertEq(gov.guardian(), alice);
    }

    function test_receive() public {
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        (bool ok,) = address(gov).call{value: 1 ether}("");
        assertTrue(ok);
        assertEq(address(gov).balance, 1 ether);
    }

    // ------------------------------------------------------------------ end-to-end with the other predeploys

    function test_e2e_feeConfigChange() public {
        FeeConfig.FeeParams memory p = _defaultFeeParams();
        p.minBaseFee = 1 gwei;
        _execute(address(feeConfig), 0, abi.encodeCall(FeeConfig.setParams, (p)));
        assertEq(feeConfig.minBaseFee(), 1 gwei);
        assertEq(feeConfig.version(), 2);
    }

    function test_e2e_treasuryGrant() public {
        _execute(
            address(treasury), 0, abi.encodeCall(Treasury.grant, (payable(alice), 1_000 ether, "hackathon"))
        );
        assertEq(alice.balance, 1_000 ether);
        assertEq(treasury.totalGranted(), 1_000 ether);
    }

    function test_e2e_stakingParams() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.maxValidators = 10;
        _execute(address(staking), 0, abi.encodeCall(IStaking.setParams, (p)));
        assertEq(staking.params().maxValidators, 10);
    }
}
