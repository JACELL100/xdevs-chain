//! Generated block workloads for the Block-STM differential tests
//! (`tests/parallel.rs`) and the benchmark (`examples/parallel_bench.rs`).
//!
//! A deterministic PRNG picks transactions from a mix of: plain transfers (to a small
//! recipient pool, the beneficiary and the treasury), a global counter (one hot slot),
//! a self-calling counter, an ERC-20-like transfer (hot receiver slots), a keyed map
//! (storage collisions on a few keys), a reverting contract, out-of-gas calls,
//! contracts that read `BALANCE(COINBASE)` / `BALANCE(treasury)` (observe the deferred
//! fees of earlier transactions), a contract that forwards value to `COINBASE`,
//! deployments (later called by other senders in the same block) and constructors that
//! self-destruct to `COINBASE`. Senders mix EIP-1559 and legacy transactions with
//! random tips (including zero).

#![allow(dead_code)]

use alloy_consensus::{SignableTransaction, TxEip1559, TxLegacy};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use xdevs_execution::{
    BlockAttributes, BlockExecutor, EvmExecutor, ExecutionOutcome, ParallelStats, StakingInputs,
};
use xdevs_primitives::{
    constants::GWEI, set_compat_header_fields, AccountInfo, Address, Block, Bytes, ChainConfig,
    Header, RecoveredTx, Signed, TxEnvelope, TxKind, B256, U256,
};
use xdevs_state::{InMemoryState, StateProvider};

pub const BENEFICIARY: Address = Address::repeat_byte(0xbe);
pub const ETHER: u128 = 1_000_000_000_000_000_000;

const fn contract(n: u8) -> Address {
    let mut a = [0xc0u8; 20];
    a[19] = n;
    Address::new(a)
}

/// `slot0 += 1`.
pub const COUNTER: Address = contract(1);
const COUNTER_CODE: &[u8] = &[0x5f, 0x54, 0x60, 0x01, 0x01, 0x5f, 0x55, 0x00];
/// `slot[calldata[0..32]] += 1`.
pub const MAP: Address = contract(2);
const MAP_CODE: &[u8] = &[0x5f, 0x35, 0x80, 0x54, 0x60, 0x01, 0x01, 0x90, 0x55, 0x00];
/// Writes slot0, then reverts.
pub const REVERTER: Address = contract(3);
const REVERTER_CODE: &[u8] = &[0x60, 0x01, 0x5f, 0x55, 0x5f, 0x5f, 0xfd];
/// `slot0 += 1`, then (if called without calldata) calls itself once with 1 byte.
pub const SELF_CALL: Address = contract(4);
const SELF_CALL_CODE: &[u8] = &[
    0x5f, 0x54, 0x60, 0x01, 0x01, 0x5f, 0x55, // slot0 += 1
    0x36, 0x60, 0x15, 0x57, // calldata non-empty -> jump to the end
    0x5f, 0x5f, 0x60, 0x01, 0x5f, 0x5f, 0x30, 0x5a, 0xf1, 0x50, // CALL(self, 1 byte)
    0x5b, 0x00,
];
/// `slot0 = BALANCE(COINBASE)`.
pub const COINBASE_READER: Address = contract(5);
const COINBASE_READER_CODE: &[u8] = &[0x41, 0x31, 0x5f, 0x55, 0x00];
/// `slot0 = BALANCE(treasury)` (code built at genesis).
pub const TREASURY_READER: Address = contract(6);
/// Forwards `CALLVALUE` to `COINBASE`.
pub const PAY_COINBASE: Address = contract(7);
const PAY_COINBASE_CODE: &[u8] = &[0x5f, 0x5f, 0x5f, 0x5f, 0x34, 0x41, 0x5a, 0xf1, 0x50, 0x00];
/// ERC-20-like: `slot[caller] -= 1; slot[calldata[0..32]] += 1`.
pub const TOKEN: Address = contract(8);
const TOKEN_CODE: &[u8] = &[
    0x33, 0x54, 0x60, 0x01, 0x90, 0x03, 0x33, 0x55, // slot[caller] -= 1
    0x5f, 0x35, 0x80, 0x54, 0x60, 0x01, 0x01, 0x90, 0x55, 0x00, // slot[to] += 1
];
/// Constructor that self-destructs to `COINBASE` (EIP-6780: really destroyed).
pub const SELFDESTRUCT_TO_COINBASE_INIT: &[u8] = &[0x41, 0xff];

