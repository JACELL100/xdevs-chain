//! Chain data in storage (plan §14.1–14.2): genesis initialisation, atomic block
//! commits and typed reads of headers, bodies, receipts and the tx index.
//!
//! Read helpers take any [`DbRead`] so RPC handlers can run several reads against one
//! consistent [`Database::snapshot`].

use anyhow::{bail, Context};
use std::sync::Arc;
use xdevs_execution::{BuiltBlock, ExecutionOutcome};
use xdevs_genesis::{genesis_block, Genesis, GenesisValidator};
use xdevs_primitives::{
    Address, Block, Bytes, ChainConfig, Header, ReceiptEnvelope, RecoveredTx, TxEnvelope,
    TxLocation, B256,
};
use xdevs_state::{
    history::{record_block_history, set_history_start},
    write_state_diff, StateDiff,
};
use xdevs_storage::{
    meta_keys, put_commit_certificate, tables, Database, DbError, DbRead, DbReadExt, WriteBatch,
};

/// Shared handle to the node database.
pub type Db = Arc<dyn Database>;

fn meta_key(key: &str) -> String {
    key.to_string()
}

fn decode_u64(bytes: &[u8]) -> Result<u64, DbError> {
    let arr: [u8; 8] = bytes
        .try_into()
        .map_err(|_| DbError::Decode(format!("expected 8 bytes, got {}", bytes.len())))?;
    Ok(u64::from_be_bytes(arr))
}

/// Number of the latest committed block (`None` = database not initialised).
pub fn latest_number<R: DbRead + ?Sized>(db: &R) -> Result<Option<u64>, DbError> {
    db.get::<tables::Meta>(&meta_key(meta_keys::LATEST_BLOCK))?
        .map(|b| decode_u64(&b))
        .transpose()
}

/// Stored genesis hash (`None` = database not initialised).
pub fn genesis_hash<R: DbRead + ?Sized>(db: &R) -> Result<Option<B256>, DbError> {
    db.get::<tables::Meta>(&meta_key(meta_keys::GENESIS_HASH))?
        .map(|b| {
            if b.len() == 32 {
                Ok(B256::from_slice(&b))
            } else {
                Err(DbError::Decode("genesis hash must be 32 bytes".into()))
            }
        })
        .transpose()
}

/// Header of block `number`.
pub fn header<R: DbRead + ?Sized>(db: &R, number: u64) -> Result<Option<Header>, DbError> {
    db.get::<tables::Headers>(&number)
}

/// Latest committed header (errors if the database is not initialised).
pub fn latest_header<R: DbRead + ?Sized>(db: &R) -> Result<(u64, Header), DbError> {
    let number = latest_number(db)?
        .ok_or_else(|| DbError::Backend("database has no committed block".into()))?;
    let header = header(db, number)?
        .ok_or_else(|| DbError::Backend(format!("missing header of head block {number}")))?;
    Ok((number, header))
}

/// Block number for a block hash.
pub fn number_by_hash<R: DbRead + ?Sized>(db: &R, hash: B256) -> Result<Option<u64>, DbError> {
    db.get::<tables::HeaderNumbers>(&hash)
}

/// Transactions of block `number`.
pub fn body<R: DbRead + ?Sized>(db: &R, number: u64) -> Result<Option<Vec<TxEnvelope>>, DbError> {
    db.get::<tables::Bodies>(&number)
}

/// Full block `number` (with its consensus data, Phase 3).
pub fn block<R: DbRead + ?Sized>(db: &R, number: u64) -> Result<Option<Block>, DbError> {
    let Some(header) = header(db, number)? else {
        return Ok(None);
    };
    let body = body(db, number)?.unwrap_or_default();
    let consensus = db
        .get::<tables::BlockConsensus>(&number)?
        .unwrap_or_default();
    Ok(Some(Block::with_consensus(header, body, consensus)))
}

/// Receipts of block `number`, in transaction order.
pub fn receipts<R: DbRead + ?Sized>(db: &R, number: u64) -> Result<Vec<ReceiptEnvelope>, DbError> {
    Ok(db
        .collect_prefix::<tables::Receipts>(&number.to_be_bytes())?
        .into_iter()
        .map(|(_, r)| r)
        .collect())
}

