// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";
import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";
import {IERC165} from "@openzeppelin/contracts/utils/introspection/IERC165.sol";
import {XDRC20} from "../../src/periphery/token/XDRC20.sol";
import {IXDRC20} from "../../src/periphery/token/IXDRC20.sol";

/// Minimal ERC-1271 smart account owned by one EOA.
contract MockSmartAccount is IERC1271 {
    address public immutable signer;

    constructor(address s) {
        signer = s;
    }

    function isValidSignature(bytes32 hash, bytes calldata sig) external view returns (bytes4) {
        (uint8 v, bytes32 r, bytes32 s) = (uint8(sig[64]), bytes32(sig[0:32]), bytes32(sig[32:64]));
        return ecrecover(hash, v, r, s) == signer ? IERC1271.isValidSignature.selector : bytes4(0xffffffff);
    }
}

/// Stand-in for an EIP-7702-delegated XdevsSmartAccount whose EOA lock / Guardian Vault is on: its ERC-1271
/// refuses every off-chain signature (the real account only accepts hashes pre-approved on-chain).
contract LockedAccountMock is IERC1271 {
    function isValidSignature(bytes32, bytes calldata) external pure returns (bytes4) {
        return 0xffffffff;
    }
}

contract XDRC20Test is Test {
    XDRC20 internal token;
    uint256 internal aliceKey = 0xA11CE;
    address internal alice;
    address internal spender = makeAddr("spender");
    address internal bob = makeAddr("bob");

    uint64 internal constant DEFAULT = 30 days;
    uint64 internal constant MAX = 365 days;

    bytes32 internal constant PERMIT_TYPEHASH =
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");
    bytes32 internal constant PERMIT_WITH_EXPIRY_TYPEHASH = keccak256(
        "PermitWithExpiry(address owner,address spender,uint256 value,uint64 expiry,uint256 nonce,uint256 deadline)"
    );

    function setUp() public {
        vm.warp(1_800_000_000);
        alice = vm.addr(aliceKey);
        token = new XDRC20("xDEVS Test Token", "XTT", alice, 1_000_000 ether, DEFAULT, MAX);
    }

    function _now() internal view returns (uint64) {
        return uint64(block.timestamp);
    }

    // ---------------------------------------------------------------- basics

    function test_metadata() public view {
        assertEq(token.name(), "xDEVS Test Token");
        assertEq(token.symbol(), "XTT");
        assertEq(token.decimals(), 18);
        assertEq(token.totalSupply(), 1_000_000 ether);
        assertEq(token.balanceOf(alice), 1_000_000 ether);
        assertEq(token.defaultApprovalDuration(), DEFAULT);
        assertEq(token.maxApprovalDuration(), MAX);
        assertTrue(token.supportsInterface(type(IXDRC20).interfaceId));
        assertTrue(token.supportsInterface(type(IERC165).interfaceId));
        assertFalse(token.supportsInterface(0xffffffff));
    }

    function test_constructorValidatesDurations() public {
        vm.expectRevert(abi.encodeWithSelector(XDRC20.InvalidDurations.selector, 0, 10));
        new XDRC20("a", "b", alice, 0, 0, 10);
        vm.expectRevert(abi.encodeWithSelector(XDRC20.InvalidDurations.selector, 11, 10));
        new XDRC20("a", "b", alice, 0, 11, 10);
    }

    function test_transfer() public {
        vm.prank(alice);
        assertTrue(token.transfer(bob, 5 ether));
        assertEq(token.balanceOf(bob), 5 ether);
    }

    // ---------------------------------------------------------------- legacy approve

    function test_legacyApproveUsesDefaultExpiry() public {
        vm.expectEmit(true, true, false, true, address(token));
        emit IXDRC20.ApprovalWithExpiry(alice, spender, 100, _now() + DEFAULT);
        vm.prank(alice);
        assertTrue(token.approve(spender, 100));
        assertEq(token.allowance(alice, spender), 100);
        assertEq(token.allowanceExpiry(alice, spender), _now() + DEFAULT);

        vm.prank(spender);
        token.transferFrom(alice, bob, 40);
        assertEq(token.allowance(alice, spender), 60);

        vm.warp(block.timestamp + DEFAULT);
        assertEq(token.allowance(alice, spender), 0);
        vm.prank(spender);
        vm.expectRevert(
            abi.encodeWithSelector(IXDRC20.AllowanceExpired.selector, alice, spender, uint64(block.timestamp))
        );
        token.transferFrom(alice, bob, 1);
    }

    function test_infiniteApprovalStillExpires() public {
        vm.prank(alice);
        token.approve(spender, type(uint256).max);
        vm.prank(spender);
        token.transferFrom(alice, bob, 10);
        assertEq(token.allowance(alice, spender), type(uint256).max);
        vm.warp(block.timestamp + DEFAULT);
        vm.prank(spender);
        vm.expectRevert();
        token.transferFrom(alice, bob, 10);
    }

    function test_approveZeroRevokes() public {
        vm.startPrank(alice);
        token.approve(spender, 100, _now() + 1 days);
        token.approve(spender, 0);
        vm.stopPrank();
        assertEq(token.allowance(alice, spender), 0);
        assertEq(token.allowanceExpiry(alice, spender), 0);
        vm.prank(spender);
        vm.expectRevert(
            abi.encodeWithSelector(IERC20Errors.ERC20InsufficientAllowance.selector, spender, 0, 1)
        );
        token.transferFrom(alice, bob, 1);
    }

    // ---------------------------------------------------------------- 3-arg approve

    function test_approveWithExpiry() public {
        uint64 exp = _now() + 7 days;
        vm.prank(alice);
        token.approve(spender, 500, exp);
        assertEq(token.allowance(alice, spender), 500);
        assertEq(token.allowanceExpiry(alice, spender), exp);

        vm.warp(exp - 1);
        vm.prank(spender);
        token.transferFrom(alice, bob, 500);
        assertEq(token.allowance(alice, spender), 0);
    }

    function test_approveExpiryBounds() public {
        vm.startPrank(alice);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidApprovalExpiry.selector, _now(), _now() + MAX));
        token.approve(spender, 1, _now());
        vm.expectRevert(
            abi.encodeWithSelector(IXDRC20.InvalidApprovalExpiry.selector, _now() + MAX + 1, _now() + MAX)
        );
        token.approve(spender, 1, _now() + MAX + 1);
        token.approve(spender, 1, _now() + MAX); // boundary ok
        token.approve(spender, 0, 0); // revoke ignores expiry
        vm.stopPrank();
    }

    function testFuzz_expiredAllowanceNeverSpendable(
        uint256 amount,
        uint64 window,
        uint64 later,
        uint256 spend
    ) public {
        amount = bound(amount, 1, 1_000_000 ether);
        window = uint64(bound(window, 1, MAX));
        later = uint64(bound(later, window, 3 * MAX));
        spend = bound(spend, 1, amount);
        vm.prank(alice);
        token.approve(spender, amount, _now() + window);
        vm.warp(block.timestamp + later);
        assertEq(token.allowance(alice, spender), 0);
        uint256 balBefore = token.balanceOf(alice);
        vm.prank(spender);
        vm.expectRevert();
        token.transferFrom(alice, bob, spend);
        assertEq(token.balanceOf(alice), balBefore);
    }

    function testFuzz_liveAllowanceSpendable(uint256 amount, uint64 window, uint64 later) public {
        amount = bound(amount, 1, 1_000_000 ether);
        window = uint64(bound(window, 2, MAX));
        later = uint64(bound(later, 0, window - 1));
        vm.prank(alice);
        token.approve(spender, amount, _now() + window);
        vm.warp(block.timestamp + later);
        vm.prank(spender);
        token.transferFrom(alice, bob, amount);
        assertEq(token.balanceOf(bob), amount);
    }

    // ---------------------------------------------------------------- revokeAll

    function test_revokeAll() public {
        address[] memory sp = new address[](3);
        sp[0] = spender;
        sp[1] = bob;
        sp[2] = makeAddr("dex");
        vm.startPrank(alice);
        for (uint256 i; i < sp.length; ++i) {
            token.approve(sp[i], 1000, _now() + 10 days);
        }
        vm.expectEmit(true, false, false, true, address(token));
        emit IXDRC20.AllApprovalsRevoked(alice, 1);
        token.revokeAll();
        vm.stopPrank();
        assertEq(token.approvalEpoch(alice), 1);
        for (uint256 i; i < sp.length; ++i) {
            assertEq(token.allowance(alice, sp[i]), 0);
            assertEq(token.allowanceExpiry(alice, sp[i]), 0);
            vm.prank(sp[i]);
            vm.expectRevert(
                abi.encodeWithSelector(IERC20Errors.ERC20InsufficientAllowance.selector, sp[i], 0, 1)
            );
            token.transferFrom(alice, sp[i], 1);
        }
        // New approvals after revokeAll work normally.
        vm.prank(alice);
        token.approve(spender, 5, _now() + 1 days);
        assertEq(token.allowance(alice, spender), 5);
    }

    // ---------------------------------------------------------------- permits

    function _sign(uint256 key, bytes32 structHash) internal view returns (uint8 v, bytes32 r, bytes32 s) {
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", token.DOMAIN_SEPARATOR(), structHash));
        return vm.sign(key, digest);
    }

    function test_eip2612Permit() public {
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) =
            _sign(aliceKey, keccak256(abi.encode(PERMIT_TYPEHASH, alice, spender, 77, 0, deadline)));
        token.permit(alice, spender, 77, deadline, v, r, s);
        assertEq(token.allowance(alice, spender), 77);
        assertEq(token.allowanceExpiry(alice, spender), _now() + DEFAULT);
        assertEq(token.nonces(alice), 1);

        // replay fails (nonce consumed)
        vm.expectRevert();
        token.permit(alice, spender, 77, deadline, v, r, s);
    }

    function test_permitDeadline() public {
        uint256 deadline = block.timestamp - 1;
        (uint8 v, bytes32 r, bytes32 s) =
            _sign(aliceKey, keccak256(abi.encode(PERMIT_TYPEHASH, alice, spender, 77, 0, deadline)));
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.PermitDeadlineExpired.selector, deadline));
        token.permit(alice, spender, 77, deadline, v, r, s);
    }

    function _permitWithExpirySig(uint256 key, address owner, uint256 value, uint64 expiry, uint256 nonce)
        internal
        view
        returns (bytes memory)
    {
        (uint8 v, bytes32 r, bytes32 s) = _sign(
            key,
            keccak256(
                abi.encode(
                    PERMIT_WITH_EXPIRY_TYPEHASH,
                    owner,
                    spender,
                    value,
                    expiry,
                    nonce,
                    block.timestamp + 1 hours
                )
            )
        );
        return abi.encodePacked(r, s, v);
    }

    function test_permitWithExpiry() public {
        uint64 exp = _now() + 3 days;
        bytes memory sig = _permitWithExpirySig(aliceKey, alice, 9, exp, 0);
        token.permitWithExpiry(alice, spender, 9, exp, block.timestamp + 1 hours, sig);
        assertEq(token.allowance(alice, spender), 9);
        assertEq(token.allowanceExpiry(alice, spender), exp);

        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, alice));
        token.permitWithExpiry(alice, spender, 9, exp, block.timestamp + 1 hours, sig);
    }

    function test_permitWithExpiryRejectsTamperingAndOtherChains() public {
        uint64 exp = _now() + 3 days;
        bytes memory sig = _permitWithExpirySig(aliceKey, alice, 9, exp, 0);
        // tampered value / expiry
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, alice));
        token.permitWithExpiry(alice, spender, 10, exp, block.timestamp + 1 hours, sig);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, alice));
        token.permitWithExpiry(alice, spender, 9, exp + 1, block.timestamp + 1 hours, sig);
        // same signature on another chain id
        uint256 snap = vm.snapshotState();
        vm.chainId(block.chainid + 1);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, alice));
        token.permitWithExpiry(alice, spender, 9, exp, block.timestamp + 1 hours, sig);
        vm.revertToState(snap);
        token.permitWithExpiry(alice, spender, 9, exp, block.timestamp + 1 hours, sig);
    }

    function test_permitWithExpirySmartAccount() public {
        MockSmartAccount acct = new MockSmartAccount(alice);
        vm.prank(alice);
        token.transfer(address(acct), 100);
        uint64 exp = _now() + 1 days;
        bytes memory sig = _permitWithExpirySig(aliceKey, address(acct), 50, exp, 0);
        token.permitWithExpiry(address(acct), spender, 50, exp, block.timestamp + 1 hours, sig);
        assertEq(token.allowance(address(acct), spender), 50);
        vm.prank(spender);
        token.transferFrom(address(acct), bob, 50);
        assertEq(token.balanceOf(bob), 50);
    }

    // ---------------------------------------------------------------- permits: code owners (ERC-1271 / 7702)

    function _permitSig(uint256 key, address owner, uint256 value, uint256 nonce, uint256 deadline)
        internal
        view
        returns (uint8 v, bytes32 r, bytes32 s)
    {
        return _sign(key, keccak256(abi.encode(PERMIT_TYPEHASH, owner, spender, value, nonce, deadline)));
    }

    /// @dev EOA with a known key, delegated (EIP-7702) to `impl`, holding 100 tokens.
    function _delegatedOwner(uint256 key, address impl) internal returns (address owner) {
        owner = vm.addr(key);
        vm.signAndAttachDelegation(impl, key);
        assertEq(owner.code, abi.encodePacked(hex"ef0100", impl));
        vm.prank(alice);
        token.transfer(owner, 100);
    }

    function test_eip2612PermitBadSignatureReverts() public {
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) = _permitSig(0xB0B, alice, 77, 0, deadline); // wrong key
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, alice));
        token.permit(alice, spender, 77, deadline, v, r, s);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, alice));
        token.permit(alice, spender, 77, deadline, 0, r, s); // malformed v
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, address(0)));
        token.permit(address(0), spender, 77, deadline, v, r, s);
    }

    /// A contract wallet (no private key) can now use the standard permit through ERC-1271.
    function test_eip2612PermitContractWalletUses1271() public {
        MockSmartAccount acct = new MockSmartAccount(alice);
        vm.prank(alice);
        token.transfer(address(acct), 100);
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) = _permitSig(aliceKey, address(acct), 50, 0, deadline);
        vm.expectCall(address(acct), abi.encodeWithSelector(IERC1271.isValidSignature.selector));
        token.permit(address(acct), spender, 50, deadline, v, r, s);
        assertEq(token.allowance(address(acct), spender), 50);
        assertEq(token.nonces(address(acct)), 1);
        vm.prank(spender);
        token.transferFrom(address(acct), bob, 50);
        assertEq(token.balanceOf(bob), 50);

        // A signature the wallet does not accept is rejected.
        (v, r, s) = _permitSig(0xB0B, address(acct), 50, 1, deadline);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, address(acct)));
        token.permit(address(acct), spender, 50, deadline, v, r, s);
    }

    /// The bypass this fixes: a thief holding the EOA key of a 7702 smart account with its lock on signs a
    /// permit. The ECDSA signature IS valid for the owner address, but the owner has code, so only the
    /// account's ERC-1271 decides - and it refuses. Nothing can be drained.
    function test_eip2612PermitEoaKeyCannotBypassLocked7702Account() public {
        uint256 ownerKey = 0x7702;
        address owner = _delegatedOwner(ownerKey, address(new LockedAccountMock()));
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) = _permitSig(ownerKey, owner, 100, 0, deadline);
        bytes32 digest = keccak256(
            abi.encodePacked(
                "\x19\x01",
                token.DOMAIN_SEPARATOR(),
                keccak256(abi.encode(PERMIT_TYPEHASH, owner, spender, 100, 0, deadline))
            )
        );
        assertEq(ecrecover(digest, v, r, s), owner, "plain ECDSA would have accepted");

        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, owner));
        token.permit(owner, spender, 100, deadline, v, r, s);
        assertEq(token.allowance(owner, spender), 0);
        vm.prank(spender);
        vm.expectRevert();
        token.transferFrom(owner, spender, 100);
        assertEq(token.balanceOf(owner), 100);

        // Same for the bytes-signature variant.
        uint64 exp = _now() + 1 days;
        bytes memory sig = _permitWithExpirySig(ownerKey, owner, 100, exp, 0);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidPermitSignature.selector, owner));
        token.permitWithExpiry(owner, spender, 100, exp, block.timestamp + 1 hours, sig);
        assertEq(token.allowance(owner, spender), 0);
    }

    /// A 7702 account whose ERC-1271 accepts the EOA key (lock off) still gets permits - through 1271.
    function test_eip2612Permit7702AccountAcceptedVia1271() public {
        uint256 ownerKey = 0x7702;
        address owner = vm.addr(ownerKey);
        address owner_ = _delegatedOwner(ownerKey, address(new MockSmartAccount(owner)));
        assertEq(owner_, owner);
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) = _permitSig(ownerKey, owner, 40, 0, deadline);
        vm.expectCall(owner, abi.encodeWithSelector(IERC1271.isValidSignature.selector));
        token.permit(owner, spender, 40, deadline, v, r, s);
        assertEq(token.allowance(owner, spender), 40);
    }

    function test_permitWithExpiryValidatesExpiry() public {
        uint64 exp = _now() + MAX + 1;
        bytes memory sig = _permitWithExpirySig(aliceKey, alice, 9, exp, 0);
        vm.expectRevert(abi.encodeWithSelector(IXDRC20.InvalidApprovalExpiry.selector, exp, _now() + MAX));
        token.permitWithExpiry(alice, spender, 9, exp, block.timestamp + 1 hours, sig);
    }
}

