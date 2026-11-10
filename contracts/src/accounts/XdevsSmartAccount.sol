// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {IAccount} from "account-abstraction/interfaces/IAccount.sol";
import {IAccountExecute} from "account-abstraction/interfaces/IAccountExecute.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {
    _packValidationData,
    SIG_VALIDATION_FAILED,
    SIG_VALIDATION_SUCCESS
} from "account-abstraction/core/Helpers.sol";

import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";
import {IERC165} from "@openzeppelin/contracts/utils/introspection/IERC165.sol";
import {IERC721Receiver} from "@openzeppelin/contracts/token/ERC721/IERC721Receiver.sol";
import {IERC1155Receiver} from "@openzeppelin/contracts/token/ERC1155/IERC1155Receiver.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

import {SignatureVerifier} from "./base/SignatureVerifier.sol";
import {VaultDispatcher} from "./base/VaultDispatcher.sol";
import {SessionExecutor} from "./base/SessionExecutor.sol";
import {IGuardianVault} from "./interfaces/IGuardianVault.sol";
import {IAccountExtension} from "./interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "./libraries/AccountConstants.sol";
import {AccountStorage} from "./libraries/AccountStorage.sol";
import {Call} from "./libraries/AccountTypes.sol";

/// @title XdevsSmartAccount
/// @notice xDEVS Chain smart account (plan §16.1): multiple secp256k1 / passkey signers, social recovery, session
///         keys and the Guardian Vault policy. ERC-4337 v0.9 account, ERC-1271 signer, EIP-7702 delegation target.
/// @dev Predeployed (runtime code only, no constructor) at `Predeploys.SMART_ACCOUNT_IMPL`.
///      Usage modes:
///      * EIP-7702: an existing EOA delegates to the implementation and keeps its address. The EOA key is an
///        implicit signer; the EOA calls `initialize` on itself (or anyone relays `initializeWithSignature`).
///      * Factory: `SmartAccountFactory` deploys an ERC-1967 proxy with CREATE2 and initializes it in the proxy
///        constructor (passkey-only accounts supported).
///      This contract is the *hot path* (validation, execution, vault dispatch / queue, sessions, ERC-1271). The
///      cold path (initialization, signer / guardian / policy / session management, recovery, views, upgrades) is
///      `AccountExtension`, reached through `fallback()` with DELEGATECALL into `AccountConstants.ACCOUNT_EXTENSION`
///      (the factory predeploy). Both share the ERC-7201 storage in `AccountStorage`, so state never collides with
///      storage an EOA may have written under a previous delegation.
contract XdevsSmartAccount is
    SignatureVerifier,
    VaultDispatcher,
    SessionExecutor,
    IAccount,
    IAccountExecute,
    IERC1271,
    IERC165,
    IERC721Receiver,
    IERC1155Receiver
{
    string public constant VERSION = "1.0.0";

    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant MESSAGE_TYPEHASH = keccak256("XdevsAccountMessage(bytes32 hash)");

    /// @dev Only relevant for non-genesis deployments of the implementation: locks the implementation's own
    ///      storage. The genesis copy is locked by an address check in `AccountExtension.initialize`.
    constructor() {
        AccountStorage.core().initialized = true;
    }

    // ============================================================================================================
    // ERC-4337
    // ============================================================================================================

    function entryPoint() public pure returns (IEntryPoint) {
        return IEntryPoint(AccountConstants.ENTRY_POINT);
    }

    function getNonce() external view returns (uint256) {
        return entryPoint().getNonce(address(this), 0);
    }

    /// @notice Deposit (anyone) to the EntryPoint for this account's gas.
    function addDeposit() external payable {
        entryPoint().depositTo{value: msg.value}(address(this));
    }

    function getDeposit() external view returns (uint256) {
        return entryPoint().balanceOf(address(this));
    }

    /// @inheritdoc IAccount
    /// @dev Signatures: an owner signature over `userOpHash` (see `SignatureVerifier`), or a session key:
    ///      `0x03 || sessionKey (20 bytes) || r || s || v`, only valid when `callData` is
    ///      `executeUserOp.selector || abi.encode(Call[])` and every call is permitted for the key; the session's
    ///      validity window is returned in `validationData` and caps are enforced at execution. With the vault
    ///      enabled, prefunds are charged to the daily gas budget.
    function validateUserOp(
        PackedUserOperation calldata userOp,
        bytes32 userOpHash,
        uint256 missingAccountFunds
    ) external returns (uint256 validationData) {
        if (msg.sender != AccountConstants.ENTRY_POINT) revert Unauthorized();
        validationData = _validateUserOpSignature(userOp, userOpHash);
        if (missingAccountFunds != 0) {
            // Over budget => report failure (the EntryPoint then reverts the whole bundle, refunding the prefund).
            if (!_chargeGasBudget(missingAccountFunds)) validationData = SIG_VALIDATION_FAILED;
            // Prefund is paid even on signature failure, as ERC-4337 requires (gas estimation with dummy sigs).
            (bool ok,) = payable(msg.sender).call{value: missingAccountFunds}("");
            (ok); // the EntryPoint verifies the deposit
        }
    }

    function _validateUserOpSignature(PackedUserOperation calldata userOp, bytes32 userOpHash)
        private
        view
        returns (uint256)
    {
        bytes calldata sig = userOp.signature;
        // Dispatch on the exact length first: a raw 65-byte ECDSA signature may start with any byte (incl. 0x03).
        if (sig.length == 86 && uint8(sig[0]) == SIG_TYPE_SESSION) {
            if (userOp.callData.length < 4) return SIG_VALIDATION_FAILED;
            if (bytes4(userOp.callData[:4]) != IAccountExecute.executeUserOp.selector) {
                return SIG_VALIDATION_FAILED;
            }
            address key = address(bytes20(sig[1:21]));
            (address rec, ECDSA.RecoverError err,) = ECDSA.tryRecoverCalldata(userOpHash, sig[21:]);
            if (err != ECDSA.RecoverError.NoError || rec != key) return SIG_VALIDATION_FAILED;
            Call[] memory calls = abi.decode(userOp.callData[4:], (Call[]));
            (bool ok, uint48 validAfter, uint48 validUntil) = _checkSessionCalls(key, calls);
            if (!ok) return SIG_VALIDATION_FAILED;
            return _packValidationData(false, validUntil, validAfter);
        }
        return _verifyOwnerSignature(userOpHash, sig) ? SIG_VALIDATION_SUCCESS : SIG_VALIDATION_FAILED;
    }

    /// @inheritdoc IAccountExecute
    /// @dev `userOp.callData = executeUserOp.selector || abi.encode(Call[])`. The EntryPoint already validated the
    ///      signature of this exact UserOperation; here it only selects owner vs session-key authority.
    function executeUserOp(PackedUserOperation calldata userOp, bytes32) external {
        if (msg.sender != AccountConstants.ENTRY_POINT) revert Unauthorized();
        Call[] memory calls = abi.decode(userOp.callData[4:], (Call[]));
        bytes calldata sig = userOp.signature;
        if (sig.length == 86 && uint8(sig[0]) == SIG_TYPE_SESSION) {
            _executeSession(address(bytes20(sig[1:21])), calls);
        } else {
            _dispatch(calls, false);
        }
    }

    // ============================================================================================================
    // Execution
    // ============================================================================================================

    /// @notice Executes (or queues, per the vault policy) one call. Owner authority.
    /// @return queuedId Vault action id if the call was queued, 0 if executed instantly.
    function execute(address target, uint256 value, bytes calldata data)
        external
        onlyOwner
        returns (uint256)
    {
        Call[] memory calls = new Call[](1);
        calls[0] = Call(target, value, data);
        return _dispatch(calls, false);
    }

    /// @notice Executes (or queues atomically) a batch of calls. Owner authority.
    function executeBatch(Call[] calldata calls) external onlyOwner returns (uint256) {
        return _dispatch(calls, false);
    }

    /// @notice Direct session-key execution (`msg.sender` is the session key and pays its own gas).
    function executeWithSession(Call[] calldata calls) external {
        _executeSession(msg.sender, calls);
    }

    /// @inheritdoc IGuardianVault
    /// @dev Served by the extension (delayed when the vault is enabled).
    function setPolicy(uint256, uint64, address[] calldata) external {
        _delegateToExtension();
    }

    function _routeSelfCall(bytes memory data) internal override {
        Call[] memory calls = new Call[](1);
        calls[0] = Call(address(this), 0, data);
        _dispatch(calls, false);
    }

    function _dispatchSession(Call[] memory calls) internal override {
        _dispatch(calls, true);
    }

    /// @dev Self-calls executed instantly with the vault enabled: only actions that cannot weaken protection
    ///      (adding keys / sessions, revocations, cancellations). Unknown selectors are delayed (fail-safe).
    function _isInstantSelfSelector(bytes4 selector) internal pure override returns (bool) {
        return selector == IAccountExtension.addSigner.selector
            || selector == IAccountExtension.addSessionKey.selector
            || selector == IAccountExtension.revokeSessionKey.selector
            || selector == IAccountExtension.revokeAllSessionKeys.selector
            || selector == IGuardianVault.cancel.selector
            || selector == IAccountExtension.cancelRecovery.selector
            || selector == IAccountExtension.revokeHash.selector
            || selector == VaultDispatcher.pokeGasBudget.selector;
    }

    // ============================================================================================================
    // ERC-1271
    // ============================================================================================================

    /// @notice Account-bound hash that signers sign for ERC-1271 (prevents replay across accounts sharing a key).
    function replaySafeHash(bytes32 hash) public view returns (bytes32) {
        bytes32 domainSeparator = keccak256(
            abi.encode(
                DOMAIN_TYPEHASH,
                keccak256("xDEVS Smart Account"),
                keccak256("1"),
                block.chainid,
                address(this)
            )
        );
        return
            MessageHashUtils.toTypedDataHash(domainSeparator, keccak256(abi.encode(MESSAGE_TYPEHASH, hash)));
    }

    /// @inheritdoc IERC1271
    /// @dev Accepts a hash pre-approved on-chain (`approveHash`, delayed by the vault). Otherwise — only if the vault
    ///      is disabled or `allowInstantSignatures` is set — an owner signature over `replaySafeHash(hash)`, or
    ///      (EIP-7702 compatibility) a raw 65-byte signature of the account's own EOA key over `hash`.
    function isValidSignature(bytes32 hash, bytes calldata signature) external view returns (bytes4) {
        AccountStorage.Vault storage $ = AccountStorage.vault();
        if ($.approvedHashes[hash]) return IERC1271.isValidSignature.selector;
        if ($.policy.enabled && !$.policy.allowInstantSignatures) return 0xffffffff;
        if (_verifyOwnerSignature(replaySafeHash(hash), signature)) {
            return IERC1271.isValidSignature.selector;
        }
        if (signature.length == 65) {
            (address rec, ECDSA.RecoverError err,) = ECDSA.tryRecoverCalldata(hash, signature);
            if (err == ECDSA.RecoverError.NoError && rec == address(this)) {
                return IERC1271.isValidSignature.selector;
            }
        }
        return 0xffffffff;
    }

    // ============================================================================================================
    // Receiving assets / ERC-165 / extension
    // ============================================================================================================

    receive() external payable {}

    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return IERC721Receiver.onERC721Received.selector;
    }

    function onERC1155Received(address, address, uint256, uint256, bytes calldata)
        external
        pure
        returns (bytes4)
    {
        return IERC1155Receiver.onERC1155Received.selector;
    }

    function onERC1155BatchReceived(address, address, uint256[] calldata, uint256[] calldata, bytes calldata)
        external
        pure
        returns (bytes4)
    {
        return IERC1155Receiver.onERC1155BatchReceived.selector;
    }

    function supportsInterface(bytes4 interfaceId) external pure returns (bool) {
        return interfaceId == type(IERC165).interfaceId || interfaceId == type(IAccount).interfaceId
            || interfaceId == type(IAccountExecute).interfaceId || interfaceId == type(IERC1271).interfaceId
            || interfaceId == type(IERC721Receiver).interfaceId
            || interfaceId == type(IERC1155Receiver).interfaceId
            || interfaceId == type(IGuardianVault).interfaceId;
    }

    /// @notice Every other selector is served by the extension (DELEGATECALL, same storage, same msg.sender).
    fallback() external {
        _delegateToExtension();
    }

    function _delegateToExtension() private {
        address ext = AccountConstants.ACCOUNT_EXTENSION;
        assembly {
            calldatacopy(0, 0, calldatasize())
            let ok := delegatecall(gas(), ext, 0, calldatasize(), 0, 0)
            returndatacopy(0, 0, returndatasize())
            if iszero(ok) { revert(0, returndatasize()) }
            return(0, returndatasize())
        }
    }
}
