# xDEVS Chain — Phase 1 shared interfaces

Read this first. It is the contract between the Phase-1 crates (plan §27–§28).
Rules: determinism (§10.6: no floats, `BTreeMap` not `HashMap` in consensus paths, no
system time except the header timestamp), byte-exact Ethereum compatibility, no
`unsafe`, unit tests for everything. Change a shared interface only additively and
report it.

## Workspace

| Path | Package | Owner role | Status |
|---|---|---|---|
| `crates/primitives` | `xdevs-primitives` | scaffold | complete |
| `crates/crypto` | `xdevs-crypto` | crypto/cli agent | complete (recovery, signing, keystores) |
| `crates/storage` | `xdevs-storage` | storage agent | interface + `MemDb` complete; `RedbDatabase` = `todo!()` |
| `crates/state` | `xdevs-state` | state agent | complete (simple full-recompute root; may be optimized) |
| `crates/execution` | `xdevs-execution` | execution agent | interface + `next_base_fee` complete; `EvmExecutor` = `todo!()` |
| `crates/genesis` | `xdevs-genesis` | genesis agent | complete (extend: predeploys, dev accounts) |
| `crates/mempool` | `xdevs-mempool` | mempool agent | complete (`Pool`) |
| `crates/rpc` | `xdevs-rpc` | rpc agent | interface + error mapping; server + `convert` = `todo!()` |
| `crates/node` | `xdevs-node` | node agent | `NodeConfig` complete; `Node::run` = `todo!()` |
| `bin/xdevs` | `xdevs` | node agent | clap skeleton (`xdevs node --dev`) |
| `bin/xdevs-cli` | `xdevs-cli` | crypto/cli agent | complete (`keygen`, `address`, `send`, `balance`) |

Dependency direction (arrows = "is used by"):

```
primitives ─┬─> crypto
            └─> storage ─> state ─> execution ─┬─> mempool* ─┐
                                    state ─────┴─> genesis ──┴─> rpc** ─> node ─> bin/xdevs
* mempool depends on primitives + state (may add execution for intrinsic gas).
** rpc depends only on primitives + alloy-rpc-types-eth; node implements RpcBackend.
```

Build notes: shared target dir `C:/Users/hp/.xdevs-target` (from `.cargo/config.toml`),
never `--release`, never `cargo clean`. Scope commands: `cargo test -p xdevs-state`.
`cargo test --workspace` panics only if a test hits a `todo!()`.

## Resolved dependency versions (workspace `Cargo.toml`)

alloy-primitives 1.7.3 · alloy-rlp 0.3.17 · alloy-consensus / alloy-eips /
alloy-rpc-types-eth / alloy-serde / alloy-signer / alloy-signer-local / alloy-network 2.5.0 ·
alloy-trie 0.9.9 · revm 43.0.3 (`default-features = false`: std + `optional_*` call
flags; no c-kzg/blst/secp256k1-sys) · tokio 1.54.1 · jsonrpsee 0.26.1 (server, macros;
`http-client` in dev-deps) · async-trait 0.1.92 · redb 4.5.0 · k256 0.13.4 · p256 0.13.2 ·
ed25519-dalek 2.2.0 · serde 1.0.229 · serde_json 1.0.154 · toml 1.2.1 · thiserror 2.0.21 ·
anyhow 1.0.104 · tracing 0.1.44 · tracing-subscriber 0.3.23 · clap 4.6.7 · parking_lot 0.13.0 ·
proptest 1.12.0 · tempfile 3.27.0. (k256/p256 stay on 0.13 because alloy/revm use 0.13.)

**APIs of alloy/revm change between versions — read the sources in
`~/.cargo/registry/src/*/<crate>-<version>/` before using an API.**

---

## xdevs-primitives

Re-exports (use these, do not depend on alloy directly unless you need more):
`Address, B256, B64, U256, Bytes, Bloom, Log, LogData, TxHash, TxKind, BlockHash,
BlockNumber, ChainId, StorageKey, StorageValue, Signature, Sealed, Sealable, Header,
TxEnvelope, TxLegacy, TxEip2930, TxEip1559, TxEip7702, TxType, Signed, Transaction (trait),
SignerRecoverable, Receipt, ReceiptEnvelope, ReceiptWithBloom, TxReceipt,
Encodable2718, Decodable2718, Typed2718, EMPTY_ROOT_HASH, EMPTY_OMMER_ROOT_HASH,
KECCAK256_EMPTY, keccak256, logs_bloom, address!, b256!, hex`;
`reexports::{alloy_consensus, alloy_eips, alloy_primitives, alloy_rlp, alloy_trie}`.

