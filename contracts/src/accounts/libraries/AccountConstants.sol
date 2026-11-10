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
