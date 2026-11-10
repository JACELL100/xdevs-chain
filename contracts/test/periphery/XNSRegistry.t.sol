// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {GenesisInitializable} from "../../src/periphery/common/GenesisInitializable.sol";
import {XNSRegistry} from "../../src/periphery/xns/XNSRegistry.sol";
import {XNSLabel} from "../../src/periphery/xns/XNSLabel.sol";
import {
    IAddrResolver,
    IAddressResolver,
    ITextResolver,
    INameResolver,
    IVersionableResolver,
    IXNSRegistrar,
    IENSRegistry
} from "../../src/periphery/xns/IXNS.sol";

/// @dev External wrapper so library reverts can be tested with expectRevert.
contract LabelHarness {
    function check(string calldata l) external pure returns (XNSLabel.Invalid) {
        return XNSLabel.check(l);
    }

    function skeleton(string calldata l) external pure returns (bytes32) {
        return XNSLabel.skeleton(l);
    }

    function namehash(string calldata n) external pure returns (bytes32) {
        return XNSLabel.namehash(n);
    }

    function hexAddressLabelhash(address a) external pure returns (bytes32) {
        return XNSLabel.hexAddressLabelhash(a);
    }
}

contract RejectingTreasury {
    receive() external payable {
        revert("no");
    }
}

