// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {Checkpoints} from "@openzeppelin/contracts/utils/structs/Checkpoints.sol";
import {SafeCast} from "@openzeppelin/contracts/utils/math/SafeCast.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {Predeploys} from "../common/Predeploys.sol";
import {IStaking} from "./interfaces/IStaking.sol";

/// @title Staking — xDEVS Chain validator set, delegation, rewards and slashing (predeploy 0x…1000)
/// @author THE xDEVS
/// @notice Implements the validator lifecycle of plan §11.4:
///
///   [*] → Candidate (register) → Active (top-N at epoch) → Candidate (dropped)
///   Active → Jailed (downtime) → Candidate (unjail after jail period)
///   any registered → Tombstoned (double-sign, permanent)
///   Candidate/Active/Jailed → Exited (operator withdraws all self-stake; stake unbonds) → [*]
///
/// @dev GENESIS: this contract is written into the genesis state by the Rust genesis builder. The constructor never
///      runs at 0x…1000; the builder calls {initialize} once as SYSTEM_ADDRESS and dumps the resulting storage.
///      Not a proxy: upgrades are fork-height code replacements (see docs/specs/predeploys-system.md).
///
///      FUNDING OF REWARDS: rewards are "whatever native balance is not accounted for". Before calling
///      {recordParticipation} in a block, the node credits the block reward to this contract either by a direct
///      balance increase (preferred, same mechanism as the 20% base-fee credit to Treasury) or by sending it as
///      `msg.value` of the system call. Every wei held for stakes, unbonding, owed rewards or slashed stake is
///      tracked in {accountedBalance}; `address(this).balance - accountedBalance` is distributed on the next call.
///
///      Accounting invariant (checked by the invariant suite):
///        accountedBalance == totalBonded + totalUnbonding + totalRewardsReserved + totalSlashed <= balance.
///      Slashed stake stays locked in this contract forever (economically burned).
///
///      Delegation uses shares per validator so that slashing is O(1) and pro-rata. Undelegated stake moves to a
///      per-validator unbonding pool (also share-based) so a later double-sign slash also hits unbonding stake.
contract Staking is IStaking, ReentrancyGuardTransient {
    using Checkpoints for Checkpoints.Trace208;

    // ---------------------------------------------------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------------------------------------------------

    uint256 internal constant BPS = 10_000;
    /// @dev Fixed-point precision of `accRewardPerShare`.
    uint256 internal constant ACC_PRECISION = 1e30;
    /// @dev Width of the signer bitmap passed to {recordParticipation}.
    uint256 internal constant MAX_ACTIVE_HARD_CAP = 256;
    uint256 internal constant MAX_CANDIDATES_HARD_CAP = 2_000;
    /// @dev No slash may take the whole stake (keeps share pools well defined).
    uint256 internal constant MAX_SLASH_BPS = 5_000;
    uint256 public constant MAX_UNBONDING_ENTRIES = 32;

    uint8 internal constant REASON_DOUBLE_SIGN = 1;
    uint8 internal constant REASON_DOWNTIME = 2;
    /// @notice Evidence kind for {slash}: duplicate vote (double-sign).
    uint256 public constant EVIDENCE_DOUBLE_SIGN = 1;

    // ---------------------------------------------------------------------------------------------------------
    // Errors
    // ---------------------------------------------------------------------------------------------------------

    error OnlySystem();
    error OnlyGovernance();
    error AlreadyInitialized();
    error NotInitialized();
    error InvalidParam(string name);
    error InvalidGenesis(string reason);
    error AlreadyRegistered();
    error InvalidConsensusKey();
    error ConsensusKeyInUse();
    error InsufficientSelfStake();
    error CommissionTooHigh();
    error CommissionChangeTooLarge();
    error CommissionChangeTooSoon();
    error TooManyCandidates();
    error UnknownValidator();
    error ValidatorNotAcceptingStake();
    error DelegationTooSmall();
    error ZeroAmount();
    error InsufficientDelegation();
    error SelfStakeBelowMinimum();
    error LastActiveValidator();
    error TooManyUnbondingEntries();
    error NothingToWithdraw();
    error NothingToClaim();
    error NotJailed();
    error JailPeriodNotOver();
    error TransferFailed();
    error AlreadyCalledThisBlock();
    error FutureLookup();

    // ---------------------------------------------------------------------------------------------------------
    // Storage — slots 0..6 are read by the node (see spec). DO NOT REORDER.
    // ---------------------------------------------------------------------------------------------------------

    struct Validator {
        bytes32 consensusKey;
        uint256 totalStake; // bonded stake (wei) backing all shares
        uint256 totalShares;
        uint256 unbondingStake; // unbonding pool stake
        uint256 unbondingShares; // unbonding pool shares
        uint256 accRewardPerShare; // scaled by ACC_PRECISION
        uint256 commissionAccrued; // claimable by the operator
        ValidatorStatus status;
        uint16 commissionBps;
        uint64 jailedUntil;
        uint64 lastCommissionChange;
        uint32 candidateIndex; // 1-based index in `_candidates`, 0 = not a candidate
        uint64 windowId; // liveness window the two counters below belong to
        uint64 windowExpected; // blocks in that window during which the validator was in the signing set
        uint64 windowSigned; // ... and signed
    }

    struct Delegation {
        uint256 shares;
        uint256 principal; // bonded amount used for governance voting power
        uint256 rewardDebt;
        uint256 rewardsPending;
    }

    /// @dev slot 0: operators of the active (consensus) set, sorted by power desc, address asc.
    address[] internal _activeOperators;
    /// @dev slot 1: consensus (Ed25519) public keys, parallel to `_activeOperators`.
    bytes32[] internal _activeKeys;
    /// @dev slot 2: voting power (bonded stake in wei at selection time), parallel to `_activeOperators`.
    uint256[] internal _activePowers;
    /// @dev slot 3 (packed): currentEpoch | setVersion | lastSetChangeBlock | lastEpochBlock
    uint64 public currentEpoch;
    uint64 public setVersion;
    uint64 public lastSetChangeBlock;
    uint64 public lastEpochBlock;
    /// @dev slot 4
    bytes32 public validatorSetHash;
    /// @dev slot 5/6: the set that was in force before the last change (used for the commit of that block).
    address[] internal _prevOperators;
    uint256[] internal _prevPowers;

    /// @dev slot 7 (packed): governance | initialized
    address public governance;
    bool public initialized;
    /// @dev slots 8..11
    Params internal _params;

    /// @dev slot 12..16: global accounting
    uint256 public accountedBalance;
    uint256 public totalBonded;
    uint256 public totalUnbonding;
    uint256 public totalRewardsReserved;
    uint256 public totalSlashed;

    /// @dev slot 17 (packed): liveness window bookkeeping
    uint64 public windowId;
    uint64 public windowBlocks;
    uint64 public lastParticipationBlock;

    address[] internal _candidates;
    mapping(address operator => Validator) internal _validators;
    mapping(address validator => mapping(address delegator => Delegation)) internal _delegations;
    mapping(address delegator => UnbondingEntry[]) internal _unbonding;
    mapping(bytes32 consensusKey => address operator) public consensusKeyOwner;
    mapping(bytes32 evidenceHash => bool) public evidenceProcessed;
    mapping(address account => Checkpoints.Trace208) internal _votes;
    Checkpoints.Trace208 internal _totalVotes;

    // ---------------------------------------------------------------------------------------------------------
    // Modifiers
    // ---------------------------------------------------------------------------------------------------------

    modifier onlySystem() {
        if (msg.sender != Predeploys.SYSTEM_ADDRESS) revert OnlySystem();
        _;
    }

    modifier onlyGovernance() {
        if (msg.sender != governance) revert OnlyGovernance();
        _;
    }

    modifier whenInitialized() {
        if (!initialized) revert NotInitialized();
        _;
    }

    /// @dev Intentionally empty: never runs at the predeploy address.
    constructor() {}

    // ---------------------------------------------------------------------------------------------------------
    // Initialization (genesis builder, as SYSTEM_ADDRESS)
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IStaking
    /// @dev The genesis builder must set this contract's balance >= sum of genesis stakes BEFORE calling.
    ///      Any excess balance is treated as undistributed rewards and paid out on the first block.
    function initialize(address governance_, Params calldata p, GenesisValidator[] calldata vals)
        external
        onlySystem
    {
        if (initialized) revert AlreadyInitialized();
        if (governance_ == address(0)) revert InvalidGenesis("governance");
        if (vals.length == 0) revert InvalidGenesis("no validators");
        initialized = true;
        governance = governance_;
        _setParams(p);
        windowId = 1;

        uint256 total;
        for (uint256 i; i < vals.length; ++i) {
            GenesisValidator calldata g = vals[i];
            if (g.operator == address(0)) revert InvalidGenesis("operator");
            if (_validators[g.operator].status != ValidatorStatus.None) revert InvalidGenesis("duplicate");
            if (g.consensusKey == bytes32(0) || consensusKeyOwner[g.consensusKey] != address(0)) {
                revert InvalidGenesis("consensus key");
            }
            if (g.stake < p.minSelfStake) revert InvalidGenesis("stake");
            if (g.commissionBps > p.maxCommissionBps) revert InvalidGenesis("commission");
            if (_candidates.length >= p.maxCandidates) revert InvalidGenesis("too many");
            Validator storage v = _createValidator(g.operator, g.consensusKey, g.commissionBps);
            _bond(g.operator, g.operator, v, g.stake);
            total += g.stake;
        }
        if (total > address(this).balance) revert InvalidGenesis("balance");
        accountedBalance = total;

        // Genesis set = epoch 0. The previous set equals the genesis set so the first commit maps correctly.
        (address[] memory ops, uint256[] memory powers) = _computeTopN();
        _writeActiveSet(ops, powers);
        _prevOperators = _activeOperators;
        _prevPowers = _activePowers;
        uint64 bn = uint64(block.number);
        lastSetChangeBlock = bn;
        lastEpochBlock = bn;
        lastParticipationBlock = bn;

        emit Initialized(governance_, vals.length, total);
        emit ParamsUpdated(p);
        emit EpochTransition(0, validatorSetHash);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Validator operations
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IStaking
    function register(bytes32 consensusKey, uint16 commissionBps)
        external
        payable
        nonReentrant
        whenInitialized
    {
        Params storage p = _params;
        if (_validators[msg.sender].status != ValidatorStatus.None) revert AlreadyRegistered();
        if (consensusKey == bytes32(0)) revert InvalidConsensusKey();
        if (consensusKeyOwner[consensusKey] != address(0)) revert ConsensusKeyInUse();
        if (msg.value < p.minSelfStake) revert InsufficientSelfStake();
        if (commissionBps > p.maxCommissionBps) revert CommissionTooHigh();
        if (_candidates.length >= p.maxCandidates) revert TooManyCandidates();

        Validator storage v = _createValidator(msg.sender, consensusKey, commissionBps);
        accountedBalance += msg.value;
        _bond(msg.sender, msg.sender, v, msg.value);
        emit ValidatorRegistered(msg.sender, consensusKey, msg.value);
    }

    /// @inheritdoc IStaking
    function setCommission(uint16 newBps) external whenInitialized {
        Validator storage v = _validators[msg.sender];
        ValidatorStatus s = v.status;
        if (s == ValidatorStatus.None || s == ValidatorStatus.Tombstoned || s == ValidatorStatus.Exited) {
            revert UnknownValidator();
        }
        Params storage p = _params;
        if (newBps > p.maxCommissionBps) revert CommissionTooHigh();
        uint16 old = v.commissionBps;
        uint256 diff = newBps > old ? newBps - old : old - newBps;
        if (diff > p.maxCommissionChangeBps) revert CommissionChangeTooLarge();
        if (block.timestamp < uint256(v.lastCommissionChange) + p.commissionChangeInterval) {
            revert CommissionChangeTooSoon();
        }
        v.commissionBps = newBps;
        v.lastCommissionChange = uint64(block.timestamp);
        emit CommissionChanged(msg.sender, old, newBps);
    }

    /// @inheritdoc IStaking
    function unjail() external whenInitialized {
        Validator storage v = _validators[msg.sender];
        if (v.status != ValidatorStatus.Jailed) revert NotJailed();
        if (block.timestamp < v.jailedUntil) revert JailPeriodNotOver();
        if (_selfStake(msg.sender, v) < _params.minSelfStake) revert InsufficientSelfStake();
        v.status = ValidatorStatus.Candidate;
        v.jailedUntil = 0;
        emit Unjailed(msg.sender);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Delegation
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IStaking
    /// @dev The operator tops up its own self-stake with `delegate(operator)`.
    function delegate(address validator) external payable nonReentrant whenInitialized {
        Validator storage v = _validators[validator];
        ValidatorStatus s = v.status;
        if (s != ValidatorStatus.Candidate && s != ValidatorStatus.Active && s != ValidatorStatus.Jailed) {
            revert ValidatorNotAcceptingStake();
        }
        if (msg.value < _params.minDelegation) revert DelegationTooSmall();
        accountedBalance += msg.value;
        _bond(msg.sender, validator, v, msg.value);
    }

    /// @inheritdoc IStaking
    /// @dev `amount == type(uint256).max` undelegates the whole position. If the operator undelegates its own
    ///      stake, the remaining self-stake must stay >= minSelfStake, or be zero (which exits the validator).
    function undelegate(address validator, uint256 amount) external nonReentrant whenInitialized {
        if (amount == 0) revert ZeroAmount();
        Validator storage v = _validators[validator];
        if (v.status == ValidatorStatus.None) revert UnknownValidator();
        Delegation storage d = _delegations[validator][msg.sender];
        uint256 dShares = d.shares;
        if (dShares == 0) revert InsufficientDelegation();
        if (_unbonding[msg.sender].length >= MAX_UNBONDING_ENTRIES) revert TooManyUnbondingEntries();

        uint256 vShares = v.totalShares;
        uint256 vStake = v.totalStake;
        uint256 shares;
        if (amount == type(uint256).max) {
            shares = dShares;
            amount = Math.mulDiv(shares, vStake, vShares);
        } else {
            shares = Math.mulDiv(amount, vShares, vStake, Math.Rounding.Ceil);
            if (shares > dShares) revert InsufficientDelegation();
        }
        // Burning every share of the validator releases the whole stake (no dust left behind).
        if (shares == vShares) amount = vStake;
        if (amount == 0) revert ZeroAmount();

        // Rewards are settled on the old share balance before it changes.
        _settle(v, d);
        uint256 principalOut = shares == dShares ? d.principal : Math.mulDiv(d.principal, shares, dShares);
        d.shares = dShares - shares;
        d.principal -= principalOut;
        d.rewardDebt = _debtFor(d.shares, v.accRewardPerShare);
        v.totalShares = vShares - shares;
        v.totalStake = vStake - amount;
        totalBonded -= amount;
        _subVotes(msg.sender, principalOut);

        // Move to the validator's unbonding pool.
        uint256 poolShares = (v.unbondingShares == 0 || v.unbondingStake == 0)
            ? amount
            : Math.mulDiv(amount, v.unbondingShares, v.unbondingStake);
        v.unbondingShares += poolShares;
        v.unbondingStake += amount;
        totalUnbonding += amount;
        uint64 completion = uint64(block.timestamp + _params.unbondingSeconds);
        _unbonding[msg.sender].push(UnbondingEntry(validator, completion, poolShares));

        // Operator self-stake rules.
        if (msg.sender == validator) {
            ValidatorStatus s = v.status;
            if (s == ValidatorStatus.Candidate || s == ValidatorStatus.Active || s == ValidatorStatus.Jailed)
            {
                if (d.shares == 0) {
                    _exit(validator, v);
                } else if (_selfStake(validator, v) < _params.minSelfStake) {
                    revert SelfStakeBelowMinimum();
                }
            }
        }
        emit Undelegated(msg.sender, validator, amount, shares, completion);
    }

    /// @inheritdoc IStaking
    // aderyn-fp-next-line(eth-send-unchecked-address) SA-F03: pull payment to msg.sender
    function withdrawUnbonded() external nonReentrant {
        UnbondingEntry[] storage entries = _unbonding[msg.sender];
        uint256 total;
        uint256 matured;
        uint256 i;
        while (i < entries.length) {
            UnbondingEntry memory e = entries[i];
            if (e.completionTime <= block.timestamp) {
                Validator storage v = _validators[e.validator];
                uint256 poolShares = v.unbondingShares;
                uint256 amt = e.poolShares == poolShares
                    ? v.unbondingStake
                    : Math.mulDiv(e.poolShares, v.unbondingStake, poolShares);
                v.unbondingShares = poolShares - e.poolShares;
                v.unbondingStake -= amt;
                total += amt;
                ++matured;
                entries[i] = entries[entries.length - 1];
                entries.pop();
            } else {
                ++i;
            }
        }
        if (matured == 0) revert NothingToWithdraw();
        totalUnbonding -= total;
        accountedBalance -= total;
        emit Withdrawn(msg.sender, total);
        _send(msg.sender, total);
    }

    /// @inheritdoc IStaking
    /// @dev The validator operator also receives its accrued commission when claiming on its own validator.
    function claimRewards(address validator) external nonReentrant {
        Validator storage v = _validators[validator];
        if (v.status == ValidatorStatus.None) revert UnknownValidator();
        Delegation storage d = _delegations[validator][msg.sender];
        _settle(v, d);
        uint256 amount = d.rewardsPending;
        d.rewardsPending = 0;
        if (msg.sender == validator) {
            amount += v.commissionAccrued;
            v.commissionAccrued = 0;
        }
        if (amount == 0) revert NothingToClaim();
        totalRewardsReserved -= amount;
        accountedBalance -= amount;
        emit RewardsClaimed(msg.sender, validator, amount);
        _send(msg.sender, amount);
    }

    // ---------------------------------------------------------------------------------------------------------
    // System calls (msg.sender == SYSTEM_ADDRESS)
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IStaking
    /// @dev Called once per block, FIRST among the system calls of that block. `signedBitmap` bit i refers to
    ///      index i of {signingSet} (the set that signed the previous block's commit). `proposer` is the operator
    ///      address of this block's proposer. Distributes `balance - accountedBalance` as rewards.
    function recordParticipation(address proposer, uint256 signedBitmap)
        external
        payable
        onlySystem
        whenInitialized
    {
        uint64 bn = uint64(block.number);
        if (bn <= lastParticipationBlock) revert AlreadyCalledThisBlock();
        lastParticipationBlock = bn;

        // slither-disable-next-line incorrect-equality (SA-F07: block-number bookkeeping, not a balance)
        bool usePrev = bn - 1 == lastSetChangeBlock;
        address[] storage ops = usePrev ? _prevOperators : _activeOperators;
        uint256[] storage powers = usePrev ? _prevPowers : _activePowers;
        uint256 len = ops.length;

        // 1. Liveness counters + signed power.
        uint64 wid = windowId;
        uint256 signedPower;
        for (uint256 i; i < len; ++i) {
            Validator storage v = _validators[ops[i]];
            if (v.windowId != wid) {
                v.windowId = wid;
                v.windowExpected = 0;
                v.windowSigned = 0;
            }
            ++v.windowExpected;
            if ((signedBitmap >> i) & 1 == 1) {
                ++v.windowSigned;
                if (_rewardable(v)) signedPower += powers[i];
            }
        }

        // 2. Rewards.
        uint256 reward = address(this).balance - accountedBalance;
        if (reward > 0 && signedPower > 0) {
            uint256 distributed;
            Validator storage pv = _validators[proposer];
            if (_rewardable(pv)) {
                uint256 bonus = reward * _params.proposerBonusBps / BPS;
                _credit(pv, bonus);
                distributed = bonus;
            }
            uint256 pool = reward - distributed;
            for (uint256 i; i < len; ++i) {
                if ((signedBitmap >> i) & 1 == 0) continue;
                Validator storage v = _validators[ops[i]];
                if (!_rewardable(v)) continue;
                uint256 share = Math.mulDiv(pool, powers[i], signedPower);
                _credit(v, share);
                distributed += share;
            }
            accountedBalance += distributed;
            totalRewardsReserved += distributed;
            emit RewardsDistributed(bn, proposer, distributed);
        }

        // 3. Close the liveness window.
        if (++windowBlocks >= _params.downtimeWindow) {
            _evaluateDowntime(wid);
            windowId = wid + 1;
            windowBlocks = 0;
        }
    }

    /// @inheritdoc IStaking
    /// @dev Recomputes the active set = top-`maxValidators` eligible candidates by bonded stake, ties broken by
    ///      lower address. If no candidate is eligible the current set is kept (the chain must not lose its set).
    function onEpoch() external onlySystem whenInitialized {
        uint64 bn = uint64(block.number);
        if (bn <= lastEpochBlock) revert AlreadyCalledThisBlock();
        lastEpochBlock = bn;
        uint64 epoch = ++currentEpoch;

        (address[] memory ops, uint256[] memory powers) = _computeTopN();
        if (ops.length != 0) {
            _snapshotPrevSet();
            _writeActiveSet(ops, powers);
        }
        emit EpochTransition(epoch, validatorSetHash);
    }

    /// @inheritdoc IStaking
    /// @dev `evidence = abi.encode(uint256 kind, address validator, uint64 infractionHeight, bytes32 ref)`.
    ///      The node verifies the two conflicting Ed25519 votes BEFORE calling (the EVM cannot); `ref` is the hash
    ///      of the evidence object for traceability. Invalid/duplicate evidence is ignored with an event instead
    ///      of reverting, so block execution never fails on it.
    function slash(bytes calldata evidence) external onlySystem whenInitialized {
        bytes32 eh = keccak256(evidence);
        if (evidenceProcessed[eh]) {
            emit EvidenceIgnored(eh, 1);
            return;
        }
        if (evidence.length != 128) {
            emit EvidenceIgnored(eh, 2);
            return;
        }
        (uint256 kind, uint256 rawValidator, uint256 rawHeight, bytes32 ref) =
            abi.decode(evidence, (uint256, uint256, uint256, bytes32));
        if (rawValidator > type(uint160).max || rawHeight > type(uint64).max) {
            emit EvidenceIgnored(eh, 2);
            return;
        }
        if (kind != EVIDENCE_DOUBLE_SIGN) {
            emit EvidenceIgnored(eh, 3);
            return;
        }
        address validator = address(uint160(rawValidator));
        Validator storage v = _validators[validator];
        if (v.status == ValidatorStatus.None || v.status == ValidatorStatus.Tombstoned) {
            emit EvidenceIgnored(eh, 4);
            return;
        }
        evidenceProcessed[eh] = true;
        _tombstone(validator, v, uint64(rawHeight), ref);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Governance
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IStaking
    function setParams(Params calldata p) external onlyGovernance {
        _setParams(p);
        emit ParamsUpdated(p);
    }

    // ---------------------------------------------------------------------------------------------------------
    // Views
    // ---------------------------------------------------------------------------------------------------------

    /// @inheritdoc IStaking
    function activeValidators() external view returns (address[] memory, uint256[] memory power) {
        return (_activeOperators, _activePowers);
    }

    /// @inheritdoc IStaking
    function consensusValidatorSet()
        external
        view
        returns (address[] memory operators, bytes32[] memory consensusKeys, uint256[] memory powers)
    {
        return (_activeOperators, _activeKeys, _activePowers);
    }

    /// @inheritdoc IStaking
    /// @dev The set whose commit is recorded by {recordParticipation} in the CURRENT block.
    function signingSet() external view returns (address[] memory operators, uint256[] memory powers) {
        // slither-disable-next-line incorrect-equality (SA-F07: block-number bookkeeping, not a balance)
        if (block.number != 0 && block.number - 1 == lastSetChangeBlock) {
            return (_prevOperators, _prevPowers);
        }
        return (_activeOperators, _activePowers);
    }

    /// @inheritdoc IStaking
    function params() external view returns (Params memory) {
        return _params;
    }

    /// @inheritdoc IStaking
    function candidates() external view returns (address[] memory) {
        return _candidates;
    }

    /// @inheritdoc IStaking
    function getValidator(address validator) external view returns (ValidatorInfo memory info) {
        Validator storage v = _validators[validator];
        info = ValidatorInfo({
            consensusKey: v.consensusKey,
            status: v.status,
            commissionBps: v.commissionBps,
            jailedUntil: v.jailedUntil,
            lastCommissionChange: v.lastCommissionChange,
            totalStake: v.totalStake,
            totalShares: v.totalShares,
            selfStake: _selfStake(validator, v),
            unbondingStake: v.unbondingStake,
            unbondingShares: v.unbondingShares,
            commissionAccrued: v.commissionAccrued
        });
    }

    /// @inheritdoc IStaking
    function selfStake(address validator) external view returns (uint256) {
        return _selfStake(validator, _validators[validator]);
    }

    /// @inheritdoc IStaking
    function delegationOf(address validator, address delegator)
        external
        view
        returns (uint256 shares, uint256 amount, uint256 principal)
    {
        Validator storage v = _validators[validator];
        Delegation storage d = _delegations[validator][delegator];
        shares = d.shares;
        amount = v.totalShares == 0 ? 0 : Math.mulDiv(shares, v.totalStake, v.totalShares);
        principal = d.principal;
    }

    /// @inheritdoc IStaking
    function pendingRewards(address validator, address delegator) external view returns (uint256) {
        Validator storage v = _validators[validator];
        Delegation storage d = _delegations[validator][delegator];
        uint256 accrued = Math.mulDiv(d.shares, v.accRewardPerShare, ACC_PRECISION);
        uint256 amount = d.rewardsPending + (accrued > d.rewardDebt ? accrued - d.rewardDebt : 0);
        if (validator == delegator) amount += v.commissionAccrued;
        return amount;
    }

    /// @inheritdoc IStaking
    function unbondingEntries(address delegator) external view returns (UnbondingEntry[] memory) {
        return _unbonding[delegator];
    }

    /// @inheritdoc IStaking
    function unbondingValue(address delegator) external view returns (uint256 total, uint256 withdrawable) {
        UnbondingEntry[] storage entries = _unbonding[delegator];
        for (uint256 i; i < entries.length; ++i) {
            UnbondingEntry storage e = entries[i];
            Validator storage v = _validators[e.validator];
            uint256 amt = Math.mulDiv(e.poolShares, v.unbondingStake, v.unbondingShares);
            total += amt;
            if (e.completionTime <= block.timestamp) withdrawable += amt;
        }
    }

    /// @inheritdoc IStaking
    function getVotes(address account) external view returns (uint256) {
        return _votes[account].latest();
    }

    /// @inheritdoc IStaking
    function getPastVotes(address account, uint256 blockNumber) external view returns (uint256) {
        if (blockNumber >= block.number) revert FutureLookup();
        return _votes[account].upperLookupRecent(SafeCast.toUint48(blockNumber));
    }

    /// @inheritdoc IStaking
    function getPastTotalVotes(uint256 blockNumber) external view returns (uint256) {
        if (blockNumber >= block.number) revert FutureLookup();
        return _totalVotes.upperLookupRecent(SafeCast.toUint48(blockNumber));
    }

    // ---------------------------------------------------------------------------------------------------------
    // Internal: lifecycle
    // ---------------------------------------------------------------------------------------------------------

    function _createValidator(address operator, bytes32 key, uint16 commissionBps)
        internal
        returns (Validator storage v)
    {
        v = _validators[operator];
        v.consensusKey = key;
        v.status = ValidatorStatus.Candidate;
        v.commissionBps = commissionBps;
        v.lastCommissionChange = uint64(block.timestamp);
        consensusKeyOwner[key] = operator;
        _candidates.push(operator);
        v.candidateIndex = uint32(_candidates.length);
    }

    /// @dev Adds `amount` (already counted in accountedBalance) of bonded stake for `delegator`.
    function _bond(address delegator, address validator, Validator storage v, uint256 amount) internal {
        uint256 vShares = v.totalShares;
        uint256 shares = vShares == 0 ? amount : Math.mulDiv(amount, vShares, v.totalStake);
        if (shares == 0) revert DelegationTooSmall();
        Delegation storage d = _delegations[validator][delegator];
        _settle(v, d);
        d.shares += shares;
        d.principal += amount;
        d.rewardDebt = _debtFor(d.shares, v.accRewardPerShare);
        v.totalShares = vShares + shares;
        v.totalStake += amount;
        // slither-disable-next-line costly-loop (SA-I04: looped only by the one-time genesis initialize)
        totalBonded += amount;
        _addVotes(delegator, amount);
        emit Delegated(delegator, validator, amount);
    }

    /// @dev Moves rewards accrued since the last settlement into `rewardsPending`. Accrual is rounded DOWN and the
    ///      debt set on share changes is rounded UP ({_debtFor}), so the sum of what delegators can claim never
    ///      exceeds what was credited (the classic MasterChef "+1 wei per delegator" overshoot is impossible).
    function _settle(Validator storage v, Delegation storage d) internal {
        uint256 accrued = Math.mulDiv(d.shares, v.accRewardPerShare, ACC_PRECISION);
        uint256 debt = d.rewardDebt;
        if (accrued > debt) {
            d.rewardsPending += accrued - debt;
            d.rewardDebt = accrued;
        }
    }

    function _debtFor(uint256 shares, uint256 acc) internal pure returns (uint256) {
        return Math.mulDiv(shares, acc, ACC_PRECISION, Math.Rounding.Ceil);
    }

    function _credit(Validator storage v, uint256 amount) internal {
        uint256 commission = amount * v.commissionBps / BPS;
        v.commissionAccrued += commission;
        v.accRewardPerShare += Math.mulDiv(amount - commission, ACC_PRECISION, v.totalShares);
    }

    function _rewardable(Validator storage v) internal view returns (bool) {
        ValidatorStatus s = v.status;
        return s != ValidatorStatus.None && s != ValidatorStatus.Tombstoned && v.totalShares != 0;
    }

    function _exit(address validator, Validator storage v) internal {
        bool wasActive = v.status == ValidatorStatus.Active;
        if (wasActive && _activeOperators.length == 1) revert LastActiveValidator();
        v.status = ValidatorStatus.Exited;
        if (wasActive) _removeFromActiveSet(validator);
        _removeCandidate(v);
        emit ValidatorExited(validator);
    }

    function _evaluateDowntime(uint64 wid) internal {
        Params storage p = _params;
        address[] memory set = _activeOperators;
        for (uint256 i; i < set.length; ++i) {
            Validator storage v = _validators[set[i]];
            if (v.windowId != wid || v.windowExpected == 0) continue;
            if (uint256(v.windowSigned) * BPS >= uint256(v.windowExpected) * p.minSignedBps) continue;
            // Never jail the last remaining active validator (that would leave the chain without a set).
            if (_activeOperators.length == 1) break;
            uint256 amt = v.totalStake * p.downtimeSlashBps / BPS;
            v.totalStake -= amt;
            totalBonded -= amt;
            totalSlashed += amt;
            emit Slashed(set[i], amt, REASON_DOWNTIME);
            uint64 until = uint64(block.timestamp + p.downtimeJailSeconds);
            v.status = ValidatorStatus.Jailed;
            v.jailedUntil = until;
            _removeFromActiveSet(set[i]);
            emit Jailed(set[i], until);
        }
    }

    function _tombstone(address validator, Validator storage v, uint64 height, bytes32 ref) internal {
        uint256 bps = _params.doubleSignSlashBps;
        uint256 bondedCut = v.totalStake * bps / BPS;
        uint256 unbondingCut = v.unbondingStake * bps / BPS;
        v.totalStake -= bondedCut;
        v.unbondingStake -= unbondingCut;
        totalBonded -= bondedCut;
        totalUnbonding -= unbondingCut;
        totalSlashed += bondedCut + unbondingCut;
        emit Slashed(validator, bondedCut + unbondingCut, REASON_DOUBLE_SIGN);

        ValidatorStatus s = v.status;
        v.status = ValidatorStatus.Tombstoned;
        v.jailedUntil = type(uint64).max;
        if (s == ValidatorStatus.Active) _removeFromActiveSet(validator);
        _removeCandidate(v);
        emit Tombstoned(validator, height, ref);
    }

    function _removeCandidate(Validator storage v) internal {
        uint256 idx = v.candidateIndex;
        if (idx == 0) return;
        uint256 last = _candidates.length;
        if (idx != last) {
            address moved = _candidates[last - 1];
            _candidates[idx - 1] = moved;
            _validators[moved].candidateIndex = uint32(idx);
        }
        _candidates.pop();
        v.candidateIndex = 0;
    }

    // ---------------------------------------------------------------------------------------------------------
    // Internal: active set
    // ---------------------------------------------------------------------------------------------------------

    function _isEligible(address operator, Validator storage v) internal view returns (bool) {
        ValidatorStatus s = v.status;
        if (s != ValidatorStatus.Candidate && s != ValidatorStatus.Active) return false;
        return v.totalStake != 0 && _selfStake(operator, v) >= _params.minSelfStake;
    }

    /// @dev Strict total order: higher stake first, then lower address first.
    function _ranksBefore(uint256 stakeA, address a, uint256 stakeB, address b) internal pure returns (bool) {
        return stakeA > stakeB || (stakeA == stakeB && a < b);
    }

    /// @dev Bounded partial insertion sort: O(candidates * maxValidators) comparisons, all in memory.
    function _computeTopN() internal view returns (address[] memory ops, uint256[] memory powers) {
        uint256 n = _params.maxValidators;
        address[] memory cands = _candidates;
        ops = new address[](n);
        powers = new uint256[](n);
        uint256 count;
        for (uint256 i; i < cands.length; ++i) {
            address a = cands[i];
            Validator storage v = _validators[a];
            if (!_isEligible(a, v)) continue;
            uint256 stake = v.totalStake;
            uint256 pos;
            if (count < n) {
                pos = count++;
            } else if (_ranksBefore(stake, a, powers[n - 1], ops[n - 1])) {
                pos = n - 1;
            } else {
                continue;
            }
            while (pos > 0 && _ranksBefore(stake, a, powers[pos - 1], ops[pos - 1])) {
                ops[pos] = ops[pos - 1];
                powers[pos] = powers[pos - 1];
                --pos;
            }
            ops[pos] = a;
            powers[pos] = stake;
        }
        assembly ("memory-safe") {
            mstore(ops, count)
            mstore(powers, count)
        }
    }

    /// @dev Replaces the active set with `ops`/`powers` and updates statuses, version and hash.
    function _writeActiveSet(address[] memory ops, uint256[] memory powers) internal {
        address[] storage oldOps = _activeOperators;
        for (uint256 i; i < oldOps.length; ++i) {
            Validator storage v = _validators[oldOps[i]];
            if (v.status == ValidatorStatus.Active) v.status = ValidatorStatus.Candidate;
        }
        delete _activeOperators;
        delete _activeKeys;
        delete _activePowers;
        for (uint256 i; i < ops.length; ++i) {
            Validator storage v = _validators[ops[i]];
            v.status = ValidatorStatus.Active;
            _activeOperators.push(ops[i]);
            _activeKeys.push(v.consensusKey);
            _activePowers.push(powers[i]);
        }
        _setChanged();
    }

    /// @dev Removes `validator` from the active set, preserving the order of the others.
    function _removeFromActiveSet(address validator) internal {
        _snapshotPrevSet();
        uint256 len = _activeOperators.length;
        uint256 idx = len;
        for (uint256 i; i < len; ++i) {
            if (_activeOperators[i] == validator) {
                idx = i;
                break;
            }
        }
        if (idx == len) return;
        for (uint256 i = idx; i + 1 < len; ++i) {
            _activeOperators[i] = _activeOperators[i + 1];
            _activeKeys[i] = _activeKeys[i + 1];
            _activePowers[i] = _activePowers[i + 1];
        }
        _activeOperators.pop();
        _activeKeys.pop();
        _activePowers.pop();
        if (len == 1) {
            // The set would be empty (only possible through a tombstone): refill it from eligible candidates
            // right away. It stays empty only if nobody is eligible — the chain then halts safely.
            (address[] memory ops, uint256[] memory powers) = _computeTopN();
            if (ops.length != 0) {
                _writeActiveSet(ops, powers);
                emit ValidatorSetUpdated(currentEpoch, setVersion, validatorSetHash);
                return;
            }
        }
        _setChanged();
        emit ValidatorSetUpdated(currentEpoch, setVersion, validatorSetHash);
    }

    /// @dev Keeps the set that was in force at the START of this block, once per block.
    function _snapshotPrevSet() internal {
        uint64 bn = uint64(block.number);
        // slither-disable-next-line incorrect-equality (SA-F07: block-number bookkeeping, not a balance)
        if (lastSetChangeBlock == bn) return;
        _prevOperators = _activeOperators;
        _prevPowers = _activePowers;
        lastSetChangeBlock = bn;
    }

    function _setChanged() internal {
        ++setVersion;
        validatorSetHash = keccak256(abi.encode(_activeOperators, _activeKeys, _activePowers));
    }

    // ---------------------------------------------------------------------------------------------------------
    // Internal: misc
    // ---------------------------------------------------------------------------------------------------------

    function _selfStake(address operator, Validator storage v) internal view returns (uint256) {
        uint256 vShares = v.totalShares;
        if (vShares == 0) return 0;
        return Math.mulDiv(_delegations[operator][operator].shares, v.totalStake, vShares);
    }

    // slither-disable-next-line unused-return (SA-F12: Checkpoints.push old/new values not needed)
    function _addVotes(address account, uint256 amount) internal {
        uint48 key = SafeCast.toUint48(block.number);
        _votes[account].push(key, SafeCast.toUint208(_votes[account].latest() + amount));
        _totalVotes.push(key, SafeCast.toUint208(_totalVotes.latest() + amount));
    }

    // slither-disable-next-line unused-return (SA-F12: Checkpoints.push old/new values not needed)
    function _subVotes(address account, uint256 amount) internal {
        if (amount == 0) return;
        uint48 key = SafeCast.toUint48(block.number);
        _votes[account].push(key, _votes[account].latest() - SafeCast.toUint208(amount));
        _totalVotes.push(key, _totalVotes.latest() - SafeCast.toUint208(amount));
    }

    function _setParams(Params calldata p) internal {
        if (p.minSelfStake == 0) revert InvalidParam("minSelfStake");
        if (p.minDelegation == 0) revert InvalidParam("minDelegation");
        if (p.unbondingSeconds < 1 hours || p.unbondingSeconds > 60 days) {
            revert InvalidParam("unbondingSeconds");
        }
        if (p.commissionChangeInterval > 30 days) revert InvalidParam("commissionChangeInterval");
        if (p.downtimeWindow == 0 || p.downtimeWindow > 1_000_000) revert InvalidParam("downtimeWindow");
        if (p.downtimeJailSeconds > 30 days) revert InvalidParam("downtimeJailSeconds");
        if (p.maxValidators == 0 || p.maxValidators > MAX_ACTIVE_HARD_CAP) {
            revert InvalidParam("maxValidators");
        }
        if (p.maxCandidates < p.maxValidators || p.maxCandidates > MAX_CANDIDATES_HARD_CAP) {
            revert InvalidParam("maxCandidates");
        }
        if (p.maxCommissionBps > BPS) revert InvalidParam("maxCommissionBps");
        if (p.maxCommissionChangeBps > BPS) revert InvalidParam("maxCommissionChangeBps");
        if (p.proposerBonusBps > BPS) revert InvalidParam("proposerBonusBps");
        if (p.doubleSignSlashBps > MAX_SLASH_BPS) revert InvalidParam("doubleSignSlashBps");
        if (p.downtimeSlashBps > MAX_SLASH_BPS) revert InvalidParam("downtimeSlashBps");
        if (p.minSignedBps > BPS) revert InvalidParam("minSignedBps");
        _params = p;
    }

    function _send(address to, uint256 amount) internal {
        // slither-disable-next-line arbitrary-send-eth (SA-F03: pull payment to msg.sender, CEI + nonReentrant)
        (bool ok,) = payable(to).call{value: amount}("");
        if (!ok) revert TransferFailed();
    }
}
