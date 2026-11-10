// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title XNSLabel
/// @notice Label normalization, confusable "skeletons" and ENS namehash helpers for the xDEVS Name Service.
/// @dev Normalization is deliberately much stricter than ENSIP-15: a registrable `.xdevs` label is
///      3..63 bytes of `[a-z0-9-]` only, with no leading/trailing hyphen and no `--` anywhere (which also
///      rules out IDNA/punycode `xn--` labels). Anything else - uppercase, whitespace, dots, underscores,
///      emoji, any non-ASCII byte - is rejected rather than "normalized", so a label has exactly one
///      on-chain spelling and Unicode homograph attacks are impossible by construction.
///
///      On top of that, ASCII look-alikes are collapsed into a "skeleton" (see {skeleton}); the registrar
///      only lets one live name own a given skeleton, so `paypa1`/`paypal`, `g00gle`/`google`,
///      `rnetamask`/`metamask`, `vvallet`/`wallet` and `cloge`/`doge` cannot coexist.
library XNSLabel {
    uint256 internal constant MIN_LENGTH = 3;
    uint256 internal constant MAX_LENGTH = 63;

    /// @notice Reasons a label is not registrable.
    enum Invalid {
        None,
        TooShort,
        TooLong,
        ForbiddenCharacter,
        BadHyphen
    }

    error InvalidLabel(string label, Invalid reason);

    /// @notice Returns `Invalid.None` iff `label` is a normalized, registrable `.xdevs` label.
    function check(string memory label) internal pure returns (Invalid) {
        bytes memory b = bytes(label);
        uint256 len = b.length;
        if (len < MIN_LENGTH) return Invalid.TooShort;
        if (len > MAX_LENGTH) return Invalid.TooLong;
        bytes1 prev;
        for (uint256 i; i < len; ++i) {
            bytes1 c = b[i];
            bool isLower = c >= 0x61 && c <= 0x7a; // a-z
            bool isDigit = c >= 0x30 && c <= 0x39; // 0-9
            if (c == 0x2d) {
                // '-' : not first, not last, never doubled ("--" also covers punycode "xn--").
                if (i == 0 || i == len - 1 || prev == 0x2d) return Invalid.BadHyphen;
            } else if (!isLower && !isDigit) {
                return Invalid.ForbiddenCharacter;
            }
            prev = c;
        }
        return Invalid.None;
    }

    /// @notice True iff `label` is a normalized, registrable label.
    function isValid(string memory label) internal pure returns (bool) {
        return check(label) == Invalid.None;
    }

    /// @notice Reverts with {InvalidLabel} unless `label` is registrable.
    function validate(string memory label) internal pure {
        Invalid r = check(label);
        if (r != Invalid.None) revert InvalidLabel(label, r);
    }

    /// @notice Hash of the confusable skeleton of a (valid) label.
    /// @dev Mapping, applied greedily left to right:
    ///      `0`->`o`, `1`->`l`, `rn`->`m`, `vv`->`w`, `cl`->`d`. These are the ASCII-lowercase entries of
    ///      the Unicode UTS#39 confusables table that survive our `[a-z0-9-]` alphabet. Two labels with the
    ///      same skeleton render (near-)identically in common UI fonts.
    function skeleton(string memory label) internal pure returns (bytes32) {
        bytes memory b = bytes(label);
        uint256 len = b.length;
        bytes memory out = new bytes(len);
        uint256 j;
        for (uint256 i; i < len; ++i) {
            bytes1 c = b[i];
            bytes1 n = i + 1 < len ? b[i + 1] : bytes1(0);
            if (c == "0") {
                c = "o";
            } else if (c == "1") {
                c = "l";
            } else if (c == "r" && n == "n") {
                c = "m";
                ++i;
            } else if (c == "v" && n == "v") {
                c = "w";
                ++i;
            } else if (c == "c" && n == "l") {
                c = "d";
                ++i;
            }
            out[j++] = c;
        }
        assembly ("memory-safe") {
            mstore(out, j)
        }
        return keccak256(out);
    }

    /// @notice ENS / EIP-137 namehash of a dot-separated name (no normalization is applied).
    function namehash(string memory name) internal pure returns (bytes32 node) {
        bytes memory b = bytes(name);
        uint256 end = b.length;
        if (end == 0) return bytes32(0);
        // Walk labels right-to-left: namehash(a.b) = keccak(namehash(b), keccak(a)).
        while (true) {
            uint256 start = end;
            while (start > 0 && b[start - 1] != ".") --start;
            bytes32 labelhash;
            assembly ("memory-safe") {
                labelhash := keccak256(add(add(b, 32), start), sub(end, start))
            }
            node = keccak256(abi.encodePacked(node, labelhash));
            if (start == 0) break;
            end = start - 1;
        }
    }

    /// @notice keccak256 of the lowercase hex encoding (no `0x`) of `addr` - ENS `sha3HexAddress`.
    function hexAddressLabelhash(address addr) internal pure returns (bytes32 h) {
        bytes memory s = new bytes(40);
        bytes16 digits = "0123456789abcdef";
        uint160 v = uint160(addr);
        for (uint256 i = 40; i > 0; --i) {
            s[i - 1] = digits[v & 0xf];
            v >>= 4;
        }
        h = keccak256(s);
    }
}
