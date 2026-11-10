// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Base64} from "@openzeppelin/contracts/utils/Base64.sol";
import {WebAuthn} from "@openzeppelin/contracts/utils/cryptography/WebAuthn.sol";

/// @title WebAuthnP256
/// @notice WebAuthn assertion verification (W3C WebAuthn Level 2, §7.2 "Verifying an Authentication Assertion")
///         using only the `P256VERIFY` precompile at 0x100 (RIP-7212 / EIP-7951, plan §10.3).
/// @dev Same checks as OpenZeppelin's `WebAuthn.verify(..., requireUV = true)` (and same `WebAuthnAuth` encoding),
///      but without OpenZeppelin's pure-Solidity P-256 fallback, which would push the account implementation over
///      the EIP-170 code-size limit. xDEVS Chain always has the precompile; on a chain without it every passkey
///      signature is simply invalid (fail closed — the call to an empty account returns no data).
///
///      Verified:
///        * step 11: `"type":"webauthn.get"` at `typeIndex` of clientDataJSON
///        * step 12: `"challenge":"<base64url(challenge)>"` at `challengeIndex` of clientDataJSON
///        * step 16/17: User Present and User Verified flags set in authenticatorData
///        * BE/BS consistency (backup state requires backup eligibility)
///        * step 20: P-256 signature over `sha256(authenticatorData || sha256(clientDataJSON))`, low-s only
///      Not verified on-chain (authenticator / platform responsibility): origin, rpIdHash, signature counter,
///      extensions — see OpenZeppelin's WebAuthn notes.
library WebAuthnP256 {
    address internal constant P256_VERIFY = 0x0000000000000000000000000000000000000100;

    /// @dev secp256r1 group order and N/2 (malleability bound).
    uint256 internal constant N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551;
    uint256 internal constant HALF_N = 0x7fffffff800000007fffffffffffffffde737d56d38bcf4279dce5617e3192a8;

    function verify(bytes memory challenge, WebAuthn.WebAuthnAuth memory auth, bytes32 qx, bytes32 qy)
        internal
        view
        returns (bool)
    {
        bytes memory authData = auth.authenticatorData;
        if (authData.length < 37) return false;
        bytes1 flags = authData[32];
        if (flags & 0x01 == 0 || flags & 0x04 == 0) return false; // UP and UV
        if (flags & 0x08 == 0 && flags & 0x10 != 0) return false; // BS requires BE

        bytes memory clientData = bytes(auth.clientDataJSON);
        if (!_matchesAt(clientData, auth.typeIndex, bytes('"type":"webauthn.get"'))) return false;
        bytes memory expectedChallenge =
            bytes(string.concat('"challenge":"', Base64.encodeURL(challenge), '"'));
        if (!_matchesAt(clientData, auth.challengeIndex, expectedChallenge)) return false;

        bytes32 digest = sha256(abi.encodePacked(authData, sha256(clientData)));
        return verifySignature(digest, auth.r, auth.s, qx, qy);
    }

    /// @notice Raw P-256 verification through the precompile, rejecting high-s and out-of-range scalars.
    function verifySignature(bytes32 h, bytes32 r, bytes32 s, bytes32 qx, bytes32 qy)
        internal
        view
        returns (bool ok)
    {
        if (uint256(r) == 0 || uint256(r) >= N || uint256(s) == 0 || uint256(s) > HALF_N) return false;
        address precompile = P256_VERIFY;
        assembly ("memory-safe") {
            let ptr := mload(0x40)
            mstore(ptr, h)
            mstore(add(ptr, 0x20), r)
            mstore(add(ptr, 0x40), s)
            mstore(add(ptr, 0x60), qx)
            mstore(add(ptr, 0x80), qy)
            mstore(0x00, 0)
            let success := staticcall(gas(), precompile, ptr, 0xa0, 0x00, 0x20)
            ok := and(success, and(eq(returndatasize(), 0x20), eq(mload(0x00), 1)))
        }
    }

    /// @dev True if `data[index : index + expected.length] == expected`.
    function _matchesAt(bytes memory data, uint256 index, bytes memory expected)
        private
        pure
        returns (bool matches)
    {
        uint256 len = expected.length;
        if (index > data.length || data.length - index < len) return false;
        assembly ("memory-safe") {
            matches := eq(keccak256(add(add(data, 0x20), index), len), keccak256(add(expected, 0x20), len))
        }
    }
}
