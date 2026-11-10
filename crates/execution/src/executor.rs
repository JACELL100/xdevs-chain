//! [`EvmExecutor`]: block execution, block building, `eth_call` and gas estimation.

use crate::{
    db::{StateDbError, StateOverlay},
    env::{block_env, call_tx_env, cfg_env, tx_env, BLOB_TX_TYPE},
    next_base_fee, xip0001, BlockAttributes, BlockExecutor, BuiltBlock, CallLimits, CallResult,
    EvmExecutor, ExecutionError, ExecutionOutcome,
};
use alloy_consensus::Eip658Value;
use alloy_rpc_types_eth::TransactionRequest;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    context_interface::{
        result::{EVMError, ExecutionResult, HaltReason, InvalidTransaction},
        ContextTr,
    },
    handler::{MainnetContext, MainnetEvm},
    inspector::{InspectEvm, Inspector},
    interpreter::{interpreter::EthInterpreter, Interpreter},
    primitives::eip7825::TX_GAS_LIMIT_CAP,
    Context, ExecuteEvm, MainBuilder, MainContext,
};
use std::collections::BTreeSet;
use xdevs_primitives::{
    compute_logs_bloom, compute_receipts_root, compute_transactions_root,
    constants::MAX_EXTRA_DATA_SIZE,
    set_compat_header_fields,
    xip0001::{check_shape, check_tx, TxShape, Xip0001Error, Xip0001Violation, MAX_GAS},
    Address, Block, EoaLockStatus, Header, Log, Receipt, ReceiptEnvelope, RecoveredTx, Transaction,
    TxKind, Typed2718, U256,
};
use xdevs_state::{StateDiff, StateProvider};

/// Gas of the cheapest possible transaction; below this nothing else fits in a block.
const MIN_TX_GAS: u64 = 21_000;
/// `CALL` stipend, used by the optimistic first probe of `estimate_gas` (as in geth).
const CALL_STIPEND: u64 = 2_300;

type Evm<'o, 'a> = MainnetEvm<MainnetContext<&'o mut StateOverlay<'a>>>;

fn new_evm<'o, 'a>(db: &'o mut StateOverlay<'a>, cfg: CfgEnv, block: BlockEnv) -> Evm<'o, 'a> {
    Context::mainnet()
        .with_db(db)
        .with_block(block)
        .with_cfg(cfg)
        .build_mainnet()
}

/// EVM instructions between two [`CallLimits`] checks (each check reads the clock).
pub const CALL_LIMIT_CHECK_INTERVAL: u32 = 4096;

/// Inspector that halts a simulation once its [`CallLimits`] trip: every
/// [`CALL_LIMIT_CHECK_INTERVAL`] instructions it checks the limits; once tripped, every
/// further instruction (in any frame) halts at once, so the whole call unwinds.
struct Interrupt<'l> {
    limits: &'l CallLimits,
    steps: u32,
    tripped: Option<&'static str>,
}

impl<CTX> Inspector<CTX, EthInterpreter> for Interrupt<'_> {
    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        if self.tripped.is_none() {
            self.steps = self.steps.wrapping_add(1);
            if !self.steps.is_multiple_of(CALL_LIMIT_CHECK_INTERVAL) {
                return;
            }
            self.tripped = self.limits.tripped();
            if self.tripped.is_none() {
                return;
            }
        }
        interp.halt_oog();
    }
}

/// Why a single transaction could not be applied.
enum TxFailure {
    /// Invalid against the current state / block (revm validation).
    Invalid(InvalidTransaction),
    /// Invalid under XIP-0001 at its position in the block.
    Xip0001(Xip0001Error),
    /// State/EVM failure: aborts the whole block.
    Fatal(ExecutionError),
}

impl From<StateDbError> for TxFailure {
    fn from(e: StateDbError) -> Self {
        TxFailure::Fatal(ExecutionError::State(e.0))
    }
}

/// Result of one applied transaction.
struct AppliedTx {
    gas_used: u64,
    success: bool,
    logs: Vec<Log>,
}

/// Outcome of a message call before it is interpreted.
enum CallOutcome {
    Done(ExecutionResult<HaltReason>),
    Invalid(String),
}

fn evm_error<T>(e: EVMError<StateDbError, T>) -> ExecutionError
where
    T: core::fmt::Display,
{
    match e {
        EVMError::Database(e) => ExecutionError::State(e.0),
        other => ExecutionError::Evm(other.to_string()),
    }
}

