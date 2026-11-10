// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {SystemFixture} from "./utils/SystemFixture.sol";
import {Staking} from "../../src/system/Staking.sol";
import {IStaking} from "../../src/system/interfaces/IStaking.sol";

contract StakingFuzzTest is SystemFixture {
    address internal alice = makeAddr("alice");

    function setUp() public {
        _deployGenesis();
    }

    function testFuzz_onlySystem(address caller) public {
        vm.assume(caller != SYS);
        vm.startPrank(caller);
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.onEpoch();
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.recordParticipation(caller, type(uint256).max);
        vm.expectRevert(Staking.OnlySystem.selector);
        staking.slash(_evidence(genesisOps[0], 1, bytes32(0)));
        vm.stopPrank();
    }

    function testFuzz_delegateUndelegateRoundTrip(uint256 amount, uint256 out, uint8 valIdx) public {
        amount = bound(amount, 1 ether, 1_000_000_000 ether);
        out = bound(out, 1, amount);
        address v = genesisOps[valIdx % 4];
        _delegate(alice, v, amount);
        vm.prank(alice);
        staking.undelegate(v, out);
        (, uint256 remaining,) = staking.delegationOf(v, alice);
        assertEq(remaining, amount - out);
        vm.warp(block.timestamp + 1 days);
        vm.prank(alice);
        staking.withdrawUnbonded();
        assertEq(alice.balance, out);
        assertEq(staking.accountedBalance(), address(staking).balance);
    }

    function testFuzz_rewardsConserved(uint256 reward, uint256 bitmap, uint8 proposerIdx, uint256 extra)
        public
    {
        reward = bound(reward, 0, 10_000_000 ether);
        extra = bound(extra, 1 ether, 50_000_000 ether);
        _delegate(alice, genesisOps[1], extra);
        address proposer = proposerIdx < 4 ? genesisOps[proposerIdx] : alice;
        uint256 balBefore = address(staking).balance;
        _participate(proposer, bitmap, reward);

        uint256 distributed = staking.totalRewardsReserved();
        assertLe(distributed, reward);
        if ((bitmap & 0xF) != 0) assertGe(distributed + 10, reward, "only dust may remain");
        assertEq(address(staking).balance, balBefore + reward);
        uint256 pending = staking.pendingRewards(genesisOps[1], alice);
        for (uint256 i; i < 4; ++i) {
            pending += staking.pendingRewards(genesisOps[i], genesisOps[i]);
        }
        assertLe(pending, distributed);
    }

    function testFuzz_topNSortedAndComplete(uint256[12] memory stakes, uint8 maxVals) public {
        IStaking.Params memory p = _defaultStakingParams();
        p.maxValidators = uint16(bound(maxVals, 1, 16));
        vm.prank(address(gov));
        staking.setParams(p);
        for (uint256 i; i < stakes.length; ++i) {
            // narrow range to get frequent ties
            stakes[i] = MIN_SELF + bound(stakes[i], 0, 3) * 1 ether;
            _register(makeAddr(string.concat("cand", vm.toString(i))), stakes[i], 0);
        }
        _epoch();
        (address[] memory ops, uint256[] memory powers) = staking.activeValidators();
        assertEq(ops.length, p.maxValidators < 16 ? p.maxValidators : 16);
        for (uint256 i = 1; i < ops.length; ++i) {
            assertTrue(
                powers[i - 1] > powers[i] || (powers[i - 1] == powers[i] && ops[i - 1] < ops[i]), "sorted"
            );
        }
        // every candidate not selected ranks after the last selected one
        address[] memory cands = staking.candidates();
        address lastOp = ops[ops.length - 1];
        uint256 lastPower = powers[powers.length - 1];
        for (uint256 i; i < cands.length; ++i) {
            if (_activeContains(cands[i])) {
                assertEq(uint8(_status(cands[i])), uint8(IStaking.ValidatorStatus.Active));
                continue;
            }
            uint256 s = staking.getValidator(cands[i]).totalStake;
            assertTrue(s < lastPower || (s == lastPower && cands[i] > lastOp), "complete");
            assertEq(uint8(_status(cands[i])), uint8(IStaking.ValidatorStatus.Candidate));
        }
    }

    function testFuzz_commissionChangeBounded(uint16 newBps, uint32 dt) public {
        address v = genesisOps[0];
        vm.warp(block.timestamp + dt);
        uint256 diff = newBps > 1_000 ? newBps - 1_000 : 1_000 - newBps;
        vm.prank(v);
        if (newBps > 2_000) {
            vm.expectRevert(Staking.CommissionTooHigh.selector);
        } else if (diff > 100) {
            vm.expectRevert(Staking.CommissionChangeTooLarge.selector);
        } else if (dt < 1 days) {
            vm.expectRevert(Staking.CommissionChangeTooSoon.selector);
        }
        staking.setCommission(newBps);
    }

    function testFuzz_doubleSignSlash(uint256 delegated, uint256 unbond, uint8 idx) public {
        delegated = bound(delegated, 1 ether, 100_000_000 ether);
        unbond = bound(unbond, 1, delegated);
        address v = genesisOps[idx % 4];
        _delegate(alice, v, delegated);
        vm.prank(alice);
        staking.undelegate(v, unbond);
        IStaking.ValidatorInfo memory before = staking.getValidator(v);
        _slashDoubleSign(v);
        IStaking.ValidatorInfo memory info = staking.getValidator(v);
        assertEq(info.totalStake, before.totalStake - before.totalStake * 500 / 10_000);
        assertEq(info.unbondingStake, before.unbondingStake - before.unbondingStake * 500 / 10_000);
        assertEq(
            staking.accountedBalance(),
            staking.totalBonded() + staking.totalUnbonding() + staking.totalRewardsReserved()
                + staking.totalSlashed()
        );
        assertFalse(_activeContains(v));
    }
}
