// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {SystemFixture} from "./utils/SystemFixture.sol";
import {Treasury} from "../../src/system/Treasury.sol";
import {FeeConfig} from "../../src/system/FeeConfig.sol";
import {WXDS} from "../../src/system/WXDS.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract MockToken is ERC20 {
    constructor() ERC20("Mock", "MOCK") {
        _mint(msg.sender, 1_000 ether);
    }
}

contract NoReceive {}

contract ReentrantUnwrapper {
    WXDS internal immutable w;
    uint256 public reentered;

    constructor(WXDS w_) {
        w = w_;
    }

    function go() external payable {
        w.deposit{value: msg.value}();
        w.withdraw(msg.value);
    }

    receive() external payable {
        if (reentered == 0) {
            reentered = 1;
            // balance already debited: a second withdraw must fail
            try w.withdraw(msg.value) {
                reentered = 2;
            } catch {}
        }
    }
}

contract TreasuryTest is SystemFixture {
    address internal alice = makeAddr("alice");

    function setUp() public {
        _etch(address(gov), "Governance.sol:Governance");
        _etch(address(treasury), "Treasury.sol:Treasury");
        vm.deal(address(treasury), 100 ether);
        vm.prank(SYS);
        treasury.initialize(address(gov));
    }

    function test_initialize_reverts() public {
        vm.expectRevert(Treasury.OnlySystem.selector);
        treasury.initialize(address(gov));
        vm.prank(SYS);
        vm.expectRevert(Treasury.AlreadyInitialized.selector);
        treasury.initialize(address(gov));
    }

    function test_initialize_zeroGovernance() public {
        Treasury fresh = Treasury(payable(makeAddr("fresh-treasury")));
        vm.etch(address(fresh), vm.getDeployedCode("Treasury.sol:Treasury"));
        vm.prank(SYS);
        vm.expectRevert(Treasury.ZeroAddress.selector);
        fresh.initialize(address(0));
        // uninitialized: nobody (not even address(0)) can grant
        vm.prank(address(0));
        vm.expectRevert(Treasury.OnlyGovernance.selector);
        fresh.grant(payable(alice), 0, "");
    }

    function test_protocolCreditWithoutCall() public {
        // the node credits base fees by raising the balance directly: no event, no receive()
        vm.deal(address(treasury), address(treasury).balance + 5 ether);
        assertEq(treasury.balance(), 105 ether);
        vm.prank(address(gov));
        treasury.grant(payable(alice), 105 ether, "all");
        assertEq(alice.balance, 105 ether);
    }

    function test_receive() public {
        vm.deal(alice, 1 ether);
        vm.expectEmit(address(treasury));
        emit Treasury.Deposited(alice, 1 ether);
        vm.prank(alice);
        (bool ok,) = address(treasury).call{value: 1 ether}("");
        assertTrue(ok);
    }

    function test_grant() public {
        vm.expectEmit(address(treasury));
        emit Treasury.Granted(alice, 10 ether, "memo");
        vm.prank(address(gov));
        treasury.grant(payable(alice), 10 ether, "memo");
        assertEq(alice.balance, 10 ether);
        assertEq(treasury.totalGranted(), 10 ether);
    }

    function test_grant_reverts() public {
        vm.expectRevert(Treasury.OnlyGovernance.selector);
        treasury.grant(payable(alice), 1, "");
        vm.startPrank(address(gov));
        vm.expectRevert(Treasury.ZeroAddress.selector);
        treasury.grant(payable(address(0)), 1, "");
        vm.expectRevert(abi.encodeWithSelector(Treasury.InsufficientBalance.selector, 100 ether, 101 ether));
        treasury.grant(payable(alice), 101 ether, "");
        address nr = address(new NoReceive());
        vm.expectRevert(Treasury.TransferFailed.selector);
        treasury.grant(payable(nr), 1 ether, "");
        vm.stopPrank();
    }

    function test_grantToken() public {
        MockToken t = new MockToken();
        t.transfer(address(treasury), 100 ether);
        vm.expectEmit(address(treasury));
        emit Treasury.TokenGranted(address(t), alice, 40 ether, "t");
        vm.prank(address(gov));
        treasury.grantToken(IERC20(address(t)), alice, 40 ether, "t");
        assertEq(t.balanceOf(alice), 40 ether);

        vm.expectRevert(Treasury.OnlyGovernance.selector);
        treasury.grantToken(IERC20(address(t)), alice, 1, "");
        vm.startPrank(address(gov));
        vm.expectRevert(Treasury.ZeroAddress.selector);
        treasury.grantToken(IERC20(address(t)), address(0), 1, "");
        vm.expectRevert(Treasury.ZeroAddress.selector);
        treasury.grantToken(IERC20(address(0)), alice, 1, "");
        vm.stopPrank();
    }

    function testFuzz_grant(uint256 amount) public {
        amount = bound(amount, 0, 100 ether);
        vm.prank(address(gov));
        treasury.grant(payable(alice), amount, "");
        assertEq(alice.balance + address(treasury).balance, 100 ether);
    }
}

