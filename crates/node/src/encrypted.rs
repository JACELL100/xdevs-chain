//! Encrypted mempool, node side (Phase 5 prototype, `docs/research/encrypted-mempool.md`).
//!
//! Commit-then-reveal over the existing pipeline:
//!
//! 1. **Order blind (block N).** Envelopes are ordinary transactions to
//!    `ENCRYPTED_INBOX_ADDRESS` (see [`xdevs_crypto::encrypted_tx`]); they travel through the
//!    normal pool and gossip, pay normal gas in block N and execute as a plain call to an
//!    address without code. The *decryptable* envelopes of a block
//!    ([`decryptable_envelopes`]) are those with a well-formed payload for the current key
//!    whose proof of knowledge verifies for their sender, within the block's inner-gas
//!    budget, in block order. Anything else is ignored after paying its fee (griefing
//!    costs gas).
//! 2. **Release after commit.** When a node holding a key share commits block N it
//!    computes one DLEQ-proven decryption share per decryptable envelope and gossips them
//!    as a [`ShareMessage`] on the tx topic (prefix byte [`SHARE_MESSAGE_PREFIX`], which
//!    no EIP-2718 transaction starts with). Every node verifies and collects them in its
//!    [`SharePool`].
//! 3. **Execute first (block N+1).** The proposer of N+1 puts exactly `t` verified shares
//!    per envelope (lowest share indices) into the block ([`DecryptionData`], next to the
//!    Phase-3 consensus data in `Block::consensus`, see [`split_block_data`]), decrypts,
//!    and builds the block with the decrypted transactions as the first candidates, in
//!    committed order. Validators re-verify every share, decrypt, rebuild that prefix with
//!    the executor's skip rules and require the body to start with it. A payload that
//!    does not open, does not decode, is for another chain or exceeds its committed gas is
//!    skipped deterministically.
//!
//! A block after a block with decryptable envelopes is invalid without their decryption:
//! the proposer waits (bounded) for shares, and a round without them times out.

use alloy_rlp::{RlpDecodable, RlpEncodable};
use parking_lot::Mutex;
use std::{
    collections::{btree_map, BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, OnceLock},
};
use xdevs_crypto::{
    decode_and_recover,
    encrypted_tx::{is_envelope_target, EncryptedPayload, MempoolKey},
    threshold::{Ciphertext, DecryptionShare, KeyShare, POINT_LEN},
};
use xdevs_network::{NetworkHandle, TxValidator, Verdict};
use xdevs_primitives::{Address, Block, Bytes, ChainConfig, RecoveredTx, Transaction, B256};

/// First byte of a gossiped [`ShareMessage`] (EIP-2718 types are `< 0x80`, legacy RLP
/// lists `>= 0xc0`).
pub const SHARE_MESSAGE_PREFIX: u8 = 0xb1;
/// Committed blocks whose shares are kept.
const KEPT_BLOCKS: usize = 16;
/// Share messages buffered for blocks this node has not committed yet.
const MAX_EARLY_MESSAGES: usize = 1024;
/// Envelopes one share message may cover.
const MAX_ENVELOPES_PER_MESSAGE: usize = 4096;
const LOG: &str = "xdevs::encrypted";

/// A decryptable envelope of a committed block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEnvelope {
    /// Index of the envelope transaction in the block body.
    pub tx_index: u32,
    /// Envelope sender (bound into the payload's label).
    pub sender: Address,
    /// The decoded payload.
    pub payload: EncryptedPayload,
}

/// The envelopes of `block` the committee decrypts, in block order (see the module docs).
/// Empty unless the encrypted mempool is active at the block's height.
pub fn decryptable_envelopes(
    cfg: &ChainConfig,
    key: &MempoolKey,
    block: &Block,
) -> Vec<BlockEnvelope> {
    if !cfg.encrypted_mempool_active(block.number()) {
        return Vec::new();
    }
    let budget = cfg.encrypted_gas_budget();
    let mut used = 0u64;
    let mut out = Vec::new();
    for (i, tx) in block.body.iter().enumerate() {
        if !is_envelope_target(tx.to()) {
            continue;
        }
        let Ok(payload) = EncryptedPayload::decode(tx.input()) else {
            continue;
        };
        let Ok(rtx) = RecoveredTx::recover(tx.clone()) else {
            continue;
        };
        if key
            .check(cfg.chain_id, rtx.sender, &payload, budget)
            .is_err()
        {
            continue;
        }
        let Some(total) = used
            .checked_add(payload.inner_gas_limit)
            .filter(|t| *t <= budget)
        else {
            continue;
        };
        used = total;
        out.push(BlockEnvelope {
            tx_index: i as u32,
            sender: rtx.sender,
            payload,
        });
    }
    out
}

