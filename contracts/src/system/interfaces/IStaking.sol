// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IStaking
/// @notice Interface of the xDEVS Chain Staking predeploy (0x…1000), plan §11.4–11.6 and §18.1.
/// @dev The first block of functions/events is the §18.1 sketch, kept verbatim. Everything after it is an
///      additive extension (lifecycle, views for the node and for Governance voting power).
interface IStaking {
    // ---------------------------------------------------------------------------------------------------------
    // Types
    // ---------------------------------------------------------------------------------------------------------

    /// @notice Validator lifecycle state (plan §11.4). `Exited` is the diagram's "Unbonding → [*]" terminal state:
    ///         the operator withdrew its whole self-stake; the validator can never be selected again.
    enum ValidatorStatus {
        None,
        Candidate,
        Active,
        Jailed,
        Tombstoned,
        Exited
    }

    /// @notice Governance-tunable staking parameters (plan §11.5 / §33.1).
    struct Params {
        uint256 minSelfStake; // wei, testnet 10_000 tXDS
        uint256 minDelegation; // wei, smallest delegate() amount (anti-dust)
        uint64 unbondingSeconds; // testnet 1 day
        uint64 commissionChangeInterval; // seconds between commission changes, 1 day
        uint64 downtimeWindow; // blocks per liveness window, 10_000
        uint64 downtimeJailSeconds; // 10 min
        uint16 maxValidators; // active-set size N, 100 (hard cap 256 = signer bitmap width)
        uint16 maxCandidates; // bound on registered (non-exited, non-tombstoned) validators
        uint16 maxCommissionBps; // 2000 = 20%
        uint16 maxCommissionChangeBps; // 100 = 1% per interval
        uint16 proposerBonusBps; // 500 = 5%
        uint16 doubleSignSlashBps; // 500 = 5%
        uint16 downtimeSlashBps; // 1 = 0.01%
        uint16 minSignedBps; // 5000 = must sign >= 50% of the window
    }

    /// @notice Validator created by `initialize` (genesis). Its stake must already be in the contract balance.
    struct GenesisValidator {
        address operator;
        bytes32 consensusKey;
        uint16 commissionBps;
        uint256 stake;
    }

    /// @notice Flattened view of a validator.
    struct ValidatorInfo {
        bytes32 consensusKey;
        ValidatorStatus status;
        uint16 commissionBps;
        uint64 jailedUntil;
        uint64 lastCommissionChange;
        uint256 totalStake;
        uint256 totalShares;
        uint256 selfStake;
        uint256 unbondingStake;
        uint256 unbondingShares;
        uint256 commissionAccrued;
    }

    /// @notice One pending undelegation. `poolShares` are shares of the validator's unbonding pool, so that
    ///         double-sign slashing also reaches stake that is still unbonding.
    struct UnbondingEntry {
        address validator;
        uint64 completionTime;
        uint256 poolShares;
    }

    // ---------------------------------------------------------------------------------------------------------
    // §18.1 sketch
    // ---------------------------------------------------------------------------------------------------------

    event ValidatorRegistered(address indexed operator, bytes32 consensusKey, uint256 selfStake);
    event Delegated(address indexed delegator, address indexed validator, uint256 amount);
    /// @param reason 1 = double-sign, 2 = downtime
    event Slashed(address indexed validator, uint256 amount, uint8 reason);
    event EpochTransition(uint256 indexed epoch, bytes32 validatorSetHash);

    function register(bytes32 consensusKey, uint16 commissionBps) external payable;
    function delegate(address validator) external payable;
    function undelegate(address validator, uint256 amount) external;
    function withdrawUnbonded() external;
    function claimRewards(address validator) external;
    function unjail() external;

    // System-only (called by the protocol with msg.sender == SYSTEM_ADDRESS)
    function onEpoch() external;
    function recordParticipation(address proposer, uint256 signedBitmap) external payable;
    function slash(bytes calldata evidence) external;

    function activeValidators() external view returns (address[] memory, uint256[] memory power);

    // ---------------------------------------------------------------------------------------------------------
    // Extensions
    // ---------------------------------------------------------------------------------------------------------

    event Initialized(address indexed governance, uint256 genesisValidators, uint256 genesisStake);
    event ParamsUpdated(Params params);
    event Undelegated(
        address indexed delegator,
        address indexed validator,
        uint256 amount,
        uint256 shares,
        uint64 completionTime
    );
    event Withdrawn(address indexed delegator, uint256 amount);
    event RewardsClaimed(address indexed delegator, address indexed validator, uint256 amount);
    event RewardsDistributed(uint256 indexed blockNumber, address indexed proposer, uint256 amount);
    event CommissionChanged(address indexed validator, uint16 oldBps, uint16 newBps);
    event Jailed(address indexed validator, uint64 until);
    event Unjailed(address indexed validator);
    event Tombstoned(address indexed validator, uint64 infractionHeight, bytes32 evidenceRef);
    event ValidatorExited(address indexed validator);
    /// @notice Emitted when the active set changes outside an epoch boundary (jail / tombstone / exit removal).
    event ValidatorSetUpdated(uint256 indexed epoch, uint256 setVersion, bytes32 validatorSetHash);
    /// @param code 1 = duplicate, 2 = malformed, 3 = unknown kind, 4 = unknown or already tombstoned validator
    event EvidenceIgnored(bytes32 indexed evidenceHash, uint8 code);

    function initialize(address governance, Params calldata params, GenesisValidator[] calldata validators)
        external;
    function setParams(Params calldata params) external;
    function setCommission(uint16 newCommissionBps) external;

    function params() external view returns (Params memory);
    function governance() external view returns (address);
    function currentEpoch() external view returns (uint64);
    function setVersion() external view returns (uint64);
    function validatorSetHash() external view returns (bytes32);
    function consensusValidatorSet()
        external
        view
        returns (address[] memory operators, bytes32[] memory consensusKeys, uint256[] memory powers);
    function signingSet() external view returns (address[] memory operators, uint256[] memory powers);
    function candidates() external view returns (address[] memory);
    function getValidator(address validator) external view returns (ValidatorInfo memory);
    function selfStake(address validator) external view returns (uint256);
    function delegationOf(address validator, address delegator)
        external
        view
        returns (uint256 shares, uint256 amount, uint256 principal);
    function pendingRewards(address validator, address delegator) external view returns (uint256);
    function unbondingEntries(address delegator) external view returns (UnbondingEntry[] memory);
    function unbondingValue(address delegator) external view returns (uint256 total, uint256 withdrawable);

    // Governance voting power: bonded principal (staked + delegated), checkpointed by block number.
    function getVotes(address account) external view returns (uint256);
    function getPastVotes(address account, uint256 blockNumber) external view returns (uint256);
    function getPastTotalVotes(uint256 blockNumber) external view returns (uint256);
}