/// Init code returning `runtime`.
pub fn initcode(runtime: &[u8]) -> Bytes {
    let len = u8::try_from(runtime.len()).expect("short runtime");
    let mut code = vec![0x60, len, 0x60, 0x0a, 0x5f, 0x39, 0x60, len, 0x5f, 0xf3];
    code.extend_from_slice(runtime);
    code.into()
}

pub fn word(v: u64) -> Bytes {
    U256::from(v).to_be_bytes::<32>().to_vec().into()
}

pub fn address_word(a: Address) -> Bytes {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_slice());
    w.to_vec().into()
}

/// SplitMix64.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

/// Kind of workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mix {
    /// Everything (the differential property test).
    Mixed,
    /// Independent transfers between distinct accounts (best case for Block-STM).
    Independent,
    /// Every transaction increments the same slot (worst case: fully serial).
    HotSpot,
    /// ERC-20 transfers, half of them to one hot receiver.
    Erc20,
}

/// A chain of generated blocks on an in-memory state.
pub struct Workload {
    pub exec: EvmExecutor,
    pub state: InMemoryState,
    pub parent: Header,
    pub signers: Vec<PrivateKeySigner>,
    pub nonces: Vec<u64>,
    pub rng: Rng,
    /// Contracts deployed by earlier transactions (callable).
    pub deployed: Vec<Address>,
    /// Gas limit of the generated blocks.
    pub gas_limit: u64,
}

impl Workload {
    /// `senders` funded accounts and the test contracts.
    pub fn new(seed: u64, senders: usize) -> Self {
        let cfg = ChainConfig::dev();
        let mut state = InMemoryState::new();
        let signers: Vec<PrivateKeySigner> = (0..senders)
            .map(|i| {
                PrivateKeySigner::from_bytes(&B256::from(U256::from(0x1000 + i as u64)))
                    .expect("valid key")
            })
            .collect();
        for s in &signers {
            state.insert_account(s.address(), AccountInfo::new(0, U256::from(1_000 * ETHER)));
        }
        let mut treasury_reader = vec![0x73];
        treasury_reader.extend_from_slice(cfg.treasury.as_slice());
        treasury_reader.extend_from_slice(&[0x31, 0x5f, 0x55, 0x00]);
        for (address, code) in [
            (COUNTER, COUNTER_CODE.to_vec()),
            (MAP, MAP_CODE.to_vec()),
            (REVERTER, REVERTER_CODE.to_vec()),
            (SELF_CALL, SELF_CALL_CODE.to_vec()),
            (COINBASE_READER, COINBASE_READER_CODE.to_vec()),
            (TREASURY_READER, treasury_reader),
            (PAY_COINBASE, PAY_COINBASE_CODE.to_vec()),
            (TOKEN, TOKEN_CODE.to_vec()),
        ] {
            let code_hash = state.insert_code(code.into());
            state.insert_account(
                address,
                AccountInfo {
                    nonce: 1,
                    balance: U256::ZERO,
                    code_hash,
                },
            );
        }
        let mut parent = Header {
            number: 0,
            gas_limit: cfg.block_gas_limit,
            gas_used: cfg.block_gas_limit / 2,
            base_fee_per_gas: Some(GWEI),
            timestamp: 1_000,
            ..Default::default()
        };
        set_compat_header_fields(&mut parent);
        let cfg_gas_limit = cfg.block_gas_limit;
        Self {
            exec: EvmExecutor::new(cfg),
            state,
            parent,
            signers,
            nonces: vec![0; senders],
            rng: Rng::new(seed),
            deployed: Vec::new(),
            gas_limit: cfg_gas_limit,
        }
    }

