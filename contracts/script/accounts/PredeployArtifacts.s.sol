// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";

import {Predeploys} from "../../src/common/Predeploys.sol";
import {XdevsSmartAccount} from "../../src/accounts/XdevsSmartAccount.sol";
import {SmartAccountFactory} from "../../src/accounts/SmartAccountFactory.sol";
import {XdevsPaymaster} from "../../src/accounts/paymaster/XdevsPaymaster.sol";
import {AccountConstants} from "../../src/accounts/libraries/AccountConstants.sol";

/// @notice Prints what the Rust genesis builder must place for the account predeploys.
/// @dev All three predeploys need ONLY their runtime code (`deployedBytecode` in
///      `out/accounts/<File>.sol/<Contract>.json`); no storage, no balance, no init call:
///        0x…1005 XdevsSmartAccount   (implementation / EIP-7702 delegate; locked by an address check)
///        0x…1006 SmartAccountFactory (also hosts AccountExtension, the accounts' DELEGATECALL target)
///        0x…1007 XdevsPaymaster      (permissionless sponsor registry; stake it via addStake after genesis)
///      The canonical ERC-4337 v0.9 EntryPoint (0x433709009B8330FDa32311DF1C2AFA402eD8D009) must also be present;
///      it has constructor-set immutables, so insert the bytecode produced by its canonical CREATE2 deployment,
///      not the raw artifact.
///      Also prints the XIP-0001 EOA-lock constants the node needs (docs/specs/xip-0001-eoa-lock.md).
///      Run: `FOUNDRY_PROFILE=accounts forge script script/accounts/PredeployArtifacts.s.sol`
contract PredeployArtifacts is Script {
    function run() external view {
        _print("XdevsSmartAccount", Predeploys.SMART_ACCOUNT_IMPL, type(XdevsSmartAccount).runtimeCode);
        _print("SmartAccountFactory", Predeploys.SMART_ACCOUNT_FACTORY, type(SmartAccountFactory).runtimeCode);
        _print("XdevsPaymaster", Predeploys.PAYMASTER, type(XdevsPaymaster).runtimeCode);
        console2.log("XIP-0001 EOA lock");
        console2.log("  lock slot       ", vm.toString(AccountConstants.EOA_LOCK_SLOT));
        console2.log(
            "  designator      ", vm.toString(abi.encodePacked(hex"ef0100", Predeploys.SMART_ACCOUNT_IMPL))
        );
        console2.log("  designator hash ", vm.toString(AccountConstants.EIP7702_DESIGNATOR_HASH));
    }

    function _print(string memory name, address at, bytes memory code) internal pure {
        console2.log(name);
        console2.log("  address  ", at);
        console2.log("  size     ", code.length);
        console2.log("  codehash ", vm.toString(keccak256(code)));
    }
}
