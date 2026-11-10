// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {Permit2Expiring} from "../../src/periphery/permit2/Permit2Expiring.sol";
import {IPermit2Expiring} from "../../src/periphery/permit2/IPermit2Expiring.sol";
import {MockSmartAccount, LockedAccountMock} from "./XDRC20.t.sol";

contract MockERC20 is ERC20 {
    constructor() ERC20("Mock", "MCK") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

abstract contract Permit2Base is Test {
    Permit2Expiring internal p2;
    MockERC20 internal tokenA;
    MockERC20 internal tokenB;
    uint256 internal aliceKey = 0xA11CE;
    address internal alice;
    address internal spender = makeAddr("spender");
    address internal bob = makeAddr("bob");

    uint48 internal constant DAY = 1 days;

    function setUp() public virtual {
        vm.warp(1_800_000_000);
        alice = vm.addr(aliceKey);
        // Genesis-style placement: runtime code at the predeploy address, no constructor.
        vm.etch(Predeploys.PERMIT2_EXPIRING, vm.getDeployedCode("Permit2Expiring.sol:Permit2Expiring"));
        p2 = Permit2Expiring(Predeploys.PERMIT2_EXPIRING);
        tokenA = new MockERC20();
        tokenB = new MockERC20();
        tokenA.mint(alice, 1000 ether);
        tokenB.mint(alice, 1000 ether);
        vm.startPrank(alice);
        tokenA.approve(address(p2), type(uint256).max);
        tokenB.approve(address(p2), type(uint256).max);
        vm.stopPrank();
    }

    function _now() internal view returns (uint48) {
        return uint48(block.timestamp);
    }

    function _signSingle(uint256 key, IPermit2Expiring.PermitSingle memory p)
        internal
        view
        returns (bytes memory)
    {
        bytes32 detailsHash = keccak256(
            abi.encode(
                p2.PERMIT_DETAILS_TYPEHASH(),
                p.details.token,
                p.details.amount,
                p.details.expiration,
                p.details.nonce
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(p2.PERMIT_SINGLE_TYPEHASH(), detailsHash, p.spender, p.sigDeadline, p.epoch)
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", p2.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function _signBatch(uint256 key, IPermit2Expiring.PermitBatch memory p)
        internal
        view
        returns (bytes memory)
    {
        bytes32[] memory hs = new bytes32[](p.details.length);
        for (uint256 i; i < hs.length; ++i) {
            IPermit2Expiring.PermitDetails memory d = p.details[i];
            hs[i] =
                keccak256(abi.encode(p2.PERMIT_DETAILS_TYPEHASH(), d.token, d.amount, d.expiration, d.nonce));
        }
        bytes32 structHash = keccak256(
            abi.encode(
                p2.PERMIT_BATCH_TYPEHASH(), keccak256(abi.encodePacked(hs)), p.spender, p.sigDeadline, p.epoch
            )
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", p2.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function _single(address token, uint160 amount, uint48 expiration, uint48 nonce, uint48 epoch)
        internal
        view
        returns (IPermit2Expiring.PermitSingle memory)
    {
        return IPermit2Expiring.PermitSingle({
            details: IPermit2Expiring.PermitDetails({
                token: token, amount: amount, expiration: expiration, nonce: nonce
            }),
            spender: spender,
            sigDeadline: block.timestamp + 1 hours,
            epoch: epoch
        });
    }
}

contract Permit2ExpiringTest is Permit2Base {
    // ---------------------------------------------------------------- domain

    function test_domainSeparatorIsDynamic() public {
        bytes32 expected = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,uint256 chainId,address verifyingContract)"),
                keccak256("Permit2Expiring"),
                block.chainid,
                Predeploys.PERMIT2_EXPIRING
            )
        );
        assertEq(p2.DOMAIN_SEPARATOR(), expected);
        vm.chainId(4242);
        assertTrue(p2.DOMAIN_SEPARATOR() != expected);
        // Same bytecode at another address -> different domain (nothing cached).
        address other = makeAddr("other");
        vm.etch(other, address(p2).code);
        assertTrue(Permit2Expiring(other).DOMAIN_SEPARATOR() != p2.DOMAIN_SEPARATOR());
    }

    function test_noConstructorStateNeeded() public view {
        assertEq(p2.MAX_EXPIRY_WINDOW(), 365 days);
        assertEq(p2.epochOf(alice), 0);
    }

    // ---------------------------------------------------------------- approve + transferFrom

    function test_approveAndTransfer() public {
        uint48 exp = _now() + DAY;
        vm.expectEmit(true, true, true, true, address(p2));
        emit IPermit2Expiring.Approval(alice, address(tokenA), spender, 100, exp);
        vm.prank(alice);
        p2.approve(address(tokenA), spender, 100, exp);
        (uint160 amt, uint48 e, uint48 n) = p2.allowance(alice, address(tokenA), spender);
        assertEq(amt, 100);
        assertEq(e, exp);
        assertEq(n, 0);

        vm.prank(spender);
        p2.transferFrom(alice, bob, 60, address(tokenA));
        assertEq(tokenA.balanceOf(bob), 60);
        (amt,,) = p2.allowance(alice, address(tokenA), spender);
        assertEq(amt, 40);

        vm.prank(spender);
        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.InsufficientAllowance.selector, 40));
        p2.transferFrom(alice, bob, 41, address(tokenA));
    }

    function test_noPermanentAllowances() public {
        vm.startPrank(alice);
        vm.expectRevert(
            abi.encodeWithSelector(IPermit2Expiring.InvalidExpiration.selector, 0, block.timestamp + 365 days)
        );
        p2.approve(address(tokenA), spender, 1, 0);
        vm.expectRevert();
        p2.approve(address(tokenA), spender, 1, _now());
        vm.expectRevert();
        p2.approve(address(tokenA), spender, 1, type(uint48).max);
        vm.expectRevert();
        p2.approve(address(tokenA), spender, 1, _now() + 365 days + 1);
        p2.approve(address(tokenA), spender, 1, _now() + 365 days);
        vm.stopPrank();
    }

    function test_expiredCannotBeSpent() public {
        uint48 exp = _now() + DAY;
        vm.prank(alice);
        p2.approve(address(tokenA), spender, type(uint160).max, exp);
        vm.warp(exp);
        (uint160 amt, uint48 e,) = p2.allowance(alice, address(tokenA), spender);
        assertEq(amt, 0);
        assertEq(e, exp);
        vm.prank(spender);
        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.AllowanceExpired.selector, exp));
        p2.transferFrom(alice, bob, 1, address(tokenA));
    }

    function test_unlimitedAmountNotDecremented() public {
        vm.prank(alice);
        p2.approve(address(tokenA), spender, type(uint160).max, _now() + DAY);
        vm.prank(spender);
        p2.transferFrom(alice, bob, 5 ether, address(tokenA));
        (uint160 amt,,) = p2.allowance(alice, address(tokenA), spender);
        assertEq(amt, type(uint160).max);
    }

    function test_batchTransfer() public {
        vm.startPrank(alice);
        p2.approve(address(tokenA), spender, 10, _now() + DAY);
        p2.approve(address(tokenB), spender, 20, _now() + DAY);
        vm.stopPrank();
        IPermit2Expiring.AllowanceTransferDetails[] memory t =
            new IPermit2Expiring.AllowanceTransferDetails[](2);
        t[0] = IPermit2Expiring.AllowanceTransferDetails(alice, bob, 10, address(tokenA));
        t[1] = IPermit2Expiring.AllowanceTransferDetails(alice, bob, 20, address(tokenB));
        vm.prank(spender);
        p2.transferFrom(t);
        assertEq(tokenA.balanceOf(bob), 10);
        assertEq(tokenB.balanceOf(bob), 20);
    }

    function test_neverApprovedCannotSpend() public {
        vm.prank(spender);
        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.InsufficientAllowance.selector, 0));
        p2.transferFrom(alice, bob, 1, address(tokenA));
    }

    function testFuzz_expiredAllowanceNeverSpendable(uint160 amount, uint48 window, uint48 later) public {
        amount = uint160(bound(amount, 1, 1000 ether));
        window = uint48(bound(window, 1, 365 days));
        later = uint48(bound(later, window, 3 * 365 days));
        vm.prank(alice);
        p2.approve(address(tokenA), spender, amount, _now() + window);
        vm.warp(block.timestamp + later);
        vm.prank(spender);
        vm.expectRevert();
        p2.transferFrom(alice, bob, 1, address(tokenA));
        assertEq(tokenA.balanceOf(bob), 0);
    }

    // ---------------------------------------------------------------- permits

    function test_permitSingle() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        p2.permit(alice, p, sig);
        (uint160 amt, uint48 e, uint48 n) = p2.allowance(alice, address(tokenA), spender);
        assertEq(amt, 50);
        assertEq(e, _now() + DAY);
        assertEq(n, 1);

        // replay impossible: nonce already used
        vm.expectRevert(IPermit2Expiring.InvalidNonce.selector);
        p2.permit(alice, p, sig);
    }

    function test_permitWrongChainIdFails() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        vm.chainId(block.chainid + 1);
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(alice, p, sig);
    }

    function test_permitWrongContractFails() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        address other = makeAddr("otherPermit2");
        vm.etch(other, address(p2).code);
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        Permit2Expiring(other).permit(alice, p, sig);
    }

    function test_permitTamperedFails() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        p.details.amount = 51;
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(alice, p, sig);
        p.details.amount = 50;
        p.spender = bob;
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(alice, p, sig);
        p.spender = spender;
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(bob, p, sig); // claimed owner != signer
    }

    function test_permitDeadline() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        vm.warp(p.sigDeadline + 1);
        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.SignatureExpired.selector, p.sigDeadline));
        p2.permit(alice, p, sig);
    }

    function test_permitMustHaveExpiry() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + 400 days, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        vm.expectRevert();
        p2.permit(alice, p, sig);
        // Signed long ago with an expiration that is now in the past.
        p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        p.sigDeadline = block.timestamp + 10 days;
        sig = _signSingle(aliceKey, p);
        vm.warp(block.timestamp + DAY);
        vm.expectRevert();
        p2.permit(alice, p, sig);
    }

    function test_permitBatch() public {
        IPermit2Expiring.PermitDetails[] memory d = new IPermit2Expiring.PermitDetails[](2);
        d[0] = IPermit2Expiring.PermitDetails(address(tokenA), 1, _now() + DAY, 0);
        d[1] = IPermit2Expiring.PermitDetails(address(tokenB), 2, _now() + 2 * DAY, 0);
        IPermit2Expiring.PermitBatch memory p = IPermit2Expiring.PermitBatch({
            details: d, spender: spender, sigDeadline: block.timestamp + 1, epoch: 0
        });
        bytes memory sig = _signBatch(aliceKey, p);
        p2.permit(alice, p, sig);
        (uint160 a,,) = p2.allowance(alice, address(tokenA), spender);
        (uint160 b,,) = p2.allowance(alice, address(tokenB), spender);
        assertEq(a, 1);
        assertEq(b, 2);
        vm.expectRevert(IPermit2Expiring.InvalidNonce.selector);
        p2.permit(alice, p, sig);
    }

    function test_permitSmartAccount() public {
        MockSmartAccount acct = new MockSmartAccount(alice);
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        p2.permit(address(acct), p, sig);
        (uint160 amt,,) = p2.allowance(address(acct), address(tokenA), spender);
        assertEq(amt, 50);
        MockSmartAccount bobAcct = new MockSmartAccount(bob);
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(address(bobAcct), p, sig);
    }

    /// A thief with the EOA key of a 7702 smart account whose lock is on cannot sign Permit2 permits: the owner has
    /// code (0xef0100 designator), so only its ERC-1271 is consulted, and it refuses. Single and batch.
    function test_permitEoaKeyCannotBypassLocked7702Account() public {
        uint256 ownerKey = 0x7702;
        address owner = vm.addr(ownerKey);
        address impl = address(new LockedAccountMock());
        vm.signAndAttachDelegation(impl, ownerKey);
        assertEq(owner.code, abi.encodePacked(hex"ef0100", impl));
        tokenA.mint(owner, 100 ether);
        vm.prank(owner); // the standing ERC-20 approval to Permit2 that users give once
        tokenA.approve(address(p2), type(uint256).max);

        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 100 ether, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(ownerKey, p);
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(owner, p, sig);

        IPermit2Expiring.PermitDetails[] memory d = new IPermit2Expiring.PermitDetails[](1);
        d[0] = IPermit2Expiring.PermitDetails(address(tokenA), 100 ether, _now() + DAY, 0);
        IPermit2Expiring.PermitBatch memory b = IPermit2Expiring.PermitBatch({
            details: d, spender: spender, sigDeadline: block.timestamp + 1, epoch: 0
        });
        sig = _signBatch(ownerKey, b);
        vm.expectRevert(IPermit2Expiring.InvalidSignature.selector);
        p2.permit(owner, b, sig);

        (uint160 amt,,) = p2.allowance(owner, address(tokenA), spender);
        assertEq(amt, 0);
        vm.prank(spender);
        vm.expectRevert();
        p2.transferFrom(owner, spender, 1, address(tokenA));
        assertEq(tokenA.balanceOf(owner), 100 ether);
    }

    /// The same 7702 account with an accepting ERC-1271 (lock off) still works, through 1271.
    function test_permit7702AccountAcceptedVia1271() public {
        uint256 ownerKey = 0x7702;
        address owner = vm.addr(ownerKey);
        vm.signAndAttachDelegation(address(new MockSmartAccount(owner)), ownerKey);
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 5, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(ownerKey, p);
        vm.expectCall(owner, abi.encodeWithSelector(MockSmartAccount.isValidSignature.selector));
        p2.permit(owner, p, sig);
        (uint160 amt,,) = p2.allowance(owner, address(tokenA), spender);
        assertEq(amt, 5);
    }

    function test_invalidateNonces() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        vm.startPrank(alice);
        p2.invalidateNonces(address(tokenA), spender, 1);
        vm.expectRevert(IPermit2Expiring.InvalidNonce.selector);
        p2.invalidateNonces(address(tokenA), spender, 1);
        vm.expectRevert(IPermit2Expiring.ExcessiveInvalidation.selector);
        p2.invalidateNonces(address(tokenA), spender, 1 + 65536);
        vm.stopPrank();
        vm.expectRevert(IPermit2Expiring.InvalidNonce.selector);
        p2.permit(alice, p, sig);
    }

    // ---------------------------------------------------------------- revocation

    function test_lockdown() public {
        vm.startPrank(alice);
        p2.approve(address(tokenA), spender, 10, _now() + DAY);
        p2.approve(address(tokenB), bob, 10, _now() + DAY);
        IPermit2Expiring.TokenSpenderPair[] memory pairs = new IPermit2Expiring.TokenSpenderPair[](2);
        pairs[0] = IPermit2Expiring.TokenSpenderPair(address(tokenA), spender);
        pairs[1] = IPermit2Expiring.TokenSpenderPair(address(tokenB), bob);
        p2.lockdown(pairs);
        vm.stopPrank();
        (uint160 a,,) = p2.allowance(alice, address(tokenA), spender);
        (uint160 b,,) = p2.allowance(alice, address(tokenB), bob);
        assertEq(a + b, 0);
        vm.prank(spender);
        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.InsufficientAllowance.selector, 0));
        p2.transferFrom(alice, bob, 1, address(tokenA));
    }

    function test_revokeAllZeroesAllowancesAndPendingSignatures() public {
        // A live allowance and an unsubmitted signed permit.
        vm.prank(alice);
        p2.approve(address(tokenA), spender, 10, _now() + DAY);
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenB), 50, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);

        vm.expectEmit(true, false, false, true, address(p2));
        emit IPermit2Expiring.AllRevoked(alice, 1);
        vm.prank(alice);
        p2.revokeAll();

        (uint160 a, uint48 e,) = p2.allowance(alice, address(tokenA), spender);
        assertEq(a, 0);
        assertEq(e, 0);
        vm.prank(spender);
        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.InsufficientAllowance.selector, 0));
        p2.transferFrom(alice, bob, 1, address(tokenA));

        vm.expectRevert(abi.encodeWithSelector(IPermit2Expiring.StaleEpoch.selector, 0, 1));
        p2.permit(alice, p, sig);

        // Fresh permits for the new epoch work, and nonces were NOT reset.
        p = _single(address(tokenA), 5, _now() + DAY, 0, 1);
        p2.permit(alice, p, _signSingle(aliceKey, p));
        (a,,) = p2.allowance(alice, address(tokenA), spender);
        assertEq(a, 5);
    }

    function test_revokeAllDoesNotResetNonces() public {
        IPermit2Expiring.PermitSingle memory p = _single(address(tokenA), 5, _now() + DAY, 0, 0);
        bytes memory sig = _signSingle(aliceKey, p);
        p2.permit(alice, p, sig);
        vm.prank(alice);
        p2.revokeAll();
        (,, uint48 n) = p2.allowance(alice, address(tokenA), spender);
        assertEq(n, 1);
        // re-signing nonce 0 for the new epoch is rejected
        p = _single(address(tokenA), 5, _now() + DAY, 0, 1);
        sig = _signSingle(aliceKey, p);
        vm.expectRevert(IPermit2Expiring.InvalidNonce.selector);
        p2.permit(alice, p, sig);
    }

    function testFuzz_revokeAllZeroesEverything(uint8 nTokens, uint8 nSpenders, uint160 amount) public {
        nTokens = uint8(bound(nTokens, 1, 4));
        nSpenders = uint8(bound(nSpenders, 1, 4));
        amount = uint160(bound(amount, 1, type(uint160).max));
        address[] memory toks = new address[](nTokens);
        for (uint256 i; i < nTokens; ++i) {
            MockERC20 t = new MockERC20();
            t.mint(alice, 100);
            vm.prank(alice);
            t.approve(address(p2), type(uint256).max);
            toks[i] = address(t);
        }
        vm.startPrank(alice);
        for (uint256 i; i < nTokens; ++i) {
            for (uint256 j; j < nSpenders; ++j) {
                p2.approve(toks[i], address(uint160(0x5000 + j)), amount, _now() + DAY);
            }
        }
        p2.revokeAll();
        vm.stopPrank();
        for (uint256 i; i < nTokens; ++i) {
            for (uint256 j; j < nSpenders; ++j) {
                address s = address(uint160(0x5000 + j));
                (uint160 a, uint48 e,) = p2.allowance(alice, toks[i], s);
                assertEq(a, 0);
                assertEq(e, 0);
                vm.prank(s);
                vm.expectRevert();
                p2.transferFrom(alice, s, 1, toks[i]);
            }
        }
    }
}

