// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {EthProofs} from "./lib/EthProofs.sol";
import {IExecutionHeaderOracle} from "./interfaces/IExecutionHeaderOracle.sol";

/// @title ExecutionHeaderStore - verified execution-layer headers of the remote chain (bridge prototype)
/// @notice Base of both light clients. The consensus proof of a light client (sync-committee signature on the
///         Ethereum side, commit certificate on the xDEVS side) marks a block hash as trusted; anyone may then
///         submit the RLP header with that hash, which stores its `stateRoot` / `receiptsRoot` by number and
///         trusts its parent hash in turn, so a relayer can walk back from one finalized block to any ancestor
///         at ~25k gas per header instead of proving each block's finality.
///
///         Safety rule: two different verified headers for the same block number can only exist if the remote
///         chain's finality failed (an equivocating supermajority). The store then **freezes** for good:
///         `verifiedRoots` reverts and nothing is accepted any more. Recovery is a new deployment (governance,
///         timelocked), never an admin override of a verified fact.
abstract contract ExecutionHeaderStore is IExecutionHeaderOracle {
    struct Roots {
        bytes32 blockHash;
        bytes32 stateRoot;
        bytes32 receiptsRoot;
    }

    /// @dev `trusted[hash]` = expected block number + 1, or {ANY_NUMBER} when the consensus proof does not carry
    ///      the number (Ethereum: the beacon header commits to the execution block hash only).
    uint256 internal constant ANY_NUMBER = type(uint256).max;

    mapping(uint256 number => Roots) internal _roots;
    mapping(bytes32 blockHash => uint256 numberPlusOne) public trusted;
    bool public frozen;
    uint256 public latestNumber;

    event BlockHashTrusted(bytes32 indexed blockHash, uint256 numberPlusOne);
    event HeaderStored(
        uint256 indexed number, bytes32 indexed blockHash, bytes32 stateRoot, bytes32 receiptsRoot
    );
    event Frozen(uint256 indexed number, bytes32 storedHash, bytes32 conflictingHash);

    error IsFrozen();
    error UntrustedHeader(bytes32 blockHash);
    error WrongNumber(uint256 expected, uint256 got);
    error UnknownBlock(uint256 number);

    modifier notFrozen() {
        if (frozen) revert IsFrozen();
        _;
    }

    /// @notice Store a header whose hash a consensus proof (or a stored child header) vouched for.
    function submitHeader(bytes calldata rlp) external notFrozen {
        EthProofs.Header memory h = EthProofs.decodeHeader(rlp);
        uint256 expect = trusted[h.hash];
        if (expect == 0) revert UntrustedHeader(h.hash);
        if (expect != ANY_NUMBER && expect != h.number + 1) revert WrongNumber(expect - 1, h.number);
        Roots storage r = _roots[h.number];
        if (r.blockHash != bytes32(0)) {
            if (r.blockHash != h.hash) _freeze(h.number, r.blockHash, h.hash);
            return;
        }
        r.blockHash = h.hash;
        r.stateRoot = h.stateRoot;
        r.receiptsRoot = h.receiptsRoot;
        if (h.number > latestNumber) latestNumber = h.number;
        if (h.number > 0 && trusted[h.parentHash] == 0) {
            trusted[h.parentHash] = h.number; // parent number + 1
        }
        emit HeaderStored(h.number, h.hash, h.stateRoot, h.receiptsRoot);
    }

    /// @inheritdoc IExecutionHeaderOracle
    function verifiedRoots(uint256 number) external view returns (bytes32 stateRoot, bytes32 receiptsRoot) {
        if (frozen) revert IsFrozen();
        Roots storage r = _roots[number];
        if (r.blockHash == bytes32(0)) revert UnknownBlock(number);
        return (r.stateRoot, r.receiptsRoot);
    }

    function blockHashAt(uint256 number) external view returns (bytes32) {
        return _roots[number].blockHash;
    }

    function _trust(bytes32 blockHash, uint256 numberPlusOne) internal {
        if (trusted[blockHash] == 0) {
            trusted[blockHash] = numberPlusOne;
            emit BlockHashTrusted(blockHash, numberPlusOne);
        }
    }

    function _freeze(uint256 number, bytes32 storedHash, bytes32 conflictingHash) internal {
        frozen = true;
        emit Frozen(number, storedHash, conflictingHash);
    }
}
