//! Incremental Ethereum Merkle-Patricia tries persisted in the `TrieNodes` table
//! (plan §14.3, ADR-0004).
//!
//! ## Layout
//!
//! Every node of the account trie and of every non-empty storage trie is stored in
//! [`tables::TrieNodes`] under its nibble path (see [`account_trie_key`] /
//! [`storage_trie_key`]); the value is the node's RLP. Embedded (< 32 byte) nodes are stored
//! as well, so one node is always one entry. The account-trie root entry is always present
//! (`0x80` for the empty trie): its presence marks the tries as built. An empty storage trie
//! has no entries at all.
//!
//! ## Update
//!
//! A block's [`StateDiff`] is applied by walking down only the paths of the changed keys:
//! the nodes on those paths are loaded (each one checked against the reference its parent
//! holds), modified in memory, and re-encoded bottom-up, while untouched subtrees stay
//! represented by their stored references. Every node that was loaded is deleted and every
//! node of the rebuilt part is written again, so after an update the table holds exactly
//! the nodes of the new canonical trie, byte for byte what a fresh build from the flat
//! state produces. Cost: O(changed keys × depth) node reads and hashes instead of O(state).
//!
//! The writes go into the caller's [`WriteBatch`] ([`write_trie_updates`], called by
//! [`crate::write_state_diff`]), i.e. into the same atomic commit as the block. A
//! database written before the tries existed is migrated by the first such call, which
//! builds them from the flat state in that same batch; until then
//! [`state_root_with`] falls back to the full recompute ([`root::full_db_state_root`]).

use crate::{root, StateDiff, StateError};
use alloy_rlp::Decodable;
use alloy_trie::{
    nodes::{BranchNodeRef, ExtensionNodeRef, LeafNodeRef, RlpNode, TrieNode},
    Nibbles, TrieMask,
};
use std::collections::{BTreeMap, BTreeSet};
use xdevs_primitives::{
    keccak256, reexports::alloy_trie::TrieAccount, AccountInfo, Address, B256, EMPTY_ROOT_HASH,
};
use xdevs_storage::{
    tables::{self, account_trie_key, storage_trie_key, storage_trie_prefix},
    DbRead, DbReadExt, TableId, WriteBatch,
};

/// RLP of the empty string: the encoding of the empty trie.
const EMPTY_STRING_CODE: u8 = 0x80;
/// Length of a storage-trie key prefix (tag + hashed address).
const STORAGE_PREFIX_LEN: usize = 33;

fn inconsistent(msg: impl Into<String>) -> StateError {
    StateError::Inconsistent(msg.into())
}

/// Bytes to nibbles, one nibble per byte.
fn nibbles(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|b| [b >> 4, b & 0x0f]).collect()
}

/// `true` once the state tries have been built in `db` (the account-trie root exists).
pub fn state_trie_exists<D: DbRead + ?Sized>(db: &D) -> Result<bool, StateError> {
    Ok(db
        .get_raw(TableId::TrieNodes, &account_trie_key(&[]))?
        .is_some())
}

/// Ethereum MPT state root of the latest state in `db` with `diff` applied on top, without
/// writing anything. Incremental over the stored tries; a database whose tries have not
/// been built yet is answered with the full recompute.
pub fn state_root_with<D: DbRead + ?Sized>(db: &D, diff: &StateDiff) -> Result<B256, StateError> {
    if !state_trie_exists(db)? {
        return Ok(root::full_db_state_root(db, diff)?);
    }
    apply_diff(&mut NodeOverlay::new(db), diff)
}

/// Append to `batch` the trie-node writes that move the stored tries from the state in
/// `db` to that state with `diff` applied, and return the new state root. Builds the
/// tries from the flat state first (in the same batch) if the database has none yet.
///
/// `db` must be the latest committed state that `batch` is applied on top of.
pub fn write_trie_updates<D: DbRead + ?Sized>(
    batch: &mut WriteBatch,
    db: &D,
    diff: &StateDiff,
) -> Result<B256, StateError> {
    let mut store = NodeOverlay::new(db);
    if !state_trie_exists(db)? {
        build_from_flat(&mut store)?;
    }
    let root = apply_diff(&mut store, diff)?;
    store.write_to(batch);
    Ok(root)
}

