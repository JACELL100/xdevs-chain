//! Phase 4 user-safety layer (plan §16): wire types of `xdevs_simulateTransaction`,
//! `xdevs_checkRecipient`, `xdevs_getApprovals`, `xdevs_getVaultQueue` and the
//! `eth_subscribe("vaultEvents")` subscription, plus the ABI constants and helpers they
//! share (event topics, selectors, unit formatting, XNS names, look-alike addresses).
//!
//! Principle 7 ("facts vs opinions"): on-chain data is fact; every warning and flag is
//! **advice** and is labelled with [`DISCLAIMER`]. Token symbols and decimals are
//! self-reported by the token contracts and are not verified.

use crate::{address, b256, keccak256, xip0001, Address, Bytes, EoaLockStatus, Log, B256, U256};
use alloy_primitives::U64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::LazyLock;

/// Disclaimer attached to every advisory result.
pub const DISCLAIMER: &str = "advice, not fact: warnings and flags are heuristics; simulated \
     effects may differ if state changes before the transaction is included";

/// `Predeploys.XNS_REGISTRY`: xDEVS Name Service registry + public resolver.
pub const XNS_REGISTRY: Address = address!("0x0000000000000000000000000000000000001004");
/// `Predeploys.SMART_ACCOUNT_IMPL`: the xDEVS Smart Account implementation (Guardian Vault).
pub const SMART_ACCOUNT_IMPL: Address = xip0001::ACCOUNT_IMPL;
/// `Predeploys.PERMIT2_EXPIRING`.
pub const PERMIT2_EXPIRING: Address = address!("0x0000000000000000000000000000000000001008");
/// `Predeploys.WXDS` (wrapped native coin, WETH9-style `Deposit` / `Withdrawal` events).
pub const WXDS: Address = address!("0x0000000000000000000000000000000000001009");
/// First and last predeploy address (`0x…1000` Staking .. `0x…1009` WXDS).
pub const PREDEPLOY_RANGE: (Address, Address) = (
    address!("0x0000000000000000000000000000000000001000"),
    address!("0x0000000000000000000000000000000000001009"),
);
/// ERC-1967 implementation slot (`keccak256("eip1967.proxy.implementation") - 1`): factory
/// accounts are ERC-1967 proxies of [`SMART_ACCOUNT_IMPL`].
pub const ERC1967_IMPLEMENTATION_SLOT: B256 =
    b256!("0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc");
/// Approvals of at least `2^160 - 1` (Permit2's `type(uint160).max`, or any larger ERC-20
/// amount such as `type(uint256).max`) are reported as `"unlimited"` (same rule as the
/// reference wallet).
pub const UNLIMITED_THRESHOLD: U256 = U256::from_limbs([u64::MAX, u64::MAX, 0xffff_ffff, 0]);
/// `AccountConstants.VAULT_EXECUTION_WINDOW`: a queued action can be executed during
/// `[executeAfter, executeAfter + window]`, after that it is expired.
pub const VAULT_EXECUTION_WINDOW: u64 = 14 * 86_400;
/// `AccountCore.VOTE_CANCEL`: guardian vote kind of `guardianVotes(kind, id)` for a cancel.
pub const VOTE_CANCEL: u8 = 2;
/// XNS top-level suffix.
pub const XNS_SUFFIX: &str = ".xdevs";

/// `true` when `amount` is reported as an unlimited approval ([`UNLIMITED_THRESHOLD`]).
pub fn is_unlimited(amount: U256) -> bool {
    amount >= UNLIMITED_THRESHOLD
}

/// Ethereum precompiles `0x01..=0x11` and the P-256 verifier `0x100`.
pub fn is_precompile(a: Address) -> bool {
    let v = U256::from_be_slice(a.as_slice());
    (U256::from(1)..=U256::from(0x11)).contains(&v) || v == U256::from(0x100)
}

/// xDEVS predeploys (`0x…1000..=0x…1009`) and the protocol system caller.
pub fn is_system_address(a: Address) -> bool {
    (PREDEPLOY_RANGE.0..=PREDEPLOY_RANGE.1).contains(&a) || a == crate::constants::SYSTEM_ADDRESS
}

/// Target of an EIP-7702 delegation designator (`0xef0100 ‖ address`), if `code` is one.
pub fn delegation_target(code: &[u8]) -> Option<Address> {
    match code {
        [0xef, 0x01, 0x00, rest @ ..] if rest.len() == 20 => Some(Address::from_slice(rest)),
        _ => None,
    }
}

/// Address poisoning heuristic (plan §16.3, same rule as `isLookAlikeAddress(a, b, 4)` of
/// `@xdevs/sdk`): different addresses sharing the first and last 4 hex digits.
pub fn is_lookalike(a: Address, b: Address) -> bool {
    a != b && a.as_slice()[..2] == b.as_slice()[..2] && a.as_slice()[18..] == b.as_slice()[18..]
}

/// `raw` scaled by `10^decimals` as a plain decimal string (`"12.5"`, `"0.000001"`, `"3"`).
pub fn format_units(raw: U256, decimals: u8) -> String {
    if decimals == 0 || decimals > 77 {
        return raw.to_string();
    }
    let unit = U256::from(10).pow(U256::from(decimals));
    let (int, frac) = (raw / unit, raw % unit);
    if frac.is_zero() {
        return int.to_string();
    }
    let frac = format!(
        "{:0>width$}",
        frac.to_string(),
        width = usize::from(decimals)
    );
    format!("{int}.{}", frac.trim_end_matches('0'))
}