/// A decryption share on the wire.
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct WireShare {
    /// Share index (1-based).
    pub index: u32,
    /// `D_i` (33 bytes).
    pub d: Bytes,
    /// DLEQ challenge.
    pub c: B256,
    /// DLEQ response.
    pub z: B256,
}

impl From<&DecryptionShare> for WireShare {
    fn from(s: &DecryptionShare) -> Self {
        Self {
            index: s.index,
            d: Bytes::copy_from_slice(&s.d),
            c: B256::from(s.c),
            z: B256::from(s.z),
        }
    }
}

impl WireShare {
    /// The share, if `d` has the right length.
    pub fn to_share(&self) -> Option<DecryptionShare> {
        if self.d.len() != POINT_LEN {
            return None;
        }
        let mut d = [0u8; POINT_LEN];
        d.copy_from_slice(&self.d);
        Some(DecryptionShare {
            index: self.index,
            d,
            c: self.c.0,
            z: self.z.0,
        })
    }
}

/// Shares for one envelope.
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct EnvelopeShares {
    /// Index of the envelope in its block's body.
    pub tx_index: u32,
    /// Shares, by strictly increasing index.
    pub shares: Vec<WireShare>,
}

/// What block N+1 carries to decrypt block N's envelopes: exactly `t` shares per
/// decryptable envelope, in envelope order.
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct DecryptionData {
    /// Hash of the block whose envelopes are decrypted (the parent).
    pub block_hash: B256,
    /// One entry per decryptable envelope.
    pub envelopes: Vec<EnvelopeShares>,
}

impl DecryptionData {
    /// Canonical RLP.
    pub fn to_bytes(&self) -> Vec<u8> {
        alloy_rlp::encode(self)
    }

    /// Decode canonical RLP (no trailing bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        alloy_rlp::decode_exact(bytes).map_err(|e| format!("undecodable decryption data: {e}"))
    }
}

/// A validator's shares for the envelopes of one committed block (gossiped).
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct ShareMessage {
    /// Block number.
    pub block_number: u64,
    /// Block hash.
    pub block_hash: B256,
    /// One entry per decryptable envelope (one share each).
    pub envelopes: Vec<EnvelopeShares>,
}

impl ShareMessage {
    /// `SHARE_MESSAGE_PREFIX ‖ rlp(self)`.
    pub fn to_gossip(&self) -> Bytes {
        let mut out = vec![SHARE_MESSAGE_PREFIX];
        out.extend_from_slice(&alloy_rlp::encode(self));
        out.into()
    }

    /// Inverse of [`ShareMessage::to_gossip`].
    pub fn from_gossip(raw: &[u8]) -> Option<Self> {
        let (&prefix, rest) = raw.split_first()?;
        if prefix != SHARE_MESSAGE_PREFIX {
            return None;
        }
        alloy_rlp::decode_exact(rest).ok()
    }

    /// `share`'s message for the decryptable envelopes `envs` of block `number`.
    pub fn create(
        share: &KeyShare,
        block_number: u64,
        block_hash: B256,
        envs: &[BlockEnvelope],
    ) -> anyhow::Result<Self> {
        let mut envelopes = Vec::with_capacity(envs.len());
        for env in envs {
            let s = share
                .decryption_share_random(&env.payload.ciphertext)
                .map_err(|e| anyhow::anyhow!("decryption share: {e}"))?;
            envelopes.push(EnvelopeShares {
                tx_index: env.tx_index,
                shares: vec![WireShare::from(&s)],
            });
        }
        Ok(Self {
            block_number,
            block_hash,
            envelopes,
        })
    }
}

/// `Block::consensus` from the encrypted-mempool activation on:
/// `rlp([staking_data, decryption_data])` (either may be empty, not both).
#[derive(RlpEncodable, RlpDecodable)]
struct BlockData {
    staking: Bytes,
    decryption: Bytes,
}

