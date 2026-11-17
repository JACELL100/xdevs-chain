//! Transaction simulation for xDEVS Chain (plan §16.4, anti blind-signing).
//!
//! [`simulate`] runs a transaction request as a message call against a block's state with
//! revm (through [`EvmExecutor::inspect_call`], so the RPC gas cap and the EVM time limit /
//! cancellation of `eth_call` apply) and a [`SafetyInspector`] that records, frame by
//! frame, what survives: native value moves, logs, contract creations and self-destructs.
//! [`analyze`] then derives
//!
//! * asset changes (native, ERC-20 / XDRC-20, WXDS wrap / unwrap, ERC-721, ERC-1155) in
//!   execution order, plus the net balance change per address and asset;
//! * approvals set or changed (ERC-20, XDRC-20 with expiry, Permit2Expiring, ERC-721,
//!   operator approvals), flagging unlimited and never-expiring ones;
//! * contracts created;
//! * the Guardian Vault outcome (queued with its release time, or instant);
//! * advisory warnings with a severity.
//!
//! Nothing is ever committed: the simulation reads the state through a write buffer that
//! is dropped. Results are deterministic for a given state and request. **Warnings are
//! advice, not fact** (principle 7): every report carries
//! [`xdevs_primitives::safety::DISCLAIMER`].
//!
//! Fees are not simulated: `gasPrice` / `maxFeePerGas` are cleared so the result shows
//! the transaction's own effects; a sender that could not pay `gas * maxFee + value` gets
//! an `INSUFFICIENT_FUNDS` warning instead.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod analyze;
pub mod inspector;
#[cfg(test)]
mod tests;

pub use analyze::{analyze, report, Analysis, ReportContext, TokenInfo};
pub use inspector::{SafetyInspector, TraceEvent, MAX_TRACE_EVENTS};

use alloy_rpc_types_eth::{TransactionInput, TransactionRequest};
use std::collections::BTreeMap;
use xdevs_execution::{CallLimits, CallResult, EvmExecutor, ExecutionError};
use xdevs_primitives::{
    reexports::alloy_primitives::U64,
    safety::{
        abi, codes, format_units, sort_warnings, Severity, SimulationReport, Warning, DISCLAIMER,
    },
    xip0001::DESIGNATOR_HASH,
    Address, Bytes, Header, TxKind, U256,
};
use xdevs_state::StateProvider;

/// Gas limit of each metadata view call (`symbol()`, `decimals()`, `supportsInterface`).
pub const METADATA_CALL_GAS: u64 = 100_000;
/// Token contracts whose metadata is looked up per simulation (others get `null`).
pub const MAX_METADATA_TOKENS: usize = 32;

/// Simulation settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimulationOptions {
    /// Gas cap (RPC `gas_cap`; 0 = unlimited).
    pub gas_cap: u64,
    /// Native coin symbol shown in asset changes (`tXDS` on test networks).
    pub native_symbol: String,
}

impl Default for SimulationOptions {
    fn default() -> Self {
        Self {
            gas_cap: 50_000_000,
            native_symbol: "XDS".to_string(),
        }
    }
}

/// Decode an ABI `Error(string)` / `Panic(uint256)` revert payload.
pub fn decode_revert(data: &[u8]) -> Option<String> {
    if let Some(body) = data.strip_prefix(&[0x08, 0xc3, 0x79, 0xa0]) {
        let s = abi::bytes(body, 0)?;
        return Some(String::from_utf8_lossy(s).into_owned());
    }
    if let Some(body) = data.strip_prefix(&[0x4e, 0x48, 0x7b, 0x71]) {
        return Some(format!("panic code {:#x}", abi::word(body, 0)?));
    }
    None
}

