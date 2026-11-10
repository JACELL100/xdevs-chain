// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title WXDS — Wrapped XDS (predeploy 0x…1009)
/// @author THE xDEVS
/// @notice WETH9-style 1:1 wrapper of the native coin, as an ERC-20 (name "Wrapped XDS", symbol "WXDS", 18 decimals).
/// @dev Faithful port of WETH9 to Solidity 0.8 with two deliberate differences:
///      - `withdraw` sends with a plain `call` (not `transfer`'s 2300 gas stipend) so smart accounts can unwrap;
///        state is updated before the call (checks-effects-interactions), so re-entering is harmless.
///      - custom errors instead of bare `require`.
///      Like WETH9, `totalSupply()` is `address(this).balance`; native coin forced in without `deposit`
///      (e.g. SELFDESTRUCT) is unclaimable surplus. No storage needs initialization: genesis needs only the code.
contract WXDS {
    string public constant name = "Wrapped XDS";
    string public constant symbol = "WXDS";
    uint8 public constant decimals = 18;

    error InsufficientBalance();
    error InsufficientAllowance();
    error TransferFailed();

    event Approval(address indexed src, address indexed guy, uint256 wad);
    event Transfer(address indexed src, address indexed dst, uint256 wad);
    event Deposit(address indexed dst, uint256 wad);
    event Withdrawal(address indexed src, uint256 wad);

    mapping(address => uint256) public balanceOf; // slot 0
    mapping(address => mapping(address => uint256)) public allowance; // slot 1

    receive() external payable {
        deposit();
    }

    function deposit() public payable {
        balanceOf[msg.sender] += msg.value;
        emit Deposit(msg.sender, msg.value);
    }

    function withdraw(uint256 wad) external {
        uint256 bal = balanceOf[msg.sender];
        if (bal < wad) revert InsufficientBalance();
        balanceOf[msg.sender] = bal - wad;
        emit Withdrawal(msg.sender, wad);
        (bool ok,) = payable(msg.sender).call{value: wad}("");
        if (!ok) revert TransferFailed();
    }

    function totalSupply() external view returns (uint256) {
        return address(this).balance;
    }

    function approve(address guy, uint256 wad) external returns (bool) {
        allowance[msg.sender][guy] = wad;
        emit Approval(msg.sender, guy, wad);
        return true;
    }

    function transfer(address dst, uint256 wad) external returns (bool) {
        return transferFrom(msg.sender, dst, wad);
    }

    /// @dev An allowance of type(uint256).max is never decreased (WETH9 semantics).
    function transferFrom(address src, address dst, uint256 wad) public returns (bool) {
        uint256 bal = balanceOf[src];
        if (bal < wad) revert InsufficientBalance();
        if (src != msg.sender) {
            uint256 allowed = allowance[src][msg.sender];
            if (allowed != type(uint256).max) {
                if (allowed < wad) revert InsufficientAllowance();
                allowance[src][msg.sender] = allowed - wad;
            }
        }
        balanceOf[src] = bal - wad;
        balanceOf[dst] += wad;
        emit Transfer(src, dst, wad);
        return true;
    }
}