// ---------------------------------------------------------------------------
// ABI helpers (static words only; enough for events and simple views)
// ---------------------------------------------------------------------------

/// Minimal ABI encoding / decoding used by the safety RPCs (no dynamic encoding; decoders
/// are bounds-checked and return `None` on malformed data).
pub mod abi {
    use super::*;

    /// 4-byte function selector of a canonical signature (`"allowance(address,address)"`).
    pub fn selector(signature: &str) -> [u8; 4] {
        let h = keccak256(signature.as_bytes());
        [h[0], h[1], h[2], h[3]]
    }

    /// Calldata `selector(signature) ‖ args` (static 32-byte arguments only).
    pub fn encode_call(signature: &str, args: &[B256]) -> Bytes {
        let mut out = Vec::with_capacity(4 + 32 * args.len());
        out.extend_from_slice(&selector(signature));
        for a in args {
            out.extend_from_slice(a.as_slice());
        }
        out.into()
    }

    /// Left-padded address word.
    pub fn address_word(a: Address) -> B256 {
        a.into_word()
    }

    /// Big-endian `uint256` word.
    pub fn u256_word(v: U256) -> B256 {
        B256::from(v)
    }

    /// Word at byte offset `offset`.
    pub fn word_at(data: &[u8], offset: usize) -> Option<U256> {
        let w = data.get(offset..offset.checked_add(32)?)?;
        Some(U256::from_be_slice(w))
    }

    /// `index`-th head word.
    pub fn word(data: &[u8], index: usize) -> Option<U256> {
        word_at(data, index.checked_mul(32)?)
    }

    /// `index`-th head word as an address (the low 20 bytes).
    pub fn address(data: &[u8], index: usize) -> Option<Address> {
        word(data, index).map(|w| Address::from_word(B256::from(w)))
    }

    /// `index`-th head word as a `bool` / small integer (`None` if it does not fit `u64`).
    pub fn u64_word(data: &[u8], index: usize) -> Option<u64> {
        word(data, index).and_then(|w| u64::try_from(w).ok())
    }

    /// Word at `offset` as an in-bounds length / offset.
    pub fn usize_at(data: &[u8], offset: usize) -> Option<usize> {
        let v = usize::try_from(word_at(data, offset)?).ok()?;
        (v <= data.len()).then_some(v)
    }

    /// Dynamic `bytes` whose offset is the `index`-th head word (offsets are relative to
    /// `data`).
    pub fn bytes(data: &[u8], index: usize) -> Option<&[u8]> {
        bytes_at(data, usize_at(data, index.checked_mul(32)?)?)
    }

    /// Dynamic `bytes` / `string` encoded at byte `offset` (length word, then the bytes).
    pub fn bytes_at(data: &[u8], offset: usize) -> Option<&[u8]> {
        let len = usize_at(data, offset)?;
        let start = offset.checked_add(32)?;
        data.get(start..start.checked_add(len)?)
    }

