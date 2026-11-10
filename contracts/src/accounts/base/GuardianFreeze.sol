// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {AccountCore} from "./AccountCore.sol";
import {IGuardianFreeze} from "../interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "../libraries/AccountConstants.sol";
import {AccountStorage} from "../libraries/AccountStorage.sol";

/// @title GuardianFreeze
/// @notice Guardian freeze: breaks the "owner vs thief" recovery stalemate. The guardian threshold (the recovery
///         threshold, via on-chain votes) freezes the account — no signer / session key can do anything, signers
///         cannot cancel recoveries, nothing queued executes, ERC-1271 refuses everything — then completes a recovery,
///         which unfreezes it. A freeze always ends by itself after a bounded time (<= MAX_FREEZE_DURATION), and a new
///         one needs an equally long unfrozen gap, so a guardian majority cannot hold the account hostage indefinitely
///         without executing a (public, attributable) recovery.
/// @dev Lives in the account core (not the extension) only for EIP-170 headroom in the factory predeploy.
///      `setFreezeDuration` (a delayed owner change) is in the extension.
abstract contract GuardianFreeze is AccountCore, IGuardianFreeze {
    /// @inheritdoc IGuardianFreeze
    function freeze() external {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        if (_isFrozen()) revert AccountIsFrozen();
        uint64 next = _nextFreezeAllowedAt($);
        if (block.timestamp < next) revert FreezeCooldown(next);
        if (!_guardianVote(_voteId(VOTE_FREEZE, $.freezeNonce))) return;
        uint64 duration = _freezeDuration($);
        uint64 minDuration = $.delay + AccountConstants.FREEZE_RECOVERY_GRACE;
        if (duration < minDuration) duration = minDuration;
        uint64 until = uint64(block.timestamp) + duration;
        $.frozenUntil = until;
        $.freezeStart = uint64(block.timestamp);
        $.freezeEnd = until;
        unchecked {
            ++$.freezeNonce;
        }
        emit AccountFrozen(until);
        // 7702 account: SEALED for the freeze — the EOA key can do nothing in the account, so the node refuses all
        // of its transactions (no bypass with plain transfers / re-delegation, no fee burn).
        if (_isDelegatedEoa()) _writeEoaLock(AccountConstants.EOA_SEALED);
    }

    /// @inheritdoc IGuardianFreeze
    function unfreeze() external {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        if (!_isFrozen()) revert NotFrozen();
        if (!_guardianVote(_voteId(VOTE_UNFREEZE, $.freezeNonce))) return;
        _endFreeze($);
    }

    /// @inheritdoc IGuardianFreeze
    function isFrozen() external view returns (bool) {
        return _isFrozen();
    }

    /// @inheritdoc IGuardianFreeze
    function freezeState()
        external
        view
        returns (uint64 frozenUntil, uint64 freezeDuration, uint64 nextFreezeAllowedAt)
    {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        return ($.frozenUntil, _freezeDuration($), _nextFreezeAllowedAt($));
    }

    /// @inheritdoc IGuardianFreeze
    function guardianVotes(uint8 kind, uint256 actionId) external view returns (bytes32 voteId, uint8 votes) {
        voteId = _voteId(kind, kind == VOTE_CANCEL ? actionId : AccountStorage.recovery().freezeNonce);
        votes = AccountStorage.recovery().proposals[voteId].approvals;
    }

    function _freezeDuration(AccountStorage.Recovery storage $) private view returns (uint64 d) {
        d = $.freezeDuration;
        if (d == 0) d = AccountConstants.DEFAULT_FREEZE_DURATION;
    }

    /// @dev Anti-hostage cooldown: after a freeze the account must stay unfrozen for as long as it was frozen.
    function _nextFreezeAllowedAt(AccountStorage.Recovery storage $) private view returns (uint64) {
        uint64 end = $.freezeEnd;
        return end == 0 ? 0 : end + (end - $.freezeStart);
    }
}
