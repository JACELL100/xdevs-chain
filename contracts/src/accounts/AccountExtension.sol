// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {P256} from "@openzeppelin/contracts/utils/cryptography/P256.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";

import {Predeploys} from "../common/Predeploys.sol";
import {AccountCore} from "./base/AccountCore.sol";
import {IAccountExtension} from "./interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "./libraries/AccountConstants.sol";
import {AccountStorage} from "./libraries/AccountStorage.sol";
import {
    Call,
    InitParams,
    Signer,
    SignerType,
    VaultPolicy,
    SessionKeyConfig
} from "./libraries/AccountTypes.sol";

// slither-disable-next-line missing-inheritance (SA-I06: helper interface for the self-call only)
interface IAccountExecuteSelf {
    function execute(address target, uint256 value, bytes calldata data) external returns (uint256);
}

/// @title AccountExtension
/// @notice Cold-path logic of the xDEVS smart account: initialization, signer / guardian / vault-policy /
///         session-key management, social recovery, views and proxy upgrades.
/// @dev Executed ONLY through DELEGATECALL from `XdevsSmartAccount.fallback()` (msg.sender, storage and transient
///      storage are the account's). It is hosted by the `SmartAccountFactory` predeploy. Called directly on the
///      factory, every state-changing function fails its authorization (the factory has no signers / guardians and
///      its code is non-empty), so the factory's own (unused) storage cannot be meaningfully modified.
///
///      Delay semantics (vault enabled): `removeSigner`, `setGuardians`, `setRecoveryDelay`, `setFreezeDuration`,
///      `setPolicy`, `setVaultPolicy`, `setWhitelisted`, `setTokenLimit`, `approveHash`, `disableEoaLock` and
///      `upgradeToAndCall` are `delayedSelf`: an owner calling them gets a queued vault action instead. Adding keys /
///      sessions, enabling the EOA lock and every revocation / cancellation stay instant. While the account is frozen
///      no owner function works at all (`_requireOwner`).
abstract contract AccountExtension is AccountCore, IAccountExtension {
    bytes32 private constant INIT_TYPEHASH =
        keccak256("XdevsAccountInit(uint256 chainId,address account,bytes32 params)");

    /// @dev Delayed self-calls are routed through the core's `execute(self, 0, data)` (owner authority = self), so the
    ///      vault dispatcher queues them (enabled) or runs them instantly (disabled).
    function _routeSelfCall(bytes memory data) internal override {
        // slither-disable-next-line reentrancy-no-eth,reentrancy-benign,reentrancy-events,unused-return (SA-F05, SA-F13)
        IAccountExecuteSelf(address(this)).execute(address(this), 0, data);
    }

    // ============================================================================================================
    // Initialization
    // ============================================================================================================

    /// @inheritdoc IAccountExtension
    /// @dev Allowed (a) inside a proxy constructor (`address(this).code.length == 0`: the factory deployment, whose
    ///      CREATE2 address commits to these params) or (b) as an EIP-7702 self-call (`msg.sender == address(this)`).
    function initialize(InitParams calldata params) external {
        if (address(this).code.length != 0 && msg.sender != address(this)) revert Unauthorized();
        _initialize(params);
    }

    /// @inheritdoc IAccountExtension
    /// @dev EIP-7702 initialization relayed by anyone (e.g. an ERC-4337 `initCode = 0x7702…` op or a sponsor),
    ///      authorized by an EIP-191 signature of the EOA key over `initHash(params)`.
    function initializeWithSignature(InitParams calldata params, bytes calldata eoaSignature) external {
        bytes32 digest = MessageHashUtils.toEthSignedMessageHash(initHash(params));
        if (ECDSA.recoverCalldata(digest, eoaSignature) != address(this)) revert Unauthorized();
        _initialize(params);
    }

    // aderyn-fp-next-line(unprotected-initializer) SA-A05: view helper, not an initializer
    function initHash(InitParams calldata params) public view returns (bytes32) {
        return
            keccak256(abi.encode(INIT_TYPEHASH, block.chainid, address(this), keccak256(abi.encode(params))));
    }

    function isInitialized() external view returns (bool) {
        return AccountStorage.core().initialized;
    }

    function _initialize(InitParams calldata p) private {
        if (
            address(this) == Predeploys.SMART_ACCOUNT_IMPL
                || address(this) == Predeploys.SMART_ACCOUNT_FACTORY
        ) {
            revert Unauthorized();
        }
        AccountStorage.Core storage $ = AccountStorage.core();
        if ($.initialized) revert AlreadyInitialized();
        $.initialized = true;
        // The EOA lock can only be enabled after initialization: a non-zero value here is garbage left by a previous
        // EIP-7702 delegate, which must not lock the EOA by surprise.
        _writeEoaLock(0);

        _replaceSigners(p.signers);
        _setGuardians(p.guardians, p.guardianThreshold);
        _setRecoveryDelay(p.recoveryDelay);
        _setPolicy(p.policy);
        for (uint256 i; i < p.whitelist.length; ++i) {
            _setWhitelisted(p.whitelist[i], true);
        }
        for (uint256 i; i < p.tokenLimits.length; ++i) {
            _setTokenLimit(p.tokenLimits[i].token, p.tokenLimits[i].perTx, p.tokenLimits[i].daily);
        }
        emit AccountInitialized(p.signers.length, p.guardians.length, p.policy.enabled);
    }

    // ============================================================================================================
    // Signers
    // ============================================================================================================

    /// @notice Identifier of a signer: the left-padded address for secp256k1, keccak256(x, y) for passkeys.
    function signerId(Signer memory s) public pure returns (bytes32) {
        if (s.kind == SignerType.ECDSA) return bytes32(uint256(uint160(uint256(s.x))));
        return keccak256(abi.encode(s.x, s.y));
    }

    function getSigners() external view returns (Signer[] memory out) {
        AccountStorage.Signers storage $ = AccountStorage.signers();
        uint256 n = $.ids.length;
        out = new Signer[](n);
        for (uint256 i; i < n; ++i) {
            out[i] = $.signers[$.ids[i]];
        }
    }

    function isSigner(bytes32 id) external view returns (bool) {
        return AccountStorage.signers().signers[id].kind != SignerType.None;
    }

    /// @notice Adds a signer (e.g. a new device). Instant: an extra key does not increase a thief's power, and a
    ///         social recovery replaces the whole signer set.
    ///         Listing the account address itself re-activates a revoked EIP-7702 EOA key (lock word SEALED -> LOCKED).
    function addSigner(Signer calldata s) external onlyOwner {
        _addSigner(s);
        _syncEoaLock();
    }

    /// @notice Removes a signer. Delayed by the vault (a thief must not be able to lock the owner out instantly).
    function removeSigner(bytes32 id) external delayedSelf {
        AccountStorage.Signers storage $ = AccountStorage.signers();
        uint256 idx = $.indexPlusOne[id];
        if (idx == 0) revert SignerNotFound(id);
        if ($.ids.length == 1) revert LastSigner();
        bytes32 last = $.ids[$.ids.length - 1];
        $.ids[idx - 1] = last;
        $.indexPlusOne[last] = idx;
        $.ids.pop();
        delete $.indexPlusOne[id];
        delete $.signers[id];
        emit SignerRemoved(id);
        _syncEoaLock();
    }

    function _addSigner(Signer memory s) private {
        if (s.kind == SignerType.ECDSA) {
            if (uint256(s.x) >> 160 != 0 || s.x == bytes32(0) || s.y != bytes32(0)) revert InvalidSigner();
        } else if (s.kind == SignerType.Passkey) {
            if (!P256.isValidPublicKey(s.x, s.y)) revert InvalidSigner();
        } else {
            revert InvalidSigner();
        }
        AccountStorage.Signers storage $ = AccountStorage.signers();
        bytes32 id = signerId(s);
        if ($.indexPlusOne[id] != 0) revert SignerAlreadyExists(id);
        if ($.ids.length >= AccountConstants.MAX_SIGNERS) revert TooManySigners();
        $.signers[id] = s;
        $.ids.push(id);
        $.indexPlusOne[id] = $.ids.length;
        emit SignerAdded(id, s.kind, s.x, s.y);
    }

    /// @dev Replaces the whole signer set (initialization and social recovery).
    function _replaceSigners(Signer[] memory newSigners) private {
        if (newSigners.length == 0) revert InvalidSigner();
        AccountStorage.Signers storage $ = AccountStorage.signers();
        for (uint256 i = $.ids.length; i > 0; --i) {
            bytes32 id = $.ids[i - 1];
            $.ids.pop();
            delete $.indexPlusOne[id];
            delete $.signers[id];
            emit SignerRemoved(id);
        }
        for (uint256 i; i < newSigners.length; ++i) {
            _addSigner(newSigners[i]);
        }
    }

    // ============================================================================================================
    // Guardian Vault policy
    // ============================================================================================================

    /// @notice IGuardianVault.setPolicy: sets threshold + delay, replaces the whitelist and enables the vault.
    ///         Delayed when the vault is already enabled (served through the core's `setPolicy` forwarder).
    function setPolicy(uint256 nativeThreshold, uint64 delaySeconds, address[] calldata newWhitelist)
        external
        delayedSelf
    {
        if (nativeThreshold > type(uint128).max) revert InvalidParameter();
        VaultPolicy memory p = AccountStorage.vault().policy;
        p.enabled = true;
        p.nativeThreshold = uint128(nativeThreshold);
        p.delay = delaySeconds;
        _setPolicy(p);
        AccountStorage.Vault storage $ = AccountStorage.vault();
        for (uint256 i = $.whitelist.length; i > 0; --i) {
            _setWhitelisted($.whitelist[i - 1], false);
        }
        for (uint256 i; i < newWhitelist.length; ++i) {
            _setWhitelisted(newWhitelist[i], true);
        }
    }

    /// @notice Sets every policy field. Delayed when the vault is enabled (so disabling it is delayed too).
    function setVaultPolicy(VaultPolicy calldata p) external delayedSelf {
        _setPolicy(p);
    }

    function setWhitelisted(address who, bool trusted) external delayedSelf {
        _setWhitelisted(who, trusted);
    }

    /// @notice Per-token instant limits (token units, no oracle). `daily == 0` removes the limit (transfers are then
    ///         queued unless the recipient is whitelisted).
    function setTokenLimit(address token, uint128 perTx, uint128 daily) external delayedSelf {
        _setTokenLimit(token, perTx, daily);
    }

    /// @notice Pre-approves a hash for ERC-1271 (required when `allowInstantSignatures` is false).
    function approveHash(bytes32 hash) external delayedSelf {
        AccountStorage.vault().approvedHashes[hash] = true;
        emit HashApproved(hash, true);
    }

    /// @notice Revokes a pre-approved hash. Instant (tightening).
    function revokeHash(bytes32 hash) external onlyOwner {
        AccountStorage.vault().approvedHashes[hash] = false;
        emit HashApproved(hash, false);
    }

    function vaultPolicy() external view returns (VaultPolicy memory) {
        return AccountStorage.vault().policy;
    }

    function isWhitelisted(address who) external view returns (bool) {
        return AccountStorage.isWhitelisted(who);
    }

    function getWhitelist() external view returns (address[] memory) {
        return AccountStorage.vault().whitelist;
    }

    function tokenLimit(address token)
        external
        view
        returns (uint128 perTx, uint128 daily, uint64 day, uint128 spent)
    {
        AccountStorage.TokenLimitState storage t = AccountStorage.vault().tokenLimits[token];
        return (t.perTx, t.daily, t.day, t.spent);
    }

    function getAction(uint256 id)
        external
        view
        returns (uint64 executeAfter, uint8 status, bool valid, Call[] memory calls)
    {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        AccountStorage.QueuedAction storage a = $.actions[id];
        return (a.executeAfter, uint8(a.status), a.epoch == $.epoch, a.calls);
    }

    function lastActionId() external view returns (uint256) {
        return AccountStorage.vault().lastId;
    }

    /// @notice Native value already spent instantly today (UTC day).
    function spentToday() external view returns (uint128) {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        // slither-disable-next-line incorrect-equality (SA-F09: UTC day index, not a balance)
        return $.day == _today() ? $.spentToday : 0;
    }

    function isHashApproved(bytes32 hash) external view returns (bool) {
        return AccountStorage.vault().approvedHashes[hash];
    }

    function gasBudgetSpent() external view returns (uint128) {
        return AccountStorage.vault().gasSpent;
    }

    function _setPolicy(VaultPolicy memory p) private {
        if (p.delay == 0) p.delay = AccountConstants.DEFAULT_VAULT_DELAY;
        if (p.delay < AccountConstants.MIN_VAULT_DELAY || p.delay > AccountConstants.MAX_VAULT_DELAY) {
            revert InvalidParameter();
        }
        if (p.dailyGasBudget == 0) p.dailyGasBudget = AccountConstants.DEFAULT_DAILY_GAS_BUDGET;
        AccountStorage.Vault storage $ = AccountStorage.vault();
        // Start the gas-budget day now, so the first execution does not wipe prefunds charged during validation.
        if ($.gasDay == 0) $.gasDay = _today();
        $.policy = p;
        emit VaultPolicyUpdated(p);
    }

    function _setWhitelisted(address who, bool trusted) private {
        if (who == address(0) || who == address(this)) revert InvalidParameter();
        AccountStorage.Vault storage $ = AccountStorage.vault();
        uint256 idx = $.whitelistIndexPlusOne[who];
        if (trusted && idx == 0) {
            if ($.whitelist.length >= AccountConstants.MAX_WHITELIST) revert TooManyWhitelisted();
            $.whitelist.push(who);
            $.whitelistIndexPlusOne[who] = $.whitelist.length;
        } else if (!trusted && idx != 0) {
            address last = $.whitelist[$.whitelist.length - 1];
            $.whitelist[idx - 1] = last;
            $.whitelistIndexPlusOne[last] = idx;
            $.whitelist.pop();
            delete $.whitelistIndexPlusOne[who];
        } else {
            return;
        }
        emit WhitelistUpdated(who, trusted);
    }

    function _setTokenLimit(address token, uint128 perTx, uint128 daily) private {
        if (token == address(0)) revert InvalidParameter();
        AccountStorage.TokenLimitState storage t = AccountStorage.vault().tokenLimits[token];
        t.perTx = perTx;
        t.daily = daily;
        emit TokenLimitUpdated(token, perTx, daily);
    }

    // ============================================================================================================
    // Social recovery (plan §16.1): N-of-M guardians rotate the signer set after a delay; owner can cancel
    // ============================================================================================================

    /// @notice Proposes / approves rotating the signer set to `newSigners`. Guardians only. When the approvals for
    ///         the exact same `newSigners` reach the threshold, the delay starts: `readyAt = now + recoveryDelay`.
    function approveRecovery(Signer[] calldata newSigners) external {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        if (!$.isGuardian[msg.sender]) revert NotGuardian();
        if (newSigners.length == 0 || newSigners.length > AccountConstants.MAX_SIGNERS) {
            revert InvalidParameter();
        }
        bytes32 id = _recoveryId($, newSigners);
        if ($.approvedBy[id][msg.sender]) revert AlreadyApproved();
        $.approvedBy[id][msg.sender] = true;
        AccountStorage.Proposal storage p = $.proposals[id];
        uint8 approvals = p.approvals + 1;
        p.approvals = approvals;
        emit RecoveryApproved(id, msg.sender, approvals, newSigners);
        if (approvals == $.threshold) {
            uint64 readyAt = uint64(block.timestamp) + $.delay;
            p.readyAt = readyAt;
            emit RecoveryReady(id, readyAt);
        }
    }

    /// @notice Executes a recovery whose threshold was reached and whose delay elapsed. Callable by anyone.
    ///         Replaces the signer set, revokes all session keys and invalidates every queued vault action.
    ///         Also revokes the account address's own key (the EOA key of an EIP-7702 account) unless `newSigners`
    ///         lists it explicitly — sealing a 7702 account at the protocol level (XIP-0001) — and ends a guardian
    ///         freeze (resetting the freeze cooldown).
    function executeRecovery(Signer[] calldata newSigners) external {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        bytes32 id = _recoveryId($, newSigners);
        AccountStorage.Proposal storage p = $.proposals[id];
        if ($.threshold == 0 || p.approvals < $.threshold || p.readyAt == 0 || block.timestamp < p.readyAt) {
            revert RecoveryNotReady();
        }
        unchecked {
            ++$.nonce;
        }
        emit RecoveryExecuted(id);
        _replaceSigners(newSigners);
        AccountStorage.core().eoaKeyRevoked = true;
        emit EoaKeyRevoked();
        if ($.frozenUntil != 0) _endFreeze($);
        $.freezeStart = 0;
        $.freezeEnd = 0;
        // 7702 account whose EOA key was revoked => SEALED (even if it was not locked). A kept key (listed in
        // `newSigners`) keeps the previous lock state.
        _syncEoaLock();
        _revokeAllSessions();
        AccountStorage.Vault storage v = AccountStorage.vault();
        unchecked {
            ++v.epoch;
        }
        emit VaultEpochBumped(v.epoch);
    }

    /// @notice Owner cancels every pending recovery proposal (nonce bump). Impossible while frozen.
    function cancelRecovery() external onlyOwner {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        unchecked {
            ++$.nonce;
        }
        emit RecoveryCancelled($.nonce);
    }

    function setGuardians(address[] calldata guardians, uint8 threshold) external delayedSelf {
        _setGuardians(guardians, threshold);
    }

    function setRecoveryDelay(uint64 delay) external delayedSelf {
        _setRecoveryDelay(delay);
    }

    function getGuardians() external view returns (address[] memory guardians, uint8 threshold) {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        return ($.guardians, $.threshold);
    }

    function isGuardian(address who) external view returns (bool) {
        return AccountStorage.isGuardian(who);
    }

    function recoveryDelay() external view returns (uint64) {
        return AccountStorage.recovery().delay;
    }

    function recoveryNonce() external view returns (uint64) {
        return AccountStorage.recovery().nonce;
    }

    function getRecoveryProposal(Signer[] calldata newSigners)
        external
        view
        returns (bytes32 id, uint8 approvals, uint64 readyAt)
    {
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        id = _recoveryId($, newSigners);
        AccountStorage.Proposal storage p = $.proposals[id];
        return (id, p.approvals, p.readyAt);
    }

    function _recoveryId(AccountStorage.Recovery storage $, Signer[] calldata newSigners)
        private
        view
        returns (bytes32)
    {
        return keccak256(abi.encode($.nonce, newSigners));
    }

    function _setGuardians(address[] calldata guardians, uint8 threshold) private {
        if (guardians.length > AccountConstants.MAX_GUARDIANS) revert InvalidGuardians();
        if (guardians.length == 0 ? threshold != 0 : (threshold == 0 || threshold > guardians.length)) {
            revert InvalidGuardians();
        }
        AccountStorage.Recovery storage $ = AccountStorage.recovery();
        for (uint256 i; i < $.guardians.length; ++i) {
            $.isGuardian[$.guardians[i]] = false;
        }
        delete $.guardians;
        for (uint256 i; i < guardians.length; ++i) {
            address g = guardians[i];
            if (g == address(0) || g == address(this) || $.isGuardian[g]) revert InvalidGuardians();
            $.isGuardian[g] = true;
            $.guardians.push(g);
        }
        $.threshold = threshold;
        unchecked {
            ++$.nonce; // invalidate approvals collected under the previous guardian set
            ++$.guardianEpoch; // ... and freeze / unfreeze / cancel votes
        }
        emit GuardiansUpdated(guardians, threshold);
    }

    function _setRecoveryDelay(uint64 delay) private {
        if (delay == 0) delay = AccountConstants.DEFAULT_RECOVERY_DELAY;
        if (delay < AccountConstants.MIN_RECOVERY_DELAY || delay > AccountConstants.MAX_RECOVERY_DELAY) {
            revert InvalidParameter();
        }
        AccountStorage.recovery().delay = delay;
        emit RecoveryDelayUpdated(delay);
    }

    // ============================================================================================================
    // Guardian freeze configuration (freeze / unfreeze themselves are in the core: base/GuardianFreeze.sol)
    // ============================================================================================================

    /// @notice Sets the freeze duration (0 = 30 days; bounds [3 d, 60 d]). Delayed by the vault. A freeze always
    ///         lasts at least `recoveryDelay + 3 days`.
    function setFreezeDuration(uint64 duration) external delayedSelf {
        if (duration == 0) duration = AccountConstants.DEFAULT_FREEZE_DURATION;
        if (
            duration < AccountConstants.MIN_FREEZE_DURATION || duration > AccountConstants.MAX_FREEZE_DURATION
        ) {
            revert InvalidParameter();
        }
        AccountStorage.recovery().freezeDuration = duration;
        emit FreezeDurationUpdated(duration);
    }

    // ============================================================================================================
    // XIP-0001 EOA lock (EIP-7702 accounts only; enforced by the node, see docs/specs/xip-0001-eoa-lock.md)
    // ============================================================================================================

    /// @notice Locks the EOA key at the protocol level (XIP-0001): once set, the node only accepts transactions from
    ///         this address that are value-0 calls to itself, i.e. that enter this account's code, where the Guardian
    ///         Vault applies to the EOA key like to any signer (no plain transfers, no direct calls, no
    ///         re-delegation). The account itself upgrades the word to SEALED (no transaction at all) while frozen or
    ///         after a recovery revoked the EOA key. Instant; owner only; requires an EIP-7702 delegation to the
    ///         implementation (meaningless for factory accounts, which have no private key) and an enabled vault.
    function enableEoaLock() external onlyOwner {
        if (!_isDelegatedEoa()) revert NotDelegatedEoa();
        if (!AccountStorage.vault().policy.enabled) revert VaultDisabled();
        _writeEoaLock(_lockedWord());
    }

    /// @notice Lifts the EOA lock. Delayed by the vault (a thief holding the EOA key cannot lift it instantly).
    ///         Refused while the EOA key is revoked: the lock is what prevents a revoked key from re-delegating.
    function disableEoaLock() external delayedSelf {
        if (!_eoaKeyActive()) revert EoaKeyInactive();
        _writeEoaLock(0);
    }

    /// @notice Permissionless: re-derives LOCKED / SEALED (e.g. back to LOCKED after a freeze expired by itself).
    function refreshEoaLock() external {
        _syncEoaLock();
    }

    /// @notice True iff the node applies XIP-0001 to this account (delegated to the implementation + word != 0).
    function isEoaLocked() external view returns (bool) {
        return _isDelegatedEoa() && _eoaLockWord() != 0;
    }

    /// @notice Raw XIP-0001 lock word: 0 unlocked, 1 locked, 2 sealed.
    function eoaLockState() external view returns (uint256) {
        return _eoaLockWord();
    }

    /// @notice Whether the account address's own key (EIP-7702 EOA key) still has owner authority.
    function isEoaKeyActive() external view returns (bool) {
        return _eoaKeyActive();
    }

    // ============================================================================================================
    // Session keys (management; enforcement is in the core's SessionExecutor)
    // ============================================================================================================

    /// @notice Registers (or replaces) a session key. Instant: a session key can never do anything the vault would
    ///         queue, and its instant spending counts against the vault's daily limit.
    function addSessionKey(SessionKeyConfig calldata cfg) external onlyOwner {
        if (
            cfg.key == address(0) || cfg.key == address(this) || cfg.validUntil <= cfg.validAfter
                || cfg.permissions.length == 0
                || cfg.permissions.length > AccountConstants.MAX_SESSION_PERMISSIONS
                || cfg.tokenCaps.length > AccountConstants.MAX_SESSION_PERMISSIONS
        ) revert InvalidSession();
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        uint64 nonce = ++$.lastNonce;
        $.sessions[cfg.key] = AccountStorage.SessionState({
            nonce: nonce,
            validAfter: cfg.validAfter,
            validUntil: cfg.validUntil,
            active: true,
            nativeCap: cfg.nativeCap,
            nativeSpent: 0
        });
        for (uint256 i; i < cfg.permissions.length; ++i) {
            address target = cfg.permissions[i].target;
            if (target == address(0) || target == address(this)) revert InvalidSession();
            $.permissions[AccountStorage.permissionKey(cfg.key, nonce, target, cfg.permissions[i].selector)] =
                true;
        }
        for (uint256 i; i < cfg.tokenCaps.length; ++i) {
            $.tokenCaps[AccountStorage.tokenCapKey(cfg.key, nonce, cfg.tokenCaps[i].token)].cap =
            cfg.tokenCaps[i].cap;
        }
        emit SessionKeyAdded(cfg.key, nonce, cfg.validAfter, cfg.validUntil, cfg.nativeCap);
    }

    function revokeSessionKey(address key) external onlyOwner {
        AccountStorage.sessions().sessions[key].active = false;
        emit SessionKeyRevoked(key);
    }

    function revokeAllSessionKeys() external onlyOwner {
        _revokeAllSessions();
    }

    function _revokeAllSessions() private {
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        $.revokedUpTo = $.lastNonce;
        emit AllSessionKeysRevoked($.lastNonce);
    }

    function getSession(address key)
        external
        view
        returns (
            uint64 nonce,
            uint48 validAfter,
            uint48 validUntil,
            bool active,
            uint128 nativeCap,
            uint128 nativeSpent
        )
    {
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        AccountStorage.SessionState storage s = $.sessions[key];
        return (
            s.nonce,
            s.validAfter,
            s.validUntil,
            s.active && s.nonce > $.revokedUpTo,
            s.nativeCap,
            s.nativeSpent
        );
    }

    function isSessionPermitted(address key, address target, bytes4 selector) external view returns (bool) {
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        uint64 nonce = $.sessions[key].nonce;
        return $.permissions[AccountStorage.permissionKey(key, nonce, target, selector)]
            || $.permissions[AccountStorage.permissionKey(key, nonce, target, bytes4(0))];
    }

    function sessionTokenCap(address key, address token) external view returns (uint128 cap, uint128 spent) {
        AccountStorage.Sessions storage $ = AccountStorage.sessions();
        AccountStorage.TokenCapState storage t =
            $.tokenCaps[AccountStorage.tokenCapKey(key, $.sessions[key].nonce, token)];
        return (t.cap, t.spent);
    }

    // ============================================================================================================
    // Upgrades (proxy accounts only; delayed by the vault)
    // ============================================================================================================

    /// @notice Upgrades a factory (ERC-1967 proxy) account. Not available to EIP-7702 accounts (they re-delegate).
    /// @dev The delegation check matters: a 7702 EOA may carry a non-zero ERC-1967 slot left by a previous delegate,
    ///      and `upgradeToAndCall`'s DELEGATECALL could then rewrite any account storage (e.g. the XIP-0001 lock word
    ///      that `disableEoaLock` refuses to clear while the EOA key is revoked). Static-analysis finding SA-01.
    function upgradeToAndCall(address newImplementation, bytes calldata data) external delayedSelf {
        if (_isDelegatedEoa() || ERC1967Utils.getImplementation() == address(0)) revert NotProxy();
        if (newImplementation.code.length == 0) revert InvalidImplementation();
        ERC1967Utils.upgradeToAndCall(newImplementation, data);
    }
}
