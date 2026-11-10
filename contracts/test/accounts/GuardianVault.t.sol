// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {Counter} from "./utils/Mocks.sol";
import {IGuardianVault} from "../../src/accounts/interfaces/IGuardianVault.sol";
import {IAccountEvents} from "../../src/accounts/interfaces/IXdevsSmartAccount.sol";
import {AccountConstants} from "../../src/accounts/libraries/AccountConstants.sol";
import {Call, InitParams, TokenLimit, VaultPolicy} from "../../src/accounts/libraries/AccountTypes.sol";

contract GuardianVaultTest is AccountTestBase {
    IXAccount internal acct;
    address internal friend = makeAddr("friend");
    address internal exchange = makeAddr("exchange"); // whitelisted recipient
    address internal router = makeAddr("router"); // whitelisted spender
    Counter internal untrusted;

    function setUp() public override {
        super.setUp();
        InitParams memory p = _params(_one(_ecdsaSigner(owner)), true);
        p.whitelist = new address[](3);
        p.whitelist[0] = address(counter);
        p.whitelist[1] = exchange;
        p.whitelist[2] = router;
        acct = _createAccount(p, 0);
        untrusted = new Counter();
        usd.mint(address(acct), 10_000e6);
    }

    function _exec(address target, uint256 value, bytes memory data) internal returns (uint256 id) {
        vm.prank(owner);
        id = acct.execute(target, value, data);
    }

    // ------------------------------------------------------------------------------------------------------------
    // Native value
    // ------------------------------------------------------------------------------------------------------------

    function test_belowThreshold_isInstant() public {
        uint256 id = _exec(friend, 0.5 ether, "");
        assertEq(id, 0);
        assertEq(friend.balance, 0.5 ether);
        assertEq(acct.spentToday(), 0.5 ether);
    }

    function test_aboveThreshold_isQueued_thenExecutableByAnyoneAfterDelay() public {
        uint64 expectedAfter = uint64(block.timestamp + 24 hours);
        vm.expectEmit(address(acct));
        emit IGuardianVault.VaultActionQueued(1, friend, 2 ether, "", expectedAfter);
        uint256 id = _exec(friend, 2 ether, "");
        assertEq(id, 1);
        assertEq(friend.balance, 0);

        vm.warp(expectedAfter - 1);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotReady.selector, id, expectedAfter));
        acct.execute(id);

        vm.warp(expectedAfter);
        vm.prank(makeAddr("anyone"));
        acct.execute(id);
        assertEq(friend.balance, 2 ether);
        (, uint8 status,,) = acct.getAction(id);
        assertEq(status, 3); // Executed

        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, id));
        acct.execute(id);
    }

    function test_dailyLimit() public {
        for (uint256 i; i < 5; ++i) {
            assertEq(_exec(friend, 1 ether, ""), 0);
        }
        assertEq(acct.spentToday(), 5 ether);
        assertGt(_exec(friend, 1 wei, ""), 0, "over the daily limit -> queued");
        vm.warp(block.timestamp + 1 days);
        assertEq(_exec(friend, 1 ether, ""), 0, "new day -> instant again");
    }

    function test_batch_isQueuedAtomically_whenTotalAboveThreshold() public {
        Call[] memory calls = new Call[](3);
        calls[0] = _call(friend, 0.6 ether, "");
        calls[1] = _call(friend, 0.6 ether, "");
        calls[2] = _call(address(counter), 0, abi.encodeCall(counter.increment, ()));
        vm.prank(owner);
        uint256 id = acct.executeBatch(calls);
        assertGt(id, 0);
        assertEq(friend.balance, 0);
        assertEq(counter.count(), 0, "whole batch queued, including the instant call");
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);
        assertEq(friend.balance, 1.2 ether);
        assertEq(counter.count(), 1);
    }

    function test_whitelistedRecipient_isInstantForAnyAmount() public {
        assertEq(_exec(exchange, 50 ether, ""), 0);
        assertEq(exchange.balance, 50 ether);
        assertEq(acct.spentToday(), 0, "whitelisted transfers do not consume the daily limit");
    }

    // ------------------------------------------------------------------------------------------------------------
    // Contract calls / approvals / token transfers
    // ------------------------------------------------------------------------------------------------------------

    function test_whitelistedContract_instant_nonWhitelisted_queued() public {
        assertEq(_exec(address(counter), 0, abi.encodeCall(counter.increment, ())), 0);
        assertEq(counter.count(), 1);
        uint256 id = _exec(address(untrusted), 0, abi.encodeCall(untrusted.increment, ()));
        assertGt(id, 0);
        assertEq(untrusted.count(), 0);
    }

    function test_approvals_policy() public {
        // limited approval to an unknown spender -> queued
        assertGt(_exec(address(usd), 0, abi.encodeCall(usd.approve, (attacker, 100e6))), 0);
        assertEq(usd.allowance(address(acct), attacker), 0);
        // unlimited approval to an unknown spender -> blocked by the allowance policy
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.UnlimitedApprovalBlocked.selector, attacker));
        acct.execute(address(usd), 0, abi.encodeCall(usd.approve, (attacker, type(uint256).max)));
        // setApprovalForAll(true) is unlimited too
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.UnlimitedApprovalBlocked.selector, attacker));
        acct.execute(address(nft), 0, abi.encodeCall(nft.setApprovalForAll, (attacker, true)));
        // Permit2-style approve(token, spender, amount, expiration)
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.UnlimitedApprovalBlocked.selector, attacker));
        acct.execute(
            makeAddr("permit2"),
            0,
            abi.encodeWithSelector(
                0x87517c45, address(usd), attacker, type(uint160).max, uint48(block.timestamp)
            )
        );
        // whitelisted spender -> instant, even unlimited
        assertEq(_exec(address(usd), 0, abi.encodeCall(usd.approve, (router, type(uint256).max))), 0);
        assertEq(usd.allowance(address(acct), router), type(uint256).max);
        // revocation -> always instant
        assertEq(_exec(address(usd), 0, abi.encodeCall(usd.approve, (router, 0))), 0);
        assertEq(_exec(address(nft), 0, abi.encodeCall(nft.setApprovalForAll, (attacker, false))), 0);
    }

    function test_unlimitedApproval_queuedWhenNotBlocked() public {
        VaultPolicy memory p = _vaultPolicy();
        p.blockUnlimitedApprovals = false;
        InitParams memory ip = _params(_one(_ecdsaSigner(owner)), true);
        ip.policy = p;
        IXAccount a2 = _createAccount(ip, bytes32(uint256(9)));
        vm.prank(owner);
        uint256 id = a2.execute(address(usd), 0, abi.encodeCall(usd.approve, (attacker, type(uint256).max)));
        assertGt(id, 0);
    }

    function test_tokenTransfer_queuedWithoutLimit_instantWithinLimit() public {
        // no per-token limit -> queued
        assertGt(_exec(address(usd), 0, abi.encodeCall(usd.transfer, (friend, 1e6))), 0);
        // to a whitelisted recipient -> instant
        assertEq(_exec(address(usd), 0, abi.encodeCall(usd.transfer, (exchange, 5_000e6))), 0);

        // configure a limit (itself delayed)
        vm.prank(owner);
        acct.setTokenLimit(address(usd), 100e6, 250e6);
        (, uint128 daily,,) = acct.tokenLimit(address(usd));
        assertEq(daily, 0, "limit change is delayed");
        uint256 id = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);

        assertEq(_exec(address(usd), 0, abi.encodeCall(usd.transfer, (friend, 100e6))), 0);
        assertGt(_exec(address(usd), 0, abi.encodeCall(usd.transfer, (friend, 101e6))), 0, "above perTx");
        assertEq(_exec(address(usd), 0, abi.encodeCall(usd.transfer, (friend, 100e6))), 0);
        assertGt(_exec(address(usd), 0, abi.encodeCall(usd.transfer, (friend, 60e6))), 0, "above daily");
        assertEq(usd.balanceOf(friend), 200e6);
    }

    function test_nftTransferOut_isQueued() public {
        nft.mint(address(acct), 42);
        assertGt(
            _exec(
                address(nft),
                0,
                abi.encodeWithSignature(
                    "safeTransferFrom(address,address,uint256)", address(acct), attacker, 42
                )
            ),
            0
        );
        assertEq(nft.ownerOf(42), address(acct));
    }

    function test_malformedKnownSelector_isQueued() public {
        // `transfer` selector with truncated arguments to a whitelisted contract
        assertGt(_exec(address(counter), 0, abi.encodePacked(bytes4(0xa9059cbb), bytes20(friend))), 0);
    }

    // ------------------------------------------------------------------------------------------------------------
    // Cancel / execute rules
    // ------------------------------------------------------------------------------------------------------------

    function test_cancel_byOwnerOrGuardian_only() public {
        uint256 id1 = _exec(friend, 2 ether, "");
        uint256 id2 = _exec(friend, 3 ether, "");

        vm.prank(attacker);
        vm.expectRevert(IAccountEvents.Unauthorized.selector);
        acct.cancel(id1);

        vm.expectEmit(address(acct));
        emit IGuardianVault.VaultActionCancelled(id1, owner);
        vm.prank(owner);
        acct.cancel(id1);

        vm.prank(guardian3);
        acct.cancel(id2);

        vm.warp(block.timestamp + 7 days);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, id1));
        acct.execute(id1);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, id2));
        acct.execute(id2);
        assertEq(friend.balance, 0);

        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, id1));
        acct.cancel(id1);
    }

    function test_execute_expiresAfterWindow() public {
        uint256 id = _exec(friend, 2 ether, "");
        vm.warp(block.timestamp + 24 hours + AccountConstants.VAULT_EXECUTION_WINDOW + 1);
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionExpired.selector, id));
        acct.execute(id);
    }

    function test_failingQueuedAction_canBeRetried() public {
        uint256 id = _exec(address(usd), 0, abi.encodeCall(usd.transfer, (friend, 20_000e6))); // > balance
        vm.warp(block.timestamp + 24 hours);
        vm.expectRevert();
        acct.execute(id);
        usd.mint(address(acct), 10_000e6);
        acct.execute(id);
        assertEq(usd.balanceOf(friend), 20_000e6);
    }

    // ------------------------------------------------------------------------------------------------------------
    // Delayed policy changes (a thief can't switch the vault off)
    // ------------------------------------------------------------------------------------------------------------

    function test_disablingVault_isDelayed() public {
        VaultPolicy memory off = _vaultPolicy();
        off.enabled = false;
        vm.prank(owner);
        acct.setVaultPolicy(off);
        uint256 id = acct.lastActionId();
        assertTrue(acct.vaultPolicy().enabled, "still enabled");
        assertGt(_exec(friend, 2 ether, ""), 0, "still enforced");
        vm.warp(block.timestamp + 24 hours);
        acct.execute(id);
        assertFalse(acct.vaultPolicy().enabled);
        assertEq(_exec(friend, 2 ether, ""), 0);
    }

    function test_setPolicy_interface_isDelayed() public {
        address[] memory wl = new address[](1);
        wl[0] = attacker;
        vm.prank(owner);
        acct.setPolicy(1000 ether, 1 hours, wl);
        assertFalse(acct.isWhitelisted(attacker));
        assertEq(acct.vaultPolicy().nativeThreshold, 1 ether);
        uint256 id = acct.lastActionId();
        vm.warp(block.timestamp + 24 hours); // the *current* delay applies, not the requested one
        acct.execute(id);
        assertTrue(acct.isWhitelisted(attacker));
        assertFalse(acct.isWhitelisted(exchange), "whitelist replaced");
        assertEq(acct.vaultPolicy().nativeThreshold, 1000 ether);
        assertEq(acct.vaultPolicy().delay, 1 hours);
    }

    function test_setPolicy_enablesInstantly_whenVaultDisabled() public {
        IXAccount plain = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(5)));
        vm.prank(owner);
        plain.setPolicy(1 ether, 2 days, new address[](0));
        assertTrue(plain.vaultPolicy().enabled);
        assertEq(plain.vaultPolicy().delay, 2 days);
        vm.prank(owner);
        assertGt(plain.execute(friend, 2 ether, ""), 0);
    }

    function test_policyDelayBounds() public {
        VaultPolicy memory p = _vaultPolicy();
        p.delay = 30 minutes;
        IXAccount plain = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(5)));
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.InvalidParameter.selector);
        plain.setVaultPolicy(p);
        p.delay = 8 days;
        vm.prank(owner);
        vm.expectRevert(IAccountEvents.InvalidParameter.selector);
        plain.setVaultPolicy(p);
    }

    function test_whitelistAndGuardianChanges_areDelayed() public {
        vm.startPrank(owner);
        acct.setWhitelisted(attacker, true);
        acct.setGuardians(new address[](0), 0);
        acct.setRecoveryDelay(1 hours);
        vm.stopPrank();
        assertFalse(acct.isWhitelisted(attacker));
        assertTrue(acct.isGuardian(guardian1));
        assertEq(acct.recoveryDelay(), 48 hours);
        assertEq(acct.lastActionId(), 3);
    }

    function test_selfCall_instantList() public {
        // addSigner through execute(self) is instant
        bytes memory data = abi.encodeCall(acct.addSigner, (_passkeySigner()));
        assertEq(_exec(address(acct), 0, data), 0);
        assertEq(acct.getSigners().length, 2);
        // a delayed function through execute(self) is queued
        assertGt(_exec(address(acct), 0, abi.encodeCall(acct.setWhitelisted, (attacker, true))), 0);
        // delayed functions can't be reached without the dispatcher, even by the account address itself
        vm.prank(address(acct));
        acct.setWhitelisted(attacker, true); // routed -> queued, not applied
        assertFalse(acct.isWhitelisted(attacker));
    }

    // ------------------------------------------------------------------------------------------------------------
    // ERC-4337 gas budget
    // ------------------------------------------------------------------------------------------------------------

    function test_gasBudget_bounds_bundlerFeeDrain() public {
        // A thief running their own bundler signs an op with an absurd gas price to convert the account's
        // balance into "fees". The prefund exceeds the 1 ether daily gas budget -> rejected.
        PackedUserOperation memory op = _userOp(address(acct), _execData(friend, 0.1 ether, ""));
        op.gasFees = bytes32((uint256(1000 gwei) << 128) | uint256(1000 gwei)); // 4.1M gas * 1000 gwei = 4.1 ether
        op = _signOpEcdsa(op, ownerPk);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, "AA24 signature error"));
        ep.handleOps(ops, payable(bundler));

        // normal gas price works
        _handle(_signOpEcdsa(_userOp(address(acct), _execData(friend, 0.1 ether, "")), ownerPk));
        assertEq(friend.balance, 0.1 ether);
        assertGt(acct.gasBudgetSpent(), 0);
    }

    function test_gasBudget_resetsDaily() public {
        // The budget counts value moved from the account to the EntryPoint (refunds stay there as a deposit and
        // are re-used before any new prefund is requested).
        PackedUserOperation memory op = _userOp(address(acct), _execData(friend, 1, ""));
        op.gasFees = bytes32((uint256(200 gwei) << 128) | uint256(200 gwei)); // 0.82 ether prefund
        _handle(_signOpEcdsa(op, ownerPk));
        assertEq(acct.gasBudgetSpent(), 0.82 ether);

        PackedUserOperation memory op3 = _userOp(address(acct), _execData(friend, 1, ""));
        op3.gasFees = bytes32((uint256(400 gwei) << 128) | uint256(400 gwei)); // 1.64 ether - deposit ~0.79
        op3 = _signOpEcdsa(op3, ownerPk);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op3;
        vm.prank(bundler, bundler);
        vm.expectRevert();
        ep.handleOps(ops, payable(bundler));

        vm.warp(block.timestamp + 1 days);
        acct.pokeGasBudget();
        assertEq(acct.gasBudgetSpent(), 0);
        _handle(op3);
    }

    // ------------------------------------------------------------------------------------------------------------
    // Phase 4 exit-criterion demo: the "stolen key" scenario (plan §27)
    // ------------------------------------------------------------------------------------------------------------

    function test_demo_stolenKey_guardianCancelsWithinWindow() public {
        uint256 startBalance = address(acct).balance;
        uint256 usdStart = usd.balanceOf(address(acct));

        // The attacker has the owner's key (we sign with `ownerPk` / prank `owner`).
        // 1. Large transfer through ERC-4337 -> queued, VaultActionQueued emitted (wallet + alert bot notify owner).
        PackedUserOperation memory op = _userOp(address(acct), _execData(attacker, 90 ether, ""));
        vm.recordLogs();
        _handle(_signOpEcdsa(op, ownerPk));
        uint256 drainId = acct.lastActionId();
        assertGt(drainId, 0);
        assertEq(attacker.balance, 0);

        // 2. The attacker tries to switch protection off / take over / grab tokens: all queued or blocked.
        vm.startPrank(owner);
        VaultPolicy memory off = _vaultPolicy();
        off.enabled = false;
        acct.setVaultPolicy(off);
        uint256 offId = acct.lastActionId();
        acct.setGuardians(new address[](0), 0);
        uint256 guardId = acct.lastActionId();
        acct.setWhitelisted(attacker, true);
        uint256 wlId = acct.lastActionId();
        acct.execute(address(usd), 0, abi.encodeCall(usd.transfer, (attacker, usdStart)));
        uint256 usdId = acct.lastActionId();
        vm.expectRevert(abi.encodeWithSelector(IAccountEvents.UnlimitedApprovalBlocked.selector, attacker));
        acct.execute(address(usd), 0, abi.encodeCall(usd.approve, (attacker, type(uint256).max)));
        // the most it can take instantly is the daily limit (in small chunks)
        for (uint256 i; i < 5; ++i) {
            acct.execute(attacker, 1 ether, "");
        }
        assertGt(acct.execute(attacker, 1 ether, ""), 0, "daily limit reached -> queued");
        uint256 lastId = acct.lastActionId();
        vm.stopPrank();

        // 3. Within the 24 h window a guardian (or the real owner on another device) cancels everything.
        vm.warp(block.timestamp + 6 hours);
        vm.startPrank(guardian1);
        for (uint256 id = drainId; id <= lastId; ++id) {
            (, uint8 status,,) = acct.getAction(id);
            if (status == 1) acct.cancel(id);
        }
        vm.stopPrank();

        // 4. After the delay nothing can be executed.
        vm.warp(block.timestamp + 2 days);
        uint256[5] memory ids = [drainId, offId, guardId, wlId, usdId];
        for (uint256 i; i < ids.length; ++i) {
            vm.expectRevert(abi.encodeWithSelector(IAccountEvents.ActionNotQueued.selector, ids[i]));
            acct.execute(ids[i]);
        }

        // Funds stay: only the bounded daily allowance (+gas) left the account; tokens untouched; vault still on.
        assertEq(attacker.balance, 5 ether);
        assertGe(address(acct).balance, startBalance - 5 ether - 0.1 ether);
        assertEq(usd.balanceOf(address(acct)), usdStart);
        assertTrue(acct.vaultPolicy().enabled);
        assertTrue(acct.isGuardian(guardian1));
        assertFalse(acct.isWhitelisted(attacker));
    }
}