/// Full consistency check (node start-up): recompute the state root from the flat tables
/// and, if the tries exist, compare it with the stored trie root. Returns the root; a
/// mismatch is [`StateError::Inconsistent`]. O(state).
pub fn check_state_root<D: DbRead + ?Sized>(db: &D) -> Result<B256, StateError> {
    let full = root::full_db_state_root(db, &StateDiff::new())?;
    if state_trie_exists(db)? {
        let trie = state_root_with(db, &StateDiff::new())?;
        if trie != full {
            return Err(inconsistent(format!(
                "state trie root {trie} does not match flat state root {full}"
            )));
        }
    }
    Ok(full)
}

/// Build the whole state trie from the flat state into a batch (migration helper; the
/// result is identical to what [`write_trie_updates`] maintains incrementally). Any
/// existing trie nodes are deleted first. Returns the state root.
pub fn rebuild_state_trie<D: DbRead + ?Sized>(
    batch: &mut WriteBatch,
    db: &D,
) -> Result<B256, StateError> {
    let mut store = NodeOverlay::new(db);
    let root = build_from_flat(&mut store)?;
    store.write_to(batch);
    Ok(root)
}

/// Trie nodes as stored in the database, with pending changes on top.
pub(crate) struct NodeOverlay<'a, D: ?Sized> {
    db: &'a D,
    /// Every stored node is deleted (full rebuild).
    wipe_all: bool,
    /// Storage-trie prefixes deleted as a whole (wiped / destroyed accounts).
    wiped: BTreeSet<Vec<u8>>,
    /// Pending writes: `Some` = put, `None` = delete.
    nodes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