/// Location of a mined transaction.
pub fn tx_location<R: DbRead + ?Sized>(db: &R, hash: B256) -> Result<Option<TxLocation>, DbError> {
    db.get::<tables::Transactions>(&hash)
}

/// Recover the senders of stored transactions (they were verified before inclusion).
pub fn recover_senders(body: &[TxEnvelope]) -> Result<Vec<Address>, DbError> {
    body.iter()
        .map(|tx| {
            RecoveredTx::recover(tx.clone())
                .map(|r| r.sender)
                .map_err(|e| DbError::Decode(format!("stored tx {}: {e}", tx.tx_hash())))
        })
        .collect()
}

/// Write `header`/`body`/`receipts`/`state_diff` (flat state and trie nodes, on top of the
/// latest state in `db`) of one block and move the head to it.
fn block_batch(
    db: &dyn Database,
    header: &Header,
    body: &[TxEnvelope],
    receipts: &[ReceiptEnvelope],
    state_diff: &StateDiff,
) -> anyhow::Result<(WriteBatch, B256)> {
    let number = header.number;
    let hash = header.hash_slow();
    let mut batch = WriteBatch::new();
    batch.put::<tables::Headers>(&number, header);
    batch.put::<tables::HeaderNumbers>(&hash, &number);
    batch.put::<tables::Bodies>(&number, &body.to_vec());
    for (index, (tx, receipt)) in body.iter().zip(receipts).enumerate() {
        let index = index as u64;
        batch.put::<tables::Transactions>(
            tx.tx_hash(),
            &TxLocation {
                block_number: number,
                index,
            },
        );
        batch.put::<tables::Receipts>(&(number, index), receipt);
    }
    write_state_diff(&mut batch, db, state_diff).context("update state trie")?;
    batch.put::<tables::Meta>(
        &meta_key(meta_keys::LATEST_BLOCK),
        &number.to_be_bytes().to_vec(),
    );
    Ok((batch, hash))
}

/// Commit a built block atomically (one [`WriteBatch`], §14.2), together with its
/// reverse state diff so the state of the last `state_history` blocks stays readable
/// (older history is pruned in the same batch). Returns the block hash.
///
/// Must be called by the only writer, on top of the latest committed block: the
/// pre-block values for the history are read from the current flat state.
pub fn commit_block(
    db: &dyn Database,
    built: &BuiltBlock,
    state_history: u64,
) -> anyhow::Result<B256> {
    commit_executed_block(db, &built.block, &built.outcome, None, state_history)
}

/// Commit an executed block and, for consensus blocks, its commit certificate (RLP) in
/// **one** [`WriteBatch`]: header, body, tx index, receipts, state diff, state history,
/// certificate and the head pointer (research doc §6.5). Same rules as [`commit_block`].
pub fn commit_executed_block(
    db: &dyn Database,
    block: &Block,
    outcome: &ExecutionOutcome,
    certificate: Option<&[u8]>,
    state_history: u64,
) -> anyhow::Result<B256> {
    commit_consensus_block(db, block, outcome, certificate, None, state_history)
}

/// [`commit_executed_block`] for consensus blocks from the staking activation on: the
/// block's consensus data and, for an epoch-boundary block, the validator set of the
/// next epoch (`next_set`, the node's encoding) are written in the same batch.
pub fn commit_consensus_block(
    db: &dyn Database,
    block: &Block,
    outcome: &ExecutionOutcome,
    certificate: Option<&[u8]>,
    next_set: Option<&[u8]>,
    state_history: u64,
) -> anyhow::Result<B256> {
    let header = &block.header;
    let diff = &outcome.state_diff;
    let (mut batch, hash) = block_batch(db, header, &block.body, &outcome.receipts, diff)?;
    if let Some(cert) = certificate {
        put_commit_certificate(&mut batch, header.number, cert);
    }
    if !block.consensus.is_empty() {
        batch.put::<tables::BlockConsensus>(&header.number, &block.consensus);
    }
    if let Some(set) = next_set {
        batch.put::<tables::ValidatorSets>(&header.number, &Bytes::copy_from_slice(set));
    }
    record_block_history(&mut batch, db, header.number, diff, state_history)
        .context("record state history")?;
    db.write(batch)?;
    Ok(hash)
}

