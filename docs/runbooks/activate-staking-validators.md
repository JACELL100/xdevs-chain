# Runbook: activating the staking-driven validator set (Phase 3)

Runbook index item 9 (plan §11.4–11.6, §24.6). This is a **fork-height upgrade**: read
[upgrade-network.md](upgrade-network.md) first. It switches a running network (Enclave, chain id
717171) from the static genesis validator set to the set chosen by the Staking contract
(`0x…1000`), epoch by epoch, with block rewards and downtime / double-sign slashing.

## What changes at the activation height N

Nothing changes before N: a binary with this feature produces and accepts exactly the same
blocks as before, so old and new binaries can run side by side **until N**.

From block N on, every block:

1. credits `blockReward` wei to the Staking contract (new issuance) and calls
   `recordParticipation(proposer, signedBitmap)` as the system address: the reward is split
   (5% proposer bonus, the rest to the validators that signed the parent block, pro rata to
   power, minus each validator's commission, then shared with its delegators), and the liveness
   counters advance. At the end of each `downtimeWindow` (10,000 blocks on Enclave) every
   active validator that signed fewer than 50% of the blocks it was expected to sign is slashed
   0.01% and jailed for 10 minutes;
2. calls `slash(evidence)` for each double-sign evidence item it carries (5% slash of bonded
   and unbonding stake, permanent tombstone);
3. at an epoch boundary (`number % epochLength == 0`, 3,600 on Enclave) calls `onEpoch()`;
   the node then reads `consensusValidatorSet()` and that set (top 100 eligible validators by
   stake, voting power = stake in whole tXDS) signs the blocks `B + 1 ..= B + 3600`. Jailing,
   tombstoning or exits during an epoch take effect at the next boundary.

Blocks from N on also carry consensus data (proposer, the parent's commit certificate,
evidence) committed by the header's `requestsHash`. **Binaries without this feature cannot
decode or validate them**: every validator, RPC node and bootnode must run the new binary
before N, or it stops following the chain at N.

## Parameters (genesis `config`)

| Key | Enclave value | Meaning |
|---|---|---|
| `stakingValidatorsBlock` | N (choose below) | First block with the Staking system calls. Absent = never (default) |
| `epochLength` | `3600` (already in the genesis) | Blocks per epoch; a fork parameter, never change it after N |
| `blockReward` | `"1000000000000000000"` (1 tXDS) | Wei minted to Staking per block. 1 tXDS × ~15.8 M blocks/year ≈ 1.6% of the 1 B supply per year: deliberately below the plan's 7% schedule (§20.1) until the inflation curve is implemented. Default 0 (no issuance) |

The Staking parameters (minimum self-stake 10,000 tXDS, 100 validators, 5% double-sign slash,
0.01% + 10 min jail below 50% of 10,000 blocks, 5% proposer bonus) live in the contract's
storage since genesis and change only through Governance.

The node accepts these three keys being added to (or changed in) the `genesis.json` of an
existing database **only while the chain head is below both the old and the new N**; after that
it refuses to start with a different schedule. The genesis block hash does not change (the config
is not part of it).

## Procedure

### 1. Choose N

- At least **3 days** out (2 s blocks: ~43,200 blocks/day), like any fork height.
- Prefer a multiple of `epochLength` (e.g. `N = 3600 × k`): block N is then itself the first
  epoch boundary, so validators that registered before N join at N + 1. Otherwise the genesis
  set stays in force until the first boundary after N.
- Announce N with the approximate UTC time, and the reward value, on Discord, GitHub Releases and
  the docs site.

### 2. Rehearse on a devnet

- `cargo test -p xdevs-testutils --test phase3_staking` (in-process: join, downtime, double sign).
- A Docker localnet or the devnet with a copy of the Enclave genesis and `stakingValidatorsBlock`
  a few hundred blocks out, crossing N with all nodes upgraded (upgrade test of
  [upgrade-network.md](upgrade-network.md)).

### 3. Roll out the binary (any time before N)

```bash
cd deploy/ansible
ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z --check --diff
ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z
```

With the unchanged genesis the new binary behaves exactly like the old one (the feature is off).

### 4. Schedule the activation in genesis.json (before N, after step 3)

1. In `deploy/ansible/inventory/testnet/files/genesis.json`, add to `config`:
   ```json
   "stakingValidatorsBlock": N,
   "blockReward": "1000000000000000000"
   ```
   (keep `"epochLength": 3600`).
2. Update `xdevs_genesis_sha256` in `inventory/testnet/group_vars/all/main.yml`
   (`sha256sum files/genesis.json`).
3. Push the file to every node, one node restarted at a time:
   ```bash
   ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z -e xdevs_fork_height=N
   ```
   (`tasks/roll_one.yml` runs the `xdevs_node` role, which installs the new `genesis.json` and
   restarts the node because its config changed; `xdevs_fork_height` refuses to start within
   `xdevs_fork_margin_blocks` of N). Steps 3 and 4 can be one rollout if the new genesis is in
   place when the binary is rolled out.
4. On each node, the log shows `fork schedule updated from the genesis config` once and
   `staking-driven validator set scheduled activation=N epoch_length=3600 first_epoch_block=…`
   at every start.

### 5. Before N

- `ansible all -b -m command -a "xdevs --version"`: every node on the new version.
- Every node logged `staking-driven validator set scheduled` with the same N.
- New validators (e.g. the home machine) can already `register()` (see
  [validators](../../apps/site/content/operators/validators.md)); their node must run with
  `[validator] enabled = true`, its consensus key, and `persistent_peers` pointing at the
  validators, and **its IP must be added to the P2P allow-list / OCI security list** of the
  validator VMs (consensus port 30333).

### 6. At N and at the first boundary

- Height keeps advancing, rounds stay at 0, no `XdevsNodeLagging` alert.
- `eth_getBalance(0x…1000)` grows by `blockReward` per block; `pendingRewards(validator,
  validator)` grows for signers.
- At the first boundary B, every node logs `epoch boundary: validator set of the next epoch fixed
  epoch_block=B validators=…` with the same count and total power; from B + 1 the commit
  certificates (`xdevs_getCommitCertificate`) carry the new set's signatures.
- `scripts/check.sh --expect progress` from a VCN host: no fork.

### 7. If something goes wrong

- **Chain halts at N** (e.g. a node bug in the system calls, `system call failed` in the logs):
  [chain-halted.md](chain-halted.md), then [emergency-restart.md](emergency-restart.md) with a
  binary that fixes the bug. Removing `stakingValidatorsBlock` after N is refused by the node
  on purpose: blocks after N are only valid under the new rules.
- **A node stopped following at N**: it runs an old binary or a genesis without the schedule
  (`missing consensus data` / `consensus data before the staking activation` rejections).
  Upgrade it and restart; it catches up through block sync.
- **The set lost members unexpectedly**: check `getValidator(operator)` (status 3 = jailed: wait
  `jailedUntil`, then the operator calls `unjail()`; status 4 = tombstoned, permanent).

## Proving slashing on the devnet (exit criterion)

A deliberately double-signing test validator (never a real one): sign two different precommits
for the same height and round with its consensus key and submit them to any node with
`xdevs_submitEvidence` (RLP of `xdevs_consensus::ConflictingVotes`; the in-process test
`c_a_double_signing_validator_is_slashed` shows the exact construction). Nodes also pick up
equivocation that Malachite observes on the consensus network by themselves. Within a few blocks
`getValidator(operator).status` is 4 (Tombstoned) and the stake is 5% lower; the validator
leaves the set at the next epoch boundary.
