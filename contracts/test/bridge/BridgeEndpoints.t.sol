// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {BridgeBase} from "../../src/bridge/BridgeBase.sol";
import {SepoliaTokenVault} from "../../src/bridge/SepoliaTokenVault.sol";
import {XdevsBridge} from "../../src/bridge/XdevsBridge.sol";
import {IExecutionHeaderOracle} from "../../src/bridge/interfaces/IExecutionHeaderOracle.sol";
import {EthProofs} from "../../src/bridge/lib/EthProofs.sol";
import {CanonicalTokenFactory} from "../../src/periphery/token/CanonicalTokenFactory.sol";
import {XDRC20Bridged} from "../../src/periphery/token/XDRC20Bridged.sol";

/// Stands in for a light client: returns fixed receipts roots (the light clients themselves are tested on real
/// data in EthereumLightClient.t.sol / XdevsLightClient.t.sol).
contract MockOracle is IExecutionHeaderOracle {
    mapping(uint256 => bytes32) public roots;

    function set(uint256 n, bytes32 r) external {
        roots[n] = r;
    }

    function verifiedRoots(uint256 n) external view returns (bytes32, bytes32) {
        require(roots[n] != bytes32(0), "unknown");
        return (bytes32(0), roots[n]);
    }
}

contract TestToken is ERC20 {
    constructor() ERC20("Test", "TST") {}

    function mint(address to, uint256 a) external {
        _mint(to, a);
    }
}