impl EvmExecutor {
    /// Run `tx` on `evm`, commit its state changes and pay the treasury share of the
    /// base fee (§15.1). revm itself charges the sender, burns the base fee and pays the
    /// tip to the block beneficiary.
    ///
    /// `xip0001 = Some(senders)` when XIP-0001 is active in this block, with the senders
    /// of the transactions already in the block: rules (a) / (c) are checked and the
    /// authorization list is filtered (b), all against the current overlay = `S₀(tx)`.
    fn apply_tx(
        &self,
        evm: &mut Evm<'_, '_>,
        tx: &RecoveredTx,
        base_fee: u64,
        xip_senders: Option<&BTreeSet<Address>>,
    ) -> Result<AppliedTx, TxFailure> {
        let mut env = tx_env(tx).map_err(TxFailure::Fatal)?;
        if let Some(senders_in_block) = xip_senders {
            let db = evm.ctx.db_mut();
            let status = xip0001::status(db, tx.sender)?;
            check_tx(
                status,
                &TxShape::from(tx),
                base_fee,
                senders_in_block.contains(&tx.sender),
            )
            .map_err(TxFailure::Xip0001)?;
            xip0001::filter_authorizations(db, &mut env.authorization_list)?;
        }
        let out = match evm.transact(env) {
            Ok(out) => out,
            Err(EVMError::Transaction(e)) => return Err(TxFailure::Invalid(e)),
            Err(e) => return Err(TxFailure::Fatal(evm_error(e))),
        };
        let db = evm.ctx.db_mut();
        db.try_commit(out.state)?;

        let gas_used = out.result.tx_gas_used();
        let base_fee_total = U256::from(gas_used) * U256::from(base_fee);
        let (_burned, treasury_share) = self.chain_config().split_base_fee(base_fee_total);
        db.credit(self.chain_config().treasury, treasury_share)?;

        let success = out.result.is_success();
        Ok(AppliedTx {
            gas_used,
            success,
            logs: out.result.into_logs(),
        })
    }

    /// Roots and receipts of a finished block.
    fn finish(
        state: &dyn StateProvider,
        state_diff: StateDiff,
        receipts: Vec<ReceiptEnvelope>,
        gas_used: u64,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        Ok(ExecutionOutcome {
            state_root: state.state_root_with(&state_diff)?,
            receipts_root: compute_receipts_root(&receipts),
            logs_bloom: compute_logs_bloom(&receipts),
            gas_used,
            receipts,
            state_diff,
        })
    }

    /// Header checks of `header` against `parent` and the chain config.
    fn validate_child(&self, parent: &Header, header: &Header) -> Result<(), ExecutionError> {
        let err = |m: String| Err(ExecutionError::InvalidHeader(m));
        if header.number != parent.number + 1 {
            return err(format!(
                "number {} is not parent number {} + 1",
                header.number, parent.number
            ));
        }
        if header.parent_hash != parent.hash_slow() {
            return err("parent hash mismatch".into());
        }
        if header.timestamp <= parent.timestamp {
            return err(format!(
                "timestamp {} not after parent timestamp {}",
                header.timestamp, parent.timestamp
            ));
        }
        let expected = next_base_fee(parent, self.chain_config());
        if header.base_fee_per_gas != Some(expected) {
            return err(format!(
                "base fee {:?}, expected {expected}",
                header.base_fee_per_gas
            ));
        }
        if header.extra_data.len() > MAX_EXTRA_DATA_SIZE {
            return err("extra data longer than 32 bytes".into());
        }
        if header.gas_used > header.gas_limit {
            return err("gas used above gas limit".into());
        }
        Ok(())
    }

