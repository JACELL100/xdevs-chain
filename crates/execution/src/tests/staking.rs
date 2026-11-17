//! Phase 3 Staking system calls against the real Staking runtime bytecode (from
//! `deploy/configs/predeploys-alloc.json`), initialised in Rust exactly like the genesis
//! builder does: block reward funding, reward split, liveness / downtime jailing,
//! double-sign slashing and the epoch-boundary validator set.

use super::*;
use crate::staking::{
    abi, calls, decode_validator_info, genesis_storage, signed_bitmap, GenesisStake, SlashEvidence,
    StakingInputs, StakingParams, ValidatorStatus,
};
use std::collections::BTreeSet;
use xdevs_primitives::{constants::STAKING_ADDRESS, hex};

/// Runtime bytecode of the Staking predeploy as shipped in the genesis artifacts.
pub(crate) fn staking_code() -> Bytes {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/configs/predeploys-alloc.json"
    );
    let json = std::fs::read_to_string(path).expect("read predeploys-alloc.json");
    let alloc = json.find("\"alloc\"").expect("alloc");
    let entry = alloc
        + json[alloc..]
            .find("\"0x0000000000000000000000000000000000001000\"")
            .expect("Staking entry");
    let marker = "\"code\": \"";
    let start = entry + json[entry..].find(marker).expect("Staking code") + marker.len();
    let end = start + json[start..].find('"').expect("end of code");
    hex::decode(json[start..end].trim_start_matches("0x"))
        .expect("hex code")
        .into()
}

fn ether(n: u64) -> U256 {
    U256::from(n) * U256::from(ETHER)
}

fn key(i: u8) -> B256 {
    B256::repeat_byte(0xa0 + i)
}

fn operator(i: u8) -> Address {
    Address::repeat_byte(0x10 + i)
}

fn test_params() -> StakingParams {
    StakingParams {
        min_self_stake: ether(1_000),
        downtime_window: 1_000,
        ..StakingParams::default()
    }
}

struct Chain {
    exec: EvmExecutor,
    st: InMemoryState,
    parent: Header,
}

/// Four validators with 1,000 ether each, staking active from block 1.
fn chain(params: StakingParams, epoch_length: u64, activation: u64) -> Chain {
    let cfg = ChainConfig {
        staking_validators_block: Some(activation),
        epoch_length,
        block_reward: U256::from(ETHER),
        ..cfg()
    };
    let exec = EvmExecutor::new(cfg);
    let vals: Vec<GenesisStake> = (0..4)
        .map(|i| GenesisStake {
            operator: operator(i),
            consensus_key: key(i),
            commission_bps: 1_000,
            stake: ether(1_000),
        })
        .collect();
    let code = staking_code();
    let (balance, storage) = genesis_storage(
        &exec,
        code.clone(),
        Address::repeat_byte(0x99),
        &params,
        &vals,
    )
    .unwrap();
    assert_eq!(balance, ether(4_000));
    let mut st = funded_state();
    let code_hash = st.insert_code(code);
    let mut info = AccountInfo::new(0, balance);
    info.code_hash = code_hash;
    st.insert_account(STAKING_ADDRESS, info);
    for (slot, value) in storage {
        st.set_storage(STAKING_ADDRESS, slot, value);
    }
    Chain {
        exec,
        st,
        parent: parent(),
    }
}

impl Chain {
    /// Build block `parent + 1` with `inputs` and `txs`, re-execute it strictly, apply it.
    fn step(&mut self, inputs: &StakingInputs, txs: Vec<RecoveredTx>) -> BuiltBlock {
        let built = self
            .exec
            .build_with(&self.parent, attrs(&self.parent), txs, &self.st, inputs)
            .unwrap();
        let replay = self
            .exec
            .execute_with(&self.parent, &built.block, &self.st, inputs)
            .unwrap();
        assert_eq!(
            replay, built.outcome,
            "strict execution reproduces the build"
        );
        self.st.apply(&built.outcome.state_diff);
        assert_eq!(self.st.state_root(), built.block.header.state_root);
        self.parent = built.block.header.clone();
        built
    }

    fn view(&self, data: Bytes) -> Bytes {
        self.exec
            .system_call_at(&self.st, &self.parent, STAKING_ADDRESS, data)
            .unwrap()
            .0
    }

    fn info(&self, op: Address) -> crate::staking::ValidatorInfo {
        decode_validator_info(&self.view(calls::get_validator(op))).unwrap()
    }

