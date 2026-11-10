// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {Test} from "forge-std/Test.sol";

import {EntryPoint} from "account-abstraction/core/EntryPoint.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {Base64} from "@openzeppelin/contracts/utils/Base64.sol";
import {WebAuthn} from "@openzeppelin/contracts/utils/cryptography/WebAuthn.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

import {Predeploys} from "../../../src/common/Predeploys.sol";
import {XdevsSmartAccount} from "../../../src/accounts/XdevsSmartAccount.sol";
import {SmartAccountFactory} from "../../../src/accounts/SmartAccountFactory.sol";
import {XdevsPaymaster} from "../../../src/accounts/paymaster/XdevsPaymaster.sol";
import {
    IAccountExtension,
    IAccountEvents,
    IGuardianFreeze
} from "../../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "../../../src/accounts/libraries/AccountConstants.sol";
import {
    Call,
    InitParams,
    Signer,
    SignerType,
    VaultPolicy,
    TokenLimit,
    SessionKeyConfig,
    SessionPermission,
    SessionTokenCap
} from "../../../src/accounts/libraries/AccountTypes.sol";
import {MockERC20, MockERC721, Counter} from "./Mocks.sol";

/// @notice Combined ABI of the account core + extension, used by tests to call everything on the account address.
interface IXAccount is IAccountExtension, IGuardianFreeze, IAccountEvents {
    function execute(address target, uint256 value, bytes calldata data) external returns (uint256);
    function executeBatch(Call[] calldata calls) external returns (uint256);
    function executeWithSession(Call[] calldata calls) external;
    function execute(uint256 id) external;
    function cancel(uint256 id) external;
    function setPolicy(uint256 nativeThreshold, uint64 delaySeconds, address[] calldata whitelist) external;
    function isValidSignature(bytes32 hash, bytes calldata signature) external view returns (bytes4);
    function replaySafeHash(bytes32 hash) external view returns (bytes32);
    function pokeGasBudget() external;
    function getNonce() external view returns (uint256);
    function entryPoint() external view returns (address);
    function supportsInterface(bytes4 id) external view returns (bool);
}

