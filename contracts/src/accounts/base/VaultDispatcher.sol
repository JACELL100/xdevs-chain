// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {AccountCore} from "./AccountCore.sol";
import {IGuardianVault} from "../interfaces/IGuardianVault.sol";
import {AccountConstants} from "../libraries/AccountConstants.sol";
import {AccountStorage} from "../libraries/AccountStorage.sol";
import {CallClassifier} from "../libraries/CallClassifier.sol";
import {Call} from "../libraries/AccountTypes.sol";

/// @title VaultDispatcher
/// @notice Guardian Vault execution path (plan §16.2) + allowance policy (plan §16.5): decides, for every owner or
///         session execution, whether it runs instantly or is queued with a cancel window.
/// @dev With the vault enabled a batch is executed instantly only if *every* call in it is instant; otherwise the
///      whole batch is queued atomically (session executions revert instead of queueing):
///
///      | call                                             | instant when                                          |
///      |--------------------------------------------------|-------------------------------------------------------|
///      | self-call (account management)                   | selector is in the instant list (add signer/session,  |
///      |                                                  | revoke, cancel, ...); everything else is delayed      |
///      | approval-like (approve, increaseAllowance,       | revocation (amount 0 / false) or spender whitelisted. |
///      | setApprovalForAll, Permit2/XDRC-20 approve, 777) | Unlimited + `blockUnlimitedApprovals` => rejected     |
///      | asset transfer out (transfer, transferFrom(self),| recipient whitelisted, or ERC-20 with a configured    |
///      | safeTransferFrom(self), ERC-1155, ERC-777 send)  | per-token limit and within perTx / daily limits       |
///      | any other call with calldata                     | target whitelisted                                    |
///      | native value to a non-whitelisted target         | batch total <= nativeThreshold and within dailyLimit  |
///
///      Queued actions can be cancelled by the owner (any signer) or a guardian until executed, and executed by
///      anyone in `[executeAfter, executeAfter + VAULT_EXECUTION_WINDOW]`. A social recovery bumps the action epoch,
///      invalidating everything queued before it. Policy changes are themselves self-calls and therefore queued.
abstract contract VaultDispatcher is AccountCore, IGuardianVault {
    /// @dev Self-call selectors executed instantly even with the vault enabled.
    function _isInstantSelfSelector(bytes4 selector) internal pure virtual returns (bool);

    // ------------------------------------------------------------------------------------------------------------
    // Queue: cancel / execute
    // ------------------------------------------------------------------------------------------------------------

    /// @inheritdoc IGuardianVault
    function cancel(uint256 id) external {
        _requireOwnerOrGuardian();
        AccountStorage.QueuedAction storage a = AccountStorage.vault().actions[id];
        if (a.status != AccountStorage.ActionStatus.Queued) revert ActionNotQueued(id);
        a.status = AccountStorage.ActionStatus.Cancelled;
        delete a.calls;
        emit VaultActionCancelled(id, msg.sender);
    }

    /// @inheritdoc IGuardianVault
    function execute(uint256 id) external {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        AccountStorage.QueuedAction storage a = $.actions[id];
        if (a.status != AccountStorage.ActionStatus.Queued) revert ActionNotQueued(id);
        if (a.epoch != $.epoch) revert ActionInvalidated(id);
        if (block.timestamp < a.executeAfter) revert ActionNotReady(id, a.executeAfter);
        if (block.timestamp > uint256(a.executeAfter) + AccountConstants.VAULT_EXECUTION_WINDOW) {
            revert ActionExpired(id);
        }
        a.status = AccountStorage.ActionStatus.Executed;
        Call[] memory calls = a.calls;
        delete a.calls;
        emit VaultActionExecuted(id);
        _executeCalls(calls);
    }

    function _queue(Call[] memory calls) internal returns (uint256 id) {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        id = ++$.lastId;
        AccountStorage.QueuedAction storage a = $.actions[id];
        uint64 executeAfter = uint64(block.timestamp) + $.policy.delay;
        a.executeAfter = executeAfter;
        a.epoch = $.epoch;
        a.status = AccountStorage.ActionStatus.Queued;
        uint256 totalValue;
        for (uint256 i; i < calls.length; ++i) {
            a.calls.push(calls[i]);
            totalValue += calls[i].value;
        }
        if (calls.length == 1) {
            emit VaultActionQueued(id, calls[0].target, calls[0].value, calls[0].data, executeAfter);
        } else {
            // A batch is reported as the equivalent `executeBatch((address,uint256,bytes)[])` self-call.
            emit VaultActionQueued(
                id, address(this), totalValue, abi.encodeWithSelector(0x34fcd5be, calls), executeAfter
            );
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Dispatcher
    // ------------------------------------------------------------------------------------------------------------

    /// @dev Executes `calls` instantly or queues them according to the policy. Session executions can never be
    ///      queued: they revert instead. Returns the queued action id (0 when executed instantly).
    function _dispatch(Call[] memory calls, bool isSession) internal returns (uint256) {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        _rollGasBudget($);
        if (!$.policy.enabled) {
            _executeCalls(calls);
            return 0;
        }

        uint256 n = calls.length;
        address[] memory tokens = new address[](n);
        uint256[] memory amounts = new uint256[](n);
        uint256 nTokens;
        uint256 nativeTotal;
        bool needsQueue;

        for (uint256 i; i < n; ++i) {
            Call memory c = calls[i];
            if (c.target == address(this)) {
                if (c.value != 0 || !_isInstantSelfSelector(CallClassifier.selectorOf(c.data))) {
                    needsQueue = true;
                }
                continue;
            }
            bool trustedTarget = AccountStorage.isWhitelisted(c.target);
            if (c.value != 0 && !trustedTarget) nativeTotal += c.value;

            CallClassifier.Info memory info = CallClassifier.classify(address(this), c.target, c.data);
            if (info.kind == CallClassifier.Kind.Approval) {
                if (!info.revoke && !AccountStorage.isWhitelisted(info.counterparty)) {
                    if (info.unlimited && $.policy.blockUnlimitedApprovals) {
                        revert UnlimitedApprovalBlocked(info.counterparty);
                    }
                    needsQueue = true;
                }
            } else if (info.kind == CallClassifier.Kind.AssetTransfer) {
                if (!AccountStorage.isWhitelisted(info.counterparty)) {
                    if (info.fungible && $.tokenLimits[info.token].daily != 0) {
                        nTokens = _accumulate(tokens, amounts, nTokens, info.token, info.amount);
                    } else {
                        needsQueue = true;
                    }
                }
            } else if (info.kind == CallClassifier.Kind.Malformed) {
                needsQueue = true;
            } else if (c.data.length != 0 && !trustedTarget) {
                needsQueue = true;
            }
        }

        if (!needsQueue) needsQueue = !_consumeLimits($, nativeTotal, tokens, amounts, nTokens);

        if (needsQueue) {
            if (isSession) revert SessionActionNeedsDelay();
            return _queue(calls);
        }
        _executeCalls(calls);
        return 0;
    }

    function _accumulate(
        address[] memory tokens,
        uint256[] memory amounts,
        uint256 n,
        address token,
        uint256 amount
    ) private pure returns (uint256) {
        for (uint256 j; j < n; ++j) {
            if (tokens[j] == token) {
                // Saturating add: an overflow can only mean "way above any limit".
                unchecked {
                    uint256 s = amounts[j] + amount;
                    amounts[j] = s < amount ? type(uint256).max : s;
                }
                return n;
            }
        }
        tokens[n] = token;
        amounts[n] = amount;
        return n + 1;
    }

    /// @dev Checks native and per-token limits; if all pass, records the spend and returns true. If any fails,
    ///      nothing is recorded and false is returned (the batch gets queued).
    function _consumeLimits(
        AccountStorage.Vault storage $,
        uint256 nativeTotal,
        address[] memory tokens,
        uint256[] memory amounts,
        uint256 nTokens
    ) private returns (bool) {
        uint64 today = _today();
        uint256 nativeSpent;
        if (nativeTotal != 0) {
            if (nativeTotal > $.policy.nativeThreshold) return false;
            nativeSpent = ($.day == today ? $.spentToday : 0) + nativeTotal;
            if (nativeSpent > $.policy.dailyLimit) return false;
        }
        for (uint256 j; j < nTokens; ++j) {
            AccountStorage.TokenLimitState storage t = $.tokenLimits[tokens[j]];
            if (amounts[j] > t.perTx) return false;
            if ((t.day == today ? t.spent : 0) + amounts[j] > t.daily) return false;
        }
        // All checks passed: record. Values fit in uint128 because they are bounded by uint128 limits.
        if (nativeTotal != 0) {
            $.day = today;
            $.spentToday = uint128(nativeSpent);
        }
        for (uint256 j; j < nTokens; ++j) {
            AccountStorage.TokenLimitState storage t = $.tokenLimits[tokens[j]];
            t.spent = uint128((t.day == today ? t.spent : 0) + amounts[j]);
            t.day = today;
        }
        return true;
    }

    // ------------------------------------------------------------------------------------------------------------
    // ERC-4337 gas budget
    // ------------------------------------------------------------------------------------------------------------

    /// @dev Called from `validateUserOp` (the TIMESTAMP opcode is banned there by ERC-7562): charges the prefund to
    ///      the running daily budget. Returns false when the budget would be exceeded. Bounds the "thief runs the
    ///      bundler and sets an absurd gas price" drain.
    function _chargeGasBudget(uint256 amount) internal returns (bool) {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        if (!$.policy.enabled || amount == 0) return true;
        uint256 spent = uint256($.gasSpent) + amount;
        if (spent > $.policy.dailyGasBudget) return false;
        $.gasSpent = uint128(spent);
        return true;
    }

    /// @notice Resets the daily gas budget once a new UTC day started. Permissionless (it only ever resets the
    ///         counter at a day boundary); also done automatically on every execution.
    function pokeGasBudget() external {
        _rollGasBudget(AccountStorage.vault());
    }

    function _rollGasBudget(AccountStorage.Vault storage $) private {
        uint64 today = _today();
        if ($.gasDay != today) {
            $.gasDay = today;
            $.gasSpent = 0;
        }
    }
}
