// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Predeploys} from "../../../src/common/Predeploys.sol";
import {Staking} from "../../../src/system/Staking.sol";
import {Governance} from "../../../src/system/Governance.sol";
import {Treasury} from "../../../src/system/Treasury.sol";
import {FeeConfig} from "../../../src/system/FeeConfig.sol";
import {WXDS} from "../../../src/system/WXDS.sol";
import {IStaking} from "../../../src/system/interfaces/IStaking.sol";
import {IGovernance} from "../../../src/system/interfaces/IGovernance.sol";

/// @dev Recreates the genesis procedure: runtime code is placed at the predeploy address (no constructor), the
///      balance is set, then `initialize` runs as SYSTEM_ADDRESS.
abstract contract SystemFixture is Test {
    address internal constant SYS = Predeploys.SYSTEM_ADDRESS;

    Staking internal staking = Staking(Predeploys.STAKING);
    Governance internal gov = Governance(payable(Predeploys.GOVERNANCE));
    Treasury internal treasury = Treasury(payable(Predeploys.TREASURY));
    FeeConfig internal feeConfig = FeeConfig(Predeploys.FEE_CONFIG);
    WXDS internal wxds = WXDS(payable(Predeploys.WXDS));

    uint256 internal constant GENESIS_STAKE = 12_500_000 ether;
    uint256 internal constant MIN_SELF = 10_000 ether;
    uint256 internal constant TREASURY_GENESIS = 200_000_000 ether;

    address internal guardian = makeAddr("guardian");
    address[] internal genesisOps;

    function _etch(address at, string memory artifact) internal {
        vm.etch(at, vm.getDeployedCode(artifact));
    }

    function _defaultStakingParams() internal pure returns (IStaking.Params memory p) {
        p = IStaking.Params({
            minSelfStake: 10_000 ether,
            minDelegation: 1 ether,
            unbondingSeconds: 1 days,
            commissionChangeInterval: 1 days,
            downtimeWindow: 10_000,
            downtimeJailSeconds: 10 minutes,
            maxValidators: 100,
            maxCandidates: 1_000,
            maxCommissionBps: 2_000,
            maxCommissionChangeBps: 100,
            proposerBonusBps: 500,
            doubleSignSlashBps: 500,
            downtimeSlashBps: 1,
            minSignedBps: 5_000
        });
    }

    function _defaultGovParams() internal pure returns (IGovernance.Params memory p) {
        p = IGovernance.Params({
            votingPeriod: 3 days,
            timelockDelay: 1 days,
            gracePeriod: 14 days,
            pauseDuration: 3 days,
            quorumBps: 3_340,
            thresholdBps: 5_000,
            proposalThreshold: 10_000 ether
        });
    }

    function _defaultFeeParams() internal pure returns (FeeConfig.FeeParams memory p) {
        p = FeeConfig.FeeParams({
            blockGasLimit: 30_000_000,
            minBaseFee: 0.1 gwei,
            burnBps: 8_000,
            baseFeeMaxChangeDenominator: 8,
            elasticityMultiplier: 2,
            maxTxSizeBytes: 128 * 1024,
            localFeeEnabled: 0,
            localFeeThresholdBps: 2_500,
            localFeeMaxChangeDenominator: 8
        });
    }

    function _key(address op) internal pure returns (bytes32) {
        return keccak256(abi.encode("ed25519", op));
    }

    function _genesisValidators(uint256 n, uint256 stake)
        internal
        returns (IStaking.GenesisValidator[] memory gv)
    {
        gv = new IStaking.GenesisValidator[](n);
        delete genesisOps;
        for (uint256 i; i < n; ++i) {
            address op = makeAddr(string.concat("genesis-val-", vm.toString(i)));
            genesisOps.push(op);
            gv[i] = IStaking.GenesisValidator({
                operator: op, consensusKey: _key(op), commissionBps: 1_000, stake: stake
            });
        }
    }

    /// @dev Full genesis: all five predeploys, 4 validators x 12.5M tXDS.
    function _deployGenesis() internal {
        _deployStaking(_genesisValidators(4, GENESIS_STAKE), _defaultStakingParams(), 4 * GENESIS_STAKE);
        _deployRest();
    }

    function _deployStaking(IStaking.GenesisValidator[] memory gv, IStaking.Params memory p, uint256 balance)
        internal
    {
        _etch(address(staking), "Staking.sol:Staking");
        vm.deal(address(staking), balance);
        vm.prank(SYS);
        staking.initialize(address(gov), p, gv);
    }

    function _deployRest() internal {
        _etch(address(gov), "Governance.sol:Governance");
        _etch(address(treasury), "Treasury.sol:Treasury");
        _etch(address(feeConfig), "FeeConfig.sol:FeeConfig");
        _etch(address(wxds), "WXDS.sol:WXDS");
        vm.deal(address(treasury), TREASURY_GENESIS);
        vm.startPrank(SYS);
        gov.initialize(address(staking), guardian, _defaultGovParams());
        treasury.initialize(address(gov));
        feeConfig.initialize(address(gov), _defaultFeeParams());
        vm.stopPrank();
    }

    // ------------------------------------------------------------------ helpers

    function _register(address op, uint256 amount, uint16 commission) internal {
        vm.deal(op, op.balance + amount);
        vm.prank(op);
        staking.register{value: amount}(_key(op), commission);
    }

    function _delegate(address who, address val, uint256 amount) internal {
        vm.deal(who, who.balance + amount);
        vm.prank(who);
        staking.delegate{value: amount}(val);
    }

    function _nextBlock() internal {
        vm.roll(block.number + 1);
        vm.warp(block.timestamp + 2);
    }

    function _epoch() internal {
        _nextBlock();
        vm.prank(SYS);
        staking.onEpoch();
    }

    function _participate(address proposer, uint256 bitmap, uint256 reward) internal {
        _nextBlock();
        vm.deal(address(staking), address(staking).balance + reward);
        vm.prank(SYS);
        staking.recordParticipation(proposer, bitmap);
    }

    /// @dev Bitmap with every member of the NEXT block's signing set.
    function _allSigned() internal returns (uint256) {
        vm.roll(block.number + 1);
        (address[] memory ops,) = staking.signingSet();
        vm.roll(block.number - 1);
        return ops.length == 256 ? type(uint256).max : (uint256(1) << ops.length) - 1;
    }

    function _status(address v) internal view returns (IStaking.ValidatorStatus) {
        return staking.getValidator(v).status;
    }

    function _activeContains(address v) internal view returns (bool) {
        (address[] memory ops,) = staking.activeValidators();
        for (uint256 i; i < ops.length; ++i) {
            if (ops[i] == v) return true;
        }
        return false;
    }

    function _slashDoubleSign(address target) internal {
        vm.prank(SYS);
        staking.slash(_evidence(target, 42, keccak256(abi.encode(target))));
    }

    /// @dev Shrinks the liveness window to 10 blocks and lets `target` miss all of them.
    function _jailByDowntime(address target) internal {
        IStaking.Params memory p = staking.params();
        p.downtimeWindow = 10;
        vm.prank(address(gov));
        staking.setParams(p);
        for (uint256 b; b < 10; ++b) {
            vm.roll(block.number + 1);
            (address[] memory ops,) = staking.signingSet();
            vm.roll(block.number - 1);
            uint256 bitmap;
            for (uint256 i; i < ops.length; ++i) {
                if (ops[i] != target) bitmap |= uint256(1) << i;
            }
            _participate(ops[0] == target ? ops[1] : ops[0], bitmap, 0);
        }
        assertEq(uint8(_status(target)), uint8(IStaking.ValidatorStatus.Jailed), "jailed");
    }

    function _evidence(address v, uint64 height, bytes32 ref) internal pure returns (bytes memory) {
        return abi.encode(uint256(1), v, height, ref);
    }
}

/// @dev Contract that refuses native transfers (to test TransferFailed branches).
contract Rejecter {
    function call(address target, uint256 value, bytes calldata data)
        external
        payable
        returns (bytes memory)
    {
        (bool ok, bytes memory ret) = target.call{value: value}(data);
        if (!ok) {
            assembly ("memory-safe") {
                revert(add(ret, 0x20), mload(ret))
            }
        }
        return ret;
    }
}
