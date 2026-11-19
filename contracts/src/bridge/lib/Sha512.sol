// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title Sha512 - SHA-512 (FIPS 180-4) in Yul, for Ed25519 verification on chains without an Ed25519 precompile
/// @notice Ethereum has no SHA-512 precompile; Ed25519 needs `SHA-512(R ‖ A ‖ M)`. Cost ≈ 45k gas per 128-byte
///         block. Tested against NIST vectors and node:crypto in `test/bridge/Ed25519.t.sol`.
library Sha512 {
    /// @dev Round constants K[0..79], 8 bytes each, big-endian.
    bytes internal constant K = hex"428a2f98d728ae227137449123ef65cdb5c0fbcfec4d3b2fe9b5dba58189dbbc3956c25bf348b53859f111f1b605d019"
        hex"923f82a4af194f9bab1c5ed5da6d8118d807aa98a303024212835b0145706fbe243185be4ee4b28c550c7dc3d5ffb4e2"
        hex"72be5d74f27b896f80deb1fe3b1696b19bdc06a725c71235c19bf174cf692694e49b69c19ef14ad2efbe4786384f25e3"
        hex"0fc19dc68b8cd5b5240ca1cc77ac9c652de92c6f592b02754a7484aa6ea6e4835cb0a9dcbd41fbd476f988da831153b5"
        hex"983e5152ee66dfaba831c66d2db43210b00327c898fb213fbf597fc7beef0ee4c6e00bf33da88fc2d5a79147930aa725"
        hex"06ca6351e003826f142929670a0e6e7027b70a8546d22ffc2e1b21385c26c9264d2c6dfc5ac42aed53380d139d95b3df"
        hex"650a73548baf63de766a0abb3c77b2a881c2c92e47edaee692722c851482353ba2bfe8a14cf10364a81a664bbc423001"
        hex"c24b8b70d0f89791c76c51a30654be30d192e819d6ef5218d69906245565a910f40e35855771202a106aa07032bbd1b8"
        hex"19a4c116b8d2d0c81e376c085141ab532748774cdf8eeb9934b0bcb5e19b48a8391c0cb3c5c95a634ed8aa4ae3418acb"
        hex"5b9cca4f7763e373682e6ff3d6b2b8a3748f82ee5defb2fc78a5636f43172f6084c87814a1f0ab728cc702081a6439ec"
        hex"90befffa23631e28a4506cebde82bde9bef9a3f7b2c67915c67178f2e372532bca273eceea26619cd186b8c721c0c207"
        hex"eada7dd6cde0eb1ef57d4f7fee6ed17806f067aa72176fba0a637dc5a2c898a6113f9804bef90dae1b710b35131c471b"
        hex"28db77f523047d8432caab7b40c724933c9ebe0a15c9bebc431d67c49c100d4c4cc5d4becb3e42b6597f299cfc657e2a"
        hex"5fcb6fab3ad6faec6c44198c4a475817";

    /// @notice SHA-512 of `data`; the 64-byte digest as two big-endian words (`hi` = bytes 0..31).
    function hash(bytes memory data) internal pure returns (bytes32 hi, bytes32 lo) {
        uint256 len = data.length;
        uint256 padded = ((len + 17 + 127) / 128) * 128;
        bytes memory m = new bytes(padded);
        bytes memory k = K;
        assembly ("memory-safe") {
            mcopy(add(m, 0x20), add(data, 0x20), len)
            mstore8(add(add(m, 0x20), len), 0x80)
            // 128-bit big-endian bit length in the last 16 bytes (the high 64 bits are always zero here).
            mstore(add(m, padded), or(and(mload(add(m, padded)), not(0xffffffffffffffff)), mul(len, 8)))

            // Scratch: 8 state words then W[80] (each in its own 32-byte slot).
            let st := mload(0x40)
            let w := add(st, 0x100)
            mstore(0x40, add(w, 0xa00))
            mstore(st, 0x6a09e667f3bcc908)
            mstore(add(st, 0x20), 0xbb67ae8584caa73b)
            mstore(add(st, 0x40), 0x3c6ef372fe94f82b)
            mstore(add(st, 0x60), 0xa54ff53a5f1d36f1)
            mstore(add(st, 0x80), 0x510e527fade682d1)
            mstore(add(st, 0xa0), 0x9b05688c2b3e6c1f)
            mstore(add(st, 0xc0), 0x1f83d9abfb41bd6b)
            mstore(add(st, 0xe0), 0x5be0cd19137e2179)

            function rotr(x, n) -> r {
                r := and(or(shr(n, x), shl(sub(64, n), x)), 0xffffffffffffffff)
            }

            for { let off := 0 } lt(off, padded) { off := add(off, 128) } {
                let blk := add(add(m, 0x20), off)
                for { let t := 0 } lt(t, 16) { t := add(t, 1) } {
                    mstore(add(w, mul(t, 0x20)), shr(192, mload(add(blk, mul(t, 8)))))
                }
                for { let t := 16 } lt(t, 80) { t := add(t, 1) } {
                    let a := mload(add(w, mul(sub(t, 15), 0x20)))
                    let b := mload(add(w, mul(sub(t, 2), 0x20)))
                    let s0 := xor(xor(rotr(a, 1), rotr(a, 8)), shr(7, a))
                    let s1 := xor(xor(rotr(b, 19), rotr(b, 61)), shr(6, b))
                    mstore(
                        add(w, mul(t, 0x20)),
                        and(
                            add(
                                add(mload(add(w, mul(sub(t, 16), 0x20))), s0),
                                add(mload(add(w, mul(sub(t, 7), 0x20))), s1)
                            ),
                            0xffffffffffffffff
                        )
                    )
                }
                // Fold K into W so the round loop needs one pointer.
                for { let t := 0 } lt(t, 80) { t := add(t, 1) } {
                    let p := add(w, mul(t, 0x20))
                    mstore(p, add(mload(p), shr(192, mload(add(add(k, 0x20), mul(t, 8))))))
                }

                let a := mload(st)
                let b := mload(add(st, 0x20))
                let c := mload(add(st, 0x40))
                let d := mload(add(st, 0x60))
                let e := mload(add(st, 0x80))
                let f := mload(add(st, 0xa0))
                let g := mload(add(st, 0xc0))
                let h := mload(add(st, 0xe0))
                for { let t := 0 } lt(t, 80) { t := add(t, 1) } {
                    let t1 :=
                        add(
                            add(h, xor(xor(rotr(e, 14), rotr(e, 18)), rotr(e, 41))),
                            add(xor(and(e, f), and(not(e), g)), mload(add(w, mul(t, 0x20))))
                        )
                    let t2 :=
                        add(
                            xor(xor(rotr(a, 28), rotr(a, 34)), rotr(a, 39)),
                            xor(xor(and(a, b), and(a, c)), and(b, c))
                        )
                    h := g
                    g := f
                    f := e
                    e := and(add(d, t1), 0xffffffffffffffff)
                    d := c
                    c := b
                    b := a
                    a := and(add(t1, t2), 0xffffffffffffffff)
                }
                mstore(st, and(add(mload(st), a), 0xffffffffffffffff))
                mstore(add(st, 0x20), and(add(mload(add(st, 0x20)), b), 0xffffffffffffffff))
                mstore(add(st, 0x40), and(add(mload(add(st, 0x40)), c), 0xffffffffffffffff))
                mstore(add(st, 0x60), and(add(mload(add(st, 0x60)), d), 0xffffffffffffffff))
                mstore(add(st, 0x80), and(add(mload(add(st, 0x80)), e), 0xffffffffffffffff))
                mstore(add(st, 0xa0), and(add(mload(add(st, 0xa0)), f), 0xffffffffffffffff))
                mstore(add(st, 0xc0), and(add(mload(add(st, 0xc0)), g), 0xffffffffffffffff))
                mstore(add(st, 0xe0), and(add(mload(add(st, 0xe0)), h), 0xffffffffffffffff))
            }
            hi := or(
                or(shl(192, mload(st)), shl(128, mload(add(st, 0x20)))),
                or(shl(64, mload(add(st, 0x40))), mload(add(st, 0x60)))
            )
            lo := or(
                or(shl(192, mload(add(st, 0x80))), shl(128, mload(add(st, 0xa0)))),
                or(shl(64, mload(add(st, 0xc0))), mload(add(st, 0xe0)))
            )
        }
    }
}
