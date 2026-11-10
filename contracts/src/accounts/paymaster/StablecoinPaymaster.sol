// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {IPaymaster} from "account-abstraction/interfaces/IPaymaster.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {UserOperationLib} from "account-abstraction/core/UserOperationLib.sol";

import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";

/// @title StablecoinPaymaster
/// @notice Example "pay gas in stablecoins" ERC-4337 v0.9 paymaster (plan §15.3): users pay in an approved ERC-20
///         at an owner/governance-set rate, the paymaster pays the gas in tXDS. Application layer only — the
///         protocol has no oracle.
/// @dev Normal (non-genesis) deployment, so constructor args / immutables are fine.
///      `paymasterData = token (20) | maxTokensPerNative (32)` — the user's slippage guard: the op is rejected if
///      the effective rate (incl. markup) is above what the user signed (the paymasterData is covered by the
///      userOpHash), so the operator cannot raise the price under a pending op.
///      Rates are "token base units per 1e18 wei", e.g. a 6-decimals USD stablecoin with XDS = $0.50 →
///      `tokensPerNative = 0.5e6`.
///      Flow: validation pulls a worst-case prefund (`maxCost * rate`) from the sender (needs a prior `approve` —
///      with the Guardian Vault enabled, whitelist this paymaster or the approval is queued), postOp refunds the
///      unused part. The operator's tXDS deposit and collected tokens belong to the operator; the contract never
///      holds user funds beyond the in-flight prefund.
// aderyn-fp-next-line(contract-locks-ether) SA-A02: msg.value forwarded to the EntryPoint
contract StablecoinPaymaster is IPaymaster, Ownable2Step {
    using SafeERC20 for IERC20;
    using UserOperationLib for PackedUserOperation;

    struct TokenConfig {
        bool enabled;
        uint256 tokensPerNative;
    }

    uint256 public constant POSTOP_OVERHEAD = 30_000;
    uint256 public constant MAX_MARKUP_BPS = 2_000;

    IEntryPoint public immutable entryPoint;
    uint256 public markupBps;
    mapping(address token => TokenConfig) public tokens;

    event TokenConfigured(address indexed token, bool enabled, uint256 tokensPerNative);
    event MarkupUpdated(uint256 markupBps);
    event GasPaidInToken(
        address indexed account, address indexed token, uint256 tokenAmount, uint256 gasCost
    );

    error NotFromEntryPoint();
    error TokenNotSupported(address token);
    error RateAboveLimit(uint256 rate, uint256 maxRate);
    error InvalidPaymasterData();
    error InvalidParameter();

    constructor(IEntryPoint entryPoint_, address owner_) Ownable(owner_) {
        entryPoint = entryPoint_;
    }

    // ============================================================================================================
    // Operator
    // ============================================================================================================

    function setToken(address token, bool enabled, uint256 tokensPerNative) external onlyOwner {
        if (token == address(0) || (enabled && tokensPerNative == 0)) revert InvalidParameter();
        tokens[token] = TokenConfig(enabled, tokensPerNative);
        emit TokenConfigured(token, enabled, tokensPerNative);
    }

    function setMarkup(uint256 bps) external onlyOwner {
        if (bps > MAX_MARKUP_BPS) revert InvalidParameter();
        markupBps = bps;
        emit MarkupUpdated(bps);
    }

    /// @notice Effective rate (incl. markup) in token units per 1e18 wei.
    function effectiveRate(address token) public view returns (uint256) {
        TokenConfig memory c = tokens[token];
        if (!c.enabled) revert TokenNotSupported(token);
        return Math.mulDiv(c.tokensPerNative, 10_000 + markupBps, 10_000, Math.Rounding.Ceil);
    }

    /// @notice Token amount for `gasCost` wei at `rate`.
    function quote(uint256 gasCost, uint256 rate) public pure returns (uint256) {
        return Math.mulDiv(gasCost, rate, 1e18, Math.Rounding.Ceil);
    }

    function deposit() external payable {
        entryPoint.depositTo{value: msg.value}(address(this));
    }

    function withdrawDeposit(address payable to, uint256 amount) external onlyOwner {
        entryPoint.withdrawTo(to, amount);
    }

    function withdrawTokens(IERC20 token, address to, uint256 amount) external onlyOwner {
        token.safeTransfer(to, amount);
    }

    function addStake(uint32 unstakeDelaySec) external payable onlyOwner {
        entryPoint.addStake{value: msg.value}(unstakeDelaySec);
    }

    function unlockStake() external onlyOwner {
        entryPoint.unlockStake();
    }

    function withdrawStake(address payable to) external onlyOwner {
        entryPoint.withdrawStake(to);
    }

    // ============================================================================================================
    // ERC-4337
    // ============================================================================================================

    /// @inheritdoc IPaymaster
    function validatePaymasterUserOp(PackedUserOperation calldata userOp, bytes32, uint256 maxCost)
        external
        returns (bytes memory context, uint256 validationData)
    {
        if (msg.sender != address(entryPoint)) revert NotFromEntryPoint();
        bytes calldata data = userOp.paymasterAndData[UserOperationLib.PAYMASTER_DATA_OFFSET:];
        if (data.length < 52) revert InvalidPaymasterData();
        address token = address(bytes20(data[:20]));
        uint256 maxRate = uint256(bytes32(data[20:52]));
        uint256 rate = effectiveRate(token);
        if (rate > maxRate) revert RateAboveLimit(rate, maxRate);

        uint256 prefund = quote(maxCost, rate);
        // slither-disable-next-line arbitrary-send-erc20 (SA-F02: sender signed this op and approved)
        IERC20(token).safeTransferFrom(userOp.sender, address(this), prefund);
        context = abi.encode(userOp.sender, token, prefund, rate, userOp.unpackPostOpGasLimit());
        validationData = 0;
    }

    /// @inheritdoc IPaymaster
    function postOp(PostOpMode, bytes calldata context, uint256 actualGasCost, uint256 actualUserOpFeePerGas)
        external
    {
        if (msg.sender != address(entryPoint)) revert NotFromEntryPoint();
        (address account, address token, uint256 prefund, uint256 rate, uint256 postOpGasLimit) =
            abi.decode(context, (address, address, uint256, uint256, uint256));
        uint256 gasCost = actualGasCost + (postOpGasLimit + POSTOP_OVERHEAD) * actualUserOpFeePerGas;
        uint256 cost = quote(gasCost, rate);
        if (cost > prefund) cost = prefund;
        uint256 refund = prefund - cost;
        if (refund != 0) IERC20(token).safeTransfer(account, refund);
        emit GasPaidInToken(account, token, cost, gasCost);
    }
}
