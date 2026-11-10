// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {SystemFixture, Rejecter} from "./utils/SystemFixture.sol";
import {Staking} from "../../src/system/Staking.sol";
import {Governance} from "../../src/system/Governance.sol";
import {Treasury} from "../../src/system/Treasury.sol";
import {FeeConfig} from "../../src/system/FeeConfig.sol";
import {IStaking} from "../../src/system/interfaces/IStaking.sol";

contract ConstructorTest is SystemFixture {
    /// @dev Constructors are empty: a freshly created instance is uninitialized (genesis never runs them).
    function test_constructorsLeaveStateEmpty() public {
        assertFalse(new Staking().initialized());
        assertFalse(new Governance().initialized());
        assertEq(new Treasury().governance(), address(0));
        assertEq(new FeeConfig().version(), 0);
    }
}

contract StakingInitTest is SystemFixture {
    function setUp() public {
        _etch(address(staking), "Staking.sol:Staking");
    }

    function _init(IStaking.GenesisValidator[] memory gv, uint256 bal) internal {
        vm.deal(address(staking), bal);
        vm.prank(SYS);
        staking.initialize(address(gov), _defaultStakingParams(), gv);
    }

    function test_initialize_genesisSet() public {
        IStaking.GenesisValidator[] memory gv = _genesisValidators(4, GENESIS_STAKE);
        gv[2].stake = GENESIS_STAKE + 1; // ranks first
        vm.expectEmit(address(staking));
        emit IStaking.Initialized(address(gov), 4, 4 * GENESIS_STAKE + 1);
        _init(gv, 4 * GENESIS_STAKE + 1);

        assertTrue(staking.initialized());
        assertEq(staking.governance(), address(gov));
        assertEq(staking.currentEpoch(), 0);
        assertEq(staking.windowId(), 1);
        (address[] memory ops, uint256[] memory powers) = staking.activeValidators();
        assertEq(ops.length, 4);
        assertEq(ops[0], gv[2].operator);
        assertEq(powers[0], GENESIS_STAKE + 1);
        for (uint256 i = 2; i < 4; ++i) {
            assertLt(uint160(ops[i - 1]), uint160(ops[i]), "tie-break by address");
        }
        (address[] memory o2, bytes32[] memory keys, uint256[] memory p2) = staking.consensusValidatorSet();
        assertEq(keccak256(abi.encode(o2, keys, p2)), staking.validatorSetHash());
        assertEq(keys[0], gv[2].consensusKey);
        assertEq(uint8(_status(ops[0])), uint8(IStaking.ValidatorStatus.Active));
        assertEq(staking.accountedBalance(), 4 * GENESIS_STAKE + 1);
        assertEq(staking.totalBonded(), 4 * GENESIS_STAKE + 1);
        assertEq(staking.getVotes(gv[0].operator), GENESIS_STAKE);
        assertEq(staking.consensusKeyOwner(gv[1].consensusKey), gv[1].operator);
        assertEq(staking.candidates().length, 4);
        (address[] memory sOps,) = staking.signingSet();
        assertEq(sOps.length, 4);
    }

    function test_initialize_revertsIfNotSystem() public {
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.initialize(address(gov), _defaultStakingParams(), _genesisValidators(1, MIN_SELF));
    }

    function test_initialize_revertsTwice() public {
        _init(_genesisValidators(1, MIN_SELF), MIN_SELF);
        IStaking.GenesisValidator[] memory gv = _genesisValidators(1, MIN_SELF);
        IStaking.Params memory p = _defaultStakingParams();
        vm.prank(SYS);
        vm.expectRevert(Staking.AlreadyInitialized.selector);
        staking.initialize(address(gov), p, gv);
    }

    function _expectGenesisRevert(IStaking.GenesisValidator[] memory gv, uint256 bal, string memory reason)
        internal
    {
        vm.deal(address(staking), bal);
        IStaking.Params memory p = _defaultStakingParams();
        vm.prank(SYS);
        vm.expectRevert(abi.encodeWithSelector(Staking.InvalidGenesis.selector, reason));
        staking.initialize(address(gov), p, gv);
    }

    function test_initialize_revertsZeroGovernance() public {
        IStaking.GenesisValidator[] memory gv = _genesisValidators(1, MIN_SELF);
        IStaking.Params memory p = _defaultStakingParams();
        vm.prank(SYS);
        vm.expectRevert(abi.encodeWithSelector(Staking.InvalidGenesis.selector, "governance"));
        staking.initialize(address(0), p, gv);
    }

    function test_initialize_revertsBadGenesis() public {
        _expectGenesisRevert(new IStaking.GenesisValidator[](0), 0, "no validators");

        IStaking.GenesisValidator[] memory gv = _genesisValidators(2, MIN_SELF);
        gv[0].operator = address(0);
        _expectGenesisRevert(gv, 2 * MIN_SELF, "operator");

        gv = _genesisValidators(2, MIN_SELF);
        gv[1].operator = gv[0].operator;
        _expectGenesisRevert(gv, 2 * MIN_SELF, "duplicate");

        gv = _genesisValidators(2, MIN_SELF);
        gv[1].consensusKey = gv[0].consensusKey;
        _expectGenesisRevert(gv, 2 * MIN_SELF, "consensus key");

        gv = _genesisValidators(2, MIN_SELF);
        gv[0].consensusKey = bytes32(0);
        _expectGenesisRevert(gv, 2 * MIN_SELF, "consensus key");

        gv = _genesisValidators(2, MIN_SELF);
        gv[0].stake = MIN_SELF - 1;
        _expectGenesisRevert(gv, 2 * MIN_SELF, "stake");

        gv = _genesisValidators(2, MIN_SELF);
        gv[0].commissionBps = 2_001;
        _expectGenesisRevert(gv, 2 * MIN_SELF, "commission");

        gv = _genesisValidators(2, MIN_SELF);
        _expectGenesisRevert(gv, 2 * MIN_SELF - 1, "balance");
    }

    function test_initialize_revertsTooManyGenesisValidators() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.maxValidators = 1;
        p.maxCandidates = 1;
        IStaking.GenesisValidator[] memory gv = _genesisValidators(2, MIN_SELF);
        vm.deal(address(staking), 2 * MIN_SELF);
        vm.prank(SYS);
        vm.expectRevert(abi.encodeWithSelector(Staking.InvalidGenesis.selector, "too many"));
        staking.initialize(address(gov), p, gv);
    }

    function test_functionsRevertBeforeInit() public {
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.register(bytes32(uint256(1)), 0);
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.delegate(address(1));
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.undelegate(address(1), 1);
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.setCommission(1);
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.unjail();
        vm.startPrank(SYS);
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.onEpoch();
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.recordParticipation(address(0), 0);
        vm.expectRevert(Staking.NotInitialized.selector);
        staking.slash("");
        vm.stopPrank();
    }

    function test_setParams_validation() public {
        _init(_genesisValidators(1, MIN_SELF), MIN_SELF);
        IStaking.Params memory p;

        p = _defaultStakingParams();
        p.minSelfStake = 0;
        _expectInvalid(p, "minSelfStake");
        p = _defaultStakingParams();
        p.minDelegation = 0;
        _expectInvalid(p, "minDelegation");
        p = _defaultStakingParams();
        p.unbondingSeconds = 1 hours - 1;
        _expectInvalid(p, "unbondingSeconds");
        p.unbondingSeconds = 60 days + 1;
        _expectInvalid(p, "unbondingSeconds");
        p = _defaultStakingParams();
        p.commissionChangeInterval = 30 days + 1;
        _expectInvalid(p, "commissionChangeInterval");
        p = _defaultStakingParams();
        p.downtimeWindow = 0;
        _expectInvalid(p, "downtimeWindow");
        p.downtimeWindow = 1_000_001;
        _expectInvalid(p, "downtimeWindow");
        p = _defaultStakingParams();
        p.downtimeJailSeconds = 30 days + 1;
        _expectInvalid(p, "downtimeJailSeconds");
        p = _defaultStakingParams();
        p.maxValidators = 0;
        _expectInvalid(p, "maxValidators");
        p.maxValidators = 257;
        _expectInvalid(p, "maxValidators");
        p = _defaultStakingParams();
        p.maxCandidates = 99;
        _expectInvalid(p, "maxCandidates");
        p.maxCandidates = 2_001;
        _expectInvalid(p, "maxCandidates");
        p = _defaultStakingParams();
        p.maxCommissionBps = 10_001;
        _expectInvalid(p, "maxCommissionBps");
        p = _defaultStakingParams();
        p.maxCommissionChangeBps = 10_001;
        _expectInvalid(p, "maxCommissionChangeBps");
        p = _defaultStakingParams();
        p.proposerBonusBps = 10_001;
        _expectInvalid(p, "proposerBonusBps");
        p = _defaultStakingParams();
        p.doubleSignSlashBps = 5_001;
        _expectInvalid(p, "doubleSignSlashBps");
        p = _defaultStakingParams();
        p.downtimeSlashBps = 5_001;
        _expectInvalid(p, "downtimeSlashBps");
        p = _defaultStakingParams();
        p.minSignedBps = 10_001;
        _expectInvalid(p, "minSignedBps");
    }

    function _expectInvalid(IStaking.Params memory p, string memory name) internal {
        vm.prank(address(gov));
        vm.expectRevert(abi.encodeWithSelector(Staking.InvalidParam.selector, name));
        staking.setParams(p);
    }
}