impl<'a, D: DbRead + ?Sized> NodeOverlay<'a, D> {
    pub(crate) fn new(db: &'a D) -> Self {
        Self {
            db,
            wipe_all: false,
            wiped: BTreeSet::new(),
            nodes: BTreeMap::new(),
        }
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StateError> {
        if let Some(v) = self.nodes.get(key) {
            return Ok(v.clone());
        }
        if self.wipe_all
            || key
                .get(..STORAGE_PREFIX_LEN)
                .is_some_and(|p| self.wiped.contains(p))
        {
            return Ok(None);
        }
        Ok(self.db.get_raw(TableId::TrieNodes, key)?)
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.nodes.insert(key, Some(value));
    }

    fn delete(&mut self, key: Vec<u8>) {
        self.nodes.insert(key, None);
    }

    fn wipe_storage(&mut self, hashed_address: B256) {
        let prefix = storage_trie_prefix(hashed_address);
        self.nodes.retain(|k, _| !k.starts_with(&prefix));
        self.wiped.insert(prefix);
    }

    fn wipe_everything(&mut self) {
        self.nodes.clear();
        self.wiped.clear();
        self.wipe_all = true;
    }

    /// Append the pending changes to `batch` (prefix deletes first, then single keys).
    pub(crate) fn write_to(self, batch: &mut WriteBatch) {
        if self.wipe_all {
            batch.delete_prefix(TableId::TrieNodes, Vec::new());
        }
        for prefix in self.wiped {
            batch.delete_prefix(TableId::TrieNodes, prefix);
        }
        for (key, value) in self.nodes {
            match value {
                Some(value) => batch.put_raw(TableId::TrieNodes, key, value),
                None => batch.delete_raw(TableId::TrieNodes, key),
            };
        }
    }
}

/// In-memory trie node; `Hash` is a stored node that has not been loaded.
#[derive(Debug, Default)]
enum Node {
    #[default]
    Empty,
    /// Reference (hash or embedded RLP) of the unloaded stored node at this position.
    Hash(RlpNode),
    /// Remaining key nibbles and raw value.
    Leaf(Vec<u8>, Vec<u8>),
    /// Shared key nibbles and child.
    Ext(Vec<u8>, Box<Node>),
    Branch(Box<[Node; 16]>),
}

fn decode_node(bytes: &[u8]) -> Result<Node, StateError> {
    let mut buf = bytes;
    let node = TrieNode::decode(&mut buf).map_err(|e| inconsistent(format!("trie node: {e}")))?;
    if !buf.is_empty() {
        return Err(inconsistent("trie node: trailing bytes"));
    }
    Ok(match node {
        TrieNode::EmptyRoot => Node::Empty,
        TrieNode::Leaf(leaf) => Node::Leaf(leaf.key.to_vec(), leaf.value),
        TrieNode::Extension(ext) => Node::Ext(ext.key.to_vec(), Box::new(Node::Hash(ext.child))),
        TrieNode::Branch(branch) => {
            let mut children: Box<[Node; 16]> = Box::default();
            let mut stack = branch.stack.into_iter();
            for (i, child) in children.iter_mut().enumerate() {
                if branch.state_mask.is_bit_set(i as u8) {
                    *child = Node::Hash(
                        stack
                            .next()
                            .ok_or_else(|| inconsistent("trie node: branch mask"))?,
                    );
                }
            }
            Node::Branch(children)
        }
    })
}

/// `prefix` followed by `path`.
fn node_key(prefix: &[u8], path: &[u8]) -> Vec<u8> {
    [prefix, path].concat()
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// `prefix ++ child` as a canonical node (no extension above a leaf or extension).
fn join(prefix: Vec<u8>, child: Node) -> Node {
    match child {
        Node::Empty => Node::Empty,
        Node::Leaf(key, value) => Node::Leaf([prefix, key].concat(), value),
        Node::Ext(key, grandchild) => Node::Ext([prefix, key].concat(), grandchild),
        node if prefix.is_empty() => node,
        node => Node::Ext(prefix, Box::new(node)),
    }
}

/// Where a trie lives and which of its stored nodes were loaded.
struct Ctx {
    prefix: Vec<u8>,
    loaded: BTreeSet<Vec<u8>>,
}

impl Ctx {
    /// Load the stored node at `path`, checking it against its parent's reference.
    fn load<D: DbRead + ?Sized>(
        &mut self,
        store: &NodeOverlay<'_, D>,
        path: &[u8],
        expected: &RlpNode,
    ) -> Result<Node, StateError> {
        let bytes = store.get(&node_key(&self.prefix, path))?.ok_or_else(|| {
            inconsistent(format!(
                "missing trie node at {:02x?}{path:x?}",
                self.prefix
            ))
        })?;
        if RlpNode::from_rlp(&bytes) != *expected {
            return Err(inconsistent(format!(
                "trie node at {:02x?}{path:x?} does not match its parent",
                self.prefix
            )));
        }
        self.loaded.insert(path.to_vec());
        decode_node(&bytes)
    }

    fn resolve<D: DbRead + ?Sized>(
        &mut self,
        store: &NodeOverlay<'_, D>,
        node: &mut Node,
        path: &[u8],
    ) -> Result<(), StateError> {
        if let Node::Hash(reference) = node {
            let reference = reference.clone();
            *node = self.load(store, path, &reference)?;
        }
        Ok(())
    }

    fn insert<D: DbRead + ?Sized>(
        &mut self,
        store: &NodeOverlay<'_, D>,
        node: &mut Node,
        path: &mut Vec<u8>,
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), StateError> {
        self.resolve(store, node, path)?;
        match node {
            Node::Empty => *node = Node::Leaf(key.to_vec(), value),
            Node::Leaf(k, v) => {
                if k.as_slice() == key {
                    *v = value;
                    return Ok(());
                }
                let c = common_prefix(k, key);
                if c >= k.len() || c >= key.len() {
                    return Err(inconsistent("trie keys of different lengths"));
                }
                let mut children: Box<[Node; 16]> = Box::default();
                children[k[c] as usize] = Node::Leaf(k[c + 1..].to_vec(), std::mem::take(v));
                children[key[c] as usize] = Node::Leaf(key[c + 1..].to_vec(), value);
                *node = join(key[..c].to_vec(), Node::Branch(children));
            }
            Node::Ext(k, child) => {
                let c = common_prefix(k, key);
                if c == k.len() {
                    let depth = path.len();
                    path.extend_from_slice(k);
                    let res = self.insert(store, child, path, &key[c..], value);
                    path.truncate(depth);
                    return res;
                }
                if c >= key.len() {
                    return Err(inconsistent("trie keys of different lengths"));
                }
                // Split the extension: the old child keeps its absolute position.
                let mut children: Box<[Node; 16]> = Box::default();
                let old = std::mem::take(child.as_mut());
                children[k[c] as usize] = join(k[c + 1..].to_vec(), old);
                children[key[c] as usize] = Node::Leaf(key[c + 1..].to_vec(), value);
                *node = join(k[..c].to_vec(), Node::Branch(children));
            }
            Node::Branch(children) => {
                let Some((&first, rest)) = key.split_first() else {
                    return Err(inconsistent("trie keys of different lengths"));
                };
                path.push(first);
                let res = self.insert(store, &mut children[first as usize], path, rest, value);
                path.pop();
                return res;
            }
            Node::Hash(_) => unreachable!("resolved above"),
        }
        Ok(())
    }

    fn remove<D: DbRead + ?Sized>(
        &mut self,
        store: &NodeOverlay<'_, D>,
        node: &mut Node,
        path: &mut Vec<u8>,
        key: &[u8],
    ) -> Result<(), StateError> {
        self.resolve(store, node, path)?;
        match node {
            Node::Empty => {}
            Node::Leaf(k, _) => {
                if k.as_slice() == key {
                    *node = Node::Empty;
                }
            }
            Node::Ext(k, child) => {
                if !key.starts_with(k) {
                    return Ok(());
                }
                let depth = path.len();
                path.extend_from_slice(k);
                let res = self.remove(store, child, path, &key[k.len()..]);
                path.truncate(depth);
                res?;
                let (k, child) = (std::mem::take(k), std::mem::take(child.as_mut()));
                *node = join(k, child);
            }
            Node::Branch(children) => {
                let Some((&first, rest)) = key.split_first() else {
                    return Ok(());
                };
                path.push(first);
                let res = self.remove(store, &mut children[first as usize], path, rest);
                path.pop();
                res?;
                let mut live = children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !matches!(c, Node::Empty))
                    .map(|(i, _)| i);
                match (live.next(), live.next()) {
                    (None, _) => *node = Node::Empty,
                    (Some(only), None) => {
                        // Collapse: the single remaining child moves up into this position
                        // (load it to know whether it is a leaf, extension or branch).
                        let mut child = std::mem::take(&mut children[only]);
                        path.push(only as u8);
                        let res = self.resolve(store, &mut child, path);
                        path.pop();
                        res?;
                        *node = join(vec![only as u8], child);
                    }
                    _ => {}
                }
            }
            Node::Hash(_) => unreachable!("resolved above"),
        }
        Ok(())
    }
}

/// Encode `node` (at `path`) and its modified descendants, store them, return its reference.
fn encode<D: DbRead + ?Sized>(
    store: &mut NodeOverlay<'_, D>,
    prefix: &[u8],
    node: &Node,
    path: &mut Vec<u8>,
) -> RlpNode {
    let mut out = Vec::new();
    let reference = match node {
        Node::Hash(reference) => return reference.clone(),
        Node::Empty => {
            out.push(EMPTY_STRING_CODE);
            RlpNode::from_rlp(&out)
        }
        Node::Leaf(key, value) => {
            LeafNodeRef::new(&Nibbles::from_nibbles_unchecked(key), value).rlp(&mut out)
        }
        Node::Ext(key, child) => {
            let depth = path.len();
            path.extend_from_slice(key);
            let child = encode(store, prefix, child, path);
            path.truncate(depth);
            ExtensionNodeRef::new(&Nibbles::from_nibbles_unchecked(key), &child).rlp(&mut out)
        }
        Node::Branch(children) => {
            let mut stack = Vec::with_capacity(16);
            let mut mask = TrieMask::default();
            for (i, child) in children.iter().enumerate() {
                if !matches!(child, Node::Empty) {
                    path.push(i as u8);
                    stack.push(encode(store, prefix, child, path));
                    path.pop();
                    mask.set_bit(i as u8);
                }
            }
            BranchNodeRef::new(&stack, mask).rlp(&mut out)
        }
    };
    store.put(node_key(prefix, path), out);
    reference
}

/// One Merkle-Patricia trie over fixed-length keys, stored under `prefix` in `TrieNodes`.
pub(crate) struct Trie {
    root: Node,
    ctx: Ctx,
}

impl Trie {
    /// A new, empty trie (nothing is read from the store).
    pub(crate) fn empty(prefix: Vec<u8>) -> Self {
        Self {
            root: Node::Empty,
            ctx: Ctx {
                prefix,
                loaded: BTreeSet::new(),
            },
        }
    }