    /// Run a message call on `db` (nothing is committed) with relaxed checks like geth's
    /// `eth_call`: no nonce / base fee / block gas limit / EIP-3607 / EIP-7825 checks.
    /// Fails with [`ExecutionError::Aborted`] when `limits` trip.
    fn run_call(
        &self,
        db: &mut StateOverlay<'_>,
        header: &Header,
        request: &TransactionRequest,
        gas_limit: u64,
        limits: &CallLimits,
    ) -> Result<CallOutcome, ExecutionError> {
        if let Some(reason) = limits.tripped() {
            return Err(ExecutionError::Aborted(reason));
        }
        let from = request.from.unwrap_or_default();
        let nonce = db
            .account(from)
            .map_err(|e| ExecutionError::State(e.0))?
            .map_or(0, |a| a.nonce);
        let mut env: TxEnv = call_tx_env(request, self.chain_config().chain_id, gas_limit, nonce)?;
        // Simulations are never rejected by XIP-0001, but locked authorities are skipped
        // exactly as in a real transaction so results and gas match inclusion.
        if self.chain_config().xip0001_active(header.number + 1) {
            xip0001::filter_authorizations(db, &mut env.authorization_list)
                .map_err(|e| ExecutionError::State(e.0))?;
        }
        let mut cfg = cfg_env(self.chain_config().chain_id);
        cfg.disable_nonce_check = true;
        cfg.disable_base_fee = true;
        cfg.disable_block_gas_limit = true;
        cfg.disable_eip3607 = true;
        cfg.tx_gas_limit_cap = Some(u64::MAX);
        let out = if limits.is_unbounded() {
            new_evm(db, cfg, block_env(header)).transact(env)
        } else {
            let mut evm = Context::mainnet()
                .with_db(db)
                .with_block(block_env(header))
                .with_cfg(cfg)
                .build_mainnet_with_inspector(Interrupt {
                    limits,
                    steps: 0,
                    tripped: None,
                });
            let out = evm.inspect_tx(env);
            if let Some(reason) = evm.inspector.tripped {
                return Err(ExecutionError::Aborted(reason));
            }
            out
        };
        match out {
            Ok(out) => Ok(CallOutcome::Done(out.result)),
            Err(EVMError::Transaction(e)) => Ok(CallOutcome::Invalid(e.to_string())),
            Err(e) => Err(evm_error(e)),
        }
    }

    /// Execute `request` as a message call on `state` in the context of a block with
    /// `header` (block env: number, timestamp, beneficiary, base fee, gas limit, prevrandao).
    /// Nothing is committed. Balance / nonce / base-fee checks are relaxed as in geth's
    /// `eth_call` (missing `gasPrice` = 0, missing `gas` = `gas_cap`). `gas_cap` bounds the
    /// gas limit (RPC config `gas_cap`, default 50M; 0 = unlimited).
    pub fn call(
        &self,
        state: &dyn StateProvider,
        header: &Header,
        request: &TransactionRequest,
        gas_cap: u64,
    ) -> Result<CallResult, ExecutionError> {
        self.call_with_limits(state, header, request, gas_cap, &CallLimits::default())
    }

    /// [`EvmExecutor::call`] that stops with [`ExecutionError::Aborted`] once `limits`
    /// trip (RPC time limit / client gone).
    pub fn call_with_limits(
        &self,
        state: &dyn StateProvider,
        header: &Header,
        request: &TransactionRequest,
        gas_cap: u64,
        limits: &CallLimits,
    ) -> Result<CallResult, ExecutionError> {
        let cap = effective_cap(gas_cap);
        let gas_limit = request.gas.unwrap_or(cap).min(cap);
        let mut db = StateOverlay::new(state);
        match self.run_call(&mut db, header, request, gas_limit, limits)? {
            CallOutcome::Invalid(reason) => Err(ExecutionError::InvalidRequest(reason)),
            CallOutcome::Done(result) => Ok(to_call_result(result)),
        }
    }

    /// Smallest gas limit with which `request` succeeds (binary search over message
    /// calls, like geth). Errors with [`ExecutionError::Reverted`] /
    /// [`ExecutionError::GasCapExceeded`] if it fails even at the upper bound
    /// (`min(request.gas or block gas limit, gas_cap, EIP-7825 cap, balance allowance)`).
    pub fn estimate_gas(
        &self,
        state: &dyn StateProvider,
        header: &Header,
        request: &TransactionRequest,
        gas_cap: u64,
    ) -> Result<u64, ExecutionError> {
        self.estimate_gas_with_limits(state, header, request, gas_cap, &CallLimits::default())
    }

