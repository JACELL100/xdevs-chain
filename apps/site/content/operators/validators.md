# Validators

<span class="status next">Phase 3 — coming</span>

Running a validator on a **public** xDEVS network isn't possible yet, because no public network exists. This page describes what already works and what Phase 3 adds.

## What works today

- **BFT consensus** (Tendermint-style, Malachite) between validators listed in `genesis.json`, with equal or configured voting power.
- **Ed25519 consensus keys** in encrypted keystores (`xdevs-cli keygen --consensus`), a libp2p node identity (`--node-key`), and a persistent double-sign guard.
- **Validator and full-node modes**, block sync for late joiners, and transaction gossip.

Try it locally: [Local multi-validator network](/operators/local-network).

## What Phase 3 adds

| Item | Description |
|---|---|
| Staking-driven validator set | Register with `register(consensusKey, commission)` plus a self-stake in the Staking contract (`0x…1000`). The active set (top N by stake) is recomputed every epoch (3,600 blocks) |
| Rewards | Block rewards to validators that signed, with a 5% proposer bonus and a commission, then shared with delegators |
| Slashing | Double-signing: 5% slash and a permanent ban (tombstone). Downtime below 50% of a 10,000-block window: a small slash and a 10-minute jail |
| Delegation | Delegate and undelegate stake; 1-day unbonding on testnet |
| Public devnet | 4 validators and a bootnode on cloud VMs, then community validators |
| Operator docs | Bootstrapping a validator, key rotation, upgrades, halt diagnosis (runbooks) |

The Staking, Governance, Treasury and FeeConfig contracts are already written and tested in Foundry; Phase 3 places them in genesis and has the node call them each block. Parameters (from the [plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md), §33.1, subject to change): up to 100 validators on testnet, minimum self-stake 10,000 tXDS.

## Planned later

- **Phase 5:** BLS aggregate signatures for commit certificates.
- **Phase 6:** a long-lived public testnet with 20+ independent validators (no single operator above 1/3 of voting power), validator onboarding guide, remote signer and sentry-node support, snapshot sync.

Want to run a validator on the public testnet? Watch the [GitHub repository](https://github.com/JACELL100/xdevs-chain) and [xdevs.in](https://xdevs.in) for the announcement.