/// Replace the stored commit certificate of block `height` (certificate upgrade at
/// `Finalized`).
pub fn write_commit_certificate(
    db: &dyn Database,
    height: u64,
    certificate: &[u8],
) -> anyhow::Result<()> {
    let mut batch = WriteBatch::new();
    put_commit_certificate(&mut batch, height, certificate);
    db.write(batch)?;
    Ok(())
}

/// Stored commit certificate of block `height` (RLP), if any.
pub fn commit_certificate<R: DbRead + ?Sized>(
    db: &R,
    height: u64,
) -> Result<Option<Bytes>, DbError> {
    xdevs_storage::commit_certificate(db, height)
}

/// Canonical encoding of a static validator set (research doc §6.9), stored in
/// `Meta["validators"]`: for each validator sorted by consensus key,
/// `consensus_key (32 B) ‖ power (u64, big-endian)`.
pub fn encode_validator_set(validators: &[GenesisValidator]) -> Vec<u8> {
    let mut sorted: Vec<_> = validators.iter().collect();
    sorted.sort_by_key(|v| v.consensus_key);
    let mut out = Vec::with_capacity(sorted.len() * 40);
    for v in sorted {
        out.extend_from_slice(&v.consensus_key);
        out.extend_from_slice(&v.power.to_be_bytes());
    }
    out
}

/// Check that the database was created with the genesis validator set `validators`
/// (empty = a dev chain without validators): write it on first use, refuse a different
/// set afterwards. The set is not part of the genesis hash, so this is the only guard
/// against starting a node on an existing data dir with another validator set.
pub fn ensure_validator_set(
    db: &dyn Database,
    validators: &[GenesisValidator],
) -> anyhow::Result<()> {
    let key = meta_key(meta_keys::VALIDATORS);
    let stored = db.get::<tables::Meta>(&key)?;
    if validators.is_empty() {
        if stored.is_some_and(|s| !s.is_empty()) {
            bail!(
                "database was created with a validator set but the configured genesis has                  none; use the original genesis or another data dir"
            );
        }
        return Ok(());
    }
    let encoded = encode_validator_set(validators);
    match stored {
        Some(stored) if stored == encoded => Ok(()),
        Some(_) => bail!(
            "database was created with a different genesis validator set; use the              original genesis or another data dir"
        ),
        None => {
            let mut batch = WriteBatch::new();
            batch.put::<tables::Meta>(&key, &encoded);
            db.write(batch)?;
            Ok(())
        }
    }
}