    /// The trie stored under `prefix` (missing root = empty trie).
    pub(crate) fn open<D: DbRead + ?Sized>(
        store: &NodeOverlay<'_, D>,
        prefix: Vec<u8>,
    ) -> Result<Self, StateError> {
        let mut trie = Self::empty(prefix);
        if let Some(bytes) = store.get(&trie.ctx.prefix)? {
            trie.root = decode_node(&bytes)?;
            trie.ctx.loaded.insert(Vec::new());
        }
        Ok(trie)
    }

    /// Insert or replace `key` (nibbles) with the raw leaf `value` (must not be empty).
    pub(crate) fn insert<D: DbRead + ?Sized>(
        &mut self,
        store: &NodeOverlay<'_, D>,
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), StateError> {
        self.ctx
            .insert(store, &mut self.root, &mut Vec::new(), key, value)
    }

    /// Remove `key` (nibbles); no-op if absent.
    pub(crate) fn remove<D: DbRead + ?Sized>(
        &mut self,
        store: &NodeOverlay<'_, D>,
        key: &[u8],
    ) -> Result<(), StateError> {
        self.ctx.remove(store, &mut self.root, &mut Vec::new(), key)
    }

    /// Write the changes into `store` and return the root hash. An empty trie leaves no
    /// entries unless `keep_empty_root` (then its root is stored as `0x80`).
    pub(crate) fn commit<D: DbRead + ?Sized>(
        self,
        store: &mut NodeOverlay<'_, D>,
        keep_empty_root: bool,
    ) -> B256 {
        let Self { root, ctx } = self;
        for path in &ctx.loaded {
            store.delete(node_key(&ctx.prefix, path));
        }
        if matches!(root, Node::Empty) {
            if keep_empty_root {
                store.put(ctx.prefix, vec![EMPTY_STRING_CODE]);
            }
            return EMPTY_ROOT_HASH;
        }
        let reference = encode(store, &ctx.prefix, &root, &mut Vec::new());
        // The root is always hashed, even when its encoding is shorter than 32 bytes.
        reference
            .as_hash()
            .unwrap_or_else(|| keccak256(&reference[..]))
    }
}

/// Leaf value of an account: `RLP([nonce, balance, storage_root, code_hash])`.
fn account_leaf(info: &AccountInfo, storage_root: B256) -> Vec<u8> {
    alloy_rlp::encode(TrieAccount::from(info.with_storage_root(storage_root)))
}

/// Root of the stored storage trie of `hashed_address`.
fn stored_storage_root<D: DbRead + ?Sized>(
    store: &NodeOverlay<'_, D>,
    hashed_address: B256,
) -> Result<B256, StateError> {
    Ok(store
        .get(&storage_trie_key(hashed_address, &[]))?
        .map_or(EMPTY_ROOT_HASH, keccak256))
}

/// Apply `diff` (semantics of [`StateDiff`]) to the tries in `store`; returns the new root.
///
/// Storage tries mirror `PlainStorage` exactly (including storage of addresses without an
/// account); only existing accounts reference theirs, as in the full recompute.
fn apply_diff<D: DbRead + ?Sized>(
    store: &mut NodeOverlay<'_, D>,
    diff: &StateDiff,
) -> Result<B256, StateError> {
    let mut accounts = Trie::open(store, account_trie_key(&[]))?;
    let touched: BTreeSet<Address> = diff
        .accounts
        .keys()
        .chain(diff.storage.keys())
        .copied()
        .collect();
    for address in touched {
        let hashed = keccak256(address);
        let change = diff.accounts.get(&address);
        let storage_root = if matches!(change, Some(None)) {
            store.wipe_storage(hashed);
            EMPTY_ROOT_HASH
        } else if let Some(sd) = diff.storage.get(&address) {
            if sd.wiped {
                store.wipe_storage(hashed);
            }
            let mut trie = Trie::open(store, storage_trie_prefix(hashed))?;
            for (slot, value) in &sd.slots {
                let key = nibbles(keccak256(slot).as_slice());
                if value.is_zero() {
                    trie.remove(store, &key)?;
                } else {
                    trie.insert(store, &key, alloy_rlp::encode(value))?;
                }
            }
            trie.commit(store, false)
        } else {
            stored_storage_root(store, hashed)?
        };
        let info = match change {
            Some(change) => *change,
            None => store.db.get::<tables::PlainAccounts>(&address)?,
        };
        let key = nibbles(hashed.as_slice());
        match info {
            Some(info) => accounts.insert(store, &key, account_leaf(&info, storage_root))?,
            None => accounts.remove(store, &key)?,
        }
    }
    Ok(accounts.commit(store, true))
}

/// Replace every stored node by the tries of the flat state in `store.db`; returns the root.
fn build_from_flat<D: DbRead + ?Sized>(store: &mut NodeOverlay<'_, D>) -> Result<B256, StateError> {
    store.wipe_everything();
    let db = store.db;
    let mut storage_roots = BTreeMap::new();
    let mut current: Option<(Address, Trie)> = None;
    let mut failed = None;
    db.walk::<tables::PlainStorage>(&(Address::ZERO, B256::ZERO), |(address, slot), value| {
        if current.as_ref().is_none_or(|(a, _)| *a != address) {
            if let Some((a, trie)) = current.take() {
                storage_roots.insert(a, trie.commit(store, false));
            }
            current = Some((
                address,
                Trie::empty(storage_trie_prefix(keccak256(address))),
            ));
        }
        let (_, trie) = current.as_mut().expect("set above");
        let key = nibbles(keccak256(slot).as_slice());
        // PlainStorage never holds zero values; skip defensively like the full recompute.
        if !value.is_zero() {
            if let Err(e) = trie.insert(store, &key, alloy_rlp::encode(value)) {
                failed = Some(e);
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    if let Some(e) = failed {
        return Err(e);
    }
    if let Some((a, trie)) = current.take() {
        storage_roots.insert(a, trie.commit(store, false));
    }

    let mut accounts = Trie::empty(account_trie_key(&[]));
    db.walk::<tables::PlainAccounts>(&Address::ZERO, |address, info| {
        let storage_root = storage_roots
            .get(&address)
            .copied()
            .unwrap_or(EMPTY_ROOT_HASH);
        let key = nibbles(keccak256(address).as_slice());
        match accounts.insert(store, &key, account_leaf(&info, storage_root)) {
            Ok(()) => Ok(true),
            Err(e) => {
                failed = Some(e);
                Ok(false)
            }
        }
    })?;
    if let Some(e) = failed {
        return Err(e);
    }
    Ok(accounts.commit(store, true))
}

#[cfg(test)]
mod tests;