    /// [`EvmExecutor::estimate_gas`] whose whole binary search stops with
    /// [`ExecutionError::Aborted`] once `limits` trip.
    pub fn estimate_gas_with_limits(
        &self,
        state: &dyn StateProvider,
        header: &Header,
        request: &TransactionRequest,
        gas_cap: u64,
        limits: &CallLimits,
    ) -> Result<u64, ExecutionError> {
        let mut db = StateOverlay::new(state);
        let locked = self.check_request_xip0001(&mut db, header, request)?;
        let mut hi = request
            .gas
            .filter(|g| *g >= MIN_TX_GAS)
            .unwrap_or(header.gas_limit)
            .min(effective_cap(gas_cap))
            .min(TX_GAS_LIMIT_CAP);

        // With a fee set, the sender must be able to pay for `hi` gas (geth's allowance).
        let fee_per_gas = request
            .max_fee_per_gas
            .or(request.gas_price)
            .unwrap_or_default();
        if fee_per_gas > 0 {
            let from = request.from.unwrap_or_default();
            let balance = db
                .account(from)
                .map_err(|e| ExecutionError::State(e.0))?
                .map(|a| a.balance)
                .unwrap_or_default();
            let value = request.value.unwrap_or_default();
            if value > balance {
                return Err(ExecutionError::InvalidRequest(
                    "insufficient funds for transfer".into(),
                ));
            }
            let allowance = (balance - value) / U256::from(fee_per_gas);
            hi = hi.min(u64::try_from(allowance).unwrap_or(u64::MAX));
        }

        // Upper bound must succeed.
        let first = match self.run_call(&mut db, header, request, hi, limits)? {
            CallOutcome::Invalid(reason) => return Err(ExecutionError::InvalidRequest(reason)),
            CallOutcome::Done(r) => r,
        };
        let spent = match first {
            ExecutionResult::Success { gas, .. } => gas.total_gas_spent(),
            ExecutionResult::Revert { output, .. } => {
                return Err(ExecutionError::Reverted { output })
            }
            ExecutionResult::Halt {
                reason: HaltReason::OutOfGas(_),
                ..
            } => return Err(ExecutionError::GasCapExceeded { cap: hi }),
            ExecutionResult::Halt { reason, .. } => {
                return Err(ExecutionError::Evm(format!("execution failed: {reason:?}")))
            }
        };

        // Anything below the gas actually spent cannot succeed.
        let mut lo = spent.saturating_sub(1);
        let succeeds = |gas: u64, db: &mut StateOverlay<'_>| -> Result<bool, ExecutionError> {
            Ok(matches!(
                self.run_call(db, header, request, gas, limits)?,
                CallOutcome::Done(ExecutionResult::Success { .. })
            ))
        };

