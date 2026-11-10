// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {XDRC20} from "./XDRC20.sol";

/// @title XDRC20Bridged - canonical bridged representation of a foreign token (plan §22.3)
/// @notice An XDRC-20 (expiring approvals) whose supply is controlled by one bridge: only `bridge` can
///         mint (deposits) and burn (withdrawals), OP-StandardBridge style. Deployed exclusively by
///         {CanonicalTokenFactory}, which records it as the canonical token for (originChainId, originToken).
contract XDRC20Bridged is XDRC20 {
    address public immutable bridge;
    uint256 public immutable originChainId;
    address public immutable originToken;
    uint8 private immutable _decimals;

    error OnlyBridge(address caller);

    event Minted(address indexed to, uint256 amount);
    event Burned(address indexed from, uint256 amount);

    constructor(
        string memory name_,
        string memory symbol_,
        uint8 decimals_,
        address bridge_,
        uint256 originChainId_,
        address originToken_,
        uint64 defaultDuration,
        uint64 maxDuration
    ) XDRC20(name_, symbol_, address(0), 0, defaultDuration, maxDuration) {
        bridge = bridge_;
        originChainId = originChainId_;
        originToken = originToken_;
        _decimals = decimals_;
    }

    modifier onlyBridge() {
        if (msg.sender != bridge) revert OnlyBridge(msg.sender);
        _;
    }

    function decimals() public view override returns (uint8) {
        return _decimals;
    }

    function mint(address to, uint256 amount) external onlyBridge {
        _mint(to, amount);
        emit Minted(to, amount);
    }

    function burn(address from, uint256 amount) external onlyBridge {
        _burn(from, amount);
        emit Burned(from, amount);
    }
}
