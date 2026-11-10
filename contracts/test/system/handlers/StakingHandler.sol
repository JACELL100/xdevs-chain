// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Predeploys} from "../../../src/common/Predeploys.sol";
import {Staking} from "../../../src/system/Staking.sol";
import {IStaking} from "../../../src/system/interfaces/IStaking.sol";

/// @dev Drives Staking with random but valid-ish sequences of user and system actions.
contract StakingHandler is Test {
    Staking internal immutable staking;
    address internal constant SYS = Predeploys.SYSTEM_ADDRESS;

    address[] public actors;
    address[] public validators; // every validator ever registered (incl. genesis)
    address[] public tombstoned;
    mapping(address => bool) public isValidator;
    // (validator, delegator) pairs that ever held shares
    address[] public pairValidator;
    address[] public pairDelegator;
    mapping(bytes32 => bool) internal _pairSeen;

    bool public ghostNonSystemCallSucceeded;
    bool public ghostEmptiedWithEligibleCandidate;
    uint256 public ghostEvidenceCounter;
    mapping(bytes32 => uint256) public calls;

    constructor(Staking staking_, address[] memory genesisOps) {
        staking = staking_;
        for (uint256 i; i < genesisOps.length; ++i) {
            _trackValidator(genesisOps[i]);
            _trackPair(genesisOps[i], genesisOps[i]);
            actors.push(genesisOps[i]);
        }
        for (uint256 i; i < 6; ++i) {
            actors.push(makeAddr(string.concat("actor", vm.toString(i))));
        }
    }

    // ------------------------------------------------------------------ bookkeeping

    function _trackValidator(address v) internal {
        if (isValidator[v]) return;
        isValidator[v] = true;
        validators.push(v);
    }

    function _trackPair(address v, address d) internal {
        bytes32 k = keccak256(abi.encode(v, d));
        if (_pairSeen[k]) return;
        _pairSeen[k] = true;
        pairValidator.push(v);
        pairDelegator.push(d);
    }

    function validatorsLength() external view returns (uint256) {
        return validators.length;
    }

    function pairsLength() external view returns (uint256) {
        return pairValidator.length;
    }

    function tombstonedLength() external view returns (uint256) {
        return tombstoned.length;
    }

    function actorsLength() external view returns (uint256) {
        return actors.length;
    }

    function _actor(uint256 seed) internal view returns (address) {
        return actors[seed % actors.length];
    }

    function _validator(uint256 seed) internal view returns (address) {
        return validators[seed % validators.length];
    }

    function _nextBlock() internal {
        vm.roll(block.number + 1);
        vm.warp(block.timestamp + 2);
    }

    // ------------------------------------------------------------------ user actions

    function register(uint256 actorSeed, uint256 amount, uint16 commission) external {
        calls["register"]++;
        address a = _actor(actorSeed);
        amount = bound(amount, 10_000 ether, 5_000_000 ether);
        commission = uint16(bound(commission, 0, 2_000));
        vm.deal(a, a.balance + amount);
        vm.prank(a);
        try staking.register{value: amount}(keccak256(abi.encode(a, "key")), commission) {
            _trackValidator(a);
            _trackPair(a, a);
        } catch {}
    }

    function delegate(uint256 actorSeed, uint256 valSeed, uint256 amount) external {
        calls["delegate"]++;
        address a = _actor(actorSeed);
        address v = _validator(valSeed);
        amount = bound(amount, 1 ether, 2_000_000 ether);
        vm.deal(a, a.balance + amount);
        vm.prank(a);
        try staking.delegate{value: amount}(v) {
            _trackPair(v, a);
        } catch {}
    }

    function undelegate(uint256 actorSeed, uint256 valSeed, uint256 amount, bool all) external {
        calls["undelegate"]++;
        address a = _actor(actorSeed);
        address v = _validator(valSeed);
        (, uint256 value,) = staking.delegationOf(v, a);
        if (value == 0) return;
        amount = all ? type(uint256).max : bound(amount, 1, value);
        vm.prank(a);
        try staking.undelegate(v, amount) {} catch {}
    }

    function withdraw(uint256 actorSeed) external {
        calls["withdraw"]++;
        address a = _actor(actorSeed);
        vm.prank(a);
        try staking.withdrawUnbonded() {} catch {}
    }

    function claim(uint256 actorSeed, uint256 valSeed) external {
        calls["claim"]++;
        address a = _actor(actorSeed);
        vm.prank(a);
        try staking.claimRewards(_validator(valSeed)) {} catch {}
    }

    function unjail(uint256 valSeed) external {
        calls["unjail"]++;
        address v = _validator(valSeed);
        vm.warp(block.timestamp + 11 minutes);
        vm.prank(v);
        try staking.unjail() {} catch {}
    }

    function setCommission(uint256 valSeed, uint16 bps) external {
        calls["setCommission"]++;
        vm.warp(block.timestamp + 1 days);
        vm.prank(_validator(valSeed));
        try staking.setCommission(uint16(bound(bps, 0, 2_000))) {} catch {}
    }

    function warp(uint256 dt) external {
        calls["warp"]++;
        vm.warp(block.timestamp + bound(dt, 1, 2 days));
    }

    // ------------------------------------------------------------------ system actions

    function participate(uint256 proposerSeed, uint256 bitmap, uint256 reward) external {
        calls["participate"]++;
        _nextBlock();
        reward = bound(reward, 0, 1_000 ether);
        vm.deal(address(staking), address(staking).balance + reward);
        vm.prank(SYS);
        staking.recordParticipation(_validator(proposerSeed), bitmap);
    }

    function epoch() external {
        calls["epoch"]++;
        _nextBlock();
        vm.prank(SYS);
        staking.onEpoch();
    }

    function slashDoubleSign(uint256 valSeed) external {
        calls["slash"]++;
        address v = _validator(valSeed);
        IStaking.ValidatorStatus before = staking.getValidator(v).status;
        (address[] memory opsBefore,) = staking.activeValidators();
        vm.prank(SYS);
        staking.slash(abi.encode(uint256(1), v, uint64(block.number), bytes32(++ghostEvidenceCounter)));
        if (before != IStaking.ValidatorStatus.Tombstoned && before != IStaking.ValidatorStatus.None) {
            tombstoned.push(v);
        }
        // The set may only BECOME empty (through this tombstone) if nobody is eligible to refill it.
        (address[] memory ops,) = staking.activeValidators();
        if (ops.length == 0 && opsBefore.length != 0) {
            address[] memory cands = staking.candidates();
            uint256 minSelf = staking.params().minSelfStake;
            for (uint256 i; i < cands.length; ++i) {
                IStaking.ValidatorInfo memory c = staking.getValidator(cands[i]);
                if (c.status == IStaking.ValidatorStatus.Candidate && c.selfStake >= minSelf) {
                    ghostEmptiedWithEligibleCandidate = true;
                }
            }
        }
    }

    /// @dev Any non-system caller must be rejected by every system entry point.
    function nonSystemCall(address caller, uint8 which, uint256 bitmap) external {
        calls["nonSystemCall"]++;
        if (caller == SYS) return;
        vm.prank(caller);
        bool ok;
        if (which % 3 == 0) {
            (ok,) = address(staking).call(abi.encodeCall(Staking.onEpoch, ()));
        } else if (which % 3 == 1) {
            (ok,) = address(staking).call(abi.encodeCall(Staking.recordParticipation, (caller, bitmap)));
        } else {
            (ok,) = address(staking)
                .call(
                    abi.encodeCall(
                        Staking.slash, (abi.encode(uint256(1), _validator(bitmap), uint64(1), bytes32(0)))
                    )
                );
        }
        if (ok) ghostNonSystemCallSucceeded = true;
    }
}