        // Optimistic probe (geth): spent + stipend, scaled for the 63/64 rule.
        let optimistic = (spent + CALL_STIPEND) * 64 / 63;
        if optimistic > lo && optimistic < hi {
            if succeeds(optimistic, &mut db)? {
                hi = optimistic;
            } else {
                lo = optimistic;
            }
        }
        while lo + 1 < hi {
            let mut mid = lo + (hi - lo) / 2;
            if mid > lo.saturating_mul(2) {
                // Most txs need little more than they spend: favour the low end.
                mid = lo * 2;
            }
            if succeeds(mid, &mut db)? {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        if locked && hi > MAX_GAS {
            // c2: a locked sender's transaction could never be included.
            return Err(xip_request_error(
                request,
                Xip0001Violation::GasLimit { gas_limit: hi },
            ));
        }
        Ok(hi)
    }

    /// XIP-0001 checks of an `eth_estimateGas` request against the state of the
    /// requested block (the transaction would go into the block after `header`). Returns
    /// `true` if `from` is `LOCKED` (the caller then checks c2 on the estimate).
    fn check_request_xip0001(
        &self,
        db: &mut StateOverlay<'_>,
        header: &Header,
        request: &TransactionRequest,
    ) -> Result<bool, ExecutionError> {
        if !self.chain_config().xip0001_active(header.number + 1) {
            return Ok(false);
        }
        let from = request.from.unwrap_or_default();
        let status = xip0001::status(db, from).map_err(|e| ExecutionError::State(e.0))?;
        if status == EoaLockStatus::Unlocked {
            return Ok(false);
        }
        let shape = TxShape {
            sender: from,
            to: request.to.unwrap_or(TxKind::Create),
            value: request.value.unwrap_or_default(),
            gas_limit: 0,
            max_fee_per_gas: 0,
        };
        check_shape(status, &shape).map_err(|e| xip_request_error(request, e.violation))?;
        // c3 only if the request carries a fee field.
        if let Some(max_fee) = request.max_fee_per_gas.or(request.gas_price) {
            let base_fee = next_base_fee(header, self.chain_config());
            let shape = TxShape {
                max_fee_per_gas: max_fee,
                ..shape
            };
            check_tx(status, &shape, base_fee, false)
                .map_err(|e| xip_request_error(request, e.violation))?;
        }
        Ok(true)
    }
}

fn xip_request_error(request: &TransactionRequest, violation: Xip0001Violation) -> ExecutionError {
    ExecutionError::Xip0001 {
        hash: None,
        error: Box::new(Xip0001Error {
            sender: request.from.unwrap_or_default(),
            violation,
        }),
    }
}

fn effective_cap(gas_cap: u64) -> u64 {
    if gas_cap == 0 {
        u64::MAX / 2
    } else {
        gas_cap
    }
}

fn to_call_result(result: ExecutionResult<HaltReason>) -> CallResult {
    match result {
        ExecutionResult::Success {
            gas, logs, output, ..
        } => CallResult::Success {
            output: output.into_data(),
            gas_used: gas.tx_gas_used(),
            logs,
        },
        ExecutionResult::Revert { gas, output, .. } => CallResult::Revert {
            output,
            gas_used: gas.tx_gas_used(),
        },
        ExecutionResult::Halt { reason, gas, .. } => CallResult::Halt {
            reason: format!("{reason:?}"),
            gas_used: gas.tx_gas_used(),
        },
    }
}

fn make_receipt(tx: &RecoveredTx, applied: AppliedTx, cumulative_gas_used: u64) -> ReceiptEnvelope {
    let receipt = Receipt {
        status: Eip658Value::Eip658(applied.success),
        cumulative_gas_used,
        logs: applied.logs,
    }
    .with_bloom();
    ReceiptEnvelope::from_typed(tx.tx.tx_type(), receipt)
}

/// Build-mode failures that only mean "not now" (the tx may become valid later), so
/// the tx is skipped without being reported as rejected.
fn is_transient(e: &InvalidTransaction) -> bool {
    matches!(
        e,
        InvalidTransaction::NonceTooHigh { .. }
            | InvalidTransaction::GasPriceLessThanBasefee
            | InvalidTransaction::CallerGasLimitMoreThanBlock
    )
}

impl BlockExecutor for EvmExecutor {
    fn execute(
        &self,
        parent: &Header,
        block: &Block,
        state: &dyn StateProvider,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        let header = &block.header;
        self.validate_child(parent, header)?;
        let base_fee = header.base_fee_per_gas.unwrap_or_default();

        let mut db = StateOverlay::new(state);
        let mut receipts = Vec::with_capacity(block.body.len());
        let mut cumulative = 0u64;
        let xip0001_active = self.chain_config().xip0001_active(header.number);
        let mut senders: BTreeSet<Address> = BTreeSet::new();
        {
            let mut evm = new_evm(
                &mut db,
                cfg_env(self.chain_config().chain_id),
                block_env(header),
            );
            for tx in &block.body {
                let hash = *tx.tx_hash();
                let rtx = RecoveredTx::recover(tx.clone()).map_err(|e| {
                    ExecutionError::InvalidTransaction {
                        hash,
                        reason: format!("invalid signature: {e}"),
                    }
                })?;
                if rtx.ty() == BLOB_TX_TYPE {
                    return Err(ExecutionError::UnsupportedTxType(BLOB_TX_TYPE));
                }
                let needed = cumulative.saturating_add(rtx.gas_limit());
                if needed > header.gas_limit {
                    return Err(ExecutionError::BlockGasLimitExceeded {
                        used: needed,
                        limit: header.gas_limit,
                    });
                }
                let xip = xip0001_active.then_some(&senders);
                let applied = match self.apply_tx(&mut evm, &rtx, base_fee, xip) {
                    Ok(a) => a,
                    Err(TxFailure::Invalid(e)) => {
                        return Err(ExecutionError::InvalidTransaction {
                            hash,
                            reason: e.to_string(),
                        })
                    }
                    Err(TxFailure::Xip0001(error)) => {
                        return Err(ExecutionError::Xip0001 {
                            hash: Some(hash),
                            error: Box::new(error),
                        })
                    }
                    Err(TxFailure::Fatal(e)) => return Err(e),
                };
                senders.insert(rtx.sender);
                cumulative += applied.gas_used;
                receipts.push(make_receipt(&rtx, applied, cumulative));
            }
        }
        Self::finish(state, db.into_diff(), receipts, cumulative)
    }

