// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IPermit2Expiring - Permit2-style AllowanceTransfer where every allowance expires (plan §16.5)
/// @notice Shape follows Uniswap Permit2 `IAllowanceTransfer` so integrations port easily. Differences:
///         - every non-zero allowance needs `now < expiration <= now + MAX_EXPIRY_WINDOW` (no permanent
///           allowances; Uniswap's "expiration 0 = this block" shortcut is not supported);
///         - `allowance()` reports amount 0 for expired / revoked allowances;
///         - signed permits carry the owner's `epoch`; `revokeAll()` bumps it, which kills every allowance
///           AND every not-yet-submitted permit signature of the owner;
///         - EIP-712 domain name is "Permit2Expiring" (no version), computed per call from
///           `block.chainid` and `address(this)` (genesis predeploy, no constructor, no immutables).
interface IPermit2Expiring {
    struct PermitDetails {
        address token;
        uint160 amount;
        uint48 expiration;
        uint48 nonce;
    }

    struct PermitSingle {
        PermitDetails details;
        address spender;
        uint256 sigDeadline;
        uint48 epoch;
    }

    struct PermitBatch {
        PermitDetails[] details;
        address spender;
        uint256 sigDeadline;
        uint48 epoch;
    }

    struct AllowanceTransferDetails {
        address from;
        address to;
        uint160 amount;
        address token;
    }

    struct TokenSpenderPair {
        address token;
        address spender;
    }

    /// @notice Allowance written via `approve` (also emitted for amount 0 = revoke).
    event Approval(
        address indexed owner,
        address indexed token,
        address indexed spender,
        uint160 amount,
        uint48 expiration
    );
    /// @notice Allowance written via a signed permit.
    event Permit(
        address indexed owner,
        address indexed token,
        address indexed spender,
        uint160 amount,
        uint48 expiration,
        uint48 nonce
    );
    /// @notice Allowance zeroed via `lockdown`.
    event Lockdown(address indexed owner, address token, address spender);
    event NonceInvalidation(
        address indexed owner,
        address indexed token,
        address indexed spender,
        uint48 newNonce,
        uint48 oldNonce
    );
    /// @notice Every allowance and unsubmitted permit of `owner` is dead.
    event AllRevoked(address indexed owner, uint48 newEpoch);

    error InvalidExpiration(uint48 expiration, uint256 maxExpiration);
    error AllowanceExpired(uint256 deadline);
    error InsufficientAllowance(uint256 amount);
    error SignatureExpired(uint256 signatureDeadline);
    error InvalidSignature();
    error InvalidNonce();
    error StaleEpoch(uint48 signedEpoch, uint48 currentEpoch);
    error ExcessiveInvalidation();

    function DOMAIN_SEPARATOR() external view returns (bytes32);

    /// @return amount live amount (0 if expired / revoked), expiration, nonce (next permit nonce)
    function allowance(address user, address token, address spender)
        external
        view
        returns (uint160 amount, uint48 expiration, uint48 nonce);

    function epochOf(address user) external view returns (uint48);

    function approve(address token, address spender, uint160 amount, uint48 expiration) external;
    function permit(address owner, PermitSingle calldata permitSingle, bytes calldata signature) external;
    function permit(address owner, PermitBatch calldata permitBatch, bytes calldata signature) external;
    function transferFrom(address from, address to, uint160 amount, address token) external;
    function transferFrom(AllowanceTransferDetails[] calldata transferDetails) external;
    function lockdown(TokenSpenderPair[] calldata approvals) external;
    function revokeAll() external;
    function invalidateNonces(address token, address spender, uint48 newNonce) external;
}