    /// Return data of `symbol()` / `name()`: an ABI `string`, or a legacy `bytes32`
    /// (trailing zeros trimmed). Control characters are dropped and the result is
    /// limited to 32 characters (the value is chosen by the token contract).
    pub fn string_or_bytes32(data: &[u8]) -> Option<String> {
        let raw: &[u8] = if data.len() == 32 {
            let end = data.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
            &data[..end]
        } else {
            bytes(data, 0)?
        };
        let s: String = String::from_utf8_lossy(raw)
            .chars()
            .filter(|c| !c.is_control())
            .take(32)
            .collect();
        let s = s.trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Event topics (`keccak256` of the canonical signatures) the safety layer decodes.
#[derive(Clone, Debug)]
pub struct Topics {
    /// ERC-20 / ERC-721 `Transfer(address,address,uint256)`.
    pub transfer: B256,
    /// ERC-20 / ERC-721 `Approval(address,address,uint256)`.
    pub approval: B256,
    /// ERC-721 / ERC-1155 `ApprovalForAll(address,address,bool)`.
    pub approval_for_all: B256,
    /// ERC-1155 `TransferSingle(address,address,address,uint256,uint256)`.
    pub transfer_single: B256,
    /// ERC-1155 `TransferBatch(address,address,address,uint256[],uint256[])`.
    pub transfer_batch: B256,
    /// XDRC-20 `ApprovalWithExpiry(address,address,uint256,uint64)`.
    pub approval_with_expiry: B256,
    /// XDRC-20 `AllApprovalsRevoked(address,uint32)`.
    pub all_approvals_revoked: B256,
    /// Permit2Expiring `Approval(address,address,address,uint160,uint48)`.
    pub permit2_approval: B256,
    /// Permit2Expiring `Permit(address,address,address,uint160,uint48,uint48)`.
    pub permit2_permit: B256,
    /// Permit2Expiring `Lockdown(address,address,address)`.
    pub permit2_lockdown: B256,
    /// Permit2Expiring `AllRevoked(address,uint48)`.
    pub permit2_all_revoked: B256,
    /// WXDS `Deposit(address,uint256)`.
    pub deposit: B256,
    /// WXDS `Withdrawal(address,uint256)`.
    pub withdrawal: B256,
    /// Ownable `OwnershipTransferred(address,address)`.
    pub ownership_transferred: B256,
    /// Guardian Vault `VaultActionQueued(uint256,address,uint256,bytes,uint64)`.
    pub vault_queued: B256,
    /// Guardian Vault `VaultActionCancelled(uint256,address)`.
    pub vault_cancelled: B256,
    /// Guardian Vault `VaultActionExecuted(uint256)`.
    pub vault_executed: B256,
    /// Smart account `VaultEpochBumped(uint32)` (every queued action is invalidated).
    pub vault_epoch_bumped: B256,
    /// Smart account `AccountFrozen(uint64)`.
    pub account_frozen: B256,
    /// Smart account `AccountUnfrozen()`.
    pub account_unfrozen: B256,
}

/// The [`Topics`] (computed once).
pub static TOPICS: LazyLock<Topics> = LazyLock::new(|| {
    let t = |s: &str| keccak256(s.as_bytes());
    Topics {
        transfer: t("Transfer(address,address,uint256)"),
        approval: t("Approval(address,address,uint256)"),
        approval_for_all: t("ApprovalForAll(address,address,bool)"),
        transfer_single: t("TransferSingle(address,address,address,uint256,uint256)"),
        transfer_batch: t("TransferBatch(address,address,address,uint256[],uint256[])"),
        approval_with_expiry: t("ApprovalWithExpiry(address,address,uint256,uint64)"),
        all_approvals_revoked: t("AllApprovalsRevoked(address,uint32)"),
        permit2_approval: t("Approval(address,address,address,uint160,uint48)"),
        permit2_permit: t("Permit(address,address,address,uint160,uint48,uint48)"),
        permit2_lockdown: t("Lockdown(address,address,address)"),
        permit2_all_revoked: t("AllRevoked(address,uint48)"),
        deposit: t("Deposit(address,uint256)"),
        withdrawal: t("Withdrawal(address,uint256)"),
        ownership_transferred: t("OwnershipTransferred(address,address)"),
        vault_queued: t("VaultActionQueued(uint256,address,uint256,bytes,uint64)"),
        vault_cancelled: t("VaultActionCancelled(uint256,address)"),
        vault_executed: t("VaultActionExecuted(uint256)"),
        vault_epoch_bumped: t("VaultEpochBumped(uint32)"),
        account_frozen: t("AccountFrozen(uint64)"),
        account_unfrozen: t("AccountUnfrozen()"),
    }
});

// ---------------------------------------------------------------------------
// Warnings
// ---------------------------------------------------------------------------

/// Warning / flag codes (open-ended: clients must accept unknown codes).
pub mod codes {
    /// The transaction would revert or halt.
    pub const REVERT: &str = "REVERT";
    /// Approval of at least `2^160 - 1` units.
    pub const UNLIMITED_APPROVAL: &str = "UNLIMITED_APPROVAL";
    /// Non-zero ERC-20 approval without an expiry.
    pub const NEVER_EXPIRING_APPROVAL: &str = "NEVER_EXPIRING_APPROVAL";
    /// `setApprovalForAll(operator, true)` (all NFTs of a collection).
    pub const SET_APPROVAL_FOR_ALL: &str = "SET_APPROVAL_FOR_ALL";
    /// `OwnershipTransferred` of a contract the sender owned.
    pub const OWNERSHIP_TRANSFER: &str = "OWNERSHIP_TRANSFER";
    /// The transaction deploys contracts.
    pub const CONTRACT_CREATION: &str = "CONTRACT_CREATION";
    /// The sender sends more than half of its native balance.
    pub const HIGH_VALUE_TRANSFER: &str = "HIGH_VALUE_TRANSFER";
    /// A contract self-destructs and sends its balance away.
    pub const SELF_DESTRUCT: &str = "SELF_DESTRUCT";
    /// The Guardian Vault queues the action (cancel window).
    pub const VAULT_QUEUED: &str = "VAULT_QUEUED";
    /// Assets are sent to the zero address (burned).
    pub const BURN: &str = "BURN";
    /// More events than the simulator records.
    pub const TRACE_TRUNCATED: &str = "TRACE_TRUNCATED";
    /// The sender cannot pay `gas * maxFee + value`.
    pub const INSUFFICIENT_FUNDS: &str = "INSUFFICIENT_FUNDS";
    /// `from` has not sent tokens to the recipient within the scanned history.
    pub const FIRST_TIME_RECIPIENT: &str = "FIRST_TIME_RECIPIENT";
    /// The recipient looks like a past recipient (address poisoning).
    pub const LOOKALIKE_RECIPIENT: &str = "LOOKALIKE_RECIPIENT";
    /// The recipient sent `from` a zero-value token transfer (address poisoning pattern).
    pub const POISONING_PATTERN: &str = "POISONING_PATTERN";
    /// The recipient is the zero address.
    pub const ZERO_ADDRESS: &str = "ZERO_ADDRESS";
    /// The recipient is a precompile.
    pub const PRECOMPILE: &str = "PRECOMPILE";
    /// The recipient is a protocol system contract / address.
    pub const SYSTEM_CONTRACT: &str = "SYSTEM_CONTRACT";
    /// The recipient is a contract (not a smart account).
    pub const CONTRACT_RECIPIENT: &str = "CONTRACT_RECIPIENT";
    /// The recipient has never been used on chain (nonce 0, no balance, no code).
    pub const NEW_ADDRESS: &str = "NEW_ADDRESS";
    /// The recipient's primary XNS name differs from the name that was entered.
    pub const NAME_MISMATCH: &str = "NAME_MISMATCH";
}

/// Severity of a [`Warning`] (ordered: `info < low < medium < high`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational.
    Info,
    /// Low risk.
    Low,
    /// Medium risk.
    Medium,
    /// High risk.
    High,
}

/// An advisory warning (`{code, severity, message}`). **Advice, not fact.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// Machine-readable code (see [`codes`]).
    pub code: String,
    /// Severity.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
}

