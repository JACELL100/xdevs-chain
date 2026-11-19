// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {EthProofs} from "./lib/EthProofs.sol";
import {IExecutionHeaderOracle} from "./interfaces/IExecutionHeaderOracle.sol";

/// @title BridgeBase - shared event-proof plumbing of the two bridge endpoints (prototype)
/// @notice An endpoint accepts a message only as a log emitted by its counterpart contract on the other chain,
///         proven with a receipt-trie proof against a receipts root that the local light client verified. Every
///         message carries a nonce that is consumed exactly once (replay protection).
///
///         Governance can only **stop** an endpoint: `guardian` (the chain's timelocked Governance contract on
///         xDEVS Chain; a timelock on Sepolia) may pause and unpause. It cannot mint, release, change the light
///         client or the counterpart, or upgrade the code (there is no upgrade path).
abstract contract BridgeBase {
    struct EventProof {
        uint256 blockNumber; // remote block that contains the event
        uint256 txIndex; // transaction index in that block
        bytes[] receiptProof; // receipts-trie proof of that transaction's receipt
        uint256 logIndex; // index of the log within the receipt
    }

    IExecutionHeaderOracle public immutable lightClient;
    uint256 public immutable remoteChainId;
    address public immutable guardian;
    address private immutable _deployer;

    /// @notice The bridge endpoint on the remote chain (set once by the deployer).
    address public counterpart;
    bool public paused;
    uint256 public nextNonce;
    mapping(uint256 nonce => bool) public processed;

    event CounterpartSet(address indexed counterpart);
    event Paused(bool paused);

    error NotGuardian();
    error NotDeployer();
    error CounterpartAlreadySet();
    error IsPaused();
    error ZeroAddress();
    error ZeroAmount();
    error WrongEmitter(address emitter);
    error WrongEvent(bytes32 topic0);
    error AlreadyProcessed(uint256 nonce);

    constructor(IExecutionHeaderOracle lightClient_, uint256 remoteChainId_, address guardian_) {
        if (address(lightClient_) == address(0) || guardian_ == address(0)) revert ZeroAddress();
        lightClient = lightClient_;
        remoteChainId = remoteChainId_;
        guardian = guardian_;
        _deployer = msg.sender;
    }

    modifier whenNotPaused() {
        if (paused) revert IsPaused();
        _;
    }

    function setCounterpart(address counterpart_) external {
        if (msg.sender != _deployer) revert NotDeployer();
        if (counterpart != address(0)) revert CounterpartAlreadySet();
        if (counterpart_ == address(0)) revert ZeroAddress();
        counterpart = counterpart_;
        emit CounterpartSet(counterpart_);
    }

    function setPaused(bool paused_) external {
        if (msg.sender != guardian) revert NotGuardian();
        paused = paused_;
        emit Paused(paused_);
    }

    /// @dev Proves `p` and returns the counterpart's log with event signature `topic0`; consumes its nonce
    ///      (topics[1]).
    function _consume(EventProof calldata p, bytes32 topic0) internal returns (EthProofs.Log memory log) {
        // slither-disable-next-line unused-return (BR-01: events are proven against the receipts root only)
        (, bytes32 receiptsRoot) = lightClient.verifiedRoots(p.blockNumber);
        bytes memory receipt = EthProofs.provenReceipt(receiptsRoot, p.txIndex, p.receiptProof);
        log = EthProofs.receiptLog(receipt, p.logIndex);
        if (log.emitter != counterpart || counterpart == address(0)) revert WrongEmitter(log.emitter);
        if (log.topics.length != 4 || log.topics[0] != topic0) {
            revert WrongEvent(log.topics.length == 0 ? bytes32(0) : log.topics[0]);
        }
        uint256 nonce = uint256(log.topics[1]);
        if (processed[nonce]) revert AlreadyProcessed(nonce);
        processed[nonce] = true;
    }
}
