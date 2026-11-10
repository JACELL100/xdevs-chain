// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {IAccountEvents} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {AccountStorage} from "../../src/accounts/libraries/AccountStorage.sol";
import {InitParams, Signer} from "../../src/accounts/libraries/AccountTypes.sol";

/// @notice EIP-7702: an existing EOA delegates to the predeployed implementation and keeps its address.
contract EIP7702Test is AccountTestBase {
    address internal eoa;
    uint256 internal eoaPk;
    IXAccount internal acct;

    function setUp() public override {
        super.setUp();
        (eoa, eoaPk) = makeAddrAndKey("metamask-user");
        vm.deal(eoa, 100 ether);
        vm.signAndAttachDelegation(Predeploys.SMART_ACCOUNT_IMPL, eoaPk);
        acct = IXAccount(eoa);
    }

    function test_delegationDesignator() public view {
        assertEq(eoa.code, abi.encodePacked(hex"ef0100", Predeploys.SMART_ACCOUNT_IMPL));
    }

    function test_uninitialized_eoaKeyIsImplicitSigner() public {
        address to = makeAddr("to");
        vm.prank(eoa, eoa); // EOA sends a tx to itself
        acct.execute(to, 1 ether, "");
        assertEq(to.balance, 1 ether);
        assertFalse(acct.isInitialized());

        // ERC-4337 op signed by the EOA key
        PackedUserOperation memory op = _userOp(eoa, _execData(to, 1 ether, ""));
        _handle(_signOpEcdsa(op, eoaPk));
        assertEq(to.balance, 2 ether);

        // raw EOA signature accepted by ERC-1271 (Permit2 & co. treat 7702 EOAs as contracts)
        bytes32 h = keccak256("hello");
        assertEq(acct.isValidSignature(h, _ecdsaSign(eoaPk, h)), IERC1271.isValidSignature.selector);
    }

    function test_selfInitialize_andFrontRunProtection() public {
        InitParams memory p = _params(_one(_ecdsaSigner(eoa)), true);
        p.signers = new Signer[](2);
        p.signers[0] = _ecdsaSigner(eoa);
        p.signers[1] = _passkeySigner();

        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.initialize(_params(_one(_ecdsaSigner(attacker)), false));

        vm.prank(eoa, eoa);
        acct.initialize(p);
        assertTrue(acct.isInitialized());
        assertEq(acct.getSigners().length, 2);
        assertTrue(acct.vaultPolicy().enabled);

        // the vault now protects the EOA's contract-level execution paths
        vm.prank(eoa, eoa);
        assertGt(acct.execute(attacker, 10 ether, ""), 0);
        assertEq(attacker.balance, 0);
        // and the EOA cannot bypass the delay by calling a delayed function on itself
        vm.prank(eoa, eoa);
        acct.setWhitelisted(attacker, true);
        assertFalse(acct.isWhitelisted(attacker));
    }

    function test_initializeWithSignature_relayed() public {
        InitParams memory p = _params(_one(_passkeySigner()), true);
        bytes32 digest = _ethSigned(acct.initHash(p));
        bytes memory sig = _ecdsaSign(eoaPk, digest);

        // a different signature is rejected
        vm.expectRevert();
        acct.initializeWithSignature(p, _ecdsaSign(attackerPk, digest));

        vm.prank(bundler);
        acct.initializeWithSignature(p, sig);
        assertTrue(acct.isInitialized());
        // replay is impossible
        vm.expectRevert(IAccountEvents.AlreadyInitialized.selector);
        acct.initializeWithSignature(p, sig);
    }

    function test_namespacedStorage_ignoresPreviousDelegateState() public {
        // garbage left in sequential slots by a previous delegate
        for (uint256 i; i < 8; ++i) {
            vm.store(eoa, bytes32(i), bytes32(type(uint256).max));
        }
        vm.prank(eoa, eoa);
        acct.initialize(_params(_one(_ecdsaSigner(eoa)), false));
        assertEq(acct.getSigners().length, 1);
        assertEq(acct.lastActionId(), 0);
        (address[] memory g,) = acct.getGuardians();
        assertEq(g.length, 3);
    }

    function test_erc7201SlotsMatchFormula() public pure {
        assertEq(AccountStorage.CORE_SLOT, _erc7201("xdevs.account.core"));
        assertEq(AccountStorage.SIGNERS_SLOT, _erc7201("xdevs.account.signers"));
        assertEq(AccountStorage.VAULT_SLOT, _erc7201("xdevs.account.vault"));
        assertEq(AccountStorage.RECOVERY_SLOT, _erc7201("xdevs.account.recovery"));
        assertEq(AccountStorage.SESSIONS_SLOT, _erc7201("xdevs.account.sessions"));
        assertEq(AccountStorage.SELF_AUTH_TSLOT, _erc7201("xdevs.account.transient.selfauth"));
    }

    function _erc7201(string memory id) internal pure returns (bytes32) {
        return keccak256(abi.encode(uint256(keccak256(bytes(id))) - 1)) & ~bytes32(uint256(0xff));
    }

    function test_upgrade_notAvailableFor7702() public {
        vm.prank(eoa, eoa);
        vm.expectRevert(IAccountEvents.NotProxy.selector);
        acct.upgradeToAndCall(address(counter), "");
    }

    function test_userOp_with7702InitCodeMarker() public {
        address to = makeAddr("to");
        PackedUserOperation memory op = _userOp(eoa, _execData(to, 1 ether, ""));
        op.initCode = abi.encodePacked(bytes20(uint160(0x7702) << 144)); // 0x7702 marker, no init call
        _handle(_signOpEcdsa(op, eoaPk));
        assertEq(to.balance, 1 ether);
    }
}

