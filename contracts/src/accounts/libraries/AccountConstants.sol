// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Predeploys} from "../../common/Predeploys.sol";

/// @title AccountConstants
/// @notice Constants shared by the xDEVS smart account, factory and paymaster.
/// @dev Everything is a compile-time constant (never an immutable), because the contracts are placed into the
///      genesis state without running a constructor at their final address.
library AccountConstants {
    /// @notice Canonical ERC-4337 v0.9 EntryPoint (same address as on Ethereum; inserted in genesis).
    address internal constant ENTRY_POINT = 0x433709009B8330FDa32311DF1C2AFA402eD8D009;

    /// @notice Where the account's cold-path logic (`AccountExtension`) lives. The account core forwards unknown
    ///         selectors there with DELEGATECALL. Hosted by the factory predeploy so no extra genesis address is
    ///         needed (the core alone would exceed the EIP-170 24 KiB limit).
    address internal constant ACCOUNT_EXTENSION = Predeploys.SMART_ACCOUNT_FACTORY;

    /// @notice Guardian Vault default delay (plan §33.1).
    uint64 internal constant DEFAULT_VAULT_DELAY = 24 hours;
    /// @notice Guardian Vault delay bounds (plan §33.1: user-configurable 1 h – 7 d).
    uint64 internal constant MIN_VAULT_DELAY = 1 hours;
    uint64 internal constant MAX_VAULT_DELAY = 7 days;
    /// @notice A matured vault action must be executed within this window, otherwise it expires.
    uint64 internal constant VAULT_EXECUTION_WINDOW = 14 days;

    /// @notice Social recovery default delay (plan §33.1) and bounds.
    uint64 internal constant DEFAULT_RECOVERY_DELAY = 48 hours;
    uint64 internal constant MIN_RECOVERY_DELAY = 1 hours;
    uint64 internal constant MAX_RECOVERY_DELAY = 30 days;

    /// @notice Guardian freeze (see `AccountExtension.freeze`). A freeze lasts `max(freezeDuration, recoveryDelay +
    ///         FREEZE_RECOVERY_GRACE)` and never more than `MAX_FREEZE_DURATION`; after it ends, a new freeze is only
    ///         possible once the account has been unfrozen for as long as it was frozen (anti-hostage cooldown).
    uint64 internal constant DEFAULT_FREEZE_DURATION = 30 days;
    uint64 internal constant MIN_FREEZE_DURATION = 3 days;
    uint64 internal constant MAX_FREEZE_DURATION = 60 days;
    /// @dev A recovery approved right after the freeze always matures (and can be executed) while still frozen.
    ///      MAX_RECOVERY_DELAY + FREEZE_RECOVERY_GRACE <= MAX_FREEZE_DURATION.
    uint64 internal constant FREEZE_RECOVERY_GRACE = 3 days;

    /// @notice XIP-0001 "EOA lock" word of an EIP-7702 delegated EOA: `bytes32(uint256(keccak256(
    ///         "xdevs.account.eoaLock")) - 1)`. A plain storage slot (not ERC-7201-masked, not inside a struct) so
    ///         the node can read it without any ABI knowledge. Values: 0 = unlocked, `EOA_LOCKED` (1) = the EOA may
    ///         only send value-0 transactions to itself, `EOA_SEALED` (2) = the EOA may send no transaction at all
    ///         (written while the account is frozen or after a recovery revoked the EOA key). Any other non-zero
    ///         value (garbage from a previous delegate) is treated as LOCKED by the node.
    /// @dev The node applies the rules iff the code is exactly `0xef0100 || 0x…1005` (docs/specs/xip-0001-eoa-lock.md).
    bytes32 internal constant EOA_LOCK_SLOT =
        0xfd81d7eef3094fecca95e5e53fe72709c7315b39f65e414992ead9c855e841ce;
    uint256 internal constant EOA_LOCKED = 1;
    uint256 internal constant EOA_SEALED = 2;
    /// @notice keccak256(0xef0100 || 0x…1005): code hash of an EOA delegated to the account implementation.
    bytes32 internal constant EIP7702_DESIGNATOR_HASH =
        0x4879b5ba5d3e15615c1dc61bbc61a36231da1b95582299c433c2edbee1ad7333;

    uint256 internal constant MAX_SIGNERS = 16;
    uint256 internal constant MAX_GUARDIANS = 16;
    uint256 internal constant MAX_WHITELIST = 64;
    uint256 internal constant MAX_SESSION_PERMISSIONS = 32;

    /// @notice Unlimited-approval detection: amounts >= this are treated as "unlimited" (covers uint256.max and
    ///         Permit2's uint160.max).
    uint256 internal constant UNLIMITED_APPROVAL = type(uint160).max;

    /// @notice Default daily ERC-4337 gas prefund budget when a vault policy leaves it at 0.
    uint128 internal constant DEFAULT_DAILY_GAS_BUDGET = 1 ether;
}
