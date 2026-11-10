// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {SystemFixture} from "./utils/SystemFixture.sol";
import {StakingHandler} from "./handlers/StakingHandler.sol";
import {IStaking} from "../../src/system/interfaces/IStaking.sol";
import {Staking} from "../../src/system/Staking.sol";

contract StakingInvariantTest is SystemFixture {
    StakingHandler internal handler;

    function setUp() public {
        IStaking.Params memory p = _defaultStakingParams();
        p.maxValidators = 5; // small set so the top-N cut is exercised
        p.maxCandidates = 12;
        p.downtimeWindow = 5; // frequent liveness evaluation -> jails happen
        p.downtimeSlashBps = 100;
        _deployStaking(_genesisValidators(4, 1_000_000 ether), p, 4 * 1_000_000 ether);
        handler = new StakingHandler(staking, genesisOps);
        targetContract(address(handler));
    }

    /// accountedBalance == bonded + unbonding + owed rewards + slashed, and it is fully backed.
    function invariant_accounting() public view {
        assertEq(
            staking.accountedBalance(),
            staking.totalBonded() + staking.totalUnbonding() + staking.totalRewardsReserved()
                + staking.totalSlashed()
        );
        assertGe(address(staking).balance, staking.accountedBalance());
    }

    /// total staked == sum of all validator stakes; total unbonding == sum of all unbonding pools.
    function invariant_totalStakedEqualsSumOfStakes() public view {
        uint256 bonded;
        uint256 unbonding;
        for (uint256 i; i < handler.validatorsLength(); ++i) {
            IStaking.ValidatorInfo memory v = staking.getValidator(handler.validators(i));
            bonded += v.totalStake;
            unbonding += v.unbondingStake;
        }
        assertEq(staking.totalBonded(), bonded);
        assertEq(staking.totalUnbonding(), unbonding);
    }

    /// shares are fully accounted, delegations never exceed the validator stake, rewards are solvent,
    /// and voting power equals bonded principal.
    function invariant_sharesRewardsAndVotes() public view {
        uint256 n = handler.validatorsLength();
        uint256[] memory sharesSum = new uint256[](n);
        uint256[] memory valueSum = new uint256[](n);
        uint256 pending;
        uint256 principal;
        for (uint256 i; i < handler.pairsLength(); ++i) {
            address v = handler.pairValidator(i);
            address d = handler.pairDelegator(i);
            (uint256 shares, uint256 value, uint256 pr) = staking.delegationOf(v, d);
            principal += pr;
            pending += staking.pendingRewards(v, d);
            for (uint256 j; j < n; ++j) {
                if (handler.validators(j) == v) {
                    sharesSum[j] += shares;
                    valueSum[j] += value;
                    break;
                }
            }
        }
        for (uint256 j; j < n; ++j) {
            IStaking.ValidatorInfo memory info = staking.getValidator(handler.validators(j));
            assertEq(sharesSum[j], info.totalShares, "shares");
            assertLe(valueSum[j], info.totalStake, "value");
        }
        assertLe(pending, staking.totalRewardsReserved(), "rewards solvent");
        uint256 votes;
        for (uint256 i; i < handler.actorsLength(); ++i) {
            votes += staking.getVotes(handler.actors(i));
        }
        assertEq(votes, principal, "votes == principal");
    }

    /// A tombstoned validator is never active again, never a candidate, and stays tombstoned.
    function invariant_tombstonedNeverActive() public view {
        address[] memory cands = staking.candidates();
        (address[] memory ops,) = staking.activeValidators();
        for (uint256 i; i < handler.tombstonedLength(); ++i) {
            address t = handler.tombstoned(i);
            assertEq(uint8(staking.getValidator(t).status), uint8(IStaking.ValidatorStatus.Tombstoned));
            for (uint256 j; j < ops.length; ++j) {
                assertTrue(ops[j] != t, "tombstoned in active set");
            }
            for (uint256 j; j < cands.length; ++j) {
                assertTrue(cands[j] != t, "tombstoned candidate");
            }
        }
    }

    /// The active set is bounded, strictly sorted (power desc, address asc), consistent with statuses and hash.
    function invariant_activeSetBoundedAndSorted() public view {
        (address[] memory ops, bytes32[] memory keys, uint256[] memory powers) =
            staking.consensusValidatorSet();
        assertLe(ops.length, staking.params().maxValidators);
        assertFalse(handler.ghostEmptiedWithEligibleCandidate(), "emptied with eligible candidate");
        for (uint256 i = 1; i < ops.length; ++i) {
            assertTrue(
                powers[i - 1] > powers[i] || (powers[i - 1] == powers[i] && ops[i - 1] < ops[i]), "sorted"
            );
        }
        uint256 activeCount;
        for (uint256 i; i < handler.validatorsLength(); ++i) {
            if (staking.getValidator(handler.validators(i)).status == IStaking.ValidatorStatus.Active) {
                ++activeCount;
            }
        }
        assertEq(activeCount, ops.length, "status Active <=> in set");
        for (uint256 i; i < ops.length; ++i) {
            assertEq(uint8(staking.getValidator(ops[i]).status), uint8(IStaking.ValidatorStatus.Active));
            assertEq(staking.getValidator(ops[i]).consensusKey, keys[i]);
        }
        assertEq(keccak256(abi.encode(ops, keys, powers)), staking.validatorSetHash());
    }

    /// Only SYSTEM_ADDRESS can call onEpoch / recordParticipation / slash.
    function invariant_onlySystem() public view {
        assertFalse(handler.ghostNonSystemCallSucceeded());
    }

    /// @dev Runs after EVERY invariant run (Foundry semantics), so it must hold for any single random sequence.
    ///      It used to assert that the random sequence happened to include `participate` or `epoch`; with 12
    ///      handler selectors and depth 32 that is false for ~0.3% of runs (~17% of 64-run campaigns). Foundry
    ///      reported it against the first invariant (`invariant_accounting`, "0 <= 0" = assertGt(0, 0)), shrank
    ///      it to an empty sequence, persisted it in cache/invariant/failures and replayed it on every later seed.
    ///      Instead every run now ends with a settlement: the system paths run deterministically, all unbonding
    ///      matures and every party takes out its liquid funds. Checked end-to-end: every withdrawal and claim
    ///      succeeds (only "nothing to do" reverts are allowed), the unbonding pools empty completely, nothing
    ///      is left claimable, and the accounting identity and backing still hold afterwards.
    function afterInvariant() external {
        handler.participate(0, type(uint256).max, 100 ether);
        handler.epoch();
        vm.warp(block.timestamp + staking.params().unbondingSeconds + 1);

        for (uint256 i; i < handler.actorsLength(); ++i) {
            address a = handler.actors(i);
            vm.prank(a);
            try staking.withdrawUnbonded() {}
            catch (bytes memory err) {
                assertEq(bytes4(err), Staking.NothingToWithdraw.selector, "withdraw reverted");
            }
            assertEq(staking.unbondingEntries(a).length, 0, "matured entry left");
        }
        for (uint256 i; i < handler.pairsLength(); ++i) {
            address v = handler.pairValidator(i);
            address d = handler.pairDelegator(i);
            vm.prank(d);
            try staking.claimRewards(v) {}
            catch (bytes memory err) {
                assertEq(bytes4(err), Staking.NothingToClaim.selector, "claim reverted");
            }
            assertEq(staking.pendingRewards(v, d), 0, "rewards left after claim");
        }

        assertEq(staking.totalUnbonding(), 0, "unbonding left");
        for (uint256 i; i < handler.validatorsLength(); ++i) {
            IStaking.ValidatorInfo memory info = staking.getValidator(handler.validators(i));
            assertEq(info.unbondingStake, 0, "pool stake left");
            assertEq(info.unbondingShares, 0, "pool shares left");
            assertEq(info.commissionAccrued, 0, "commission left");
        }
        invariant_accounting();
        assertGe(address(staking).balance, staking.totalBonded() + staking.totalSlashed(), "bonded backed");
    }
}