impl Warning {
    /// New warning.
    pub fn new(code: &str, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity,
            message: message.into(),
        }
    }
}

/// Sort warnings by severity (highest first), keeping the order of equal ones.
pub fn sort_warnings(warnings: &mut [Warning]) {
    warnings.sort_by(|a, b| b.severity.cmp(&a.severity));
}

// ---------------------------------------------------------------------------
// xdevs_simulateTransaction
// ---------------------------------------------------------------------------

/// `"native"` or a token contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetToken {
    /// The native coin (XDS / tXDS).
    Native,
    /// A token contract.
    Contract(Address),
}

impl Serialize for AssetToken {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AssetToken::Native => s.serialize_str("native"),
            AssetToken::Contract(a) => a.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for AssetToken {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.eq_ignore_ascii_case("native") {
            return Ok(AssetToken::Native);
        }
        s.parse::<Address>()
            .map(AssetToken::Contract)
            .map_err(serde::de::Error::custom)
    }
}

/// Token standard of an [`AssetChange`] (`"native"`, `"erc20"`, `"erc721"`, `"erc1155"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetStandard {
    /// Native coin.
    Native,
    /// ERC-20 / XDRC-20 (also WXDS deposits / withdrawals).
    Erc20,
    /// ERC-721 (one token id).
    Erc721,
    /// ERC-1155.
    Erc1155,
}

/// One asset movement, in execution order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetChange {
    /// `"native"` or the token contract.
    pub token: AssetToken,
    /// Token standard.
    pub standard: AssetStandard,
    /// Symbol (self-reported by the token; `null` if unknown).
    pub symbol: Option<String>,
    /// Decimals (self-reported; `null` if unknown, then `amount` is the raw amount).
    pub decimals: Option<u8>,
    /// Sender (`0x0` for a mint).
    pub from: Address,
    /// Recipient (`0x0` for a burn).
    pub to: Address,
    /// Human-readable amount scaled by `decimals` (`"12.5"`).
    pub amount: String,
    /// Raw amount (base units).
    pub raw_amount: U256,
    /// NFT token id (ERC-721 / ERC-1155).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<U256>,
}

/// Net balance change of one address in one asset over the whole transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceChange {
    /// Account.
    pub address: Address,
    /// `"native"` or the token contract.
    pub token: AssetToken,
    /// Symbol (self-reported; `null` if unknown).
    pub symbol: Option<String>,
    /// Signed, scaled amount (`"-12.5"`, `"3"`).
    pub delta: String,
    /// Signed raw amount in base units (decimal string, `"-12500000"`).
    pub raw_delta: String,
    /// NFT token id (ERC-721 / ERC-1155).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<U256>,
}

/// Kind of an approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ApprovalKind {
    /// ERC-20 `approve` (no expiry).
    #[serde(rename = "erc20")]
    Erc20,
    /// XDRC-20 approval with expiry.
    #[serde(rename = "xdrc20")]
    Xdrc20,
    /// Permit2Expiring allowance.
    #[serde(rename = "permit2")]
    Permit2,
    /// ERC-721 approval of one token id.
    #[serde(rename = "erc721")]
    Erc721,
    /// ERC-721 operator approval (`setApprovalForAll`).
    #[serde(rename = "erc721-all")]
    Erc721All,
    /// ERC-1155 operator approval (`setApprovalForAll`).
    #[serde(rename = "erc1155-all")]
    Erc1155All,
}

impl ApprovalKind {
    /// `true` for operator (`*-all`) approvals.
    pub const fn is_operator(self) -> bool {
        matches!(self, ApprovalKind::Erc721All | ApprovalKind::Erc1155All)
    }
}

/// An approval set or changed by a simulated transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalChange {
    /// Kind.
    pub kind: ApprovalKind,
    /// Token contract.
    pub token: Address,
    /// Symbol (self-reported; `null` if unknown).
    pub symbol: Option<String>,
    /// Owner.
    pub owner: Address,
    /// Spender / operator.
    pub spender: Address,
    /// `"unlimited"`, `"all"` (operator approved), `"0"` (revoked) or the scaled amount.
    pub amount: String,
    /// Raw amount (`1` / `0` for operator approvals).
    pub raw_amount: U256,
    /// Unlimited (see [`UNLIMITED_THRESHOLD`]) or operator approval.
    pub unlimited: bool,
    /// Unix seconds when it expires, `null` = never.
    pub expires: Option<u64>,
    /// ERC-721 token id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<U256>,
}

/// A contract deployed by the simulated transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractCreation {
    /// New contract address.
    pub address: Address,
    /// Deployer.
    pub creator: Address,
}

