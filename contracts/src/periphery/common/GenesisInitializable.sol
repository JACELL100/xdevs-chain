// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Predeploys} from "../../common/Predeploys.sol";

/// @title GenesisInitializable
/// @notice One-time initializer guard for contracts that are *placed* in the genesis state (plan §18).
/// @dev The genesis builder copies runtime bytecode to the predeploy address, so the constructor never runs
///      there and storage starts empty. `initialize(...)` may therefore be called exactly once, and only by:
///        - `Predeploys.SYSTEM_ADDRESS` (the genesis builder / protocol system call), or
///        - the account that ran the constructor (normal deployments: tests, local devnets).
///      `_DEPLOYER` is an immutable. In the runtime bytecode emitted by solc (`deployedBytecode`) immutable
///      slots are zero-filled, so for genesis-placed code `_DEPLOYER == address(0)`, which can never be
///      `msg.sender` - only the system address can initialize. Nothing else (address(this), chainid) is
///      cached in immutables.
abstract contract GenesisInitializable {
    error AlreadyInitialized();
    error NotInitializer(address caller);
    error NotInitialized();

    address private immutable _DEPLOYER;
    bool private _initialized;

    constructor() {
        _DEPLOYER = msg.sender;
    }

    modifier initializer() {
        if (_initialized) revert AlreadyInitialized();
        if (msg.sender != Predeploys.SYSTEM_ADDRESS && msg.sender != _DEPLOYER) {
            revert NotInitializer(msg.sender);
        }
        _initialized = true;
        _;
    }

    modifier whenInitialized() {
        if (!_initialized) revert NotInitialized();
        _;
    }

    /// @notice True once `initialize` has run.
    function initialized() public view returns (bool) {
        return _initialized;
    }
}