/// Split `Block::consensus` of block `height` into the Phase-3 consensus data and the
/// decryption data. Before the activation the bytes are the consensus data unchanged.
/// `Err` = the block is invalid.
pub fn split_block_data(
    cfg: &ChainConfig,
    height: u64,
    raw: &Bytes,
) -> Result<(Bytes, Option<DecryptionData>), String> {
    if !cfg.encrypted_mempool_active(height) || raw.is_empty() {
        return Ok((raw.clone(), None));
    }
    let data: BlockData =
        alloy_rlp::decode_exact(&raw[..]).map_err(|e| format!("undecodable block data: {e}"))?;
    if data.staking.is_empty() && data.decryption.is_empty() {
        return Err("non-canonical empty block data".into());
    }
    let decryption = if data.decryption.is_empty() {
        None
    } else {
        Some(DecryptionData::from_bytes(&data.decryption)?)
    };
    Ok((data.staking, decryption))
}

/// Inverse of [`split_block_data`].
pub fn join_block_data(
    cfg: &ChainConfig,
    height: u64,
    staking: Bytes,
    decryption: Option<&DecryptionData>,
) -> Bytes {
    if !cfg.encrypted_mempool_active(height) {
        return staking;
    }
    let decryption = decryption
        .map(|d| Bytes::from(d.to_bytes()))
        .unwrap_or_default();
    if staking.is_empty() && decryption.is_empty() {
        return Bytes::new();
    }
    Bytes::from(alloy_rlp::encode(BlockData {
        staking,
        decryption,
    }))
}

/// Verify `data` against the decryptable envelopes `envs` of the parent and decrypt.
/// `Err` (the block is invalid): wrong envelope list, not exactly `t` shares, indices not
/// strictly increasing or unknown, or a share whose proof fails. A payload that does not
/// open or decode into a transaction for this chain within its committed gas is skipped
/// (its envelope already paid). Returns the decrypted transactions in committed order.
pub fn decrypted_candidates(
    cfg: &ChainConfig,
    key: &MempoolKey,
    envs: &[BlockEnvelope],
    data: &DecryptionData,
) -> Result<Vec<RecoveredTx>, String> {
    if data.envelopes.len() != envs.len() {
        return Err(format!(
            "decryption data for {} envelopes, the parent has {} decryptable",
            data.envelopes.len(),
            envs.len()
        ));
    }
    let t = key.threshold as usize;
    let mut out = Vec::with_capacity(envs.len());
    for (env, entry) in envs.iter().zip(&data.envelopes) {
        if entry.tx_index != env.tx_index {
            return Err(format!(
                "decryption data for tx {}, expected envelope {}",
                entry.tx_index, env.tx_index
            ));
        }
        if entry.shares.len() != t {
            return Err(format!(
                "envelope {}: {} shares, exactly {t} required",
                env.tx_index,
                entry.shares.len()
            ));
        }
        let mut shares = Vec::with_capacity(t);
        let mut last = 0u32;
        for w in &entry.shares {
            if w.index <= last {
                return Err(format!(
                    "envelope {}: share indices not strictly increasing",
                    env.tx_index
                ));
            }
            last = w.index;
            let vk = key
                .verification_key(w.index)
                .ok_or_else(|| format!("share index {} out of range", w.index))?;
            let share = w
                .to_share()
                .ok_or_else(|| format!("envelope {}: malformed share", env.tx_index))?;
            share
                .verify(vk, &env.payload.ciphertext)
                .map_err(|e| format!("envelope {}: {e}", env.tx_index))?;
            shares.push(share);
        }
        // From here on a failure is the sender's: skipped, the envelope paid its gas.
        let Ok(raw) = key.open(cfg.chain_id, env.sender, &env.payload, &shares) else {
            tracing::debug!(target: LOG, envelope = env.tx_index, "payload does not open; skipped");
            continue;
        };
        let Ok(tx) = decode_and_recover(&raw, cfg.chain_id) else {
            tracing::debug!(target: LOG, envelope = env.tx_index, "payload is not a valid transaction; skipped");
            continue;
        };
        if tx.chain_id() != Some(cfg.chain_id) || tx.gas_limit() > env.payload.inner_gas_limit {
            tracing::debug!(target: LOG, envelope = env.tx_index, "decrypted tx exceeds its committed gas or chain; skipped");
            continue;
        }
        out.push(tx);
    }
    Ok(out)
}

