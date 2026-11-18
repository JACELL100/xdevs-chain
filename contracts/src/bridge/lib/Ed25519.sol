// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Sha512} from "./Sha512.sol";

/// @title Ed25519 - RFC 8032 Ed25519 signature verification in Solidity (bridge prototype)
/// @notice For chains without an Ed25519 precompile (Ethereum). Verifies the cofactorless equation
///         `encode([S]B - [k]A) == R` with `k = SHA-512(R ‖ A ‖ M) mod L`, rejecting `S >= L` and public keys that
///         do not decode to a curve point. Arithmetic: extended twisted-Edwards coordinates (HWCD 2008, a = -1),
///         Shamir's trick for the double-scalar multiplication, the MODEXP precompile (0x05) for the inversion
///         and the square root. Cost ≈ 250-300k gas per signature (measured in `test/bridge/Ed25519.t.sol`).
/// @dev    Honest signatures verify identically under this rule, ed25519-dalek's `verify_strict` and the ZIP-215
///         rule of `ed25519-consensus` (which xDEVS Chain consensus uses). They differ only on adversarial edge
///         cases (small-order keys / non-canonical encodings), where this library is the stricter one: it can
///         reject such a signature that consensus accepted (liveness only), never the reverse. Not audited.
library Ed25519 {
    uint256 internal constant P = 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffed;
    uint256 internal constant L = 0x1000000000000000000000000000000014def9dea2f79cd65812631a5cf5d3ed;
    uint256 internal constant D = 0x52036cee2b6ffe738cc740797779e89800700a4d4141d8ab75eb4dca135978a3;
    uint256 internal constant D2 = 0x2406d9dc56dffce7198e80f2eef3d13000e0149a8283b156ebd69b9426b2f159;
    uint256 internal constant SQRT_M1 = 0x2b8324804fc1df0b2b4d00993dfbd7a72f431806ad2fe478c4ee1b274a0ea0b0;
    // Base point B, cached as (y - x, y + x, 2d·x·y) for the mixed addition.
    uint256 internal constant B_X = 0x216936d3cd6e53fec0a4e231fdd6dc5c692cc7609525a7b2c9562d608f25d51a;
    uint256 internal constant B_Y = 0x6666666666666666666666666666666666666666666666666666666666666658;
    uint256 internal constant B_YMX = 0x44fd2f9298f81267a5c18434688f8a09fd399f05d140beb39d103905d740913e;
    uint256 internal constant B_YPX = 0x07cf9d3a33d4ba65270b4898643d42c2cf932dc6fb8c0e192fbc93c6f58c3b85;
    uint256 internal constant B_T2D = 0x6f117b689f0c65a85a1b7dcbdd43598c26d9e823ccaac49eabc91205877aaa68;
    /// @dev 2^256 mod L, to reduce the 512-bit hash.
    uint256 internal constant R256 = 0x0ffffffffffffffffffffffffffffffec6ef5bf4737dcf70d6ec31748d98951d;

    /// @notice True iff `sig = R ‖ S` is a valid Ed25519 signature of `message` under the 32-byte key `pubkey`.
    function verify(bytes32 pubkey, bytes32 r, bytes32 s, bytes memory message) internal view returns (bool) {
        uint256 sInt = _le(s);
        if (sInt >= L) return false;
        (bool ok, uint256 ax, uint256 ay) = decompress(pubkey);
        if (!ok) return false;
        (bytes32 hHi, bytes32 hLo) = Sha512.hash(abi.encodePacked(r, pubkey, message));
        // The digest is a 512-bit little-endian integer: bytes 0..31 are the low word.
        uint256 k = _reduce(hHi, hLo);
        // Q = [s]B + [k](-A)
        (uint256 qx, uint256 qy) = _doubleScalarMul(sInt, k, (P - ax) % P, ay);
        return bytes32(_reverse(qy | ((qx & 1) << 255))) == r;
    }

    /// @notice Decodes a 32-byte Ed25519 point encoding (RFC 8032 §5.1.3) with the permissive rules of
    ///         curve25519-dalek / ZIP-215 that xDEVS consensus keys follow (a non-canonical y is reduced mod p,
    ///         `-0` is accepted). `ok = false` if the encoding is not a curve point.
    function decompress(bytes32 enc) internal view returns (bool ok, uint256 x, uint256 y) {
        uint256 v = _le(enc);
        uint256 sign = v >> 255;
        y = v & ((1 << 255) - 1);
        if (y >= P) y -= P; // non-canonical y is reduced, as curve25519-dalek and ZIP-215 do
        uint256 yy = mulmod(y, y, P);
        uint256 u = addmod(yy, P - 1, P);
        uint256 w = addmod(mulmod(D, yy, P), 1, P);
        // x = u·w^3·(u·w^7)^((p-5)/8)
        uint256 w3 = mulmod(mulmod(w, w, P), w, P);
        uint256 w7 = mulmod(mulmod(w3, w3, P), w, P);
        x = mulmod(mulmod(u, w3, P), _expmod(mulmod(u, w7, P), (P - 5) / 8), P);
        uint256 wxx = mulmod(w, mulmod(x, x, P), P);
        if (wxx != u) {
            if (wxx != P - u) return (false, 0, 0);
            x = mulmod(x, SQRT_M1, P);
        }
        if (x & 1 != sign) x = (P - x) % P;
        return (true, x, y);
    }

    function _reduce(bytes32 hHi, bytes32 hLo) private pure returns (uint256) {
        // digest (little-endian) = lo + hi·2^256, where lo = bytes 0..31 = reversed `hHi`.
        uint256 lo = _reverse(uint256(hHi));
        uint256 hi = _reverse(uint256(hLo));
        return addmod(lo % L, mulmod(hi, R256, L), L);
    }

    /// @dev [a]B + [b]Q with Q = (qx, qy) affine and a, b < 2^253; returns the affine result.
    function _doubleScalarMul(uint256 a, uint256 b, uint256 qx, uint256 qy)
        private
        view
        returns (uint256 x, uint256 y)
    {
        // Cached points (y - x, y + x, 2d·t, 2z), indexed by the 2-bit selector: 1 = B, 2 = Q, 3 = B + Q.
        uint256[16] memory tbl;
        tbl[4] = B_YMX;
        tbl[5] = B_YPX;
        tbl[6] = B_T2D;
        tbl[7] = 2;
        tbl[8] = addmod(qy, P - qx, P);
        tbl[9] = addmod(qy, qx, P);
        tbl[10] = mulmod(D2, mulmod(qx, qy, P), P);
        tbl[11] = 2;
        _cacheBPlusQ(tbl);
        uint256 z;
        assembly ("memory-safe") {
            function ladder(sa, sb, table) -> px, py, pz {
                px := 0
                py := 1
                pz := 1
                let pt := 0
                for { let i := 253 } gt(i, 0) {} {
                    i := sub(i, 1)
                    // doubling: dbl-2008-hwcd for a = -1, with the signs of E, F, G, H folded
                    {
                        let aa := mulmod(px, px, P)
                        let bb := mulmod(py, py, P)
                        let cc := mulmod(2, mulmod(pz, pz, P), P)
                        let hh := addmod(aa, bb, P)
                        let xy := addmod(px, py, P)
                        let ee := addmod(hh, sub(P, mulmod(xy, xy, P)), P)
                        aa := addmod(aa, sub(P, bb), P) // -G
                        cc := addmod(cc, aa, P) // -F
                        px := mulmod(ee, cc, P)
                        py := mulmod(aa, hh, P)
                        pt := mulmod(ee, hh, P)
                        pz := mulmod(cc, aa, P)
                    }
                    let c := add(table, shl(7, or(and(shr(i, sa), 1), shl(1, and(shr(i, sb), 1)))))
                    if gt(c, table) {
                        // addition: add-2008-hwcd-3 with a cached point
                        let aa := mulmod(addmod(py, sub(P, px), P), mload(c), P)
                        let bb := mulmod(addmod(py, px, P), mload(add(c, 0x20)), P)
                        let cc := mulmod(pt, mload(add(c, 0x40)), P)
                        let dd := mulmod(pz, mload(add(c, 0x60)), P)
                        let ee := addmod(bb, sub(P, aa), P) // E
                        bb := addmod(bb, aa, P) // H
                        aa := addmod(dd, sub(P, cc), P) // F
                        dd := addmod(dd, cc, P) // G
                        px := mulmod(ee, aa, P)
                        py := mulmod(dd, bb, P)
                        pt := mulmod(ee, bb, P)
                        pz := mulmod(aa, dd, P)
                    }
                }
            }
            x, y, z := ladder(a, b, tbl)
        }
        uint256 zi = _expmod(z, P - 2);
        x = mulmod(x, zi, P);
        y = mulmod(y, zi, P);
    }

    /// @dev tbl[12..15] = cached(B + Q) from cached Q in tbl[8..10] (add-2008-hwcd-3 with Z1 = Z2 = 1).
    function _cacheBPlusQ(uint256[16] memory tbl) private pure {
        uint256 a = mulmod(addmod(B_Y, P - B_X, P), tbl[8], P);
        uint256 b = mulmod(addmod(B_Y, B_X, P), tbl[9], P);
        uint256 c = mulmod(mulmod(B_X, B_Y, P), tbl[10], P);
        uint256 e = addmod(b, P - a, P);
        uint256 h = addmod(b, a, P);
        uint256 f = addmod(2, P - c, P);
        uint256 g = addmod(2, c, P);
        uint256 x = mulmod(e, f, P);
        uint256 y = mulmod(g, h, P);
        tbl[12] = addmod(y, P - x, P);
        tbl[13] = addmod(y, x, P);
        tbl[14] = mulmod(D2, mulmod(e, h, P), P);
        tbl[15] = mulmod(2, mulmod(f, g, P), P);
    }

    function _expmod(uint256 base, uint256 e) private view returns (uint256 r) {
        assembly ("memory-safe") {
            let p := mload(0x40)
            mstore(p, 0x20)
            mstore(add(p, 0x20), 0x20)
            mstore(add(p, 0x40), 0x20)
            mstore(add(p, 0x60), base)
            mstore(add(p, 0x80), e)
            mstore(add(p, 0xa0), P)
            if iszero(staticcall(gas(), 0x05, p, 0xc0, p, 0x20)) { revert(0, 0) }
            r := mload(p)
        }
    }

    function _le(bytes32 v) private pure returns (uint256) {
        return _reverse(uint256(v));
    }

    function _reverse(uint256 v) private pure returns (uint256) {
        v = ((v & 0xff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00) >> 8)
            | ((v & 0x00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff) << 8);
        v = ((v & 0xffff0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000) >> 16)
            | ((v & 0x0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff) << 16);
        v = ((v & 0xffffffff00000000ffffffff00000000ffffffff00000000ffffffff00000000) >> 32)
            | ((v & 0x00000000ffffffff00000000ffffffff00000000ffffffff00000000ffffffff) << 32);
        v = ((v & 0xffffffffffffffff0000000000000000ffffffffffffffff0000000000000000) >> 64)
            | ((v & 0x0000000000000000ffffffffffffffff0000000000000000ffffffffffffffff) << 64);
        return (v >> 128) | (v << 128);
    }
}
