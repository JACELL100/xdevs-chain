// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {SignatureChecker} from "@openzeppelin/contracts/utils/cryptography/SignatureChecker.sol";
import {IPermit2Expiring} from "./IPermit2Expiring.sol";

/// @title Permit2Expiring - expiring allowances for any ERC-20 (predeploy 0x…1008, plan §16.5 / §18)
/// @author THE xDEVS
/// @notice Users `IERC20.approve(Permit2Expiring, …)` once per token; dApps then get *expiring*, per-spender
///         allowances here (directly or through EIP-712 permits) and pull with `transferFrom`.
///         No allowance can outlive `MAX_EXPIRY_WINDOW`, `lockdown` revokes pairs in batch and `revokeAll`
///         kills every allowance plus every unsubmitted signature of the caller in O(1).
/// @dev Genesis predeploy: no constructor, no immutables, no storage needing initialization. The EIP-712
///      domain separator is recomputed from `block.chainid` and `address(this)` on every use.
///      Signatures are checked with OZ SignatureChecker: ECDSA for EOAs, ERC-1271 for smart accounts
///      (the default account type on xDEVS Chain).
contract Permit2Expiring is IPermit2Expiring {
    using SafeERC20 for IERC20;

    /// @notice Longest allowed `expiration - now`.
    uint256 public constant MAX_EXPIRY_WINDOW = 365 days;

    bytes32 private constant _DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,uint256 chainId,address verifyingContract)");
    bytes32 private constant _NAME_HASH = keccak256("Permit2Expiring");

    bytes32 public constant PERMIT_DETAILS_TYPEHASH =
        keccak256("PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)");
    bytes32 public constant PERMIT_SINGLE_TYPEHASH = keccak256(
        "PermitSingle(PermitDetails details,address spender,uint256 sigDeadline,uint48 epoch)"
        "PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
    );
    bytes32 public constant PERMIT_BATCH_TYPEHASH = keccak256(
        "PermitBatch(PermitDetails[] details,address spender,uint256 sigDeadline,uint48 epoch)"
        "PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
    );

    /// @dev One slot: amount | expiration | epoch the allowance was written in.
    struct PackedAllowance {
        uint160 amount;
        uint48 expiration;
        uint48 epoch;
    }

    mapping(address owner => mapping(address token => mapping(address spender => PackedAllowance))) private
        _allowances;
    /// @dev Kept outside PackedAllowance so that revokeAll / lockdown never reset permit nonces (anti-replay).
    mapping(address owner => mapping(address token => mapping(address spender => uint48))) private _nonces;
    /// @inheritdoc IPermit2Expiring
    mapping(address owner => uint48) public epochOf;

    // ------------------------------------------------------------------ EIP-712

    /// @inheritdoc IPermit2Expiring
    function DOMAIN_SEPARATOR() public view returns (bytes32) {
        return keccak256(abi.encode(_DOMAIN_TYPEHASH, _NAME_HASH, block.chainid, address(this)));
    }

    function _hashDetails(PermitDetails calldata d) private pure returns (bytes32) {
        return keccak256(abi.encode(PERMIT_DETAILS_TYPEHASH, d.token, d.amount, d.expiration, d.nonce));
    }

    function hashPermitSingle(PermitSingle calldata p) public pure returns (bytes32) {
        return keccak256(
            abi.encode(PERMIT_SINGLE_TYPEHASH, _hashDetails(p.details), p.spender, p.sigDeadline, p.epoch)
        );
    }

    function hashPermitBatch(PermitBatch calldata p) public pure returns (bytes32) {
        uint256 n = p.details.length;
        bytes32[] memory hashes = new bytes32[](n);
        for (uint256 i; i < n; ++i) {
            hashes[i] = _hashDetails(p.details[i]);
        }
        return keccak256(
            abi.encode(
                PERMIT_BATCH_TYPEHASH, keccak256(abi.encodePacked(hashes)), p.spender, p.sigDeadline, p.epoch
            )
        );
    }

    function _verify(address owner, bytes32 structHash, bytes calldata signature, uint48 signedEpoch)
        private
        view
    {
        uint48 current = epochOf[owner];
        if (signedEpoch != current) revert StaleEpoch(signedEpoch, current);
        bytes32 digest = MessageHashUtils.toTypedDataHash(DOMAIN_SEPARATOR(), structHash);
        if (owner == address(0) || !SignatureChecker.isValidSignatureNowCalldata(owner, digest, signature)) {
            revert InvalidSignature();
        }
    }

    // ------------------------------------------------------------------ views

    /// @inheritdoc IPermit2Expiring
    function allowance(address user, address token, address spender)
        external
        view
        returns (uint160 amount, uint48 expiration, uint48 nonce)
    {
        PackedAllowance memory a = _allowances[user][token][spender];
        nonce = _nonces[user][token][spender];
        if (a.epoch != epochOf[user]) return (0, 0, nonce);
        expiration = a.expiration;
        amount = block.timestamp < a.expiration ? a.amount : 0;
    }

    // ------------------------------------------------------------------ approvals

    /// @inheritdoc IPermit2Expiring
    function approve(address token, address spender, uint160 amount, uint48 expiration) external {
        _write(msg.sender, token, spender, amount, expiration);
        emit Approval(msg.sender, token, spender, amount, amount == 0 ? 0 : expiration);
    }

    /// @inheritdoc IPermit2Expiring
    function permit(address owner, PermitSingle calldata permitSingle, bytes calldata signature) external {
        if (block.timestamp > permitSingle.sigDeadline) revert SignatureExpired(permitSingle.sigDeadline);
        _verify(owner, hashPermitSingle(permitSingle), signature, permitSingle.epoch);
        _permitOne(owner, permitSingle.details, permitSingle.spender);
    }

    /// @inheritdoc IPermit2Expiring
    function permit(address owner, PermitBatch calldata permitBatch, bytes calldata signature) external {
        if (block.timestamp > permitBatch.sigDeadline) revert SignatureExpired(permitBatch.sigDeadline);
        _verify(owner, hashPermitBatch(permitBatch), signature, permitBatch.epoch);
        uint256 n = permitBatch.details.length;
        for (uint256 i; i < n; ++i) {
            _permitOne(owner, permitBatch.details[i], permitBatch.spender);
        }
    }

    function _permitOne(address owner, PermitDetails calldata d, address spender) private {
        uint48 nonce = _nonces[owner][d.token][spender];
        if (d.nonce != nonce) revert InvalidNonce();
        _nonces[owner][d.token][spender] = nonce + 1; // checked: a wrapped nonce would re-enable old permits
        _write(owner, d.token, spender, d.amount, d.expiration);
        emit Permit(owner, d.token, spender, d.amount, d.amount == 0 ? 0 : d.expiration, d.nonce);
    }

    function _write(address owner, address token, address spender, uint160 amount, uint48 expiration)
        private
    {
        if (amount == 0) {
            expiration = 0;
        } else {
            uint256 maxExpiration = block.timestamp + MAX_EXPIRY_WINDOW;
            if (expiration <= block.timestamp || expiration > maxExpiration) {
                revert InvalidExpiration(expiration, maxExpiration);
            }
        }
        _allowances[owner][token][spender] =
            PackedAllowance({amount: amount, expiration: expiration, epoch: epochOf[owner]});
    }

    // ------------------------------------------------------------------ transfers

    /// @inheritdoc IPermit2Expiring
    function transferFrom(address from, address to, uint160 amount, address token) external {
        _transfer(from, to, amount, token);
    }

    /// @inheritdoc IPermit2Expiring
    function transferFrom(AllowanceTransferDetails[] calldata transferDetails) external {
        uint256 n = transferDetails.length;
        for (uint256 i; i < n; ++i) {
            AllowanceTransferDetails calldata t = transferDetails[i];
            _transfer(t.from, t.to, t.amount, t.token);
        }
    }

    function _transfer(address from, address to, uint160 amount, address token) private {
        PackedAllowance storage a = _allowances[from][token][msg.sender];
        PackedAllowance memory m = a;
        if (m.epoch != epochOf[from] || m.expiration == 0) revert InsufficientAllowance(0);
        if (block.timestamp >= m.expiration) revert AllowanceExpired(m.expiration);
        if (m.amount != type(uint160).max) {
            if (amount > m.amount) revert InsufficientAllowance(m.amount);
            unchecked {
                a.amount = m.amount - amount;
            }
        }
        IERC20(token).safeTransferFrom(from, to, amount);
    }

    // ------------------------------------------------------------------ revocation

    /// @inheritdoc IPermit2Expiring
    function lockdown(TokenSpenderPair[] calldata approvals) external {
        uint256 n = approvals.length;
        for (uint256 i; i < n; ++i) {
            TokenSpenderPair calldata p = approvals[i];
            _allowances[msg.sender][p.token][p.spender] =
                PackedAllowance({amount: 0, expiration: 0, epoch: epochOf[msg.sender]});
            emit Lockdown(msg.sender, p.token, p.spender);
        }
    }

    /// @inheritdoc IPermit2Expiring
    function revokeAll() external {
        uint48 e = ++epochOf[msg.sender];
        emit AllRevoked(msg.sender, e);
    }

    /// @inheritdoc IPermit2Expiring
    function invalidateNonces(address token, address spender, uint48 newNonce) external {
        uint48 old = _nonces[msg.sender][token][spender];
        if (newNonce <= old) revert InvalidNonce();
        if (newNonce - old > type(uint16).max) revert ExcessiveInvalidation();
        _nonces[msg.sender][token][spender] = newNonce;
        emit NonceInvalidation(msg.sender, token, spender, newNonce, old);
    }
}