```rust
pub type SealedHeader = Sealed<Header>;

pub struct Block { pub header: Header, pub body: Vec<TxEnvelope> }   // Serialize/Deserialize
impl Block {
    fn new(header, body) -> Self; fn hash(&self) -> B256 /* header.hash_slow() */;
    fn number(&self) -> u64; fn seal_header(&self) -> SealedHeader;
    fn tx_hashes(&self) -> impl Iterator<Item = TxHash>; fn compute_transactions_root(&self) -> B256;
}
pub fn compute_transactions_root(txs: &[TxEnvelope]) -> B256;
pub fn compute_receipts_root(receipts: &[ReceiptEnvelope]) -> B256;
pub fn compute_logs_bloom(receipts: &[ReceiptEnvelope]) -> Bloom;
/// difficulty=0, nonce=0, ommers=empty, withdrawals_root=Some(EMPTY_ROOT_HASH),
/// blob_gas_used=Some(0), excess_blob_gas=Some(0), parent_beacon_block_root=Some(ZERO),
/// requests_hash=Some(EMPTY_REQUESTS_HASH). EVERY header producer must call this.
pub fn set_compat_header_fields(header: &mut Header);

pub struct RecoveredTx { pub tx: TxEnvelope, pub sender: Address }  // Deref<Target = TxEnvelope>
impl RecoveredTx {
    fn recover(tx: TxEnvelope) -> Result<Self, RecoveryError>;  // k256, EIP-2 low-s
    const fn new_unchecked(tx, sender) -> Self; fn hash(&self) -> TxHash;
    fn tx_type_byte(&self) -> u8; fn encoded_len(&self) -> usize /* EIP-2718 len */;
    fn effective_tip(&self, base_fee: u64) -> Option<u128>;
}
pub struct TxLocation { pub block_number: u64, pub index: u64 }    // RLP + serde

/// Flat-state account (PlainAccounts value, StateProvider::basic_account). No storage root.
pub struct AccountInfo { pub nonce: u64, pub balance: U256, pub code_hash: B256 } // RLP + serde
//  Default = {0, 0, KECCAK256_EMPTY}; new(nonce, balance); has_code(); is_empty() (EIP-161);
//  with_storage_root(root) -> Account
/// Full Ethereum account (state trie value), From/Into alloy_trie::TrieAccount.
pub struct Account { pub nonce: u64, pub balance: U256, pub code_hash: B256, pub storage_root: B256 }

pub struct BaseFeeParams { pub initial: u64, pub minimum: u64, pub elasticity: u64,
                           pub max_change_denominator: u64, pub burn_bps: u16 }
pub struct ChainConfig { pub chain_id: u64, pub block_gas_limit: u64, pub block_time_ms: u64,
                         pub base_fee: BaseFeeParams, pub treasury: Address, pub system_address: Address }
//  serde camelCase = genesis `config` object (unknown keys ignored, numbers may be strings);
//  ChainConfig::new(chain_id), ::dev() (chain id 7117), split_base_fee(amount) -> (burned, treasury)
```

`constants`: `DEV_CHAIN_ID = 7117` (**local dev only**, not registered), `GWEI`,
`DEFAULT_BLOCK_GAS_LIMIT = 30_000_000`, `DEFAULT_BLOCK_TIME_MS = 2000`,
`INITIAL_BASE_FEE = 1 gwei`, `MIN_BASE_FEE = 0.1 gwei`, `BASE_FEE_ELASTICITY = 2`,
`BASE_FEE_MAX_CHANGE_DENOMINATOR = 8`, `BURN_BPS = 8000`, `BPS_DENOMINATOR = 10_000`,
`MAX_TX_SIZE = 128 KiB`, `MAX_EXTRA_DATA_SIZE = 32`, `STAKING_ADDRESS = 0x…1000`,
`GOVERNANCE_ADDRESS = 0x…1001`, `TREASURY_ADDRESS = 0x…1002`, `FEE_CONFIG_ADDRESS = 0x…1003`,
`SYSTEM_ADDRESS = 0xffff…fffe`, `EMPTY_REQUESTS_HASH = sha256("")`, `CLIENT_VERSION = "xdevs/v0.1.0"`.

