// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {IAccountEvents} from "../interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "../libraries/AccountConstants.sol";
import {AccountStorage} from "../libraries/AccountStorage.sol";
import {Call, SignerType} from "../libraries/AccountTypes.sol";

/// @title AccountCore
/// @notice Authorization plumbing shared by the account core and its DELEGATECALL extension.
/// @dev Authority levels:
///      * owner authority : the account itself (EIP-7702 EOA self-call, or an internal self-call), the ERC-4337
///                          EntryPoint (after `validateUserOp` accepted an *owner* signature — session-key
///                          signatures are only accepted for `executeUserOp`), or a registered secp256k1 signer
///                          calling directly.
///      * self-authorized : an internal self-call made by the account's own dispatcher (an instant self-call the
///                          vault allows, or a matured queued action). Marked with an EIP-1153 transient flag so that
///                          an EIP-7702 EOA cannot reach delayed functions by simply sending a tx to itself.
///      Functions that must respect the vault delay use `delayedSelf`: when an owner calls them directly they route
///      themselves through the vault dispatcher, which queues them when the vault is enabled.
abstract contract AccountCore is IAccountEvents {
    /// @dev Routes a self-call through the vault dispatcher (instant or queued).
    function _routeSelfCall(bytes memory data) internal virtual;

    function _isEcdsaSigner(address who) internal view returns (bool) {
        return AccountStorage.signers().signers[bytes32(uint256(uint160(who)))].kind == SignerType.ECDSA;
    }

    function _isOwnerCaller() internal view returns (bool) {
        return msg.sender == address(this) || msg.sender == AccountConstants.ENTRY_POINT
            || _isEcdsaSigner(msg.sender);
    }

    function _requireOwner() internal view {
        if (!_isOwnerCaller()) revert Unauthorized();
    }

    function _requireOwnerOrGuardian() internal view {
        if (!_isOwnerCaller() && !AccountStorage.isGuardian(msg.sender)) revert Unauthorized();
    }

    /// @dev True only inside a self-call issued by `_rawCall`.
    function _isSelfAuthorized() internal view returns (bool flag) {
        if (msg.sender != address(this)) return false;
        bytes32 slot = AccountStorage.SELF_AUTH_TSLOT;
        assembly ("memory-safe") {
            flag := tload(slot)
        }
    }

    modifier onlyOwner() {
        _requireOwner();
        _;
    }

    /// @dev Body runs only when self-authorized; otherwise an owner call is routed through the vault dispatcher,
    ///      which executes it instantly (vault disabled) or queues it (vault enabled).
    modifier delayedSelf() {
        if (_isSelfAuthorized()) {
            _;
        } else {
            _requireOwner();
            _routeSelfCall(msg.data);
        }
    }

    /// @dev Performs a call, bubbling up revert data. Self-calls are flagged as self-authorized for their duration.
    function _rawCall(address target, uint256 value, bytes memory data) internal {
        bool isSelf = target == address(this);
        bytes32 slot = AccountStorage.SELF_AUTH_TSLOT;
        uint256 prev;
        if (isSelf) {
            assembly ("memory-safe") {
                prev := tload(slot)
                tstore(slot, 1)
            }
        }
        (bool ok, bytes memory ret) = target.call{value: value}(data);
        if (isSelf) {
            assembly ("memory-safe") {
                tstore(slot, prev)
            }
        }
        if (!ok) {
            assembly ("memory-safe") {
                revert(add(ret, 0x20), mload(ret))
            }
        }
    }

    function _executeCalls(Call[] memory calls) internal {
        uint256 n = calls.length;
        for (uint256 i; i < n; ++i) {
            _rawCall(calls[i].target, calls[i].value, calls[i].data);
        }
    }

    function _today() internal view returns (uint64) {
        return uint64(block.timestamp / 1 days);
    }
}
