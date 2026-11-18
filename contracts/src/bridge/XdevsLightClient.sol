// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ExecutionHeaderStore} from "./ExecutionHeaderStore.sol";
import {Ed25519} from "./lib/Ed25519.sol";
import {EthProofs} from "./lib/EthProofs.sol";

/// @title XdevsLightClient - xDEVS Chain BFT light client, deployed on Ethereum (Sepolia)
/// @notice Verifies xDEVS commit certificates: Ed25519 precommit signatures of more than 2/3 of the voting power
///         of the validator set in force at the block's height over
///         `"XDEVS/VOTE/1" ‖ chainId:u64be ‖ 0x02 ‖ height:u64be ‖ round:u32be ‖ 0x01 ‖ blockHash`
///         (`xdevs_consensus::vote_sign_bytes`). A certified block hash is trusted ({ExecutionHeaderStore}); its
///         RLP header yields the state and receipts roots bridge proofs are checked against. xDEVS finality is
///         instant: a certified block is final.
///
///         Validator sets follow the node's epoch rule (`ChainConfig::validator_set_boundary`): the set fixed by
///         epoch-boundary block `B` governs heights `B+1 ..= B+epochLength`. A new set is proven, not signed:
///         {submitValidatorSet} checks a storage proof of the Staking predeploy's `validatorSetHash` (slot 4) and
///         `lastEpochBlock` (slot 3) against the state root of the certified boundary block, and converts the
///         Staking set to consensus powers exactly like `xdevs_node::staking::set_from_staking`.
///
///         Ethereum has no Ed25519 precompile: each signature costs ~250-300k gas in Solidity ({Ed25519}), so a
///         certificate of n signatures costs ~n × 0.3M gas. Relayers submit only as many signatures as a quorum
///         needs. A BLS-aggregated or ZK-compressed certificate is the planned path to a constant cost.
contract XdevsLightClient is ExecutionHeaderStore {
    struct Validator {
        bytes32 key; // Ed25519 public key
        uint64 power;
    }

    struct Signature {
        uint256 validatorIndex; // index in the (address-sorted) validator set
        bytes32 r;
        bytes32 s;
    }

    /// @notice `Staking.consensusValidatorSet()` at a boundary block and the `eth_getProof` proofs of the Staking
    ///         account and of its storage slots 3 and 4 at that block.
    struct StakingSetProof {
        address[] operators;
        bytes32[] keys;
        uint256[] powersWei;
        bytes[] accountProof;
        bytes[] epochWordProof;
        bytes[] setHashProof;
    }

    struct ValidatorSet {
        bytes32 hash; // keccak256(abi.encode(Validator[])) of the address-sorted set
        uint64 totalPower;
    }

    uint256 public constant WEI_PER_POWER = 1e18;
    uint64 public constant MAX_POWER = type(uint64).max >> 16;
    bytes12 internal constant VOTE_DOMAIN = "XDEVS/VOTE/1";
    uint256 internal constant SLOT_EPOCH_WORD = 3; // currentEpoch | setVersion | lastSetChangeBlock | lastEpochBlock
    uint256 internal constant SLOT_SET_HASH = 4; // validatorSetHash

    uint64 public immutable chainId;
    uint64 public immutable epochLength;
    /// @notice `ChainConfig::staking_validators_block` (type(uint64).max = staking never activates).
    uint64 public immutable stakingStart;
    address public immutable staking;

    /// @notice Validator set per epoch boundary; key 0 = the set in force before the first boundary (genesis).
    mapping(uint64 boundary => ValidatorSet) public validatorSets;
    mapping(uint64 height => bytes32) public certifiedBlock;

    event ValidatorSetStored(uint64 indexed boundary, bytes32 setHash, uint64 totalPower, uint256 members);
    event CommitVerified(uint64 indexed height, uint32 round, bytes32 indexed blockHash, uint64 signedPower);

    error UnknownValidatorSet(uint64 boundary);
    error WrongValidatorSet();
    error BadValidatorSet();
    error SignaturesNotSorted();
    error InvalidSignature(uint256 validatorIndex);
    error InsufficientPower(uint64 signed, uint64 total);
    error NotEpochBoundary(uint64 boundary);
    error SetAlreadyKnown(uint64 boundary);
    error BadStakingProof();

    /// @param initialBoundary epoch boundary of the trusted checkpoint set (0 = the genesis set)
    /// @param initialSet that set, sorted by consensus address (`sha256(key)[..20]`)
    constructor(
        uint64 chainId_,
        uint64 epochLength_,
        uint64 stakingStart_,
        address staking_,
        uint64 initialBoundary,
        Validator[] memory initialSet
    ) {
        chainId = chainId_;
        epochLength = epochLength_;
        stakingStart = stakingStart_;
        staking = staking_;
        (bytes32 h, uint64 total) = _hashSet(initialSet);
        validatorSets[initialBoundary] = ValidatorSet(h, total);
        emit ValidatorSetStored(initialBoundary, h, total, initialSet.length);
    }

    /// @notice `ChainConfig::validator_set_boundary`: the boundary whose set governs `height` (0 = genesis set).
    function validatorSetBoundary(uint64 height) public view returns (uint64) {
        if (stakingStart == type(uint64).max || epochLength == 0 || height < 2) return 0;
        uint64 last = (height - 1) / epochLength * epochLength;
        return (last > 0 && last >= stakingStart) ? last : 0;
    }

    /// @notice Verify a commit certificate for (`height`, `round`, `blockHash`) and trust `blockHash`.
    /// @param set the full validator set in force at `height` (must hash to the stored set)
    /// @param sigs signatures with strictly increasing validator indexes (any subset holding > 2/3 of the power)
    function submitCommit(
        uint64 height,
        uint32 round,
        bytes32 blockHash,
        Validator[] calldata set,
        Signature[] calldata sigs
    ) external notFrozen {
        ValidatorSet memory vs = validatorSets[validatorSetBoundary(height)];
        if (vs.hash == bytes32(0)) revert UnknownValidatorSet(validatorSetBoundary(height));
        if (keccak256(abi.encode(set)) != vs.hash) revert WrongValidatorSet();

        bytes memory message =
            abi.encodePacked(VOTE_DOMAIN, chainId, uint8(2), height, round, uint8(1), blockHash);
        uint64 signed;
        for (uint256 i; i < sigs.length; ++i) {
            Signature calldata s = sigs[i];
            if (i != 0 && s.validatorIndex <= sigs[i - 1].validatorIndex) revert SignaturesNotSorted();
            Validator calldata v = set[s.validatorIndex];
            if (!Ed25519.verify(v.key, s.r, s.s, message)) revert InvalidSignature(s.validatorIndex);
            signed += v.power;
        }
        if (uint256(signed) * 3 <= uint256(vs.totalPower) * 2) {
            revert InsufficientPower(signed, vs.totalPower);
        }

        bytes32 known = certifiedBlock[height];
        if (known != bytes32(0)) {
            // Two quorum certificates for different blocks at one height: > 1/3 of the power equivocated.
            if (known != blockHash) _freeze(height, known, blockHash);
            return;
        }
        certifiedBlock[height] = blockHash;
        _trust(blockHash, uint256(height) + 1);
        emit CommitVerified(height, round, blockHash, signed);
    }

    /// @notice Prove the validator set fixed by epoch-boundary block `boundary` (whose header must be stored)
    ///         from the Staking predeploy's storage (see {StakingSetProof}).
    function submitValidatorSet(uint64 boundary, StakingSetProof calldata p) external notFrozen {
        if (
            stakingStart == type(uint64).max || boundary == 0 || epochLength == 0
                || boundary % epochLength != 0 || boundary < stakingStart
        ) revert NotEpochBoundary(boundary);
        if (validatorSets[boundary].hash != bytes32(0)) revert SetAlreadyKnown(boundary);
        if (p.operators.length != p.keys.length || p.keys.length != p.powersWei.length) {
            revert BadValidatorSet();
        }
        _checkStakingProof(boundary, p);

        Validator[] memory set = _toConsensusSet(p.keys, p.powersWei);
        (bytes32 h, uint64 total) = set.length == 0 ? _previous(boundary) : _hashSet(set); // no usable member: the node keeps its set
        validatorSets[boundary] = ValidatorSet(h, total);
        emit ValidatorSetStored(boundary, h, total, set.length);
    }

    /// @dev Staking storage at the boundary block: `lastEpochBlock == boundary` (onEpoch ran there) and
    ///      `validatorSetHash == keccak256(abi.encode(operators, keys, powersWei))`.
    function _checkStakingProof(uint64 boundary, StakingSetProof calldata p) private view {
        Roots storage r = _roots[boundary];
        if (r.blockHash == bytes32(0)) revert UnknownBlock(boundary);
        bytes32 sRoot = EthProofs.storageRoot(r.stateRoot, staking, p.accountProof);
        uint256 epochWord = EthProofs.storageValue(sRoot, bytes32(SLOT_EPOCH_WORD), p.epochWordProof);
        if (uint64(epochWord >> 192) != boundary) revert BadStakingProof();
        uint256 setHash = EthProofs.storageValue(sRoot, bytes32(SLOT_SET_HASH), p.setHashProof);
        if (keccak256(abi.encode(p.operators, p.keys, p.powersWei)) != bytes32(setHash)) {
            revert BadStakingProof();
        }
    }

    // ----------------------------------------------------------------------------------------------------------

    function _previous(uint64 boundary) private view returns (bytes32, uint64) {
        ValidatorSet memory p = validatorSets[validatorSetBoundary(boundary)];
        if (p.hash == bytes32(0)) revert UnknownValidatorSet(validatorSetBoundary(boundary));
        return (p.hash, p.totalPower);
    }

    /// @dev `set_from_staking`: power = stake / 1e18 clamped to [1, MAX_POWER]; keys that are not Ed25519 points
    ///      are left out; the result sorted by consensus address.
    function _toConsensusSet(bytes32[] calldata keys, uint256[] calldata powersWei)
        private
        view
        returns (Validator[] memory set)
    {
        set = new Validator[](keys.length);
        uint256 n;
        for (uint256 i; i < keys.length; ++i) {
            (bool ok,,) = Ed25519.decompress(keys[i]);
            if (!ok) continue;
            uint256 p = powersWei[i] / WEI_PER_POWER;
            uint64 power = p > MAX_POWER ? MAX_POWER : (p == 0 ? 1 : uint64(p));
            // insertion sort by address
            Validator memory v = Validator(keys[i], power);
            bytes20 a = consensusAddress(v.key);
            uint256 j = n;
            while (j > 0 && consensusAddress(set[j - 1].key) > a) {
                set[j] = set[j - 1];
                --j;
            }
            set[j] = v;
            ++n;
        }
        assembly ("memory-safe") {
            mstore(set, n)
        }
    }

    /// @notice Consensus address of a validator key: `sha256(key)[..20]`.
    function consensusAddress(bytes32 key) public pure returns (bytes20) {
        return bytes20(sha256(abi.encodePacked(key)));
    }

    function _hashSet(Validator[] memory set) private pure returns (bytes32 h, uint64 total) {
        if (set.length == 0) revert BadValidatorSet();
        bytes20 prev;
        for (uint256 i; i < set.length; ++i) {
            bytes20 a = consensusAddress(set[i].key);
            if (i != 0 && a <= prev) revert BadValidatorSet(); // sorted, no duplicates
            if (set[i].power == 0) revert BadValidatorSet();
            prev = a;
            total += set[i].power;
        }
        h = keccak256(abi.encode(set));
    }
}