contract StakingTest is SystemFixture {
    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");
    address internal carol = makeAddr("carol");
    address internal v0;
    address internal v1;
    address internal v2;
    address internal v3;

    function setUp() public {
        _deployGenesis();
        (address[] memory ops,) = staking.activeValidators();
        (v0, v1, v2, v3) = (ops[0], ops[1], ops[2], ops[3]);
    }

    // ------------------------------------------------------------------ setParams / views

    function test_setParams_onlyGovernance() public {
        IStaking.Params memory p = _defaultStakingParams();
        vm.expectRevert(Staking.OnlyGovernance.selector);
        staking.setParams(p);

        p.maxValidators = 50;
        vm.prank(address(gov));
        vm.expectEmit(address(staking));
        emit IStaking.ParamsUpdated(p);
        staking.setParams(p);
        assertEq(staking.params().maxValidators, 50);
    }

    function test_getPastVotes_futureReverts() public {
        vm.expectRevert(Staking.FutureLookup.selector);
        staking.getPastVotes(v0, block.number);
        vm.expectRevert(Staking.FutureLookup.selector);
        staking.getPastTotalVotes(block.number);
        _nextBlock();
        assertEq(staking.getPastTotalVotes(block.number - 1), 4 * GENESIS_STAKE);
        assertEq(staking.getPastVotes(v0, block.number - 1), GENESIS_STAKE);
    }

    // ------------------------------------------------------------------ register

    function test_register() public {
        vm.deal(alice, MIN_SELF);
        vm.expectEmit(address(staking));
        emit IStaking.Delegated(alice, alice, MIN_SELF);
        vm.expectEmit(address(staking));
        emit IStaking.ValidatorRegistered(alice, _key(alice), MIN_SELF);
        vm.prank(alice);
        staking.register{value: MIN_SELF}(_key(alice), 500);

        IStaking.ValidatorInfo memory info = staking.getValidator(alice);
        assertEq(uint8(info.status), uint8(IStaking.ValidatorStatus.Candidate));
        assertEq(info.totalStake, MIN_SELF);
        assertEq(info.selfStake, MIN_SELF);
        assertEq(info.commissionBps, 500);
        assertEq(info.consensusKey, _key(alice));
        assertEq(staking.candidates().length, 5);
        assertFalse(_activeContains(alice));
        assertEq(staking.accountedBalance(), 4 * GENESIS_STAKE + MIN_SELF);

        _epoch();
        assertTrue(_activeContains(alice));
        assertEq(uint8(_status(alice)), uint8(IStaking.ValidatorStatus.Active));
    }

    function test_register_reverts() public {
        vm.deal(alice, 10 * MIN_SELF);
        vm.startPrank(alice);
        vm.expectRevert(Staking.InvalidConsensusKey.selector);
        staking.register{value: MIN_SELF}(bytes32(0), 0);
        vm.expectRevert(Staking.ConsensusKeyInUse.selector);
        staking.register{value: MIN_SELF}(_key(v0), 0);
        vm.expectRevert(Staking.InsufficientSelfStake.selector);
        staking.register{value: MIN_SELF - 1}(_key(alice), 0);
        vm.expectRevert(Staking.CommissionTooHigh.selector);
        staking.register{value: MIN_SELF}(_key(alice), 2_001);
        staking.register{value: MIN_SELF}(_key(alice), 0);
        vm.expectRevert(Staking.AlreadyRegistered.selector);
        staking.register{value: MIN_SELF}(bytes32(uint256(5)), 0);
        vm.stopPrank();
    }

    function test_register_tooManyCandidates() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.maxValidators = 4;
        p.maxCandidates = 4;
        vm.prank(address(gov));
        staking.setParams(p);
        vm.deal(alice, MIN_SELF);
        vm.prank(alice);
        vm.expectRevert(Staking.TooManyCandidates.selector);
        staking.register{value: MIN_SELF}(_key(alice), 0);
    }

    // ------------------------------------------------------------------ delegate

    function test_delegate() public {
        _delegate(alice, v1, 100 ether);
        (uint256 shares, uint256 amount, uint256 principal) = staking.delegationOf(v1, alice);
        assertEq(shares, 100 ether);
        assertEq(amount, 100 ether);
        assertEq(principal, 100 ether);
        assertEq(staking.getVotes(alice), 100 ether);
        assertEq(staking.getValidator(v1).totalStake, GENESIS_STAKE + 100 ether);
        assertEq(staking.totalBonded(), 4 * GENESIS_STAKE + 100 ether);
    }

    function test_delegate_reverts() public {
        vm.deal(alice, 10 ether);
        vm.startPrank(alice);
        vm.expectRevert(Staking.ValidatorNotAcceptingStake.selector);
        staking.delegate{value: 1 ether}(bob);
        vm.expectRevert(Staking.DelegationTooSmall.selector);
        staking.delegate{value: 1 ether - 1}(v0);
        vm.stopPrank();

        _slashDoubleSign(v0);
        vm.prank(alice);
        vm.expectRevert(Staking.ValidatorNotAcceptingStake.selector);
        staking.delegate{value: 1 ether}(v0);
    }

    function test_delegate_toJailedValidatorAllowed() public {
        _jailByDowntime(v3);
        _delegate(alice, v3, 5 ether);
        (, uint256 amount,) = staking.delegationOf(v3, alice);
        assertApproxEqAbs(amount, 5 ether, 1);
    }

    function test_delegate_afterSlashGetsMoreShares() public {
        _jailByDowntime(v3);
        uint256 stakeAfter = staking.getValidator(v3).totalStake;
        _delegate(alice, v3, 100 ether);
        (uint256 shares,,) = staking.delegationOf(v3, alice);
        assertEq(shares, 100 ether * GENESIS_STAKE / stakeAfter);
    }

    // ------------------------------------------------------------------ undelegate / withdraw

    function test_undelegate_partialAndWithdraw() public {
        _delegate(alice, v1, 100 ether);
        uint64 expected = uint64(block.timestamp + 1 days);
        vm.expectEmit(address(staking));
        emit IStaking.Undelegated(alice, v1, 40 ether, 40 ether, expected);
        vm.prank(alice);
        staking.undelegate(v1, 40 ether);

        (, uint256 amount, uint256 principal) = staking.delegationOf(v1, alice);
        assertEq(amount, 60 ether);
        assertEq(principal, 60 ether);
        assertEq(staking.getVotes(alice), 60 ether);
        assertEq(staking.totalUnbonding(), 40 ether);
        IStaking.UnbondingEntry[] memory e = staking.unbondingEntries(alice);
        assertEq(e.length, 1);
        assertEq(e[0].validator, v1);
        assertEq(e[0].completionTime, expected);
        (uint256 total, uint256 withdrawable) = staking.unbondingValue(alice);
        assertEq(total, 40 ether);
        assertEq(withdrawable, 0);

        vm.prank(alice);
        vm.expectRevert(Staking.NothingToWithdraw.selector);
        staking.withdrawUnbonded();

        vm.warp(block.timestamp + 1 days);
        (, withdrawable) = staking.unbondingValue(alice);
        assertEq(withdrawable, 40 ether);
        vm.expectEmit(address(staking));
        emit IStaking.Withdrawn(alice, 40 ether);
        vm.prank(alice);
        staking.withdrawUnbonded();
        assertEq(alice.balance, 40 ether);
        assertEq(staking.totalUnbonding(), 0);
        assertEq(staking.unbondingEntries(alice).length, 0);
    }

    function test_undelegate_max_and_mixedMaturity() public {
        _delegate(alice, v1, 100 ether);
        _delegate(alice, v2, 50 ether);
        vm.prank(alice);
        staking.undelegate(v1, type(uint256).max);
        vm.warp(block.timestamp + 12 hours);
        vm.prank(alice);
        staking.undelegate(v2, 50 ether);
        (uint256 shares,,) = staking.delegationOf(v1, alice);
        assertEq(shares, 0);
        assertEq(staking.getVotes(alice), 0);

        vm.warp(block.timestamp + 12 hours); // first matured, second not
        vm.prank(alice);
        staking.withdrawUnbonded();
        assertEq(alice.balance, 100 ether);
        assertEq(staking.unbondingEntries(alice).length, 1);
        vm.warp(block.timestamp + 12 hours);
        vm.prank(alice);
        staking.withdrawUnbonded();
        assertEq(alice.balance, 150 ether);
    }

    function test_undelegate_reverts() public {
        vm.startPrank(alice);
        vm.expectRevert(Staking.ZeroAmount.selector);
        staking.undelegate(v1, 0);
        vm.expectRevert(Staking.UnknownValidator.selector);
        staking.undelegate(bob, 1);
        vm.expectRevert(Staking.InsufficientDelegation.selector);
        staking.undelegate(v1, 1);
        vm.stopPrank();

        _delegate(alice, v1, 10 ether);
        vm.prank(alice);
        vm.expectRevert(Staking.InsufficientDelegation.selector);
        staking.undelegate(v1, 10 ether + 1);
    }

    function test_undelegate_tooManyEntries() public {
        _delegate(alice, v1, 100 ether);
        vm.startPrank(alice);
        for (uint256 i; i < staking.MAX_UNBONDING_ENTRIES(); ++i) {
            staking.undelegate(v1, 1 ether);
        }
        vm.expectRevert(Staking.TooManyUnbondingEntries.selector);
        staking.undelegate(v1, 1 ether);
        vm.stopPrank();
    }

    function test_undelegate_operatorPartialBelowMinReverts() public {
        _register(alice, MIN_SELF + 5 ether, 0);
        vm.startPrank(alice);
        staking.undelegate(alice, 5 ether); // exactly at minimum: fine
        vm.expectRevert(Staking.SelfStakeBelowMinimum.selector);
        staking.undelegate(alice, 1);
        vm.stopPrank();
    }

    function test_undelegate_operatorFullExit() public {
        _register(alice, MIN_SELF, 0);
        _delegate(bob, alice, 10 ether);
        _epoch();
        assertTrue(_activeContains(alice));
        uint64 versionBefore = staking.setVersion();

        vm.expectEmit(address(staking));
        emit IStaking.ValidatorExited(alice);
        vm.prank(alice);
        staking.undelegate(alice, type(uint256).max);

        assertEq(uint8(_status(alice)), uint8(IStaking.ValidatorStatus.Exited));
        assertFalse(_activeContains(alice));
        assertEq(staking.setVersion(), versionBefore + 1);
        assertEq(staking.candidates().length, 4);

        // Delegators can still leave, nobody can join, and re-registering is impossible.
        vm.prank(bob);
        staking.undelegate(alice, type(uint256).max);
        vm.deal(carol, 1 ether);
        vm.prank(carol);
        vm.expectRevert(Staking.ValidatorNotAcceptingStake.selector);
        staking.delegate{value: 1 ether}(alice);
        vm.deal(alice, MIN_SELF);
        vm.prank(alice);
        vm.expectRevert(Staking.AlreadyRegistered.selector);
        staking.register{value: MIN_SELF}(bytes32(uint256(77)), 0);
        vm.prank(alice);
        vm.expectRevert(Staking.UnknownValidator.selector);
        staking.setCommission(0);
        // Exited operator may still undelegate leftovers without restrictions (nothing left here).
        assertEq(staking.getValidator(alice).totalStake, 0);
    }

    function test_undelegate_candidateExitNotInSet() public {
        _register(alice, MIN_SELF, 0);
        vm.prank(alice);
        staking.undelegate(alice, MIN_SELF);
        assertEq(uint8(_status(alice)), uint8(IStaking.ValidatorStatus.Exited));
    }

    function test_undelegate_tombstonedOperatorNoRestrictions() public {
        _slashDoubleSign(v0);
        vm.prank(v0);
        staking.undelegate(v0, 1 ether);
        assertEq(uint8(_status(v0)), uint8(IStaking.ValidatorStatus.Tombstoned));
    }

    function test_undelegate_ceilRoundingAfterSlash() public {
        _delegate(alice, v3, 3 ether);
        _jailByDowntime(v3);
        vm.prank(alice);
        staking.undelegate(v3, 1 ether);
        (, uint256 amount,) = staking.delegationOf(v3, alice);
        assertLe(amount, 3 ether * 9_999 / 10_000 - 1 ether + 1);
    }

    function test_withdraw_transferFailed() public {
        Rejecter r = new Rejecter();
        vm.deal(address(r), 10 ether);
        r.call(address(staking), 10 ether, abi.encodeCall(staking.delegate, (v1)));
        r.call(address(staking), 0, abi.encodeCall(staking.undelegate, (v1, 10 ether)));
        vm.warp(block.timestamp + 1 days);
        vm.expectRevert(Staking.TransferFailed.selector);
        r.call(address(staking), 0, abi.encodeCall(staking.withdrawUnbonded, ()));
    }

    // ------------------------------------------------------------------ rewards

    function test_rewards_distribution() public {
        _delegate(alice, v0, GENESIS_STAKE); // after genesis power snapshot: does not change power until epoch
        uint256 reward = 100 ether;
        vm.expectEmit(address(staking));
        emit IStaking.RewardsDistributed(block.number + 1, v0, reward);
        _participate(v0, _allSigned(), reward);

        // v0: bonus 5 + 95/4 = 28.75; commission 10% -> 2.875; delegators share 25.875 by shares (50/50)
        uint256 v0Total = 5 ether + 95 ether / 4;
        uint256 commission = v0Total / 10;
        uint256 half = (v0Total - commission) / 2;
        assertApproxEqAbs(staking.pendingRewards(v0, alice), half, 10);
        assertApproxEqAbs(staking.pendingRewards(v0, v0), half + commission, 10);
        uint256 other = 95 ether / 4;
        assertApproxEqAbs(staking.pendingRewards(v1, v1), other, 10);
        assertEq(staking.totalRewardsReserved(), reward);
        assertEq(staking.accountedBalance(), address(staking).balance);

        uint256 before = v0.balance;
        uint256 pending = staking.pendingRewards(v0, v0);
        vm.expectEmit(address(staking));
        emit IStaking.RewardsClaimed(v0, v0, pending);
        vm.prank(v0);
        staking.claimRewards(v0);
        assertApproxEqAbs(v0.balance - before, half + commission, 10);
        assertEq(staking.getValidator(v0).commissionAccrued, 0);

        vm.prank(alice);
        staking.claimRewards(v0);
        assertApproxEqAbs(alice.balance, half, 10);

        vm.prank(alice);
        vm.expectRevert(Staking.NothingToClaim.selector);
        staking.claimRewards(v0);
    }

    function test_rewards_partialSignersAndUnknownProposer() public {
        // only v1 and v2 sign (bits 1,2), proposer is not a validator -> no bonus
        _participate(alice, (1 << 1) | (1 << 2), 10 ether);
        assertApproxEqAbs(staking.pendingRewards(v1, v1), 5 ether, 10);
        assertApproxEqAbs(staking.pendingRewards(v2, v2), 5 ether, 10);
        assertEq(staking.pendingRewards(v0, v0), 0);
    }

    function test_rewards_noSignersCarryOver() public {
        _participate(v0, 0, 10 ether);
        assertEq(staking.totalRewardsReserved(), 0);
        _participate(v0, _allSigned(), 0);
        assertEq(staking.totalRewardsReserved(), 10 ether);
    }

    function test_rewards_viaMsgValue() public {
        uint256 bitmap = _allSigned();
        _nextBlock();
        vm.deal(SYS, 8 ether);
        vm.prank(SYS);
        staking.recordParticipation{value: 8 ether}(v0, bitmap);
        assertEq(staking.totalRewardsReserved(), 8 ether);
    }

    function test_rewards_tombstonedExcluded() public {
        _slashDoubleSign(v0);
        // v0 is removed now, but the next block's commit is still signed by the pre-slash set (incl. v0)
        vm.roll(block.number + 1);
        (address[] memory sOps,) = staking.signingSet();
        assertEq(sOps.length, 4);
        _participate(v0, (uint256(1) << 4) - 1, 30 ether);
        assertEq(staking.pendingRewards(v0, v0), 0);
        assertApproxEqAbs(staking.pendingRewards(v1, v1), 10 ether, 10);
    }

    function test_rewards_claimOnUnknownValidatorReverts() public {
        vm.expectRevert(Staking.UnknownValidator.selector);
        staking.claimRewards(alice);
    }

    function test_rewards_claimTransferFailed() public {
        Rejecter r = new Rejecter();
        vm.deal(address(r), 10 ether);
        r.call(address(staking), 10 ether, abi.encodeCall(staking.delegate, (v1)));
        _participate(v1, _allSigned(), 100 ether);
        vm.expectRevert(Staking.TransferFailed.selector);
        r.call(address(staking), 0, abi.encodeCall(staking.claimRewards, (v1)));
    }

    function test_rewards_settledAcrossShareChanges() public {
        _delegate(alice, v1, 100 ether);
        _participate(v1, _allSigned(), 40 ether);
        uint256 p1 = staking.pendingRewards(v1, alice);
        _delegate(alice, v1, 100 ether); // settles
        assertEq(staking.pendingRewards(v1, alice), p1);
        vm.prank(alice);
        staking.undelegate(v1, 50 ether);
        assertEq(staking.pendingRewards(v1, alice), p1);
    }

    // ------------------------------------------------------------------ commission

    function test_setCommission() public {
        vm.prank(v0);
        vm.expectRevert(Staking.CommissionChangeTooSoon.selector);
        staking.setCommission(1_050);

        vm.warp(block.timestamp + 1 days);
        vm.prank(v0);
        vm.expectRevert(Staking.CommissionChangeTooLarge.selector);
        staking.setCommission(1_101);
        vm.prank(v0);
        vm.expectRevert(Staking.CommissionChangeTooLarge.selector);
        staking.setCommission(899);

        vm.expectEmit(address(staking));
        emit IStaking.CommissionChanged(v0, 1_000, 1_100);
        vm.prank(v0);
        staking.setCommission(1_100);
        assertEq(staking.getValidator(v0).commissionBps, 1_100);

        vm.prank(v0);
        vm.expectRevert(Staking.CommissionChangeTooSoon.selector);
        staking.setCommission(1_000);

        vm.warp(block.timestamp + 1 days);
        vm.prank(v0);
        staking.setCommission(1_000); // decrease
    }

    function test_setCommission_reverts() public {
        vm.prank(alice);
        vm.expectRevert(Staking.UnknownValidator.selector);
        staking.setCommission(0);
        _slashDoubleSign(v0);
        vm.prank(v0);
        vm.expectRevert(Staking.UnknownValidator.selector);
        staking.setCommission(0);

        IStaking.Params memory p = _defaultStakingParams();
        p.maxCommissionBps = 1_000;
        vm.prank(address(gov));
        staking.setParams(p);
        vm.warp(block.timestamp + 1 days);
        vm.prank(v1);
        vm.expectRevert(Staking.CommissionTooHigh.selector);
        staking.setCommission(1_001);
    }

    // ------------------------------------------------------------------ liveness / jail

    function test_downtime_jailAndSlash() public {
        uint256 stake = staking.getValidator(v3).totalStake;
        uint64 versionBefore = staking.setVersion();
        _jailByDowntime(v3);
        IStaking.ValidatorInfo memory info = staking.getValidator(v3);
        assertEq(info.totalStake, stake - stake / 10_000);
        assertEq(info.jailedUntil, block.timestamp + 10 minutes);
        assertEq(staking.totalSlashed(), stake / 10_000);
        assertFalse(_activeContains(v3));
        assertEq(staking.setVersion(), versionBefore + 1);
        assertEq(staking.windowId(), 2);

        // Jailed validators are not eligible at the next epoch.
        _epoch();
        assertFalse(_activeContains(v3));

        vm.prank(v3);
        vm.expectRevert(Staking.JailPeriodNotOver.selector);
        staking.unjail();
        vm.warp(block.timestamp + 10 minutes);
        vm.expectEmit(address(staking));
        emit IStaking.Unjailed(v3);
        vm.prank(v3);
        staking.unjail();
        assertEq(uint8(_status(v3)), uint8(IStaking.ValidatorStatus.Candidate));
        _epoch();
        assertTrue(_activeContains(v3));
    }

    function test_downtime_signedEnoughNotJailed() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.downtimeWindow = 4;
        vm.prank(address(gov));
        staking.setParams(p);
        // v3 (index 3) signs exactly 2 of 4 blocks = 50%: not jailed
        _participate(v0, 0x7, 0);
        _participate(v0, 0x7, 0);
        _participate(v0, 0xF, 0);
        _participate(v0, 0xF, 0);
        assertEq(uint8(_status(v3)), uint8(IStaking.ValidatorStatus.Active));
    }

    function test_downtime_lastValidatorNeverJailed() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.downtimeWindow = 2;
        vm.prank(address(gov));
        staking.setParams(p);
        _participate(v0, 0, 0);
        _participate(v0, 0, 0); // everyone offline: three jailed, last one kept
        (address[] memory ops,) = staking.activeValidators();
        assertEq(ops.length, 1);
        assertEq(ops[0], v3); // evaluated in set order: v0, v1, v2 jailed, v3 kept
    }

    function test_unjail_reverts() public {
        vm.prank(v0);
        vm.expectRevert(Staking.NotJailed.selector);
        staking.unjail();
    }

    function test_recordParticipation_reverts() public {
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.recordParticipation(v0, 0);
        _participate(v0, 0, 0);
        vm.prank(SYS);
        vm.expectRevert(Staking.AlreadyCalledThisBlock.selector);
        staking.recordParticipation(v0, 0);
    }

    function test_recordParticipation_usesPreviousSetAfterChange() public {
        _register(alice, 100_000_000 ether, 0);
        _epoch(); // alice ranks first from next block on
        (address[] memory ops,) = staking.activeValidators();
        assertEq(ops[0], alice);
        vm.roll(block.number + 1);
        (address[] memory sOps,) = staking.signingSet();
        assertEq(sOps.length, 4); // the epoch block itself was signed by the old set
        assertEq(sOps[0], v0);
        vm.roll(block.number - 1);
        // bit 0 refers to the OLD index 0 (v0), not alice
        _participate(v0, 1, 10 ether);
        assertEq(staking.pendingRewards(alice, alice), 0);
        assertGt(staking.pendingRewards(v0, v0), 0);
        // one block later the new set signs
        _participate(v0, 1, 10 ether);
        assertGt(staking.pendingRewards(alice, alice), 0);
    }

    // ------------------------------------------------------------------ epochs

    function test_onEpoch_reverts() public {
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.onEpoch();
        _epoch();
        vm.prank(SYS);
        vm.expectRevert(Staking.AlreadyCalledThisBlock.selector);
        staking.onEpoch();
    }

    function test_onEpoch_topNAndDemotion() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.maxValidators = 4;
        vm.prank(address(gov));
        staking.setParams(p);
        _register(alice, GENESIS_STAKE + 2 ether, 0);
        _delegate(bob, v3, 1 ether);

        _nextBlock();
        vm.prank(SYS);
        vm.recordLogs();
        staking.onEpoch();
        assertEq(staking.currentEpoch(), 1);

        (address[] memory ops, uint256[] memory powers) = staking.activeValidators();
        assertEq(ops.length, 4);
        assertEq(ops[0], alice);
        assertEq(ops[1], v3);
        assertEq(powers[1], GENESIS_STAKE + 1 ether);
        // v0..v2 tie at GENESIS_STAKE: lowest addresses win, the highest one is dropped
        assertEq(ops[2], v0);
        assertEq(ops[3], v1);
        assertEq(uint8(_status(v2)), uint8(IStaking.ValidatorStatus.Candidate));
    }

    function test_onEpoch_emitsTransition() public {
        _nextBlock();
        bytes32 h = staking.validatorSetHash(); // unchanged set -> same hash
        vm.expectEmit(address(staking));
        emit IStaking.EpochTransition(1, h);
        vm.prank(SYS);
        staking.onEpoch();
    }

    function test_onEpoch_belowMinSelfStakeNotEligible() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.minSelfStake = GENESIS_STAKE + 1;
        vm.prank(address(gov));
        staking.setParams(p);
        _delegate(v1, v1, 1 ether);
        _epoch();
        (address[] memory ops,) = staking.activeValidators();
        assertEq(ops.length, 1);
        assertEq(ops[0], v1);
    }

    function test_onEpoch_keepsSetWhenNoneEligible() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.minSelfStake = GENESIS_STAKE * 2;
        vm.prank(address(gov));
        staking.setParams(p);
        bytes32 h = staking.validatorSetHash();
        _epoch();
        assertEq(staking.validatorSetHash(), h);
        (address[] memory ops,) = staking.activeValidators();
        assertEq(ops.length, 4);
    }

    // ------------------------------------------------------------------ slashing

    function test_slash_doubleSign() public {
        _delegate(alice, v0, 1_000 ether);
        vm.prank(alice);
        staking.undelegate(v0, 400 ether);
        uint256 bonded = staking.getValidator(v0).totalStake;

        vm.expectEmit(address(staking));
        emit IStaking.Slashed(v0, bonded * 500 / 10_000 + 400 ether * 500 / 10_000, 1);
        vm.expectEmit(address(staking));
        emit IStaking.Tombstoned(v0, 42, keccak256(abi.encode(v0)));
        _slashDoubleSign(v0);

        IStaking.ValidatorInfo memory info = staking.getValidator(v0);
        assertEq(uint8(info.status), uint8(IStaking.ValidatorStatus.Tombstoned));
        assertEq(info.totalStake, bonded - bonded * 500 / 10_000);
        assertEq(info.unbondingStake, 380 ether);
        assertEq(info.jailedUntil, type(uint64).max);
        assertFalse(_activeContains(v0));
        assertEq(staking.candidates().length, 3);

        // unbonding stake was slashed too
        vm.warp(block.timestamp + 1 days);
        vm.prank(alice);
        staking.withdrawUnbonded();
        assertEq(alice.balance, 380 ether);

        // never selected again, cannot unjail, key stays burned
        _epoch();
        assertFalse(_activeContains(v0));
        vm.prank(v0);
        vm.expectRevert(Staking.NotJailed.selector);
        staking.unjail();
        assertEq(staking.consensusKeyOwner(_key(v0)), v0);
    }

    function test_slash_exitedValidatorStillSlashable() public {
        _register(alice, MIN_SELF, 0);
        vm.prank(alice);
        staking.undelegate(alice, type(uint256).max);
        _slashDoubleSign(alice);
        assertEq(uint8(_status(alice)), uint8(IStaking.ValidatorStatus.Tombstoned));
        assertEq(staking.getValidator(alice).unbondingStake, MIN_SELF - MIN_SELF * 500 / 10_000);
    }

    function test_slash_ignoredEvidence() public {
        bytes memory ev = _evidence(v0, 1, bytes32(0));
        vm.startPrank(SYS);
        staking.slash(ev);

        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(ev), 1);
        staking.slash(ev); // duplicate

        bytes memory bad = hex"1234";
        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(bad), 2);
        staking.slash(bad);

        bad = abi.encode(uint256(1), type(uint256).max, uint256(1), bytes32(0));
        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(bad), 2);
        staking.slash(bad);

        bad = abi.encode(uint256(1), uint256(uint160(v1)), uint256(type(uint64).max) + 1, bytes32(0));
        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(bad), 2);
        staking.slash(bad);

        bad = abi.encode(uint256(2), v1, uint64(1), bytes32(0));
        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(bad), 3);
        staking.slash(bad);

        bad = _evidence(alice, 1, bytes32(0));
        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(bad), 4);
        staking.slash(bad);

        bad = _evidence(v0, 2, bytes32(0)); // different evidence, already tombstoned
        vm.expectEmit(address(staking));
        emit IStaking.EvidenceIgnored(keccak256(bad), 4);
        staking.slash(bad);
        vm.stopPrank();

        assertTrue(staking.evidenceProcessed(keccak256(ev)));
    }

    function test_slash_onlySystem() public {
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.slash(_evidence(v0, 1, bytes32(0)));
    }

    function test_selfStakeView() public view {
        assertEq(staking.selfStake(v0), GENESIS_STAKE);
        assertEq(staking.selfStake(alice), 0);
    }

    function test_withdraw_sharedPoolPartial() public {
        _delegate(alice, v1, 100 ether);
        _delegate(bob, v1, 100 ether);
        vm.prank(alice);
        staking.undelegate(v1, 30 ether);
        vm.prank(bob);
        staking.undelegate(v1, 50 ether);
        vm.prank(alice);
        staking.undelegate(v1, 20 ether);
        vm.warp(block.timestamp + 1 days);
        vm.prank(alice);
        staking.withdrawUnbonded();
        assertEq(alice.balance, 50 ether);
        vm.prank(bob);
        staking.withdrawUnbonded();
        assertEq(bob.balance, 50 ether);
        assertEq(staking.getValidator(v1).unbondingShares, 0);
    }

    function test_downtime_validatorJoiningAtWindowEndNotEvaluated() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.downtimeWindow = 3;
        vm.prank(address(gov));
        staking.setParams(p);
        _register(alice, 100 ether + MIN_SELF, 0);
        _participate(v0, 0xF, 0);
        _participate(v0, 0xF, 0);
        _epoch(); // alice joins; the next commit is still signed by the old set
        _participate(v0, 0xF, 0); // window closes: alice has no blocks in this window -> skipped
        assertEq(uint8(_status(alice)), uint8(IStaking.ValidatorStatus.Active));
        assertEq(staking.windowId(), 2);
    }

    // ------------------------------------------------------------------ raw storage layout (read by the node)

    function test_storageLayout_nodeSlots() public view {
        (address[] memory ops, bytes32[] memory keys, uint256[] memory powers) =
            staking.consensusValidatorSet();
        assertEq(uint256(vm.load(address(staking), bytes32(uint256(0)))), ops.length);
        assertEq(uint256(vm.load(address(staking), bytes32(uint256(1)))), keys.length);
        assertEq(uint256(vm.load(address(staking), bytes32(uint256(2)))), powers.length);
        bytes32 base0 = keccak256(abi.encode(uint256(0)));
        bytes32 base1 = keccak256(abi.encode(uint256(1)));
        bytes32 base2 = keccak256(abi.encode(uint256(2)));
        for (uint256 i; i < ops.length; ++i) {
            assertEq(
                address(uint160(uint256(vm.load(address(staking), bytes32(uint256(base0) + i))))), ops[i]
            );
            assertEq(vm.load(address(staking), bytes32(uint256(base1) + i)), keys[i]);
            assertEq(uint256(vm.load(address(staking), bytes32(uint256(base2) + i))), powers[i]);
        }
        uint256 slot3 = uint256(vm.load(address(staking), bytes32(uint256(3))));
        assertEq(uint64(slot3), staking.currentEpoch());
        assertEq(uint64(slot3 >> 64), staking.setVersion());
        assertEq(uint64(slot3 >> 128), staking.lastSetChangeBlock());
        assertEq(uint64(slot3 >> 192), staking.lastEpochBlock());
        assertEq(vm.load(address(staking), bytes32(uint256(4))), staking.validatorSetHash());
    }
}