`serde_helpers::{u64_flex, u64_hex_flex, u256_flex}` — accept integers, decimal strings and
`0x` hex strings (for genesis / config files).

## xdevs-storage

```rust
pub enum TableId { Headers, HeaderNumbers, Bodies, Transactions, Receipts,
                   PlainAccounts, PlainStorage, Bytecodes, Meta }   // ALL, name()
pub trait Table { const ID: TableId; type Key: Encode + Decode; type Value: Encode + Decode; }
pub mod tables {  // typed tables (§14.1)
  Headers:       u64            -> Header            (RLP)
  HeaderNumbers: B256           -> u64
  Bodies:        u64            -> Vec<TxEnvelope>   (RLP list, network encoding)
  Transactions:  B256 (tx hash) -> TxLocation        (tx itself lives in Bodies)
  Receipts:      (u64, u64)     -> ReceiptEnvelope   (EIP-2718)
  PlainAccounts: Address        -> AccountInfo       (RLP)
  PlainStorage:  (Address, B256)-> U256              (32 B BE; zero = delete, never stored)
  Bytecodes:     B256           -> Bytes
  Meta:          String         -> Vec<u8>           (keys in meta_keys::{LATEST_BLOCK,
                                                      CHAIN_CONFIG, GENESIS_HASH, SCHEMA_VERSION})
}
// Integer keys are big-endian: byte order == numeric order. (Address,B256) = 20+32 bytes,
// so an address is a prefix of all its storage keys.

pub trait DbRead {
    fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError>;
    fn walk_raw(&self, table: TableId, start: &[u8],
                visit: &mut dyn FnMut(&[u8], &[u8]) -> bool) -> Result<(), DbError>; // ascending, key >= start
}
pub trait Database: DbRead + Send + Sync + 'static {
    fn snapshot(&self) -> Result<Box<dyn DbRead + '_>, DbError>;   // consistent view
    fn write(&self, batch: WriteBatch) -> Result<(), DbError>;     // atomic, in order
}
pub trait DbReadExt: DbRead {   // blanket for every DbRead incl. dyn / &T / Box / Arc
    fn get<T: Table>(&self, key: &T::Key) -> Result<Option<T::Value>, DbError>;
    fn walk<T: Table>(&self, start: &T::Key, visit: impl FnMut(T::Key, T::Value) -> Result<bool, DbError>) -> Result<(), DbError>;
    fn collect_prefix<T: Table>(&self, prefix: &[u8]) -> Result<Vec<(T::Key, T::Value)>, DbError>;
}
pub struct WriteBatch;  // put::<T>(&k, &v), delete::<T>(&k), put_raw, delete_raw,
                        // delete_prefix(table, prefix), extend(other), ops(), len()
pub enum WriteOp { Put{table,key,value}, Delete{table,key}, DeletePrefix{table,prefix} }
pub enum DbError { Backend(String), Decode(String), SchemaMismatch{found, expected} }
pub struct MemDb;                    // complete in-memory Database (tests)
pub struct RedbDatabase;             // RedbDatabase::open(&Path) — storage agent implements
pub const SCHEMA_VERSION: u64 = 1;
```

One committed block = one `WriteBatch`: header, `HeaderNumbers`, body, `Transactions`,
receipts, `write_state_diff(...)`, `Meta[LATEST_BLOCK]` (§14.2).

## xdevs-state

