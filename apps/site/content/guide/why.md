# Why xDEVS Chain?

Most blockchains compete on speed and fees. Meanwhile, the things that actually cost ordinary users money are left for wallets and apps to fix, or not:

- People lose funds forever because they lost a **seed phrase**.
- **Phishing and address poisoning** trick people into sending to the wrong address.
- Forgotten **unlimited token approvals** get drained years later.
- **Front-running bots** (MEV) sandwich normal users' trades.
- **Bridges** secured by small multisigs get hacked.

xDEVS Chain makes **user protection part of the chain itself**, while staying fully compatible with Ethereum tools.

## How it compares

| Problem users face | Ethereum | Solana | xDEVS Chain |
|---|---|---|---|
| **Lost seed phrase = lost funds** | Smart wallets exist as optional add-ons (ERC-4337) | Wallet-level only | Smart accounts are a built-in system contract: **passkey login (Face ID / fingerprint), no seed phrase**, social recovery by guardians |
| **Stolen key drains the wallet instantly** | Nothing at protocol level | Nothing at protocol level | **Guardian Vault**: large transfers, approvals and unknown contract calls wait out a cancel window, and switching protection off is delayed too. An opt-in protocol rule ([XIP-0001](/concepts/eoa-lock)) extends this to existing MetaMask addresses |
| **Forgotten unlimited approvals get drained years later** | ERC-20 approvals are unlimited and permanent by default | Token delegations persist until revoked | **Approvals expire**: the XDRC-20 standard and a Permit2-style contract require an expiry on every allowance, plus one-call "revoke all" |
| **Address poisoning and look-alike names** | Wallet warnings only; ENS still allows ASCII look-alikes such as `paypa1` | Wallet warnings only | Native `.xdevs` names that **reject look-alikes** (`paypa1` can't be registered while `paypal` exists), plus an `xdevs_checkRecipient` RPC that flags first-time and look-alike recipients (live on Enclave) |
| **Blind signing of unreadable transactions** | Third-party simulation services | Third-party simulation services | Built-in `xdevs_simulateTransaction` RPC that shows asset changes and plain-language warnings before you sign (live on Enclave; the [wallet](https://wallet.xdevschain.tech) previews every send with it) |
| **Front-running / sandwich bots (MEV)** | Private relays and builder markets, which centralise | Validator-level, widely exploited | **Encrypted mempool**: transactions stay hidden until their order is final (built as a prototype; off on Enclave until it has a distributed key setup) |
| **Time until a payment is final** | About 13 minutes (2 epochs) | Seconds, with past full-network outages | **BFT finality**: a block is final when committed, no reorgs (live on Enclave with 2 s blocks; 1 s targeted later) |
| **One hot app makes everyone pay more** | Single global fee market | Local fee markets | **Local fee markets**: a congested contract gets its own fee; everyone else keeps paying the normal fee (activates at block 97,200 on Enclave) |
| **Bridge hacks** | Many bridges secured by small multisigs | Same | Policy: **light-client bridges only**, no multisig bridges (Sepolia prototype built, not deployed) |
| **Developer compatibility** | The standard | Different VM and languages | **EVM-equivalent** (revm, Osaka rules): Solidity, MetaMask, Foundry, Hardhat and viem work unchanged |

All protection features are **opt-in** and **never reverse a finalized transaction**: there is no admin key that can freeze or seize anyone's funds.

## Design principles

- **Safety lives in accounts and RPC, never in finality** ([ADR-0006](https://github.com/JACELL100/xdevs-chain/blob/main/docs/adr/0006-safety-in-accounts-and-rpc.md)). Protections are enforced by code the user opts into. Nobody, not validators and not governance, can reverse, freeze or seize funds.
- **Warnings are advice, not facts.** Simulation and recipient checks inform the user; they never block a transaction at the protocol level.
- **Ethereum compatibility first.** Standard transaction types, hashes, receipts and state roots are computed the Ethereum way, so existing tools work.
- **Honest limits.** Every spec lists what a feature does *not* protect against.

## Honest trade-offs

- **Security grows with adoption.** A new chain has far less stake and fewer independent validators than Ethereum. Enclave runs on four validators operated by the project; until many independent operators join, it is a testnet, not a decentralized network.
- **Fast finality means a bounded validator set** (up to 100 once the staking set is active): a deliberate choice of strong safety and speed over maximum decentralization.
- **MEV can be reduced, not eliminated.** The encrypted mempool is research-grade work that will be switched on only after a distributed key setup and external review.
- **Protection is opt-in.** A user who turns nothing on gets ordinary Ethereum-style behaviour.
- **Guardians are trusted.** If you enable social recovery and the guardian freeze, a threshold of your guardians can freeze your account and recover it to new keys. Choose guardians accordingly. See [Guardian Vault & freeze](/concepts/guardian-vault#trade-offs).

See it for yourself: [add Enclave to your wallet](/guide/quickstart), or open the [passkey wallet](https://wallet.xdevschain.tech).