/// What the Guardian Vault did with the simulated transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultOutcome {
    /// The smart account.
    pub account: Address,
    /// `true` = queued with a cancel window, `false` = executed instantly.
    pub queued: bool,
    /// Queued action id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<U256>,
    /// Unix seconds from which the queued action can be executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_at: Option<u64>,
    /// Queued actions this transaction executes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executed: Vec<U256>,
    /// Queued actions this transaction cancels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancelled: Vec<U256>,
}

/// Result of `xdevs_simulateTransaction` (plan §16.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationReport {
    /// The transaction would succeed.
    pub success: bool,
    /// Gas used.
    pub gas_used: U64,
    /// Decoded revert reason / halt reason when `success` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_reason: Option<String>,
    /// Raw revert data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_data: Option<Bytes>,
    /// Asset movements in execution order (native, ERC-20/XDRC-20, ERC-721, ERC-1155).
    pub asset_changes: Vec<AssetChange>,
    /// Net change per address and asset.
    pub balance_changes: Vec<BalanceChange>,
    /// Approvals set or changed.
    pub approvals: Vec<ApprovalChange>,
    /// Contracts created.
    pub contracts_created: Vec<ContractCreation>,
    /// Guardian Vault outcome when the transaction goes through a smart account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<VaultOutcome>,
    /// Advisory warnings, highest severity first.
    pub warnings: Vec<Warning>,
    /// Always [`DISCLAIMER`].
    pub disclaimer: String,
    /// Block whose state the simulation ran on.
    pub block_number: U64,
}

// ---------------------------------------------------------------------------
// xdevs_checkRecipient
// ---------------------------------------------------------------------------

/// On-chain facts about a recipient (at the requested block).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientFacts {
    /// Resolved address.
    pub address: Address,
    /// The XNS name that was entered (normalized), if any.
    pub resolved_from: Option<String>,
    /// `0x0000…0000`.
    pub is_zero_address: bool,
    /// Precompile (`0x01..=0x11`, `0x100`).
    pub is_precompile: bool,
    /// Predeploy (`0x…1000..=0x…1009`) or the system caller.
    pub is_system_contract: bool,
    /// Has code (contract or EIP-7702 delegated account).
    pub is_contract: bool,
    /// Code size in bytes.
    pub code_size: u64,
    /// xDEVS Smart Account (EIP-7702 delegation to, or ERC-1967 proxy of, `0x…1005`).
    pub is_smart_account: bool,
    /// EIP-7702 delegation target.
    pub delegated_to: Option<Address>,
    /// XIP-0001 lock status (`null` when the node does not serve it).
    pub eoa_lock: Option<EoaLockStatus>,
    /// Nonce.
    pub nonce: u64,
    /// Native balance (wei).
    pub balance: U256,
    /// Forward-verified primary XNS name.
    pub primary_name: Option<String>,
}

/// Block range of a bounded history scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryScan {
    /// First block scanned.
    pub from_block: u64,
    /// Last block scanned.
    pub to_block: u64,
    /// `false` if the scan hit the result limit and only covered part of the window.
    pub complete: bool,
}

/// Advisory part of `xdevs_checkRecipient`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientAdvice {
    /// `from` has not sent tokens to the recipient within [`RecipientAdvice::history`]
    /// (`null` without `from`).
    pub first_time: Option<bool>,
    /// Past recipients of `from` that look like the recipient.
    pub look_alike_of: Vec<Address>,
    /// Advisory flags, highest severity first.
    pub flags: Vec<Warning>,
    /// Scanned block range (token `Transfer` logs of `from`).
    pub history: Option<HistoryScan>,
    /// Always [`DISCLAIMER`].
    pub disclaimer: String,
}

/// Result of `xdevs_checkRecipient` (plan §16.3). `facts` are on-chain data, `advice` is
/// opinion; the top-level `firstTime` / `lookAlikeOf` / `name` / `isContract` / `warnings`
/// are the summary older clients read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientCheck {
    /// Resolved recipient address.
    pub address: Address,
    /// Same as `advice.firstTime` (`false` without `from`).
    pub first_time: bool,
    /// Same as `advice.lookAlikeOf`.
    pub look_alike_of: Vec<Address>,
    /// Same as `facts.primaryName`.
    pub name: Option<String>,
    /// Same as `facts.isContract`.
    pub is_contract: bool,
    /// Same as `advice.flags`.
    pub warnings: Vec<Warning>,
    /// Facts.
    pub facts: RecipientFacts,
    /// Advice.
    pub advice: RecipientAdvice,
    /// Block of the facts.
    pub block_number: U64,
}

// ---------------------------------------------------------------------------
// xdevs_getApprovals
// ---------------------------------------------------------------------------

/// A live approval of an owner (`xdevs_getApprovals`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    /// Kind.
    pub kind: ApprovalKind,
    /// Token contract.
    pub token: Address,
    /// Symbol (self-reported; `null` if unknown).
    pub symbol: Option<String>,
    /// Owner.
    pub owner: Address,
    /// Spender / operator.
    pub spender: Address,
    /// Raw remaining amount as a decimal integer string, `"unlimited"`, or `"all"` for
    /// operator approvals.
    pub amount: String,
    /// Raw remaining amount (`1` for operator approvals).
    pub raw_amount: U256,
    /// Unlimited or operator approval.
    pub unlimited: bool,
    /// Unix seconds when it expires, `null` = never.
    pub expires: Option<u64>,
    /// Block of the last approval event seen for this pair.
    pub last_updated_block: U64,
    /// Transaction of that event.
    pub transaction_hash: Option<B256>,
}