    fn build(
        &self,
        parent: &Header,
        attrs: BlockAttributes,
        candidates: Vec<RecoveredTx>,
        state: &dyn StateProvider,
    ) -> Result<BuiltBlock, ExecutionError> {
        if attrs.timestamp <= parent.timestamp {
            return Err(ExecutionError::InvalidHeader(format!(
                "timestamp {} not after parent timestamp {}",
                attrs.timestamp, parent.timestamp
            )));
        }
        if attrs.extra_data.len() > MAX_EXTRA_DATA_SIZE {
            return Err(ExecutionError::InvalidHeader(
                "extra data longer than 32 bytes".into(),
            ));
        }
        let base_fee = next_base_fee(parent, self.chain_config());
        let mut header = Header {
            parent_hash: parent.hash_slow(),
            beneficiary: attrs.beneficiary,
            number: parent.number + 1,
            gas_limit: attrs.gas_limit,
            timestamp: attrs.timestamp,
            extra_data: attrs.extra_data,
            mix_hash: attrs.prev_randao,
            base_fee_per_gas: Some(base_fee),
            ..Default::default()
        };
        set_compat_header_fields(&mut header);

        let mut db = StateOverlay::new(state);
        let mut body = Vec::new();
        let mut senders = Vec::new();
        let mut receipts = Vec::new();
        let mut rejected = Vec::new();
        // Once a sender's tx is skipped, its later (higher-nonce) txs cannot apply.
        let mut skipped: BTreeSet<Address> = BTreeSet::new();
        // Senders of the included txs (XIP-0001 c1).
        let mut included: BTreeSet<Address> = BTreeSet::new();
        let xip0001_active = self.chain_config().xip0001_active(header.number);
        let mut cumulative = 0u64;
        {
            let mut evm = new_evm(
                &mut db,
                cfg_env(self.chain_config().chain_id),
                block_env(&header),
            );
            for tx in candidates {
                if header.gas_limit.saturating_sub(cumulative) < MIN_TX_GAS {
                    break;
                }
                if skipped.contains(&tx.sender) {
                    continue;
                }
                let hash = tx.hash();
                if tx.ty() == BLOB_TX_TYPE {
                    skipped.insert(tx.sender);
                    rejected.push((hash, ExecutionError::UnsupportedTxType(BLOB_TX_TYPE)));
                    continue;
                }
                if cumulative.saturating_add(tx.gas_limit()) > header.gas_limit {
                    // Does not fit in this block; may fit in the next one.
                    skipped.insert(tx.sender);
                    continue;
                }
                let xip = xip0001_active.then_some(&included);
                match self.apply_tx(&mut evm, &tx, base_fee, xip) {
                    Ok(applied) => {
                        included.insert(tx.sender);
                        cumulative += applied.gas_used;
                        receipts.push(make_receipt(&tx, applied, cumulative));
                        senders.push(tx.sender);
                        body.push(tx.tx);
                    }
                    Err(TxFailure::Invalid(e)) => {
                        skipped.insert(tx.sender);
                        if !is_transient(&e) {
                            tracing::debug!(%hash, error = %e, "rejecting transaction");
                            rejected.push((
                                hash,
                                ExecutionError::InvalidTransaction {
                                    hash,
                                    reason: e.to_string(),
                                },
                            ));
                        }
                    }
                    Err(TxFailure::Xip0001(error)) => {
                        skipped.insert(tx.sender);
                        // c1 (next block) and c3 (base fee may move) are "not now".
                        if !error.violation.is_transient() {
                            tracing::debug!(%hash, %error, "rejecting transaction (XIP-0001)");
                            rejected.push((
                                hash,
                                ExecutionError::Xip0001 {
                                    hash: Some(hash),
                                    error: Box::new(error),
                                },
                            ));
                        }
                    }
                    Err(TxFailure::Fatal(e)) => return Err(e),
                }
            }
        }

        let outcome = Self::finish(state, db.into_diff(), receipts, cumulative)?;
        header.state_root = outcome.state_root;
        header.transactions_root = compute_transactions_root(&body);
        header.receipts_root = outcome.receipts_root;
        header.logs_bloom = outcome.logs_bloom;
        header.gas_used = outcome.gas_used;
        Ok(BuiltBlock {
            block: Block::new(header, body),
            outcome,
            senders,
            rejected,
        })
    }
}
