// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {BridgeBase} from "./BridgeBase.sol";
import {EthProofs} from "./lib/EthProofs.sol";
import {IExecutionHeaderOracle} from "./interfaces/IExecutionHeaderOracle.sol";

/// @title SepoliaTokenVault - Ethereum-side endpoint: lock ERC-20s, release them on proven xDEVS burns
/// @notice `lock` escrows tokens and emits {Locked}; a relayer proves that log on xDEVS Chain, where
///         `XdevsBridge` mints the canonical bridged token. `release` pays out when the xDEVS `XdevsBridge`
///         emitted `BurnedForRelease`, proven against a block certified by the {XdevsLightClient}.
///         Per-token escrow accounting means a release can never pay out more of a token than was locked.
contract SepoliaTokenVault is BridgeBase, ReentrancyGuard {
    using SafeERC20 for IERC20;

    /// @notice keccak256("BurnedForRelease(uint256,address,address,address,uint256)") on xDEVS Chain.
    bytes32 public constant BURNED_TOPIC =
        keccak256("BurnedForRelease(uint256,address,address,address,uint256)");

    mapping(address token => uint256) public escrowed;

    event Locked(
        uint256 indexed nonce,
        address indexed token,
        address indexed sender,
        address recipient,
        uint256 amount
    );
    event Released(uint256 indexed nonce, address indexed token, address indexed recipient, uint256 amount);

    constructor(IExecutionHeaderOracle xdevsLightClient, uint256 xdevsChainId, address guardian_)
        BridgeBase(xdevsLightClient, xdevsChainId, guardian_)
    {}

    /// @notice Escrow `amount` of `token` for `recipient` on xDEVS Chain.
    function lock(address token, uint256 amount, address recipient) external nonReentrant whenNotPaused {
        if (recipient == address(0) || token == address(0)) revert ZeroAddress();
        uint256 before = IERC20(token).balanceOf(address(this));
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
        uint256 received = IERC20(token).balanceOf(address(this)) - before; // fee-on-transfer safe
        // slither-disable-next-line incorrect-equality (BR-02: zero received = nothing locked; not a balance check)
        if (received == 0) revert ZeroAmount();
        escrowed[token] += received;
        uint256 nonce = nextNonce++;
        emit Locked(nonce, token, msg.sender, recipient, received);
    }

    /// @notice Release tokens for a proven `BurnedForRelease(nonce, originToken, sender, recipient, amount)`.
    function release(EventProof calldata p) external nonReentrant whenNotPaused {
        EthProofs.Log memory log = _consume(p, BURNED_TOPIC);
        uint256 nonce = uint256(log.topics[1]);
        address token = address(uint160(uint256(log.topics[2])));
        (address recipient, uint256 amount) = abi.decode(log.data, (address, uint256));
        escrowed[token] -= amount; // reverts if more than was locked
        IERC20(token).safeTransfer(recipient, amount);
        emit Released(nonce, token, recipient, amount);
    }
}
