//! Turn the surviving [`TraceEvent`]s of a simulation into asset changes, approvals, the
//! Guardian Vault outcome and advisory warnings. Pure functions (no EVM access): token
//! metadata is looked up by the caller and passed in as [`TokenInfo`]s.

use crate::inspector::TraceEvent;
use std::collections::BTreeMap;
use xdevs_primitives::{
    safety::{
        abi, codes, decode_vault_event, format_units, is_unlimited, ApprovalChange, ApprovalKind,
        AssetChange, AssetStandard, AssetToken, BalanceChange, ContractCreation, Severity,
        VaultEventKind, VaultOutcome, Warning, PERMIT2_EXPIRING, TOPICS, WXDS,
    },
    Address, Log, U256,
};

/// Max ids decoded from one ERC-1155 `TransferBatch`.
pub const MAX_BATCH_ITEMS: usize = 256;

/// Self-reported metadata of a token contract.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenInfo {
    /// `symbol()`.
    pub symbol: Option<String>,
    /// `decimals()`.
    pub decimals: Option<u8>,
    /// `supportsInterface(0xd9b67a26)` (ERC-1155).
    pub erc1155: bool,
}

/// A movement before metadata is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTransfer {
    /// Asset.
    pub token: AssetToken,
    /// Standard.
    pub standard: AssetStandard,
    /// Sender.
    pub from: Address,
    /// Recipient.
    pub to: Address,
    /// Raw amount.
    pub amount: U256,
    /// NFT id.
    pub token_id: Option<U256>,
}

/// An approval before metadata is applied. Operator approvals use
/// [`ApprovalKind::Erc721All`] until the token is known to be ERC-1155.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawApproval {
    /// Kind.
    pub kind: ApprovalKind,
    /// Token.
    pub token: Address,
    /// Owner.
    pub owner: Address,
    /// Spender / operator.
    pub spender: Address,
    /// Raw amount (operator: 1 = approved, 0 = revoked).
    pub amount: U256,
    /// Expiry (unix seconds).
    pub expires: Option<u64>,
    /// ERC-721 token id.
    pub token_id: Option<U256>,
}

/// Everything decoded from a trace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Movements in execution order.
    pub transfers: Vec<RawTransfer>,
    /// Approvals (last write per token / owner / spender / id), in first-seen order.
    pub approvals: Vec<RawApproval>,
    /// Created contracts.
    pub created: Vec<ContractCreation>,
    /// `(contract, beneficiary, value)` of self-destructs.
    pub self_destructs: Vec<(Address, Address, U256)>,
    /// `(contract, previous owner, new owner)`.
    pub ownership: Vec<(Address, Address, Address)>,
    /// First `VaultActionQueued`: `(account, id, executeAfter)`.
    pub queued: Option<(Address, U256, u64)>,
    /// `VaultActionExecuted`: `(account, id)`.
    pub executed: Vec<(Address, U256)>,
    /// `VaultActionCancelled`: `(account, id)`.
    pub cancelled: Vec<(Address, U256)>,
}

