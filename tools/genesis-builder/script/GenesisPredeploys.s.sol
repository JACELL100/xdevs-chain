// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Script} from "forge-std/Script.sol";
import {Vm} from "forge-std/Vm.sol";

import {Predeploys} from "src/common/Predeploys.sol";
import {Staking} from "src/system/Staking.sol";
import {Governance} from "src/system/Governance.sol";
import {Treasury} from "src/system/Treasury.sol";
import {FeeConfig} from "src/system/FeeConfig.sol";
import {IStaking} from "src/system/interfaces/IStaking.sol";
import {IGovernance} from "src/system/interfaces/IGovernance.sol";
import {XNSRegistry} from "src/periphery/xns/XNSRegistry.sol";
import {SystemGenesis} from "script/system/SystemGenesis.s.sol";
import {DeployPeriphery} from "script/periphery/DeployPeriphery.s.sol";

import {CanonicalCode} from "./CanonicalCode.sol";

/// @title GenesisPredeploys - computes the genesis state of every xDEVS predeploy (plan §18, §21.2)
/// @notice Driven by `tools/genesis-builder/genesis-builder.ts build`; not meant to be run by hand.
///   Runs entirely in forge's in-memory EVM (nothing is broadcast) at block.number = 0,
///   block.timestamp = genesis timestamp, block.chainid = target chain id:
///     1. runtime code (deployedBytecode, never a constructor) at 0x…1000-0x…1009;
///     2. canonical helpers: deterministic deployment proxy (runtime code), EntryPoint v0.9 via its real CREATE2
///        deployment through that proxy (constructor runs, so its immutables/SenderCreator match mainnet's
///        procedure for this chain id), Multicall3 (runtime code);
///     3. Staking balance = Σ genesis stakes, then `initialize` as SYSTEM_ADDRESS in the documented order:
///        Staking, Governance, Treasury, FeeConfig (docs/specs/predeploys-system.md), then XNSRegistry
///        (docs/specs/xns-and-approvals.md). Permit2Expiring, WXDS and the account predeploys need code only;
///     4. returns every predeploy account (code, balance, nonce, every storage slot written in steps 1-3) as JSON.
/// @dev Input: env `GENESIS_PARAMS` (JSON, see docs/specs/genesis-builder.md). Missing parameter groups default to
///      the reference values of script/system/SystemGenesis.s.sol and script/periphery/DeployPeriphery.s.sol.
///      No filesystem access is needed (code comes from the compiled artifacts, the output is the return value).
contract GenesisPredeploys is Script {
    struct Out {
        address[] accounts;
        bytes32[][] slots;
    }

    string internal params;

    function run() external returns (string memory json) {
        params = vm.envString("GENESIS_PARAMS");

        vm.chainId(vm.parseJsonUint(params, ".chainId"));
        vm.roll(0);
        vm.warp(vm.parseJsonUint(params, ".timestamp"));

        address[] memory accounts = _accountList();
        for (uint256 i; i < accounts.length; ++i) {
            // the CREATE2 deployer may be pre-installed by forge itself; everything else must be empty
            if (accounts[i] != CanonicalCode.CREATE2_DEPLOYER) {
                require(accounts[i].code.length == 0 && accounts[i].balance == 0, "predeploy address not empty");
            }
        }

        // Defaults are taken from the reference scripts (deployed in the scratch EVM only, never dumped).
        SystemGenesis ref = new SystemGenesis();
        DeployPeriphery periphery = new DeployPeriphery();

        vm.startStateDiffRecording();

        // 1. runtime code at the fixed addresses
        vm.etch(Predeploys.STAKING, vm.getDeployedCode("Staking.sol:Staking"));
        vm.etch(Predeploys.GOVERNANCE, vm.getDeployedCode("Governance.sol:Governance"));
        vm.etch(Predeploys.TREASURY, vm.getDeployedCode("Treasury.sol:Treasury"));
        vm.etch(Predeploys.FEE_CONFIG, vm.getDeployedCode("FeeConfig.sol:FeeConfig"));
        vm.etch(Predeploys.XNS_REGISTRY, vm.getDeployedCode("XNSRegistry.sol:XNSRegistry"));
        vm.etch(Predeploys.SMART_ACCOUNT_IMPL, vm.getDeployedCode("XdevsSmartAccount.sol:XdevsSmartAccount"));
        vm.etch(Predeploys.SMART_ACCOUNT_FACTORY, vm.getDeployedCode("SmartAccountFactory.sol:SmartAccountFactory"));
        vm.etch(Predeploys.PAYMASTER, vm.getDeployedCode("XdevsPaymaster.sol:XdevsPaymaster"));
        vm.etch(Predeploys.PERMIT2_EXPIRING, vm.getDeployedCode("Permit2Expiring.sol:Permit2Expiring"));
        vm.etch(Predeploys.WXDS, vm.getDeployedCode("WXDS.sol:WXDS"));

        // 2. canonical helpers
        bytes memory c2 = CanonicalCode.create2DeployerRuntimeCode();
        require(keccak256(c2) == CanonicalCode.CREATE2_DEPLOYER_CODEHASH, "create2 deployer code");
        vm.etch(CanonicalCode.CREATE2_DEPLOYER, c2);
        // Nonces as after a real deployment on a fresh chain (EIP-161: a created contract starts at nonce 1);
        // forge may pre-install the deployer with another nonce, so pin it.
        vm.setNonceUnsafe(CanonicalCode.CREATE2_DEPLOYER, 1);
        bytes memory mc = CanonicalCode.multicall3RuntimeCode();
        require(keccak256(mc) == CanonicalCode.MULTICALL3_CODEHASH, "multicall3 code");
        vm.etch(CanonicalCode.MULTICALL3, mc);
        vm.setNonce(CanonicalCode.MULTICALL3, 1);
        _deployEntryPoint();

        // 3. balances + one-time initialization as SYSTEM_ADDRESS
        IStaking.GenesisValidator[] memory gv = _validators();
        uint256 stakingBalance = vm.parseJsonUint(params, ".stakingBalance");
        vm.deal(Predeploys.STAKING, stakingBalance);

        vm.startPrank(Predeploys.SYSTEM_ADDRESS);
        Staking(Predeploys.STAKING).initialize(Predeploys.GOVERNANCE, _stakingParams(ref.stakingParams()), gv);
        Governance(payable(Predeploys.GOVERNANCE))
            .initialize(
                Predeploys.STAKING, _addr(".guardian", address(0)), _governanceParams(ref.governanceParams())
            );
        Treasury(payable(Predeploys.TREASURY)).initialize(Predeploys.GOVERNANCE);
        FeeConfig(Predeploys.FEE_CONFIG).initialize(Predeploys.GOVERNANCE, _feeParams(ref.feeParams()));
        (bool ok, bytes memory ret) = Predeploys.XNS_REGISTRY.call(_xnsInitCalldata(periphery));
        if (!ok) {
            assembly ("memory-safe") {
                revert(add(ret, 32), mload(ret))
            }
        }
        vm.stopPrank();

        Vm.AccountAccess[] memory diff = vm.stopAndReturnStateDiff();

        // 4. serialize
        json = _serialize(accounts, diff);
    }

    // ------------------------------------------------------------------ canonical helpers

    function _deployEntryPoint() internal {
        bytes memory init = CanonicalCode.entryPointInitCode();
        require(keccak256(init) == CanonicalCode.ENTRY_POINT_INITCODE_HASH, "entrypoint init code");
        (bool ok, bytes memory ret) =
            CanonicalCode.CREATE2_DEPLOYER.call(abi.encodePacked(CanonicalCode.ENTRY_POINT_SALT, init));
        require(ok && ret.length == 20, "entrypoint create2 failed");
        require(address(bytes20(ret)) == CanonicalCode.ENTRY_POINT, "entrypoint address mismatch");
        require(CanonicalCode.ENTRY_POINT.code.length != 0, "entrypoint has no code");
    }

    function _senderCreator() internal pure returns (address) {
        // EntryPoint's constructor does `new SenderCreator()` (CREATE from the EntryPoint at nonce 1)
        return vm.computeCreateAddress(CanonicalCode.ENTRY_POINT, 1);
    }

    function _accountList() internal pure returns (address[] memory a) {
        a = new address[](14);
        a[0] = Predeploys.STAKING;
        a[1] = Predeploys.GOVERNANCE;
        a[2] = Predeploys.TREASURY;
        a[3] = Predeploys.FEE_CONFIG;
        a[4] = Predeploys.XNS_REGISTRY;
        a[5] = Predeploys.SMART_ACCOUNT_IMPL;
        a[6] = Predeploys.SMART_ACCOUNT_FACTORY;
        a[7] = Predeploys.PAYMASTER;
        a[8] = Predeploys.PERMIT2_EXPIRING;
        a[9] = Predeploys.WXDS;
        a[10] = CanonicalCode.CREATE2_DEPLOYER;
        a[11] = CanonicalCode.ENTRY_POINT;
        a[12] = _senderCreator();
        a[13] = CanonicalCode.MULTICALL3;
    }

    // ------------------------------------------------------------------ parameters

    function _has(string memory key) internal view returns (bool) {
        return vm.keyExistsJson(params, key);
    }

    function _u(string memory key, uint256 def) internal view returns (uint256) {
        return _has(key) ? vm.parseJsonUint(params, key) : def;
    }

    function _addr(string memory key, address def) internal view returns (address) {
        return _has(key) ? vm.parseJsonAddress(params, key) : def;
    }

    function _validators() internal view returns (IStaking.GenesisValidator[] memory gv) {
        address[] memory ops = vm.parseJsonAddressArray(params, ".validators.operators");
        bytes32[] memory keys = vm.parseJsonBytes32Array(params, ".validators.consensusKeys");
        uint256[] memory comm = vm.parseJsonUintArray(params, ".validators.commissionBps");
        uint256[] memory stakes = vm.parseJsonUintArray(params, ".validators.stakes");
        require(
            ops.length == keys.length && ops.length == comm.length && ops.length == stakes.length,
            "validator arrays length mismatch"
        );
        gv = new IStaking.GenesisValidator[](ops.length);
        for (uint256 i; i < ops.length; ++i) {
            require(comm[i] <= type(uint16).max, "commission");
            gv[i] = IStaking.GenesisValidator({
                operator: ops[i], consensusKey: keys[i], commissionBps: uint16(comm[i]), stake: stakes[i]
            });
        }
    }

    function _stakingParams(IStaking.Params memory p) internal view returns (IStaking.Params memory) {
        p.minSelfStake = _u(".staking.minSelfStake", p.minSelfStake);
        p.minDelegation = _u(".staking.minDelegation", p.minDelegation);
        p.unbondingSeconds = uint64(_u(".staking.unbondingSeconds", p.unbondingSeconds));
        p.commissionChangeInterval = uint64(_u(".staking.commissionChangeInterval", p.commissionChangeInterval));
        p.downtimeWindow = uint64(_u(".staking.downtimeWindow", p.downtimeWindow));
        p.downtimeJailSeconds = uint64(_u(".staking.downtimeJailSeconds", p.downtimeJailSeconds));
        p.maxValidators = uint16(_u(".staking.maxValidators", p.maxValidators));
        p.maxCandidates = uint16(_u(".staking.maxCandidates", p.maxCandidates));
        p.maxCommissionBps = uint16(_u(".staking.maxCommissionBps", p.maxCommissionBps));
        p.maxCommissionChangeBps = uint16(_u(".staking.maxCommissionChangeBps", p.maxCommissionChangeBps));
        p.proposerBonusBps = uint16(_u(".staking.proposerBonusBps", p.proposerBonusBps));
        p.doubleSignSlashBps = uint16(_u(".staking.doubleSignSlashBps", p.doubleSignSlashBps));
        p.downtimeSlashBps = uint16(_u(".staking.downtimeSlashBps", p.downtimeSlashBps));
        p.minSignedBps = uint16(_u(".staking.minSignedBps", p.minSignedBps));
        return p;
    }

    function _governanceParams(IGovernance.Params memory p) internal view returns (IGovernance.Params memory) {
        p.votingPeriod = uint64(_u(".governance.votingPeriod", p.votingPeriod));
        p.timelockDelay = uint64(_u(".governance.timelockDelay", p.timelockDelay));
        p.gracePeriod = uint64(_u(".governance.gracePeriod", p.gracePeriod));
        p.pauseDuration = uint64(_u(".governance.pauseDuration", p.pauseDuration));
        p.quorumBps = uint16(_u(".governance.quorumBps", p.quorumBps));
        p.thresholdBps = uint16(_u(".governance.thresholdBps", p.thresholdBps));
        p.proposalThreshold = _u(".governance.proposalThreshold", p.proposalThreshold);
        return p;
    }

    function _feeParams(FeeConfig.FeeParams memory p) internal view returns (FeeConfig.FeeParams memory) {
        p.blockGasLimit = _u(".fee.blockGasLimit", p.blockGasLimit);
        p.minBaseFee = _u(".fee.minBaseFee", p.minBaseFee);
        p.burnBps = _u(".fee.burnBps", p.burnBps);
        p.baseFeeMaxChangeDenominator = _u(".fee.baseFeeMaxChangeDenominator", p.baseFeeMaxChangeDenominator);
        p.elasticityMultiplier = _u(".fee.elasticityMultiplier", p.elasticityMultiplier);
        p.maxTxSizeBytes = _u(".fee.maxTxSizeBytes", p.maxTxSizeBytes);
        p.localFeeEnabled = _u(".fee.localFeeEnabled", p.localFeeEnabled);
        p.localFeeThresholdBps = _u(".fee.localFeeThresholdBps", p.localFeeThresholdBps);
        p.localFeeMaxChangeDenominator = _u(".fee.localFeeMaxChangeDenominator", p.localFeeMaxChangeDenominator);
        return p;
    }

    function _xnsInitCalldata(DeployPeriphery periphery) internal view returns (bytes memory) {
        if (!_has(".xns")) return periphery.genesisInitCalldata(); // reference: gov 0x…1001, treasury 0x…1002, fee 0
        // DeployPeriphery's reserved list is the default; an explicit list replaces it.
        string[] memory reserved;
        if (_has(".xns.reserved")) {
            reserved = vm.parseJsonStringArray(params, ".xns.reserved");
        } else {
            (,,, reserved) = abi.decode(_slice4(periphery.genesisInitCalldata()), (address, address, uint256, string[]));
        }
        return abi.encodeCall(
            XNSRegistry.initialize,
            (
                _addr(".xns.governance", Predeploys.GOVERNANCE),
                _addr(".xns.treasury", Predeploys.TREASURY),
                _u(".xns.feePerYear", 0),
                reserved
            )
        );
    }

    function _slice4(bytes memory data) internal pure returns (bytes memory out) {
        out = new bytes(data.length - 4);
        for (uint256 i; i < out.length; ++i) {
            out[i] = data[i + 4];
        }
    }

    // ------------------------------------------------------------------ output

    function _serialize(address[] memory accounts, Vm.AccountAccess[] memory diff)
        internal
        returns (string memory json)
    {
        // every storage write must land in a predeploy account
        for (uint256 i; i < diff.length; ++i) {
            for (uint256 j; j < diff[i].storageAccesses.length; ++j) {
                Vm.StorageAccess memory s = diff[i].storageAccesses[j];
                if (s.isWrite && !s.reverted) require(_contains(accounts, s.account), "write outside predeploys");
            }
        }

        string memory accJson;
        for (uint256 a; a < accounts.length; ++a) {
            address acct = accounts[a];
            string memory id = vm.toString(acct);
            string memory storageId = string.concat("storage:", id);
            string memory storageJson = "{}";
            bytes32[] memory seen = new bytes32[](_writeCount(diff));
            uint256 n;
            for (uint256 i; i < diff.length; ++i) {
                for (uint256 j; j < diff[i].storageAccesses.length; ++j) {
                    Vm.StorageAccess memory s = diff[i].storageAccesses[j];
                    if (!s.isWrite || s.reverted || s.account != acct || _seen(seen, n, s.slot)) continue;
                    seen[n++] = s.slot;
                    bytes32 v = vm.load(acct, s.slot);
                    if (v != bytes32(0)) storageJson = vm.serializeBytes32(storageId, vm.toString(s.slot), v);
                }
            }
            string memory accId = string.concat("account:", id);
            vm.serializeString(accId, "balance", vm.toString(acct.balance));
            vm.serializeString(accId, "nonce", vm.toString(uint256(vm.getNonce(acct))));
            vm.serializeBytes(accId, "code", acct.code);
            string memory one = vm.serializeString(accId, "storage", storageJson);
            accJson = vm.serializeString("accounts", id, one);
        }

        (address[] memory ops, bytes32[] memory keys, uint256[] memory powers) =
            Staking(Predeploys.STAKING).consensusValidatorSet();
        string[] memory pw = new string[](powers.length);
        for (uint256 i; i < powers.length; ++i) {
            pw[i] = vm.toString(powers[i]);
        }
        vm.serializeAddress("set", "operators", ops);
        vm.serializeBytes32("set", "consensusKeys", keys);
        string memory setJson = vm.serializeString("set", "powers", pw);

        vm.serializeString("out", "accounts", accJson);
        vm.serializeString("out", "validatorSet", setJson);
        vm.serializeBytes32("out", "validatorSetHash", Staking(Predeploys.STAKING).validatorSetHash());
        vm.serializeAddress("out", "senderCreator", _senderCreator());
        json = vm.serializeUint("out", "feeConfigVersion", FeeConfig(Predeploys.FEE_CONFIG).version());
    }

    function _writeCount(Vm.AccountAccess[] memory diff) internal pure returns (uint256 n) {
        for (uint256 i; i < diff.length; ++i) {
            n += diff[i].storageAccesses.length;
        }
    }

    function _seen(bytes32[] memory seen, uint256 n, bytes32 slot) internal pure returns (bool) {
        for (uint256 i; i < n; ++i) {
            if (seen[i] == slot) return true;
        }
        return false;
    }

    function _contains(address[] memory list, address a) internal pure returns (bool) {
        for (uint256 i; i < list.length; ++i) {
            if (list[i] == a) return true;
        }
        return false;
    }
}
