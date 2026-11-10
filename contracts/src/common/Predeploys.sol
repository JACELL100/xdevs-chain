// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title Predeploys
/// @notice Fixed addresses of xDEVS Chain system contracts (plan §18) and the protocol system caller (§18.2).
/// @dev These contracts are placed in the genesis state by the Rust genesis builder. Constructors therefore
///      never run at their final address: every predeploy must be configured through a one-time
///      `initialize(...)` call instead of constructor arguments, and must not cache `address(this)`.
library Predeploys {
    /// @notice Caller used for protocol-initiated system calls (no signature, no gas charge).
    address internal constant SYSTEM_ADDRESS = 0xffffFFFfFFffffffffffffffFfFFFfffFFFfFFfE;

    address internal constant STAKING = 0x0000000000000000000000000000000000001000;
    address internal constant GOVERNANCE = 0x0000000000000000000000000000000000001001;
    address internal constant TREASURY = 0x0000000000000000000000000000000000001002;
    address internal constant FEE_CONFIG = 0x0000000000000000000000000000000000001003;
    address internal constant XNS_REGISTRY = 0x0000000000000000000000000000000000001004;
    address internal constant SMART_ACCOUNT_IMPL = 0x0000000000000000000000000000000000001005;
    address internal constant SMART_ACCOUNT_FACTORY = 0x0000000000000000000000000000000000001006;
    address internal constant PAYMASTER = 0x0000000000000000000000000000000000001007;
    address internal constant PERMIT2_EXPIRING = 0x0000000000000000000000000000000000001008;
    address internal constant WXDS = 0x0000000000000000000000000000000000001009;

    /// @notice RIP-7212 / EIP-7951 P-256 signature verification precompile (plan §10.3).
    address internal constant P256_VERIFY = 0x0000000000000000000000000000000000000100;
}