/// Result of adding a share message to a [`SharePool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareVerdict {
    /// At least one new verified share.
    Added,
    /// Every share was already known.
    Duplicate,
    /// For a block not committed here yet: buffered, verified once it is.
    Early,
    /// For a block too old (or skipped) to matter.
    Stale,
    /// Malformed, for another block at that height, or a proof that fails.
    Invalid,
}

struct BlockShares {
    hash: B256,
    /// Decryptable envelopes' ciphertexts by tx index.
    cts: BTreeMap<u32, Ciphertext>,
    /// tx index -> share index -> verified share.
    shares: BTreeMap<u32, BTreeMap<u32, DecryptionShare>>,
}

#[derive(Default)]
struct PoolState {
    blocks: BTreeMap<u64, BlockShares>,
    early: VecDeque<ShareMessage>,
    released: BTreeSet<u64>,
}

/// Verified decryption shares of recent committed blocks (see the module docs).
pub struct SharePool {
    key: MempoolKey,
    state: Mutex<PoolState>,
}

impl std::fmt::Debug for SharePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharePool")
            .field("key_id", &self.key.key_id)
            .finish_non_exhaustive()
    }
}

impl SharePool {
    /// An empty pool for the committee `key`.
    pub fn new(key: MempoolKey) -> Self {
        Self {
            key,
            state: Mutex::new(PoolState::default()),
        }
    }

    /// The committee key.
    pub fn key(&self) -> &MempoolKey {
        &self.key
    }

    /// `true` once block `number` with `hash` is registered.
    pub fn is_registered(&self, number: u64, hash: B256) -> bool {
        self.state
            .lock()
            .blocks
            .get(&number)
            .is_some_and(|b| b.hash == hash)
    }

    /// Register the decryptable envelopes of committed block `number` (idempotent) and
    /// apply the shares that arrived before it.
    pub fn register(&self, number: u64, hash: B256, envs: &[BlockEnvelope]) {
        let mut st = self.state.lock();
        if st.blocks.get(&number).is_some_and(|b| b.hash == hash) {
            return;
        }
        st.blocks.insert(
            number,
            BlockShares {
                hash,
                cts: envs
                    .iter()
                    .map(|e| (e.tx_index, e.payload.ciphertext.clone()))
                    .collect(),
                shares: BTreeMap::new(),
            },
        );
        while st.blocks.len() > KEPT_BLOCKS {
            st.blocks.pop_first();
        }
        let low = number.saturating_sub(KEPT_BLOCKS as u64);
        st.released.retain(|n| *n >= low);
        let pending: Vec<ShareMessage> = st.early.drain(..).collect();
        let mut now = Vec::new();
        for m in pending {
            if m.block_number == number {
                now.push(m);
            } else if m.block_number > number {
                st.early.push_back(m);
            }
        }
        for m in &now {
            Self::apply(&self.key, &mut st, m);
        }
    }

    /// Mark block `number`'s own shares as released; `false` if they already were.
    pub fn mark_released(&self, number: u64) -> bool {
        self.state.lock().released.insert(number)
    }

    /// Verify and add the shares of `msg`.
    pub fn add_message(&self, msg: &ShareMessage) -> ShareVerdict {
        let mut st = self.state.lock();
        Self::apply(&self.key, &mut st, msg)
    }

