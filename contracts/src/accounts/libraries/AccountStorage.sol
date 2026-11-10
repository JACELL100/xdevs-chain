// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Call, Signer, VaultPolicy} from "./AccountTypes.sol";

/// @title AccountStorage
/// @notice ERC-7201 namespaced storage of the xDEVS smart account, shared by the account core
///         (`XdevsSmartAccount`) and its cold-path extension (`AccountExtension`, reached by DELEGATECALL).
/// @dev Namespaces never overlap with storage written by any other EIP-7702 delegate an EOA may have used before
///      (sequential slots 0..n, or other ERC-7201 ids). Slot = keccak256(abi.encode(uint256(keccak256(id)) - 1))
///      & ~bytes32(uint256(0xff)).
library AccountStorage {
    // ----------------------------------------------------------------------------------------------- core ----
    /// @custom:storage-location erc7201:xdevs.account.core
    struct Core {
        bool initialized;
        /// @dev Set by a social recovery: the account address's own secp256k1 key (the EOA key of an EIP-7702
        ///      account) is no longer an implicit signer, unless the new signer set lists it explicitly.
        bool eoaKeyRevoked;
    }

    // -------------------------------------------------------------------------------------------- signers ----
    /// @custom:storage-location erc7201:xdevs.account.signers
    struct Signers {
        mapping(bytes32 id => Signer) signers;
        bytes32[] ids;
        mapping(bytes32 id => uint256) indexPlusOne;
    }

    // ---------------------------------------------------------------------------------------------- vault ----
    enum ActionStatus {
        None,
        Queued,
        Cancelled,
        Executed
    }

    struct QueuedAction {
        uint64 executeAfter;
        uint32 epoch;
        ActionStatus status;
        Call[] calls;
    }

    struct TokenLimitState {
        uint128 perTx;
        uint128 daily;
        uint64 day;
        uint128 spent;
    }

    /// @custom:storage-location erc7201:xdevs.account.vault
    struct Vault {
        VaultPolicy policy;
        uint64 day;
        uint128 spentToday;
        uint64 gasDay;
        uint128 gasSpent;
        uint32 epoch;
        uint256 lastId;
        mapping(uint256 id => QueuedAction) actions;
        address[] whitelist;
        mapping(address => uint256) whitelistIndexPlusOne;
        mapping(address token => TokenLimitState) tokenLimits;
        mapping(bytes32 hash => bool) approvedHashes;
    }

    // ------------------------------------------------------------------------------------------- recovery ----
    struct Proposal {
        uint8 approvals;
        uint64 readyAt;
    }

    /// @custom:storage-location erc7201:xdevs.account.recovery
    struct Recovery {
        address[] guardians;
        mapping(address => bool) isGuardian;
        uint8 threshold;
        uint64 delay;
        uint64 nonce;
        /// @dev Guardian freeze: frozen iff `block.timestamp < frozenUntil` (packs into the threshold/delay slot).
        uint64 frozenUntil;
        /// @dev Recovery proposals AND guardian votes (freeze / unfreeze / cancel; tagged ids, see AccountCore).
        mapping(bytes32 id => Proposal) proposals;
        mapping(bytes32 id => mapping(address guardian => bool)) approvedBy;
        /// @dev Start / effective end of the last freeze (cooldown bookkeeping; reset by a recovery).
        uint64 freezeStart;
        uint64 freezeEnd;
        /// @dev Configured freeze duration; 0 = DEFAULT_FREEZE_DURATION.
        uint64 freezeDuration;
        /// @dev Bumped on every guardian-set change: invalidates freeze / unfreeze / cancel votes (the recovery
        ///      nonce cannot be used: signers bump it with `cancelRecovery`).
        uint32 guardianEpoch;
        /// @dev Bumped on every freeze start / end: votes are single-use.
        uint32 freezeNonce;
    }

    // ------------------------------------------------------------------------------------------- sessions ----
    struct SessionState {
        uint64 nonce;
        uint48 validAfter;
        uint48 validUntil;
        bool active;
        uint128 nativeCap;
        uint128 nativeSpent;
    }

    struct TokenCapState {
        uint128 cap;
        uint128 spent;
    }

    /// @custom:storage-location erc7201:xdevs.account.sessions
    struct Sessions {
        uint64 lastNonce;
        /// @dev Every session with `nonce <= revokedUpTo` is revoked (bulk revocation on recovery).
        uint64 revokedUpTo;
        mapping(address key => SessionState) sessions;
        mapping(bytes32 => bool) permissions; // keccak256(abi.encode(key, nonce, target, selector))
        mapping(bytes32 => TokenCapState) tokenCaps; // keccak256(abi.encode(key, nonce, token))
    }

    bytes32 internal constant CORE_SLOT = 0x4fb707dc31f3c888e2f3b1f3950af08abe11750bfa6dde38bfbc009dfff4e600;
    bytes32 internal constant SIGNERS_SLOT =
        0x21ef5ba36445439568de9d93ffe50a1b636abb2f9f1dbdf7632c73b4de7b5700;
    bytes32 internal constant VAULT_SLOT = 0x3b7942c4632d59ecbc64fa168430d4f52e480831291d1cfa5d411ad6960f6800;
    bytes32 internal constant RECOVERY_SLOT =
        0x16baddd4c0fda797a0d3882b67686eb4bfc7c2204c2df624c7e02b7d760da300;
    bytes32 internal constant SESSIONS_SLOT =
        0x67bd91acb6e6fc2368bd19707c2fc36943099bf0066891511ab3c54667060000;
    /// @dev Transient (EIP-1153) slot of the self-authorization flag (`xdevs.account.transient.selfauth`).
    bytes32 internal constant SELF_AUTH_TSLOT =
        0x88ad6324a261da9ba14b4c392d05cf2b1f49063cd1187e8e45deee7afae23600;

    function core() internal pure returns (Core storage $) {
        assembly ("memory-safe") {
            $.slot := CORE_SLOT
        }
    }

    function signers() internal pure returns (Signers storage $) {
        assembly ("memory-safe") {
            $.slot := SIGNERS_SLOT
        }
    }

    function vault() internal pure returns (Vault storage $) {
        assembly ("memory-safe") {
            $.slot := VAULT_SLOT
        }
    }

    function recovery() internal pure returns (Recovery storage $) {
        assembly ("memory-safe") {
            $.slot := RECOVERY_SLOT
        }
    }

    function sessions() internal pure returns (Sessions storage $) {
        assembly ("memory-safe") {
            $.slot := SESSIONS_SLOT
        }
    }

    // --------------------------------------------------------------------------------------- shared views ----
    function isWhitelisted(address who) internal view returns (bool) {
        return vault().whitelistIndexPlusOne[who] != 0;
    }

    function isGuardian(address who) internal view returns (bool) {
        return recovery().isGuardian[who];
    }

    function permissionKey(address key, uint64 nonce, address target, bytes4 selector)
        internal
        pure
        returns (bytes32)
    {
        return keccak256(abi.encode(key, nonce, target, selector));
    }

    function tokenCapKey(address key, uint64 nonce, address token) internal pure returns (bytes32) {
        return keccak256(abi.encode(key, nonce, token));
    }
}
