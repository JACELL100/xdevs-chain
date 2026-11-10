// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {SignatureChecker} from "@openzeppelin/contracts/utils/cryptography/SignatureChecker.sol";
import {Nonces} from "@openzeppelin/contracts/utils/Nonces.sol";
import {IERC165} from "@openzeppelin/contracts/utils/introspection/IERC165.sol";
import {IXDRC20} from "./IXDRC20.sol";

/// @title XDRC20 - reference XDRC-20 token (ERC-20 + expiring approvals), plan §1.4 / §16.5
/// @author THE xDEVS
/// @notice A fully ERC-20 / EIP-2612 compatible token in which **every allowance expires**:
///         - `approve(spender, value, expiry)` sets an explicit expiry (bounded by `maxApprovalDuration`);
///         - legacy `approve(spender, value)` and standard `permit(...)` get `now + defaultApprovalDuration`;
///         - `allowance()` returns 0 once expired and `transferFrom` reverts with {AllowanceExpired};
///         - `revokeAll()` kills every outstanding allowance of the caller in O(1).
/// @dev Compatibility trade-off of the legacy 2-arg `approve`: dApps that "approve max once" will find the
///      allowance gone after `defaultApprovalDuration` and must ask the user to approve again. That is the
///      point (stale unlimited approvals are the #1 drainer vector), but integrators must handle it.
///      This contract is deployed normally (not a predeploy), so constructor args and OZ EIP712's cached
///      domain separator (which self-heals on chain-id change) are fine here.
contract XDRC20 is ERC20, EIP712, Nonces, IERC20Permit, IXDRC20, IERC165 {
    bytes32 private constant PERMIT_TYPEHASH =
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");
    bytes32 private constant PERMIT_WITH_EXPIRY_TYPEHASH = keccak256(
        "PermitWithExpiry(address owner,address spender,uint256 value,uint64 expiry,uint256 nonce,uint256 deadline)"
    );

    struct ApprovalMeta {
        uint64 expiry;
        uint32 epoch;
    }

    uint64 public immutable defaultApprovalDuration;
    uint64 public immutable maxApprovalDuration;

    mapping(address owner => mapping(address spender => ApprovalMeta)) private _meta;
    mapping(address owner => uint32) public approvalEpoch;

    error InvalidDurations(uint64 defaultDuration, uint64 maxDuration);

    /// @param name_ ERC-20 name (also the EIP-712 domain name, version "1").
    /// @param symbol_ ERC-20 symbol.
    /// @param recipient Receives `initialSupply`.
    /// @param initialSupply Fixed supply minted at deploy (18 decimals).
    /// @param defaultDuration Expiry window for legacy approve/permit, e.g. 30 days.
    /// @param maxDuration Upper bound for any approval window, e.g. 365 days (>= defaultDuration).
    constructor(
        string memory name_,
        string memory symbol_,
        address recipient,
        uint256 initialSupply,
        uint64 defaultDuration,
        uint64 maxDuration
    ) ERC20(name_, symbol_) EIP712(name_, "1") {
        if (defaultDuration == 0 || defaultDuration > maxDuration) {
            revert InvalidDurations(defaultDuration, maxDuration);
        }
        defaultApprovalDuration = defaultDuration;
        maxApprovalDuration = maxDuration;
        if (initialSupply != 0) _mint(recipient, initialSupply);
    }

    // ------------------------------------------------------------------ views

    function _isLive(address owner, address spender) internal view returns (bool) {
        ApprovalMeta memory m = _meta[owner][spender];
        return m.epoch == approvalEpoch[owner] && block.timestamp < m.expiry;
    }

    /// @inheritdoc ERC20
    /// @dev 0 for expired or revoked allowances.
    function allowance(address owner, address spender) public view override returns (uint256) {
        return _isLive(owner, spender) ? super.allowance(owner, spender) : 0;
    }

    /// @inheritdoc IXDRC20
    function allowanceExpiry(address owner, address spender) external view returns (uint64) {
        ApprovalMeta memory m = _meta[owner][spender];
        if (m.epoch != approvalEpoch[owner] || super.allowance(owner, spender) == 0) return 0;
        return m.expiry;
    }

    function supportsInterface(bytes4 interfaceId) external pure returns (bool) {
        return interfaceId == type(IXDRC20).interfaceId || interfaceId == type(IERC165).interfaceId;
    }

    // ------------------------------------------------------------------ approvals

    /// @notice Legacy ERC-20 approve: allowance expires after `defaultApprovalDuration`.
    function approve(address spender, uint256 value) public override returns (bool) {
        _approveWithExpiry(msg.sender, spender, value, uint64(block.timestamp) + defaultApprovalDuration);
        return true;
    }

    /// @inheritdoc IXDRC20
    function approve(address spender, uint256 value, uint64 expiry) external returns (bool) {
        _checkExpiry(value, expiry);
        _approveWithExpiry(msg.sender, spender, value, expiry);
        return true;
    }

    /// @inheritdoc IXDRC20
    function revokeAll() external {
        uint32 e = ++approvalEpoch[msg.sender];
        emit AllApprovalsRevoked(msg.sender, e);
    }

    function _checkExpiry(uint256 value, uint64 expiry) internal view {
        if (value == 0) return;
        uint64 maxExpiry = uint64(block.timestamp) + maxApprovalDuration;
        if (expiry <= block.timestamp || expiry > maxExpiry) revert InvalidApprovalExpiry(expiry, maxExpiry);
    }

    function _approveWithExpiry(address owner, address spender, uint256 value, uint64 expiry) internal {
        if (value == 0) expiry = 0;
        _meta[owner][spender] = ApprovalMeta({expiry: expiry, epoch: approvalEpoch[owner]});
        _approve(owner, spender, value); // validates zero addresses, emits Approval
        emit ApprovalWithExpiry(owner, spender, value, expiry);
    }

    /// @dev Expired/revoked allowances can never be spent (a zero-value transferFrom is a no-op as in ERC-20).
    function _spendAllowance(address owner, address spender, uint256 value) internal override {
        if (value != 0 && !_isLive(owner, spender)) {
            ApprovalMeta memory m = _meta[owner][spender];
            if (m.epoch == approvalEpoch[owner] && m.expiry != 0 && super.allowance(owner, spender) != 0) {
                revert AllowanceExpired(owner, spender, m.expiry);
            }
            revert ERC20InsufficientAllowance(spender, 0, value);
        }
        super._spendAllowance(owner, spender, value);
    }

    // ------------------------------------------------------------------ permits

    /// @notice Standard EIP-2612 permit. The resulting allowance expires after `defaultApprovalDuration`,
    ///         exactly like the legacy `approve`.
    /// @dev `(v, r, s)` is checked with OZ {SignatureChecker} (OZ >= 5.0 semantics): an owner WITHOUT code is
    ///      verified by ECDSA; an owner WITH code — a contract wallet or an EIP-7702-delegated EOA, whose code is
    ///      the 0xef0100 designator — is verified ONLY through its ERC-1271 `isValidSignature` (with
    ///      `abi.encodePacked(r, s, v)`), never by ECDSA. A plain ECDSA check would let a thief holding the EOA
    ///      key of a 7702 smart account sign token permits behind the account's EOA lock / Guardian Vault.
    function permit(
        address owner,
        address spender,
        uint256 value,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        if (block.timestamp > deadline) revert PermitDeadlineExpired(deadline);
        bytes32 digest = _hashTypedDataV4(
            keccak256(abi.encode(PERMIT_TYPEHASH, owner, spender, value, _useNonce(owner), deadline))
        );
        if (
            owner == address(0)
                || !SignatureChecker.isValidSignatureNow(owner, digest, abi.encodePacked(r, s, v))
        ) {
            revert InvalidPermitSignature(owner);
        }
        _approveWithExpiry(owner, spender, value, uint64(block.timestamp) + defaultApprovalDuration);
    }

    /// @inheritdoc IXDRC20
    function permitWithExpiry(
        address owner,
        address spender,
        uint256 value,
        uint64 expiry,
        uint256 deadline,
        bytes calldata signature
    ) external {
        if (block.timestamp > deadline) revert PermitDeadlineExpired(deadline);
        _checkExpiry(value, expiry);
        bytes32 digest = _hashTypedDataV4(
            keccak256(
                abi.encode(
                    PERMIT_WITH_EXPIRY_TYPEHASH, owner, spender, value, expiry, _useNonce(owner), deadline
                )
            )
        );
        if (owner == address(0) || !SignatureChecker.isValidSignatureNowCalldata(owner, digest, signature)) {
            revert InvalidPermitSignature(owner);
        }
        _approveWithExpiry(owner, spender, value, expiry);
    }

    /// @inheritdoc IERC20Permit
    function nonces(address owner) public view override(IERC20Permit, Nonces) returns (uint256) {
        return super.nonces(owner);
    }

    /// @inheritdoc IERC20Permit
    // solhint-disable-next-line func-name-mixedcase
    function DOMAIN_SEPARATOR() external view returns (bytes32) {
        return _domainSeparatorV4();
    }
}