// -------------------------------------------------------------------- invariant

contract Permit2Handler is Test {
    Permit2Expiring public p2;
    MockERC20 public token;
    address[] public owners;
    address[] public spenders;
    bool public spentExpired;
    bool public spentAfterRevokeAll;
    uint256 public spends;

    mapping(address owner => mapping(address spender => uint48)) public ghostExpiration;
    mapping(address owner => mapping(address spender => uint48)) public ghostEpoch;

    constructor(Permit2Expiring p, MockERC20 t, address[] memory o, address[] memory s) {
        p2 = p;
        token = t;
        owners = o;
        spenders = s;
    }

    function approve(uint256 oi, uint256 si, uint160 amount, uint48 window) external {
        address o = owners[oi % owners.length];
        address s = spenders[si % spenders.length];
        amount = uint160(bound(amount, 1, 1000 ether));
        window = uint48(bound(window, 1, 365 days));
        uint48 exp = uint48(block.timestamp) + window;
        vm.prank(o);
        p2.approve(address(token), s, amount, exp);
        ghostExpiration[o][s] = exp;
        ghostEpoch[o][s] = p2.epochOf(o);
    }

    /// @dev Only every 8th seed actually revokes, so allowances survive long enough to be spent.
    function revokeAll(uint256 oi) external {
        if (oi % 8 != 0) return;
        vm.prank(owners[(oi / 8) % owners.length]);
        p2.revokeAll();
    }

    function warp(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 20 days));
    }

    function spend(uint256 oi, uint256 si, uint160 amount) external {
        address o = owners[oi % owners.length];
        address s = spenders[si % spenders.length];
        amount = uint160(bound(amount, 1, 10 ether));
        bool expired = block.timestamp >= ghostExpiration[o][s];
        bool revoked = ghostEpoch[o][s] != p2.epochOf(o);
        vm.prank(s);
        try p2.transferFrom(o, s, amount, address(token)) {
            spends++;
            if (expired) spentExpired = true;
            if (revoked) spentAfterRevokeAll = true;
        } catch {}
    }
}