/// Initialise an empty database with `genesis`, or check that an existing database
/// was created from the same genesis. Returns the genesis hash.
pub fn init_genesis(db: &dyn Database, genesis: &Genesis) -> anyhow::Result<B256> {
    let (block, diff) = genesis_block(genesis);
    let hash = block.hash();
    if let Some(stored) = genesis_hash(db)? {
        if stored != hash {
            bail!(
                "database was created from a different genesis (stored {stored}, \
                 configured {hash}); use another data dir or the original genesis"
            );
        }
        if latest_number(db)?.is_none() {
            bail!("database has a genesis hash but no head block (corrupted?)");
        }
        // The chain id and fee parameters are not part of the genesis hash.
        if let Some(stored) = db.get::<tables::Meta>(&meta_key(meta_keys::CHAIN_CONFIG))? {
            let stored: ChainConfig =
                serde_json::from_slice(&stored).context("decode stored chain config")?;
            let configured = genesis.chain_config();
            if stored != configured && same_but_fork_schedule(&stored, &configured) {
                // A coordinated upgrade schedules (or reschedules) the staking-driven
                // validator set or the encrypted mempool: allowed only while the chain is
                // below both the stored and the new activation height, so no executed
                // block changes meaning.
                let head = latest_number(db)?.unwrap_or_default();
                let encrypted_changed = stored.encrypted_mempool_block
                    != configured.encrypted_mempool_block
                    || stored.encrypted_mempool != configured.encrypted_mempool;
                let started =
                    |c: &ChainConfig| c.encrypted_mempool_block.is_some_and(|b| head >= b);
                let ed25519_started =
                    |c: &ChainConfig| c.ed25519_verify_block.is_some_and(|b| head >= b);
                if stored.ed25519_verify_block != configured.ed25519_verify_block
                    && (ed25519_started(&stored) || ed25519_started(&configured))
                {
                    bail!(
                        "the ED25519VERIFY activation (ed25519VerifyBlock) cannot change once it is active: head {head}, stored {:?}, configured {:?}",
                        stored.ed25519_verify_block,
                        configured.ed25519_verify_block
                    );
                }
                let rotation_started =
                    |c: &ChainConfig| c.proposer_rotation_active(head);
                if stored.proposer_rotation_block != configured.proposer_rotation_block
                    && (rotation_started(&stored) || rotation_started(&configured))
                {
                    bail!(
                        "the proposer rotation activation (proposerRotationBlock) cannot change once it is active: head {head}, stored {:?}, configured {:?}",
                        stored.proposer_rotation_block,
                        configured.proposer_rotation_block
                    );
                }
                if encrypted_changed && (started(&stored) || started(&configured)) {
                    bail!(
                        "the encrypted mempool schedule (encryptedMempoolBlock, \
                         encryptedMempool) cannot change once it is active: head {head}, \
                         stored activation {:?}, configured {:?}",
                        stored.encrypted_mempool_block,
                        configured.encrypted_mempool_block
                    );
                }
                let staking_changed = stored.staking_validators_block
                    != configured.staking_validators_block
                    || stored.epoch_length != configured.epoch_length
                    || stored.block_reward != configured.block_reward;
                if staking_changed
                    && (stored.staking_active(head) || configured.staking_active(head))
                {
                    bail!(
                        "the staking fork schedule (stakingValidatorsBlock, epochLength, \
                         blockReward) cannot change once it is active: head {head}, stored \
                         activation {:?}, configured {:?}",
                        stored.staking_validators_block,
                        configured.staking_validators_block
                    );
                }
                let mut batch = WriteBatch::new();
                let cfg = serde_json::to_vec(&configured).context("serialize chain config")?;
                batch.put::<tables::Meta>(&meta_key(meta_keys::CHAIN_CONFIG), &cfg);
                db.write(batch)?;
                tracing::warn!(
                    target: "xdevs::node",
                    head,
                    activation = ?configured.staking_validators_block,
                    epoch_length = configured.epoch_length,
                    block_reward = %configured.block_reward,
                    encrypted_mempool = ?configured.encrypted_mempool_block,
                    proposer_rotation = ?configured.proposer_rotation_block,
                    "fork schedule updated from the genesis config"
                );
                return Ok(hash);
            }
            if stored != configured {
                bail!(
                    "database was created with a different chain config (stored chain id {},                      configured chain id {}); use another data dir or the original genesis",
                    stored.chain_id,
                    genesis.config.chain_id
                );
            }
        }
        return Ok(hash);
    }
    if latest_number(db)?.is_some() {
        bail!("database has blocks but no genesis hash (corrupted?)");
    }
    let (mut batch, _) = block_batch(db, &block.header, &[], &[], &diff)?;
    batch.put::<tables::Meta>(&meta_key(meta_keys::GENESIS_HASH), &hash.to_vec());
    // Nothing exists before genesis: every state from block 0 on is readable.
    set_history_start(&mut batch, 1);
    let cfg = serde_json::to_vec(&genesis.chain_config()).context("serialize chain config")?;
    batch.put::<tables::Meta>(&meta_key(meta_keys::CHAIN_CONFIG), &cfg);
    db.write(batch)?;
    Ok(hash)
}

