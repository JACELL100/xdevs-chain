// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {WebAuthn} from "@openzeppelin/contracts/utils/cryptography/WebAuthn.sol";

import {AccountCore} from "./AccountCore.sol";
import {AccountStorage} from "../libraries/AccountStorage.sol";
import {SignerType} from "../libraries/AccountTypes.sol";
import {WebAuthnP256} from "../libraries/WebAuthnP256.sol";

/// @title SignatureVerifier
/// @notice Owner-signature verification for secp256k1 signers and P-256 WebAuthn passkeys (plan §16.1, §10.3).
/// @dev Owner signature encodings:
///        * 65 bytes                                  : raw secp256k1 `r || s || v`
///        * 0x01 || r || s || v                       : secp256k1 (explicit type)
///        * 0x02 || abi.encode(x, y, WebAuthnAuth)    : passkey; (x, y) must be a registered passkey signer and
///                                                      `WebAuthnAuth` is OpenZeppelin's struct
///      Session-key signatures (0x03) are handled in `validateUserOp`.
abstract contract SignatureVerifier is AccountCore {
    uint8 internal constant SIG_TYPE_ECDSA = 0x01;
    uint8 internal constant SIG_TYPE_PASSKEY = 0x02;
    uint8 internal constant SIG_TYPE_SESSION = 0x03;

    /// @dev Verifies an owner signature over `hash`. Returns false on a wrong signature; may revert on a malformed
    ///      passkey payload.
    function _verifyOwnerSignature(bytes32 hash, bytes calldata sig) internal view returns (bool) {
        if (sig.length == 65) return _verifyEcdsa(hash, sig);
        if (sig.length == 0) return false;
        uint8 t = uint8(sig[0]);
        if (t == SIG_TYPE_ECDSA) return sig.length == 66 && _verifyEcdsa(hash, sig[1:]);
        if (t == SIG_TYPE_PASSKEY) return _verifyPasskey(hash, sig[1:]);
        return false;
    }

    /// @dev The account address itself is an implicit secp256k1 signer: for an EIP-7702 delegated EOA this is the
    ///      EOA key (all-powerful at the protocol level anyway); for a CREATE2 proxy nobody holds such a key.
    function _verifyEcdsa(bytes32 hash, bytes calldata sig) internal view returns (bool) {
        (address rec, ECDSA.RecoverError err,) = ECDSA.tryRecoverCalldata(hash, sig);
        if (err != ECDSA.RecoverError.NoError) return false;
        return rec == address(this) || _isEcdsaSigner(rec);
    }

    function _verifyPasskey(bytes32 hash, bytes calldata payload) internal view returns (bool) {
        (bytes32 x, bytes32 y, WebAuthn.WebAuthnAuth memory auth) =
            abi.decode(payload, (bytes32, bytes32, WebAuthn.WebAuthnAuth));
        if (AccountStorage.signers().signers[keccak256(abi.encode(x, y))].kind != SignerType.Passkey) {
            return false;
        }
        // Challenge = the 32-byte hash; the authenticator must report User Present + User Verified.
        return WebAuthnP256.verify(abi.encodePacked(hash), auth, x, y);
    }
}
