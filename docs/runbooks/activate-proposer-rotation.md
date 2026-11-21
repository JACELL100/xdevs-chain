# Runbook: activating proposer rotation (`proposerRotationBlock`)

This is a **fork-height upgrade**: read [upgrade-network.md](upgrade-network.md) first.

## Why

The original proposer rule (research doc §6.6) takes `k = (height + round) mod total_power`
and walks the validators in address order until the running power exceeds `k`. That is
round-robin only when every validator has power 1. On Enclave each validator has power
12,500,000 (stake in whole tXDS), so `k < 12,500,000` for the first 12.5 M heights and the
validator with the lowest consensus address proposes every block. That validator is val-4, so
`miner` has been `0x4207…0782` on every block since genesis. Rounds above 0 do not help, because
`height + round` stays in the first validator's interval. The header's `miner` is correct: it is
the fee recipient of the node that built the block. Only the choice of proposer is wrong.

## What changes at the activation height N

Nothing changes before N. From height N on, the proposer of `(height, round)` is chosen like
this, with slot `s = height + round`:

- **Equal powers** (Enclave today): `validators[s mod n]` in address order. This is plain
  round-robin, and each new round moves to the next validator.
- **Unequal powers**: `k = floor(frac(s / φ) · total_power)` in 64-bit fixed point (φ is the
  golden ratio), then the same walk. Each validator proposes its power share of the slots,
  interleaved rather than in runs.

It is still a pure function of the height's validator set, the height and the round. Nodes
running a binary without the feature reject proposals from the new proposers at N
(`NotTheProposer`). If they hold more than 1/3 of the power, the chain stops at N. **Every
validator must run the new binary before N.** Full nodes and RPC nodes do not choose proposers.
They follow commit certificates, but upgrade them too.

The node accepts `proposerRotationBlock` being added or moved only while the chain head is below
both the old and the new value. After that it refuses to start with a different value. The
genesis hash does not change.

## Procedure

1. **Choose N** at least 1 day out (2 s blocks: about 43,200 blocks per day). N does not have to
   be an epoch boundary. Announce N with the approximate UTC time.
2. **Rehearse**: `cargo test -p xdevs-consensus proposer` and
   `cargo test -p xdevs-testutils --test proposer_rotation`. The second runs 4 validators with
   power 12.5 M and shows one proposer before N, all four from N on, and no fork.
3. **Roll out the release** containing the feature, with the genesis unchanged. The binary then
   behaves exactly like the old one:
   `ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z`.
   Check with `xdevs --version` on every node.
4. **Schedule**: add `"proposerRotationBlock": N` to `config` in
   `deploy/ansible/inventory/testnet/files/genesis.json`, update `xdevs_genesis_sha256`, and
   roll it out one node at a time
   (`ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z -e xdevs_fork_height=N`).
   Each node logs `fork schedule updated from the genesis config` with `proposer_rotation=Some(N)`.
   Also update `deploy/configs/enclave-genesis.json` and
   `tools/genesis-builder/configs/enclave.json` so that new nodes start with the same schedule.
5. **At N**: `miner` of blocks N, N+1, N+2, N+3 cycles through the four fee recipients, rounds
   stay at 0, and `scripts/check.sh --expect progress` shows no fork.

## Rollback

Before N, remove the key and roll out again. After N the schedule is frozen. If the chain halts
at N because too few validators upgraded, upgrade them; do not remove the key.
