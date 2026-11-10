// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {
    Call,
    InitParams,
    Signer,
    SignerType,
    VaultPolicy,
    SessionKeyConfig
} from "../libraries/AccountTypes.sol";

/// @title IAccountEvents
/// @notice Events and errors shared by the account core and its extension (Guardian Vault queue events are in
///         `IGuardianVault`).
interface IAccountEvents {
    // core / init
    event AccountInitialized(uint256 signers, uint256 guardians, bool vaultEnabled);
    // signers
    event SignerAdded(bytes32 indexed id, SignerType kind, bytes32 x, bytes32 y);
    event SignerRemoved(bytes32 indexed id);
    // vault
    event VaultPolicyUpdated(VaultPolicy policy);
    event WhitelistUpdated(address indexed who, bool trusted);
    event TokenLimitUpdated(address indexed token, uint128 perTx, uint128 daily);
    event HashApproved(bytes32 indexed hash, bool approved);
    event VaultEpochBumped(uint32 epoch);
    // recovery
    event GuardiansUpdated(address[] guardians, uint8 threshold);
    event RecoveryDelayUpdated(uint64 delay);
    event RecoveryApproved(
        bytes32 indexed id, address indexed guardian, uint8 approvals, Signer[] newSigners
    );
    event RecoveryReady(bytes32 indexed id, uint64 readyAt);
    event RecoveryExecuted(bytes32 indexed id);
    event RecoveryCancelled(uint64 newNonce);
    // sessions
    event SessionKeyAdded(
        address indexed key, uint64 nonce, uint48 validAfter, uint48 validUntil, uint128 nativeCap
    );
    event SessionKeyRevoked(address indexed key);
    event AllSessionKeysRevoked(uint64 upToNonce);

    error Unauthorized();
    error InvalidParameter();
    error AlreadyInitialized();
    // signers
    error InvalidSigner();
    error SignerAlreadyExists(bytes32 id);
    error SignerNotFound(bytes32 id);
    error TooManySigners();
    error LastSigner();
    // vault
    error ActionNotQueued(uint256 id);
    error ActionNotReady(uint256 id, uint64 executeAfter);
    error ActionExpired(uint256 id);
    error ActionInvalidated(uint256 id);
    error UnlimitedApprovalBlocked(address spender);
    error SessionActionNeedsDelay();
    error TooManyWhitelisted();
    // recovery
    error NotGuardian();
    error AlreadyApproved();
    error RecoveryNotReady();
    error InvalidGuardians();
    // sessions
    error InvalidSession();
    error SessionNotActive(address key);
    error SessionCallNotAllowed(address target, bytes4 selector);
    error SessionNativeCapExceeded();
    error SessionTokenCapExceeded(address token);
    // upgrades / extension
    error NotProxy();
    error InvalidImplementation();
}

/// @title IAccountExtension
/// @notice Cold-path functions of an xDEVS smart account. They are served by `AccountExtension` (hosted in the
///         `SmartAccountFactory` predeploy) through the account's fallback via DELEGATECALL, so callers simply call
///         them on the account address.
interface IAccountExtension {
    // init
    function initialize(InitParams calldata params) external;
    function initializeWithSignature(InitParams calldata params, bytes calldata eoaSignature) external;
    function initHash(InitParams calldata params) external view returns (bytes32);
    function isInitialized() external view returns (bool);

    // signers
    function addSigner(Signer calldata s) external;
    function removeSigner(bytes32 id) external;
    function getSigners() external view returns (Signer[] memory);
    function isSigner(bytes32 id) external view returns (bool);
    function signerId(Signer calldata s) external pure returns (bytes32);

    // vault policy
    function setVaultPolicy(VaultPolicy calldata p) external;
    function setWhitelisted(address who, bool trusted) external;
    function setTokenLimit(address token, uint128 perTx, uint128 daily) external;
    function approveHash(bytes32 hash) external;
    function revokeHash(bytes32 hash) external;
    function vaultPolicy() external view returns (VaultPolicy memory);
    function isWhitelisted(address who) external view returns (bool);
    function getWhitelist() external view returns (address[] memory);
    function tokenLimit(address token)
        external
        view
        returns (uint128 perTx, uint128 daily, uint64 day, uint128 spent);
    function getAction(uint256 id)
        external
        view
        returns (uint64 executeAfter, uint8 status, bool valid, Call[] memory calls);
    function lastActionId() external view returns (uint256);
    function spentToday() external view returns (uint128);
    function isHashApproved(bytes32 hash) external view returns (bool);
    function gasBudgetSpent() external view returns (uint128);

    // recovery
    function approveRecovery(Signer[] calldata newSigners) external;
    function executeRecovery(Signer[] calldata newSigners) external;
    function cancelRecovery() external;
    function setGuardians(address[] calldata guardians, uint8 threshold) external;
    function setRecoveryDelay(uint64 delay) external;
    function getGuardians() external view returns (address[] memory guardians, uint8 threshold);
    function isGuardian(address who) external view returns (bool);
    function recoveryDelay() external view returns (uint64);
    function recoveryNonce() external view returns (uint64);
    function getRecoveryProposal(Signer[] calldata newSigners)
        external
        view
        returns (bytes32 id, uint8 approvals, uint64 readyAt);

    // sessions
    function addSessionKey(SessionKeyConfig calldata cfg) external;
    function revokeSessionKey(address key) external;
    function revokeAllSessionKeys() external;
    function getSession(address key)
        external
        view
        returns (
            uint64 nonce,
            uint48 validAfter,
            uint48 validUntil,
            bool active,
            uint128 nativeCap,
            uint128 nativeSpent
        );
    function isSessionPermitted(address key, address target, bytes4 selector) external view returns (bool);
    function sessionTokenCap(address key, address token) external view returns (uint128 cap, uint128 spent);

    // upgrades
    function upgradeToAndCall(address newImplementation, bytes calldata data) external;
}
