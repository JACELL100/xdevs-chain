// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Predeploys} from "../common/Predeploys.sol";

/// @title FeeConfig — governance-controlled fee parameters read by the node (predeploy 0x…1003)
/// @author THE xDEVS
/// @notice The Rust node reads these values by RAW STORAGE SLOT at the start of every block (plan §15).
///         Every value occupies its own full 32-byte slot, as a big-endian uint256 (address in the low 20 bytes):
///
///   slot 0  governance                     address allowed to call {setParams}
///   slot 1  blockGasLimit                  gas, testnet 30_000_000
///   slot 2  minBaseFee                     wei, testnet 100_000_000 (0.1 gwei)
///   slot 3  burnBps                        share of base fees burned, 8000 (rest credited to Treasury)
///   slot 4  baseFeeMaxChangeDenominator    EIP-1559 denominator, 8 (= ±12.5% per block)
///   slot 5  elasticityMultiplier           EIP-1559 elasticity, 2 (target = limit / 2)
///   slot 6  maxTxSizeBytes                 131_072 (128 KB)
///   slot 7  localFeeEnabled                0 / 1 (Phase 5 local fee markets, §15.2)
///   slot 8  localFeeThresholdBps           share of block gas that triggers a local fee, 2500
///   slot 9  localFeeMaxChangeDenominator   8
///   slot 10 version                        0 = not initialized, then +1 on every change
///
/// @dev Changes apply from the next block (the node reads the post-state of the previous block). The layout is
///      append-only: new parameters get new slots after slot 10, existing slots never move.
///      GENESIS: the builder calls {initialize} once as SYSTEM_ADDRESS (or writes the slots directly).
contract FeeConfig {
    struct FeeParams {
        uint256 blockGasLimit;
        uint256 minBaseFee;
        uint256 burnBps;
        uint256 baseFeeMaxChangeDenominator;
        uint256 elasticityMultiplier;
        uint256 maxTxSizeBytes;
        uint256 localFeeEnabled;
        uint256 localFeeThresholdBps;
        uint256 localFeeMaxChangeDenominator;
    }

    error OnlySystem();
    error OnlyGovernance();
    error AlreadyInitialized();
    error ZeroAddress();
    error InvalidParam(string name);

    event Initialized(address indexed governance);
    event ParamsUpdated(uint256 indexed version, FeeParams params);

    address public governance; // slot 0
    uint256 public blockGasLimit; // slot 1
    uint256 public minBaseFee; // slot 2
    uint256 public burnBps; // slot 3
    uint256 public baseFeeMaxChangeDenominator; // slot 4
    uint256 public elasticityMultiplier; // slot 5
    uint256 public maxTxSizeBytes; // slot 6
    uint256 public localFeeEnabled; // slot 7
    uint256 public localFeeThresholdBps; // slot 8
    uint256 public localFeeMaxChangeDenominator; // slot 9
    uint256 public version; // slot 10

    /// @dev Intentionally empty: never runs at the predeploy address.
    constructor() {}

    /// @notice One-time setup by the genesis builder (as SYSTEM_ADDRESS).
    function initialize(address governance_, FeeParams calldata p) external {
        if (msg.sender != Predeploys.SYSTEM_ADDRESS) revert OnlySystem();
        if (version != 0) revert AlreadyInitialized();
        if (governance_ == address(0)) revert ZeroAddress();
        governance = governance_;
        emit Initialized(governance_);
        _set(p);
    }

    /// @notice Replaces all parameters. Only Governance (i.e. after vote + timelock).
    function setParams(FeeParams calldata p) external {
        if (msg.sender != governance || msg.sender == address(0)) revert OnlyGovernance();
        _set(p);
    }

    function params() external view returns (FeeParams memory p) {
        p = FeeParams({
            blockGasLimit: blockGasLimit,
            minBaseFee: minBaseFee,
            burnBps: burnBps,
            baseFeeMaxChangeDenominator: baseFeeMaxChangeDenominator,
            elasticityMultiplier: elasticityMultiplier,
            maxTxSizeBytes: maxTxSizeBytes,
            localFeeEnabled: localFeeEnabled,
            localFeeThresholdBps: localFeeThresholdBps,
            localFeeMaxChangeDenominator: localFeeMaxChangeDenominator
        });
    }

    /// @notice Share of base fees credited to the Treasury, in bps (= 10_000 - burnBps).
    function treasuryBps() external view returns (uint256) {
        return 10_000 - burnBps;
    }

    function _set(FeeParams calldata p) internal {
        if (p.blockGasLimit < 1_000_000 || p.blockGasLimit > 1_000_000_000) {
            revert InvalidParam("blockGasLimit");
        }
        if (p.minBaseFee == 0 || p.minBaseFee > 1_000 gwei) revert InvalidParam("minBaseFee");
        if (p.burnBps > 10_000) revert InvalidParam("burnBps");
        if (p.baseFeeMaxChangeDenominator == 0 || p.baseFeeMaxChangeDenominator > 1_000) {
            revert InvalidParam("baseFeeMaxChangeDenominator");
        }
        if (p.elasticityMultiplier == 0 || p.elasticityMultiplier > 100) {
            revert InvalidParam("elasticityMultiplier");
        }
        if (p.maxTxSizeBytes < 1_024 || p.maxTxSizeBytes > 4 * 1024 * 1024) {
            revert InvalidParam("maxTxSizeBytes");
        }
        if (p.localFeeEnabled > 1) revert InvalidParam("localFeeEnabled");
        if (p.localFeeThresholdBps == 0 || p.localFeeThresholdBps > 10_000) {
            revert InvalidParam("localFeeThresholdBps");
        }
        if (p.localFeeMaxChangeDenominator == 0 || p.localFeeMaxChangeDenominator > 1_000) {
            revert InvalidParam("localFeeMaxChangeDenominator");
        }
        blockGasLimit = p.blockGasLimit;
        minBaseFee = p.minBaseFee;
        burnBps = p.burnBps;
        baseFeeMaxChangeDenominator = p.baseFeeMaxChangeDenominator;
        elasticityMultiplier = p.elasticityMultiplier;
        maxTxSizeBytes = p.maxTxSizeBytes;
        localFeeEnabled = p.localFeeEnabled;
        localFeeThresholdBps = p.localFeeThresholdBps;
        localFeeMaxChangeDenominator = p.localFeeMaxChangeDenominator;
        uint256 v = version + 1;
        version = v;
        emit ParamsUpdated(v, p);
    }
}
