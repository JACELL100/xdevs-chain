// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title SSZ - the consensus-layer Merkle helpers a sync-committee light client needs (bridge prototype)
/// @notice hash_tree_root of `BeaconBlockHeader`, Merkle branches by generalized index, and the signing domain.
///         SHA-256 is the 0x02 precompile.
library SSZ {
    struct BeaconBlockHeader {
        uint64 slot;
        uint64 proposerIndex;
        bytes32 parentRoot;
        bytes32 stateRoot;
        bytes32 bodyRoot;
    }

    bytes4 internal constant DOMAIN_SYNC_COMMITTEE = 0x07000000;

    error BadBranch();

    function hashPair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return sha256(abi.encodePacked(a, b));
    }

    /// @notice uint64 as an SSZ chunk (little-endian, zero-padded).
    function toChunk(uint64 v) internal pure returns (bytes32) {
        uint256 x = v;
        x = ((x & 0xff00ff00ff00ff00) >> 8) | ((x & 0x00ff00ff00ff00ff) << 8);
        x = ((x & 0xffff0000ffff0000) >> 16) | ((x & 0x0000ffff0000ffff) << 16);
        x = (x >> 32) | ((x & 0xffffffff) << 32);
        return bytes32(x << 192);
    }

    function headerRoot(BeaconBlockHeader memory h) internal pure returns (bytes32) {
        bytes32 a = hashPair(toChunk(h.slot), toChunk(h.proposerIndex));
        bytes32 b = hashPair(h.parentRoot, h.stateRoot);
        bytes32 c = hashPair(h.bodyRoot, bytes32(0));
        bytes32 zero2 = hashPair(bytes32(0), bytes32(0));
        return hashPair(hashPair(a, b), hashPair(c, zero2));
    }

    /// @notice True iff `leaf` is at generalized index `gindex` under `root` with the given branch.
    function isValidBranch(bytes32 leaf, bytes32[] memory branch, uint256 gindex, bytes32 root)
        internal
        pure
        returns (bool)
    {
        uint256 depth;
        for (uint256 g = gindex; g > 1; g >>= 1) {
            ++depth;
        }
        if (branch.length != depth) return false;
        bytes32 node = leaf;
        for (uint256 i; i < depth; ++i) {
            node = (gindex >> i) & 1 == 1 ? hashPair(branch[i], node) : hashPair(node, branch[i]);
        }
        return node == root;
    }

    /// @notice compute_domain(DOMAIN_SYNC_COMMITTEE, forkVersion, genesisValidatorsRoot).
    function syncCommitteeDomain(bytes4 forkVersion, bytes32 genesisValidatorsRoot)
        internal
        pure
        returns (bytes32)
    {
        bytes32 forkDataRoot = hashPair(bytes32(forkVersion), genesisValidatorsRoot);
        return bytes32(DOMAIN_SYNC_COMMITTEE) | (forkDataRoot >> 32);
    }

    function signingRoot(bytes32 objectRoot, bytes32 domain) internal pure returns (bytes32) {
        return hashPair(objectRoot, domain);
    }
}