    fn pending(&self, op: Address) -> U256 {
        abi::uint_at(&self.view(calls::pending_rewards(op, op)), 0).unwrap()
    }
}

fn all_sign(proposer: u8) -> StakingInputs {
    StakingInputs {
        proposer: Some(key(proposer)),
        signers: (0..4).map(key).collect(),
        evidence: vec![],
    }
}

#[test]
fn selectors_match_the_spec() {
    // docs/specs/predeploys-system.md §1.3
    let cases = [
        ("recordParticipation(address,uint256)", "671a8af4"),
        ("slash(bytes)", "5e47655f"),
        ("onEpoch()", "c5242beb"),
        ("consensusValidatorSet()", "3c32d66c"),
        ("signingSet()", "39c47408"),
        ("currentEpoch()", "76671808"),
    ];
    for (sig, want) in cases {
        assert_eq!(hex::encode(abi::selector(sig)), want, "{sig}");
    }
    assert_eq!(
        hex::encode(&calls::initialize(Address::ZERO, &StakingParams::default(), &[])[..4]),
        "dd74dafd"
    );
    // slash(bytes): offset, length 128, then the 4 evidence words.
    let data = calls::slash(operator(1), 7, B256::repeat_byte(3));
    assert_eq!(data.len(), 4 + 6 * 32);
    assert_eq!(abi::u64_at(&data[4..], 1).unwrap(), 128);
    assert_eq!(abi::address_at(&data[4..], 3).unwrap(), operator(1));
}

#[test]
fn signer_bitmap() {
    let set: Vec<Address> = (0..4).map(operator).collect();
    let signed: BTreeSet<Address> = [operator(0), operator(2), operator(9)].into();
    assert_eq!(signed_bitmap(&set, &signed), U256::from(0b101u64));
    assert_eq!(signed_bitmap(&[], &signed), U256::ZERO);
}

#[test]
fn genesis_initialisation_matches_the_builder_layout() {
    let c = chain(test_params(), 100, 1);
    for i in 0..4 {
        let info = c.info(operator(i));
        assert_eq!(info.status, ValidatorStatus::Active);
        assert_eq!(info.consensus_key, key(i));
        assert_eq!(info.total_stake, ether(1_000));
    }
    let set =
        crate::staking::decode_validator_set(&c.view(calls::consensus_validator_set())).unwrap();
    assert_eq!(set.len(), 4);
    assert!(set.iter().all(|v| v.stake == ether(1_000)));
}

#[test]
fn block_reward_is_split_between_proposer_and_signers() {
    let mut c = chain(test_params(), 100, 1);
    let before = balance(&c.st, STAKING_ADDRESS);
    let b1 = c.step(&all_sign(0), vec![]);
    assert!(b1.outcome.next_validators.is_none());
    assert_eq!(balance(&c.st, STAKING_ADDRESS), before + U256::from(ETHER));
    // System calls use no block gas and create no receipt.
    assert_eq!(b1.block.header.gas_used, 0);
    assert!(b1.outcome.receipts.is_empty());

    let p: Vec<U256> = (0..4).map(|i| c.pending(operator(i))).collect();
    // Proposer bonus 5%: the proposer earns more than the others, who earn the same.
    assert!(p[0] > p[1], "{p:?}");
    assert_eq!(p[1], p[2]);
    assert_eq!(p[2], p[3]);
    let total = p.iter().fold(U256::ZERO, |a, b| a + b);
    assert!(total <= U256::from(ETHER) && U256::from(ETHER) - total < U256::from(100u64));

    // Validator 3 misses block 1's commit: it earns nothing in block 2.
    let inputs = StakingInputs {
        proposer: Some(key(1)),
        signers: vec![key(0), key(1), key(2)],
        evidence: vec![],
    };
    c.step(&inputs, vec![]);
    assert_eq!(c.pending(operator(3)), p[3]);
    assert!(c.pending(operator(1)) > p[1]);
}

