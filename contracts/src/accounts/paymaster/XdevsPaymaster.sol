// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {IPaymaster} from "account-abstraction/interfaces/IPaymaster.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {UserOperationLib} from "account-abstraction/core/UserOperationLib.sol";
import {_packValidationData} from "account-abstraction/core/Helpers.sol";

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

import {Predeploys} from "../../common/Predeploys.sol";
import {AccountConstants} from "../libraries/AccountConstants.sol";

/// @title XdevsPaymaster
/// @notice Shared, deposit-based ERC-4337 v0.9 gas-sponsorship paymaster (plan §15.3), predeployed at
///         `Predeploys.PAYMASTER`. Any app ("sponsor") deposits tXDS and sets its own rules; users' operations name
///         the sponsor in `paymasterData` and are paid from that sponsor's balance.
/// @dev Genesis predeploy: no constructor, no immutables, no init params, no owner. Every sponsor controls only its
///      own balance and rules; nobody can move a sponsor's balance except the sponsor (withdraw) and the gas its own
///      rules accept. Governance (`Predeploys.GOVERNANCE`) can only unlock / withdraw the EntryPoint *stake* (the
///      ERC-7562 reputation bond, funded by anyone via `addStake`), never sponsor deposits.
///
///      `paymasterAndData = paymaster (20) | verificationGasLimit (16) | postOpGasLimit (16) | paymasterData`
///        * open / allowlist mode : `paymasterData = sponsor (20)`
///        * signer mode           : `paymasterData = sponsor (20) | validUntil (6) | validAfter (6)` followed by the
///                                  v0.9 paymaster-signature suffix `sig (65) | uint16(65) | PAYMASTER_SIG_MAGIC`;
///                                  `sig` = sponsor signer's EIP-191 signature over the userOpHash (which covers
///                                  sponsor, validUntil and validAfter but not the suffix).
///
///      Sponsor rules: `active`, `maxCostPerOp` (0 = unlimited), `perAccountCap` (lifetime gas per account, 0 =
///      unlimited, resettable by the sponsor), sender allowlist or `openToAll`, optional off-chain `signer`.
///      ERC-7562 forbids TIMESTAMP in validation, so time-based limits belong in signer mode (`validUntil`).
///
///      Accounting: validation reserves `maxCost` from the sponsor's balance; `postOp` charges
///      `min(maxCost, actualGasCost + (postOpGasLimit + POSTOP_OVERHEAD) * feePerGas)` — never less than what the
///      EntryPoint actually deducts — and refunds the rest. Hence `entryPoint.balanceOf(this) >= totalSponsorBalance`
///      always; the (small) surplus can be swept to the Treasury by anyone.
///      This paymaster must be staked in the EntryPoint (it reads sponsor-keyed storage during validation).
contract XdevsPaymaster is IPaymaster {
    using UserOperationLib for PackedUserOperation;
    using UserOperationLib for bytes;

    struct Sponsor {
        uint128 balance;
        uint128 maxCostPerOp;
        uint128 perAccountCap;
        bool active;
        bool openToAll;
        address signer;
    }

    /// @custom:storage-location erc7201:xdevs.paymaster.main
    struct PaymasterStorage {
        mapping(address sponsor => Sponsor) sponsors;
        mapping(address sponsor => mapping(address account => bool)) allowlist;
        mapping(address sponsor => mapping(address account => uint256)) spent;
        uint256 totalSponsorBalance;
    }

    // keccak256(abi.encode(uint256(keccak256("xdevs.paymaster.main")) - 1)) & ~bytes32(uint256(0xff))
    bytes32 private constant STORAGE_SLOT =
        0x8d3f6c7b5b47817ccada215d77f06104b1ae363f60066aafad2762303b92f800;

    /// @dev Gas charged on top of `postOpGasLimit` for the EntryPoint's own post-postOp bookkeeping.
    uint256 public constant POSTOP_OVERHEAD = 15_000;

    event Deposited(address indexed sponsor, address indexed from, uint256 amount);
    event Withdrawn(address indexed sponsor, address indexed to, uint256 amount);
    event SponsorConfigured(
        address indexed sponsor,
        bool active,
        bool openToAll,
        uint128 maxCostPerOp,
        uint128 perAccountCap,
        address signer
    );
    event AllowlistUpdated(address indexed sponsor, address indexed account, bool allowed);
    event AccountSpendReset(address indexed sponsor, address indexed account);
    event Sponsored(address indexed sponsor, address indexed account, uint256 charged);
    event SurplusSwept(uint256 amount);

    error NotFromEntryPoint();
    error NotGovernance();
    error InsufficientBalance();
    error InvalidPaymasterData();
    error SponsorRejected(uint8 reason);

    function _s() private pure returns (PaymasterStorage storage $) {
        assembly ("memory-safe") {
            $.slot := STORAGE_SLOT
        }
    }

    function entryPoint() public pure returns (IEntryPoint) {
        return IEntryPoint(AccountConstants.ENTRY_POINT);
    }

    // ============================================================================================================
    // Sponsors
    // ============================================================================================================

    /// @notice Deposits `msg.value` for `sponsor` (anyone can top up any sponsor).
    function deposit(address sponsor) external payable {
        PaymasterStorage storage $ = _s();
        $.sponsors[sponsor].balance += uint128(msg.value);
        $.totalSponsorBalance += msg.value;
        entryPoint().depositTo{value: msg.value}(address(this));
        emit Deposited(sponsor, msg.sender, msg.value);
    }

    /// @notice Withdraws from the caller's own sponsor balance.
    function withdraw(address payable to, uint256 amount) external {
        PaymasterStorage storage $ = _s();
        Sponsor storage sp = $.sponsors[msg.sender];
        if (amount > sp.balance) revert InsufficientBalance();
        sp.balance -= uint128(amount);
        $.totalSponsorBalance -= amount;
        entryPoint().withdrawTo(to, amount);
        emit Withdrawn(msg.sender, to, amount);
    }

    /// @notice Configures the caller's sponsorship rules.
    function configure(
        bool active,
        bool openToAll,
        uint128 maxCostPerOp,
        uint128 perAccountCap,
        address signer
    ) external {
        Sponsor storage sp = _s().sponsors[msg.sender];
        sp.active = active;
        sp.openToAll = openToAll;
        sp.maxCostPerOp = maxCostPerOp;
        sp.perAccountCap = perAccountCap;
        sp.signer = signer;
        emit SponsorConfigured(msg.sender, active, openToAll, maxCostPerOp, perAccountCap, signer);
    }

    function setAllowlist(address[] calldata accounts, bool allowed) external {
        PaymasterStorage storage $ = _s();
        for (uint256 i; i < accounts.length; ++i) {
            $.allowlist[msg.sender][accounts[i]] = allowed;
            emit AllowlistUpdated(msg.sender, accounts[i], allowed);
        }
    }

    /// @notice Resets the per-account spend counters (e.g. a new "month" of free gas).
    function resetAccountSpend(address[] calldata accounts) external {
        PaymasterStorage storage $ = _s();
        for (uint256 i; i < accounts.length; ++i) {
            $.spent[msg.sender][accounts[i]] = 0;
            emit AccountSpendReset(msg.sender, accounts[i]);
        }
    }

    function sponsorInfo(address sponsor) external view returns (Sponsor memory) {
        return _s().sponsors[sponsor];
    }

    function isAllowlisted(address sponsor, address account) external view returns (bool) {
        return _s().allowlist[sponsor][account];
    }

    function accountSpent(address sponsor, address account) external view returns (uint256) {
        return _s().spent[sponsor][account];
    }

    function totalSponsorBalance() external view returns (uint256) {
        return _s().totalSponsorBalance;
    }

    // ============================================================================================================
    // ERC-4337
    // ============================================================================================================

    /// @inheritdoc IPaymaster
    function validatePaymasterUserOp(PackedUserOperation calldata userOp, bytes32 userOpHash, uint256 maxCost)
        external
        returns (bytes memory context, uint256 validationData)
    {
        if (msg.sender != address(entryPoint())) revert NotFromEntryPoint();
        bytes calldata data = userOp.paymasterAndData.getSignedPaymasterData();
        if (data.length < 20) revert InvalidPaymasterData();
        address sponsorAddr = address(bytes20(data[:20]));

        Sponsor storage sp = _reserve(sponsorAddr, userOp.sender, maxCost);
        if (sp.signer != address(0)) {
            validationData = _checkSponsorSignature(sp.signer, data, userOp.paymasterAndData, userOpHash);
        } else if (!sp.openToAll && !_s().allowlist[sponsorAddr][userOp.sender]) {
            revert SponsorRejected(5);
        }
        context = abi.encode(sponsorAddr, userOp.sender, maxCost, userOp.unpackPostOpGasLimit());
    }

    /// @dev Checks the sponsor rules and reserves the worst-case cost (settled in postOp).
    function _reserve(address sponsorAddr, address account, uint256 maxCost)
        private
        returns (Sponsor storage sp)
    {
        PaymasterStorage storage $ = _s();
        sp = $.sponsors[sponsorAddr];
        if (!sp.active) revert SponsorRejected(1);
        if (sp.maxCostPerOp != 0 && maxCost > sp.maxCostPerOp) revert SponsorRejected(2);
        if (maxCost > sp.balance) revert SponsorRejected(3);
        uint256 newSpent = $.spent[sponsorAddr][account] + maxCost;
        if (sp.perAccountCap != 0 && newSpent > sp.perAccountCap) revert SponsorRejected(4);
        sp.balance -= uint128(maxCost);
        $.totalSponsorBalance -= maxCost;
        $.spent[sponsorAddr][account] = newSpent;
    }

    function _checkSponsorSignature(
        address signer,
        bytes calldata data,
        bytes calldata paymasterAndData,
        bytes32 userOpHash
    ) private pure returns (uint256) {
        if (data.length != 32) revert InvalidPaymasterData();
        bytes calldata sig = paymasterAndData.getPaymasterSignature();
        (address rec, ECDSA.RecoverError err,) =
            ECDSA.tryRecoverCalldata(MessageHashUtils.toEthSignedMessageHash(userOpHash), sig);
        bool sigFailed = err != ECDSA.RecoverError.NoError || rec != signer;
        return _packValidationData(sigFailed, uint48(bytes6(data[20:26])), uint48(bytes6(data[26:32])));
    }

    /// @inheritdoc IPaymaster
    function postOp(PostOpMode, bytes calldata context, uint256 actualGasCost, uint256 actualUserOpFeePerGas)
        external
    {
        if (msg.sender != address(entryPoint())) revert NotFromEntryPoint();
        (address sponsorAddr, address account, uint256 maxCost, uint256 postOpGasLimit) =
            abi.decode(context, (address, address, uint256, uint256));
        uint256 charge = actualGasCost + (postOpGasLimit + POSTOP_OVERHEAD) * actualUserOpFeePerGas;
        if (charge > maxCost) charge = maxCost;
        uint256 refund = maxCost - charge;
        PaymasterStorage storage $ = _s();
        $.sponsors[sponsorAddr].balance += uint128(refund);
        $.totalSponsorBalance += refund;
        uint256 s = $.spent[sponsorAddr][account];
        $.spent[sponsorAddr][account] = s > refund ? s - refund : 0; // the sponsor may have reset it meanwhile
        emit Sponsored(sponsorAddr, account, charge);
    }

    // ============================================================================================================
    // Stake (ERC-7562 reputation) and surplus
    // ============================================================================================================

    /// @notice Anyone can fund / extend the paymaster's EntryPoint stake.
    function addStake(uint32 unstakeDelaySec) external payable {
        entryPoint().addStake{value: msg.value}(unstakeDelaySec);
    }

    /// @notice Governance only: unlock the stake (never touches sponsor deposits).
    function unlockStake() external {
        if (msg.sender != Predeploys.GOVERNANCE) revert NotGovernance();
        entryPoint().unlockStake();
    }

    /// @notice Governance only: withdraw the unlocked stake.
    function withdrawStake(address payable to) external {
        if (msg.sender != Predeploys.GOVERNANCE) revert NotGovernance();
        entryPoint().withdrawStake(to);
    }

    /// @notice EntryPoint deposit not owed to any sponsor (gas over-estimation margin, stray deposits).
    function surplus() public view returns (uint256) {
        uint256 bal = entryPoint().balanceOf(address(this));
        uint256 owed = _s().totalSponsorBalance;
        return bal > owed ? bal - owed : 0;
    }

    /// @notice Sends the surplus to the Treasury. Permissionless; can never touch sponsor balances.
    function sweepSurplus() external {
        uint256 amount = surplus();
        if (amount == 0) return;
        entryPoint().withdrawTo(payable(Predeploys.TREASURY), amount);
        emit SurplusSwept(amount);
    }
}