    pub fn cfg(&self) -> &ChainConfig {
        self.exec.chain_config()
    }

    /// Sign the next transaction of sender `s`.
    pub fn tx(&mut self, s: usize, to: TxKind, value: U256, input: Bytes, gas: u64) -> RecoveredTx {
        let nonce = self.nonces[s];
        self.nonces[s] += 1;
        let chain_id = self.cfg().chain_id;
        let tip = u128::from(self.rng.below(4)) * u128::from(GWEI) / 2;
        let signer = &self.signers[s];
        let env: TxEnvelope = if self.rng.below(4) == 0 {
            let tx = TxLegacy {
                chain_id: Some(chain_id),
                nonce,
                gas_price: 2 * u128::from(GWEI) + tip,
                gas_limit: gas,
                to,
                value,
                input,
            };
            let sig = signer.sign_hash_sync(&tx.signature_hash()).expect("sign");
            Signed::new_unhashed(tx, sig).into()
        } else {
            let tx = TxEip1559 {
                chain_id,
                nonce,
                gas_limit: gas,
                max_fee_per_gas: 10 * u128::from(GWEI),
                max_priority_fee_per_gas: tip,
                to,
                value,
                access_list: Default::default(),
                input,
            };
            let sig = signer.sign_hash_sync(&tx.signature_hash()).expect("sign");
            Signed::new_unhashed(tx, sig).into()
        };
        RecoveredTx::recover(env).expect("recover")
    }

    fn call(&mut self, s: usize, to: Address, input: Bytes) -> RecoveredTx {
        self.tx(s, TxKind::Call(to), U256::ZERO, input, 200_000)
    }

    /// One random transaction of `mix` from sender `s` (`i` = position in the block).
    pub fn random_tx(&mut self, mix: Mix, s: usize, i: usize) -> RecoveredTx {
        let n = self.signers.len();
        match mix {
            Mix::Independent => {
                let to = Address::repeat_byte(0x10).create(s as u64 * 1_000_000 + i as u64);
                self.tx(s, TxKind::Call(to), U256::from(1u64), Bytes::new(), 21_000)
            }
            Mix::HotSpot => self.call(s, COUNTER, Bytes::new()),
            Mix::Erc20 => {
                let to = if self.rng.below(2) == 0 {
                    Address::repeat_byte(0x77)
                } else {
                    self.signers[self.rng.below(n as u64) as usize].address()
                };
                self.call(s, TOKEN, address_word(to))
            }
            Mix::Mixed => match self.rng.below(16) {
                0..=2 => {
                    let to = Address::with_last_byte(1 + self.rng.below(6) as u8);
                    let v = U256::from(1 + self.rng.below(1_000));
                    self.tx(s, TxKind::Call(to), v, Bytes::new(), 21_000)
                }
                3 => self.call(s, COUNTER, Bytes::new()),
                4 => self.call(s, SELF_CALL, Bytes::new()),
                5 => {
                    let key = self.rng.below(3);
                    self.call(s, MAP, word(key))
                }
                6 => {
                    let to = self.signers[self.rng.below(n as u64) as usize].address();
                    self.call(s, TOKEN, address_word(to))
                }
                7 => self.call(s, REVERTER, Bytes::new()),
                8 => self.tx(s, TxKind::Call(COUNTER), U256::ZERO, Bytes::new(), 21_100),
                9 => self.call(s, COINBASE_READER, Bytes::new()),
                10 => self.call(s, TREASURY_READER, Bytes::new()),
                11 => {
                    let v = U256::from(1 + self.rng.below(10_000));
                    self.tx(s, TxKind::Call(PAY_COINBASE), v, Bytes::new(), 100_000)
                }
                12 => {
                    let to = if self.rng.below(2) == 0 {
                        BENEFICIARY
                    } else {
                        self.cfg().treasury
                    };
                    self.tx(s, TxKind::Call(to), U256::from(7u64), Bytes::new(), 50_000)
                }
                13 => {
                    let address = self.signers[s].address().create(self.nonces[s]);
                    self.deployed.push(address);
                    self.tx(
                        s,
                        TxKind::Create,
                        U256::ZERO,
                        initcode(COUNTER_CODE),
                        300_000,
                    )
                }
                14 if !self.deployed.is_empty() => {
                    let k = self.rng.below(self.deployed.len() as u64) as usize;
                    let to = self.deployed[k];
                    self.call(s, to, Bytes::new())
                }
                _ => self.tx(
                    s,
                    TxKind::Create,
                    U256::from(5u64),
                    SELFDESTRUCT_TO_COINBASE_INIT.to_vec().into(),
                    100_000,
                ),
            },
        }
    }

