// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Predeploys} from "../common/Predeploys.sol";

/// @title Treasury — protocol treasury (predeploy 0x…1002)
/// @author THE xDEVS
/// @notice Holds native tXDS for grants and public goods. Spending is only possible through Governance (vote +
///         timelock).
/// @dev INFLOWS: the node credits 20% of every block's base fees by DIRECTLY increasing this account's balance
///      (no call, no event). `receive()` only exists for voluntary deposits. Nothing in this contract relies on
///      `receive()` having run: the spendable amount is always `address(this).balance`.
///      GENESIS: no constructor logic; the genesis builder calls {initialize} once as SYSTEM_ADDRESS.
contract Treasury is ReentrancyGuardTransient {
    using SafeERC20 for IERC20;

    error OnlySystem();
    error OnlyGovernance();
    error AlreadyInitialized();
    error ZeroAddress();
    error InsufficientBalance(uint256 available, uint256 requested);
    error TransferFailed();

    event Initialized(address indexed governance);
    event Deposited(address indexed from, uint256 amount);
    event Granted(address indexed to, uint256 amount, string memo);
    event TokenGranted(address indexed token, address indexed to, uint256 amount, string memo);

    /// @notice slot 0: Governance address (0 until initialized).
    address public governance;
    /// @notice slot 1: cumulative native tXDS granted.
    uint256 public totalGranted;

    modifier onlyGovernance() {
        if (msg.sender != governance || msg.sender == address(0)) revert OnlyGovernance();
        _;
    }

    /// @dev Intentionally empty: never runs at the predeploy address.
    constructor() {}

    /// @notice One-time setup, called by the genesis builder as SYSTEM_ADDRESS.
    function initialize(address governance_) external {
        if (msg.sender != Predeploys.SYSTEM_ADDRESS) revert OnlySystem();
        if (governance != address(0)) revert AlreadyInitialized();
        if (governance_ == address(0)) revert ZeroAddress();
        governance = governance_;
        emit Initialized(governance_);
    }

    receive() external payable {
        emit Deposited(msg.sender, msg.value);
    }

    /// @notice Pays `amount` native tXDS to `to`. Only callable by Governance (i.e. by an executed proposal).
    function grant(address payable to, uint256 amount, string calldata memo)
        external
        onlyGovernance
        nonReentrant
    {
        if (to == address(0)) revert ZeroAddress();
        uint256 bal = address(this).balance;
        if (amount > bal) revert InsufficientBalance(bal, amount);
        totalGranted += amount;
        emit Granted(to, amount, memo);
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }

    /// @notice Pays `amount` of an ERC-20 held by the Treasury. Only callable by Governance.
    function grantToken(IERC20 token, address to, uint256 amount, string calldata memo)
        external
        onlyGovernance
        nonReentrant
    {
        if (to == address(0) || address(token) == address(0)) revert ZeroAddress();
        emit TokenGranted(address(token), to, amount, memo);
        token.safeTransfer(to, amount);
    }

    /// @notice Native tXDS currently available for grants.
    function balance() external view returns (uint256) {
        return address(this).balance;
    }
}
