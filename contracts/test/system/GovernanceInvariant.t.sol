// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {SystemFixture} from "./utils/SystemFixture.sol";
import {GovernanceHandler, CallRecorder} from "./handlers/GovernanceHandler.sol";
import {IGovernance} from "../../src/system/interfaces/IGovernance.sol";

contract GovernanceInvariantTest is SystemFixture {
    GovernanceHandler internal handler;

    function setUp() public {
        _deployGenesis();
        (address[] memory ops,) = staking.activeValidators();
        address[] memory voters = new address[](ops.length + 1);
        for (uint256 i; i < ops.length; ++i) {
            voters[i] = ops[i];
        }
        voters[ops.length] = makeAddr("no-power");
        handler = new GovernanceHandler(gov, voters, guardian);
        targetContract(address(handler));
        vm.roll(block.number + 1);
    }

    /// forge-config: system.invariant.depth = 96
    /// forge-config: system.invariant.runs = 128
    /// A proposal's actions never run before its timelock expired, run at most once, and only for proposals
    /// that reached Executed through queue().
    function invariant_noExecutionBeforeTimelock() public view {
        CallRecorder rec = handler.recorder();
        for (uint256 i; i < handler.idsLength(); ++i) {
            uint256 id = handler.ids(i);
            uint256 hits = rec.hits(id);
            assertLe(hits, 1, "executed twice");
            (,, uint64 voteEnd, uint64 eta,,,,) = gov.getProposal(id);
            if (hits == 1) {
                assertGt(eta, 0, "executed without queue");
                assertGe(rec.hitAt(id), eta, "executed before timelock");
                assertGe(uint256(eta), uint256(voteEnd) + gov.params().timelockDelay, "eta too early");
                assertEq(uint8(gov.state(id)), uint8(IGovernance.ProposalState.Executed));
            } else {
                assertTrue(gov.state(id) != IGovernance.ProposalState.Executed, "Executed without effects");
            }
        }
    }

    /// Votes never exceed the voting power that existed at the snapshot.
    function invariant_votesBoundedBySnapshot() public view {
        for (uint256 i; i < handler.idsLength(); ++i) {
            uint256 id = handler.ids(i);
            (, uint256 snap,,,, uint256 f, uint256 a, uint256 ab) = gov.getProposal(id);
            assertLe(f + a + ab, staking.getPastTotalVotes(snap));
        }
    }
}