contract Permit2ExpiringInvariantTest is Test {
    Permit2Expiring internal p2;
    Permit2Handler internal handler;
    MockERC20 internal token;

    function setUp() public {
        vm.warp(1_800_000_000);
        vm.etch(Predeploys.PERMIT2_EXPIRING, vm.getDeployedCode("Permit2Expiring.sol:Permit2Expiring"));
        p2 = Permit2Expiring(Predeploys.PERMIT2_EXPIRING);
        token = new MockERC20();
        address[] memory o = new address[](2);
        address[] memory s = new address[](2);
        o[0] = makeAddr("o0");
        o[1] = makeAddr("o1");
        s[0] = makeAddr("s0");
        s[1] = makeAddr("s1");
        for (uint256 i; i < 2; ++i) {
            token.mint(o[i], 1_000_000 ether);
            vm.prank(o[i]);
            token.approve(address(p2), type(uint256).max);
        }
        handler = new Permit2Handler(p2, token, o, s);
        targetContract(address(handler));
    }

    function invariant_expiredAllowanceNeverSpent() public view {
        assertFalse(handler.spentExpired());
    }

    function invariant_revokedAllowanceNeverSpent() public view {
        assertFalse(handler.spentAfterRevokeAll());
    }

    function invariant_viewNeverReportsExpiredAmount() public view {
        for (uint256 i; i < 2; ++i) {
            for (uint256 j; j < 2; ++j) {
                (uint160 a, uint48 e,) = p2.allowance(handler.owners(i), address(token), handler.spenders(j));
                if (block.timestamp >= e) assertEq(a, 0);
                assertLe(e, block.timestamp + 365 days);
            }
        }
    }
}