    /// `count` random transactions from random senders (nonces in order).
    pub fn random_txs(&mut self, mix: Mix, count: usize) -> Vec<RecoveredTx> {
        (0..count)
            .map(|i| {
                let s = match mix {
                    Mix::Independent | Mix::HotSpot | Mix::Erc20 => i % self.signers.len(),
                    Mix::Mixed => self.rng.below(self.signers.len() as u64) as usize,
                };
                self.random_tx(mix, s, i)
            })
            .collect()
    }

    pub fn attrs(&self) -> BlockAttributes {
        BlockAttributes {
            timestamp: self.parent.timestamp + 2,
            beneficiary: BENEFICIARY,
            prev_randao: B256::repeat_byte(0x42),
            extra_data: Bytes::from_static(b"xdevs"),
            gas_limit: self.gas_limit,
        }
    }

    /// Build a block from `txs` (sequential builder).
    pub fn build(&self, txs: Vec<RecoveredTx>) -> Block {
        let built = self
            .exec
            .build(&self.parent, self.attrs(), txs, &self.state)
            .expect("build");
        assert!(
            built.rejected.is_empty(),
            "generated txs rejected: {:?}",
            built.rejected
        );
        built.block
    }

    /// Execute `block` sequentially and with Block-STM on `threads` threads and assert
    /// identical results (outcome or error).
    pub fn check(
        &self,
        block: &Block,
        threads: usize,
    ) -> Result<(ExecutionOutcome, ParallelStats), String> {
        differential(&self.exec, &self.parent, block, &self.state, threads)
    }

    /// Apply an executed block and make it the parent.
    pub fn advance(&mut self, block: &Block, outcome: &ExecutionOutcome) {
        self.state.apply(&outcome.state_diff);
        assert_eq!(self.state.state_root(), block.header.state_root);
        self.parent = block.header.clone();
    }
}

/// Both execution strategies on the same input; panics on any difference.
pub fn differential(
    exec: &EvmExecutor,
    parent: &Header,
    block: &Block,
    state: &(dyn StateProvider + Sync),
    threads: usize,
) -> Result<(ExecutionOutcome, ParallelStats), String> {
    let staking = StakingInputs::default();
    let seq = exec.execute_sequential(parent, block, state, &staking);
    let par = exec.execute_parallel(parent, block, state, &staking, threads);
    match (seq, par) {
        (Ok(s), Ok((p, stats))) => {
            assert_eq!(
                s, p,
                "parallel outcome differs (threads {threads}, {stats:?})"
            );
            assert_eq!(s.state_root, block.header.state_root);
            Ok((p, stats))
        }
        (Err(s), Err(p)) => {
            assert_eq!(s.to_string(), p.to_string(), "different errors");
            Err(s.to_string())
        }
        (s, p) => panic!("sequential {s:?} vs parallel {p:?}"),
    }
}
