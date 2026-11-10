// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {XNSRegistry} from "../../src/periphery/xns/XNSRegistry.sol";
import {Permit2Expiring} from "../../src/periphery/permit2/Permit2Expiring.sol";
import {XDRC20} from "../../src/periphery/token/XDRC20.sol";
import {CanonicalTokenFactory} from "../../src/periphery/token/CanonicalTokenFactory.sol";

/// @title DeployPeriphery
/// @notice Two entry points:
///   - `run()`: deploy the periphery contracts at ordinary addresses on a local devnet / anvil
///     (NOT how the real chain gets them - XNSRegistry and Permit2Expiring are genesis predeploys).
///       FOUNDRY_PROFILE=periphery forge script script/periphery/DeployPeriphery.s.sol \
///         --rpc-url $RPC --private-key $PK --broadcast
///     Env (optional): XNS_GOVERNANCE, XNS_TREASURY (default: deployer), XNS_FEE_PER_YEAR (default 0).
///   - `genesisInitCalldata()`: prints the calldata the Rust genesis builder must execute as
///     `Predeploys.SYSTEM_ADDRESS -> Predeploys.XNS_REGISTRY` after placing the runtime code
///     (`forge inspect XNSRegistry deployedBytecode`). Permit2Expiring needs no initialization.
///       FOUNDRY_PROFILE=periphery forge script script/periphery/DeployPeriphery.s.sol --sig "genesisInitCalldata()"
contract DeployPeriphery is Script {
    function _reservedLabels() internal pure returns (string[] memory r) {
        r = new string[](6);
        r[0] = "xdevs";
        r[1] = "the-xdevs";
        r[2] = "official";
        r[3] = "support";
        r[4] = "admin";
        r[5] = "faucet";
    }

    function genesisInitCalldata() external pure returns (bytes memory data) {
        data = abi.encodeCall(
            XNSRegistry.initialize, (Predeploys.GOVERNANCE, Predeploys.TREASURY, 0, _reservedLabels())
        );
        console2.log("XNSRegistry @", Predeploys.XNS_REGISTRY);
        console2.log("caller       ", Predeploys.SYSTEM_ADDRESS);
        console2.logBytes(data);
    }

    function run() external {
        vm.startBroadcast();
        (, address deployer,) = vm.readCallers();
        address gov = vm.envOr("XNS_GOVERNANCE", deployer);
        address treasury = vm.envOr("XNS_TREASURY", deployer);
        uint256 fee = vm.envOr("XNS_FEE_PER_YEAR", uint256(0));

        XNSRegistry xns = new XNSRegistry();
        xns.initialize(gov, treasury, fee, _reservedLabels());
        Permit2Expiring permit2 = new Permit2Expiring();
        XDRC20 token = new XDRC20("xDEVS Test Token", "XTT", deployer, 1_000_000 ether, 30 days, 365 days);
        CanonicalTokenFactory factory = new CanonicalTokenFactory(gov, 30 days, 365 days);
        vm.stopBroadcast();

        console2.log("XNSRegistry          ", address(xns));
        console2.log("Permit2Expiring      ", address(permit2));
        console2.log("XDRC20 (XTT)         ", address(token));
        console2.log("CanonicalTokenFactory", address(factory));
    }
}