#[test]
fn a_registered_validator_joins_at_the_epoch_boundary() {
    let mut c = chain(test_params(), 3, 1);
    let new_op = signer(KEY0).address();
    let register = tx1559(
        &signer(KEY0),
        cfg().chain_id,
        0,
        TxKind::Call(STAKING_ADDRESS),
        ether(1_000),
        calls::register(key(4), 500),
        1_000_000,
        u128::from(GWEI),
    );
    let b1 = c.step(&all_sign(0), vec![register]);
    assert_eq!(
        b1.block.body.len(),
        1,
        "register included: {:?}",
        b1.rejected
    );
    assert!(b1.outcome.receipts[0].is_success());
    assert_eq!(c.info(new_op).status, ValidatorStatus::Candidate);
    assert!(c
        .step(&all_sign(1), vec![])
        .outcome
        .next_validators
        .is_none());

    // Block 3 is the boundary: onEpoch selects the new validator, the set is returned.
    let b3 = c.step(&all_sign(2), vec![]);
    let next = b3.outcome.next_validators.expect("epoch boundary");
    assert_eq!(next.len(), 5);
    let joined = next.iter().find(|v| v.consensus_key == key(4)).unwrap();
    assert_eq!(joined.operator, new_op);
    assert_eq!(joined.stake, ether(1_000));
    assert_eq!(c.info(new_op).status, ValidatorStatus::Active);
    assert_eq!(
        abi::u64_at(&c.view(calls::current_epoch()), 0).unwrap(),
        1,
        "first epoch transition"
    );
}

#[test]
fn double_sign_evidence_slashes_and_tombstones() {
    let mut c = chain(test_params(), 2, 1);
    let evidence = SlashEvidence {
        consensus_key: key(1),
        height: 1,
        reference: B256::repeat_byte(0xee),
    };
    let inputs = StakingInputs {
        evidence: vec![evidence.clone()],
        ..all_sign(0)
    };
    c.step(&inputs, vec![]);
    let info = c.info(operator(1));
    assert_eq!(info.status, ValidatorStatus::Tombstoned);
    assert_eq!(info.total_stake, ether(950), "5% double-sign slash");
    assert_eq!(info.jailed_until, u64::MAX);

    // The same evidence again and evidence against an unknown key are ignored (the
    // block stays valid); the boundary drops the tombstoned validator from the set.
    let unknown = SlashEvidence {
        consensus_key: key(9),
        ..evidence.clone()
    };
    let inputs = StakingInputs {
        evidence: vec![evidence, unknown],
        ..all_sign(2)
    };
    let b2 = c.step(&inputs, vec![]);
    assert_eq!(c.info(operator(1)).total_stake, ether(950));
    let next = b2.outcome.next_validators.expect("epoch boundary");
    assert_eq!(next.len(), 3);
    assert!(next.iter().all(|v| v.consensus_key != key(1)));
}

#[test]
fn an_absent_validator_is_jailed_and_leaves_the_set() {
    let params = StakingParams {
        downtime_window: 4,
        ..test_params()
    };
    let mut c = chain(params, 4, 1);
    let absent = StakingInputs {
        proposer: Some(key(0)),
        signers: vec![key(0), key(1), key(2)],
        evidence: vec![],
    };
    for _ in 0..3 {
        c.step(&absent, vec![]);
        assert_eq!(c.info(operator(3)).status, ValidatorStatus::Active);
    }
    // Block 4 closes the liveness window (0 of 4 signed) and is the epoch boundary.
    let b4 = c.step(&absent, vec![]);
    let info = c.info(operator(3));
    assert_eq!(info.status, ValidatorStatus::Jailed);
    assert_eq!(
        info.total_stake,
        ether(1_000) - ether(1_000) / U256::from(10_000u64),
        "0.01% downtime slash"
    );
    let next = b4.outcome.next_validators.expect("epoch boundary");
    assert_eq!(next.len(), 3);
    assert!(next.iter().all(|v| v.consensus_key != key(3)));
    // The signers are not jailed.
    assert_eq!(c.info(operator(0)).status, ValidatorStatus::Active);
}

#[test]
fn no_system_calls_before_the_activation() {
    let mut c = chain(test_params(), 2, 5);
    let before = balance(&c.st, STAKING_ADDRESS);
    for _ in 0..4 {
        let b = c.step(&all_sign(0), vec![]);
        assert!(b.outcome.next_validators.is_none());
    }
    assert_eq!(balance(&c.st, STAKING_ADDRESS), before);
    assert_eq!(c.pending(operator(0)), U256::ZERO);
    // Block 5 activates: reward credited; block 6 is the first boundary.
    c.step(&all_sign(0), vec![]);
    assert_eq!(balance(&c.st, STAKING_ADDRESS), before + U256::from(ETHER));
    assert!(c
        .step(&all_sign(1), vec![])
        .outcome
        .next_validators
        .is_some());
}