impl Analysis {
    /// Token contracts whose metadata the report needs (first-seen order, no duplicates).
    pub fn tokens(&self) -> Vec<Address> {
        let mut out: Vec<Address> = Vec::new();
        let transfers = self.transfers.iter().filter_map(|t| match t.token {
            AssetToken::Contract(a) => Some(a),
            AssetToken::Native => None,
        });
        for a in transfers.chain(self.approvals.iter().map(|a| a.token)) {
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }

    fn approve(&mut self, approval: RawApproval) {
        let same = |a: &RawApproval| {
            a.token == approval.token
                && a.owner == approval.owner
                && a.spender == approval.spender
                && a.token_id == approval.token_id
                && a.kind.is_operator() == approval.kind.is_operator()
                && (a.kind == ApprovalKind::Permit2) == (approval.kind == ApprovalKind::Permit2)
        };
        match self.approvals.iter_mut().find(|a| same(a)) {
            Some(existing) => *existing = approval,
            None => self.approvals.push(approval),
        }
    }
}

fn topic_address(log: &Log, i: usize) -> Option<Address> {
    log.topics().get(i).map(|t| Address::from_word(*t))
}

fn topic_u256(log: &Log, i: usize) -> Option<U256> {
    log.topics().get(i).map(|t| U256::from_be_bytes(t.0))
}

/// Decode a dynamic `uint256[]` whose offset is head word `index`.
fn u256_array(data: &[u8], index: usize) -> Option<Vec<U256>> {
    let offset = abi::usize_at(data, index.checked_mul(32)?)?;
    let len = abi::usize_at(data, offset)?.min(MAX_BATCH_ITEMS);
    (0..len)
        .map(|i| abi::word_at(data, offset + 32 + i * 32))
        .collect()
}

/// Decode the surviving events of a simulation.
pub fn analyze(events: &[TraceEvent]) -> Analysis {
    let mut a = Analysis::default();
    for event in events {
        match event {
            TraceEvent::Native { from, to, value } => a.transfers.push(RawTransfer {
                token: AssetToken::Native,
                standard: AssetStandard::Native,
                from: *from,
                to: *to,
                amount: *value,
                token_id: None,
            }),
            TraceEvent::SelfDestruct {
                contract,
                beneficiary,
                value,
            } => {
                a.self_destructs.push((*contract, *beneficiary, *value));
                a.transfers.push(RawTransfer {
                    token: AssetToken::Native,
                    standard: AssetStandard::Native,
                    from: *contract,
                    to: *beneficiary,
                    amount: *value,
                    token_id: None,
                });
            }
            TraceEvent::Created { address, creator } => a.created.push(ContractCreation {
                address: *address,
                creator: *creator,
            }),
            TraceEvent::Log(log) => decode_log(&mut a, log),
        }
    }
    a
}

fn decode_log(a: &mut Analysis, log: &Log) {
    let t = &*TOPICS;
    let Some(topic0) = log.topics().first().copied() else {
        return;
    };
    let n = log.topics().len();
    let data = log.data.data.as_ref();
    let token = log.address;
    // Topic / data shapes are checked so that e.g. an ERC-721 `Approval` (4 topics) is
    // never read as an ERC-20 one (3 topics).
    if topic0 == t.transfer {
        let (Some(from), Some(to)) = (topic_address(log, 1), topic_address(log, 2)) else {
            return;
        };
        if n == 3 {
            if let Some(amount) = abi::word(data, 0) {
                a.transfers.push(RawTransfer {
                    token: AssetToken::Contract(token),
                    standard: AssetStandard::Erc20,
                    from,
                    to,
                    amount,
                    token_id: None,
                });
            }
        } else if n == 4 {
            a.transfers.push(RawTransfer {
                token: AssetToken::Contract(token),
                standard: AssetStandard::Erc721,
                from,
                to,
                amount: U256::from(1),
                token_id: topic_u256(log, 3),
            });
        }
    } else if topic0 == t.transfer_single && n == 4 {
        let (Some(from), Some(to), Some(id), Some(amount)) = (
            topic_address(log, 2),
            topic_address(log, 3),
            abi::word(data, 0),
            abi::word(data, 1),
        ) else {
            return;
        };
        a.transfers.push(RawTransfer {
            token: AssetToken::Contract(token),
            standard: AssetStandard::Erc1155,
            from,
            to,
            amount,
            token_id: Some(id),
        });
    } else if topic0 == t.transfer_batch && n == 4 {
        let (Some(from), Some(to), Some(ids), Some(amounts)) = (
            topic_address(log, 2),
            topic_address(log, 3),
            u256_array(data, 0),
            u256_array(data, 1),
        ) else {
            return;
        };
        for (id, amount) in ids.into_iter().zip(amounts) {
            a.transfers.push(RawTransfer {
                token: AssetToken::Contract(token),
                standard: AssetStandard::Erc1155,
                from,
                to,
                amount,
                token_id: Some(id),
            });
        }
    } else if (topic0 == t.deposit || topic0 == t.withdrawal) && token == WXDS && n == 2 {
        // WETH9-style wrapper: mint / burn of WXDS (the native leg is a call value).
        let (Some(who), Some(amount)) = (topic_address(log, 1), abi::word(data, 0)) else {
            return;
        };
        let (from, to) = if topic0 == t.deposit {
            (Address::ZERO, who)
        } else {
            (who, Address::ZERO)
        };
        a.transfers.push(RawTransfer {
            token: AssetToken::Contract(token),
            standard: AssetStandard::Erc20,
            from,
            to,
            amount,
            token_id: None,
        });
    } else if topic0 == t.approval {
        let (Some(owner), Some(spender)) = (topic_address(log, 1), topic_address(log, 2)) else {
            return;
        };
        if n == 3 {
            if let Some(amount) = abi::word(data, 0) {
                // An XDRC-20 also emits `ApprovalWithExpiry` (after `Approval`), which
                // upgrades this entry.
                let expires = a
                    .approvals
                    .iter()
                    .find(|x| {
                        x.kind == ApprovalKind::Xdrc20
                            && x.token == token
                            && x.owner == owner
                            && x.spender == spender
                    })
                    .map(|x| (x.kind, x.expires));
                let (kind, expires) = expires.unwrap_or((ApprovalKind::Erc20, None));
                a.approve(RawApproval {
                    kind,
                    token,
                    owner,
                    spender,
                    amount,
                    expires,
                    token_id: None,
                });
            }
        } else if n == 4 {
            a.approve(RawApproval {
                kind: ApprovalKind::Erc721,
                token,
                owner,
                spender,
                amount: U256::from(u8::from(spender != Address::ZERO)),
                expires: None,
                token_id: topic_u256(log, 3),
            });
        }
    } else if topic0 == t.approval_with_expiry && n == 3 {
        let (Some(owner), Some(spender), Some(amount), Some(expiry)) = (
            topic_address(log, 1),
            topic_address(log, 2),
            abi::word(data, 0),
            abi::u64_word(data, 1),
        ) else {
            return;
        };
        a.approve(RawApproval {
            kind: ApprovalKind::Xdrc20,
            token,
            owner,
            spender,
            amount,
            expires: (expiry != 0).then_some(expiry),
            token_id: None,
        });
    } else if topic0 == t.approval_for_all && n == 3 {
        let (Some(owner), Some(operator), Some(approved)) = (
            topic_address(log, 1),
            topic_address(log, 2),
            abi::word(data, 0),
        ) else {
            return;
        };
        a.approve(RawApproval {
            kind: ApprovalKind::Erc721All,
            token,
            owner,
            spender: operator,
            amount: U256::from(u8::from(!approved.is_zero())),
            expires: None,
            token_id: None,
        });
    } else if (topic0 == t.permit2_approval || topic0 == t.permit2_permit)
        && token == PERMIT2_EXPIRING
        && n == 4
    {
        let (Some(owner), Some(erc20), Some(spender), Some(amount), Some(expiration)) = (
            topic_address(log, 1),
            topic_address(log, 2),
            topic_address(log, 3),
            abi::word(data, 0),
            abi::u64_word(data, 1),
        ) else {
            return;
        };
        a.approve(RawApproval {
            kind: ApprovalKind::Permit2,
            token: erc20,
            owner,
            spender,
            amount,
            expires: (expiration != 0).then_some(expiration),
            token_id: None,
        });
    } else if topic0 == t.ownership_transferred && n == 3 {
        if let (Some(prev), Some(new)) = (topic_address(log, 1), topic_address(log, 2)) {
            a.ownership.push((token, prev, new));
        }
    } else if let Some(ev) = decode_vault_event(log) {
        match ev.kind {
            VaultEventKind::Queued => {
                if a.queued.is_none() {
                    if let (Some(id), Some(after)) = (ev.id, ev.execute_after) {
                        a.queued = Some((ev.account, id, after.to::<u64>()));
                    }
                }
            }
            VaultEventKind::Executed => a.executed.extend(ev.id.map(|id| (ev.account, id))),
            VaultEventKind::Cancelled => a.cancelled.extend(ev.id.map(|id| (ev.account, id))),
            _ => {}
        }
    }
}

/// Inputs of [`report`] that do not come from the trace.
#[derive(Clone, Debug)]
pub struct ReportContext<'a> {
    /// Transaction sender.
    pub from: Address,
    /// Transaction target (`None` = contract creation).
    pub to: Option<Address>,
    /// Sender's native balance before the transaction.
    pub sender_balance: U256,
    /// Native coin symbol (`XDS` / `tXDS`).
    pub native_symbol: &'a str,
    /// `to` is an xDEVS Smart Account (Guardian Vault).
    pub to_is_smart_account: bool,
}