/// `true` if `a` and `b` differ at most in the fork schedules: staking
/// ([`same_but_staking_schedule`]) and the encrypted mempool (`encryptedMempoolBlock`,
/// `encryptedMempool`).
fn same_but_fork_schedule(a: &ChainConfig, b: &ChainConfig) -> bool {
    let strip = |c: &ChainConfig| ChainConfig {
        encrypted_mempool_block: None,
        encrypted_mempool: None,
        ed25519_verify_block: None,
        local_fee_markets_block: None,
        proposer_rotation_block: None,
        ..c.clone()
    };
    same_but_staking_schedule(&strip(a), &strip(b))
}

/// `true` if `a` and `b` differ at most in the staking fork schedule
/// (`stakingValidatorsBlock`, `epochLength`, `blockReward`).
fn same_but_staking_schedule(a: &ChainConfig, b: &ChainConfig) -> bool {
    let strip = |c: &ChainConfig| ChainConfig {
        staking_validators_block: None,
        epoch_length: 0,
        block_reward: Default::default(),
        ..c.clone()
    };
    strip(a) == strip(b)
}

/// Recompute the state root of the stored flat state, check it against the persisted state
/// trie and compare it with the head header (detects a torn or corrupted database on
/// start). Returns the head.
pub fn verify_head(db: &dyn Database) -> anyhow::Result<(u64, Header)> {
    let snapshot = db.snapshot()?;
    let (number, head) = latest_header(&*snapshot)?;
    let root = xdevs_state::trie::check_state_root(&*snapshot)?;
    if root != head.state_root {
        bail!(
            "stored state root {root} does not match head block {number} state root {}",
            head.state_root
        );
    }
    Ok((number, head))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::constants::DEV_CHAIN_ID;
    use xdevs_storage::MemDb;

    #[test]
    fn genesis_init_is_idempotent_and_checked() {
        let db = MemDb::new();
        let g = Genesis::dev(DEV_CHAIN_ID);
        let h1 = init_genesis(&db, &g).unwrap();
        let h2 = init_genesis(&db, &g).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(latest_number(&db).unwrap(), Some(0));
        assert_eq!(genesis_hash(&db).unwrap(), Some(h1));
        assert_eq!(number_by_hash(&db, h1).unwrap(), Some(0));
        assert_eq!(body(&db, 0).unwrap(), Some(vec![]));
        let (n, head) = verify_head(&db).unwrap();
        assert_eq!(n, 0);
        assert_eq!(head.hash_slow(), h1);

        let mut other = Genesis::dev(DEV_CHAIN_ID);
        other.timestamp = 1;
        let err = init_genesis(&db, &other).unwrap_err().to_string();
        assert!(err.contains("different genesis"), "{err}");

        // Same genesis block, different chain id: also refused.
        let err = init_genesis(&db, &Genesis::dev(DEV_CHAIN_ID + 1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("different chain config"), "{err}");
    }

    #[test]
    fn staking_schedule_can_be_added_before_activation_only() {
        let db = MemDb::new();
        let g = Genesis::dev(DEV_CHAIN_ID);
        init_genesis(&db, &g).unwrap();
        // Schedule the activation at block 10 on a chain at height 0: accepted, stored.
        let mut scheduled = g.clone();
        scheduled.config.staking_validators_block = Some(10);
        scheduled.config.epoch_length = 20;
        init_genesis(&db, &scheduled).unwrap();
        let stored: ChainConfig = serde_json::from_slice(
            &db.get::<tables::Meta>(&meta_key(meta_keys::CHAIN_CONFIG))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stored.staking_validators_block, Some(10));
        init_genesis(&db, &scheduled).unwrap();
        // Any other difference is still refused.
        let mut other = scheduled.clone();
        other.config.base_fee.burn_bps = 1;
        assert!(init_genesis(&db, &other).is_err());

        // Once the head is at/after the activation the schedule is frozen.
        let mut batch = WriteBatch::new();
        batch.put::<tables::Meta>(
            &meta_key(meta_keys::LATEST_BLOCK),
            &10u64.to_be_bytes().to_vec(),
        );
        db.write(batch).unwrap();
        let mut moved = scheduled.clone();
        moved.config.staking_validators_block = Some(50);
        let err = init_genesis(&db, &moved).unwrap_err().to_string();
        assert!(err.contains("cannot change once it is active"), "{err}");
    }

    #[test]
    fn proposer_rotation_can_be_added_before_activation_only() {
        let db = MemDb::new();
        let g = Genesis::dev(DEV_CHAIN_ID);
        init_genesis(&db, &g).unwrap();
        // Scheduled at 10 on a chain at height 0: accepted, stored, rescheduling too.
        let mut scheduled = g.clone();
        scheduled.config.proposer_rotation_block = Some(10);
        init_genesis(&db, &scheduled).unwrap();
        let stored = || -> ChainConfig {
            serde_json::from_slice(
                &db.get::<tables::Meta>(&meta_key(meta_keys::CHAIN_CONFIG))
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(stored().proposer_rotation_block, Some(10));
        let mut moved = scheduled.clone();
        moved.config.proposer_rotation_block = Some(12);
        init_genesis(&db, &moved).unwrap();
        assert_eq!(stored().proposer_rotation_block, Some(12));

        // Head at the activation: frozen (moving or removing it is refused).
        let mut batch = WriteBatch::new();
        batch.put::<tables::Meta>(
            &meta_key(meta_keys::LATEST_BLOCK),
            &12u64.to_be_bytes().to_vec(),
        );
        db.write(batch).unwrap();
        for other in [Some(50), None] {
            let mut changed = moved.clone();
            changed.config.proposer_rotation_block = other;
            let err = init_genesis(&db, &changed).unwrap_err().to_string();
            assert!(err.contains("cannot change once it is active"), "{err}");
        }
        // Scheduling it at or below the head of a running chain is refused too.
        let fresh_db = MemDb::new();
        init_genesis(&fresh_db, &g).unwrap();
        let mut batch = WriteBatch::new();
        batch.put::<tables::Meta>(
            &meta_key(meta_keys::LATEST_BLOCK),
            &20u64.to_be_bytes().to_vec(),
        );
        fresh_db.write(batch).unwrap();
        let mut late = g.clone();
        late.config.proposer_rotation_block = Some(20);
        assert!(init_genesis(&fresh_db, &late).is_err());
        init_genesis(&db, &moved).unwrap();
    }

    #[test]
    fn encrypted_mempool_schedule_can_be_added_before_activation_only() {
        let db = MemDb::new();
        let g = Genesis::dev(DEV_CHAIN_ID);
        init_genesis(&db, &g).unwrap();
        let mut scheduled = g.clone();
        scheduled.config.encrypted_mempool_block = Some(10);
        scheduled.config.encrypted_mempool = Some(xdevs_primitives::EncryptedMempoolParams {
            threshold: 1,
            public_key: xdevs_primitives::Bytes::from(vec![2u8; 33]),
            verification_keys: vec![xdevs_primitives::Bytes::from(vec![2u8; 33])],
        });
        init_genesis(&db, &scheduled).unwrap();
        init_genesis(&db, &scheduled).unwrap();
        let mut batch = WriteBatch::new();
        batch.put::<tables::Meta>(
            &meta_key(meta_keys::LATEST_BLOCK),
            &10u64.to_be_bytes().to_vec(),
        );
        db.write(batch).unwrap();
        let mut moved = scheduled.clone();
        moved.config.encrypted_mempool_block = Some(50);
        let err = init_genesis(&db, &moved).unwrap_err().to_string();
        assert!(err.contains("encrypted mempool schedule"), "{err}");
        // The staking schedule can still be added (head 10 is before its activation).
        let mut staking = scheduled.clone();
        staking.config.staking_validators_block = Some(100);
        init_genesis(&db, &staking).unwrap();
    }

    #[test]
    fn verify_head_detects_tampered_state() {
        let db = MemDb::new();
        init_genesis(&db, &Genesis::dev(DEV_CHAIN_ID)).unwrap();
        let mut batch = WriteBatch::new();
        batch.put::<tables::PlainAccounts>(
            &Address::repeat_byte(0x42),
            &xdevs_primitives::AccountInfo::new(0, xdevs_primitives::U256::from(1)),
        );
        db.write(batch).unwrap();
        assert!(verify_head(&db).is_err());
    }
}
