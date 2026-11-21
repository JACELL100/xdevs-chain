# Validators

<span class="status ok">Staking-driven set live on Enclave since block 97,200</span>

xDEVS Chain validators run Tendermint-style BFT consensus (Malachite). The Enclave testnet (chain id 717171) runs **4 validators** on Oracle Cloud (two in Hyderabad, two in Mumbai), fixed in `genesis.json`. Phase 3 adds a **validator set chosen by stake** in the Staking system contract (`0x0000000000000000000000000000000000001000`): registered validators with enough self-stake are ranked by stake, the top ones sign blocks, they earn block rewards, and misbehaviour is slashed.

The node implements it behind an activation height (`stakingValidatorsBlock` in the genesis config). On Enclave it has been active since block **97,200** (19 November 2026, about 21:45 UTC), with release [`v0.3.0-enclave.1`](https://github.com/JACELL100/xdevs-chain/releases/tag/v0.3.0-enclave.1). Until then the four genesis validators stay in force. Block 97,200 is an epoch boundary (27 × 3,600), so validators registered before it can join at block 97,201.

:::warning[Not open to the public yet]
Becoming a validator on Enclave is **not open to the public yet**. The four validators accept P2P connections only from each other, so a new node can't follow the chain until the maintainers allow-list it, and 10,000 tXDS of self-stake is far more than the faucet gives out. The steps below describe how registration works once the staking set is active; if you want to run a validator, ask on [GitHub](https://github.com/JACELL100/xdevs-chain/issues) first. Phase 6 opens the network to independent validators.
:::

## How the set is chosen

- Time is divided into **epochs** of `epochLength` blocks (3,600 on Enclave, about 2 hours at 2 s blocks). Block `B` is an epoch boundary when `B % 3600 == 0`.
- At each boundary the node calls `Staking.onEpoch()`, which selects the **top 100 eligible validators by bonded stake** (ties: lower operator address first). That set signs blocks `B + 1` to `B + 3600`. It is fixed for the whole epoch on every node.
- Voting power = bonded stake in **whole tXDS** (self-stake plus delegations).
- Eligible = registered, not jailed, not tombstoned, self-stake ≥ 10,000 tXDS.
- Jailing, tombstoning or exiting during an epoch removes the validator from the contract's active set at once (it stops earning), and from consensus at the next boundary.

## Proposer selection

Each height has one proposer per round, a pure function of the height's validator set, the height and the round:

- **Original rule** (in force on Enclave today): walk the validators in address order to `(height + round) mod total_power`. This rotates only when every validator has power 1. With Enclave's 12,500,000 each, the validator with the lowest consensus address (val-4) proposes every block, which is why every block's `miner` is val-4's fee recipient.
- **Rotating rule**, from the genesis activation height `proposerRotationBlock` on: with equal powers, `validators[(height + round) mod n]` in address order (round-robin; a new round moves to the next validator). With unequal powers, each validator proposes its share of power, interleaved rather than in runs.

The rotating rule is a fork. It is off until `proposerRotationBlock` is scheduled (not yet on Enclave), and every validator must upgrade before that height. See the [runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/activate-proposer-rotation.md).

## Join as a validator

1. **Run a node that follows the chain**, with a consensus key (see [Run a node](/operators/run-a-node) and the [bootstrap runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/bootstrap-validator.md)):
   ```bash
   xdevs-cli keygen --consensus --out consensus_key.json   # prints the Ed25519 public key
   ```
   In `config.toml`: `[validator] enabled = true`, `key_file = "consensus_key.json"`, and `[p2p] persistent_peers` pointing at the existing validators. A key that is not in the set yet is fine: the node follows the chain and starts signing when it is selected. On Enclave, the validators' firewalls use an allow-list: the maintainers must allow your IP on the consensus port (30333) first.
2. **Register** from the account that will operate the validator, sending at least the minimum self-stake (10,000 tXDS) as value:
   ```bash
   cast send 0x0000000000000000000000000000000000001000 \
     "register(bytes32,uint16)" 0x<consensus public key, 32 bytes hex> 500 \
     --value 10000ether --rpc-url https://rpc.xdevschain.tech --private-key $OPERATOR_KEY
   ```
   `500` is the commission in basis points (5%, at most 20%). One registration per operator address; the consensus key is bound to it for good.
3. **Wait for the next epoch boundary.** Check your status with `getValidator(operator)` (`status`: 1 Candidate, 2 Active, 3 Jailed, 4 Tombstoned, 5 Exited) and the signatures in `xdevs_getCommitCertificate`.

Others can `delegate(validator)` to you (minimum 1 tXDS); stake leaves through `undelegate` and a 1-day unbonding period (`withdrawUnbonded`).

## Rewards

From the activation on, every block credits a fixed **block reward** to the Staking contract (1 tXDS per block on Enclave, about 1.6% of the supply per year) and calls `recordParticipation`:

- 5% of it goes to the block's proposer as a bonus;
- the rest is split between the validators that signed the parent block, pro rata to voting power;
- each validator keeps its commission, the remainder is shared by its delegators (the operator's self-stake included) pro rata to stake.

Rewards accumulate in the contract; claim them with `claimRewards(validator)` (the operator also receives its commission there). Priority fees (tips) still go to the proposer's fee recipient; 80% of the base fee is burned and 20% goes to the Treasury.

## Slashing

| Offence | Detection | Penalty |
|---|---|---|
| **Double signing**: two different votes for the same height and round | Conflicting signed votes seen by consensus, or submitted with `xdevs_submitEvidence`; verified by the node and included in a block | 5% of bonded **and** unbonding stake, **tombstoned** (permanently removed) |
| **Downtime**: signed fewer than 50% of the blocks it was expected to sign in a 10,000-block window | The previous block's commit certificate, recorded every block | 0.01% of bonded stake and **jailed for 10 minutes**; after that, the operator calls `unjail()` (self-stake must still be ≥ the minimum) |

Never run the same consensus key on two machines: that is how validators double-sign. The node's double-sign guard protects one machine only.

## Parameters (Enclave)

| Parameter | Value | Changed by |
|---|---|---|
| Epoch length | 3,600 blocks | Network upgrade (fork) |
| Block reward | 1 tXDS per block | Network upgrade (fork) |
| Max active validators | 100 | Governance |
| Minimum self-stake | 10,000 tXDS | Governance |
| Minimum delegation | 1 tXDS | Governance |
| Commission | 0–20%, at most 1 point per day | Governance |
| Unbonding | 1 day | Governance |
| Proposer bonus | 5% of the block reward | Governance |
| Double-sign slash | 5% + tombstone | Governance |
| Downtime | < 50% of 10,000 blocks → 0.01% + 10 min jail | Governance |

## What works today

- BFT consensus with stored commit certificates, Ed25519 consensus keys in encrypted keystores, a libp2p node identity and a persistent double-sign guard.
- Validator and full-node modes, block sync for late joiners, transaction gossip.
- The staking-driven validator set described above, tested on in-process networks (a 5th validator joining, an offline validator jailed, a double-signing validator slashed); it live on Enclave since block 97,200 ([runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/activate-staking-validators.md)).

Try it locally: [Local multi-validator network](/operators/local-network).

## Planned later

- **Phase 5:** BLS aggregate signatures for commit certificates.
- **Phase 6:** a long-lived public testnet with 20+ independent validators (no single operator above 1/3 of voting power), a validator onboarding guide, remote signer and sentry-node support, snapshot sync.