contract FeeConfigTest is SystemFixture {
    function setUp() public {
        _etch(address(feeConfig), "FeeConfig.sol:FeeConfig");
        vm.prank(SYS);
        feeConfig.initialize(address(gov), _defaultFeeParams());
    }

    function test_initialize() public view {
        assertEq(feeConfig.governance(), address(gov));
        assertEq(feeConfig.blockGasLimit(), 30_000_000);
        assertEq(feeConfig.minBaseFee(), 0.1 gwei);
        assertEq(feeConfig.burnBps(), 8_000);
        assertEq(feeConfig.treasuryBps(), 2_000);
        assertEq(feeConfig.baseFeeMaxChangeDenominator(), 8);
        assertEq(feeConfig.elasticityMultiplier(), 2);
        assertEq(feeConfig.version(), 1);
        FeeConfig.FeeParams memory p = feeConfig.params();
        assertEq(keccak256(abi.encode(p)), keccak256(abi.encode(_defaultFeeParams())));
    }

    function test_initialize_reverts() public {
        FeeConfig.FeeParams memory p = _defaultFeeParams();
        vm.expectRevert(FeeConfig.OnlySystem.selector);
        feeConfig.initialize(address(gov), p);
        vm.prank(SYS);
        vm.expectRevert(FeeConfig.AlreadyInitialized.selector);
        feeConfig.initialize(address(gov), p);

        FeeConfig fresh = FeeConfig(makeAddr("fresh-fee"));
        vm.etch(address(fresh), vm.getDeployedCode("FeeConfig.sol:FeeConfig"));
        vm.prank(SYS);
        vm.expectRevert(FeeConfig.ZeroAddress.selector);
        fresh.initialize(address(0), p);
        vm.prank(address(0));
        vm.expectRevert(FeeConfig.OnlyGovernance.selector);
        fresh.setParams(p);
    }

    /// @dev The node reads these slots directly: this test pins the layout documented in the spec.
    function test_rawStorageLayout() public view {
        address a = address(feeConfig);
        assertEq(address(uint160(uint256(vm.load(a, bytes32(uint256(0)))))), address(gov));
        assertEq(uint256(vm.load(a, bytes32(uint256(1)))), 30_000_000);
        assertEq(uint256(vm.load(a, bytes32(uint256(2)))), 0.1 gwei);
        assertEq(uint256(vm.load(a, bytes32(uint256(3)))), 8_000);
        assertEq(uint256(vm.load(a, bytes32(uint256(4)))), 8);
        assertEq(uint256(vm.load(a, bytes32(uint256(5)))), 2);
        assertEq(uint256(vm.load(a, bytes32(uint256(6)))), 128 * 1024);
        assertEq(uint256(vm.load(a, bytes32(uint256(7)))), 0);
        assertEq(uint256(vm.load(a, bytes32(uint256(8)))), 2_500);
        assertEq(uint256(vm.load(a, bytes32(uint256(9)))), 8);
        assertEq(uint256(vm.load(a, bytes32(uint256(10)))), 1);
    }

    function test_setParams() public {
        FeeConfig.FeeParams memory p = _defaultFeeParams();
        p.blockGasLimit = 60_000_000;
        p.localFeeEnabled = 1;
        vm.expectEmit(address(feeConfig));
        emit FeeConfig.ParamsUpdated(2, p);
        vm.prank(address(gov));
        feeConfig.setParams(p);
        assertEq(feeConfig.blockGasLimit(), 60_000_000);
        assertEq(feeConfig.localFeeEnabled(), 1);
        assertEq(feeConfig.version(), 2);
    }

    function test_setParams_onlyGovernance() public {
        FeeConfig.FeeParams memory p = _defaultFeeParams();
        vm.expectRevert(FeeConfig.OnlyGovernance.selector);
        feeConfig.setParams(p);
    }

    function test_setParams_validation() public {
        FeeConfig.FeeParams memory p = _defaultFeeParams();
        p.blockGasLimit = 999_999;
        _expectInvalid(p, "blockGasLimit");
        p.blockGasLimit = 1_000_000_001;
        _expectInvalid(p, "blockGasLimit");
        p = _defaultFeeParams();
        p.minBaseFee = 0;
        _expectInvalid(p, "minBaseFee");
        p.minBaseFee = 1_000 gwei + 1;
        _expectInvalid(p, "minBaseFee");
        p = _defaultFeeParams();
        p.burnBps = 10_001;
        _expectInvalid(p, "burnBps");
        p = _defaultFeeParams();
        p.baseFeeMaxChangeDenominator = 0;
        _expectInvalid(p, "baseFeeMaxChangeDenominator");
        p.baseFeeMaxChangeDenominator = 1_001;
        _expectInvalid(p, "baseFeeMaxChangeDenominator");
        p = _defaultFeeParams();
        p.elasticityMultiplier = 0;
        _expectInvalid(p, "elasticityMultiplier");
        p.elasticityMultiplier = 101;
        _expectInvalid(p, "elasticityMultiplier");
        p = _defaultFeeParams();
        p.maxTxSizeBytes = 1_023;
        _expectInvalid(p, "maxTxSizeBytes");
        p.maxTxSizeBytes = 4 * 1024 * 1024 + 1;
        _expectInvalid(p, "maxTxSizeBytes");
        p = _defaultFeeParams();
        p.localFeeEnabled = 2;
        _expectInvalid(p, "localFeeEnabled");
        p = _defaultFeeParams();
        p.localFeeThresholdBps = 0;
        _expectInvalid(p, "localFeeThresholdBps");
        p.localFeeThresholdBps = 10_001;
        _expectInvalid(p, "localFeeThresholdBps");
        p = _defaultFeeParams();
        p.localFeeMaxChangeDenominator = 0;
        _expectInvalid(p, "localFeeMaxChangeDenominator");
        p.localFeeMaxChangeDenominator = 1_001;
        _expectInvalid(p, "localFeeMaxChangeDenominator");
    }

    function _expectInvalid(FeeConfig.FeeParams memory p, string memory name) internal {
        vm.prank(address(gov));
        vm.expectRevert(abi.encodeWithSelector(FeeConfig.InvalidParam.selector, name));
        feeConfig.setParams(p);
    }

    function testFuzz_setParams_gasLimit(uint256 limit) public {
        FeeConfig.FeeParams memory p = _defaultFeeParams();
        p.blockGasLimit = bound(limit, 1_000_000, 1_000_000_000);
        vm.prank(address(gov));
        feeConfig.setParams(p);
        assertEq(uint256(vm.load(address(feeConfig), bytes32(uint256(1)))), p.blockGasLimit);
    }
}