```rust
pub trait StateProvider {   // not Send/Sync-bound; use `dyn StateProvider + Send + Sync` if needed
    fn basic_account(&self, address: Address) -> Result<Option<AccountInfo>, StateError>;
    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError>;   // 0 if unset
    fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytes>, StateError>;
    fn block_hash(&self, number: u64) -> Result<Option<B256>, StateError>;        // BLOCKHASH
    fn state_root_with(&self, diff: &StateDiff) -> Result<B256, StateError>;     // root after diff
    fn code(&self, address: Address) -> Result<Bytes, StateError>;               // provided
}   // forwarded for &T, Box<T>, Arc<T>
pub enum StateError { Db(DbError), Inconsistent(String), Unavailable(String) }

pub struct StorageDiff { pub wiped: bool, pub slots: BTreeMap<B256, U256> }   // 0 = delete
pub struct StateDiff {
    pub accounts: BTreeMap<Address, Option<AccountInfo>>,   // None = destroyed (+ its storage)
    pub storage:  BTreeMap<Address, StorageDiff>,
    pub codes:    BTreeMap<B256, Bytes>,
}   // new, is_empty, set_account, destroy_account, set_storage, insert_code -> hash, merge(later)
// Apply order: accounts, then storage (wiped first, then slots; skipped for destroyed
// accounts), then codes.

pub struct InMemoryState;   // complete: new, from_diff, apply(&diff), insert_account, insert_code,
                            // set_storage, set_block_hash, accounts(), account_storage(),
                            // account(&addr) -> Option<Account>, storage_root(&addr), state_root()
pub struct DbStateProvider<D: DbRead>;   // new(db), load_all() -> InMemoryState; StateProvider impl
pub fn write_state_diff(batch: &mut WriteBatch, diff: &StateDiff);
pub mod root { fn storage_root(slots) -> B256; fn state_root(&accounts, &storage) -> B256; }
```

## xdevs-execution

```rust
pub struct ExecutionOutcome { pub state_root: B256, pub receipts_root: B256, pub logs_bloom: Bloom,
    pub gas_used: u64, pub receipts: Vec<ReceiptEnvelope>, pub state_diff: StateDiff }
pub struct BlockAttributes { pub timestamp: u64, pub beneficiary: Address, pub prev_randao: B256,
    pub extra_data: Bytes, pub gas_limit: u64 }
pub struct BuiltBlock { pub block: Block, pub outcome: ExecutionOutcome, pub senders: Vec<Address>,
    pub rejected: Vec<(TxHash, ExecutionError)> }

pub trait BlockExecutor: Send + Sync {
    /// strict; state = state after parent; caller compares roots with block.header
    fn execute(&self, parent: &Header, block: &Block, state: &dyn StateProvider)
        -> Result<ExecutionOutcome, ExecutionError>;
    /// builds header (base fee = next_base_fee(parent), roots, gas_used, compat fields)
    fn build(&self, parent: &Header, attrs: BlockAttributes, candidates: Vec<RecoveredTx>,
             state: &dyn StateProvider) -> Result<BuiltBlock, ExecutionError>;
}
pub fn next_base_fee(parent: &Header, cfg: &ChainConfig) -> u64;   // implemented + tested

pub enum CallResult { Success{output, gas_used, logs}, Revert{output, gas_used}, Halt{reason, gas_used} }
pub struct EvmExecutor;   // new(ChainConfig); impl BlockExecutor
impl EvmExecutor {
    fn call(&self, state: &dyn StateProvider, header: &Header, request: &TransactionRequest,
            gas_cap: u64) -> Result<CallResult, ExecutionError>;          // eth_call
    fn estimate_gas(&self, state: &dyn StateProvider, header: &Header, request: &TransactionRequest,
            gas_cap: u64) -> Result<u64, ExecutionError>;                 // eth_estimateGas
}
pub enum ExecutionError { InvalidTransaction{hash, reason}, UnsupportedTxType(u8),
    BlockGasLimitExceeded{used, limit}, InvalidHeader(String), RootMismatch{field, expected, got},
    Reverted{output}, GasCapExceeded{cap}, InvalidRequest(String), State(String), Evm(String) }
```

Fees (§10.2/§15.1): `base_fee * gas_used` → `ChainConfig::split_base_fee` (80% burned = not
credited anywhere, 20% credited to `cfg.treasury`); tip → `header.beneficiary`. These
credits are part of `state_diff`. `TransactionRequest` = `alloy_rpc_types_eth::TransactionRequest`.

## xdevs-mempool