// ---------------------------------------------------------------------------
// xdevs_getVaultQueue / vaultEvents
// ---------------------------------------------------------------------------

/// One call of a queued action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultCall {
    /// Target.
    pub target: Address,
    /// Native value (wei).
    pub value: U256,
    /// Calldata.
    pub data: Bytes,
}

/// Status of a pending queued action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultActionStatus {
    /// In the cancel window (`now < executeAfter`).
    Queued,
    /// Executable by anyone (`executeAfter <= now <= executeAfter + window`).
    Ready,
}

/// A pending Guardian Vault action (`xdevs_getVaultQueue`); `target` / `value` / `data`
/// mirror the `VaultActionQueued` event (a batch is reported as the equivalent
/// `executeBatch` self-call; `calls` lists every call).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultQueueItem {
    /// Smart account.
    pub account: Address,
    /// Action id.
    pub id: U256,
    /// Target (the account itself for a batch).
    pub target: Address,
    /// Native value (batch: total).
    pub value: U256,
    /// Calldata (batch: `executeBatch((address,uint256,bytes)[])`).
    pub data: Bytes,
    /// Unix seconds from which anyone may `execute(id)`.
    pub execute_after: U64,
    /// Unix seconds after which the action can no longer be executed.
    pub expires_at: U64,
    /// `queued` or `ready`.
    pub status: VaultActionStatus,
    /// Calls of the action.
    pub calls: Vec<VaultCall>,
    /// Guardian cancel votes so far.
    pub cancel_votes: u8,
}

/// Kind of a [`VaultEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultEventKind {
    /// `VaultActionQueued`.
    Queued,
    /// `VaultActionCancelled`.
    Cancelled,
    /// `VaultActionExecuted`.
    Executed,
    /// `VaultEpochBumped`: every action queued before is invalidated (social recovery).
    Invalidated,
    /// `AccountFrozen`.
    Frozen,
    /// `AccountUnfrozen`.
    Unfrozen,
}

/// A Guardian Vault event (`eth_subscribe("vaultEvents")` notification).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEvent {
    /// Kind.
    #[serde(rename = "type")]
    pub kind: VaultEventKind,
    /// Emitting smart account.
    pub account: Address,
    /// Action id (`queued` / `cancelled` / `executed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<U256>,
    /// Target (`queued`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Address>,
    /// Value (`queued`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<U256>,
    /// Calldata (`queued`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Bytes>,
    /// Unix seconds from which the action can be executed (`queued`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execute_after: Option<U64>,
    /// Who cancelled (`cancelled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Address>,
    /// Frozen until (`frozen`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_until: Option<U64>,
    /// New vault epoch (`invalidated`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u32>,
    /// Block number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_number: Option<U64>,
    /// Block hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<B256>,
    /// Transaction hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<B256>,
    /// Block-wide log index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_index: Option<U64>,
}

/// Decode a Guardian Vault / freeze event (block metadata left empty). `None` for any
/// other log or a malformed one. The emitter is the account: a subscriber filtering by
/// account cannot be spoofed by another contract.
pub fn decode_vault_event(log: &Log) -> Option<VaultEvent> {
    let t = &*TOPICS;
    let topics = log.topics();
    let data = log.data.data.as_ref();
    let id = || topics.get(1).map(|w| U256::from_be_bytes(w.0));
    let mut ev = VaultEvent {
        kind: VaultEventKind::Queued,
        account: log.address,
        id: None,
        target: None,
        value: None,
        data: None,
        execute_after: None,
        by: None,
        frozen_until: None,
        epoch: None,
        block_number: None,
        block_hash: None,
        transaction_hash: None,
        log_index: None,
    };
    let topic0 = *topics.first()?;
    if topic0 == t.vault_queued && topics.len() == 2 {
        ev.id = id();
        ev.target = Some(abi::address(data, 0)?);
        ev.value = Some(abi::word(data, 1)?);
        ev.data = Some(Bytes::copy_from_slice(abi::bytes(data, 2)?));
        ev.execute_after = Some(U64::from(abi::u64_word(data, 3)?));
    } else if topic0 == t.vault_cancelled && topics.len() == 2 {
        ev.kind = VaultEventKind::Cancelled;
        ev.id = id();
        ev.by = Some(abi::address(data, 0)?);
    } else if topic0 == t.vault_executed && topics.len() == 2 {
        ev.kind = VaultEventKind::Executed;
        ev.id = id();
    } else if topic0 == t.vault_epoch_bumped && topics.len() == 1 {
        ev.kind = VaultEventKind::Invalidated;
        ev.epoch = Some(u32::try_from(abi::word(data, 0)?).ok()?);
    } else if topic0 == t.account_frozen && topics.len() == 1 {
        ev.kind = VaultEventKind::Frozen;
        ev.frozen_until = Some(U64::from(abi::u64_word(data, 0)?));
    } else if topic0 == t.account_unfrozen && topics.len() == 1 {
        ev.kind = VaultEventKind::Unfrozen;
    } else {
        return None;
    }
    Some(ev)
}