// -------------------------------------------------------------------- invariant

contract XDRC20Handler is Test {
    XDRC20 public token;
    address[] public owners;
    address[] public spenders;
    bool public spentExpired;
    uint256 public spends;

    constructor(XDRC20 t, address[] memory o, address[] memory s) {
        token = t;
        owners = o;
        spenders = s;
    }

    function approve(uint256 oi, uint256 si, uint256 amount, uint64 window) external {
        address o = owners[oi % owners.length];
        amount = bound(amount, 0, 1000 ether);
        window = uint64(bound(window, 1, 365 days));
        vm.prank(o);
        token.approve(spenders[si % spenders.length], amount, uint64(block.timestamp) + window);
    }

    function legacyApprove(uint256 oi, uint256 si, uint256 amount) external {
        vm.prank(owners[oi % owners.length]);
        token.approve(spenders[si % spenders.length], bound(amount, 0, 1000 ether));
    }

    /// @dev Only every 8th seed actually revokes, so allowances survive long enough to be spent.
    function revokeAll(uint256 oi) external {
        if (oi % 8 != 0) return;
        vm.prank(owners[(oi / 8) % owners.length]);
        token.revokeAll();
    }

    function warp(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 20 days));
    }

    function spend(uint256 oi, uint256 si, uint256 amount) external {
        address o = owners[oi % owners.length];
        address s = spenders[si % spenders.length];
        amount = bound(amount, 1, 10 ether);
        uint64 exp = token.allowanceExpiry(o, s);
        bool expired = block.timestamp >= exp;
        vm.prank(s);
        try token.transferFrom(o, s, amount) {
            spends++;
            if (expired) spentExpired = true;
        } catch {}
    }
}