/// Simulate `request` (with `from` set) on `state`, the state after the block `header`.
/// Errors are those of `eth_call` (invalid request, aborted by `limits`, state failure);
/// a revert or halt is a successful simulation with `success: false`.
pub fn simulate(
    executor: &EvmExecutor,
    state: &dyn StateProvider,
    header: &Header,
    request: &TransactionRequest,
    options: &SimulationOptions,
    limits: &CallLimits,
) -> Result<SimulationReport, ExecutionError> {
    let from = request
        .from
        .ok_or_else(|| ExecutionError::InvalidRequest("`from` is required".into()))?;
    let to = request.to.and_then(|kind| kind.to().copied());
    let fee_per_gas = request
        .max_fee_per_gas
        .or(request.gas_price)
        .unwrap_or_default();
    let mut req = request.clone();
    req.gas_price = None;
    req.max_fee_per_gas = None;
    req.max_priority_fee_per_gas = None;

    let state_err = |e: xdevs_state::StateError| ExecutionError::State(e.to_string());
    let sender_balance = state
        .basic_account(from)
        .map_err(state_err)?
        .map(|a| a.balance)
        .unwrap_or_default();

    let mut inspector = SafetyInspector::new();
    let result =
        executor.inspect_call(state, header, &req, options.gas_cap, limits, &mut inspector)?;
    let mut warnings = Vec::new();
    let (success, revert_reason, revert_data) = match &result {
        CallResult::Success { .. } => (true, None, None),
        CallResult::Revert { output, .. } => (
            false,
            Some(decode_revert(output).unwrap_or_else(|| "execution reverted".to_string())),
            Some(output.clone()),
        ),
        CallResult::Halt { reason, .. } => (false, Some(reason.clone()), None),
    };
    if let Some(reason) = &revert_reason {
        warnings.push(Warning::new(
            codes::REVERT,
            Severity::High,
            format!("The transaction would fail ({reason}); nothing would change except the fee."),
        ));
    }
    if fee_per_gas > 0 {
        let gas = U256::from(req.gas.unwrap_or_else(|| result.gas_used()));
        let needed = gas
            .saturating_mul(U256::from(fee_per_gas))
            .saturating_add(request.value.unwrap_or_default());
        if needed > sender_balance {
            warnings.push(Warning::new(
                codes::INSUFFICIENT_FUNDS,
                Severity::High,
                format!(
                    "The sender cannot pay gas * maxFee + value ({} {} needed).",
                    format_units(needed, 18),
                    options.native_symbol
                ),
            ));
        }
    }
    if inspector.truncated() {
        warnings.push(Warning::new(
            codes::TRACE_TRUNCATED,
            Severity::Medium,
            format!("The transaction does more than {MAX_TRACE_EVENTS} things; later effects are not shown."),
        ));
    }

    let to_is_smart_account = match to {
        Some(t) => {
            inspector.smart_accounts().contains(&t)
                || state
                    .basic_account(t)
                    .map_err(state_err)?
                    .is_some_and(|a| a.code_hash == DESIGNATOR_HASH)
        }
        None => false,
    };
    let analysis = if success {
        analyze(inspector.events())
    } else {
        Analysis::default()
    };
    let infos = token_infos(executor, state, header, &analysis, options, limits)?;
    let parts = report(
        &analysis,
        &ReportContext {
            from,
            to,
            sender_balance,
            native_symbol: &options.native_symbol,
            to_is_smart_account,
        },
        &infos,
    );
    warnings.extend(parts.warnings);
    sort_warnings(&mut warnings);
    Ok(SimulationReport {
        success,
        gas_used: U64::from(result.gas_used()),
        revert_reason,
        revert_data,
        asset_changes: parts.asset_changes,
        balance_changes: parts.balance_changes,
        approvals: parts.approvals,
        contracts_created: parts.contracts_created,
        vault: parts.vault,
        warnings,
        disclaimer: DISCLAIMER.to_string(),
        block_number: U64::from(header.number),
    })
}

/// `symbol()` / `decimals()` (and `supportsInterface(ERC-1155)` for operator approvals) of
/// the tokens in `analysis`, read from the pre-transaction state. A view that reverts,
/// halts or returns garbage yields `None`; an aborted one (time limit) aborts the
/// simulation.
fn token_infos(
    executor: &EvmExecutor,
    state: &dyn StateProvider,
    header: &Header,
    analysis: &Analysis,
    options: &SimulationOptions,
    limits: &CallLimits,
) -> Result<BTreeMap<Address, TokenInfo>, ExecutionError> {
    let view = |token: Address, data: Bytes| -> Result<Option<Bytes>, ExecutionError> {
        let req = TransactionRequest {
            to: Some(TxKind::Call(token)),
            input: TransactionInput::new(data),
            gas: Some(METADATA_CALL_GAS),
            ..Default::default()
        };
        match executor.call_with_limits(state, header, &req, options.gas_cap, limits) {
            Ok(CallResult::Success { output, .. }) => Ok(Some(output)),
            Ok(_) | Err(ExecutionError::InvalidRequest(_)) => Ok(None),
            Err(e) => Err(e),
        }
    };
    let operators: Vec<Address> = analysis
        .approvals
        .iter()
        .filter(|a| a.kind.is_operator())
        .map(|a| a.token)
        .collect();
    let mut infos = BTreeMap::new();
    for token in analysis.tokens().into_iter().take(MAX_METADATA_TOKENS) {
        let symbol = view(token, abi::encode_call("symbol()", &[]))?
            .and_then(|out| abi::string_or_bytes32(&out));
        let decimals = view(token, abi::encode_call("decimals()", &[]))?
            .and_then(|out| abi::u64_word(&out, 0))
            .and_then(|d| u8::try_from(d).ok())
            .filter(|d| *d <= 77);
        let erc1155 = if operators.contains(&token) {
            let mut id = [0u8; 32];
            id[..4].copy_from_slice(&[0xd9, 0xb6, 0x7a, 0x26]);
            view(
                token,
                abi::encode_call("supportsInterface(bytes4)", &[id.into()]),
            )?
            .and_then(|out| abi::u64_word(&out, 0))
                == Some(1)
        } else {
            false
        };
        infos.insert(
            token,
            TokenInfo {
                symbol,
                decimals,
                erc1155,
            },
        );
    }
    Ok(infos)
}