contract BridgeEndpointsTest is Test {
    string fx;
    MockOracle oracle;
    address guardian = makeAddr("guardian");
    address vaultAddr;
    address bridgeAddr;
    address token;
    address user;
    address recipient;

    function setUp() public {
        fx = vm.readFile("test/bridge/fixtures/xdevs-dev.json");
        oracle = new MockOracle();
        oracle.set(1, vm.parseJsonBytes32(fx, ".bridge.lockReceiptsRoot"));
        oracle.set(2, vm.parseJsonBytes32(fx, ".bridge.burnReceiptsRoot"));
        vaultAddr = vm.parseJsonAddress(fx, ".bridge.vault");
        bridgeAddr = vm.parseJsonAddress(fx, ".bridge.xdevsBridge");
        token = vm.parseJsonAddress(fx, ".bridge.token");
        user = vm.parseJsonAddress(fx, ".bridge.user");
        recipient = vm.parseJsonAddress(fx, ".bridge.recipient");
    }

    function _proof(uint256 block_, uint256 txIndex, string memory key)
        internal
        view
        returns (BridgeBase.EventProof memory p)
    {
        p.blockNumber = block_;
        p.txIndex = txIndex;
        p.receiptProof = vm.parseJsonBytesArray(fx, key);
        p.logIndex = 0;
    }

    function _lockProof(uint256 i) internal view returns (BridgeBase.EventProof memory) {
        return _proof(1, i, string.concat(".bridge.lockProofs[", vm.toString(i), "]"));
    }

    function _xdevsSide() internal returns (XdevsBridge b, XDRC20Bridged t) {
        CanonicalTokenFactory f = new CanonicalTokenFactory(address(this), 30 days, 365 days);
        b = new XdevsBridge(oracle, 11155111, guardian, f);
        b.setCounterpart(vaultAddr);
        t = XDRC20Bridged(f.createCanonicalToken(11155111, token, "Test (Sepolia)", "TST", 18, address(b)));
    }

    function test_mint_from_proven_lock_and_replay() public {
        (XdevsBridge b, XDRC20Bridged t) = _xdevsSide();
        BridgeBase.EventProof memory p = _lockProof(1);
        uint256 g = gasleft();
        b.mint(p);
        emit log_named_uint("XdevsBridge.mint gas", g - gasleft());
        assertEq(t.balanceOf(recipient), 100e18);
        vm.expectRevert(abi.encodeWithSelector(BridgeBase.AlreadyProcessed.selector, 0));
        b.mint(p);
    }

    function test_mint_rejects_wrong_emitter_failed_tx_other_event() public {
        (XdevsBridge b,) = _xdevsSide();
        vm.expectRevert(abi.encodeWithSelector(BridgeBase.WrongEmitter.selector, bridgeAddr));
        b.mint(_lockProof(2));
        vm.expectRevert(EthProofs.ReceiptFailed.selector);
        b.mint(_lockProof(3));
        vm.expectRevert(abi.encodeWithSelector(BridgeBase.WrongEmitter.selector, token));
        b.mint(_lockProof(0)); // an ERC-20 Transfer of the token itself
        // A proof against a block the light client does not know.
        BridgeBase.EventProof memory p = _lockProof(1);
        p.blockNumber = 7;
        vm.expectRevert("unknown");
        b.mint(p);
    }

    function test_mint_requires_canonical_token() public {
        CanonicalTokenFactory f = new CanonicalTokenFactory(address(this), 30 days, 365 days);
        XdevsBridge b = new XdevsBridge(oracle, 11155111, guardian, f);
        b.setCounterpart(vaultAddr);
        vm.expectRevert(abi.encodeWithSelector(XdevsBridge.NoCanonicalToken.selector, token));
        b.mint(_lockProof(1));
    }

    function test_burn_emits_release_message() public {
        (XdevsBridge b, XDRC20Bridged t) = _xdevsSide();
        b.mint(_lockProof(1));
        vm.prank(recipient);
        vm.expectEmit(address(b));
        emit XdevsBridge.BurnedForRelease(0, token, recipient, user, 40e18);
        b.burn(address(t), 40e18, user);
        assertEq(t.balanceOf(recipient), 60e18);
        // Only canonical tokens of this bridge.
        vm.expectRevert(abi.encodeWithSelector(XdevsBridge.NotBridgedHere.selector, address(0xdead)));
        b.burn(address(0xdead), 1, user);
    }

    function test_release_from_proven_burn() public {
        // Vault at the fixture's token address so the proven originToken is the escrowed token.
        deployCodeTo("BridgeEndpoints.t.sol:TestToken", token);
        SepoliaTokenVault v = new SepoliaTokenVault(oracle, 717171, guardian);
        v.setCounterpart(bridgeAddr);
        TestToken(token).mint(user, 100e18);
        vm.startPrank(user);
        TestToken(token).approve(address(v), 100e18);
        v.lock(token, 100e18, recipient);
        vm.stopPrank();
        assertEq(v.escrowed(token), 100e18);

        BridgeBase.EventProof memory p = _proof(2, 0, ".bridge.burnProof");
        uint256 g = gasleft();
        v.release(p);
        emit log_named_uint("SepoliaTokenVault.release gas", g - gasleft());
        // The fixture burn (sender `user` on xDEVS) releases 40 to `recipient`.
        assertEq(TestToken(token).balanceOf(recipient), 40e18);
        assertEq(v.escrowed(token), 60e18);
        vm.expectRevert(abi.encodeWithSelector(BridgeBase.AlreadyProcessed.selector, 0));
        v.release(p);
    }

    function test_release_cannot_exceed_escrow() public {
        deployCodeTo("BridgeEndpoints.t.sol:TestToken", token);
        SepoliaTokenVault v = new SepoliaTokenVault(oracle, 717171, guardian);
        v.setCounterpart(bridgeAddr);
        TestToken(token).mint(address(v), 1000e18); // donated, not locked
        vm.expectRevert(); // escrowed underflow
        v.release(_proof(2, 0, ".bridge.burnProof"));
    }

    function test_guardian_can_only_pause() public {
        (XdevsBridge b,) = _xdevsSide();
        vm.expectRevert(BridgeBase.NotGuardian.selector);
        b.setPaused(true);
        vm.prank(guardian);
        b.setPaused(true);
        vm.expectRevert(BridgeBase.IsPaused.selector);
        b.mint(_lockProof(1));
        vm.prank(guardian);
        b.setPaused(false);
        b.mint(_lockProof(1));
        // The counterpart is set once.
        vm.expectRevert(BridgeBase.CounterpartAlreadySet.selector);
        b.setCounterpart(address(1));
        vm.prank(guardian);
        vm.expectRevert(BridgeBase.NotDeployer.selector);
        b.setCounterpart(address(1));
    }
}
