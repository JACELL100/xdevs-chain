// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {RLPReader} from "./RLPReader.sol";

/// @title MerklePatricia - Ethereum Merkle-Patricia trie inclusion proofs (bridge prototype)
/// @notice Verifies `eth_getProof`-style proofs (account and storage tries) and receipt-trie proofs against a
///         root taken from a verified block header. The proof is the list of RLP trie nodes from the root to the
///         leaf; nodes shorter than 32 bytes are embedded in their parent (as in geth) and are followed in place.
library MerklePatricia {
    using RLPReader for RLPReader.Item;

    error BadProof();

    struct Cursor {
        uint256 pos; // nibbles of the key consumed
        uint256 next; // next proof element
        bytes32 expected; // hash the next proof element must have
        bool embedded; // `node` is embedded in its parent (no proof element)
        RLPReader.Item node;
    }

    /// @notice Value stored under `key` in the trie with root `root`; `found = false` for a valid exclusion proof.
    /// @dev Reverts on a proof that is inconsistent with `root` (wrong hash, malformed node, missing node).
    function get(bytes32 root, bytes memory key, bytes[] memory proof)
        internal
        pure
        returns (bool found, bytes memory value)
    {
        Cursor memory c =
            Cursor({pos: 0, next: 0, expected: root, embedded: false, node: RLPReader.Item(0, 0)});
        while (true) {
            if (!c.embedded) {
                if (c.next >= proof.length) revert BadProof();
                bytes memory enc = proof[c.next++];
                if (keccak256(enc) != c.expected) revert BadProof();
                c.node = RLPReader.toItem(enc);
            }
            bool done;
            (done, found, value) = _step(c, key);
            if (done) return (found, value);
        }
    }

    /// @dev Processes the current node: either finishes (`done`) or moves `c` to the child on `key`'s path.
    function _step(Cursor memory c, bytes memory key)
        private
        pure
        returns (bool done, bool found, bytes memory value)
    {
        RLPReader.Item[] memory items = c.node.toList();
        RLPReader.Item memory child;
        if (items.length == 17) {
            if (c.pos == key.length * 2) {
                value = items[16].toBytes();
                return (true, value.length != 0, value);
            }
            child = items[_nibble(key, c.pos)];
            ++c.pos;
        } else if (items.length == 2) {
            (bool leaf, bool matched, uint256 newPos) = _matchPath(items[0].toBytes(), key, c.pos);
            if (!matched) return (true, false, "");
            c.pos = newPos;
            if (leaf) {
                if (newPos != key.length * 2) return (true, false, "");
                return (true, true, items[1].toBytes());
            }
            child = items[1];
        } else {
            revert BadProof();
        }
        if (child.isList()) {
            // An embedded node (< 32 bytes) is referenced by its RLP, not by its hash.
            if (child.len >= 32) revert BadProof();
            c.node = child;
            c.embedded = true;
        } else {
            bytes memory ref = child.toBytes();
            if (ref.length == 0) return (true, false, "");
            if (ref.length != 32) revert BadProof();
            c.expected = bytes32(ref);
            c.embedded = false;
        }
        return (false, false, "");
    }

    /// @notice Inclusion-only variant: reverts unless `key` is in the trie.
    function getIncluded(bytes32 root, bytes memory key, bytes[] memory proof)
        internal
        pure
        returns (bytes memory value)
    {
        bool found;
        (found, value) = get(root, key, proof);
        if (!found) revert BadProof();
    }

    /// @dev Matches a hex-prefix encoded node path against `key` from nibble `pos`.
    function _matchPath(bytes memory path, bytes memory key, uint256 pos)
        private
        pure
        returns (bool leaf, bool matched, uint256 newPos)
    {
        if (path.length == 0) revert BadProof();
        uint8 flag = uint8(path[0]) >> 4;
        if (flag > 3) revert BadProof();
        leaf = flag >= 2;
        bool odd = flag & 1 == 1;
        if (!odd && uint8(path[0]) & 0x0f != 0) revert BadProof();
        uint256 pathLen = path.length * 2 - (odd ? 1 : 2);
        uint256 start = odd ? 1 : 2;
        if (pos + pathLen > key.length * 2) return (leaf, false, pos);
        for (uint256 i; i < pathLen; ++i) {
            if (_nibble(path, start + i) != _nibble(key, pos + i)) return (leaf, false, pos);
        }
        return (leaf, true, pos + pathLen);
    }

    function _nibble(bytes memory b, uint256 i) private pure returns (uint8) {
        uint8 x = uint8(b[i / 2]);
        return i % 2 == 0 ? x >> 4 : x & 0x0f;
    }
}
