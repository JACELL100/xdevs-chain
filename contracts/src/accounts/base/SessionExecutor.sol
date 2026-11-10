// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {AccountCore} from "./AccountCore.sol";
import {AccountStorage} from "../libraries/AccountStorage.sol";
import {CallClassifier} from "../libraries/CallClassifier.sol";
import {Call} from "../libraries/AccountTypes.sol";

/// @title SessionExecutor
/// @notice Session-key enforcement (plan §16.1): allowed (target, selector) pairs, lifetime native cap, per-token
///         caps, validity window, revocation. Session keys are managed by the extension (`addSessionKey`, ...).
/// @dev Session executions additionally go through the Guardian Vault dispatcher in "session" mode: anything the
///      vault would queue reverts instead, and instant native spending counts against the vault's daily limit.
///      Token spending (ERC-20 transfer / transferFrom(self) / non-revoking approvals) is debited from the session's
///      per-token cap; a token without a cap has cap 0. NFT transfers / approvals are debited by token id, so session
///      keys are practically limited to fungible spending.
abstract contract SessionExecutor is AccountCore {
    bytes4 internal constant ANY_SELECTOR = bytes4(0);

    /// @dev Session-mode dispatch through the vault (reverts if the vault would queue).
    function _dispatchSession(Call[] memory calls) internal virtual;

    function _sessionLive(AccountStorage.Sessions storage $, address key) private view returns (bool) {
        AccountStorage.SessionState storage s = $.sessions[key];
        return s.active && s.nonce > $.revokedUpTo;
    }

    function _isPermitted(
        AccountStorage.Sessions storage $,
        address key,
        uint64 nonce,
        address target,
        bytes4 sel
    ) private view returns (bool) {
        return $.permissions[AccountStorage.permissionKey(key, nonce, target, sel)]
            || $.permissions[AccountStorage.permissionKey(key, nonce, target, ANY_SELECTOR)];
    }

    /// @dev Static checks for a session-signed UserOperation (no TIMESTAMP access): key live + every call permitted.
    ///      The validity window is returned for the EntryPoint to enforce; caps are enforced at execution.
    function _checkSessionCalls(address key, Call[] memory calls)
        internal
        view
        returns (bool ok, uint48 validAfter, uint48 validUntil)
    {
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        if (!_sessionLive($, key)) return (false, 0, 0);
        AccountStorage.SessionState storage s = $.sessions[key];
        uint64 nonce = s.nonce;
        for (uint256 i; i < calls.length; ++i) {
            if (!_isPermitted($, key, nonce, calls[i].target, CallClassifier.selectorOf(calls[i].data))) {
                return (false, 0, 0);
            }
        }
        // The EntryPoint treats `validAfter` as exclusive; sessions use an inclusive [validAfter, validUntil] window.
        validAfter = s.validAfter;
        return (true, validAfter == 0 ? 0 : validAfter - 1, s.validUntil);
    }

    /// @dev Full session execution: liveness, time window, permissions, native + token caps, then the vault.
    function _executeSession(address key, Call[] memory calls) internal {
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        AccountStorage.SessionState storage s = $.sessions[key];
        if (!_sessionLive($, key) || block.timestamp < s.validAfter || block.timestamp > s.validUntil) {
            revert SessionNotActive(key);
        }
        uint64 nonce = s.nonce;
        uint256 nativeTotal;
        for (uint256 i; i < calls.length; ++i) {
            Call memory c = calls[i];
            bytes4 sel = CallClassifier.selectorOf(c.data);
            if (c.target == address(this) || !_isPermitted($, key, nonce, c.target, sel)) {
                revert SessionCallNotAllowed(c.target, sel);
            }
            nativeTotal += c.value;
            CallClassifier.Info memory info = CallClassifier.classify(address(this), c.target, c.data);
            if (
                info.kind == CallClassifier.Kind.AssetTransfer
                    || (info.kind == CallClassifier.Kind.Approval && !info.revoke)
                    || info.kind == CallClassifier.Kind.Malformed
            ) {
                AccountStorage.TokenCapState storage t =
                    $.tokenCaps[AccountStorage.tokenCapKey(key, nonce, info.token)];
                if (info.amount > t.cap - t.spent) revert SessionTokenCapExceeded(info.token);
                t.spent += uint128(info.amount);
            }
        }
        if (nativeTotal > s.nativeCap - s.nativeSpent) revert SessionNativeCapExceeded();
        s.nativeSpent += uint128(nativeTotal);
        _dispatchSession(calls);
    }
}