// ---------------------------------------------------------------------------
// XNS names
// ---------------------------------------------------------------------------

/// Normalize an XNS name like `normalizeXnsName` of `@xdevs/sdk`: ASCII only, trimmed,
/// lowercased, `.xdevs` appended when there is no dot, every label valid under
/// `XNSLabel.check` (`[a-z0-9-]`, 3..=63 bytes, no leading / trailing / double hyphen).
pub fn normalize_xns_name(input: &str) -> Result<String, String> {
    if !input.is_ascii() {
        return Err("non-ASCII characters are not allowed".into());
    }
    let mut name = input.trim().to_ascii_lowercase();
    if !name.contains('.') {
        name.push_str(XNS_SUFFIX);
    }
    let Some(labels) = name.strip_suffix(XNS_SUFFIX) else {
        return Err(format!("name must end with {XNS_SUFFIX:?}"));
    };
    for label in labels.split('.') {
        check_xns_label(label).map_err(|e| format!("label {label:?}: {e}"))?;
    }
    Ok(name)
}

/// `XNSLabel.check`: `Ok` iff `label` is registrable.
pub fn check_xns_label(label: &str) -> Result<(), &'static str> {
    let b = label.as_bytes();
    if b.len() < 3 {
        return Err("too short (min 3)");
    }
    if b.len() > 63 {
        return Err("too long (max 63)");
    }
    for (i, c) in b.iter().enumerate() {
        if *c == b'-' {
            if i == 0 || i == b.len() - 1 || b[i - 1] == b'-' {
                return Err("bad hyphen");
            }
        } else if !c.is_ascii_lowercase() && !c.is_ascii_digit() {
            return Err("forbidden character");
        }
    }
    Ok(())
}

/// ENS / EIP-137 namehash (no normalization; see [`normalize_xns_name`]).
pub fn xns_namehash(name: &str) -> B256 {
    let mut node = B256::ZERO;
    if name.is_empty() {
        return node;
    }
    for label in name.rsplit('.') {
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(node.as_slice());
        buf[32..].copy_from_slice(keccak256(label.as_bytes()).as_slice());
        node = keccak256(buf);
    }
    node
}

/// Confusable skeleton of a label (`XNSLabel.skeleton` before hashing): greedily
/// `0→o`, `1→l`, `rn→m`, `vv→w`, `cl→d`.
pub fn xns_skeleton(label: &str) -> String {
    let b = label.as_bytes();
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let (c, n) = (b[i], b.get(i + 1).copied());
        let mapped = match (c, n) {
            (b'0', _) => 'o',
            (b'1', _) => 'l',
            (b'r', Some(b'n')) => {
                i += 1;
                'm'
            }
            (b'v', Some(b'v')) => {
                i += 1;
                'w'
            }
            (b'c', Some(b'l')) => {
                i += 1;
                'd'
            }
            (c, _) => char::from(c),
        };
        out.push(mapped);
        i += 1;
    }
    out
}

