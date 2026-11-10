// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {Predeploys} from "../../src/common/Predeploys.sol";
import {XdevsPaymaster} from "../../src/accounts/paymaster/XdevsPaymaster.sol";
import {StablecoinPaymaster} from "../../src/accounts/paymaster/StablecoinPaymaster.sol";

contract PaymasterTest is AccountTestBase {
    bytes8 internal constant PM_SIG_MAGIC = 0x22e325a297439656;

    address internal app = makeAddr("app");
    uint256 internal appSignerPk;
    address internal appSigner;
    IXAccount internal acct;
    address internal to = makeAddr("to");

    function setUp() public override {
        super.setUp();
        (appSigner, appSignerPk) = makeAddrAndKey("app-signer");
        acct = _vaultAccount();
        vm.deal(app, 100 ether);
        vm.prank(app);
        paymaster.deposit{value: 10 ether}(app);
    }

    function _pmData(address sponsor) internal view returns (bytes memory) {
        return abi.encodePacked(address(paymaster), uint128(200_000), uint128(100_000), sponsor);
    }

    function _sponsoredOp(address sponsor) internal view returns (PackedUserOperation memory op) {
        op = _userOp(address(acct), _execData(to, 0.1 ether, ""));
        op.paymasterAndData = _pmData(sponsor);
        op = _signOpEcdsa(op, ownerPk);
    }

    function _expectFail(PackedUserOperation memory op) internal {
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        vm.expectRevert();
        ep.handleOps(ops, payable(bundler));
    }

    function _assertSolvent() internal view {
        assertGe(ep.balanceOf(address(paymaster)), paymaster.totalSponsorBalance(), "paymaster insolvent");
    }

    function test_openSponsorship_paysGas() public {
        vm.prank(app);
        paymaster.configure(true, true, 0, 0, address(0));
        uint256 accountBefore = address(acct).balance;
        uint256 sponsorBefore = paymaster.sponsorInfo(app).balance;

        _handle(_sponsoredOp(app));

        assertEq(to.balance, 0.1 ether);
        assertEq(address(acct).balance, accountBefore - 0.1 ether, "user paid no gas");
        assertLt(paymaster.sponsorInfo(app).balance, sponsorBefore, "sponsor paid");
        assertEq(acct.gasBudgetSpent(), 0, "sponsored ops do not consume the vault gas budget");
        _assertSolvent();
    }

    function test_inactiveOrAllowlistMode() public {
        // not configured -> inactive
        _expectFail(_sponsoredOp(app));
        // allowlist mode without the account
        vm.prank(app);
        paymaster.configure(true, false, 0, 0, address(0));
        _expectFail(_sponsoredOp(app));
        address[] memory list = new address[](1);
        list[0] = address(acct);
        vm.prank(app);
        paymaster.setAllowlist(list, true);
        _handle(_sponsoredOp(app));
        assertEq(to.balance, 0.1 ether);
        _assertSolvent();
    }

    function test_limits_maxCostPerOp_and_perAccountCap() public {
        vm.prank(app);
        paymaster.configure(true, true, 0.001 ether, 0, address(0)); // op max cost ~0.0044 ether
        _expectFail(_sponsoredOp(app));

        vm.prank(app);
        // maxCost = 4.4M gas * 1 gwei = 0.0044; after the first op ~0.0003 stays charged -> second op exceeds
        paymaster.configure(true, true, 0, 0.0046 ether, address(0));
        _handle(_sponsoredOp(app));
        _expectFail(_sponsoredOp(app));

        address[] memory list = new address[](1);
        list[0] = address(acct);
        vm.prank(app);
        paymaster.resetAccountSpend(list);
        _handle(_sponsoredOp(app));
        _assertSolvent();
    }

    function test_signerMode() public {
        vm.prank(app);
        paymaster.configure(true, false, 0, 0, appSigner);

        PackedUserOperation memory op = _userOp(address(acct), _execData(to, 0.1 ether, ""));
        uint48 validUntil = uint48(block.timestamp + 1 hours);
        bytes memory prefix = abi.encodePacked(_pmData(app), validUntil, uint48(0));
        // the paymaster signature suffix is excluded from the userOpHash (ERC-4337 v0.9)
        op.paymasterAndData = abi.encodePacked(prefix, new bytes(65), uint16(65), PM_SIG_MAGIC);
        bytes32 h = ep.getUserOpHash(op);
        bytes memory pmSig = _ecdsaSign(appSignerPk, _ethSigned(h));
        op.paymasterAndData = abi.encodePacked(prefix, pmSig, uint16(65), PM_SIG_MAGIC);
        assertEq(ep.getUserOpHash(op), h);
        op = _signOpEcdsa(op, ownerPk);
        _handle(op);
        assertEq(to.balance, 0.1 ether);

        // wrong sponsor signature
        PackedUserOperation memory bad = _userOp(address(acct), _execData(to, 0.1 ether, ""));
        bad.paymasterAndData =
            abi.encodePacked(prefix, _ecdsaSign(attackerPk, _ethSigned(h)), uint16(65), PM_SIG_MAGIC);
        bad = _signOpEcdsa(bad, ownerPk);
        _expectFail(bad);
        _assertSolvent();
    }

    function test_withdraw_onlyOwnBalance() public {
        vm.prank(attacker);
        vm.expectRevert(XdevsPaymaster.InsufficientBalance.selector);
        paymaster.withdraw(payable(attacker), 1 ether);

        vm.prank(app);
        paymaster.withdraw(payable(app), 4 ether);
        assertEq(paymaster.sponsorInfo(app).balance, 6 ether);
        assertEq(app.balance, 94 ether);
        _assertSolvent();
    }

    function test_stake_onlyGovernance_andSurplusToTreasury() public {
        paymaster.addStake{value: 1 ether}(1 days);
        vm.expectRevert(XdevsPaymaster.NotGovernance.selector);
        paymaster.unlockStake();
        vm.prank(Predeploys.GOVERNANCE);
        paymaster.unlockStake();

        // stray deposit -> surplus -> treasury; sponsor balances untouched
        ep.depositTo{value: 0.5 ether}(address(paymaster));
        assertEq(paymaster.surplus(), 0.5 ether);
        paymaster.sweepSurplus();
        assertEq(Predeploys.TREASURY.balance, 0.5 ether);
        assertEq(paymaster.sponsorInfo(app).balance, 10 ether);
        _assertSolvent();
    }

    function testFuzz_solvency_manyOps(uint8 n, uint64 feeGwei) public {
        n = uint8(bound(n, 1, 6));
        feeGwei = uint64(bound(feeGwei, 1, 50));
        vm.prank(app);
        paymaster.configure(true, true, 0, 0, address(0));
        for (uint256 i; i < n; ++i) {
            PackedUserOperation memory op = _userOp(address(acct), _execData(to, 1, ""));
            op.gasFees = bytes32((uint256(feeGwei) * 1 gwei << 128) | uint256(feeGwei) * 1 gwei);
            op.paymasterAndData = _pmData(app);
            _handle(_signOpEcdsa(op, ownerPk));
            _assertSolvent();
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Stablecoin gas (application-layer example)
    // ------------------------------------------------------------------------------------------------------------

    function test_stablecoinPaymaster_paysGasInUsd() public {
        address operator = makeAddr("operator");
        StablecoinPaymaster spm = new StablecoinPaymaster(ep, operator);
        vm.deal(operator, 10 ether);
        vm.startPrank(operator);
        spm.deposit{value: 5 ether}();
        spm.setToken(address(usd), true, 2e6); // 1 tXDS = 2 USD (6 decimals)
        spm.setMarkup(500); // 5%
        vm.stopPrank();

        // plain account (vault disabled) approves the paymaster
        IXAccount user = _createAccount(_params(_one(_ecdsaSigner(owner)), false), bytes32(uint256(77)));
        usd.mint(address(user), 100e6);
        vm.prank(owner);
        user.execute(address(usd), 0, abi.encodeCall(usd.approve, (address(spm), type(uint256).max)));
        uint256 nativeBefore = address(user).balance;

        PackedUserOperation memory op = _userOp(address(user), _execData(to, 0.1 ether, ""));
        op.paymasterAndData =
            abi.encodePacked(address(spm), uint128(200_000), uint128(100_000), address(usd), uint256(2.1e6));
        _handle(_signOpEcdsa(op, ownerPk));

        assertEq(to.balance, 0.1 ether);
        assertEq(address(user).balance, nativeBefore - 0.1 ether, "no tXDS spent on gas");
        uint256 paidUsd = 100e6 - usd.balanceOf(address(user));
        assertGt(paidUsd, 0);
        assertEq(usd.balanceOf(address(spm)), paidUsd);
        assertLt(paidUsd, spm.quote(4.4e6 gwei, spm.effectiveRate(address(usd))), "refunded unused prefund");

        // the user's rate guard: operator raises the price -> op rejected
        vm.prank(operator);
        spm.setToken(address(usd), true, 3e6);
        PackedUserOperation memory op2 = _userOp(address(user), _execData(to, 0.1 ether, ""));
        op2.paymasterAndData =
            abi.encodePacked(address(spm), uint128(200_000), uint128(100_000), address(usd), uint256(2.1e6));
        op2 = _signOpEcdsa(op2, ownerPk);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op2;
        vm.prank(bundler, bundler);
        vm.expectRevert();
        ep.handleOps(ops, payable(bundler));
    }
}
