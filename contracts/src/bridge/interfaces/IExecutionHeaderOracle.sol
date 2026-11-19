// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IExecutionHeaderOracle - verified state / receipts roots of a remote chain's blocks
/// @notice Implemented by the light clients. A bridge proves events (receipts) and storage against these roots.
interface IExecutionHeaderOracle {
    /// @notice Roots of the verified block `number`; reverts if unknown or if the light client is frozen.
    function verifiedRoots(uint256 number) external view returns (bytes32 stateRoot, bytes32 receiptsRoot);
}
