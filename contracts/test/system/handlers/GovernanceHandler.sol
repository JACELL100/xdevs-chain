// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Governance} from "../../../src/system/Governance.sol";
import {IGovernance} from "../../../src/system/interfaces/IGovernance.sol";

/// @dev Target of every proposal: records when (and how often) each proposal's action actually ran.
contract CallRecorder {
    mapping(uint256 => uint256) public hits;
    mapping(uint256 => uint256) public hitAt;

    function hit(uint256 id) external {
        hits[id]++;
        hitAt[id] = block.timestamp;
    }
}

contract GovernanceHandler is Test {
    Governance internal immutable gov;
    CallRecorder public immutable recorder;
    address[] internal voters;
    address internal guardian;

    uint256[] public ids;
    uint256 public executedCount;
    uint256 public queuedCount;

    constructor(Governance gov_, address[] memory voters_, address guardian_) {
        gov = gov_;
        recorder = new CallRecorder();
        voters = voters_;
        guardian = guardian_;
    }

    function idsLength() external view returns (uint256) {
        return ids.length;
    }

    function _id(uint256 seed) internal view returns (uint256) {
        return ids.length == 0 ? 0 : ids[seed % ids.length];
    }

    function propose(uint256 voterSeed) external {
        vm.roll(block.number + 1);
        address[] memory t = new address[](1);
        uint256[] memory v = new uint256[](1);
        bytes[] memory d = new bytes[](1);
        t[0] = address(recorder);
        uint256 next = gov.proposalCount() + 1;
        d[0] = abi.encodeCall(CallRecorder.hit, (next));
        vm.prank(voters[voterSeed % voters.length]);
        try gov.propose(t, v, d, "") returns (uint256 id) {
            ids.push(id);
        } catch {}
    }

    function vote(uint256 idSeed, uint256 voterSeed, uint8 support) external {
        uint256 id = _id(idSeed);
        if (id == 0) return;
        vm.prank(voters[voterSeed % voters.length]);
        // biased towards For so that proposals regularly pass and reach queue/execute
        uint8 choice = support % 4 == 0 ? 0 : (support % 4 == 1 ? 2 : 1);
        try gov.castVote(id, choice) {} catch {}
    }

    function queue(uint256 idSeed) external {
        uint256 id = _id(idSeed);
        if (id == 0) return;
        try gov.queue(id) {
            queuedCount++;
        } catch {}
    }

    function execute(uint256 idSeed) external {
        uint256 id = _id(idSeed);
        if (id == 0) return;
        try gov.execute(id) {
            executedCount++;
        } catch {}
    }

    function cancel(uint256 idSeed, uint256 voterSeed) external {
        uint256 id = _id(idSeed);
        if (id == 0) return;
        vm.prank(voters[voterSeed % voters.length]);
        try gov.cancel(id) {} catch {}
    }

    function pause() external {
        vm.prank(guardian);
        try gov.pause() {} catch {}
    }

    function warp(uint256 dt) external {
        vm.warp(block.timestamp + bound(dt, 1 hours, 2 days));
        vm.roll(block.number + 1);
    }
}
