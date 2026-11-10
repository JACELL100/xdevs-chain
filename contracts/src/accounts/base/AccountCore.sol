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
///
///      The "EOA key" of an EIP-7702 account acts as `msg.sender == tx.origin == address(this)` (a transaction the EOA
///      sends to itself — under XIP-0001 the only transaction a locked EOA may send) or as the account address
///      recovered from a signature. A social recovery revokes it (`Core.eoaKeyRevoked`) unless the new signer set
///      lists the account address explicitly.
///
///      Freeze: while `block.timestamp < Recovery.frozenUntil` no owner / session authority is accepted (enforced
///      here in `_requireOwner` and in the dispatcher); only guardians and permissionless recovery execution work.
abstract contract AccountCore is IAccountEvents {
    /// @dev Guardian vote kinds (tags of the vote ids, see `_voteId`).
    uint8 internal constant VOTE_FREEZE = 0;
    uint8 internal constant VOTE_UNFREEZE = 1;
    uint8 internal constant VOTE_CANCEL = 2;

    /// @dev Routes a self-call through the vault dispatcher (instant or queued).
    function _routeSelfCall(bytes memory data) internal virtual;

    function _isEcdsaSigner(address who) internal view returns (bool) {
        return AccountStorage.signers().signers[bytes32(uint256(uint160(who)))].kind == SignerType.ECDSA;
    }

    /// @dev The account address's own key (EIP-7702 EOA key) is an implicit signer until a recovery revokes it;
    ///      listing the account address as an ECDSA signer re-enables it.
    function _eoaKeyActive() internal view returns (bool) {
        return !AccountStorage.core().eoaKeyRevoked || _isEcdsaSigner(address(this));
    }

    function _isOwnerCaller() internal view returns (bool) {
        if (msg.sender == address(this)) {
            // A self-call is either internal (made by the account's own code while serving another authority) or
            // the EOA key's own transaction (`tx.origin == address(this)`), which needs the key to be active.
            return tx.origin != address(this) || _eoaKeyActive() || _isSelfAuthorized();
        }
        return msg.sender == AccountConstants.ENTRY_POINT || _isEcdsaSigner(msg.sender);
    }

    // ---------------------------------------------------------------------------------------- XIP-0001 lock ----

    /// @dev `address(this)` carries exactly the EIP-7702 designator `0xef0100 || 0x…1005`.
    function _isDelegatedEoa() internal view returns (bool) {
        return address(this).codehash == AccountConstants.EIP7702_DESIGNATOR_HASH;
    }

    function _eoaLockWord() internal view returns (uint256 word) {
        bytes32 slot = AccountConstants.EOA_LOCK_SLOT;
        assembly ("memory-safe") {
            word := sload(slot)
        }
    }

    /// @dev Writes the raw XIP-0001 lock word; emits only on an actual change.
    function _writeEoaLock(uint256 word) internal {
        if (_eoaLockWord() == word) return;
        bytes32 slot = AccountConstants.EOA_LOCK_SLOT;
        assembly ("memory-safe") {
            sstore(slot, word)
        }
        emit EoaLockUpdated(word);
    }

    /// @dev Lock word for a locked account: SEALED while the EOA key cannot act anyway (frozen / revoked), so the
    ///      node refuses every transaction from it (no fee burn); LOCKED otherwise.
    function _lockedWord() internal view returns (uint256) {
        return _isFrozen() || !_eoaKeyActive() ? AccountConstants.EOA_SEALED : AccountConstants.EOA_LOCKED;
    }

    /// @dev Re-derives LOCKED / SEALED after a freeze / signer / recovery change of a 7702 account. A revoked EOA
    ///      key always gets SEALED, even if the lock was off: without the lock it could simply re-delegate. No-op for
    ///      an unlocked account whose EOA key is active, and for non-7702 accounts.
    function _syncEoaLock() internal {
        if ((_eoaLockWord() != 0 || !_eoaKeyActive()) && _isDelegatedEoa()) _writeEoaLock(_lockedWord());
    }

    function _requireOwner() internal view {
        if (!_isOwnerCaller()) revert Unauthorized();
        _requireNotFrozen();
    }

    function _isFrozen() internal view returns (bool) {
        uint64 until = AccountStorage.recovery().frozenUntil;
        return until != 0 && block.timestamp < until;
    }

    function _requireNotFrozen() internal view {
        if (_isFrozen()) revert AccountIsFrozen();
    }

    /// @dev Ends a freeze now (guardian unfreeze or completed recovery); the cooldown counts from here.
    function _endFreeze(AccountStorage.Recovery storage $) internal {
        $.frozenUntil = 0;
        $.freezeEnd = uint64(block.timestamp);
        unchecked {
            ++$.freezeNonce;
        }
        emit AccountUnfrozen();
        _syncEoaLock();
    }

    /// @dev Id of a guardian vote: bound to the guardian set (`guardianEpoch`) and to a kind-specific argument
    ///      (the freeze nonce for freeze / unfreeze, the action id for cancels), so every vote is single-use.
    function _voteId(uint8 kind, uint256 arg) internal view returns (bytes32) {
        return
            keccak256(abi.encode("xdevs.guardian.vote", kind, AccountStorage.recovery().guardianEpoch, arg));
    }

    /// @dev Records `msg.sender`'s (a guardian's) vote; true once the votes reach the guardian threshold.
    function _guardianVote(bytes32 id) internal returns (bool) {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        if (!$.isGuardian[msg.sender]) revert NotGuardian();
        if ($.approvedBy[id][msg.sender]) revert AlreadyApproved();
        $.approvedBy[id][msg.sender] = true;
        uint8 votes = ++$.proposals[id].approvals;
        emit GuardianVoted(id, msg.sender, votes);
        return votes >= $.threshold;
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
