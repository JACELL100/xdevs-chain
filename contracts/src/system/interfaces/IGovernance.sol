// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IGovernance
/// @notice Interface of the xDEVS Chain Governance predeploy (0x…1001).
interface IGovernance {
    enum ProposalState {
        Active,
        Defeated,
        Succeeded,
        Queued,
        Expired,
        Executed,
        Canceled
    }

    /// @notice Vote options.
    enum VoteType {
        Against,
        For,
        Abstain
    }

    struct Params {
        uint64 votingPeriod; // seconds, testnet 3 days
        uint64 timelockDelay; // seconds between queue() and the earliest execute(), testnet 1 day
        uint64 gracePeriod; // seconds after eta during which execute() is allowed
        uint64 pauseDuration; // seconds a guardian pause lasts (also the cooldown before the next pause)
        uint16 quorumBps; // (for + abstain) must reach this share of total voting power at the snapshot
        uint16 thresholdBps; // for / (for + against) must be strictly greater than this
        uint256 proposalThreshold; // minimum voting power (wei of bonded principal) to propose
    }

    event Initialized(address indexed staking, address indexed guardian);
    event ParamsUpdated(Params params);
    event GuardianUpdated(address indexed oldGuardian, address indexed newGuardian);
    event ProposalCreated(
        uint256 indexed id,
        address indexed proposer,
        address[] targets,
        uint256[] values,
        bytes[] calldatas,
        uint256 snapshotBlock,
        uint64 voteEnd,
        string description
    );
    event VoteCast(address indexed voter, uint256 indexed id, uint8 support, uint256 weight);
    event ProposalQueued(uint256 indexed id, uint64 eta);
    event ProposalExecuted(uint256 indexed id);
    event ProposalCanceled(uint256 indexed id);
    event Paused(address indexed guardian, uint64 until);
    event Unpaused(address indexed by);

    function initialize(address staking, address guardian, Params calldata params) external;

    function propose(
        address[] calldata targets,
        uint256[] calldata values,
        bytes[] calldata calldatas,
        string calldata description
    ) external returns (uint256 id);
    function castVote(uint256 id, uint8 support) external returns (uint256 weight);
    function queue(uint256 id) external;
    function execute(uint256 id) external payable;
    function cancel(uint256 id) external;

    function pause() external;
    function unpause() external;
    function setParams(Params calldata params) external;
    function setGuardian(address guardian) external;

    function state(uint256 id) external view returns (ProposalState);
    function paused() external view returns (bool);
}
