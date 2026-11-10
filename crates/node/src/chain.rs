//! Chain data in storage (plan §14.1–14.2): genesis initialisation, atomic block
//! commits and typed reads of headers, bodies, receipts and the tx index.
//!
//! Read helpers take any [`DbRead`] so RPC handlers can run several reads against one
//! consistent [`Database::snapshot`].

use anyhow::{bail, Context};
use std::sync::Arc;
use xdevs_execution::BuiltBlock;
use xdevs_genesis::{genesis_block, Genesis};
use xdevs_primitives::{
    Address, Block, ChainConfig, Header, ReceiptEnvelope, RecoveredTx, TxEnvelope, TxLocation, B256,
};
use xdevs_state::{
    history::{record_block_history, set_history_start},
    write_state_diff, DbStateProvider, StateDiff, StateProvider,
};
use xdevs_storage::{meta_keys, tables, Database, DbError, DbRead, DbReadExt, WriteBatch};

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

/// Full block `number`.
pub fn block<R: DbRead + ?Sized>(db: &R, number: u64) -> Result<Option<Block>, DbError> {
    let Some(header) = header(db, number)? else {
        return Ok(None);
    };
    let body = body(db, number)?.unwrap_or_default();
    Ok(Some(Block::new(header, body)))
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

/// Write `header`/`body`/`receipts`/`state_diff` of one block and move the head to it.
fn block_batch(
    header: &Header,
    body: &[TxEnvelope],
    receipts: &[ReceiptEnvelope],
    state_diff: &StateDiff,
) -> (WriteBatch, B256) {
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
    write_state_diff(&mut batch, state_diff);
    batch.put::<tables::Meta>(
        &meta_key(meta_keys::LATEST_BLOCK),
        &number.to_be_bytes().to_vec(),
    );
    (batch, hash)
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
    let header = &built.block.header;
    let diff = &built.outcome.state_diff;
    let (mut batch, hash) = block_batch(header, &built.block.body, &built.outcome.receipts, diff);
    record_block_history(&mut batch, db, header.number, diff, state_history)
        .context("record state history")?;
    db.write(batch)?;
    Ok(hash)
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
            if stored != genesis.chain_config() {
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
    let (mut batch, _) = block_batch(&block.header, &[], &[], &diff);
    batch.put::<tables::Meta>(&meta_key(meta_keys::GENESIS_HASH), &hash.to_vec());
    // Nothing exists before genesis: every state from block 0 on is readable.
    set_history_start(&mut batch, 1);
    let cfg = serde_json::to_vec(&genesis.chain_config()).context("serialize chain config")?;
    batch.put::<tables::Meta>(&meta_key(meta_keys::CHAIN_CONFIG), &cfg);
    db.write(batch)?;
    Ok(hash)
}

/// Recompute the state root of the stored flat state and compare it with the head
/// header (detects a torn or corrupted database on start). Returns the head.
pub fn verify_head(db: &dyn Database) -> anyhow::Result<(u64, Header)> {
    let snapshot = db.snapshot()?;
    let (number, head) = latest_header(&*snapshot)?;
    let root = DbStateProvider::new(&*snapshot).state_root_with(&StateDiff::default())?;
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
