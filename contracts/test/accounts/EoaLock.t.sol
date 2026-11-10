// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {IGuardianVault} from "../../src/accounts/interfaces/IGuardianVault.sol";
import {IAccountEvents, IGuardianFreeze} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "../../src/accounts/libraries/AccountConstants.sol";
import {Call, InitParams, Signer, VaultPolicy} from "../../src/accounts/libraries/AccountTypes.sol";

/// @notice XIP-0001 "EOA lock" — account side, plus a reference model of the node's validity rule
///         (docs/specs/xip-0001-eoa-lock.md) evaluated against real account state.
contract EoaLockTest is AccountTestBase {
    address internal eoa;
    uint256 internal eoaPk;
    IXAccount internal acct;
    address internal friend = makeAddr("friend");

    function setUp() public override {
        super.setUp();
        (eoa, eoaPk) = makeAddrAndKey("metamask-user");
        vm.deal(eoa, 100 ether);
        vm.signAndAttachDelegation(Predeploys.SMART_ACCOUNT_IMPL, eoaPk);
        acct = IXAccount(eoa);
        // The user's second device (`owner`) is a registered signer; the EOA key is the implicit signer.
        vm.prank(eoa, eoa);
        acct.initialize(_params(_one(_ecdsaSigner(owner)), true));
    }

    // ============================================================================================================
    // Reference model of the XIP-0001 node rule (what crates/ must implement)
    // ============================================================================================================

    uint64 internal constant LOCKED_MAX_GAS = 2_000_000;

    struct NodeTx {
        address sender;
        address to; // address(0) with isCreate = true for contract creation
        bool isCreate;
        uint256 value;
        uint64 gasLimit;
        uint256 maxFeePerGas;
        uint256 baseFee;
        bool earlierTxFromSenderInBlock;
    }

    /// @dev §3 "locked": code is exactly the 23-byte designator 0xef0100 || 0x…1005 and the raw slot is non-zero.
    function _nodeIsLocked(address a) internal view returns (bool) {
        return keccak256(a.code) == keccak256(abi.encodePacked(hex"ef0100", Predeploys.SMART_ACCOUNT_IMPL))
            && vm.load(a, AccountConstants.EOA_LOCK_SLOT) != bytes32(0);
    }

    /// @dev §3 "sealed": locked and the word is exactly 2.
    function _nodeIsSealed(address a) internal view returns (bool) {
        return _nodeIsLocked(a) && uint256(vm.load(a, AccountConstants.EOA_LOCK_SLOT)) == 2;
    }

    /// @dev Rules (a) + (c), evaluated against the state at the start of the transaction.
    function _nodeTxValid(NodeTx memory t) internal view returns (bool) {
        if (!_nodeIsLocked(t.sender)) return true;
        if (_nodeIsSealed(t.sender)) return false; // (a) sealed: nothing at all
        if (t.isCreate || t.to != t.sender || t.value != 0) return false; // (a)
        if (t.earlierTxFromSenderInBlock) return false; // (c1)
        if (t.gasLimit > LOCKED_MAX_GAS) return false; // (c2)
        if (t.maxFeePerGas > 2 * t.baseFee) return false; // (c3)
        return true;
    }

    /// @dev Rule (b): an EIP-7702 authorization whose authority is locked is skipped.
    function _nodeAuthApplies(address authority) internal view returns (bool) {
        return !_nodeIsLocked(authority);
    }

    function _selfTx() internal view returns (NodeTx memory) {
        return NodeTx(eoa, eoa, false, 0, 300_000, 2 gwei, 1 gwei, false);
    }

    function _lock() internal {
        vm.prank(eoa, eoa);
        acct.enableEoaLock();
    }

    // ============================================================================================================
    // Constants
    // ============================================================================================================

    function test_constants_matchSpec() public view {
        assertEq(
            AccountConstants.EOA_LOCK_SLOT,
            bytes32(uint256(keccak256("xdevs.account.eoaLock")) - 1),
            "lock slot"
        );
        assertEq(
            AccountConstants.EOA_LOCK_SLOT, 0xfd81d7eef3094fecca95e5e53fe72709c7315b39f65e414992ead9c855e841ce
        );
        assertEq(
            AccountConstants.EIP7702_DESIGNATOR_HASH,
            keccak256(abi.encodePacked(hex"ef0100", Predeploys.SMART_ACCOUNT_IMPL))
        );
        // EXTCODEHASH of a delegated EOA is the hash of its 23-byte designator (what the account checks)
        assertEq(eoa.codehash, AccountConstants.EIP7702_DESIGNATOR_HASH);
    }

    // ============================================================================================================
    // enable / disable
    // ============================================================================================================

    function test_enableEoaLock_isInstant_writesRawSlot() public {
        assertFalse(acct.isEoaLocked());
        assertFalse(_nodeIsLocked(eoa));
        vm.expectEmit(eoa);
        emit IAccountEvents.EoaLockUpdated(1);
        _lock();
        assertEq(vm.load(eoa, AccountConstants.EOA_LOCK_SLOT), bytes32(uint256(1)));
        assertTrue(acct.isEoaLocked());
        assertTrue(_nodeIsLocked(eoa));
        _lock(); // idempotent
        assertEq(vm.load(eoa, AccountConstants.EOA_LOCK_SLOT), bytes32(uint256(1)));
    }

    function test_enableEoaLock_bySigner_andAsInstantSelfCall() public {
        vm.prank(owner); // registered signer
        acct.enableEoaLock();
        assertTrue(acct.isEoaLocked());

        // Through the vault dispatcher it is on the instant list (a tightening action).
        vm.store(eoa, AccountConstants.EOA_LOCK_SLOT, bytes32(0));
        vm.prank(eoa, eoa);
        uint256 id = acct.execute(eoa, 0, abi.encodeCall(acct.enableEoaLock, ()));
        assertEq(id, 0);
        assertTrue(acct.isEoaLocked());
    }

    function test_enableEoaLock_requirements() public {
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.enableEoaLock();

        // factory accounts have no private key: the lock is meaningless there and refused
        IXAccount factoryAcct = _vaultAccount();
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.NotDelegatedEoa.selector);
        factoryAcct.enableEoaLock();

        // without the vault the lock would protect nothing (everything is instant through the account)
        (address eoa2, uint256 eoa2Pk) = makeAddrAndKey("eoa2");
        vm.signAndAttachDelegation(Predeploys.SMART_ACCOUNT_IMPL, eoa2Pk);
        vm.prank(eoa2, eoa2);
        IXAccount(eoa2).initialize(_params(_one(_ecdsaSigner(owner)), false));
        vm.prank(eoa2, eoa2);
        vm.expectRevert(IAccountEvents.VaultDisabled.selector);
        IXAccount(eoa2).enableEoaLock();
    }

    function test_disableEoaLock_isDelayed_andGuardianCancellable() public {
        _lock();
        // the EOA key (thief or owner) asks to lift the lock: queued, still locked
        vm.prank(eoa, eoa);
        acct.disableEoaLock();
        uint256 id = acct.lastActionId();
        assertGt(id, 0);
        assertTrue(acct.isEoaLocked());
        // guardians (threshold) cancel it
        vm.prank(guardian1);
        acct.cancel(id);
        vm.prank(guardian2);
        acct.cancel(id);
        vm.warp(block.timestamp + 2 days);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, id));
        acct.execute(id);
        assertTrue(acct.isEoaLocked());

        // an uncontested request matures after the vault delay
        vm.prank(owner);
        acct.disableEoaLock();
        id = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours - 1);
        vm.expectRevert();
        acct.execute(id);
        vm.warp(block.timestamp + 1);
        acct.execute(id);
        assertFalse(acct.isEoaLocked());
        assertEq(vm.load(eoa, AccountConstants.EOA_LOCK_SLOT), bytes32(0));
    }

    function test_initialize_clearsGarbageLockSlot() public {
        (address eoa2, uint256 eoa2Pk) = makeAddrAndKey("eoa-with-garbage");
        vm.store(eoa2, AccountConstants.EOA_LOCK_SLOT, bytes32(uint256(0xdead))); // left by a previous delegate
        vm.signAndAttachDelegation(Predeploys.SMART_ACCOUNT_IMPL, eoa2Pk);
        assertTrue(IXAccount(eoa2).isEoaLocked());
        assertTrue(_nodeIsLocked(eoa2));
        // garbage (!= 2) counts as LOCKED, not SEALED: the node still lets the EOA call itself, so it can initialize
        // (or disable the lock: vault off => instant)
        assertFalse(_nodeIsSealed(eoa2));
        NodeTx memory t = NodeTx(eoa2, eoa2, false, 0, 1_000_000, 1 gwei, 1 gwei, false);
        assertTrue(_nodeTxValid(t));
        vm.prank(eoa2, eoa2);
        IXAccount(eoa2).initialize(_params(_one(_ecdsaSigner(owner)), true));
        assertFalse(IXAccount(eoa2).isEoaLocked());
    }

    function test_freeze_seals7702Account_unfreezeBackToLocked() public {
        assertFalse(acct.isEoaLocked());
        vm.prank(guardian1);
        acct.freeze();
        vm.prank(guardian3);
        acct.freeze();
        assertTrue(acct.isFrozen());
        assertTrue(acct.isEoaLocked(), "freeze also sets the protocol-level lock on a 7702 account");
        assertEq(acct.eoaLockState(), 2, "SEALED while frozen");
        // sealed: not even a self-call (the EOA key could do nothing in the account anyway) => no fee burn
        assertFalse(_nodeTxValid(_selfTx()));
        assertFalse(_nodeAuthApplies(eoa));

        vm.prank(guardian1);
        acct.unfreeze();
        vm.prank(guardian2);
        acct.unfreeze();
        assertEq(acct.eoaLockState(), 1, "back to LOCKED (disabling the lock still needs the vault delay)");
        assertTrue(_nodeTxValid(_selfTx()));
    }

    function test_freezeExpiry_refreshBackToLocked() public {
        _lock();
        vm.prank(guardian2);
        acct.freeze();
        vm.prank(guardian3);
        acct.freeze();
        assertEq(acct.eoaLockState(), 2);
        (uint64 until,,) = acct.freezeState();
        vm.warp(until);
        // A freeze that expires by itself cannot write storage: the word stays SEALED until anyone refreshes it
        // (another signer, a UserOp signed by the EOA key, a watcher bot...).
        assertEq(acct.eoaLockState(), 2);
        vm.prank(makeAddr("anyone"));
        acct.refreshEoaLock();
        assertEq(acct.eoaLockState(), 1);
        // refresh never weakens: an unlocked account with an active key stays unlocked
        vm.store(eoa, AccountConstants.EOA_LOCK_SLOT, bytes32(0));
        acct.refreshEoaLock();
        assertEq(acct.eoaLockState(), 0);
    }

    // ============================================================================================================
    // The EOA key under the lock: only the account path, vault applies like to any signer
    // ============================================================================================================

    /// @dev `vm.prank(eoa, eoa)` = a transaction signed by the EOA key with `tx.to == eoa`, `value == 0`
    ///      (msg.sender == tx.origin == address(this)): the only transaction shape the node accepts when locked.
    function test_lockedEoaKey_selfCallPath_isSubjectToVault() public {
        _lock();
        assertTrue(_nodeTxValid(_selfTx()));

        vm.startPrank(eoa, eoa);
        // below threshold: instant
        assertEq(acct.execute(friend, 0.5 ether, ""), 0);
        // above threshold: queued, like for any signer
        uint256 big = acct.execute(attacker, 50 ether, "");
        assertGt(big, 0);
        // switching protection off / adding trust / removing guardians: queued
        VaultPolicy memory off = _vaultPolicy();
        off.enabled = false;
        acct.setVaultPolicy(off);
        acct.setWhitelisted(attacker, true);
        acct.setGuardians(new address[](0), 0);
        acct.disableEoaLock();
        // unlimited approval: rejected
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.UnlimitedApprovalBlocked.selector, attacker));
        acct.execute(address(usd), 0, abi.encodeCall(usd.approve, (attacker, type(uint256).max)));
        vm.stopPrank();

        assertEq(friend.balance, 0.5 ether);
        assertEq(attacker.balance, 0);
        assertTrue(acct.vaultPolicy().enabled);
        assertFalse(acct.isWhitelisted(attacker));
        assertTrue(acct.isGuardian(guardian1));
        assertTrue(acct.isEoaLocked());
        assertEq(acct.lastActionId(), big + 4);
    }

    // ============================================================================================================
    // Recovery revokes the implicit EOA key
    // ============================================================================================================

    function _recoverTo(Signer[] memory ns) internal {
        vm.prank(guardian1);
        acct.approveRecovery(ns);
        vm.prank(guardian2);
        acct.approveRecovery(ns);
        vm.warp(block.timestamp + 48 hours);
        acct.executeRecovery(ns);
    }

    function test_recovery_revokesEoaKey_everywhere() public {
        assertTrue(acct.isEoaKeyActive());
        _recoverTo(_one(_passkeySigner()));
        assertFalse(acct.isEoaKeyActive());
        // the account was not locked before: the recovery seals it anyway (else the revoked key could re-delegate)
        assertEq(acct.eoaLockState(), 2);
        assertFalse(_nodeTxValid(_selfTx()));
        assertFalse(_nodeAuthApplies(eoa));

        // direct self-call transaction of the EOA key
        vm.prank(eoa, eoa);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(friend, 0.1 ether, "");
        // ERC-4337 op signed by the EOA key
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = _signOpEcdsa(_userOp(eoa, _execData(friend, 0.1 ether, "")), eoaPk);
        vm.prank(bundler, bundler);
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, "AA24 signature error"));
        ep.handleOps(ops, payable(bundler));

        // (contract level, defence in depth: the node already refuses every tx from the sealed EOA)
        // permissionless entry points still work when the (revoked) EOA key is tx.origin
        vm.prank(eoa, eoa);
        acct.pokeGasBudget();
        // the new owner is in control
        _handle(_signOpPasskey(_userOp(eoa, _execData(friend, 0.1 ether, ""))));
        assertEq(friend.balance, 0.1 ether);
    }

    function test_recovery_revokesEoaKey_erc1271_vaultOff() public {
        (address eoa2, uint256 eoa2Pk) = makeAddrAndKey("eoa-vault-off");
        vm.signAndAttachDelegation(Predeploys.SMART_ACCOUNT_IMPL, eoa2Pk);
        IXAccount a = IXAccount(eoa2);
        vm.prank(eoa2, eoa2);
        a.initialize(_params(_one(_ecdsaSigner(owner)), false));
        bytes32 h = keccak256("login");
        bytes memory raw = _ecdsaSign(eoa2Pk, h);
        bytes memory safe = _ecdsaSign(eoa2Pk, a.replaySafeHash(h));
        assertEq(a.isValidSignature(h, raw), IERC1271.isValidSignature.selector);
        assertEq(a.isValidSignature(h, safe), IERC1271.isValidSignature.selector);

        Signer[] memory ns = _one(_passkeySigner());
        vm.prank(guardian1);
        a.approveRecovery(ns);
        vm.prank(guardian3);
        a.approveRecovery(ns);
        vm.warp(block.timestamp + 48 hours);
        a.executeRecovery(ns);

        assertEq(a.isValidSignature(h, raw), bytes4(0xffffffff));
        assertEq(a.isValidSignature(h, safe), bytes4(0xffffffff));
        assertEq(a.isValidSignature(h, _passkeySig(a.replaySafeHash(h))), IERC1271.isValidSignature.selector);
    }

    /// @dev A matured action queued by the new owner still runs when the revoked EOA key happens to trigger it
    ///      (`execute(id)` is permissionless; its internal self-calls are self-authorized, not EOA-key authority).
    function test_revokedEoaKey_canStillTriggerMaturedActions() public {
        _recoverTo(_one(_ecdsaSigner(owner)));
        address device = makeAddr("device");
        Call[] memory calls = new Call[](2);
        calls[0] = _call(friend, 2 ether, "");
        calls[1] = _call(eoa, 0, abi.encodeCall(acct.addSigner, (_ecdsaSigner(device))));
        vm.prank(owner);
        uint256 id = acct.executeBatch(calls);
        assertGt(id, 0);
        vm.warp(block.timestamp + 24 hours);
        vm.prank(eoa, eoa);
        acct.execute(id);
        assertEq(friend.balance, 2 ether);
        assertTrue(acct.isSigner(bytes32(uint256(uint160(device)))));
    }

    function test_recovery_keepsEoaKey_whenListedExplicitly() public {
        Signer[] memory ns = new Signer[](2);
        ns[0] = _passkeySigner();
        ns[1] = _ecdsaSigner(eoa);
        _recoverTo(ns);
        assertTrue(acct.isEoaKeyActive());
        assertEq(acct.eoaLockState(), 0, "kept key: lock state unchanged");
        vm.prank(eoa, eoa);
        acct.execute(friend, 0.1 ether, "");
        assertEq(friend.balance, 0.1 ether);

        // removing the explicit entry later revokes the key => sealed
        _handle(
            _signOpPasskey(_userOp(eoa, abi.encodeCall(acct.removeSigner, (bytes32(uint256(uint160(eoa)))))))
        );
        uint256 id = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);
        assertFalse(acct.isEoaKeyActive());
        assertEq(acct.eoaLockState(), 2);
    }

    function test_revokedKey_addBackAsSigner_unseals_disableRefusedWhileRevoked() public {
        _recoverTo(_one(_ecdsaSigner(owner)));
        assertEq(acct.eoaLockState(), 2);
        // lifting the lock of a revoked key is refused (the lock is what stops it from re-delegating)
        vm.prank(owner);
        acct.disableEoaLock();
        uint256 id = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours);
        vm.expectRevert(IAccountEvents.EoaKeyInactive.selector);
        acct.execute(id);
        assertEq(acct.eoaLockState(), 2);
        // the new owner re-trusts the EOA key (e.g. a lost-phone recovery, the seed phrase was found again)
        vm.prank(owner);
        acct.addSigner(_ecdsaSigner(eoa));
        assertTrue(acct.isEoaKeyActive());
        assertEq(acct.eoaLockState(), 1, "SEALED -> LOCKED");
        vm.prank(eoa, eoa);
        acct.execute(friend, 0.1 ether, "");
        assertEq(friend.balance, 0.1 ether);
    }

    // ============================================================================================================
    // Demo: stolen EOA key on a 7702 account
    // ============================================================================================================

    function test_demo_stolenEoaKey_lockFreezeRecover() public {
        uint256 start = eoa.balance;
        // 0. The user opted into the lock (wallet default for 7702 accounts with the vault on).
        _lock();

        // 1. The thief has the EOA key. Raw transactions that bypass the account are invalid at the node:
        assertFalse(
            _nodeTxValid(NodeTx(eoa, attacker, false, 90 ether, 21_000, 2 gwei, 1 gwei, false)),
            "plain transfer"
        );
        assertFalse(
            _nodeTxValid(NodeTx(eoa, address(usd), false, 0, 100_000, 2 gwei, 1 gwei, false)), "token call"
        );
        assertFalse(_nodeTxValid(NodeTx(eoa, address(0), true, 0, 500_000, 2 gwei, 1 gwei, false)), "create");
        assertFalse(_nodeAuthApplies(eoa), "re-delegation / un-delegation skipped");
        // ... so it uses the account path, where the vault applies:
        vm.startPrank(eoa, eoa);
        uint256 drainId = acct.execute(attacker, 90 ether, "");
        acct.disableEoaLock();
        uint256 unlockId = acct.lastActionId();
        for (uint256 i; i < 5; ++i) {
            acct.execute(attacker, 1 ether, ""); // the bounded daily allowance
        }
        // ... and it would cancel any recovery (it has owner authority)
        acct.cancelRecovery();
        vm.stopPrank();

        // 2. Guardians freeze: the EOA key can do nothing any more (and the lock stays on).
        vm.prank(guardian1);
        acct.freeze();
        vm.prank(guardian2);
        acct.freeze();
        // ... at the protocol level the address is SEALED: not even fee-burning self-calls are accepted
        assertEq(acct.eoaLockState(), 2);
        assertFalse(_nodeTxValid(_selfTx()));
        // (contract level, defence in depth)
        vm.startPrank(eoa, eoa);
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.execute(attacker, 0.1 ether, "");
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.cancelRecovery();
        vm.expectRevert(IAccountEvents.AccountIsFrozen.selector);
        acct.cancel(unlockId);
        vm.stopPrank();

        // 3. Recovery to the owner's other device; it cannot be cancelled while frozen.
        _recoverTo(_one(_ecdsaSigner(owner)));
        assertFalse(acct.isFrozen());
        assertFalse(acct.isEoaKeyActive());
        assertTrue(acct.isEoaLocked(), "lock stays: the stolen key still cannot re-delegate");
        assertEq(acct.eoaLockState(), 2, "SEALED: the revoked key cannot send any transaction (no fee burn)");
        assertFalse(_nodeAuthApplies(eoa));
        assertFalse(_nodeTxValid(_selfTx()));

        // 4. The thief's key is useless; its queued actions died with the recovery epoch bump.
        vm.prank(eoa, eoa);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(attacker, 0.1 ether, "");
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionInvalidated.selector, drainId));
        acct.execute(drainId);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionInvalidated.selector, unlockId));
        acct.execute(unlockId);

        // 5. The owner controls the address again (e.g. to migrate funds through the vault).
        vm.prank(owner);
        acct.execute(friend, 1 ether, "");
        assertEq(friend.balance, 1 ether);
        assertEq(attacker.balance, 5 ether, "loss bounded by the daily limit");
        assertEq(eoa.balance, start - 6 ether);
    }

    // ============================================================================================================
    // Node rule test vectors (mirrors docs/specs/xip-0001-eoa-lock.md §8)
    // ============================================================================================================

    function test_nodeRule_vectors() public {
        // V1: unlocked delegated EOA — rules do not apply
        assertTrue(_nodeTxValid(NodeTx(eoa, attacker, false, 1 ether, 21_000, 100 gwei, 1 gwei, false)));
        assertTrue(_nodeAuthApplies(eoa));

        _lock();
        // V2: self-call, value 0, within limits — valid
        assertTrue(_nodeTxValid(_selfTx()));
        // V3: self-call with value — invalid
        NodeTx memory t = _selfTx();
        t.value = 1;
        assertFalse(_nodeTxValid(t));
        // V4: call to another address — invalid
        t = _selfTx();
        t.to = friend;
        assertFalse(_nodeTxValid(t));
        // V5: creation — invalid
        t = _selfTx();
        t.isCreate = true;
        t.to = address(0);
        assertFalse(_nodeTxValid(t));
        // V6: second tx from the same locked sender in the block — invalid
        t = _selfTx();
        t.earlierTxFromSenderInBlock = true;
        assertFalse(_nodeTxValid(t));
        // V7: gas limit 2,000,000 valid / 2,000,001 invalid
        t = _selfTx();
        t.gasLimit = 2_000_000;
        assertTrue(_nodeTxValid(t));
        t.gasLimit = 2_000_001;
        assertFalse(_nodeTxValid(t));
        // V8: max fee exactly 2x base fee valid / +1 wei invalid
        t = _selfTx();
        t.maxFeePerGas = 2 gwei;
        assertTrue(_nodeTxValid(t));
        t.maxFeePerGas = 2 gwei + 1;
        assertFalse(_nodeTxValid(t));
        // V9: authorization by the locked authority — skipped
        assertFalse(_nodeAuthApplies(eoa));

        // V13: sealed (word 2) - even a valid-looking self-call is invalid; authorizations skipped
        vm.store(eoa, AccountConstants.EOA_LOCK_SLOT, bytes32(uint256(2)));
        assertFalse(_nodeTxValid(_selfTx()));
        assertFalse(_nodeAuthApplies(eoa));
        // V14: any other non-zero word (garbage) - locked, not sealed
        vm.store(eoa, AccountConstants.EOA_LOCK_SLOT, bytes32(uint256(0xdead)));
        assertTrue(_nodeTxValid(_selfTx()));
        vm.store(eoa, AccountConstants.EOA_LOCK_SLOT, bytes32(uint256(1)));

        // V10: slot set but delegated elsewhere — not locked (rule only covers delegations to 0x…1005)
        (address other, uint256 otherPk) = makeAddrAndKey("other-delegate");
        vm.signAndAttachDelegation(address(counter), otherPk);
        vm.store(other, AccountConstants.EOA_LOCK_SLOT, bytes32(uint256(1)));
        assertFalse(_nodeIsLocked(other));
        // V11: factory account (real contract code) with the slot written — not locked (not a designator)
        IXAccount f = _vaultAccount();
        vm.store(address(f), AccountConstants.EOA_LOCK_SLOT, bytes32(uint256(1)));
        assertFalse(_nodeIsLocked(address(f)));
        // V12: lock disabled through the matured vault action — rules stop applying from the next tx
        vm.prank(eoa, eoa);
        acct.disableEoaLock();
        uint256 id = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);
        assertFalse(_nodeIsLocked(eoa));
        assertTrue(_nodeTxValid(NodeTx(eoa, friend, false, 1 ether, 21_000, 10 gwei, 1 gwei, true)));
    }

    function test_erc1271_rawEoaSignature_respectsVault() public view {
        // vault on + allowInstantSignatures false: even the (active) EOA key cannot sign off-chain permits
        bytes32 h = keccak256("permit2");
        assertEq(acct.isValidSignature(h, _ecdsaSign(eoaPk, h)), bytes4(0xffffffff));
        assertTrue(acct.supportsInterface(type(IERC1271).interfaceId));
        assertTrue(acct.supportsInterface(type(IGuardianVault).interfaceId));
        assertTrue(acct.supportsInterface(type(IGuardianFreeze).interfaceId));
    }
}