contract WXDSTest is SystemFixture {
    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");

    function setUp() public {
        _etch(address(wxds), "WXDS.sol:WXDS");
        vm.deal(alice, 100 ether);
    }

    function test_metadata() public view {
        assertEq(wxds.name(), "Wrapped XDS");
        assertEq(wxds.symbol(), "WXDS");
        assertEq(wxds.decimals(), 18);
    }

    function test_depositWithdraw() public {
        vm.expectEmit(address(wxds));
        emit WXDS.Deposit(alice, 10 ether);
        vm.prank(alice);
        wxds.deposit{value: 10 ether}();
        assertEq(wxds.balanceOf(alice), 10 ether);
        assertEq(wxds.totalSupply(), 10 ether);

        vm.prank(alice);
        (bool ok,) = address(wxds).call{value: 5 ether}("");
        assertTrue(ok);
        assertEq(wxds.balanceOf(alice), 15 ether);

        vm.expectEmit(address(wxds));
        emit WXDS.Withdrawal(alice, 15 ether);
        vm.prank(alice);
        wxds.withdraw(15 ether);
        assertEq(alice.balance, 100 ether);
        assertEq(wxds.totalSupply(), 0);

        vm.prank(alice);
        vm.expectRevert(WXDS.InsufficientBalance.selector);
        wxds.withdraw(1);
    }

    function test_withdraw_transferFailed() public {
        NoReceive nr = new NoReceive();
        vm.prank(alice);
        wxds.deposit{value: 1 ether}();
        vm.prank(alice);
        wxds.transfer(address(nr), 1 ether);
        vm.prank(address(nr));
        vm.expectRevert(WXDS.TransferFailed.selector);
        wxds.withdraw(1 ether);
    }

    function test_withdraw_reentrancySafe() public {
        ReentrantUnwrapper r = new ReentrantUnwrapper(wxds);
        vm.deal(address(this), 1 ether);
        r.go{value: 1 ether}();
        assertEq(r.reentered(), 1);
        assertEq(address(r).balance, 1 ether);
        assertEq(wxds.totalSupply(), 0);
    }

    function test_transferAndAllowance() public {
        vm.startPrank(alice);
        wxds.deposit{value: 10 ether}();
        vm.expectEmit(address(wxds));
        emit WXDS.Transfer(alice, bob, 3 ether);
        assertTrue(wxds.transfer(bob, 3 ether));
        vm.expectEmit(address(wxds));
        emit WXDS.Approval(alice, bob, 4 ether);
        assertTrue(wxds.approve(bob, 4 ether));
        vm.stopPrank();

        vm.prank(bob);
        wxds.transferFrom(alice, bob, 4 ether);
        assertEq(wxds.allowance(alice, bob), 0);
        assertEq(wxds.balanceOf(bob), 7 ether);

        vm.prank(bob);
        vm.expectRevert(WXDS.InsufficientAllowance.selector);
        wxds.transferFrom(alice, bob, 1);

        vm.prank(alice);
        wxds.approve(bob, type(uint256).max);
        vm.prank(bob);
        wxds.transferFrom(alice, bob, 1 ether);
        assertEq(wxds.allowance(alice, bob), type(uint256).max);

        vm.prank(bob);
        vm.expectRevert(WXDS.InsufficientBalance.selector);
        wxds.transferFrom(alice, bob, 100 ether);
    }

    function testFuzz_wrapUnwrap(uint256 amount) public {
        amount = bound(amount, 0, 100 ether);
        vm.startPrank(alice);
        wxds.deposit{value: amount}();
        wxds.withdraw(amount);
        vm.stopPrank();
        assertEq(alice.balance, 100 ether);
        assertEq(wxds.totalSupply(), 0);
    }
}
