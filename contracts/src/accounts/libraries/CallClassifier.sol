// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {AccountConstants} from "./AccountConstants.sol";

/// @title CallClassifier
/// @notice Recognises approval-like and asset-transfer calls from their calldata so the Guardian Vault and the
///         session-key module can apply the allowance / spending policy (plan §16.2, §16.5).
/// @dev Classification is purely syntactic (selector + arguments). Unknown selectors are `Generic` and are
///      governed by the contract whitelist. Known selectors with malformed (too short) calldata are `Malformed` and
///      are always treated as needing the delay.
library CallClassifier {
    enum Kind {
        Generic,
        Approval,
        AssetTransfer,
        Malformed
    }

    struct Info {
        Kind kind;
        /// @dev Token whose allowance / balance is affected (usually the call target; Permit2: argument 0).
        address token;
        /// @dev Spender / operator (Approval) or recipient (AssetTransfer).
        address counterparty;
        /// @dev Amount (Approval / fungible transfer). For NFTs this is the token id and must not be summed.
        uint256 amount;
        /// @dev AssetTransfer only: true for ERC-20-style fungible transfers that per-token limits can apply to.
        bool fungible;
        /// @dev Approval only: allowance revocation (amount 0 / setApprovalForAll(false)). Always instant.
        bool revoke;
        /// @dev Approval only: unlimited approval (amount >= uint160.max, setApprovalForAll(true), operators).
        bool unlimited;
    }

    // Approvals
    bytes4 internal constant APPROVE = 0x095ea7b3; // approve(address,uint256)  (ERC-20 and ERC-721)
    bytes4 internal constant INCREASE_ALLOWANCE = 0x39509351; // increaseAllowance(address,uint256)
    bytes4 internal constant SET_APPROVAL_FOR_ALL = 0xa22cb465; // setApprovalForAll(address,bool)
    bytes4 internal constant PERMIT2_APPROVE = 0x87517c45; // approve(address,address,uint160,uint48)
    bytes4 internal constant XDRC20_APPROVE = 0xf6eaa763; // approve(address,uint256,uint64)  (plan §16.5)
    bytes4 internal constant AUTHORIZE_OPERATOR = 0x959b8c3f; // authorizeOperator(address)  (ERC-777)
    // Transfers
    bytes4 internal constant TRANSFER = 0xa9059cbb; // transfer(address,uint256)
    bytes4 internal constant SEND_777 = 0x9bd9bbc6; // send(address,uint256,bytes)
    bytes4 internal constant TRANSFER_FROM = 0x23b872dd; // transferFrom(address,address,uint256)
    bytes4 internal constant SAFE_TRANSFER_FROM = 0x42842e0e; // safeTransferFrom(address,address,uint256)
    bytes4 internal constant SAFE_TRANSFER_FROM_DATA = 0xb88d4fde; // safeTransferFrom(address,address,uint256,bytes)
    bytes4 internal constant SAFE_TRANSFER_FROM_1155 = 0xf242432a; // safeTransferFrom(address,address,uint256,uint256,bytes)
    bytes4 internal constant SAFE_BATCH_TRANSFER_FROM = 0x2eb2c2d6; // safeBatchTransferFrom(...)

    function selectorOf(bytes memory data) internal pure returns (bytes4 sel) {
        if (data.length < 4) return bytes4(0);
        assembly ("memory-safe") {
            sel := mload(add(data, 0x20))
        }
    }

    /// @dev Reads the `i`-th 32-byte ABI word after the selector. Caller must check the length.
    function _word(bytes memory data, uint256 i) private pure returns (uint256 w) {
        assembly ("memory-safe") {
            w := mload(add(add(data, 0x24), mul(i, 0x20)))
        }
    }

    function _hasWords(bytes memory data, uint256 n) private pure returns (bool) {
        return data.length >= 4 + 32 * n;
    }

    /// @notice Classifies `data` sent by account `self` to `target`.
    function classify(address self, address target, bytes memory data)
        internal
        pure
        returns (Info memory info)
    {
        info.token = target;
        if (data.length < 4) return info; // Generic (plain value transfer / fallback)
        bytes4 sel = selectorOf(data);

        if (sel == APPROVE || sel == INCREASE_ALLOWANCE || sel == XDRC20_APPROVE) {
            if (!_hasWords(data, 2)) return _malformed(info);
            info.kind = Kind.Approval;
            info.counterparty = address(uint160(_word(data, 0)));
            info.amount = _word(data, 1);
            info.revoke = info.amount == 0;
            info.unlimited = info.amount >= AccountConstants.UNLIMITED_APPROVAL;
        } else if (sel == SET_APPROVAL_FOR_ALL) {
            if (!_hasWords(data, 2)) return _malformed(info);
            info.kind = Kind.Approval;
            info.counterparty = address(uint160(_word(data, 0)));
            bool approved = _word(data, 1) != 0;
            info.revoke = !approved;
            info.unlimited = approved;
            info.amount = approved ? type(uint256).max : 0;
        } else if (sel == PERMIT2_APPROVE) {
            if (!_hasWords(data, 4)) return _malformed(info);
            info.kind = Kind.Approval;
            info.token = address(uint160(_word(data, 0)));
            info.counterparty = address(uint160(_word(data, 1)));
            info.amount = _word(data, 2);
            info.revoke = info.amount == 0;
            info.unlimited = info.amount >= AccountConstants.UNLIMITED_APPROVAL;
        } else if (sel == AUTHORIZE_OPERATOR) {
            if (!_hasWords(data, 1)) return _malformed(info);
            info.kind = Kind.Approval;
            info.counterparty = address(uint160(_word(data, 0)));
            info.unlimited = true;
            info.amount = type(uint256).max;
        } else if (sel == TRANSFER || sel == SEND_777) {
            if (!_hasWords(data, 2)) return _malformed(info);
            info.kind = Kind.AssetTransfer;
            info.counterparty = address(uint160(_word(data, 0)));
            info.amount = _word(data, 1);
            info.fungible = true;
        } else if (
            sel == TRANSFER_FROM || sel == SAFE_TRANSFER_FROM || sel == SAFE_TRANSFER_FROM_DATA
                || sel == SAFE_TRANSFER_FROM_1155 || sel == SAFE_BATCH_TRANSFER_FROM
        ) {
            if (!_hasWords(data, 3)) return _malformed(info);
            // Only transfers *out of this account* are asset transfers; pulling third-party tokens is Generic.
            if (address(uint160(_word(data, 0))) != self) return info;
            info.kind = Kind.AssetTransfer;
            info.counterparty = address(uint160(_word(data, 1)));
            info.amount = _word(data, 2);
            // transferFrom is shared by ERC-20 and ERC-721; it is only treated as fungible where a per-token
            // limit is configured (ERC-20 tokens). safeTransferFrom variants are NFTs / ERC-1155.
            info.fungible = sel == TRANSFER_FROM;
        }
    }

    function _malformed(Info memory info) private pure returns (Info memory) {
        info.kind = Kind.Malformed;
        info.unlimited = true;
        info.amount = type(uint256).max;
        return info;
    }
}
