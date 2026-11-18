// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ExecutionHeaderStore} from "./ExecutionHeaderStore.sol";
import {BLS12381} from "./lib/BLS12381.sol";
import {SSZ} from "./lib/SSZ.sol";

/// @title EthereumLightClient - Ethereum (Sepolia) sync-committee light client, deployed on xDEVS Chain
/// @notice Follows the Altair light-client protocol as specified for Gloas (EIP-7732, Sepolia's current fork):
///         anyone submits a `LightClientUpdate`; the contract checks the sync committee's BLS aggregate
///         signature (EIP-2537 precompiles) over the attested beacon header, the Merkle branch of the finalized
///         header in the attested state, and the branch of the finalized block's execution block hash in its
///         body. The execution block hash is then trusted ({ExecutionHeaderStore}): its RLP header yields the
///         state and receipts roots that bridge proofs are checked against.
///
///         Sync committees rotate every 8192 slots (~27 h). The next committee's SSZ root is proven in the
///         attested state of a finalized update of the current period; its 512 keys are then loaded with
///         {loadCommittee} (a separate transaction: ~50 KB of calldata).
///
///         Trust assumptions: the initial checkpoint (period + committee root, weak subjectivity) and an honest
///         2/3 supermajority of the sync committee, as for any Ethereum light client. No signer set, no owner,
///         no upgrade path: a new Ethereum fork that changes these containers needs a new deployment.
contract EthereumLightClient is ExecutionHeaderStore {
    using BLS12381 for BLS12381.G1;

    uint256 public constant SLOTS_PER_EPOCH = 32;
    uint256 public constant SLOTS_PER_PERIOD = 8192;
    uint256 public constant SYNC_COMMITTEE_SIZE = 512;
    uint256 public constant SECONDS_PER_SLOT = 12;
    /// @dev Generalized indices of the Gloas light-client protocol (consensus-specs gloas/light-client).
    uint256 public constant FINALIZED_ROOT_GINDEX = 735;
    uint256 public constant NEXT_SYNC_COMMITTEE_GINDEX = 2946;
    uint256 public constant EXECUTION_BLOCK_HASH_GINDEX = 2856;
    bytes public constant DST = "BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

    struct Update {
        SSZ.BeaconBlockHeader attested;
        SSZ.BeaconBlockHeader finalized;
        bytes32[] finalityBranch;
        bytes32 finalizedExecutionBlockHash;
        bytes32[] executionBranch;
        /// @dev hash_tree_root(next_sync_committee), or 0 for an update without rotation.
        bytes32 nextSyncCommitteeRoot;
        bytes32[] nextSyncCommitteeBranch;
        /// @dev SSZ Bitvector[512] (64 bytes).
        bytes32[2] syncCommitteeBits;
        /// @dev The aggregate signature as an uncompressed G2 point (EIP-2537 encoding).
        uint256[8] signature;
        uint64 signatureSlot;
    }

    /// @notice A committee member that did not sign: its key and Merkle proof in the committee's key tree.
    struct NonSigner {
        uint256 index;
        BLS12381.G1 pubkey;
        bytes32[] proof;
    }

    struct Committee {
        bytes32 keysRoot; // keccak Merkle root of the 512 uncompressed keys (abi.encode(G1) leaves)
        BLS12381.G1 aggregate;
    }

    bytes32 public immutable genesisValidatorsRoot;
    uint256 public immutable genesisTime;
    /// @notice First slot whose containers have the Gloas layout these gindices describe.
    uint64 public immutable minSlot;

    uint64[] public forkEpochs;
    bytes4[] public forkVersions;

    /// @notice hash_tree_root(SyncCommittee) per period: trusted checkpoint or proven by a rotation.
    mapping(uint256 period => bytes32) public committeeRoot;
    mapping(uint256 period => Committee) internal _committees;
    uint64 public finalizedSlot;

    event CommitteeRootProven(uint256 indexed period, bytes32 root);
    event CommitteeLoaded(uint256 indexed period, bytes32 keysRoot);
    event FinalizedHeader(uint64 indexed slot, bytes32 indexed beaconRoot, bytes32 executionBlockHash);

    error BadForkSchedule();
    error CommitteeUnknown(uint256 period);
    error CommitteeAlreadyLoaded(uint256 period);
    error BadCommittee();
    error BadSlots();
    error FutureSlot(uint64 slot);
    error InsufficientParticipation(uint256 participants);
    error BadNonSigners();
    error BadFinalityBranch();
    error BadExecutionBranch();
    error BadNextCommitteeBranch();
    error BadSignature();
    error StaleUpdate();
    error PreGloas();

    /// @param initialPeriod sync-committee period of the trusted checkpoint
    /// @param initialCommitteeRoot hash_tree_root of that period's current sync committee (from a bootstrap at a
    ///        trusted block root)
    constructor(
        bytes32 genesisValidatorsRoot_,
        uint256 genesisTime_,
        uint64[] memory forkEpochs_,
        bytes4[] memory forkVersions_,
        uint64 minSlot_,
        uint256 initialPeriod,
        bytes32 initialCommitteeRoot
    ) {
        if (forkEpochs_.length == 0 || forkEpochs_.length != forkVersions_.length) {
            revert BadForkSchedule();
        }
        for (uint256 i = 1; i < forkEpochs_.length; ++i) {
            if (forkEpochs_[i] <= forkEpochs_[i - 1]) revert BadForkSchedule();
        }
        genesisValidatorsRoot = genesisValidatorsRoot_;
        genesisTime = genesisTime_;
        forkEpochs = forkEpochs_;
        forkVersions = forkVersions_;
        minSlot = minSlot_;
        committeeRoot[initialPeriod] = initialCommitteeRoot;
        emit CommitteeRootProven(initialPeriod, initialCommitteeRoot);
    }

    // ----------------------------------------------------------------------------------------------------------
    // Committees
    // ----------------------------------------------------------------------------------------------------------

    /// @notice Load the 512 public keys (uncompressed, EIP-2537 encoding, in committee order) of a period whose
    ///         committee root is known. Checks every key is on the curve (G1ADD) and that the compressed keys and
    ///         their sum hash to the SSZ committee root. Permissionless; ~3M gas.
    function loadCommittee(uint256 period, BLS12381.G1[] calldata keys) external notFrozen {
        bytes32 root = committeeRoot[period];
        if (root == bytes32(0)) revert CommitteeUnknown(period);
        if (_committees[period].keysRoot != bytes32(0)) revert CommitteeAlreadyLoaded(period);
        if (keys.length != SYNC_COMMITTEE_SIZE) revert BadCommittee();

        bytes32[] memory sszLeaves = new bytes32[](SYNC_COMMITTEE_SIZE);
        bytes32[] memory keyLeaves = new bytes32[](SYNC_COMMITTEE_SIZE);
        BLS12381.G1 memory agg = keys[0];
        for (uint256 i; i < SYNC_COMMITTEE_SIZE; ++i) {
            BLS12381.G1 memory k = keys[i];
            if (i != 0) agg = agg.add(k); // also rejects off-curve keys
            (bytes32 w0, bytes32 w1) = k.compress();
            sszLeaves[i] = SSZ.hashPair(w0, w1);
            keyLeaves[i] = keccak256(abi.encode(k));
        }
        (bytes32 a0, bytes32 a1) = agg.compress();
        bytes32 computed = SSZ.hashPair(_merkleize(sszLeaves, true), SSZ.hashPair(a0, a1));
        if (computed != root) revert BadCommittee();

        Committee storage c = _committees[period];
        c.keysRoot = _merkleize(keyLeaves, false);
        c.aggregate = agg;
        emit CommitteeLoaded(period, c.keysRoot);
    }

    function committeeLoaded(uint256 period) external view returns (bool) {
        return _committees[period].keysRoot != bytes32(0);
    }

    // ----------------------------------------------------------------------------------------------------------
    // Updates
    // ----------------------------------------------------------------------------------------------------------

    /// @notice Verify a finality update (optionally carrying the next sync committee) and trust the finalized
    ///         execution block hash. `nonSigners` lists every committee member whose bit is 0, by index.
    function submitUpdate(Update calldata u, NonSigner[] calldata nonSigners) external notFrozen {
        if (!(u.signatureSlot > u.attested.slot && u.attested.slot >= u.finalized.slot)) revert BadSlots();
        if (u.finalized.slot < minSlot) revert PreGloas();
        if (u.signatureSlot > currentSlot()) revert FutureSlot(u.signatureSlot);

        uint256 sigPeriod = u.signatureSlot / SLOTS_PER_PERIOD;
        Committee storage c = _committees[sigPeriod];
        if (c.keysRoot == bytes32(0)) revert CommitteeUnknown(sigPeriod);

        // Finalized header in the attested state, execution block hash in the finalized body.
        bytes32 finalizedRoot = SSZ.headerRoot(u.finalized);
        if (!SSZ.isValidBranch(finalizedRoot, u.finalityBranch, FINALIZED_ROOT_GINDEX, u.attested.stateRoot))
        {
            revert BadFinalityBranch();
        }
        if (!SSZ.isValidBranch(
                u.finalizedExecutionBlockHash,
                u.executionBranch,
                EXECUTION_BLOCK_HASH_GINDEX,
                u.finalized.bodyRoot
            )) {
            revert BadExecutionBranch();
        }

        bool rotates;
        uint256 nextPeriod = u.attested.slot / SLOTS_PER_PERIOD + 1;
        if (u.nextSyncCommitteeRoot != bytes32(0)) {
            // Only a next committee proven by a finalized update of the same period is accepted (spec:
            // `is_next_sync_committee_known` / `update_has_finalized_next_sync_committee`).
            if (u.finalized.slot / SLOTS_PER_PERIOD != nextPeriod - 1) revert BadNextCommitteeBranch();
            if (!SSZ.isValidBranch(
                    u.nextSyncCommitteeRoot,
                    u.nextSyncCommitteeBranch,
                    NEXT_SYNC_COMMITTEE_GINDEX,
                    u.attested.stateRoot
                )) {
                revert BadNextCommitteeBranch();
            }
            rotates = committeeRoot[nextPeriod] == bytes32(0);
        }
        if (u.finalized.slot <= finalizedSlot && !rotates) revert StaleUpdate();

        // Supermajority of the committee, and its aggregate key minus the non-signers.
        BLS12381.G1 memory aggKey = _aggregate(c, u.syncCommitteeBits, nonSigners);

        bytes32 domain = SSZ.syncCommitteeDomain(
            forkVersionAt((u.signatureSlot - 1) / SLOTS_PER_EPOCH), genesisValidatorsRoot
        );
        bytes32 signingRoot = SSZ.signingRoot(SSZ.headerRoot(u.attested), domain);
        if (!BLS12381.verify(aggKey, signingRoot, u.signature, DST)) revert BadSignature();

        if (u.nextSyncCommitteeRoot != bytes32(0)) {
            bytes32 known = committeeRoot[nextPeriod];
            if (known == bytes32(0)) {
                committeeRoot[nextPeriod] = u.nextSyncCommitteeRoot;
                emit CommitteeRootProven(nextPeriod, u.nextSyncCommitteeRoot);
            } else if (known != u.nextSyncCommitteeRoot) {
                // Two supermajority-signed, finalized states disagree on the next committee: unsafe, stop.
                _freeze(nextPeriod, known, u.nextSyncCommitteeRoot);
                return;
            }
        }
        if (u.finalized.slot > finalizedSlot) {
            finalizedSlot = u.finalized.slot;
            _trust(u.finalizedExecutionBlockHash, ANY_NUMBER);
            emit FinalizedHeader(u.finalized.slot, finalizedRoot, u.finalizedExecutionBlockHash);
        }
    }

    /// @notice Current beacon slot from the local clock (updates for future slots are refused, as in the spec).
    function currentSlot() public view returns (uint64) {
        if (block.timestamp < genesisTime) return 0;
        return uint64((block.timestamp - genesisTime) / SECONDS_PER_SLOT);
    }

    function forkVersionAt(uint256 epoch) public view returns (bytes4) {
        uint256 i = forkEpochs.length;
        while (i > 1 && forkEpochs[i - 1] > epoch) --i;
        return forkVersions[i - 1];
    }

    // ----------------------------------------------------------------------------------------------------------

    function _aggregate(Committee storage c, bytes32[2] calldata bits, NonSigner[] calldata nonSigners)
        private
        view
        returns (BLS12381.G1 memory agg)
    {
        uint256 participants;
        for (uint256 w; w < 2; ++w) {
            uint256 x = uint256(bits[w]);
            while (x != 0) {
                x &= x - 1;
                ++participants;
            }
        }
        if (participants * 3 < SYNC_COMMITTEE_SIZE * 2) revert InsufficientParticipation(participants);
        if (nonSigners.length != SYNC_COMMITTEE_SIZE - participants) revert BadNonSigners();

        agg = c.aggregate;
        bytes32 keysRoot = c.keysRoot;
        uint256 prev;
        for (uint256 i; i < nonSigners.length; ++i) {
            NonSigner calldata n = nonSigners[i];
            if ((i != 0 && n.index <= prev) || n.index >= SYNC_COMMITTEE_SIZE || _bit(bits, n.index)) {
                revert BadNonSigners();
            }
            prev = n.index;
            BLS12381.G1 memory k = n.pubkey;
            if (!_verifyKeyProof(keccak256(abi.encode(k)), n.index, n.proof, keysRoot)) {
                revert BadNonSigners();
            }
            agg = agg.add(k.neg());
        }
    }

    /// @dev SSZ Bitvector: bit i is bit (i % 8) of byte (i / 8).
    function _bit(bytes32[2] calldata bits, uint256 i) private pure returns (bool) {
        uint256 byteIndex = i / 8;
        uint256 b = uint8(bits[byteIndex / 32][byteIndex % 32]);
        return (b >> (i % 8)) & 1 == 1;
    }

    function _verifyKeyProof(bytes32 leaf, uint256 index, bytes32[] calldata proof, bytes32 root)
        private
        pure
        returns (bool)
    {
        if (proof.length != 9) return false;
        bytes32 node = leaf;
        for (uint256 i; i < 9; ++i) {
            node = (index >> i) & 1 == 1
                ? keccak256(abi.encodePacked(proof[i], node))
                : keccak256(abi.encodePacked(node, proof[i]));
        }
        return node == root;
    }

    /// @dev Root of a complete binary tree over `leaves` (length a power of two), SHA-256 or keccak256.
    function _merkleize(bytes32[] memory leaves, bool sha) private pure returns (bytes32) {
        uint256 n = leaves.length;
        while (n > 1) {
            for (uint256 i; i < n / 2; ++i) {
                leaves[i] = sha
                    ? SSZ.hashPair(leaves[2 * i], leaves[2 * i + 1])
                    : keccak256(abi.encodePacked(leaves[2 * i], leaves[2 * i + 1]));
            }
            n /= 2;
        }
        return leaves[0];
    }
}
