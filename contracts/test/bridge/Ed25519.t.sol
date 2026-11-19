// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Ed25519} from "../../src/bridge/lib/Ed25519.sol";
import {Sha512} from "../../src/bridge/lib/Sha512.sol";

contract Ed25519Harness {
    function verify(bytes32 pk, bytes32 r, bytes32 s, bytes calldata m) external view returns (bool) {
        return Ed25519.verify(pk, r, s, m);
    }

    function sha512(bytes calldata m) external pure returns (bytes memory) {
        (bytes32 hi, bytes32 lo) = Sha512.hash(m);
        return abi.encodePacked(hi, lo);
    }
}

contract Ed25519Test is Test {
    struct Vector {
        bytes message;
        bytes32 pubkey;
        bytes32 r;
        bytes32 s;
        bytes sha512;
    }

    Ed25519Harness h;

    function setUp() public {
        h = new Ed25519Harness();
    }

    function _vectors() internal view returns (Vector[] memory) {
        string memory json = vm.readFile("test/bridge/fixtures/ed25519-vectors.json");
        return abi.decode(
            vm.parseJsonTypeArray(
                json, ".vectors", "Vector(bytes message,bytes32 pubkey,bytes32 r,bytes32 s,bytes sha512)"
            ),
            (Vector[])
        );
    }

    function test_sha512_nist() public view {
        assertEq(
            h.sha512("abc"),
            hex"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assertEq(
            h.sha512(""),
            hex"cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    function test_sha512_vectors() public view {
        Vector[] memory v = _vectors();
        for (uint256 i; i < v.length; ++i) {
            assertEq(h.sha512(v[i].message), v[i].sha512);
        }
    }

    /// RFC 8032 §7.1 tests 1 and 2.
    function test_rfc8032() public view {
        assertTrue(
            h.verify(
                0xd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a,
                0xe5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155,
                0x5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b,
                ""
            )
        );
        assertTrue(
            h.verify(
                0x3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c,
                0x92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da,
                0x085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00,
                hex"72"
            )
        );
    }

    function test_vectors_and_tampering() public view {
        Vector[] memory v = _vectors();
        for (uint256 i; i < v.length; ++i) {
            assertTrue(h.verify(v[i].pubkey, v[i].r, v[i].s, v[i].message), "valid");
            // flipped message bit, wrong R, wrong S, wrong key
            bytes memory m = bytes.concat(v[i].message, hex"00");
            assertFalse(h.verify(v[i].pubkey, v[i].r, v[i].s, m), "message");
            assertFalse(h.verify(v[i].pubkey, v[i].r ^ bytes32(uint256(1)), v[i].s, v[i].message), "R");
            assertFalse(h.verify(v[i].pubkey, v[i].r, v[i].s ^ bytes32(uint256(1 << 8)), v[i].message), "S");
            assertFalse(h.verify(v[(i + 1) % v.length].pubkey, v[i].r, v[i].s, v[i].message), "key");
        }
    }

    function test_rejects_s_not_reduced() public view {
        Vector[] memory v = _vectors();
        // S + L (little-endian) encodes the same scalar but is not canonical: rejected (malleability).
        uint256 s = _le(v[0].s) + Ed25519.L;
        assertFalse(h.verify(v[0].pubkey, v[0].r, bytes32(_le(bytes32(s))), v[0].message));
    }

    function test_gas_one_signature() public {
        Vector[] memory v = _vectors();
        uint256 g = gasleft();
        h.verify(v[4].pubkey, v[4].r, v[4].s, v[4].message); // 66-byte message, like a vote
        emit log_named_uint("Ed25519.verify gas (66-byte message)", g - gasleft());
    }

    function _le(bytes32 x) internal pure returns (uint256 r) {
        for (uint256 i; i < 32; ++i) {
            r |= uint256(uint8(x[i])) << (8 * i);
        }
    }
}
