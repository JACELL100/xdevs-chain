// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

/// Self-contained copy of the ERC-4337 v0.9 PackedUserOperation (no imports).
struct PackedUserOperation {
    address sender;
    uint256 nonce;
    bytes initCode;
    bytes callData;
    bytes32 accountGasLimits;
    uint256 preVerificationGas;
    bytes32 gasFees;
    bytes paymasterAndData;
    bytes signature;
}

interface IEntryPointStake {
    function addStake(uint32 unstakeDelaySec) external payable;
    function depositTo(address account) external payable;
}

/// @title TestTogglePaymaster (BUNDLER TESTS ONLY)
/// @notice A staked paymaster whose validation anyone can switch off. Used to check that the bundler drops ops
///         invalidated after admission, throttles the paymaster (ERC-7562 reputation) and bans it when a bundle
///         reverts on chain because of it (front-run invalidation).
contract TestTogglePaymaster {
    address public constant ENTRY_POINT = 0x433709009B8330FDa32311DF1C2AFA402eD8D009;
    bool public broken;

    function setBroken(bool b) external {
        broken = b;
    }

    function deposit() external payable {
        IEntryPointStake(ENTRY_POINT).depositTo{value: msg.value}(address(this));
    }

    function addStake(uint32 unstakeDelaySec) external payable {
        IEntryPointStake(ENTRY_POINT).addStake{value: msg.value}(unstakeDelaySec);
    }

    function validatePaymasterUserOp(PackedUserOperation calldata, bytes32, uint256)
        external
        view
        returns (bytes memory context, uint256 validationData)
    {
        require(msg.sender == ENTRY_POINT, "not from EntryPoint");
        require(!broken, "paymaster switched off");
        return ("", 0);
    }

    function postOp(uint8, bytes calldata, uint256, uint256) external {}
}
