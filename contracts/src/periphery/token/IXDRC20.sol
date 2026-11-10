// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IXDRC20 - xDEVS expiring-approval extension of ERC-20 (plan §16.5)
/// @notice The XDRC-20 additions on top of a plain ERC-20 + EIP-2612. A token advertises support through
///         ERC-165 `supportsInterface(type(IXDRC20).interfaceId)` so wallets can offer the 3-arg approve.
/// @dev Live-approval semantics: an allowance `(owner, spender)` is live iff it was written in the owner's
///      current `approvalEpoch` and `block.timestamp < allowanceExpiry(owner, spender)`. `allowance()`
///      returns 0 for anything not live and `transferFrom` cannot spend it.
interface IXDRC20 {
    /// @notice Emitted on every allowance write (together with the ERC-20 `Approval` event).
    ///         Indexers build the live-approval set from this event (`xdevs_getApprovals`): the latest
    ///         `ApprovalWithExpiry` for (owner, spender) after the owner's latest `AllApprovalsRevoked`,
    ///         if `value > 0` and `expiry > now`. Spending through `transferFrom` does NOT emit an event
    ///         (OpenZeppelin v5 behaviour); read `allowance()` for the remaining amount.
    event ApprovalWithExpiry(address indexed owner, address indexed spender, uint256 value, uint64 expiry);

    /// @notice All of `owner`'s allowances were revoked at once (epoch bump).
    event AllApprovalsRevoked(address indexed owner, uint32 newEpoch);

    error AllowanceExpired(address owner, address spender, uint64 expiry);
    error InvalidApprovalExpiry(uint64 expiry, uint64 maxExpiry);
    error PermitDeadlineExpired(uint256 deadline);
    error InvalidPermitSignature(address owner);

    /// @notice Approve `spender` for `value` until `expiry` (unix seconds, exclusive).
    /// @dev `value == 0` revokes (expiry ignored). Otherwise `now < expiry <= now + maxApprovalDuration`.
    function approve(address spender, uint256 value, uint64 expiry) external returns (bool);

    /// @notice Expiry of the live allowance of `spender` over `owner`'s tokens; 0 if none / revoked.
    function allowanceExpiry(address owner, address spender) external view returns (uint64);

    /// @notice Revoke every allowance the caller has ever granted (O(1), bumps `approvalEpoch`).
    function revokeAll() external;

    /// @notice Current approval epoch of `owner` (allowances from older epochs are dead).
    function approvalEpoch(address owner) external view returns (uint32);

    /// @notice EIP-2612-style permit with an explicit allowance expiry. Supports EOAs (65-byte ECDSA) and
    ///         smart accounts (ERC-1271). Shares `nonces(owner)` with the standard `permit`.
    function permitWithExpiry(
        address owner,
        address spender,
        uint256 value,
        uint64 expiry,
        uint256 deadline,
        bytes calldata signature
    ) external;

    /// @notice Expiry applied by the legacy 2-arg `approve` and the standard EIP-2612 `permit`.
    function defaultApprovalDuration() external view returns (uint64);

    /// @notice Longest allowed `expiry - now` for any approval.
    function maxApprovalDuration() external view returns (uint64);
}
