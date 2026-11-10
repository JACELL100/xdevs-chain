// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {XNSRegistry} from "../../src/periphery/xns/XNSRegistry.sol";
import {XNSLabel} from "../../src/periphery/xns/XNSLabel.sol";

contract XNSHandler is Test {
    XNSRegistry public xns;
    string[] public labels;
    address[] internal actors;

    /// Set if a registration ever succeeded while the previous one was still active or in grace.
    bool public doubleRegistration;
    /// Set if a registration succeeded while a confusable name was active or in grace.
    bool public confusableCoexist;
    uint256 public registrations;

    constructor(XNSRegistry xns_) {
        xns = xns_;
        labels.push("paypal");
        labels.push("paypa1");
        labels.push("alice");
        labels.push("al1ce");
        labels.push("metamask");
        labels.push("rnetamask");
        labels.push("wallet");
        labels.push("vvallet");
        labels.push("doge");
        labels.push("cloge");
        labels.push("bob");
        labels.push("xyz-1");
        actors.push(makeAddr("a0"));
        actors.push(makeAddr("a1"));
        actors.push(makeAddr("a2"));
    }

    function labelCount() external view returns (uint256) {
        return labels.length;
    }

    function _occupied(string memory l) internal view returns (bool) {
        uint256 exp = xns.nameExpires(uint256(keccak256(bytes(l))));
        return exp != 0 && block.timestamp < exp + xns.GRACE_PERIOD();
    }

    function register(uint256 labelSeed, uint256 actorSeed, uint256 years_) external {
        string memory l = labels[labelSeed % labels.length];
        address who = actors[actorSeed % actors.length];
        uint256 duration = bound(years_, 1, 3) * 365 days;
        bool wasOccupied = _occupied(l);
        bool confusableOccupied;
        bytes32 sk = XNSLabel.skeleton(l);
        for (uint256 i; i < labels.length; ++i) {
            string memory o = labels[i];
            if (keccak256(bytes(o)) != keccak256(bytes(l)) && XNSLabel.skeleton(o) == sk && _occupied(o)) {
                confusableOccupied = true;
            }
        }
        vm.prank(who);
        try xns.register(l, who, duration) {
            registrations++;
            if (wasOccupied) doubleRegistration = true;
            if (confusableOccupied) confusableCoexist = true;
        } catch {}
    }

    function renew(uint256 labelSeed, uint256 years_) external {
        string memory l = labels[labelSeed % labels.length];
        try xns.renew(l, bound(years_, 1, 2) * 365 days) {} catch {}
    }

    function warp(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1 hours, 400 days));
    }

    function transfer(uint256 labelSeed, uint256 actorSeed) external {
        string memory l = labels[labelSeed % labels.length];
        bytes32 n = xns.namehash(string.concat(l, ".xdevs"));
        address o = xns.owner(n);
        if (o == address(0)) return;
        vm.prank(o);
        xns.setOwner(n, actors[actorSeed % actors.length]);
    }
}

contract XNSRegistryInvariantTest is Test {
    XNSRegistry internal xns;
    XNSHandler internal handler;

    function setUp() public {
        vm.warp(1_800_000_000);
        xns = new XNSRegistry();
        xns.initialize(makeAddr("gov"), makeAddr("treasury"), 0, new string[](0));
        handler = new XNSHandler(xns);
        targetContract(address(handler));
    }

    /// A name can never be registered while an earlier registration is active (or in grace).
    function invariant_noDoubleRegistration() public view {
        assertFalse(handler.doubleRegistration());
    }

    /// Two confusable names are never simultaneously registered.
    function invariant_noConfusableCoexistence() public view {
        assertFalse(handler.confusableCoexist());
    }

    /// Ownership / resolution never outlive the expiry.
    function invariant_expiredNamesDoNotResolve() public view {
        uint256 n = handler.labelCount();
        for (uint256 i; i < n; ++i) {
            string memory l = handler.labels(i);
            bytes32 node = xns.namehash(string.concat(l, ".xdevs"));
            uint256 exp = xns.nameExpires(uint256(keccak256(bytes(l))));
            if (block.timestamp >= exp) {
                assertEq(xns.owner(node), address(0));
                assertEq(xns.resolver(node), address(0));
                assertEq(xns.addr(node), address(0));
            }
        }
    }
}
