//! Dev-mode block producer (plan §5.4 steps 2–3 and 7–8, single validator).
//!
//! Per tick: take the best pool txs for the next base fee, build the block on the
//! latest committed state, commit it in one atomic batch, then update the pool.

use crate::chain::{self, Db};
use parking_lot::Mutex;
use std::sync::Arc;
use xdevs_execution::{next_base_fee, BlockAttributes, BlockExecutor, EvmExecutor};
use xdevs_mempool::{Pool, TxPool};
use xdevs_primitives::{Address, Bytes, ChainConfig, TxHash, B256};
use xdevs_state::{history::DEFAULT_STATE_HISTORY_BLOCKS, DbStateProvider};

/// Summary of a produced block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducedBlock {
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: B256,
    /// Header timestamp.
    pub timestamp: u64,
    /// Included transactions.
    pub tx_hashes: Vec<TxHash>,
    /// Gas used.
    pub gas_used: u64,
    /// Base fee of the block.
    pub base_fee: u64,
    /// Candidates dropped from the pool as invalid.
    pub rejected: usize,
}

/// Builds and commits blocks on top of the stored head.
pub struct BlockProducer {
    db: Db,
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
    cfg: ChainConfig,
    fee_recipient: Address,
    extra_data: Bytes,
    /// Past blocks whose state stays readable (see [`chain::commit_block`]).
    state_history: u64,
    /// Serialises block production (one block at a time).
    lock: Mutex<()>,
}

impl std::fmt::Debug for BlockProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockProducer")
            .field("chain_id", &self.cfg.chain_id)
            .field("fee_recipient", &self.fee_recipient)
            .finish_non_exhaustive()
    }
}

impl BlockProducer {
    /// New producer; `extra_data` must be at most 32 bytes.
    pub fn new(
        db: Db,
        pool: Arc<Pool>,
        executor: Arc<EvmExecutor>,
        cfg: ChainConfig,
        fee_recipient: Address,
        extra_data: Bytes,
    ) -> Self {
        Self {
            db,
            pool,
            executor,
            cfg,
            fee_recipient,
            extra_data,
            state_history: DEFAULT_STATE_HISTORY_BLOCKS,
            lock: Mutex::new(()),
        }
    }

    /// Keep the state of the last `blocks` blocks readable (default
    /// [`DEFAULT_STATE_HISTORY_BLOCKS`]; `u64::MAX` = archive).
    pub fn with_state_history(mut self, blocks: u64) -> Self {
        self.state_history = blocks;
        self
    }

    /// Build, commit and announce one block. `now_secs` is the wall-clock time used for
    /// the header timestamp (bumped to `parent + 1` if needed). Blocking: call from a
    /// blocking thread.
    pub fn produce(&self, now_secs: u64) -> anyhow::Result<ProducedBlock> {
        let _guard = self.lock.lock();
        let built = {
            let snapshot = self.db.snapshot()?;
            let (_, parent) = chain::latest_header(&*snapshot)?;
            let base_fee = next_base_fee(&parent, &self.cfg);
            let candidates = self.pool.best_txs(base_fee, self.cfg.block_gas_limit);
            let attrs = BlockAttributes {
                timestamp: now_secs.max(parent.timestamp + 1),
                beneficiary: self.fee_recipient,
                prev_randao: parent.hash_slow(),
                extra_data: self.extra_data.clone(),
                gas_limit: self.cfg.block_gas_limit,
            };
            let state = DbStateProvider::new(&*snapshot);
            self.executor.build(&parent, attrs, candidates, &state)?
        };
        let hash = chain::commit_block(&*self.db, &built, self.state_history)?;

        let rejected: Vec<TxHash> = built.rejected.iter().map(|(h, _)| *h).collect();
        for (h, e) in &built.rejected {
            tracing::debug!(target: "xdevs::producer", tx = %h, error = %e, "dropping invalid tx");
        }
        self.pool.remove(&rejected);
        self.pool
            .on_block_committed(&built.block, &built.outcome.receipts);
        self.pool
            .set_base_fee(next_base_fee(&built.block.header, &self.cfg));

        let header = &built.block.header;
        Ok(ProducedBlock {
            number: header.number,
            hash,
            timestamp: header.timestamp,
            tx_hashes: built.block.tx_hashes().collect(),
            gas_used: header.gas_used,
            base_fee: header.base_fee_per_gas.unwrap_or_default(),
            rejected: rejected.len(),
        })
    }
}

