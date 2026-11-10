// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {Staking} from "../../src/system/Staking.sol";
import {Governance} from "../../src/system/Governance.sol";
import {Treasury} from "../../src/system/Treasury.sol";
import {FeeConfig} from "../../src/system/FeeConfig.sol";
import {IStaking} from "../../src/system/interfaces/IStaking.sol";
import {IGovernance} from "../../src/system/interfaces/IGovernance.sol";

/// @title SystemGenesis — reference implementation of the genesis procedure for the system predeploys
/// @notice Mirrors what the Rust genesis builder (`xdevs-cli genesis build`) must do, so both can be diffed:
///   1. put the RUNTIME bytecode (out/system/<C>.sol/<C>.json -> deployedBytecode) at the fixed address;
///   2. set the genesis balances (Staking = sum of genesis stakes, Treasury = 200M tXDS);
///   3. call initialize(...) on each contract with msg.sender = SYSTEM_ADDRESS (in this order:
///      Staking, Governance, Treasury, FeeConfig; WXDS needs no initialization);
///   4. dump the resulting storage of each address into genesis `alloc`.
/// @dev Run (local simulation only, nothing is broadcast):
///        FOUNDRY_PROFILE=system forge script script/system/SystemGenesis.s.sol -vv
///      Optional env: GUARDIAN (address), VALIDATOR_OPERATORS (comma-separated addresses),
///      VALIDATOR_KEYS (comma-separated bytes32 Ed25519 public keys), GENESIS_STATE_OUT (path for vm.dumpState;
///      requires an `fs_permissions` write entry for that path in foundry.toml, not configured by default).
contract SystemGenesis is Script {
    uint256 internal constant STAKE_PER_VALIDATOR = 12_500_000 ether;
    uint256 internal constant TREASURY_BALANCE = 200_000_000 ether;

    function stakingParams() public pure returns (IStaking.Params memory) {
        return IStaking.Params({
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

    function governanceParams() public pure returns (IGovernance.Params memory) {
        return IGovernance.Params({
            votingPeriod: 3 days,
            timelockDelay: 1 days,
            gracePeriod: 14 days,
            pauseDuration: 3 days,
            quorumBps: 3_340,
            thresholdBps: 5_000,
            proposalThreshold: 10_000 ether
        });
    }

    function feeParams() public pure returns (FeeConfig.FeeParams memory) {
        return FeeConfig.FeeParams({
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

    function _validators() internal view returns (IStaking.GenesisValidator[] memory gv) {
        address[] memory defOps = new address[](4);
        bytes32[] memory defKeys = new bytes32[](4);
        for (uint256 i; i < 4; ++i) {
            defOps[i] = address(uint160(0xA11CE0 + i));
            defKeys[i] = keccak256(abi.encode("dev-ed25519-key", i));
        }
        address[] memory ops = vm.envOr("VALIDATOR_OPERATORS", ",", defOps);
        bytes32[] memory keys = vm.envOr("VALIDATOR_KEYS", ",", defKeys);
        require(ops.length == keys.length, "operators/keys length mismatch");
        gv = new IStaking.GenesisValidator[](ops.length);
        for (uint256 i; i < ops.length; ++i) {
            gv[i] = IStaking.GenesisValidator({
                operator: ops[i], consensusKey: keys[i], commissionBps: 1_000, stake: STAKE_PER_VALIDATOR
            });
        }
    }

    function run() external {
        address guardian = vm.envOr("GUARDIAN", address(0));
        IStaking.GenesisValidator[] memory gv = _validators();

        // 1. runtime code at the fixed addresses (no constructor runs)
        vm.etch(Predeploys.STAKING, vm.getDeployedCode("Staking.sol:Staking"));
        vm.etch(Predeploys.GOVERNANCE, vm.getDeployedCode("Governance.sol:Governance"));
        vm.etch(Predeploys.TREASURY, vm.getDeployedCode("Treasury.sol:Treasury"));
        vm.etch(Predeploys.FEE_CONFIG, vm.getDeployedCode("FeeConfig.sol:FeeConfig"));
        vm.etch(Predeploys.WXDS, vm.getDeployedCode("WXDS.sol:WXDS"));

        // 2. balances
        vm.deal(Predeploys.STAKING, gv.length * STAKE_PER_VALIDATOR);
        vm.deal(Predeploys.TREASURY, TREASURY_BALANCE);

        // 3. one-time initialization as SYSTEM_ADDRESS
        vm.startPrank(Predeploys.SYSTEM_ADDRESS);
        Staking(Predeploys.STAKING).initialize(Predeploys.GOVERNANCE, stakingParams(), gv);
        Governance(payable(Predeploys.GOVERNANCE))
            .initialize(Predeploys.STAKING, guardian, governanceParams());
        Treasury(payable(Predeploys.TREASURY)).initialize(Predeploys.GOVERNANCE);
        FeeConfig(Predeploys.FEE_CONFIG).initialize(Predeploys.GOVERNANCE, feeParams());
        vm.stopPrank();

        // 4. report / dump
        (address[] memory ops, bytes32[] memory keys, uint256[] memory powers) =
            Staking(Predeploys.STAKING).consensusValidatorSet();
        console.log("genesis validators:", ops.length);
        for (uint256 i; i < ops.length; ++i) {
            console.log(ops[i], vm.toString(keys[i]), powers[i] / 1 ether);
        }
        console.log("validatorSetHash:", vm.toString(Staking(Predeploys.STAKING).validatorSetHash()));
        console.log("FeeConfig version:", FeeConfig(Predeploys.FEE_CONFIG).version());

        string memory out = vm.envOr("GENESIS_STATE_OUT", string(""));
        if (bytes(out).length != 0) {
            vm.dumpState(out);
            console.log("state dumped to", out);
        }
    }
}