/// @dev Genesis with a single validator.
contract StakingSingleValidatorTest is SystemFixture {
    address internal op;

    function setUp() public {
        IStaking.GenesisValidator[] memory gv = _genesisValidators(1, MIN_SELF);
        _deployStaking(gv, _defaultStakingParams(), MIN_SELF);
        op = gv[0].operator;
    }

    function test_undelegate_lastActiveValidatorCannotExit() public {
        vm.prank(op);
        vm.expectRevert(Staking.LastActiveValidator.selector);
        staking.undelegate(op, type(uint256).max);
    }

    function test_tombstoneLastValidator_refillsFromCandidates() public {
        address alice = makeAddr("alice");
        _register(alice, MIN_SELF, 0);
        _slashDoubleSign(op);
        (address[] memory ops,) = staking.activeValidators();
        assertEq(ops.length, 1);
        assertEq(ops[0], alice);
        assertEq(uint8(_status(alice)), uint8(IStaking.ValidatorStatus.Active));
    }

    function test_tombstoneLastValidator_noCandidatesEmptiesSet() public {
        _slashDoubleSign(op);
        (address[] memory ops,) = staking.activeValidators();
        assertEq(ops.length, 0);
        // onEpoch keeps the (empty) set when nobody is eligible
        _epoch();
        (ops,) = staking.activeValidators();
        assertEq(ops.length, 0);
    }

    function test_initialize_excessBalanceBecomesFirstReward() public {
        vm.deal(address(staking), address(staking).balance + 7 ether);
        _participate(op, 1, 0);
        assertEq(staking.pendingRewards(op, op), 7 ether);
    }
}

/// @dev Genesis with two validators at exactly the minimum self-stake.
contract StakingMinStakeTest is SystemFixture {
    function setUp() public {
        _deployStaking(_genesisValidators(2, MIN_SELF), _defaultStakingParams(), 2 * MIN_SELF);
    }

    function test_unjail_requiresSelfStake() public {
        (address[] memory ops,) = staking.activeValidators();
        address target = ops[1];
        _jailByDowntime(target);
        vm.warp(block.timestamp + 1 hours);
        vm.prank(target);
        vm.expectRevert(Staking.InsufficientSelfStake.selector);
        staking.unjail();
        _delegate(target, target, 1 ether);
        vm.prank(target);
        staking.unjail();
        assertEq(uint8(_status(target)), uint8(IStaking.ValidatorStatus.Candidate));
    }
}
