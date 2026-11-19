//! Local fee markets in block execution (Phase 5, plan §15.2): the per-transaction
//! local base fee read from the parent state, and the end-of-block table update. The
//! math is in [`xdevs_primitives::local_fee`].

use crate::db::{StateDbError, StateOverlay};
use std::collections::BTreeMap;
use xdevs_primitives::{
    local_fee::{fee_at, next_local_fee, pack, slot, LOCAL_FEE_MARKET_ADDRESS},
    AccountInfo, Address, ChainConfig, Header, RecoveredTx, Transaction, TxKind,
};

/// Local fee of `target` for transactions in block `number`, read from `db` (the table
/// only changes at the end of a block, so this is the parent state's value).
pub(crate) fn of(
    db: &mut StateOverlay<'_>,
    target: Address,
    number: u64,
) -> Result<u64, StateDbError> {
    Ok(fee_at(
        db.slot(LOCAL_FEE_MARKET_ADDRESS, slot(target))?,
        number,
    ))
}

/// Local fee `tx` pays in block `number` (zero before the activation and for contract
/// creations).
pub(crate) fn of_tx(
    db: &mut StateOverlay<'_>,
    cfg: &ChainConfig,
    number: u64,
    tx: &RecoveredTx,
) -> Result<u64, StateDbError> {
    match tx.kind() {
        TxKind::Call(to) if cfg.local_fee_markets_active(number) => of(db, to, number),
        _ => Ok(0),
    }
}

/// Gas used per target (top-level `to`) by the transactions of one block.
pub(crate) struct GasByTarget {
    active: bool,
    gas: BTreeMap<Address, u64>,
}

impl GasByTarget {
    pub(crate) fn new(cfg: &ChainConfig, number: u64) -> Self {
        Self {
            active: cfg.local_fee_markets_active(number),
            gas: BTreeMap::new(),
        }
    }

    /// Count an included transaction (successful or not: its gas was used).
    pub(crate) fn record(&mut self, tx: &RecoveredTx, gas_used: u64) {
        if let (true, TxKind::Call(to)) = (self.active, tx.kind()) {
            let g = self.gas.entry(to).or_default();
            *g = g.saturating_add(gas_used);
        }
    }

    /// End of block (after all transactions): new local fees of the targets used in
    /// `header`'s block. Targets without gas decay lazily, so they are not touched.
    pub(crate) fn apply(
        &self,
        db: &mut StateOverlay<'_>,
        header: &Header,
    ) -> Result<(), StateDbError> {
        let mut wrote = false;
        for (target, gas) in &self.gas {
            let key = slot(*target);
            let word = db.slot(LOCAL_FEE_MARKET_ADDRESS, key)?;
            let current = fee_at(word, header.number);
            let new = pack(
                next_local_fee(current, *gas, header.gas_limit),
                header.number,
            );
            if new != word {
                db.set_slot(LOCAL_FEE_MARKET_ADDRESS, key, new);
                wrote = true;
            }
        }
        if wrote && db.account(LOCAL_FEE_MARKET_ADDRESS)?.is_none() {
            // Nonce 1: never "empty" (EIP-161), so a value transfer cannot delete it.
            db.set_account(
                LOCAL_FEE_MARKET_ADDRESS,
                AccountInfo {
                    nonce: 1,
                    ..Default::default()
                },
            );
        }
        Ok(())
    }
}
