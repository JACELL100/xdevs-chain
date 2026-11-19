// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title BLS12381 - Ethereum BLS signature verification over the EIP-2537 precompiles (bridge prototype)
/// @notice Verifies `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_` signatures (public keys in G1, signatures in
///         G2), the scheme of Ethereum's sync committee. Points use the EIP-2537 encoding: an Fp element is
///         64 bytes (16 zero bytes ‖ 48-byte big-endian value); a {G1} is (x, y) = four words, a G2 point is
///         eight words (x.c0, x.c1, y.c0, y.c1). Precompiles: G1ADD 0x0b, G2ADD 0x0d, PAIRING_CHECK 0x0f,
///         MAP_FP2_TO_G2 0x11 (all live on Ethereum since Prague and on xDEVS Chain, which runs Osaka rules),
///         SHA-256 0x02 and MODEXP 0x05.
library BLS12381 {
    /// @dev x = (xHi ‖ xLo): `xHi` holds the top 16 bytes in its low half, as in the EIP-2537 encoding.
    struct G1 {
        uint256 xHi;
        uint256 xLo;
        uint256 yHi;
        uint256 yLo;
    }

    uint256 internal constant P_HI = 0x1a0111ea397fe69a4b1ba7b6434bacd7;
    uint256 internal constant P_LO = 0x64774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab;
    uint256 internal constant HALF_HI = 0x0d0088f51cbff34d258dd3db21a5d66b;
    uint256 internal constant HALF_LO = 0xb23ba5c279c2895fb39869507b587b120f55ffff58a9ffffdcff7fffffffd555;
    // -G1 generator (x, p - y), for the pairing check.
    uint256 internal constant NG1_X_HI = 0x17f1d3a73197d7942695638c4fa9ac0f;
    uint256 internal constant NG1_X_LO = 0xc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb;
    uint256 internal constant NG1_Y_HI = 0x114d1d6855d545a8aa7d76c8cf2e21f2;
    uint256 internal constant NG1_Y_LO = 0x67816aef1db507c96655b9d5caac42364e6f38ba0ecb751bad54dcd6b939c2ca;

    error PrecompileFailed(uint256 precompile);
    error NotFieldElement();

    // ----------------------------------------------------------------------------------------------------------
    // G1
    // ----------------------------------------------------------------------------------------------------------

    /// @notice a + b (reverts if either point is not on the curve, which G1ADD checks).
    function add(G1 memory a, G1 memory b) internal view returns (G1 memory r) {
        bool ok;
        assembly ("memory-safe") {
            let m := mload(0x40)
            mcopy(m, a, 0x80)
            mcopy(add(m, 0x80), b, 0x80)
            ok := staticcall(gas(), 0x0b, m, 0x100, r, 0x80)
        }
        if (!ok) revert PrecompileFailed(0x0b);
    }

    /// @notice -a.
    function neg(G1 memory a) internal pure returns (G1 memory r) {
        if (a.yHi == 0 && a.yLo == 0) return a; // the point at infinity (0, 0)
        if (!lt(a.yHi, a.yLo, P_HI, P_LO)) revert NotFieldElement();
        r.xHi = a.xHi;
        r.xLo = a.xLo;
        unchecked {
            uint256 borrow = P_LO < a.yLo ? 1 : 0;
            r.yLo = P_LO - a.yLo;
            r.yHi = P_HI - a.yHi - borrow;
        }
    }

    /// @notice The 48-byte ZCash/Ethereum compressed encoding of an affine G1 point, as two words
    ///         (`w0` = bytes 0..31, `w1` = bytes 32..47 followed by 16 zero bytes): flag bits
    ///         0x80 (compressed) and 0x20 (y is the larger root) in the first byte. Infinity is never a valid
    ///         sync-committee key and is not handled.
    function compress(G1 memory a) internal pure returns (bytes32 w0, bytes32 w1) {
        if (!lt(a.xHi, a.xLo, P_HI, P_LO)) revert NotFieldElement();
        uint256 flags = 0x80 | (lt(HALF_HI, HALF_LO, a.yHi, a.yLo) ? 0x20 : 0);
        w0 = bytes32((a.xHi << 128) | (a.xLo >> 128) | (flags << 248));
        w1 = bytes32(a.xLo << 128);
    }

    /// @dev (aHi, aLo) < (bHi, bLo)
    function lt(uint256 aHi, uint256 aLo, uint256 bHi, uint256 bLo) internal pure returns (bool) {
        return aHi < bHi || (aHi == bHi && aLo < bLo);
    }

    // ----------------------------------------------------------------------------------------------------------
    // Signatures
    // ----------------------------------------------------------------------------------------------------------

    /// @notice e(pk, H(msg)) == e(G1, sig) for the POP ciphersuite with `dst`, where `sig` is an uncompressed G2
    ///         point. The pairing precompile checks that `pk` and `sig` are on their curves and in the subgroups.
    function verify(G1 memory pk, bytes32 message, uint256[8] memory sig, bytes memory dst)
        internal
        view
        returns (bool)
    {
        uint256[8] memory h = hashToG2(message, dst);
        uint256[24] memory input;
        input[0] = pk.xHi;
        input[1] = pk.xLo;
        input[2] = pk.yHi;
        input[3] = pk.yLo;
        for (uint256 i; i < 8; ++i) {
            input[4 + i] = h[i];
            input[16 + i] = sig[i];
        }
        input[12] = NG1_X_HI;
        input[13] = NG1_X_LO;
        input[14] = NG1_Y_HI;
        input[15] = NG1_Y_LO;
        uint256[1] memory out;
        bool ok;
        assembly ("memory-safe") {
            ok := staticcall(gas(), 0x0f, input, 0x300, out, 0x20)
        }
        return ok && out[0] == 1;
    }

    /// @notice hash_to_curve for G2 (RFC 9380, BLS12381G2_XMD:SHA-256_SSWU_RO_) of a 32-byte message.
    function hashToG2(bytes32 message, bytes memory dst) internal view returns (uint256[8] memory r) {
        uint256[8] memory u = _hashToField(message, dst);
        uint256[8] memory q0 = _mapToG2(u[0], u[1], u[2], u[3]);
        uint256[8] memory q1 = _mapToG2(u[4], u[5], u[6], u[7]);
        bool ok;
        assembly ("memory-safe") {
            let m := mload(0x40)
            mcopy(m, q0, 0x100)
            mcopy(add(m, 0x100), q1, 0x100)
            ok := staticcall(gas(), 0x0d, m, 0x200, r, 0x100)
        }
        if (!ok) revert PrecompileFailed(0x0d);
    }

    /// @dev Two Fp2 elements (u0.c0, u0.c1, u1.c0, u1.c1), each as (hi, lo) words.
    function _hashToField(bytes32 message, bytes memory dst) private view returns (uint256[8] memory u) {
        bytes memory dstPrime = abi.encodePacked(dst, uint8(dst.length));
        bytes32 b0 =
            sha256(abi.encodePacked(bytes32(0), bytes32(0), message, uint16(256), uint8(0), dstPrime));
        bytes32 bi = sha256(abi.encodePacked(b0, uint8(1), dstPrime));
        bytes32[8] memory uniform;
        uniform[0] = bi;
        for (uint256 i = 1; i < 8; ++i) {
            bi = sha256(abi.encodePacked(b0 ^ bi, uint8(i + 1), dstPrime));
            uniform[i] = bi;
        }
        for (uint256 j; j < 4; ++j) {
            (u[2 * j], u[2 * j + 1]) = _modP(uniform[2 * j], uniform[2 * j + 1]);
        }
    }

    /// @dev (a ‖ b) mod p for a 64-byte big-endian value, via MODEXP with exponent 1.
    function _modP(bytes32 a, bytes32 b) private view returns (uint256 hi, uint256 lo) {
        bool ok;
        assembly ("memory-safe") {
            let m := mload(0x40)
            mstore(m, 64)
            mstore(add(m, 0x20), 1)
            mstore(add(m, 0x40), 48)
            mstore(add(m, 0x60), a)
            mstore(add(m, 0x80), b)
            mstore8(add(m, 0xa0), 1)
            mstore(add(m, 0xa1), shl(128, P_HI))
            mstore(add(m, 0xb1), P_LO)
            ok := staticcall(gas(), 0x05, m, 0xd1, m, 48)
            let w0 := mload(m)
            let w1 := mload(add(m, 0x20))
            hi := shr(128, w0)
            lo := or(shl(128, w0), shr(128, w1))
        }
        if (!ok) revert PrecompileFailed(0x05);
    }

    function _mapToG2(uint256 c0Hi, uint256 c0Lo, uint256 c1Hi, uint256 c1Lo)
        private
        view
        returns (uint256[8] memory r)
    {
        uint256[4] memory input = [c0Hi, c0Lo, c1Hi, c1Lo];
        bool ok;
        assembly ("memory-safe") {
            ok := staticcall(gas(), 0x11, input, 0x80, r, 0x100)
        }
        if (!ok) revert PrecompileFailed(0x11);
    }
}