/// Report parts produced from an [`Analysis`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportParts {
    /// Asset changes.
    pub asset_changes: Vec<AssetChange>,
    /// Net balance changes.
    pub balance_changes: Vec<BalanceChange>,
    /// Approvals.
    pub approvals: Vec<ApprovalChange>,
    /// Created contracts.
    pub contracts_created: Vec<ContractCreation>,
    /// Vault outcome.
    pub vault: Option<VaultOutcome>,
    /// Warnings (unsorted).
    pub warnings: Vec<Warning>,
}

fn short(a: Address) -> String {
    let s = a.to_checksum(None);
    format!("{}…{}", &s[..6], &s[s.len() - 4..])
}

/// `YYYY-MM-DD HH:MM UTC` of unix seconds (proleptic Gregorian calendar).
pub fn format_utc(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX / 2);
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3_600,
        (rem % 3_600) / 60
    )
}

fn signed(pos: U256, neg: U256, decimals: Option<u8>) -> (String, String) {
    let (sign, abs) = if pos >= neg {
        ("", pos - neg)
    } else {
        ("-", neg - pos)
    };
    let scaled = decimals.map_or_else(|| abs.to_string(), |d| format_units(abs, d));
    (format!("{sign}{scaled}"), format!("{sign}{abs}"))
}

