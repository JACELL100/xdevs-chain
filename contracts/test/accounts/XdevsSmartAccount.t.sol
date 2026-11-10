// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IAccount} from "account-abstraction/interfaces/IAccount.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";
import {WebAuthn} from "@openzeppelin/contracts/utils/cryptography/WebAuthn.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {DummyImplementationV2} from "./utils/Mocks.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {SmartAccountFactory} from "../../src/accounts/SmartAccountFactory.sol";
import {IAccountEvents} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {Call, InitParams, Signer, SignerType} from "../../src/accounts/libraries/AccountTypes.sol";

contract XdevsSmartAccountTest is AccountTestBase {
    // ------------------------------------------------------------------------------------------------------------
    // Factory / deployment
    // ------------------------------------------------------------------------------------------------------------

    function test_factory_deterministicAddress() public {
        Signer[] memory signers = _one(_ecdsaSigner(owner));
        address predicted = factory.getAddress(signers, bytes32(uint256(7)));
        address created = factory.createAccount(signers, bytes32(uint256(7)));
        assertEq(created, predicted);
        assertGt(created.code.length, 0);
        // idempotent
        assertEq(factory.createAccount(signers, bytes32(uint256(7))), created);
        // different salt / different signers => different address
        assertTrue(factory.getAddress(signers, bytes32(uint256(8))) != created);
        assertTrue(factory.getAddress(_one(_ecdsaSigner(attacker)), bytes32(uint256(7))) != created);
        // proxy points at the predeployed implementation
        assertEq(
            address(uint160(uint256(vm.load(created, ERC1967Utils.IMPLEMENTATION_SLOT)))),
            Predeploys.SMART_ACCOUNT_IMPL
        );
        assertTrue(IXAccount(created).isInitialized());
        assertEq(IXAccount(created).getSigners().length, 1);
    }

    function test_factory_withConfig_matchesPrediction() public {
        InitParams memory p = _params(_one(_passkeySigner()), true);
        address predicted = factory.getAddressWithConfig(p, bytes32(uint256(1)));
        IXAccount acct = _createAccount(p, bytes32(uint256(1)));
        assertEq(address(acct), predicted);
        (address[] memory g, uint8 t) = acct.getGuardians();
        assertEq(g.length, 3);
        assertEq(t, 2);
        assertEq(acct.recoveryDelay(), 48 hours);
        assertTrue(acct.vaultPolicy().enabled);
    }

    function test_factory_rejectsEmptySigners() public {
        vm.expectRevert();
        factory.createAccount(new Signer[](0), bytes32(0));
    }

    function test_factory_rejectsInvalidPasskey() public {
        Signer[] memory s = _one(Signer(SignerType.Passkey, bytes32(uint256(1)), bytes32(uint256(2))));
        vm.expectRevert();
        factory.createAccount(s, bytes32(0));
    }

    // ------------------------------------------------------------------------------------------------------------
    // Genesis / initialization safety
    // ------------------------------------------------------------------------------------------------------------

    function test_predeployedImplementation_cannotBeInitialized() public {
        InitParams memory p = _params(_one(_ecdsaSigner(attacker)), false);
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        IXAccount(payable(Predeploys.SMART_ACCOUNT_IMPL)).initialize(p);
        // even as a "self-call" on the implementation address
        vm.prank(Predeploys.SMART_ACCOUNT_IMPL);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        IXAccount(payable(Predeploys.SMART_ACCOUNT_IMPL)).initialize(p);
    }

    function test_factoryPredeploy_extensionIsInert() public {
        InitParams memory p = _params(_one(_ecdsaSigner(attacker)), false);
        vm.startPrank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        IXAccount(address(factory)).initialize(p);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        IXAccount(address(factory)).addSigner(_ecdsaSigner(attacker));
        vm.expectRevert(IAccountEvents.NotGuardian.selector);
        IXAccount(address(factory)).approveRecovery(_one(_ecdsaSigner(attacker)));
        vm.expectRevert(IAccountEvents.RecoveryNotReady.selector);
        IXAccount(address(factory)).executeRecovery(_one(_ecdsaSigner(attacker)));
        vm.stopPrank();
    }

    function test_factoryFunctions_notReachableThroughAccount() public {
        IXAccount acct = _vaultAccount();
        vm.expectRevert(SmartAccountFactory.NotFactoryContext.selector);
        SmartAccountFactory(address(acct)).createAccount(_one(_ecdsaSigner(owner)), bytes32(0));
    }

    function test_reinitialize_reverts() public {
        IXAccount acct = _vaultAccount();
        InitParams memory p = _params(_one(_ecdsaSigner(attacker)), false);
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.initialize(p);
        vm.prank(address(acct));
        vm.expectRevert(IAccountEvents.AlreadyInitialized.selector);
        acct.initialize(p);
    }

    function test_unknownSelector_reverts() public {
        IXAccount acct = _vaultAccount();
        (bool ok,) = address(acct).call(abi.encodeWithSignature("doesNotExist()"));
        assertFalse(ok);
    }

    function test_supportsInterface() public {
        IXAccount acct = _vaultAccount();
        assertTrue(acct.supportsInterface(type(IAccount).interfaceId));
        assertTrue(acct.supportsInterface(type(IERC1271).interfaceId));
        assertFalse(acct.supportsInterface(0xdeadbeef));
    }

    // ------------------------------------------------------------------------------------------------------------
    // Direct execution (owner key)
    // ------------------------------------------------------------------------------------------------------------

    function test_execute_byEcdsaSigner() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        address to = makeAddr("to");
        vm.prank(owner);
        acct.execute(to, 3 ether, "");
        assertEq(to.balance, 3 ether);
    }

    function test_execute_byStranger_reverts() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(attacker, 1 ether, "");
        vm.prank(guardian1); // guardians are not owners
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(guardian1, 1 ether, "");
    }

    function test_executeBatch() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        Call[] memory calls = new Call[](2);
        calls[0] = _call(address(counter), 0, abi.encodeCall(counter.increment, ()));
        calls[1] = _call(address(counter), 1 ether, abi.encodeCall(counter.pay, ()));
        vm.prank(owner);
        acct.executeBatch(calls);
        assertEq(counter.count(), 1);
        assertEq(counter.paid(address(acct)), 1 ether);
    }

    function test_executeBatch_bubblesRevert() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        Call[] memory calls = new Call[](2);
        calls[0] = _call(address(counter), 0, abi.encodeCall(counter.increment, ()));
        calls[1] = _call(address(usd), 0, abi.encodeCall(usd.transfer, (owner, 1))); // no balance -> revert
        vm.prank(owner);
        vm.expectRevert();
        acct.executeBatch(calls);
        assertEq(counter.count(), 0);
    }

    // ------------------------------------------------------------------------------------------------------------
    // ERC-4337
    // ------------------------------------------------------------------------------------------------------------

    function test_userOp_ecdsa_execute() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        address to = makeAddr("to");
        PackedUserOperation memory op = _userOp(address(acct), _execData(to, 1 ether, ""));
        _handle(_signOpEcdsa(op, ownerPk));
        assertEq(to.balance, 1 ether);
        assertEq(acct.getNonce(), 1);
    }

    function test_userOp_wrongSigner_fails() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        PackedUserOperation memory op = _userOp(address(acct), _execData(attacker, 1 ether, ""));
        op = _signOpEcdsa(op, attackerPk);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, "AA24 signature error"));
        ep.handleOps(ops, payable(bundler));
    }

    function test_validateUserOp_onlyEntryPoint() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        PackedUserOperation memory op = _userOp(address(acct), "");
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        IAccount(address(acct)).validateUserOp(op, bytes32(0), 0);
    }

    /// @notice Phase 4 exit criterion: a brand-new user with ONLY a passkey (no seed phrase) gets an account
    ///         deployed counterfactually through the EntryPoint and transacts.
    function test_passkeyOnlyAccount_deployAndTransactViaUserOp() public {
        Signer[] memory signers = _one(_passkeySigner());
        bytes32 salt = keccak256("alice");
        address predicted = factory.getAddress(signers, salt);
        assertEq(predicted.code.length, 0);
        vm.deal(predicted, 2 ether); // e.g. faucet funds sent to the counterfactual address

        address to = makeAddr("bob");
        PackedUserOperation memory op = _userOp(predicted, _execData(to, 0.5 ether, ""));
        op.initCode =
            abi.encodePacked(address(factory), abi.encodeCall(factory.createAccount, (signers, salt)));
        _handle(_signOpPasskey(op));

        assertGt(predicted.code.length, 0);
        assertEq(to.balance, 0.5 ether);
        // a second op without initCode
        PackedUserOperation memory op2 = _userOp(predicted, _execData(to, 0.25 ether, ""));
        _handle(_signOpPasskey(op2));
        assertEq(to.balance, 0.75 ether);
    }

    function test_passkey_rejectsTamperedOrUnverified() public {
        IXAccount acct = _createAccount(_params(_one(_passkeySigner()), false), 0);
        bytes32 h = keccak256("msg");
        bytes32 safe = acct.replaySafeHash(h);
        // valid
        assertEq(acct.isValidSignature(h, _passkeySig(safe)), IERC1271.isValidSignature.selector);
        // wrong challenge
        assertEq(acct.isValidSignature(h, _passkeySig(keccak256("other"))), bytes4(0xffffffff));
        // user-present only (no UV flag)
        assertEq(
            acct.isValidSignature(h, _encodePasskeySig(_webAuthn(passkeyPk, safe, 0x01))), bytes4(0xffffffff)
        );
        // backup state without backup eligibility
        assertEq(
            acct.isValidSignature(h, _encodePasskeySig(_webAuthn(passkeyPk, safe, 0x15))), bytes4(0xffffffff)
        );
        // wrong key
        assertEq(
            acct.isValidSignature(h, _encodePasskeySig(_webAuthn(passkeyPk + 1, safe, 0x05))),
            bytes4(0xffffffff)
        );
        // high-s (malleated) signature
        WebAuthn.WebAuthnAuth memory auth = _webAuthn(passkeyPk, safe, 0x05);
        auth.s = bytes32(P256_N - uint256(auth.s));
        assertEq(acct.isValidSignature(h, _encodePasskeySig(auth)), bytes4(0xffffffff));
        // wrong type
        auth = _webAuthn(passkeyPk, safe, 0x05);
        auth.typeIndex = 2;
        assertEq(acct.isValidSignature(h, _encodePasskeySig(auth)), bytes4(0xffffffff));
    }

    function test_passkey_failsClosedWithoutPrecompile() public {
        IXAccount acct = _createAccount(_params(_one(_passkeySigner()), false), 0);
        bytes32 h = keccak256("msg");
        bytes memory sig = _passkeySig(acct.replaySafeHash(h));
        vm.etch(Predeploys.P256_VERIFY, "");
        assertEq(acct.isValidSignature(h, sig), bytes4(0xffffffff));
    }

    // ------------------------------------------------------------------------------------------------------------
    // ERC-1271
    // ------------------------------------------------------------------------------------------------------------

    function test_erc1271_vaultDisabled_replaySafe() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        IXAccount acct2 = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(2)));
        bytes32 h = keccak256("login");
        bytes memory sig = _ecdsaSign(ownerPk, acct.replaySafeHash(h));
        assertEq(acct.isValidSignature(h, sig), IERC1271.isValidSignature.selector);
        // same key, other account: not replayable
        assertEq(acct2.isValidSignature(h, sig), bytes4(0xffffffff));
        // raw hash signature is not accepted for a proxy account
        assertEq(acct.isValidSignature(h, _ecdsaSign(ownerPk, h)), bytes4(0xffffffff));
    }

    function test_erc1271_vaultEnabled_requiresDelayedApproval() public {
        IXAccount acct = _vaultAccount();
        bytes32 h = keccak256("permit2 transfer");
        bytes memory sig = _ecdsaSign(ownerPk, acct.replaySafeHash(h));
        assertEq(acct.isValidSignature(h, sig), bytes4(0xffffffff));

        vm.prank(owner);
        acct.approveHash(h); // queued
        uint256 id = acct.lastActionId();
        assertFalse(acct.isHashApproved(h));
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);
        assertTrue(acct.isHashApproved(h));
        assertEq(acct.isValidSignature(h, ""), IERC1271.isValidSignature.selector);

        vm.prank(owner);
        acct.revokeHash(h); // instant
        assertEq(acct.isValidSignature(h, ""), bytes4(0xffffffff));
    }

    // ------------------------------------------------------------------------------------------------------------
    // Signers
    // ------------------------------------------------------------------------------------------------------------

    function test_addSigner_instant_and_removeSigner() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        vm.startPrank(owner);
        acct.addSigner(_passkeySigner());
        assertEq(acct.getSigners().length, 2);
        vm.expectRevert();
        acct.addSigner(_passkeySigner()); // duplicate
        bytes32 id = acct.signerId(_ecdsaSigner(owner));
        acct.removeSigner(id); // vault disabled -> instant
        vm.stopPrank();
        assertEq(acct.getSigners().length, 1);
        assertFalse(acct.isSigner(id));
        // the old key lost access
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.execute(owner, 1, "");
    }

    function test_removeLastSigner_reverts() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        bytes32 id = acct.signerId(_ecdsaSigner(owner));
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.LastSigner.selector);
        acct.removeSigner(id);
    }

    function test_removeSigner_delayedByVault() public {
        IXAccount acct = _vaultAccount();
        vm.prank(owner);
        acct.addSigner(_passkeySigner());
        bytes32 id = acct.signerId(_passkeySigner());
        vm.prank(owner);
        acct.removeSigner(id);
        assertTrue(acct.isSigner(id), "removal must be queued");
        uint256 actionId = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours);
        acct.execute(actionId);
        assertFalse(acct.isSigner(id));
    }

    // ------------------------------------------------------------------------------------------------------------
    // Upgrades
    // ------------------------------------------------------------------------------------------------------------

    function test_upgrade_vaultDisabled() public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        DummyImplementationV2 v2 = new DummyImplementationV2();
        vm.prank(owner);
        acct.upgradeToAndCall(address(v2), "");
        assertEq(DummyImplementationV2(address(acct)).version2(), 2);
    }

    function test_upgrade_delayedByVault_andCancellable() public {
        IXAccount acct = _vaultAccount();
        DummyImplementationV2 v2 = new DummyImplementationV2();
        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.upgradeToAndCall(address(v2), "");

        vm.prank(owner);
        acct.upgradeToAndCall(address(v2), "");
        uint256 id = acct.lastActionId();
        vm.prank(guardian2);
        acct.cancel(id);
        vm.warp(block.timestamp + 30 days);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, id));
        acct.execute(id);
        assertEq(
            address(uint160(uint256(vm.load(address(acct), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            Predeploys.SMART_ACCOUNT_IMPL
        );
    }

    function test_receivesNftAndNative() public {
        IXAccount acct = _vaultAccount();
        nft.mint(address(this), 1);
        nft.safeTransferFrom(address(this), address(acct), 1);
        assertEq(nft.ownerOf(1), address(acct));
        (bool ok,) = address(acct).call{value: 1 ether}("");
        assertTrue(ok);
    }
}

contract XdevsSmartAccountFuzzTest is AccountTestBase {
    /// @dev Regression: a raw 65-byte owner signature must validate whatever its first byte is (it once collided
    ///      with the 0x03 session-key prefix when `r` started with 0x03).
    function testFuzz_validateUserOp_rawEcdsaAlwaysValid(bytes32 userOpHash) public {
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        PackedUserOperation memory op = _userOp(address(acct), "");
        op.signature = _ecdsaSign(ownerPk, userOpHash);
        vm.prank(address(ep));
        assertEq(IAccount(address(acct)).validateUserOp(op, userOpHash, 0), 0);
        op.signature = _ecdsaSign(attackerPk, userOpHash);
        vm.prank(address(ep));
        assertEq(IAccount(address(acct)).validateUserOp(op, userOpHash, 0), 1);
    }

    /// @dev ERC-1271 with the vault disabled accepts exactly the owner's signature over the replay-safe hash.
    function testFuzz_isValidSignature(bytes32 h, uint256 pk) public {
        pk = bound(pk, 1, 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140);
        IXAccount acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        bytes4 res = acct.isValidSignature(h, _ecdsaSign(pk, acct.replaySafeHash(h)));
        assertEq(res, pk == ownerPk ? bytes4(0x1626ba7e) : bytes4(0xffffffff));
    }
}
