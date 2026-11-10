//! The embedded load-test token: a minimal ERC-20-style contract hand-assembled here
//! (no compiler needed, fully self-contained).
//!
//! * `transfer(address to, uint256 amount) -> bool`: moves `amount` from the caller,
//!   reverts on insufficient balance, emits the standard `Transfer` event; balances live
//!   at storage slot `uint160(owner)`.
//! * `balanceOf(address) -> uint256`.
//! * `stress(uint256 n)`: writes `n` fresh storage slots (`keccak(counter + i)`) and
//!   advances a global counter: a state-root / trie-growth stressor.
//!
//! The constructor credits [`TOKEN_SUPPLY`] to the deployer.

use xdevs_primitives::{keccak256, Address, Bytes, B256, U256};

/// Initial supply credited to the deployer (10^9 tokens with 18 decimals).
pub const TOKEN_SUPPLY: U256 = U256::from_limbs([0x9fd0_803c_e800_0000, 0x033b_2e3c, 0, 0]);

/// Storage slot of the `stress` counter (2^255: far away from any address slot).
pub const STRESS_COUNTER_SLOT: U256 = U256::from_limbs([0, 0, 0, 1 << 63]);

/// `keccak256("Transfer(address,address,uint256)")`.
pub fn transfer_topic() -> B256 {
    keccak256("Transfer(address,address,uint256)")
}

/// First 4 bytes of `keccak256(signature)`.
pub fn selector(signature: &str) -> [u8; 4] {
    let h = keccak256(signature);
    [h[0], h[1], h[2], h[3]]
}

/// Calldata of `transfer(to, amount)`.
pub fn transfer_calldata(to: Address, amount: U256) -> Bytes {
    let mut v = Vec::with_capacity(68);
    v.extend_from_slice(&selector("transfer(address,uint256)"));
    v.extend_from_slice(&[0u8; 12]);
    v.extend_from_slice(to.as_slice());
    v.extend_from_slice(&amount.to_be_bytes::<32>());
    v.into()
}

/// Calldata of `balanceOf(owner)`.
pub fn balance_of_calldata(owner: Address) -> Bytes {
    let mut v = Vec::with_capacity(36);
    v.extend_from_slice(&selector("balanceOf(address)"));
    v.extend_from_slice(&[0u8; 12]);
    v.extend_from_slice(owner.as_slice());
    v.into()
}

/// Calldata of `stress(slots)`.
pub fn stress_calldata(slots: u64) -> Bytes {
    let mut v = Vec::with_capacity(36);
    v.extend_from_slice(&selector("stress(uint256)"));
    v.extend_from_slice(&U256::from(slots).to_be_bytes::<32>());
    v.into()
}

/// Gas limit for an ERC-20 `transfer` to a possibly fresh recipient (uses ~52k).
pub const ERC20_TRANSFER_GAS: u64 = 90_000;

/// Gas limit for `stress(slots)`: 22.1k per fresh slot plus overhead.
pub fn stress_gas(slots: u64) -> u64 {
    60_000 + 26_000 * slots
}

/// Gas limit for the deployment.
pub const DEPLOY_GAS: u64 = 400_000;

// --- A tiny two-pass assembler ------------------------------------------------------

