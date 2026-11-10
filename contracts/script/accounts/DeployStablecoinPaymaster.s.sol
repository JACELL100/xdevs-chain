// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";

import {AccountConstants} from "../../src/accounts/libraries/AccountConstants.sol";
import {StablecoinPaymaster} from "../../src/accounts/paymaster/StablecoinPaymaster.sol";

/// @notice Deploys the example StablecoinPaymaster (application layer, not a predeploy).
/// @dev Env: `SPM_OWNER` (operator), optional `SPM_TOKEN` + `SPM_TOKENS_PER_NATIVE` (token units per 1e18 wei),
///      optional `SPM_DEPOSIT` (wei of tXDS deposited to the EntryPoint), optional `SPM_MARKUP_BPS`.
///      The owner-only configuration calls are only made when the broadcaster is the owner.
///      `FOUNDRY_PROFILE=accounts forge script script/accounts/DeployStablecoinPaymaster.s.sol --rpc-url $RPC --broadcast`
contract DeployStablecoinPaymaster is Script {
    function run() external returns (StablecoinPaymaster spm) {
        address owner = vm.envAddress("SPM_OWNER");
        address token = vm.envOr("SPM_TOKEN", address(0));
        uint256 rate = vm.envOr("SPM_TOKENS_PER_NATIVE", uint256(0));
        uint256 depositAmount = vm.envOr("SPM_DEPOSIT", uint256(0));
        uint256 markup = vm.envOr("SPM_MARKUP_BPS", uint256(0));

        vm.startBroadcast();
        spm = new StablecoinPaymaster(IEntryPoint(AccountConstants.ENTRY_POINT), owner);
        if (depositAmount != 0) spm.deposit{value: depositAmount}();
        if (msg.sender == owner) {
            if (token != address(0) && rate != 0) spm.setToken(token, true, rate);
            if (markup != 0) spm.setMarkup(markup);
        }
        vm.stopBroadcast();
        console2.log("StablecoinPaymaster", address(spm));
    }
}
