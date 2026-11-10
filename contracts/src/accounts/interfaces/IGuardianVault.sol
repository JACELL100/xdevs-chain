// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IGuardianVault
/// @notice Guardian Vault policy interface (plan §16.2, §18.1).
/// @dev Outgoing actions that exceed the account's policy are queued for `delay` seconds. During the delay the
///      owner (any signer) or any guardian can `cancel(id)`; once the delay has elapsed anyone can `execute(id)`.
///      Changing the policy is itself a queued action when the vault is enabled.
interface IGuardianVault {
    event VaultActionQueued(
        uint256 indexed id, address target, uint256 value, bytes data, uint64 executeAfter
    );
    event VaultActionCancelled(uint256 indexed id, address by);
    event VaultActionExecuted(uint256 indexed id);

    /// @notice Sets the native threshold, delay and (replaces the) whitelist and enables the vault.
    ///         Delayed by the current delay when the vault is already enabled.
    function setPolicy(uint256 nativeThreshold, uint64 delaySeconds, address[] calldata whitelist) external;

    /// @notice Cancels a queued action. Callable by the owner (any signer) or any guardian.
    function cancel(uint256 id) external;

    /// @notice Executes a queued action once `executeAfter` has passed. Callable by anyone.
    function execute(uint256 id) external;
}