contract XNSRegistryTest is Test {
    XNSRegistry internal xns;
    LabelHarness internal lib;

    address internal gov = makeAddr("governance");
    address internal treasury = makeAddr("treasury");
    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");
    address internal mallory = makeAddr("mallory");

    uint256 internal constant YEAR = 365 days;
    bytes32 internal constant ETH_NODE = 0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae;
    bytes32 internal constant FOO_ETH_NODE =
        0xde9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f;

    function setUp() public {
        vm.warp(1_800_000_000);
        lib = new LabelHarness();
        xns = new XNSRegistry();
        string[] memory res = new string[](2);
        res[0] = "xdevs";
        res[1] = "official";
        xns.initialize(gov, treasury, 0, res);
    }

    function _node(string memory label) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(XNSLabelNodes.TLD, keccak256(bytes(label))));
    }

    function _register(string memory label, address owner) internal returns (bytes32 n) {
        (n,) = xns.register(label, owner, YEAR);
    }

    // ---------------------------------------------------------------- ENS compatibility vectors

    function test_namehashVectors() public view {
        assertEq(lib.namehash(""), bytes32(0));
        assertEq(lib.namehash("eth"), ETH_NODE);
        assertEq(lib.namehash("foo.eth"), FOO_ETH_NODE);
        assertEq(xns.namehash("eth"), ETH_NODE);
        assertEq(xns.namehash("xdevs"), xns.TLD_NODE());
        assertEq(xns.namehash("reverse"), xns.REVERSE_NODE());
        assertEq(xns.namehash("addr.reverse"), xns.ADDR_REVERSE_NODE());
        assertEq(xns.TLD_LABELHASH(), keccak256("xdevs"));
        assertEq(xns.namehash("alice.xdevs"), _node("alice"));
    }

    function test_reverseNodeMatchesENS() public view {
        // ENS sha3HexAddress: keccak of lowercase hex without 0x.
        address a = 0x314159265dD8dbb310642f98f50C066173C1259b;
        bytes32 expected = keccak256("314159265dd8dbb310642f98f50c066173c1259b");
        assertEq(lib.hexAddressLabelhash(a), expected);
        assertEq(xns.node(a), keccak256(abi.encodePacked(xns.ADDR_REVERSE_NODE(), expected)));
        assertEq(xns.node(a), xns.namehash("314159265dd8dbb310642f98f50c066173c1259b.addr.reverse"));
    }

    function test_interfaceIds() public view {
        assertEq(type(IAddrResolver).interfaceId, bytes4(0x3b3b57de));
        assertEq(type(IAddressResolver).interfaceId, bytes4(0xf1cb7e06));
        assertEq(type(ITextResolver).interfaceId, bytes4(0x59d1d43c));
        assertEq(type(INameResolver).interfaceId, bytes4(0x691f3431));
        assertEq(type(IVersionableResolver).interfaceId, bytes4(0xd700ff33));
        assertTrue(xns.supportsInterface(0x01ffc9a7));
        assertTrue(xns.supportsInterface(0x3b3b57de));
        assertTrue(xns.supportsInterface(0xf1cb7e06));
        assertTrue(xns.supportsInterface(0x59d1d43c));
        assertTrue(xns.supportsInterface(0x691f3431));
        assertTrue(xns.supportsInterface(0xd700ff33));
        assertFalse(xns.supportsInterface(0xffffffff));
    }

    function test_initialState() public view {
        assertEq(xns.owner(bytes32(0)), address(xns));
        assertEq(xns.owner(xns.TLD_NODE()), address(xns));
        assertEq(xns.resolver(xns.TLD_NODE()), address(xns));
        assertEq(xns.owner(xns.ADDR_REVERSE_NODE()), address(xns));
        assertEq(xns.governance(), gov);
        assertEq(xns.treasury(), treasury);
        assertTrue(xns.reserved(keccak256("xdevs")));
        assertTrue(xns.initialized());
    }

    // ---------------------------------------------------------------- initialization / genesis

    function test_initializeOnlyOnce() public {
        vm.expectRevert(GenesisInitializable.AlreadyInitialized.selector);
        xns.initialize(gov, treasury, 0, new string[](0));
    }

    function test_initializeRejectsStrangers() public {
        XNSRegistry fresh = new XNSRegistry();
        vm.prank(mallory);
        vm.expectRevert(abi.encodeWithSelector(GenesisInitializable.NotInitializer.selector, mallory));
        fresh.initialize(mallory, mallory, 0, new string[](0));
    }

    function test_registerBeforeInitReverts() public {
        XNSRegistry fresh = new XNSRegistry();
        vm.expectRevert(GenesisInitializable.NotInitialized.selector);
        fresh.register("alice", alice, YEAR);
    }

    /// Simulates the Rust genesis builder: runtime bytecode copied to 0x…1004, constructor never ran.
    function test_genesisPlacement() public {
        address predeploy = Predeploys.XNS_REGISTRY;
        vm.etch(predeploy, vm.getDeployedCode("XNSRegistry.sol:XNSRegistry"));
        XNSRegistry g = XNSRegistry(predeploy);
        assertFalse(g.initialized());

        // The deployer immutable is zero in genesis code: nobody but SYSTEM_ADDRESS may initialize.
        vm.prank(address(this));
        vm.expectRevert(abi.encodeWithSelector(GenesisInitializable.NotInitializer.selector, address(this)));
        g.initialize(gov, treasury, 0, new string[](0));

        vm.prank(Predeploys.SYSTEM_ADDRESS);
        g.initialize(Predeploys.GOVERNANCE, Predeploys.TREASURY, 0, new string[](0));
        assertEq(g.owner(g.TLD_NODE()), predeploy);

        (bytes32 n,) = g.register("alice", alice, YEAR);
        assertEq(g.resolver(n), predeploy);
        assertEq(g.addr(n), alice);
    }

    // ---------------------------------------------------------------- registration

    function test_register() public {
        vm.expectEmit(true, true, false, true, address(xns));
        emit IXNSRegistrar.NameRegistered("alice", keccak256("alice"), alice, block.timestamp + YEAR, 0);
        bytes32 n = _register("alice", alice);
        assertEq(n, xns.namehash("alice.xdevs"));
        assertEq(xns.owner(n), alice);
        assertEq(xns.resolver(n), address(xns));
        assertEq(xns.addr(n), alice);
        assertEq(xns.addr(n, 60), abi.encodePacked(alice));
        assertTrue(xns.recordExists(n));
        assertEq(xns.nameExpires(uint256(keccak256("alice"))), block.timestamp + YEAR);
        assertFalse(xns.available("alice"));
    }

    function test_cannotRegisterTwiceWhileActive() public {
        _register("alice", alice);
        vm.prank(mallory);
        vm.expectRevert();
        xns.register("alice", mallory, YEAR);

        // Still blocked during the grace period.
        vm.warp(block.timestamp + YEAR + 89 days);
        vm.prank(mallory);
        vm.expectRevert();
        xns.register("alice", mallory, YEAR);
        assertFalse(xns.available("alice"));
    }

    function test_durationBounds() public {
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.InvalidDuration.selector, YEAR - 1));
        xns.register("alice", alice, YEAR - 1);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.InvalidDuration.selector, 10 * YEAR + 1));
        xns.register("alice", alice, 10 * YEAR + 1);
        vm.expectRevert(XNSRegistry.ZeroAddress.selector);
        xns.register("alice", address(0), YEAR);
    }

    function test_expiryStopsResolutionAndGraceAllowsRenew() public {
        bytes32 n = _register("alice", alice);
        vm.prank(alice);
        xns.setText(n, "url", "https://alice.example");
        uint256 exp = block.timestamp + YEAR;

        vm.warp(exp); // expired: nothing resolves, owner cannot manage
        assertEq(xns.owner(n), address(0));
        assertEq(xns.resolver(n), address(0));
        assertEq(xns.addr(n), address(0));
        assertEq(xns.text(n, "url"), "");
        assertFalse(xns.recordExists(n));
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.Unauthorized.selector, n, alice));
        xns.setAddr(n, bob);

        // Anyone can renew during grace; records come back unchanged.
        vm.prank(bob);
        xns.renew("alice", YEAR);
        assertEq(xns.owner(n), alice);
        assertEq(xns.addr(n), alice);
        assertEq(xns.text(n, "url"), "https://alice.example");
        assertEq(xns.nameExpires(uint256(keccak256("alice"))), exp + YEAR);
    }

    function test_renewAfterGraceFails() public {
        _register("alice", alice);
        vm.warp(block.timestamp + YEAR + 90 days);
        vm.expectRevert();
        xns.renew("alice", YEAR);
        vm.expectRevert();
        xns.renew("neverregistered", YEAR);
    }

    function test_renewHorizonCapped() public {
        _register("alice", alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.InvalidDuration.selector, 10 * YEAR));
        xns.renew("alice", 10 * YEAR);
        xns.renew("alice", 9 * YEAR);
    }

    function test_reRegistrationAfterGraceWipesEverything() public {
        bytes32 n = _register("alice", alice);
        vm.startPrank(alice);
        xns.setText(n, "avatar", "x");
        bytes32 sub = xns.setSubnodeOwner(n, keccak256("pay"), alice);
        xns.setResolver(sub, address(xns));
        xns.setAddr(sub, alice);
        vm.stopPrank();
        assertEq(xns.addr(sub), alice);

        vm.warp(block.timestamp + YEAR + 90 days);
        assertTrue(xns.available("alice"));
        (bytes32 n2,) = xns.register("alice", bob, YEAR);
        assertEq(n2, n);
        assertEq(xns.owner(n), bob);
        assertEq(xns.addr(n), bob, "addr reset to new owner");
        assertEq(xns.text(n, "avatar"), "", "old text wiped");
        assertEq(xns.owner(sub), address(0), "old subname gone");
        assertEq(xns.addr(sub), address(0), "old subname addr gone");

        // Old owner cannot touch the stale subname; new owner recreates it from scratch.
        vm.prank(alice);
        vm.expectRevert();
        xns.setAddr(sub, alice);
        vm.prank(bob);
        xns.setSubnodeOwner(n, keccak256("pay"), bob);
        assertEq(xns.owner(sub), bob);
        assertEq(xns.resolver(sub), address(0));
        assertEq(xns.addr(sub), address(0));
    }

    function test_fees() public {
        vm.prank(gov);
        xns.setFeePerYear(1 ether);
        assertEq(xns.rentPrice(YEAR), 1 ether);
        assertEq(xns.rentPrice(2 * YEAR), 2 ether);
        vm.deal(alice, 10 ether);

        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.WrongFee.selector, 1 ether, 0));
        xns.register("alice", alice, YEAR);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.WrongFee.selector, 1 ether, 2 ether));
        xns.register{value: 2 ether}("alice", alice, YEAR);

        vm.prank(alice);
        xns.register{value: 1 ether}("alice", alice, YEAR);
        assertEq(treasury.balance, 1 ether);
        vm.prank(alice);
        xns.renew{value: 2 ether}("alice", 2 * YEAR);
        assertEq(treasury.balance, 3 ether);
        assertEq(address(xns).balance, 0);
    }

    function test_zeroFeeRejectsValue() public {
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.WrongFee.selector, 0, 1));
        xns.register{value: 1}("alice", alice, YEAR);
    }

    function test_treasuryFailureReverts() public {
        RejectingTreasury rt = new RejectingTreasury();
        vm.startPrank(gov);
        xns.setTreasury(address(rt));
        xns.setFeePerYear(1 ether);
        vm.stopPrank();
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(XNSRegistry.FeeTransferFailed.selector);
        xns.register{value: 1 ether}("alice", alice, YEAR);
    }

    function test_governance() public {
        vm.prank(mallory);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.NotGovernance.selector, mallory));
        xns.setFeePerYear(1);
        vm.prank(mallory);
        vm.expectRevert();
        xns.setReserved("bank", true);
        vm.prank(mallory);
        vm.expectRevert();
        xns.setTreasury(mallory);
        vm.prank(mallory);
        vm.expectRevert();
        xns.registerReserved("xdevs", mallory, YEAR);

        vm.prank(gov);
        xns.setGovernance(bob);
        assertEq(xns.governance(), bob);
        vm.prank(gov);
        vm.expectRevert();
        xns.setFeePerYear(1);
        vm.prank(bob);
        vm.expectRevert(XNSRegistry.ZeroAddress.selector);
        xns.setGovernance(address(0));
    }

    function test_reservedLabels() public {
        assertFalse(xns.available("xdevs"));
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.NameReserved.selector, "xdevs"));
        xns.register("xdevs", alice, YEAR);

        vm.prank(gov);
        (bytes32 n,) = xns.registerReserved("xdevs", gov, YEAR);
        assertEq(xns.owner(n), gov);

        vm.prank(gov);
        xns.setReserved("bank", true);
        vm.expectRevert();
        xns.register("bank", alice, YEAR);
        vm.prank(gov);
        xns.setReserved("bank", false);
        _register("bank", alice);
    }

    // ---------------------------------------------------------------- normalization & confusables

    function test_validLabels() public view {
        assertTrue(xns.valid("abc"));
        assertTrue(xns.valid("alice"));
        assertTrue(xns.valid("a-b"));
        assertTrue(xns.valid("web3"));
        assertTrue(xns.valid("123"));
        assertTrue(xns.valid("a1-b2-c3"));
        assertTrue(xns.valid("abcdefghijklmnopqrstuvwxyz0123456789-abcdefghijklmnopqrstuvwxyz")); // 63 bytes
    }

    function test_invalidLabels() public view {
        assertEq(uint8(lib.check("ab")), uint8(XNSLabel.Invalid.TooShort));
        assertEq(uint8(lib.check("")), uint8(XNSLabel.Invalid.TooShort));
        assertEq(
            uint8(lib.check("abcdefghijklmnopqrstuvwxyz0123456789-abcdefghijklmnopqrstuvwxyz0")),
            uint8(XNSLabel.Invalid.TooLong)
        );
        // uppercase is rejected, not lowercased
        assertEq(uint8(lib.check("Alice")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check("ALICE")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        // separators / punctuation / whitespace
        assertEq(uint8(lib.check("ali.ce")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check("ali_ce")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check("ali ce")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check("alice\x00")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        // unicode: Cyrillic "а" (U+0430) homograph of Latin "a", accented, zero-width joiner, emoji
        assertEq(uint8(lib.check(unicode"аlice")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check(unicode"alicé")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check(unicode"ali‍ce")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        assertEq(uint8(lib.check(unicode"🚀rocket")), uint8(XNSLabel.Invalid.ForbiddenCharacter));
        // hyphen rules (incl. punycode prefix)
        assertEq(uint8(lib.check("-alice")), uint8(XNSLabel.Invalid.BadHyphen));
        assertEq(uint8(lib.check("alice-")), uint8(XNSLabel.Invalid.BadHyphen));
        assertEq(uint8(lib.check("al--ice")), uint8(XNSLabel.Invalid.BadHyphen));
        assertEq(uint8(lib.check("xn--80ak6aa92e")), uint8(XNSLabel.Invalid.BadHyphen));
    }

    function test_registerRejectsInvalid() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                XNSLabel.InvalidLabel.selector, "Alice", XNSLabel.Invalid.ForbiddenCharacter
            )
        );
        xns.register("Alice", alice, YEAR);
        vm.expectRevert(
            abi.encodeWithSelector(
                XNSLabel.InvalidLabel.selector, unicode"аlice", XNSLabel.Invalid.ForbiddenCharacter
            )
        );
        xns.register(unicode"аlice", alice, YEAR);
        vm.expectRevert();
        xns.register("ab", alice, YEAR);
        assertFalse(xns.available("Alice"));
    }

    function test_confusableSkeletons() public view {
        assertEq(lib.skeleton("paypa1"), lib.skeleton("paypal"));
        assertEq(lib.skeleton("g00gle"), lib.skeleton("google"));
        assertEq(lib.skeleton("rnetamask"), lib.skeleton("metamask"));
        assertEq(lib.skeleton("vvallet"), lib.skeleton("wallet"));
        assertEq(lib.skeleton("cloge"), lib.skeleton("doge"));
        assertTrue(lib.skeleton("alice") != lib.skeleton("alicf"));
        assertTrue(lib.skeleton("pay-pal") != lib.skeleton("paypal"));
    }

    function test_confusableRegistrationBlocked() public {
        bytes32 paypal = _register("paypal", alice);
        assertFalse(xns.available("paypa1"));
        vm.prank(mallory);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.ConfusableWithExisting.selector, "paypa1", paypal));
        xns.register("paypa1", mallory, YEAR);
        // First come first served: "rnetamask" is free, but then "metamask" is blocked.
        bytes32 rn = _register("rnetamask", mallory);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.ConfusableWithExisting.selector, "metamask", rn));
        xns.register("metamask", alice, YEAR);
    }

    function test_confusableReleasedAfterGrace() public {
        _register("paypal", alice);
        vm.warp(block.timestamp + YEAR + 90 days);
        assertTrue(xns.available("paypa1"));
        _register("paypa1", bob);
        assertFalse(xns.available("paypal"));
    }

    /// Reference model of the normalization rules, checked against the library on random bytes.
    function testFuzz_normalizationMatchesModel(bytes memory raw) public view {
        vm.assume(raw.length <= 80);
        bool ok = raw.length >= 3 && raw.length <= 63;
        for (uint256 i; ok && i < raw.length; ++i) {
            uint8 c = uint8(raw[i]);
            bool allowed = (c >= 97 && c <= 122) || (c >= 48 && c <= 57) || c == 45;
            if (!allowed) ok = false;
            if (c == 45 && (i == 0 || i == raw.length - 1 || uint8(raw[i - 1]) == 45)) ok = false;
        }
        assertEq(xns.valid(string(raw)), ok);
    }

    /// Any byte outside [a-z0-9-] (uppercase, unicode, punctuation) makes a label invalid.
    function testFuzz_forbiddenByteRejected(uint8 c, uint8 pos) public view {
        vm.assume(!((c >= 97 && c <= 122) || (c >= 48 && c <= 57) || c == 45));
        bytes memory l = bytes("abcdefgh");
        l[pos % 8] = bytes1(c);
        assertFalse(xns.valid(string(l)));
    }

    function testFuzz_uppercaseRejected(uint8 c, uint8 pos) public view {
        c = uint8(bound(c, 65, 90)); // A-Z
        bytes memory l = bytes("abcdefgh");
        l[pos % 8] = bytes1(c);
        assertFalse(xns.valid(string(l)));
    }

    // ---------------------------------------------------------------- ENS registry operations

    function test_registryOps() public {
        bytes32 n = _register("alice", alice);
        bytes32 sub = keccak256(abi.encodePacked(n, keccak256("www")));

        vm.prank(mallory);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.Unauthorized.selector, n, mallory));
        xns.setSubnodeOwner(n, keccak256("www"), mallory);

        vm.startPrank(alice);
        vm.expectEmit(true, true, false, true, address(xns));
        emit IENSRegistry.NewOwner(n, keccak256("www"), bob);
        xns.setSubnodeOwner(n, keccak256("www"), bob);
        xns.setTTL(n, 300);
        xns.setSubnodeRecord(n, keccak256("api"), alice, address(xns), 60);
        vm.stopPrank();
        assertEq(xns.owner(sub), bob);
        assertEq(xns.ttl(n), 300);
        assertEq(xns.ttl(xns.namehash("api.alice.xdevs")), 60);
        assertEq(xns.resolver(xns.namehash("api.alice.xdevs")), address(xns));

        // operators
        vm.prank(alice);
        xns.setApprovalForAll(bob, true);
        assertTrue(xns.isApprovedForAll(alice, bob));
        vm.prank(bob);
        xns.setAddr(n, bob);
        assertEq(xns.addr(n), bob);

        // transfer
        vm.prank(alice);
        xns.setOwner(n, bob);
        assertEq(xns.owner(n), bob);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.Unauthorized.selector, n, alice));
        xns.setTTL(n, 1);
        vm.prank(bob);
        xns.setRecord(n, bob, address(0xBEEF), 7);
        assertEq(xns.resolver(n), address(0xBEEF));
        assertEq(xns.ttl(n), 7);
    }

    function test_subnodeExpiresWithParent() public {
        bytes32 n = _register("alice", alice);
        vm.prank(alice);
        bytes32 sub = xns.setSubnodeOwner(n, keccak256("www"), bob);
        assertEq(xns.owner(sub), bob);
        vm.warp(block.timestamp + YEAR);
        assertEq(xns.owner(sub), address(0));
        vm.prank(bob);
        vm.expectRevert();
        xns.setAddr(sub, bob);
    }

    function test_protocolNodesLocked() public {
        bytes32 tld = xns.TLD_NODE();
        vm.startPrank(gov);
        vm.expectRevert();
        xns.setSubnodeOwner(tld, keccak256("alice"), gov);
        vm.expectRevert();
        xns.setSubnodeOwner(bytes32(0), keccak256("xdevs"), gov);
        vm.expectRevert();
        xns.setOwner(tld, gov);
        vm.stopPrank();
    }

    // ---------------------------------------------------------------- resolver

    function test_resolverRecords() public {
        bytes32 n = _register("alice", alice);
        uint256 xdevsCoin = 0x80000000 | block.chainid;
        vm.startPrank(alice);
        xns.setAddr(n, xdevsCoin, abi.encodePacked(bob));
        xns.setAddr(n, 0, hex"0014deadbeef"); // BTC script, arbitrary length allowed
        xns.setText(n, "com.twitter", "@alice");
        vm.expectRevert(abi.encodeWithSelector(XNSRegistry.InvalidAddressLength.selector, 60, 3));
        xns.setAddr(n, 60, hex"010203");
        vm.expectRevert();
        xns.setAddr(n, xdevsCoin, hex"010203");
        vm.stopPrank();
        assertEq(xns.addr(n, xdevsCoin), abi.encodePacked(bob));
        assertEq(xns.addr(n, 0), hex"0014deadbeef");
        assertEq(xns.text(n, "com.twitter"), "@alice");

        vm.prank(alice);
        xns.clearRecords(n);
        assertEq(xns.recordVersions(n), 1);
        assertEq(xns.addr(n), address(0));
        assertEq(xns.text(n, "com.twitter"), "");
    }

    // ---------------------------------------------------------------- reverse / primary names

    function test_reverseAndPrimaryName() public {
        _register("alice", alice);
        vm.prank(alice);
        bytes32 rnode = xns.setName("alice.xdevs");
        assertEq(rnode, xns.node(alice));
        assertEq(xns.owner(rnode), alice);
        assertEq(xns.resolver(rnode), address(xns));
        assertEq(xns.name(rnode), "alice.xdevs");
        assertEq(xns.primaryName(alice), "alice.xdevs");

        // Pointing the name elsewhere invalidates the primary name (forward check at read time).
        bytes32 n = xns.namehash("alice.xdevs");
        vm.prank(alice);
        xns.setAddr(n, bob);
        assertEq(xns.primaryName(alice), "");
        assertEq(xns.name(rnode), "alice.xdevs"); // raw record unchanged

        // Expired names are not primary names either.
        vm.prank(alice);
        xns.setAddr(n, alice);
        assertEq(xns.primaryName(alice), "alice.xdevs");
        vm.warp(block.timestamp + YEAR);
        assertEq(xns.primaryName(alice), "");

        // clearing
        vm.prank(alice);
        xns.setName("");
        assertEq(xns.name(rnode), "");
    }

    function test_cannotClaimSomeoneElsesName() public {
        _register("alice", alice);
        vm.prank(mallory);
        vm.expectRevert(
            abi.encodeWithSelector(XNSRegistry.ReverseNameMismatch.selector, "alice.xdevs", alice)
        );
        xns.setName("alice.xdevs");
        assertEq(xns.primaryName(mallory), "");

        // Even if mallory writes the raw record via the resolver, primaryName refuses it.
        vm.startPrank(mallory);
        bytes32 rnode = xns.claim(mallory);
        xns.setName(rnode, "alice.xdevs");
        vm.stopPrank();
        assertEq(xns.name(rnode), "alice.xdevs");
        assertEq(xns.primaryName(mallory), "");
    }

    function test_reverseClaimToOtherOwner() public {
        vm.prank(alice);
        bytes32 rnode = xns.claim(bob);
        assertEq(xns.owner(rnode), bob);
        vm.prank(bob);
        xns.setName(rnode, "whatever");
        assertEq(xns.name(rnode), "whatever");
    }
}