#[derive(Debug, Clone)]
enum Item {
    Op(u8),
    Push(Vec<u8>),
    /// `PUSH2 <offset of label>`.
    PushLabel(&'static str),
    /// `JUMPDEST` at this label.
    Label(&'static str),
}

mod op {
    pub const STOP: u8 = 0x00;
    pub const ADD: u8 = 0x01;
    pub const SUB: u8 = 0x03;
    pub const LT: u8 = 0x10;
    pub const EQ: u8 = 0x14;
    pub const ISZERO: u8 = 0x15;
    pub const AND: u8 = 0x16;
    pub const SHR: u8 = 0x1c;
    pub const SHA3: u8 = 0x20;
    pub const CALLER: u8 = 0x33;
    pub const CALLDATALOAD: u8 = 0x35;
    pub const CODECOPY: u8 = 0x39;
    pub const POP: u8 = 0x50;
    pub const MSTORE: u8 = 0x52;
    pub const SLOAD: u8 = 0x54;
    pub const SSTORE: u8 = 0x55;
    pub const JUMP: u8 = 0x56;
    pub const JUMPI: u8 = 0x57;
    pub const JUMPDEST: u8 = 0x5b;
    pub const DUP1: u8 = 0x80;
    pub const DUP2: u8 = 0x81;
    pub const DUP3: u8 = 0x82;
    pub const SWAP1: u8 = 0x90;
    pub const LOG3: u8 = 0xa3;
    pub const RETURN: u8 = 0xf3;
    pub const REVERT: u8 = 0xfd;
}

fn size(item: &Item) -> usize {
    match item {
        Item::Op(_) | Item::Label(_) => 1,
        Item::Push(b) => 1 + b.len(),
        Item::PushLabel(_) => 3,
    }
}

fn assemble(items: &[Item]) -> Vec<u8> {
    let mut labels = std::collections::HashMap::new();
    let mut pc = 0usize;
    for it in items {
        if let Item::Label(l) = it {
            assert!(labels.insert(*l, pc).is_none(), "duplicate label {l}");
        }
        pc += size(it);
    }
    let mut code = Vec::with_capacity(pc);
    for it in items {
        match it {
            Item::Op(o) => code.push(*o),
            Item::Label(_) => code.push(op::JUMPDEST),
            Item::Push(b) => {
                assert!((1..=32).contains(&b.len()), "push size");
                code.push(0x5f + b.len() as u8);
                code.extend_from_slice(b);
            }
            Item::PushLabel(l) => {
                let target = *labels.get(l).unwrap_or_else(|| panic!("unknown label {l}"));
                let target = u16::try_from(target).expect("code fits PUSH2");
                code.push(0x61);
                code.extend_from_slice(&target.to_be_bytes());
            }
        }
    }
    code
}

fn push1(v: u8) -> Item {
    Item::Push(vec![v])
}

fn push_word(v: U256) -> Item {
    Item::Push(v.to_be_bytes::<32>().to_vec())
}

fn push_address_mask() -> Item {
    Item::Push(vec![0xff; 20])
}

/// Runtime bytecode of the token.
pub fn token_runtime() -> Bytes {
    use op::*;
    use Item::{Label as L, Op as O, PushLabel as J};
    let sel = |s: &str| Item::Push(selector(s).to_vec());
    let items = vec![
        // selector = calldata[0..4]
        push1(0),
        O(CALLDATALOAD),
        push1(0xe0),
        O(SHR),
        O(DUP1),
        sel("transfer(address,uint256)"),
        O(EQ),
        J("transfer"),
        O(JUMPI),
        O(DUP1),
        sel("balanceOf(address)"),
        O(EQ),
        J("balance_of"),
        O(JUMPI),
        O(DUP1),
        sel("stress(uint256)"),
        O(EQ),
        J("stress"),
        O(JUMPI),
        L("revert"),
        push1(0),
        O(DUP1),
        O(REVERT),
        // --- transfer(to, amount) ---
        L("transfer"),
        O(CALLER),
        O(SLOAD), // [fb]
        push1(0x24),
        O(CALLDATALOAD), // [fb, amt]
        O(DUP1),
        O(DUP3),
        O(LT), // fb < amt
        J("revert"),
        O(JUMPI),
        O(DUP1),
        O(DUP3),
        O(SUB), // [fb, amt, fb-amt]
        O(CALLER),
        O(SSTORE), // balance[caller] = fb - amt
        O(SWAP1),
        O(POP), // [amt]
        push1(0x04),
        O(CALLDATALOAD),
        push_address_mask(),
        O(AND), // [amt, to]
        O(DUP1),
        O(SLOAD), // [amt, to, tb]
        O(DUP3),
        O(ADD), // [amt, to, tb+amt]
        O(SWAP1),
        O(SSTORE), // balance[to] = tb + amt
        push1(0),
        O(MSTORE), // mem[0] = amt
        push1(0x04),
        O(CALLDATALOAD),
        push_address_mask(),
        O(AND), // [to]
        O(CALLER),
        Item::Push(transfer_topic().to_vec()),
        push1(0x20),
        push1(0),
        O(LOG3), // Transfer(caller, to, amt)
        push1(1),
        push1(0),
        O(MSTORE),
        push1(0x20),
        push1(0),
        O(RETURN), // true
        // --- balanceOf(owner) ---
        L("balance_of"),
        push1(0x04),
        O(CALLDATALOAD),
        push_address_mask(),
        O(AND),
        O(SLOAD),
        push1(0),
        O(MSTORE),
        push1(0x20),
        push1(0),
        O(RETURN),
        // --- stress(n) ---
        L("stress"),
        push1(0x04),
        O(CALLDATALOAD), // [n]
        push_word(STRESS_COUNTER_SLOT),
        O(SLOAD), // [n, c]
        L("loop"),
        O(DUP2),
        O(ISZERO),
        J("done"),
        O(JUMPI),
        O(DUP1),
        push1(0),
        O(MSTORE), // mem[0] = c
        push1(1),
        O(DUP2),
        O(ADD), // [n, c, c+1]
        push1(0x20),
        push1(0),
        O(SHA3),   // [n, c, c+1, keccak(c)]
        O(SSTORE), // storage[keccak(c)] = c + 1
        push1(1),
        O(ADD), // [n, c+1]
        O(SWAP1),
        push1(1),
        O(SWAP1),
        O(SUB),
        O(SWAP1), // [n-1, c+1]
        J("loop"),
        O(JUMP),
        L("done"),
        push_word(STRESS_COUNTER_SLOT),
        O(SSTORE), // counter = c
        O(STOP),
    ];
    assemble(&items).into()
}

/// Init code: credit [`TOKEN_SUPPLY`] to the deployer, return [`token_runtime`].
pub fn token_init_code() -> Bytes {
    use op::*;
    let runtime = token_runtime();
    let len = u16::try_from(runtime.len()).expect("runtime fits");
    let prefix = |off: u16| {
        assemble(&[
            push_word(TOKEN_SUPPLY),
            Item::Op(CALLER),
            Item::Op(SSTORE),
            Item::Push(len.to_be_bytes().to_vec()),
            Item::Op(DUP1),
            Item::Push(off.to_be_bytes().to_vec()),
            push1(0),
            Item::Op(CODECOPY),
            push1(0),
            Item::Op(RETURN),
        ])
    };
    let off = u16::try_from(prefix(0).len()).expect("small");
    let mut code = prefix(off);
    code.extend_from_slice(&runtime);
    code.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::hex;

    #[test]
    fn constants() {
        assert_eq!(
            TOKEN_SUPPLY,
            U256::from(10u64).pow(U256::from(27u64)),
            "10^9 * 10^18"
        );
        assert_eq!(STRESS_COUNTER_SLOT, U256::from(1u64) << 255);
        assert_eq!(selector("transfer(address,uint256)"), hex!("a9059cbb"));
        assert_eq!(selector("balanceOf(address)"), hex!("70a08231"));
        assert_eq!(
            transfer_topic(),
            B256::from(hex!(
                "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
            ))
        );
    }

    #[test]
    fn calldata_layout() {
        let to = Address::repeat_byte(0x11);
        let d = transfer_calldata(to, U256::from(5u64));
        assert_eq!(d.len(), 68);
        assert_eq!(&d[..4], &hex!("a9059cbb"));
        assert_eq!(&d[16..36], to.as_slice());
        assert_eq!(d[67], 5);
        assert_eq!(balance_of_calldata(to).len(), 36);
        let s = stress_calldata(3);
        assert_eq!(s.len(), 36);
        assert_eq!(s[35], 3);
    }

    #[test]
    fn bytecode_is_well_formed() {
        let rt = token_runtime();
        // Every PUSH2 jump target lands on a JUMPDEST.
        let mut pc = 0;
        let mut jumpdests = Vec::new();
        let mut targets = Vec::new();
        while pc < rt.len() {
            let o = rt[pc];
            if o == 0x5b {
                jumpdests.push(pc);
            }
            if (0x60..=0x7f).contains(&o) {
                let n = usize::from(o - 0x5f);
                if n == 2 && matches!(rt.get(pc + 3), Some(0x56) | Some(0x57)) {
                    targets.push(usize::from(u16::from_be_bytes([rt[pc + 1], rt[pc + 2]])));
                }
                pc += n;
            }
            pc += 1;
        }
        assert_eq!(pc, rt.len(), "no truncated PUSH");
        assert_eq!(
            targets.len(),
            6,
            "transfer/balanceOf/stress dispatch + revert + loop + done"
        );
        for t in targets {
            assert!(jumpdests.contains(&t), "target {t} is not a JUMPDEST");
        }
        // Init code = 48-byte prefix + runtime; it copies exactly the runtime.
        let init = token_init_code();
        assert_eq!(init.len(), 48 + rt.len());
        assert_eq!(&init[48..], &rt[..]);
        let len = u16::from_be_bytes([init[36], init[37]]);
        let off = u16::from_be_bytes([init[40], init[41]]);
        assert_eq!((usize::from(len), off), (rt.len(), 48));
    }
}
