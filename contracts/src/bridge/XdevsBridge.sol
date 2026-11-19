// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {BridgeBase} from "./BridgeBase.sol";
import {EthProofs} from "./lib/EthProofs.sol";
import {IExecutionHeaderOracle} from "./interfaces/IExecutionHeaderOracle.sol";
import {CanonicalTokenFactory} from "../periphery/token/CanonicalTokenFactory.sol";
import {XDRC20Bridged} from "../periphery/token/XDRC20Bridged.sol";

/// @title XdevsBridge - xDEVS-side endpoint: mint canonical tokens for proven Sepolia locks, burn to release
/// @notice `mint` proves a `Locked` log of the {SepoliaTokenVault} against a Sepolia block verified by the
///         {EthereumLightClient} and mints the canonical bridged token of {CanonicalTokenFactory} (plan §22.3:
///         one verified token per origin asset; governance creates it with this contract as its `bridge`).
///         `burn` destroys bridged tokens and emits {BurnedForRelease}, which the vault releases on Sepolia.
contract XdevsBridge is BridgeBase {
    /// @notice keccak256("Locked(uint256,address,address,address,uint256)") on Sepolia.
    bytes32 public constant LOCKED_TOPIC = keccak256("Locked(uint256,address,address,address,uint256)");

    CanonicalTokenFactory public immutable factory;

    event Minted(uint256 indexed nonce, address indexed token, address indexed recipient, uint256 amount);
    event BurnedForRelease(
        uint256 indexed nonce,
        address indexed originToken,
        address indexed sender,
        address recipient,
        uint256 amount
    );

    error NoCanonicalToken(address originToken);
    error NotBridgedHere(address token);

    constructor(
        IExecutionHeaderOracle ethereumLightClient,
        uint256 sepoliaChainId,
        address guardian_,
        CanonicalTokenFactory factory_
    ) BridgeBase(ethereumLightClient, sepoliaChainId, guardian_) {
        if (address(factory_) == address(0)) revert ZeroAddress();
        factory = factory_;
    }

    /// @notice Mint for a proven `Locked(nonce, token, sender, recipient, amount)` of the Sepolia vault.
    function mint(EventProof calldata p) external whenNotPaused {
        EthProofs.Log memory log = _consume(p, LOCKED_TOPIC);
        uint256 nonce = uint256(log.topics[1]);
        address originToken = address(uint160(uint256(log.topics[2])));
        (address recipient, uint256 amount) = abi.decode(log.data, (address, uint256));
        address token = factory.canonicalToken(remoteChainId, originToken);
        if (token == address(0)) revert NoCanonicalToken(originToken);
        if (XDRC20Bridged(token).bridge() != address(this)) revert NotBridgedHere(token);
        emit Minted(nonce, token, recipient, amount);
        XDRC20Bridged(token).mint(recipient, amount);
    }

    /// @notice Burn `amount` of the canonical bridged `token` and release the origin token to `recipient` on
    ///         Sepolia.
    function burn(address token, uint256 amount, address recipient) external whenNotPaused {
        if (recipient == address(0)) revert ZeroAddress();
        if (amount == 0) revert ZeroAmount();
        (uint256 originChain, address originToken) = factory.originOf(token);
        if (
            originChain != remoteChainId || originToken == address(0)
                || XDRC20Bridged(token).bridge() != address(this)
        ) {
            revert NotBridgedHere(token);
        }
        uint256 nonce = nextNonce++;
        emit BurnedForRelease(nonce, originToken, msg.sender, recipient, amount);
        XDRC20Bridged(token).burn(msg.sender, amount); // reverts the whole call on insufficient balance
    }
}