```rust
pub enum TxOrigin { Local, External }
pub struct PoolConfig { max_txs: 10_000, max_bytes: 64 MiB, max_per_sender: 64, max_nonce_gap: 64,
                        price_bump_percent: 10, queued_ttl_secs: 10_800 }
pub struct PoolStatus { pub pending: usize, pub queued: usize }
pub enum PoolError { Oversized{size,max}, UnsupportedType(u8), InvalidSignature(String),
    WrongChainId{expected, got}, NonceTooLow{state,tx}, NonceGapTooLarge{state,tx},
    InsufficientFunds{balance,cost}, Underpriced{max_fee,required}, ReplacementUnderpriced,
    IntrinsicGasTooLow{gas_limit,intrinsic}, GasLimitExceedsBlock, AlreadyKnown,
    SenderLimit(Address), PoolFull, State(String) }   // Display = geth wording

pub trait TxPool: Send + Sync {
    fn add(&self, tx: RecoveredTx, origin: TxOrigin) -> Result<TxHash, PoolError>;
    fn best_txs(&self, base_fee: u64, gas_limit: u64) -> Vec<RecoveredTx>;  // does not remove
    fn on_block_committed(&self, block: &Block, receipts: &[ReceiptEnvelope]);
    fn remove(&self, hashes: &[TxHash]);
    fn get(&self, hash: &TxHash) -> Option<RecoveredTx>;
    fn contains(&self, hash: &TxHash) -> bool;          // provided
    fn pending_nonce(&self, sender: &Address) -> Option<u64>;
    fn pending(&self) -> Vec<RecoveredTx>;
    fn status(&self) -> PoolStatus;
}
```

Additional `PoolError` variants: `GasLimitTooHigh{gas_limit,cap}` (optional EIP-7825 cap),
`TipAboveFeeCap{max_fee,tip}`, `InitCodeTooLarge{size,max}`.

Concrete pool (node wiring):

```rust
pub trait AccountReader: Send + Sync {            // latest committed state
    fn account(&self, Address) -> Result<Option<AccountInfo>, StateError>; }
// impl'd for closures Fn(Address) -> Result<Option<AccountInfo>, StateError> + Send + Sync
pub struct FromState<S>(pub S);                   // any StateProvider + Send + Sync
pub struct PoolParams { chain_id, block_gas_limit, max_tx_gas: Option<u64>, max_tx_size }
impl PoolParams { fn from_chain_config(&ChainConfig) -> Self }   // max_tx_gas = None
pub struct Pool;   // impl TxPool
impl Pool { fn new(PoolParams, PoolConfig, Arc<dyn AccountReader>, base_fee: u64) -> Self;
            fn set_base_fee(&self, u64); fn base_fee(&self) -> u64; fn queued(&self) -> Vec<RecoveredTx>;
            fn len(&self); fn is_empty(&self); fn total_bytes(&self) }
pub fn intrinsic_gas(&TxEnvelope) -> u64;   // max(intrinsic, EIP-7623 floor)
```

Accepted types: 0x00/0x01/0x02 (0x03 and 0x04 → `UnsupportedType`). `on_block_committed`
sets the entry-check base fee to the block's base fee — call `set_base_fee(next_base_fee)`
afterwards if you want the next block's. Queued TTL is measured in block timestamps.

Base fee is `u64` everywhere (header type), tx fee fields are `u128` (alloy).

## xdevs-crypto

```rust
pub fn decode_and_recover(raw: &[u8], chain_id: u64) -> Result<RecoveredTx, TxRecoveryError>; // eth_sendRawTransaction
pub fn recover_tx(TxEnvelope, chain_id) -> Result<RecoveredTx, TxRecoveryError>;  // rejects pre-EIP-155
pub enum TxRecoveryError { Decode(String) /* "rlp: .." */, Unprotected /* geth wording */,
                           WrongChainId{expected, got}, InvalidSignature(String) }
pub fn sign_tx<T: SignableTransaction<Signature>>(&PrivateKeySigner, T) -> Result<Signed<T>, _>;
pub fn sign_tx_envelope(&PrivateKeySigner, T) -> Result<TxEnvelope, _>;
pub fn generate_signer() -> PrivateKeySigner; pub fn signer_from_hex(&str) -> Result<_, KeyError>;
pub struct Keystore;   // Web3 Secret Storage v3: from_json_str, to_json_string, stored_address, decrypt
pub fn encrypt_signer(&PrivateKeySigner, password, ScryptParams::{STANDARD, LIGHT}) -> Result<Keystore, _>;
pub fn decrypt_signer(&Keystore, password) -> Result<PrivateKeySigner, KeystoreError>;
```