contract XDRC20InvariantTest is Test {
    XDRC20 internal token;
    XDRC20Handler internal handler;

    function setUp() public {
        vm.warp(1_800_000_000);
        address[] memory o = new address[](2);
        o[0] = makeAddr("o0");
        o[1] = makeAddr("o1");
        address[] memory s = new address[](2);
        s[0] = makeAddr("s0");
        s[1] = makeAddr("s1");
        token = new XDRC20("T", "T", address(this), 2_000_000 ether, 30 days, 365 days);
        token.transfer(o[0], 1_000_000 ether);
        token.transfer(o[1], 1_000_000 ether);
        handler = new XDRC20Handler(token, o, s);
        targetContract(address(handler));
    }

    /// An expired (or revoked) allowance can never be spent.
    function invariant_expiredAllowanceNeverSpent() public view {
        assertFalse(handler.spentExpired());
    }

    /// allowance() is 0 whenever the expiry has passed.
    function invariant_allowanceZeroAfterExpiry() public view {
        for (uint256 i; i < 2; ++i) {
            for (uint256 j; j < 2; ++j) {
                address o = handler.owners(i);
                address s = handler.spenders(j);
                if (block.timestamp >= token.allowanceExpiry(o, s)) assertEq(token.allowance(o, s), 0);
            }
        }
    }

    function invariant_supplyConserved() public view {
        assertEq(token.totalSupply(), 2_000_000 ether);
    }
}

