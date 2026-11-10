// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @notice A single call made by the account. Same ABI layout as eth-infinitism `BaseAccount.Call`, so
///         `executeBatch((address,uint256,bytes)[])` has the same selector as `Simple7702Account.executeBatch`.
struct Call {
    address target;
    uint256 value;
    bytes data;
}

/// @notice Kind of an account signer.
enum SignerType {
    /// @dev Unused / not a signer.
    None,
    /// @dev secp256k1 key identified by its Ethereum address (MetaMask, hardware wallet, ...).
    ECDSA,
    /// @dev P-256 (secp256r1) WebAuthn passkey identified by its public key (x, y).
    Passkey
}

/// @notice An account signer ("owner key").
/// @dev ECDSA: `x = bytes32(uint256(uint160(signerAddress)))`, `y = 0`.
///      Passkey: `(x, y)` is the uncompressed P-256 public key.
struct Signer {
    SignerType kind;
    bytes32 x;
    bytes32 y;
}

/// @notice Guardian Vault policy (plan §16.2 / §16.5).
struct VaultPolicy {
    /// @dev Master switch. When false the account behaves like a plain smart account (everything instant).
    bool enabled;
    /// @dev Allowance policy: when true, unlimited approvals to non-whitelisted spenders are rejected outright
    ///      (instead of being queued).
    bool blockUnlimitedApprovals;
    /// @dev When false (recommended), ERC-1271 `isValidSignature` only accepts hashes pre-approved on-chain via
    ///      the (delayed) `approveHash`, so a thief cannot bypass the vault with off-chain permits (Permit2,
    ///      Seaport orders, ...). When true, any valid signer signature is accepted instantly.
    bool allowInstantSignatures;
    /// @dev Delay applied to queued actions, in seconds. Bounded to [MIN_VAULT_DELAY, MAX_VAULT_DELAY].
    uint64 delay;
    /// @dev Max total native value (wei) of one instant (non-whitelisted) execution.
    uint128 nativeThreshold;
    /// @dev Max total native value (wei) of instant (non-whitelisted) executions per UTC day.
    uint128 dailyLimit;
    /// @dev Max ERC-4337 gas prefund (wei) the account pays per UTC day. Bounds the "thief is the bundler and sets
    ///      an absurd gas price" drain. Ops sponsored by a paymaster are not affected.
    uint128 dailyGasBudget;
}

/// @notice Per-token instant-transfer limits (token units, no oracle).
struct TokenLimit {
    address token;
    uint128 perTx;
    uint128 daily;
}

/// @notice One (target, selector) pair a session key may call. `selector == 0x00000000` means "any function".
struct SessionPermission {
    address target;
    bytes4 selector;
}

/// @notice Per-token spend cap of a session key (ERC-20 transfer / transferFrom / approve amounts).
struct SessionTokenCap {
    address token;
    uint128 cap;
}

/// @notice Session key configuration.
struct SessionKeyConfig {
    /// @dev secp256k1 address of the session key.
    address key;
    uint48 validAfter;
    uint48 validUntil;
    /// @dev Lifetime native value cap (wei).
    uint128 nativeCap;
    SessionPermission[] permissions;
    SessionTokenCap[] tokenCaps;
}

/// @notice One-time initialization parameters of an xDEVS smart account.
struct InitParams {
    /// @dev At least one signer. Passkey-only accounts are allowed.
    Signer[] signers;
    /// @dev Optional guardians (social recovery + vault cancel rights). May be empty.
    address[] guardians;
    /// @dev Guardian approvals required for recovery. 0 when `guardians` is empty.
    uint8 guardianThreshold;
    /// @dev Recovery delay in seconds. 0 = default (48 h).
    uint64 recoveryDelay;
    /// @dev Initial vault policy. `policy.delay == 0` = default (24 h).
    VaultPolicy policy;
    /// @dev Initial whitelist (contracts / recipients / spenders trusted for instant execution).
    address[] whitelist;
    /// @dev Initial per-token limits.
    TokenLimit[] tokenLimits;
}