## xdevs-genesis

```rust
pub struct GenesisAccount { pub balance: U256, pub code: Option<Bytes>,
                            pub storage: BTreeMap<B256, U256>, pub nonce: u64 }
pub struct Genesis { pub config: ChainConfig, pub timestamp: u64, pub gas_limit: u64,
    pub extra_data: Bytes, pub alloc: BTreeMap<Address, GenesisAccount>,
    pub validators: Vec<serde_json::Value> /* Phase 2+, ignored */ }
impl Genesis { fn from_json_str(&str) -> Result<Self, GenesisError>; fn from_file(&Path) -> ...;
    fn validate(&self); fn chain_config(&self) -> ChainConfig /* block_gas_limit = gas_limit */;
    fn dev(accounts: &[Address], balance: U256) -> Self }
pub fn genesis_state(&Genesis) -> StateDiff;
pub fn genesis_header(&Genesis, state_root: B256) -> Header;  // base_fee = config.base_fee.initial
pub fn genesis_block(&Genesis) -> (Block, StateDiff);
pub enum GenesisError { Io, Json, Invalid(String) }
```

JSON shape = plan §21.1 (`config`, `timestamp`, `gasLimit`, `extraData`,
`alloc{addr:{balance, code?, storage?, nonce?}}`, `validators`).

## xdevs-rpc

```rust
pub mod types { RpcBlock, RpcHeader, RpcTransaction, RpcReceipt, RpcLog, BlockId,
                BlockNumberOrTag, BlockTransactions, BlockTransactionsKind, FeeHistory, Filter,
                SyncStatus, SyncInfo, TransactionRequest }   // alloy-rpc-types-eth
pub type BackendResult<T> = Result<T, RpcBackendError>;

#[async_trait] pub trait RpcBackend: Send + Sync + 'static {
    fn chain_id(&self) -> u64;                              // eth_chainId, net_version
    fn client_version(&self) -> String;                     // provided
    async fn block_number(&self) -> BackendResult<u64>;
    async fn syncing(&self) -> BackendResult<SyncStatus>;   // provided: None
    async fn balance(&self, Address, BlockId) -> BackendResult<U256>;
    async fn transaction_count(&self, Address, BlockId) -> BackendResult<u64>;
    async fn code(&self, Address, BlockId) -> BackendResult<Bytes>;
    async fn storage_at(&self, Address, slot: B256, BlockId) -> BackendResult<B256>;
    async fn call(&self, TransactionRequest, BlockId) -> BackendResult<Bytes>;
    async fn estimate_gas(&self, TransactionRequest, BlockId) -> BackendResult<u64>;
    async fn send_raw_transaction(&self, raw: Bytes) -> BackendResult<B256>;
    async fn block_by_number(&self, BlockNumberOrTag, full: bool) -> BackendResult<Option<RpcBlock>>;
    async fn block_by_hash(&self, B256, full: bool) -> BackendResult<Option<RpcBlock>>;
    async fn block_transaction_count(&self, BlockId) -> BackendResult<Option<u64>>;
    async fn transaction_by_hash(&self, B256) -> BackendResult<Option<RpcTransaction>>;
    async fn transaction_receipt(&self, B256) -> BackendResult<Option<RpcReceipt>>;
    async fn block_receipts(&self, BlockId) -> BackendResult<Option<Vec<RpcReceipt>>>;
    async fn logs(&self, Filter) -> BackendResult<Vec<RpcLog>>;
    async fn gas_price(&self) -> BackendResult<U256>;
    async fn max_priority_fee_per_gas(&self) -> BackendResult<U256>;
    async fn next_base_fee(&self) -> BackendResult<u64>;
    async fn fee_history(&self, block_count: u64, newest: BlockNumberOrTag,
                         reward_percentiles: Option<Vec<f64>>) -> BackendResult<FeeHistory>;
}
pub enum RpcBackendError {          // -> jsonrpsee ErrorObjectOwned (implemented)
    InvalidParams(String) /*-32602*/, UnknownBlock /*-32000 "header not found"*/,
    Reverted{data, reason} /*3, data = 0x-hex*/, Rejected(String) /*-32000, geth wording*/,
    LimitExceeded(String) /*-32005*/, Unsupported(String) /*-32601*/, Internal(String) /*-32603*/ }
pub struct RpcServerConfig { http: 127.0.0.1:8545, gas_cap: 50M, max_logs_range: 10_000,
                             max_logs_results: 10_000, max_batch: 50, cors: ["*"] }
pub async fn start_server(RpcServerConfig, Arc<dyn RpcBackend>) -> anyhow::Result<RpcServerHandle>;
pub struct RpcServerHandle { pub local_addr: SocketAddr, .. }   // stop(), stopped().await
pub mod convert {   // rpc agent implements; node uses
    struct TxMeta { block_hash, block_number, index, base_fee: Option<u64>, timestamp }
    struct ReceiptMeta { gas_used, first_log_index, contract_address: Option<Address> }
    fn rpc_header(&SealedHeader) -> RpcHeader;
    fn rpc_block(&Block, hash: B256, senders: &[Address], full: bool) -> RpcBlock;
    fn rpc_transaction(&RecoveredTx, Option<TxMeta>) -> RpcTransaction;
    fn rpc_receipt(&RecoveredTx, &ReceiptEnvelope, TxMeta, ReceiptMeta) -> RpcReceipt;
    fn rpc_logs(&ReceiptEnvelope, tx_hash: B256, TxMeta, first_log_index: u64) -> Vec<RpcLog>;
}
```