/// Build asset changes, net balances, approvals, the vault outcome and the warnings.
pub fn report(
    a: &Analysis,
    ctx: &ReportContext<'_>,
    infos: &BTreeMap<Address, TokenInfo>,
) -> ReportParts {
    let mut out = ReportParts::default();
    let meta = |token: AssetToken, standard: AssetStandard| -> (Option<String>, Option<u8>) {
        match (token, standard) {
            (AssetToken::Native, _) => (Some(ctx.native_symbol.to_string()), Some(18)),
            (AssetToken::Contract(t), AssetStandard::Erc20) => {
                let info = infos.get(&t);
                (
                    info.and_then(|i| i.symbol.clone()),
                    info.and_then(|i| i.decimals),
                )
            }
            (AssetToken::Contract(t), _) => (infos.get(&t).and_then(|i| i.symbol.clone()), None),
        }
    };

    // Asset changes + net balances.
    let mut net: BTreeMap<(Address, AssetToken, Option<U256>), (U256, U256)> = BTreeMap::new();
    for t in &a.transfers {
        let (symbol, decimals) = meta(t.token, t.standard);
        let amount = match (t.standard, decimals) {
            (AssetStandard::Erc721, _) => "1".to_string(),
            (_, Some(d)) => format_units(t.amount, d),
            (_, None) => t.amount.to_string(),
        };
        out.asset_changes.push(AssetChange {
            token: t.token,
            standard: t.standard,
            symbol,
            decimals,
            from: t.from,
            to: t.to,
            amount,
            raw_amount: t.amount,
            token_id: t.token_id,
        });
        if t.from != t.to {
            for (who, credit) in [(t.from, false), (t.to, true)] {
                if who == Address::ZERO {
                    continue;
                }
                let e = net.entry((who, t.token, t.token_id)).or_default();
                if credit {
                    e.0 = e.0.saturating_add(t.amount);
                } else {
                    e.1 = e.1.saturating_add(t.amount);
                }
            }
        }
    }
    for ((address, token, token_id), (pos, neg)) in net {
        if pos == neg {
            continue;
        }
        let standard = match (token, token_id) {
            (AssetToken::Native, _) => AssetStandard::Native,
            (_, None) => AssetStandard::Erc20,
            (_, Some(_)) => AssetStandard::Erc1155,
        };
        let (symbol, decimals) = meta(token, standard);
        let (delta, raw_delta) = signed(pos, neg, decimals);
        out.balance_changes.push(BalanceChange {
            address,
            token,
            symbol,
            delta,
            raw_delta,
            token_id,
        });
    }

    // Approvals.
    for r in &a.approvals {
        let info = infos.get(&r.token);
        let symbol = info.and_then(|i| i.symbol.clone());
        let kind = match r.kind {
            ApprovalKind::Erc721All if info.is_some_and(|i| i.erc1155) => ApprovalKind::Erc1155All,
            k => k,
        };
        let token_label = symbol.clone().unwrap_or_else(|| short(r.token));
        let (amount, unlimited) = if kind.is_operator() {
            let approved = !r.amount.is_zero();
            (if approved { "all" } else { "0" }.to_string(), approved)
        } else if kind == ApprovalKind::Erc721 {
            (r.amount.to_string(), false)
        } else if is_unlimited(r.amount) {
            ("unlimited".to_string(), true)
        } else {
            let decimals = info.and_then(|i| i.decimals);
            (
                decimals.map_or_else(|| r.amount.to_string(), |d| format_units(r.amount, d)),
                false,
            )
        };
        let spender = short(r.spender);
        if kind.is_operator() && unlimited {
            out.warnings.push(Warning::new(
                codes::SET_APPROVAL_FOR_ALL,
                Severity::High,
                format!(
                    "This lets {spender} move ALL your {token_label} NFTs, now and in the future."
                ),
            ));
        } else if unlimited {
            match r.expires {
                None => out.warnings.push(Warning::new(
                    codes::UNLIMITED_APPROVAL,
                    Severity::High,
                    format!("This gives {spender} permission to spend ALL your {token_label}, with no expiry."),
                )),
                Some(e) => out.warnings.push(Warning::new(
                    codes::UNLIMITED_APPROVAL,
                    Severity::Medium,
                    format!(
                        "This gives {spender} permission to spend ALL your {token_label} until {}.",
                        format_utc(e)
                    ),
                )),
            }
        } else if matches!(kind, ApprovalKind::Erc20) && !r.amount.is_zero() {
            out.warnings.push(Warning::new(
                codes::NEVER_EXPIRING_APPROVAL,
                Severity::Low,
                format!("This lets {spender} spend {amount} {token_label} of yours until you revoke it (no expiry)."),
            ));
        }
        out.approvals.push(ApprovalChange {
            kind,
            token: r.token,
            symbol,
            owner: r.owner,
            spender: r.spender,
            amount,
            raw_amount: r.amount,
            unlimited,
            expires: r.expires,
            token_id: r.token_id,
        });
    }

    out.contracts_created = a.created.clone();
    if !a.created.is_empty() {
        out.warnings.push(Warning::new(
            codes::CONTRACT_CREATION,
            Severity::Info,
            format!("This transaction deploys {} contract(s).", a.created.len()),
        ));
    }
    for (contract, beneficiary, value) in &a.self_destructs {
        out.warnings.push(Warning::new(
            codes::SELF_DESTRUCT,
            Severity::Medium,
            format!(
                "Contract {} self-destructs and sends {} {} to {}.",
                short(*contract),
                format_units(*value, 18),
                ctx.native_symbol,
                short(*beneficiary)
            ),
        ));
    }
    for (contract, prev, new) in &a.ownership {
        if *prev == Address::ZERO {
            continue; // a fresh deployment setting its first owner
        }
        let severity = if *prev == ctx.from {
            Severity::High
        } else {
            Severity::Medium
        };
        out.warnings.push(Warning::new(
            codes::OWNERSHIP_TRANSFER,
            severity,
            format!(
                "Ownership of contract {} moves from {} to {}.",
                short(*contract),
                short(*prev),
                short(*new)
            ),
        ));
    }

    // Native value leaving the sender vs. its balance.
    let sent: U256 = a
        .transfers
        .iter()
        .filter(|t| t.token == AssetToken::Native && t.from == ctx.from && t.to != ctx.from)
        .fold(U256::ZERO, |acc, t| acc.saturating_add(t.amount));
    if !sent.is_zero() && sent > ctx.sender_balance / U256::from(2) {
        out.warnings.push(Warning::new(
            codes::HIGH_VALUE_TRANSFER,
            Severity::Medium,
            format!(
                "This sends {} {} — more than half of your balance.",
                format_units(sent, 18),
                ctx.native_symbol
            ),
        ));
    }
    if a.transfers.iter().any(|t| {
        t.to == Address::ZERO && t.from == ctx.from && t.token != AssetToken::Contract(WXDS)
    }) {
        out.warnings.push(Warning::new(
            codes::BURN,
            Severity::Low,
            "This sends assets to the zero address (they are burned).",
        ));
    }

    // Guardian Vault.
    if let Some((account, id, release_at)) = a.queued {
        out.vault = Some(VaultOutcome {
            account,
            queued: true,
            action_id: Some(id),
            release_at: Some(release_at),
            executed: ids_of(&a.executed, account),
            cancelled: ids_of(&a.cancelled, account),
        });
        out.warnings.push(Warning::new(
            codes::VAULT_QUEUED,
            Severity::Info,
            format!(
                "Your Guardian Vault queues this action (#{id}) until {}: nothing moves now, and it can be cancelled before then.",
                format_utc(release_at)
            ),
        ));
    } else if let Some(account) = ctx.to.filter(|_| ctx.to_is_smart_account) {
        out.vault = Some(VaultOutcome {
            account,
            queued: false,
            action_id: None,
            release_at: None,
            executed: ids_of(&a.executed, account),
            cancelled: ids_of(&a.cancelled, account),
        });
    }
    out
}

fn ids_of(list: &[(Address, U256)], account: Address) -> Vec<U256> {
    list.iter()
        .filter(|(a, _)| *a == account)
        .map(|(_, id)| *id)
        .collect()
}