/// @notice Shared fixture: mirrors the genesis layout (runtime code etched at the predeploy addresses, no
///         constructor at the final address) and installs the canonical v0.9 EntryPoint. P-256 verification uses
///         the EVM's native EIP-7951 precompile (Osaka).
abstract contract AccountTestBase is Test {
    uint256 internal constant P256_N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551;

    IEntryPoint internal ep;
    SmartAccountFactory internal factory;
    XdevsPaymaster internal paymaster;

    address internal bundler = makeAddr("bundler");

    uint256 internal ownerPk;
    address internal owner;
    uint256 internal passkeyPk;
    bytes32 internal passkeyX;
    bytes32 internal passkeyY;
    uint256 internal attackerPk;
    address internal attacker;

    address internal guardian1 = makeAddr("guardian1");
    address internal guardian2 = makeAddr("guardian2");
    address internal guardian3 = makeAddr("guardian3");

    MockERC20 internal usd;
    MockERC721 internal nft;
    Counter internal counter;

    function setUp() public virtual {
        vm.warp(1_750_000_000);

        // Canonical ERC-4337 v0.9 EntryPoint (constructor runs at the target so its immutables are correct).
        deployCodeTo(
            "lib/account-abstraction/contracts/core/EntryPoint.sol:EntryPoint", AccountConstants.ENTRY_POINT
        );
        ep = IEntryPoint(AccountConstants.ENTRY_POINT);

        // P256VERIFY (EIP-7951) at 0x100 is native under `evm_version = "osaka"`, exactly as on the node (revm Osaka).
        require(_hasNativeP256(), "P256VERIFY (EIP-7951) missing at 0x100: run with evm_version = osaka");

        // Genesis-style predeploys: runtime code only, empty storage.
        vm.etch(Predeploys.SMART_ACCOUNT_IMPL, address(new XdevsSmartAccount()).code);
        vm.etch(Predeploys.SMART_ACCOUNT_FACTORY, address(new SmartAccountFactory()).code);
        vm.etch(Predeploys.PAYMASTER, address(new XdevsPaymaster()).code);
        factory = SmartAccountFactory(Predeploys.SMART_ACCOUNT_FACTORY);
        paymaster = XdevsPaymaster(Predeploys.PAYMASTER);

        (owner, ownerPk) = makeAddrAndKey("owner");
        (attacker, attackerPk) = makeAddrAndKey("attacker");
        passkeyPk = uint256(keccak256("xdevs passkey")) % P256_N;
        (uint256 x, uint256 y) = vm.publicKeyP256(passkeyPk);
        passkeyX = bytes32(x);
        passkeyY = bytes32(y);

        usd = new MockERC20();
        nft = new MockERC721();
        counter = new Counter();
    }

    function _hasNativeP256() internal returns (bool) {
        uint256 pk = 7;
        (uint256 x, uint256 y) = vm.publicKeyP256(pk);
        bytes32 h = keccak256("probe");
        (bytes32 r, bytes32 s) = vm.signP256(pk, h);
        (bool ok, bytes memory ret) = Predeploys.P256_VERIFY.staticcall(abi.encode(h, r, s, x, y));
        return ok && ret.length == 32 && abi.decode(ret, (uint256)) == 1;
    }

    // ------------------------------------------------------------------------------------------------------------
    // Signers / params
    // ------------------------------------------------------------------------------------------------------------

    function _ecdsaSigner(address a) internal pure returns (Signer memory) {
        return Signer(SignerType.ECDSA, bytes32(uint256(uint160(a))), bytes32(0));
    }

    function _passkeySigner() internal view returns (Signer memory) {
        return Signer(SignerType.Passkey, passkeyX, passkeyY);
    }

    function _one(Signer memory s) internal pure returns (Signer[] memory arr) {
        arr = new Signer[](1);
        arr[0] = s;
    }

    function _guardians() internal view returns (address[] memory g) {
        g = new address[](3);
        g[0] = guardian1;
        g[1] = guardian2;
        g[2] = guardian3;
    }

    /// @dev Vault on: 1 ether per tx, 5 ether per day, 24 h delay, unlimited approvals blocked.
    function _vaultPolicy() internal pure returns (VaultPolicy memory) {
        return VaultPolicy({
            enabled: true,
            blockUnlimitedApprovals: true,
            allowInstantSignatures: false,
            delay: 24 hours,
            nativeThreshold: 1 ether,
            dailyLimit: 5 ether,
            dailyGasBudget: 1 ether
        });
    }

    function _params(Signer[] memory signers, bool vault) internal view returns (InitParams memory p) {
        p.signers = signers;
        p.guardians = _guardians();
        p.guardianThreshold = 2;
        p.recoveryDelay = 0; // default 48 h
        if (vault) {
            p.policy = _vaultPolicy();
        }
        p.whitelist = new address[](0);
        p.tokenLimits = new TokenLimit[](0);
    }

    function _createAccount(InitParams memory p, bytes32 salt) internal returns (IXAccount acct) {
        acct = IXAccount(factory.createAccountWithConfig(p, salt));
        vm.deal(address(acct), 100 ether);
    }

    /// @dev Factory account owned by `owner` (ECDSA), 3 guardians (2-of-3), vault enabled.
    function _vaultAccount() internal returns (IXAccount) {
        return _createAccount(_params(_one(_ecdsaSigner(owner)), true), bytes32(0));
    }

    // ------------------------------------------------------------------------------------------------------------
    // ERC-4337 helpers
    // ------------------------------------------------------------------------------------------------------------

    function _userOp(address sender, bytes memory callData)
        internal
        view
        returns (PackedUserOperation memory op)
    {
        op.sender = sender;
        op.nonce = ep.getNonce(sender, 0);
        op.callData = callData;
        op.accountGasLimits = bytes32((uint256(3_000_000) << 128) | uint256(1_000_000));
        op.preVerificationGas = 100_000;
        op.gasFees = bytes32((uint256(1 gwei) << 128) | uint256(1 gwei));
    }

    function _handle(PackedUserOperation memory op) internal {
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        ep.handleOps(ops, payable(bundler));
    }

    function _ecdsaSign(uint256 pk, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return abi.encodePacked(r, s, v);
    }

    function _signOpEcdsa(PackedUserOperation memory op, uint256 pk)
        internal
        view
        returns (PackedUserOperation memory)
    {
        op.signature = _ecdsaSign(pk, ep.getUserOpHash(op));
        return op;
    }

    // ------------------------------------------------------------------------------------------------------------
    // WebAuthn / passkey helpers (real P-256 signatures via vm.signP256)
    // ------------------------------------------------------------------------------------------------------------

    function _webAuthn(uint256 pk, bytes32 challenge, bytes1 flags)
        internal
        pure
        returns (WebAuthn.WebAuthnAuth memory auth)
    {
        auth.authenticatorData = abi.encodePacked(sha256("wallet.xdevs.dev"), flags, uint32(1));
        auth.clientDataJSON = string.concat(
            '{"type":"webauthn.get","challenge":"',
            Base64.encodeURL(abi.encodePacked(challenge)),
            '","origin":"https://wallet.xdevs.dev","crossOrigin":false}'
        );
        auth.typeIndex = 1;
        auth.challengeIndex = 23;
        bytes32 digest = sha256(abi.encodePacked(auth.authenticatorData, sha256(bytes(auth.clientDataJSON))));
        (bytes32 r, bytes32 s) = vm.signP256(pk, digest);
        if (uint256(s) > P256_N / 2) s = bytes32(P256_N - uint256(s)); // low-s
        auth.r = r;
        auth.s = s;
    }

    function _passkeySig(bytes32 challenge) internal view returns (bytes memory) {
        return _encodePasskeySig(_webAuthn(passkeyPk, challenge, 0x05));
    }

    function _encodePasskeySig(WebAuthn.WebAuthnAuth memory auth) internal view returns (bytes memory) {
        return abi.encodePacked(uint8(0x02), abi.encode(passkeyX, passkeyY, auth));
    }

    function _signOpPasskey(PackedUserOperation memory op)
        internal
        view
        returns (PackedUserOperation memory)
    {
        op.signature = _passkeySig(ep.getUserOpHash(op));
        return op;
    }

    // ------------------------------------------------------------------------------------------------------------
    // Misc
    // ------------------------------------------------------------------------------------------------------------

    function _execData(address target, uint256 value, bytes memory data)
        internal
        pure
        returns (bytes memory)
    {
        return abi.encodeWithSignature("execute(address,uint256,bytes)", target, value, data);
    }

    function _executeUserOpData(Call[] memory calls) internal pure returns (bytes memory) {
        return abi.encodePacked(bytes4(0x8dd7712f), abi.encode(calls)); // IAccountExecute.executeUserOp.selector
    }

    function _call(address target, uint256 value, bytes memory data) internal pure returns (Call memory) {
        return Call(target, value, data);
    }

    function _calls1(Call memory c) internal pure returns (Call[] memory arr) {
        arr = new Call[](1);
        arr[0] = c;
    }

    function _ethSigned(bytes32 h) internal pure returns (bytes32) {
        return MessageHashUtils.toEthSignedMessageHash(h);
    }
}