/// `XNSLabel.skeleton(label)`: hash of the confusable skeleton (`XNSRegistry.skeletonHolder`
/// key).
pub fn xns_skeleton_hash(label: &str) -> B256 {
    keccak256(xns_skeleton(label).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hex, LogData};

    #[test]
    fn well_known_topics() {
        let t = &*TOPICS;
        assert_eq!(
            t.transfer,
            b256!("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef")
        );
        assert_eq!(
            t.approval,
            b256!("0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925")
        );
        assert_eq!(
            t.approval_for_all,
            b256!("0x17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31")
        );
        assert_eq!(
            abi::selector("transfer(address,uint256)"),
            [0xa9, 0x05, 0x9c, 0xbb]
        );
        assert_eq!(
            abi::selector("approve(address,uint256)"),
            [0x09, 0x5e, 0xa7, 0xb3]
        );
    }

    #[test]
    fn units_and_thresholds() {
        assert_eq!(format_units(U256::from(12_500_000u64), 6), "12.5");
        assert_eq!(format_units(U256::from(1u64), 6), "0.000001");
        assert_eq!(format_units(U256::from(3_000_000u64), 6), "3");
        assert_eq!(format_units(U256::from(42u64), 0), "42");
        assert_eq!(format_units(U256::from(10u64).pow(U256::from(18)), 18), "1");
        assert!(is_unlimited(U256::MAX));
        assert!(is_unlimited((U256::from(1) << 160) - U256::from(1)));
        assert!(!is_unlimited((U256::from(1) << 160) - U256::from(2)));
    }

    #[test]
    fn address_classes() {
        let a = address!("0x1234000000000000000000000000000000005678");
        let b = address!("0x1234ffffffffffffffffffffffffffffffff5678");
        let c = address!("0x1235000000000000000000000000000000005678");
        assert!(is_lookalike(a, b));
        assert!(!is_lookalike(a, a));
        assert!(!is_lookalike(a, c));
        assert!(is_precompile(address!(
            "0x0000000000000000000000000000000000000001"
        )));
        assert!(is_precompile(address!(
            "0x0000000000000000000000000000000000000100"
        )));
        assert!(!is_precompile(Address::ZERO));
        assert!(is_system_address(WXDS));
        assert!(is_system_address(crate::constants::SYSTEM_ADDRESS));
        assert!(!is_system_address(a));
        let mut code = vec![0xef, 0x01, 0x00];
        code.extend_from_slice(SMART_ACCOUNT_IMPL.as_slice());
        assert_eq!(delegation_target(&code), Some(SMART_ACCOUNT_IMPL));
        assert_eq!(delegation_target(&code[..22]), None);
    }

    #[test]
    fn xns_names() {
        // `XNSRegistry.TLD_NODE`.
        assert_eq!(
            xns_namehash("xdevs"),
            b256!("0xa058e3bd8a205b95bca4815c553cac26b3228e39fce9d34f8417f3665c626edd")
        );
        assert_eq!(normalize_xns_name(" Alice ").unwrap(), "alice.xdevs");
        assert_eq!(
            normalize_xns_name("pay.alice.xdevs").unwrap(),
            "pay.alice.xdevs"
        );
        assert!(normalize_xns_name("al").is_err());
        assert!(normalize_xns_name("-alice").is_err());
        assert!(normalize_xns_name("a--b").is_err());
        assert!(normalize_xns_name("alice.eth").is_err());
        assert!(normalize_xns_name("аlice").is_err()); // Cyrillic a
        assert_eq!(xns_skeleton("paypa1"), "paypal");
        assert_eq!(xns_skeleton("g00gle"), "google");
        assert_eq!(xns_skeleton("rnvvcl"), "mwd");
        assert_eq!(xns_skeleton_hash("paypa1"), xns_skeleton_hash("paypal"));
    }

    #[test]
    fn vault_event_decoding() {
        let account = Address::repeat_byte(0xac);
        let target = Address::repeat_byte(0x7a);
        let mut data = Vec::new();
        data.extend_from_slice(abi::address_word(target).as_slice());
        data.extend_from_slice(abi::u256_word(U256::from(5)).as_slice());
        data.extend_from_slice(abi::u256_word(U256::from(128)).as_slice());
        data.extend_from_slice(abi::u256_word(U256::from(1_700_000_000u64)).as_slice());
        data.extend_from_slice(abi::u256_word(U256::from(2)).as_slice());
        let mut tail = [0u8; 32];
        tail[..2].copy_from_slice(&[0xbe, 0xef]);
        data.extend_from_slice(&tail);
        let log = Log {
            address: account,
            data: LogData::new_unchecked(
                vec![TOPICS.vault_queued, B256::from(U256::from(7))],
                data.into(),
            ),
        };
        let ev = decode_vault_event(&log).unwrap();
        assert_eq!(ev.kind, VaultEventKind::Queued);
        assert_eq!(ev.account, account);
        assert_eq!(ev.id, Some(U256::from(7)));
        assert_eq!(ev.target, Some(target));
        assert_eq!(ev.value, Some(U256::from(5)));
        assert_eq!(ev.data, Some(Bytes::from(hex::decode("beef").unwrap())));
        assert_eq!(ev.execute_after, Some(U64::from(1_700_000_000u64)));
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "queued");
        assert_eq!(json["executeAfter"], "0x6553f100");
        assert!(json.get("by").is_none());

        let frozen = Log {
            address: account,
            data: LogData::new_unchecked(
                vec![TOPICS.account_frozen],
                abi::u256_word(U256::from(99)).0.to_vec().into(),
            ),
        };
        assert_eq!(
            decode_vault_event(&frozen).unwrap().frozen_until,
            Some(U64::from(99))
        );
        // Truncated data: rejected, not panicking.
        let bad = Log {
            address: account,
            data: LogData::new_unchecked(
                vec![TOPICS.vault_queued, B256::ZERO],
                Bytes::from_static(&[1, 2, 3]),
            ),
        };
        assert!(decode_vault_event(&bad).is_none());
    }

    #[test]
    fn abi_strings() {
        let mut data = Vec::new();
        data.extend_from_slice(abi::u256_word(U256::from(32)).as_slice());
        data.extend_from_slice(abi::u256_word(U256::from(4)).as_slice());
        let mut s = [0u8; 32];
        s[..4].copy_from_slice(b"USDC");
        data.extend_from_slice(&s);
        assert_eq!(abi::string_or_bytes32(&data).as_deref(), Some("USDC"));
        let mut b32 = [0u8; 32];
        b32[..3].copy_from_slice(b"MKR");
        assert_eq!(abi::string_or_bytes32(&b32).as_deref(), Some("MKR"));
        assert_eq!(abi::string_or_bytes32(&[]), None);
        // Offset pointing outside the data.
        let mut bad = abi::u256_word(U256::from(1_000)).0.to_vec();
        bad.extend_from_slice(&[0u8; 32]);
        assert_eq!(abi::string_or_bytes32(&bad), None);
    }

    #[test]
    fn asset_token_serde() {
        let n = serde_json::to_value(AssetToken::Native).unwrap();
        assert_eq!(n, "native");
        let a = AssetToken::Contract(WXDS);
        let v = serde_json::to_value(a).unwrap();
        assert_eq!(serde_json::from_value::<AssetToken>(v).unwrap(), a);
        assert_eq!(
            serde_json::to_value(ApprovalKind::Erc1155All).unwrap(),
            "erc1155-all"
        );
        assert_eq!(serde_json::to_value(Severity::High).unwrap(), "high");
        assert!(Severity::High > Severity::Medium && Severity::Low > Severity::Info);
    }
}