Methods served (§19.1, Phase 1): `web3_clientVersion`, `web3_sha3`, `net_version`,
`net_listening`, `net_peerCount`, `eth_chainId`, `eth_blockNumber`, `eth_syncing`,
`eth_gasPrice`, `eth_maxPriorityFeePerGas`, `eth_feeHistory`, `eth_getBalance`,
`eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`, `eth_call`,
`eth_estimateGas`, `eth_sendRawTransaction`, `eth_getBlockByNumber`, `eth_getBlockByHash`,
`eth_getBlockTransactionCountByNumber/ByHash`, `eth_getTransactionByHash`,
`eth_getTransactionReceipt`, `eth_getBlockReceipts`, `eth_getLogs`. Tags: `latest` = `safe`
= `finalized` = latest committed block.

## xdevs-node

```rust
pub struct NodeConfig { pub chain: ChainSection, pub dev: DevSection, pub rpc: RpcSection,
                        pub mempool: MempoolSection, pub log: LogSection }   // serde(default)
// chain: genesis: Option<PathBuf> (None = Genesis::dev), data_dir (".xdevs/data"), pruning
// dev:   enabled (true), block_time_ms: Option<u64>, fee_recipient: Address
// rpc:   http (127.0.0.1:8545), namespaces, gas_cap (50M), max_logs_range (10k), max_batch (50)
// mempool: max_txs (10k), max_bytes (64 MiB), max_per_sender (64)
// log:   level ("info"), format ("text"|"json")
// NodeConfig::{from_toml_str, load(&Path), to_toml_string}; unknown keys (p2p, validator...) ignored
pub struct Node;  // Node::new(NodeConfig) -> anyhow::Result<Self>; async run(self) -> anyhow::Result<()>
```

CLI: `xdevs node [--dev] [--config config.toml] [--genesis genesis.json] [--data-dir DIR]`;
`xdevs-cli keygen [--out keystore.json] [--light-kdf]`, `xdevs-cli address --key FILE`,
`xdevs-cli send --rpc URL (--from KEYSTORE | --private-key HEX) --to ADDR --value TXDS [--wait]`,
`xdevs-cli balance ADDR --rpc URL`.

## Dev-mode block production (node, per tick)

1. `parent` = latest header; `base_fee = next_base_fee(parent, cfg)`.
2. `txs = pool.best_txs(base_fee, cfg.block_gas_limit)`.
3. `built = executor.build(parent, BlockAttributes{timestamp: max(now, parent.ts + 1), beneficiary:
   dev.fee_recipient, prev_randao: parent hash, extra_data: "xdevs", gas_limit}, txs, &state)`.
4. One `WriteBatch`: header/body/tx index/receipts + `write_state_diff(&built.outcome.state_diff)` + `Meta`.
5. `pool.remove(rejected)`, `pool.on_block_committed(&block, &receipts)`.

Restart safety: on start, if `Meta[GENESIS_HASH]` exists it must equal the computed genesis
hash; otherwise write genesis (header, state) in one batch.
