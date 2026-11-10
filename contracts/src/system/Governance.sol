// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {Predeploys} from "../common/Predeploys.sol";
import {IGovernance} from "./interfaces/IGovernance.sol";

/// @notice Voting-power source (implemented by Staking).
interface IVotesSource {
    function getPastVotes(address account, uint256 blockNumber) external view returns (uint256);
    function getPastTotalVotes(uint256 blockNumber) external view returns (uint256);
}

/// @title Governance — stake-weighted proposals with a timelock (predeploy 0x…1001)
/// @author THE xDEVS
/// @notice A proposal is a list of calls `(target, value, calldata)`. Voting power is the voter's bonded principal
///         (own stake + delegations) in Staking at `snapshotBlock = block.number - 1` of proposal creation.
///
///   propose ─▶ Active (votingPeriod) ─▶ Succeeded ─queue─▶ Queued (timelockDelay) ─execute─▶ Executed
///                                   └─▶ Defeated                └─ after eta+gracePeriod ─▶ Expired
///   proposer may cancel while Active / Succeeded / Queued.
///
/// @dev Governance is the only account allowed to change FeeConfig, Staking params and to spend the Treasury; all
///      such calls therefore pass through the voting period AND the timelock. Governance's own parameters can only
///      be changed by a proposal (`msg.sender == address(this)`).
///
///      Emergency pause (plan §25.1): the guardian can only block NEW proposals, for `pauseDuration`, and must wait
///      another `pauseDuration` after a pause ends before pausing again, so it can never block governance forever.
///      Voting, queueing, executing and cancelling continue while paused.
///
///      GENESIS: no constructor logic; the genesis builder calls {initialize} once as SYSTEM_ADDRESS.
contract Governance is IGovernance, ReentrancyGuardTransient {
    uint256 internal constant BPS = 10_000;
    uint256 public constant MAX_ACTIONS = 16;

    error OnlySystem();
    error OnlySelf();
    error OnlyGuardian();
    error OnlyProposer();
    error AlreadyInitialized();
    error NotInitialized();
    error InvalidParam(string name);
    error InvalidProposal(string reason);
    error ProposalsPaused();
    error PauseCooldown();
    error NotPaused();
    error BelowProposalThreshold();
    error UnknownProposal();
    error InvalidState(ProposalState current);
    error AlreadyVoted();
    error InvalidVoteType();
    error NoVotingPower();
    error TimelockNotExpired(uint64 eta);
    error CallFailed(uint256 index, bytes returndata);

    struct Proposal {
        address proposer;
        uint48 snapshotBlock;
        uint16 thresholdBps;
        bool executed;
        bool canceled;
        uint64 voteEnd;
        uint64 eta;
        uint256 quorumVotes;
        uint256 forVotes;
        uint256 againstVotes;
        uint256 abstainVotes;
        address[] targets;
        uint256[] values;
        bytes[] calldatas;
    }

    // slot 0 (packed): staking | initialized
    address public staking;
    bool public initialized;
    // slot 1 (packed): guardian | pausedUntil
    address public guardian;
    uint64 public pausedUntil;
    // slot 2
    uint64 public nextPauseAllowedAt;
    // slots 3..5
    Params internal _params;
    // slot 6
    uint256 public proposalCount;
    mapping(uint256 id => Proposal) internal _proposals;
    mapping(uint256 id => mapping(address voter => bool)) public hasVoted;

    modifier onlySelf() {
        if (msg.sender != address(this)) revert OnlySelf();
        _;
    }

    /// @dev Intentionally empty: never runs at the predeploy address.
    constructor() {}

    receive() external payable {}

    /// @inheritdoc IGovernance
    /// @dev Called once by the genesis builder as SYSTEM_ADDRESS. `guardian` may be zero (no emergency pause).
    function initialize(address staking_, address guardian_, Params calldata p) external {
        if (msg.sender != Predeploys.SYSTEM_ADDRESS) revert OnlySystem();
        if (initialized) revert AlreadyInitialized();
        if (staking_ == address(0)) revert InvalidParam("staking");
        initialized = true;
        staking = staking_;
        guardian = guardian_;
        _setParams(p);
        emit Initialized(staking_, guardian_);
        emit ParamsUpdated(p);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Proposal lifecycle
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IGovernance
    function propose(
        address[] calldata targets,
        uint256[] calldata values,
        bytes[] calldata calldatas,
        string calldata description
    ) external returns (uint256 id) {
        id = _create(targets, values, calldatas);
        Proposal storage p = _proposals[id];
        emit ProposalCreated(
            id, msg.sender, targets, values, calldatas, p.snapshotBlock, p.voteEnd, description
        );
    }

    function _create(address[] calldata targets, uint256[] calldata values, bytes[] calldata calldatas)
        internal
        returns (uint256 id)
    {
        if (!initialized) revert NotInitialized();
        if (paused()) revert ProposalsPaused();
        uint256 n = targets.length;
        if (n == 0 || n > MAX_ACTIONS) revert InvalidProposal("actions");
        if (values.length != n || calldatas.length != n) revert InvalidProposal("length mismatch");

        uint256 snapshot = block.number - 1;
        IVotesSource src = IVotesSource(staking);
        if (src.getPastVotes(msg.sender, snapshot) < _params.proposalThreshold) {
            revert BelowProposalThreshold();
        }

        id = ++proposalCount;
        Proposal storage p = _proposals[id];
        p.proposer = msg.sender;
        p.snapshotBlock = uint48(snapshot);
        p.thresholdBps = _params.thresholdBps;
        p.voteEnd = uint64(block.timestamp + _params.votingPeriod);
        p.quorumVotes = src.getPastTotalVotes(snapshot) * _params.quorumBps / BPS;
        p.targets = targets;
        p.values = values;
        for (uint256 i; i < n; ++i) {
            p.calldatas.push(calldatas[i]);
        }
    }

    /// @inheritdoc IGovernance
    function castVote(uint256 id, uint8 support) external returns (uint256 weight) {
        ProposalState s = state(id);
        if (s != ProposalState.Active) revert InvalidState(s);
        if (support > uint8(VoteType.Abstain)) revert InvalidVoteType();
        if (hasVoted[id][msg.sender]) revert AlreadyVoted();
        Proposal storage p = _proposals[id];
        weight = IVotesSource(staking).getPastVotes(msg.sender, p.snapshotBlock);
        if (weight == 0) revert NoVotingPower();
        hasVoted[id][msg.sender] = true;
        if (support == uint8(VoteType.For)) p.forVotes += weight;
        else if (support == uint8(VoteType.Against)) p.againstVotes += weight;
        else p.abstainVotes += weight;
        emit VoteCast(msg.sender, id, support, weight);
    }

    /// @inheritdoc IGovernance
    function queue(uint256 id) external {
        ProposalState s = state(id);
        if (s != ProposalState.Succeeded) revert InvalidState(s);
        uint64 eta = uint64(block.timestamp + _params.timelockDelay);
        _proposals[id].eta = eta;
        emit ProposalQueued(id, eta);
    }

    /// @inheritdoc IGovernance
    /// @dev Anyone may execute a queued proposal once the timelock expired. Calls run in order; any failure
    ///      reverts the whole execution (the proposal stays Queued and can be retried until it expires).
    function execute(uint256 id) external payable nonReentrant {
        ProposalState s = state(id);
        if (s != ProposalState.Queued) revert InvalidState(s);
        Proposal storage p = _proposals[id];
        if (block.timestamp < p.eta) revert TimelockNotExpired(p.eta);
        p.executed = true;
        emit ProposalExecuted(id);
        uint256 n = p.targets.length;
        for (uint256 i; i < n; ++i) {
            (bool ok, bytes memory ret) = p.targets[i].call{value: p.values[i]}(p.calldatas[i]);
            if (!ok) revert CallFailed(i, ret);
        }
    }

    /// @inheritdoc IGovernance
    function cancel(uint256 id) external {
        ProposalState s = state(id);
        if (s != ProposalState.Active && s != ProposalState.Succeeded && s != ProposalState.Queued) {
            revert InvalidState(s);
        }
        Proposal storage p = _proposals[id];
        if (msg.sender != p.proposer) revert OnlyProposer();
        p.canceled = true;
        emit ProposalCanceled(id);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Emergency pause (blocks NEW proposals only)
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IGovernance
    function pause() external {
        if (msg.sender != guardian || guardian == address(0)) revert OnlyGuardian();
        if (paused() || block.timestamp < nextPauseAllowedAt) revert PauseCooldown();
        uint64 until = uint64(block.timestamp + _params.pauseDuration);
        pausedUntil = until;
        nextPauseAllowedAt = until + _params.pauseDuration;
        emit Paused(msg.sender, until);
    }

    /// @inheritdoc IGovernance
    /// @dev Callable by the guardian or by Governance itself (a proposal queued before the pause).
    function unpause() external {
        if (msg.sender != guardian && msg.sender != address(this)) revert OnlyGuardian();
        if (!paused()) revert NotPaused();
        pausedUntil = uint64(block.timestamp);
        emit Unpaused(msg.sender);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Self-administration (only through an executed proposal)
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IGovernance
    function setParams(Params calldata p) external onlySelf {
        _setParams(p);
        emit ParamsUpdated(p);
    }

    /// @inheritdoc IGovernance
    function setGuardian(address newGuardian) external onlySelf {
        emit GuardianUpdated(guardian, newGuardian);
        guardian = newGuardian;
    }

    // ---------------------------------------------------------------------------------------------------------
    // Views
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IGovernance
    function state(uint256 id) public view returns (ProposalState) {
        Proposal storage p = _proposals[id];
        if (p.proposer == address(0)) revert UnknownProposal();
        if (p.executed) return ProposalState.Executed;
        if (p.canceled) return ProposalState.Canceled;
        if (block.timestamp < p.voteEnd) return ProposalState.Active;
        if (!_succeeded(p)) return ProposalState.Defeated;
        if (p.eta == 0) return ProposalState.Succeeded;
        if (block.timestamp > uint256(p.eta) + _params.gracePeriod) return ProposalState.Expired;
        return ProposalState.Queued;
    }

    /// @inheritdoc IGovernance
    function paused() public view returns (bool) {
        return block.timestamp < pausedUntil;
    }

    function params() external view returns (Params memory) {
        return _params;
    }

    function getProposal(uint256 id)
        external
        view
        returns (
            address proposer,
            uint256 snapshotBlock,
            uint64 voteEnd,
            uint64 eta,
            uint256 quorumVotes,
            uint256 forVotes,
            uint256 againstVotes,
            uint256 abstainVotes
        )
    {
        Proposal storage p = _proposals[id];
        return (
            p.proposer,
            p.snapshotBlock,
            p.voteEnd,
            p.eta,
            p.quorumVotes,
            p.forVotes,
            p.againstVotes,
            p.abstainVotes
        );
    }

    function getActions(uint256 id)
        external
        view
        returns (address[] memory targets, uint256[] memory values, bytes[] memory calldatas)
    {
        Proposal storage p = _proposals[id];
        return (p.targets, p.values, p.calldatas);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Internal
    // ---------------------------------------------------------------------------------------------------------

    function _succeeded(Proposal storage p) internal view returns (bool) {
        uint256 forV = p.forVotes;
        if (forV == 0) return false;
        if (forV + p.abstainVotes < p.quorumVotes) return false;
        return forV * BPS > (forV + p.againstVotes) * p.thresholdBps;
    }

    function _setParams(Params calldata p) internal {
        if (p.votingPeriod < 1 hours || p.votingPeriod > 30 days) revert InvalidParam("votingPeriod");
        if (p.timelockDelay < 1 hours || p.timelockDelay > 30 days) revert InvalidParam("timelockDelay");
        if (p.gracePeriod < 1 days || p.gracePeriod > 60 days) revert InvalidParam("gracePeriod");
        if (p.pauseDuration > 14 days) revert InvalidParam("pauseDuration");
        if (p.quorumBps == 0 || p.quorumBps > BPS) revert InvalidParam("quorumBps");
        if (p.thresholdBps < BPS / 2 || p.thresholdBps >= BPS) revert InvalidParam("thresholdBps");
        _params = p;
    }
}