library XNSLabelNodes {
    bytes32 internal constant TLD = 0xa058e3bd8a205b95bca4815c553cac26b3228e39fce9d34f8417f3665c626edd;
}

contract ExternalResolver {
    mapping(bytes32 => address) public a;

    function set(bytes32 n, address x) external {
        a[n] = x;
    }

    function addr(bytes32 n) external view returns (address payable) {
        return payable(a[n]);
    }
}

contract XNSExternalResolverTest is Test {
    function test_primaryNameFollowsExternalResolver() public {
        vm.warp(1_800_000_000);
        XNSRegistry xns = new XNSRegistry();
        xns.initialize(address(1), address(2), 0, new string[](0));
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        (bytes32 n,) = xns.register("alice", alice, 365 days);
        ExternalResolver ext = new ExternalResolver();
        vm.prank(alice);
        xns.setResolver(n, address(ext));
        // Local addr record still says alice, but the authoritative resolver says nothing.
        assertEq(xns.resolveAddr(n), address(0));
        vm.prank(alice);
        vm.expectRevert(
            abi.encodeWithSelector(XNSRegistry.ReverseNameMismatch.selector, "alice.xdevs", address(0))
        );
        xns.setName("alice.xdevs");

        ext.set(n, bob);
        assertEq(xns.resolveAddr(n), bob);
        vm.prank(bob);
        xns.setName("alice.xdevs");
        assertEq(xns.primaryName(bob), "alice.xdevs");

        // EOA as "resolver" or a reverting resolver resolves to nothing.
        vm.prank(alice);
        xns.setResolver(n, address(0xdead));
        assertEq(xns.resolveAddr(n), address(0));
        assertEq(xns.primaryName(bob), "");
    }
}