/// Current Unix time in seconds (only used for header timestamps).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_crypto::{sign_tx_envelope, signer_from_hex};
    use xdevs_genesis::{genesis_state, Genesis, DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
    use xdevs_mempool::{FromState, PoolConfig, PoolParams, TxOrigin};
    use xdevs_primitives::{
        constants::{DEV_CHAIN_ID, GWEI},
        RecoveredTx, TxEip1559, TxKind, U256,
    };
    use xdevs_state::{InMemoryState, StateProvider};
    use xdevs_storage::MemDb;

    #[test]
    fn produced_block_reexecutes_to_the_same_roots() {
        let db: Db = Arc::new(MemDb::new());
        let genesis = Genesis::dev(DEV_CHAIN_ID);
        chain::init_genesis(&*db, &genesis).unwrap();
        let cfg = genesis.chain_config();
        let (_, genesis_header) = chain::latest_header(&*db).unwrap();
        let pool = Arc::new(Pool::new(
            PoolParams::from_chain_config(&cfg),
            PoolConfig::default(),
            Arc::new(FromState(DbStateProvider::new(db.clone()))),
            next_base_fee(&genesis_header, &cfg),
        ));
        let executor = Arc::new(EvmExecutor::new(cfg.clone()));
        let producer = BlockProducer::new(
            db.clone(),
            pool.clone(),
            executor.clone(),
            cfg,
            Address::repeat_byte(0xfe),
            Bytes::from_static(b"xdevs"),
        );

        let signer = signer_from_hex(DEV_PRIVATE_KEYS[0]).unwrap();
        let tx = sign_tx_envelope(
            &signer,
            TxEip1559 {
                chain_id: DEV_CHAIN_ID,
                nonce: 0,
                gas_limit: 21_000,
                max_fee_per_gas: 10 * u128::from(GWEI),
                max_priority_fee_per_gas: u128::from(GWEI),
                to: TxKind::Call(Address::repeat_byte(0x77)),
                value: U256::from(5u64),
                input: Bytes::new(),
                access_list: Default::default(),
            },
        )
        .unwrap();
        pool.add(RecoveredTx::recover(tx).unwrap(), TxOrigin::Local)
            .unwrap();

        // Timestamp 0 is bumped to parent + 1.
        let produced = producer.produce(0).unwrap();
        assert_eq!(produced.number, 1);
        assert_eq!(produced.tx_hashes.len(), 1);
        assert_eq!(produced.timestamp, genesis_header.timestamp + 1);
        assert!(pool.is_empty(), "included tx left the pool");

        // Strict re-execution on the genesis state reproduces the header.
        let block = chain::block(&*db, 1).unwrap().unwrap();
        assert_eq!(block.hash(), produced.hash);
        assert_eq!(block.header.parent_hash, genesis_header.hash_slow());
        let pre = InMemoryState::from_diff(&genesis_state(&genesis));
        let outcome = executor.execute(&genesis_header, &block, &pre).unwrap();
        assert_eq!(outcome.state_root, block.header.state_root);
        assert_eq!(outcome.receipts_root, block.header.receipts_root);
        assert_eq!(outcome.logs_bloom, block.header.logs_bloom);
        assert_eq!(outcome.gas_used, block.header.gas_used);
        assert_eq!(chain::receipts(&*db, 1).unwrap(), outcome.receipts);
        let stored = DbStateProvider::new(db.clone());
        assert_eq!(
            stored
                .basic_account(DEV_ACCOUNTS[0])
                .unwrap()
                .unwrap()
                .nonce,
            1
        );
        chain::verify_head(&*db).unwrap();

        // An empty block on top keeps the state root.
        let empty = producer.produce(0).unwrap();
        assert_eq!(empty.number, 2);
        let h2 = chain::header(&*db, 2).unwrap().unwrap();
        assert_eq!(h2.state_root, block.header.state_root);
        assert_eq!(h2.parent_hash, produced.hash);
    }
}