contract GuardianVaultFuzzTest is AccountTestBase {
    IXAccount internal acct;
    address internal friend = makeAddr("friend");

    function setUp() public override {
        super.setUp();
        acct = _vaultAccount(); // 1 ether per tx, 5 ether per day, 24 h
        vm.deal(address(acct), 1_000 ether);
    }

    /// @dev A native transfer is instant iff it is within the per-tx threshold and the remaining daily limit.
    function testFuzz_nativeClassification(uint256 first, uint256 second) public {
        first = bound(first, 1, 10 ether);
        second = bound(second, 1, 10 ether);
        uint256 spent;
        uint256[2] memory amounts = [first, second];
        for (uint256 i; i < 2; ++i) {
            bool instant = amounts[i] <= 1 ether && spent + amounts[i] <= 5 ether;
            vm.prank(owner);
            uint256 id = acct.execute(friend, amounts[i], "");
            assertEq(id == 0, instant);
            if (instant) spent += amounts[i];
        }
        assertEq(friend.balance, spent);
    }

    /// @dev A queued action never executes before `executeAfter`, always executes inside the window.
    function testFuzz_noEarlyExecution(uint256 amount, uint256 wait) public {
        amount = bound(amount, 1 ether + 1, 100 ether);
        wait = bound(wait, 0, 24 hours + 14 days + 1 hours);
        vm.prank(owner);
        uint256 id = acct.execute(friend, amount, "");
        (uint64 executeAfter,,,) = acct.getAction(id);
        vm.warp(block.timestamp + wait);
        (bool ok,) = address(acct).call(abi.encodeWithSignature("execute(uint256)", id));
        assertEq(ok, block.timestamp >= executeAfter && block.timestamp <= executeAfter + 14 days);
        assertEq(friend.balance, ok ? amount : 0);
    }

    /// @dev Once cancelled, an action can never execute, whoever cancels and whenever execution is tried.
    function testFuzz_cancelledNeverExecutes(uint256 cancelAt, uint256 execAt, uint8 who) public {
        vm.prank(owner);
        uint256 id = acct.execute(friend, 2 ether, "");
        vm.warp(block.timestamp + bound(cancelAt, 0, 24 hours + 13 days));
        address[4] memory cancellers = [owner, guardian1, guardian2, guardian3];
        vm.prank(cancellers[who % 4]);
        acct.cancel(id);
        vm.warp(block.timestamp + bound(execAt, 0, 30 days));
        (bool ok,) = address(acct).call(abi.encodeWithSignature("execute(uint256)", id));
        assertFalse(ok);
        assertEq(friend.balance, 0);
    }
}