    fn apply(key: &MempoolKey, st: &mut PoolState, msg: &ShareMessage) -> ShareVerdict {
        if msg.envelopes.is_empty() || msg.envelopes.len() > MAX_ENVELOPES_PER_MESSAGE {
            return ShareVerdict::Invalid;
        }
        if !st.blocks.contains_key(&msg.block_number) {
            let newest = st.blocks.keys().next_back().copied();
            if newest.is_some_and(|n| msg.block_number <= n) {
                return ShareVerdict::Stale;
            }
            if st.early.len() >= MAX_EARLY_MESSAGES {
                st.early.pop_front();
            }
            st.early.push_back(msg.clone());
            return ShareVerdict::Early;
        }
        let Some(block) = st.blocks.get_mut(&msg.block_number) else {
            return ShareVerdict::Stale;
        };
        if block.hash != msg.block_hash {
            return ShareVerdict::Invalid;
        }
        // All or nothing: verify every share first.
        let mut verified = Vec::new();
        for e in &msg.envelopes {
            let Some(ct) = block.cts.get(&e.tx_index) else {
                return ShareVerdict::Invalid;
            };
            for w in &e.shares {
                let (Some(vk), Some(share)) = (key.verification_key(w.index), w.to_share()) else {
                    return ShareVerdict::Invalid;
                };
                if share.verify(vk, ct).is_err() {
                    return ShareVerdict::Invalid;
                }
                verified.push((e.tx_index, share));
            }
        }
        let mut added = false;
        for (tx_index, share) in verified {
            let slot = block.shares.entry(tx_index).or_default();
            if let btree_map::Entry::Vacant(v) = slot.entry(share.index) {
                v.insert(share);
                added = true;
            }
        }
        if added {
            ShareVerdict::Added
        } else {
            ShareVerdict::Duplicate
        }
    }

    /// Exactly `t` shares (lowest indices) for every decryptable envelope of block
    /// `number`, or `None` while some envelope has fewer (or the block is unknown).
    pub fn selection(&self, number: u64, hash: B256) -> Option<Vec<EnvelopeShares>> {
        let st = self.state.lock();
        let block = st.blocks.get(&number).filter(|b| b.hash == hash)?;
        let t = self.key.threshold as usize;
        let mut out = Vec::with_capacity(block.cts.len());
        for idx in block.cts.keys() {
            let shares = block.shares.get(idx)?;
            if shares.len() < t {
                return None;
            }
            out.push(EnvelopeShares {
                tx_index: *idx,
                shares: shares.values().take(t).map(WireShare::from).collect(),
            });
        }
        Some(out)
    }
}

/// Wrap the tx gossip validator: share messages go to `pool`, everything else to `inner`.
pub fn share_gossip_validator(inner: TxValidator, pool: Arc<SharePool>) -> TxValidator {
    Arc::new(move |raw: &[u8]| {
        if raw.first() != Some(&SHARE_MESSAGE_PREFIX) {
            return inner(raw);
        }
        let Some(msg) = ShareMessage::from_gossip(raw) else {
            return Verdict::Reject;
        };
        match pool.add_message(&msg) {
            ShareVerdict::Added => Verdict::Accept,
            ShareVerdict::Duplicate | ShareVerdict::Early | ShareVerdict::Stale => Verdict::Ignore,
            ShareVerdict::Invalid => {
                tracing::debug!(target: LOG, block = msg.block_number, "rejecting invalid decryption shares");
                Verdict::Reject
            }
        }
    })
}

/// The encrypted-mempool parts of a consensus node.
#[derive(Debug)]
pub struct Encryption {
    /// Verified shares of recent blocks.
    pub pool: Arc<SharePool>,
    /// This node's key share (committee members only).
    pub share: Option<KeyShare>,
    /// Where own shares are gossiped (set once the gossip swarm runs).
    pub gossip: OnceLock<Arc<NetworkHandle>>,
}

impl Encryption {
    /// Encryption state over `pool` with an optional local `share`.
    pub fn new(pool: Arc<SharePool>, share: Option<KeyShare>) -> Self {
        Self {
            pool,
            share,
            gossip: OnceLock::new(),
        }
    }

