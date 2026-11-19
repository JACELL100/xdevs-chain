// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {RLPReader} from "./RLPReader.sol";
import {MerklePatricia} from "./MerklePatricia.sol";

/// @title EthProofs - block headers, receipts, logs and account/storage proofs (bridge prototype)
/// @notice Shared by both light clients: xDEVS Chain and Ethereum use the same header, receipt and state-trie
///         formats, so a header whose hash a light client has verified yields `stateRoot` / `receiptsRoot`,
///         and receipt / storage proofs are checked against those.
library EthProofs {
    using RLPReader for RLPReader.Item;

    struct Header {
        bytes32 hash;
        bytes32 parentHash;
        bytes32 stateRoot;
        bytes32 receiptsRoot;
        uint256 number;
        uint256 timestamp;
    }

    struct Log {
        address emitter;
        bytes32[] topics;
        bytes data;
    }

    error ReceiptFailed();
    error LogIndexOutOfRange();
    error BadHeader();

    /// @notice Decodes the fields a bridge needs from an RLP block header (any fork: extra trailing fields such
    ///         as `requestsHash` or `blockAccessListHash` are ignored). `hash = keccak256(rlp)`.
    function decodeHeader(bytes memory rlp) internal pure returns (Header memory h) {
        RLPReader.Item[] memory f = RLPReader.toItem(rlp).toList();
        if (f.length < 15) revert BadHeader();
        h.hash = keccak256(rlp);
        h.parentHash = f[0].toBytes32();
        h.stateRoot = f[3].toBytes32();
        h.receiptsRoot = f[5].toBytes32();
        h.number = f[8].toUint();
        h.timestamp = f[11].toUint();
    }

    /// @notice RLP of a transaction index: the key of the receipts (and transactions) trie.
    function indexKey(uint256 i) internal pure returns (bytes memory) {
        if (i == 0) return hex"80";
        if (i < 0x80) return abi.encodePacked(uint8(i));
        if (i < 0x100) return abi.encodePacked(uint8(0x81), uint8(i));
        if (i < 0x10000) return abi.encodePacked(uint8(0x82), uint16(i));
        if (i < 0x1000000) return abi.encodePacked(uint8(0x83), uint24(i));
        revert BadHeader();
    }

    /// @notice The receipt at `txIndex` proven against `receiptsRoot` (typed envelope or legacy RLP).
    function provenReceipt(bytes32 receiptsRoot, uint256 txIndex, bytes[] memory proof)
        internal
        pure
        returns (bytes memory)
    {
        return MerklePatricia.getIncluded(receiptsRoot, indexKey(txIndex), proof);
    }

    /// @notice Log `logIndex` of a successful receipt (reverts if the transaction failed).
    function receiptLog(bytes memory receipt, uint256 logIndex) internal pure returns (Log memory log) {
        if (receipt.length == 0) revert ReceiptFailed();
        bytes memory body = receipt;
        if (uint8(receipt[0]) < 0x80) {
            // EIP-2718 typed receipt: type byte ‖ RLP([status, cumulativeGas, bloom, logs]).
            body = new bytes(receipt.length - 1);
            assembly ("memory-safe") {
                mcopy(add(body, 0x20), add(receipt, 0x21), mload(body))
            }
        }
        RLPReader.Item[] memory r = RLPReader.toItem(body).toList();
        if (r.length != 4) revert ReceiptFailed();
        // Post-Byzantium status: 1 = success. (A 32-byte field would be a pre-Byzantium state root.)
        if (r[0].payloadLength() > 1 || r[0].toUint() != 1) revert ReceiptFailed();
        RLPReader.Item[] memory logs = r[3].toList();
        if (logIndex >= logs.length) revert LogIndexOutOfRange();
        RLPReader.Item[] memory l = logs[logIndex].toList();
        if (l.length != 3) revert ReceiptFailed();
        log.emitter = l[0].toAddress();
        RLPReader.Item[] memory t = l[1].toList();
        log.topics = new bytes32[](t.length);
        for (uint256 i; i < t.length; ++i) {
            log.topics[i] = t[i].toBytes32();
        }
        log.data = l[2].toBytes();
    }

    /// @notice Storage root of `account` proven against `stateRoot` (`eth_getProof.accountProof`).
    function storageRoot(bytes32 stateRoot, address account, bytes[] memory accountProof)
        internal
        pure
        returns (bytes32)
    {
        bytes memory acc = MerklePatricia.getIncluded(
            stateRoot, abi.encodePacked(keccak256(abi.encodePacked(account))), accountProof
        );
        RLPReader.Item[] memory a = RLPReader.toItem(acc).toList();
        if (a.length != 4) revert MerklePatricia.BadProof();
        return a[2].toBytes32();
    }

    /// @notice Value of storage `slot` proven against an account's `storageRoot` (0 for a proven absent slot).
    function storageValue(bytes32 root, bytes32 slot, bytes[] memory storageProof)
        internal
        pure
        returns (uint256)
    {
        (bool found, bytes memory v) =
            MerklePatricia.get(root, abi.encodePacked(keccak256(abi.encodePacked(slot))), storageProof);
        if (!found) return 0;
        return RLPReader.toItem(v).toUint();
    }
}
