# Runbook: rotating a validator's consensus key

Runbook index item 2 (plan §33.8, §11.8, §25.1 "key theft").

> The consensus key signs proposals and votes. Rotating it wrongly is the easiest way to
> double-sign. Rule: at **any** moment, at most **one** running process may hold a given key,
> and a new key may only sign once the chain knows it.

## Which case are you in?

| Situation | Phase 2 (static genesis set, now) | Phase 3+ (staking) |
|---|---|---|
| Routine rotation (hygiene, operator change) | Needs a validator-set change at a fork height (below) | On-chain rotation, effective at an epoch boundary |
| **Key compromised or suspected** | Stop the validator **now**, then as above | Stop **now**, rotate on-chain, accept the downtime |
| Host compromised but the key provably not readable | Treat it as compromised | Treat it as compromised |

### Phase 2 reality

The validator set comes from `genesis.json` (research doc §6.9), and every node checks it
against `Meta["validators"]`. There is **no** in-protocol rotation yet. The options are:

1. **Devnet / early testnet (recommended now):** relaunch the network with a new genesis.
   Testnet tokens have no value. Announce it, and follow
   [bootstrap-validator.md](bootstrap-validator.md) section A.
2. **Keep history:** ship a release with a fork-height validator-set override
   (`validators_at: { <height>: [...] }` in the chain config). **Open issue: not implemented
   in the node.** Once it exists, the procedure is the same as the Phase 3 one below, with
   "epoch boundary" replaced by "fork height".

### Phase 3+ procedure (on-chain rotation)

Assumption, to verify against the Staking contract when Phase 3 lands: a rotation transaction
takes effect at the next epoch boundary (`epochLength` = 3,600 blocks, about 2 h at 2 s).

1. **Generate the new key offline**: `xdevs-cli keygen --consensus --out consensus_key.json`
   (password file). Back it up ([backup-keys.md](../../deploy/scripts/backup-keys.md)).
2. Vault-encrypt it as
   `$XDEVS_KEYS_DIR/validator-N/consensus_key.json.vault` (and `password.vault`). Move the old
   ones to `retired-<date>/`. **Do not deploy it yet.**
3. Submit the rotation transaction from the operator account. Note the activation height
   `H = next epoch boundary`.
4. Shortly **before H**, check the quorum: all other validators must be up.
5. Swap the key only once the chain has moved past H - 1 with the old key still signing. Then:
   ```bash
   ansible-playbook site.yml --limit validator-N --tags keys -e validator_keys_force_replace=true
   ```
   This installs the new key (mode 600), restarts that one validator, and waits for health
   and consensus. The node keeps its `data/consensus/` sign guard. The new key has never
   signed, so it has no history to conflict with.
6. Check: Grafana shows rounds at 0, and the validator's votes appear in commit certificates
   again. Until a per-validator missed-votes metric exists, read them from the RPC or the logs.
7. Destroy the old key: shred it from the workstation and wipe it from the backup drives once
   the new backup is verified. Write it in the ops log.

## Compromise response (any phase)

1. **Stop the validator now.** One validator down out of 4 is tolerated:
   `ansible validator-N -b -m systemd -a "name=xdevs state=stopped"`.
   With an attacker holding the key, the risk is **equivocation**: they can sign conflicting
   votes, but they cannot finalize anything alone with 1 of 4 power.
2. Isolate the host: keep it powered for forensics, or snapshot its boot volume in the Oracle
   console. Revoke its SSH keys, and rotate every secret it held (Grafana token, Object
   Storage key, tunnel credentials) via the vault + `site.yml`.
3. Rotate as above. In Phase 2, the network keeps running on 3 of 4 (75% > 2/3) while you
   relaunch or fork. **Do not** bring up a replacement with the old key.
4. Follow [security-report.md](security-report.md) for disclosure and a post-mortem.
