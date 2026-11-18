// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title RLPReader - bounds-checked RLP decoding over memory (bridge prototype)
/// @notice Decodes the RLP of Ethereum-format block headers, receipts and Merkle-Patricia trie nodes. Every item
///         is a (memory pointer, length) view into the input; nothing is copied until `toBytes`. All lengths are
///         checked against the enclosing item, so a malformed input reverts instead of reading out of bounds.
library RLPReader {
    struct Item {
        uint256 ptr; // memory address of the item's first byte (its RLP prefix)
        uint256 len; // total length including the prefix
    }

    error RLPMalformed();

    /// @notice The single item spanning all of `data` (reverts unless `data` is exactly one RLP item).
    function toItem(bytes memory data) internal pure returns (Item memory item) {
        uint256 ptr;
        assembly ("memory-safe") {
            ptr := add(data, 0x20)
        }
        item = Item(ptr, data.length);
        (,, uint256 total) = _decodeHeader(item.ptr, item.len);
        if (total != data.length) revert RLPMalformed();
    }

    function isList(Item memory item) internal pure returns (bool) {
        if (item.len == 0) revert RLPMalformed();
        uint8 b;
        uint256 p = item.ptr;
        assembly ("memory-safe") {
            b := byte(0, mload(p))
        }
        return b >= 0xc0;
    }

    /// @notice Children of a list item.
    function toList(Item memory item) internal pure returns (Item[] memory list) {
        (uint256 payloadOffset, uint256 payloadLen, bool listFlag) = _payload(item);
        if (!listFlag) revert RLPMalformed();
        uint256 p = item.ptr + payloadOffset;
        uint256 end = p + payloadLen;
        uint256 count;
        uint256 q = p;
        while (q < end) {
            (,, uint256 total) = _decodeHeader(q, end - q);
            q += total;
            ++count;
        }
        list = new Item[](count);
        for (uint256 i; i < count; ++i) {
            (,, uint256 total) = _decodeHeader(p, end - p);
            list[i] = Item(p, total);
            p += total;
        }
    }

    /// @notice Payload of a string item (reverts on a list).
    function toBytes(Item memory item) internal pure returns (bytes memory out) {
        (uint256 payloadOffset, uint256 payloadLen, bool listFlag) = _payload(item);
        if (listFlag) revert RLPMalformed();
        out = new bytes(payloadLen);
        _copy(item.ptr + payloadOffset, out, payloadLen);
    }

    /// @notice The item's full encoding (prefix included).
    function toRlpBytes(Item memory item) internal pure returns (bytes memory out) {
        out = new bytes(item.len);
        _copy(item.ptr, out, item.len);
    }

    /// @notice Big-endian unsigned integer payload of at most 32 bytes.
    function toUint(Item memory item) internal pure returns (uint256 v) {
        (uint256 payloadOffset, uint256 payloadLen, bool listFlag) = _payload(item);
        if (listFlag || payloadLen > 32) revert RLPMalformed();
        if (payloadLen == 0) return 0;
        uint256 p = item.ptr + payloadOffset;
        assembly ("memory-safe") {
            v := shr(mul(8, sub(32, payloadLen)), mload(p))
        }
    }

    /// @notice Exactly 32-byte string payload.
    function toBytes32(Item memory item) internal pure returns (bytes32 v) {
        (uint256 payloadOffset, uint256 payloadLen, bool listFlag) = _payload(item);
        if (listFlag || payloadLen != 32) revert RLPMalformed();
        uint256 p = item.ptr + payloadOffset;
        assembly ("memory-safe") {
            v := mload(p)
        }
    }

    /// @notice Exactly 20-byte string payload.
    function toAddress(Item memory item) internal pure returns (address a) {
        (uint256 payloadOffset, uint256 payloadLen, bool listFlag) = _payload(item);
        if (listFlag || payloadLen != 20) revert RLPMalformed();
        uint256 p = item.ptr + payloadOffset;
        assembly ("memory-safe") {
            a := shr(96, mload(p))
        }
    }

    /// @notice Length of a string item's payload.
    function payloadLength(Item memory item) internal pure returns (uint256) {
        (, uint256 payloadLen,) = _payload(item);
        return payloadLen;
    }

    // ----------------------------------------------------------------------------------------------------------

    function _payload(Item memory item) private pure returns (uint256 offset, uint256 len, bool listFlag) {
        uint256 total;
        (offset, len, total) = _decodeHeader(item.ptr, item.len);
        if (total != item.len) revert RLPMalformed();
        uint8 b;
        uint256 p = item.ptr;
        assembly ("memory-safe") {
            b := byte(0, mload(p))
        }
        listFlag = b >= 0xc0;
    }

    /// @dev Decodes the prefix at `p` given `avail` bytes; returns (payload offset, payload length, total length).
    function _decodeHeader(uint256 p, uint256 avail)
        private
        pure
        returns (uint256 offset, uint256 len, uint256 total)
    {
        if (avail == 0) revert RLPMalformed();
        uint256 b;
        assembly ("memory-safe") {
            b := byte(0, mload(p))
        }
        if (b < 0x80) {
            return (0, 1, 1);
        } else if (b < 0xb8) {
            len = b - 0x80;
            offset = 1;
            if (len == 1) {
                uint256 b1;
                assembly ("memory-safe") {
                    b1 := byte(0, mload(add(p, 1)))
                }
                if (avail < 2 || b1 < 0x80) revert RLPMalformed(); // non-canonical single byte
            }
        } else if (b < 0xc0) {
            uint256 lenOfLen = b - 0xb7;
            (offset, len) = _longLength(p, avail, lenOfLen);
            if (len < 56) revert RLPMalformed();
        } else if (b < 0xf8) {
            len = b - 0xc0;
            offset = 1;
        } else {
            uint256 lenOfLen = b - 0xf7;
            (offset, len) = _longLength(p, avail, lenOfLen);
            if (len < 56) revert RLPMalformed();
        }
        total = offset + len;
        if (total > avail) revert RLPMalformed();
    }

    function _longLength(uint256 p, uint256 avail, uint256 lenOfLen)
        private
        pure
        returns (uint256 offset, uint256 len)
    {
        if (lenOfLen > 4 || avail < 1 + lenOfLen) revert RLPMalformed();
        uint256 first;
        assembly ("memory-safe") {
            len := shr(mul(8, sub(32, lenOfLen)), mload(add(p, 1)))
            first := byte(0, mload(add(p, 1)))
        }
        if (first == 0) revert RLPMalformed(); // leading zero in the length
        offset = 1 + lenOfLen;
    }

    function _copy(uint256 src, bytes memory dest, uint256 len) private pure {
        assembly ("memory-safe") {
            mcopy(add(dest, 0x20), src, len)
        }
    }
}