    /// Register committed block `block` and, on a committee member, release (verify,
    /// pool and gossip) its own shares once.
    pub fn on_committed(&self, cfg: &ChainConfig, block: &Block, hash: B256) {
        let number = block.number();
        if !cfg.encrypted_mempool_active(number) || self.pool.is_registered(number, hash) {
            return;
        }
        let envs = decryptable_envelopes(cfg, self.pool.key(), block);
        self.pool.register(number, hash, &envs);
        if envs.is_empty() {
            return;
        }
        let Some(share) = &self.share else {
            return;
        };
        if !self.pool.mark_released(number) {
            return;
        }
        let msg = match ShareMessage::create(share, number, hash, &envs) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(target: LOG, number, error = %e, "cannot compute decryption shares");
                return;
            }
        };
        let verdict = self.pool.add_message(&msg);
        tracing::info!(target: LOG, number, envelopes = envs.len(), index = share.index(),
                       ?verdict, "releasing decryption shares");
        if let Some(gossip) = self.gossip.get() {
            if let Err(e) = gossip.publish_tx(msg.to_gossip()) {
                tracing::warn!(target: LOG, number, error = %e, "decryption share gossip failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_crypto::{
        sign_tx_envelope, signer_from_hex,
        threshold::{deal_random, Dealt},
    };
    use xdevs_genesis::{DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
    use xdevs_primitives::{
        constants::{DEV_CHAIN_ID, ENCRYPTED_INBOX_ADDRESS, GWEI},
        Encodable2718, EncryptedMempoolParams, Header, TxEip1559, TxEnvelope, TxKind, U256,
    };

    fn setup(t: u32, n: u32) -> (ChainConfig, MempoolKey, Dealt) {
        let d = deal_random(t, n).unwrap();
        let mut cfg = ChainConfig::dev();
        cfg.encrypted_mempool_block = Some(1);
        cfg.encrypted_mempool = Some(EncryptedMempoolParams {
            threshold: t,
            public_key: Bytes::copy_from_slice(d.public_key.as_bytes()),
            verification_keys: d
                .verification_keys
                .iter()
                .map(|k| Bytes::copy_from_slice(k.as_bytes()))
                .collect(),
        });
        let key = MempoolKey::from_config(&cfg).unwrap().unwrap();
        (cfg, key, d)
    }

    fn signed(account: usize, nonce: u64, to: Address, gas: u64, input: Bytes) -> TxEnvelope {
        let signer = signer_from_hex(DEV_PRIVATE_KEYS[account]).unwrap();
        sign_tx_envelope(
            &signer,
            TxEip1559 {
                chain_id: DEV_CHAIN_ID,
                nonce,
                gas_limit: gas,
                max_fee_per_gas: 10 * u128::from(GWEI),
                max_priority_fee_per_gas: u128::from(GWEI),
                to: TxKind::Call(to),
                value: U256::from(1u64),
                input,
                access_list: Default::default(),
            },
        )
        .unwrap()
    }

    /// An envelope from dev account `account` around `inner` (EIP-2718 bytes).
    fn envelope(
        cfg: &ChainConfig,
        key: &MempoolKey,
        account: usize,
        nonce: u64,
        inner: &[u8],
        gas: u64,
    ) -> TxEnvelope {
        let payload = key.seal(cfg.chain_id, DEV_ACCOUNTS[account], inner, gas);
        signed(
            account,
            nonce,
            ENCRYPTED_INBOX_ADDRESS,
            200_000,
            payload.encode(),
        )
    }

    fn block(number: u64, body: Vec<TxEnvelope>) -> Block {
        Block::new(
            Header {
                number,
                ..Default::default()
            },
            body,
        )
    }

    fn shares_of(
        d: &Dealt,
        idx: &[usize],
        number: u64,
        hash: B256,
        envs: &[BlockEnvelope],
    ) -> Vec<ShareMessage> {
        idx.iter()
            .map(|&i| ShareMessage::create(&d.shares[i], number, hash, envs).unwrap())
            .collect()
    }

    #[test]
    fn decryptable_envelopes_skip_griefing_and_copies() {
        let (cfg, key, _) = setup(2, 3);
        let inner = signed(1, 0, Address::repeat_byte(7), 21_000, Bytes::new()).encoded_2718();
        let good = envelope(&cfg, &key, 0, 0, &inner, 21_000);
        // Garbage calldata to the inbox: pays gas, never decrypted.
        let garbage = signed(
            2,
            0,
            ENCRYPTED_INBOX_ADDRESS,
            100_000,
            Bytes::from(vec![0xab; 200]),
        );
        // A copy of account 0's payload sent by account 3: the proof binds the sender.
        let copied = signed(3, 0, ENCRYPTED_INBOX_ADDRESS, 200_000, good.input().clone());
        // Over the block's inner-gas budget.
        let greedy = envelope(&cfg, &key, 4, 0, &inner, cfg.encrypted_gas_budget() + 1);
        let plain = signed(5, 0, Address::repeat_byte(9), 21_000, Bytes::new());
        let b = block(5, vec![plain, garbage, good, copied, greedy]);
        let envs = decryptable_envelopes(&cfg, &key, &b);
        assert_eq!(envs.len(), 1);
        assert_eq!((envs[0].tx_index, envs[0].sender), (2, DEV_ACCOUNTS[0]));
        // Before the activation nothing is decryptable.
        let mut early = cfg.clone();
        early.encrypted_mempool_block = Some(6);
        assert!(decryptable_envelopes(&early, &key, &b).is_empty());
        // The budget is cumulative, in block order.
        let half = cfg.encrypted_gas_budget() / 2 + 1;
        let a = envelope(&cfg, &key, 0, 0, &inner, half);
        let c = envelope(&cfg, &key, 1, 0, &inner, half);
        let envs = decryptable_envelopes(&cfg, &key, &block(5, vec![a, c]));
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].tx_index, 0);
    }

    #[test]
    fn shares_pool_select_and_decrypt_in_committed_order() {
        let (cfg, key, d) = setup(2, 3);
        let inner0 = signed(1, 0, Address::repeat_byte(7), 21_000, Bytes::new());
        let inner1 = signed(2, 0, Address::repeat_byte(8), 21_000, Bytes::new());
        // Valid ciphertext of bytes that are not a transaction: decrypts, then skipped.
        let junk = envelope(&cfg, &key, 3, 0, b"not a transaction", 50_000);
        let b = block(
            7,
            vec![
                envelope(&cfg, &key, 0, 0, &inner1.encoded_2718(), 21_000),
                junk,
                envelope(&cfg, &key, 4, 0, &inner0.encoded_2718(), 21_000),
            ],
        );
        let hash = B256::repeat_byte(0x77);
        let envs = decryptable_envelopes(&cfg, &key, &b);
        assert_eq!(envs.len(), 3);

        let pool = SharePool::new(key.clone());
        let msgs = shares_of(&d, &[2, 0, 1], 7, hash, &envs);
        // Before the block is registered: buffered.
        assert_eq!(pool.add_message(&msgs[0]), ShareVerdict::Early);
        pool.register(7, hash, &envs);
        assert!(pool.is_registered(7, hash));
        assert_eq!(pool.selection(7, hash), None, "one share is not enough");
        assert_eq!(pool.add_message(&msgs[0]), ShareVerdict::Duplicate);
        assert_eq!(pool.add_message(&msgs[1]), ShareVerdict::Added);
        // Wrong hash, forged share: invalid.
        let mut other = msgs[2].clone();
        other.block_hash = B256::repeat_byte(1);
        assert_eq!(pool.add_message(&other), ShareVerdict::Invalid);
        let mut forged = msgs[2].clone();
        forged.envelopes[0].shares[0].index = 1;
        assert_eq!(pool.add_message(&forged), ShareVerdict::Invalid);
        assert_eq!(pool.add_message(&msgs[2]), ShareVerdict::Added);
        // Stale: an older, unregistered block.
        let old = ShareMessage {
            block_number: 3,
            ..msgs[2].clone()
        };
        assert_eq!(pool.add_message(&old), ShareVerdict::Stale);

        let selection = pool.selection(7, hash).unwrap();
        // The lowest indices (1 and 2), strictly increasing.
        for e in &selection {
            let idx: Vec<u32> = e.shares.iter().map(|s| s.index).collect();
            assert_eq!(idx, vec![1, 2]);
        }
        let data = DecryptionData {
            block_hash: hash,
            envelopes: selection,
        };
        assert_eq!(DecryptionData::from_bytes(&data.to_bytes()).unwrap(), data);
        let txs = decrypted_candidates(&cfg, &key, &envs, &data).unwrap();
        // Committed order: envelope 0 (inner1), envelope 2 (inner0); the junk is skipped.
        let hashes: Vec<B256> = txs.iter().map(|t| t.hash()).collect();
        assert_eq!(hashes, vec![*inner1.tx_hash(), *inner0.tx_hash()]);
        // Deterministic: any node with the same data gets the same list.
        let again = decrypted_candidates(&cfg, &key, &envs, &data).unwrap();
        assert_eq!(again.iter().map(|t| t.hash()).collect::<Vec<_>>(), hashes);

        // Invalid decryption data: too few shares, unordered, missing envelope, bad proof.
        let mut few = data.clone();
        few.envelopes[0].shares.pop();
        assert!(decrypted_candidates(&cfg, &key, &envs, &few).is_err());
        let mut unordered = data.clone();
        unordered.envelopes[1].shares.reverse();
        assert!(decrypted_candidates(&cfg, &key, &envs, &unordered).is_err());
        let mut missing = data.clone();
        missing.envelopes.pop();
        assert!(decrypted_candidates(&cfg, &key, &envs, &missing).is_err());
        let mut bad = data.clone();
        bad.envelopes[2].shares[0].z = B256::repeat_byte(3);
        assert!(decrypted_candidates(&cfg, &key, &envs, &bad).is_err());
        let mut swapped = data.clone();
        swapped.envelopes.swap(0, 2);
        assert!(decrypted_candidates(&cfg, &key, &envs, &swapped).is_err());
    }

    #[test]
    fn inner_tx_over_its_gas_commitment_is_skipped() {
        let (cfg, key, d) = setup(1, 1);
        let inner = signed(1, 0, Address::repeat_byte(7), 50_000, Bytes::new());
        let b = block(
            2,
            vec![envelope(&cfg, &key, 0, 0, &inner.encoded_2718(), 21_000)],
        );
        let envs = decryptable_envelopes(&cfg, &key, &b);
        let msg = ShareMessage::create(&d.shares[0], 2, B256::ZERO, &envs).unwrap();
        let data = DecryptionData {
            block_hash: B256::ZERO,
            envelopes: msg.envelopes,
        };
        assert!(decrypted_candidates(&cfg, &key, &envs, &data)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn block_data_split_join() {
        let (cfg, _, _) = setup(1, 1);
        let staking = Bytes::from_static(b"staking");
        let data = DecryptionData {
            block_hash: B256::repeat_byte(2),
            envelopes: vec![],
        };
        // Before the activation: passthrough.
        let mut off = cfg.clone();
        off.encrypted_mempool_block = None;
        assert_eq!(
            join_block_data(&off, 5, staking.clone(), Some(&data)),
            staking
        );
        assert_eq!(
            split_block_data(&off, 5, &staking).unwrap(),
            (staking.clone(), None)
        );
        // Active: wrapped, both parts optional, never both empty.
        for (s, dd) in [
            (staking.clone(), Some(&data)),
            (Bytes::new(), Some(&data)),
            (staking.clone(), None),
        ] {
            let raw = join_block_data(&cfg, 5, s.clone(), dd);
            assert!(!raw.is_empty());
            assert_eq!(split_block_data(&cfg, 5, &raw).unwrap(), (s, dd.cloned()));
        }
        assert!(join_block_data(&cfg, 5, Bytes::new(), None).is_empty());
        assert_eq!(
            split_block_data(&cfg, 5, &Bytes::new()).unwrap(),
            (Bytes::new(), None)
        );
        let empty = Bytes::from(alloy_rlp::encode(BlockData {
            staking: Bytes::new(),
            decryption: Bytes::new(),
        }));
        assert!(split_block_data(&cfg, 5, &empty).is_err());
        assert!(split_block_data(&cfg, 5, &staking).is_err());
    }

    #[test]
    fn gossip_dispatch() {
        let (cfg, key, d) = setup(1, 1);
        let inner = signed(1, 0, Address::repeat_byte(7), 21_000, Bytes::new());
        let b = block(
            3,
            vec![envelope(&cfg, &key, 0, 0, &inner.encoded_2718(), 21_000)],
        );
        let envs = decryptable_envelopes(&cfg, &key, &b);
        let pool = Arc::new(SharePool::new(key));
        pool.register(3, B256::ZERO, &envs);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = calls.clone();
        let inner_validator: TxValidator = Arc::new(move |_raw: &[u8]| {
            c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Verdict::Accept
        });
        let v = share_gossip_validator(inner_validator, pool.clone());
        let msg = ShareMessage::create(&d.shares[0], 3, B256::ZERO, &envs).unwrap();
        let raw = msg.to_gossip();
        assert_eq!(ShareMessage::from_gossip(&raw).unwrap(), msg);
        assert_eq!(v(&raw[..]), Verdict::Accept);
        assert_eq!(v(&raw[..]), Verdict::Ignore);
        assert_eq!(v(&[SHARE_MESSAGE_PREFIX, 0xff][..]), Verdict::Reject);
        assert_eq!(v(&inner.encoded_2718()[..]), Verdict::Accept);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(pool.selection(3, B256::ZERO).is_some());
    }
}
