# Runbook: restoring a node from a snapshot

Runbook index item 5 (plan §24.8). Tools: `deploy/scripts/snapshot.sh` / `restore.sh` (installed
as `xdevs-snapshot` / `xdevs-restore`), and `deploy/ansible/restore-from-snapshot.yml`.

## How snapshots are made

- A daily cron job on **one** host (`snapshot_enabled: true`, validator-1 by default per plan
  §24.2) runs `xdevs-snapshot` at 03:17 UTC.
- On a validator it first checks that **every other validator** is advancing (via their
  `/metrics` on the private subnet). If any is not, it skips the run: stopping this validator
  could take 1/3 of the power offline. Skips show up as `XdevsSnapshotStale` after 36 h.
- It stops `xdevs` and archives `genesis.json` and `data/` **without `data/consensus/`**
  (the WAL and sign guard are per-validator and never leave the host). Then it restarts the
  node; the downtime is the time tar + zstd take. It uploads
  `xdevs-testnet-<UTC>-h<height>.tar.zst` with `.sha256` and `.json` (manifest) to Object
  Storage, keeps the newest **3**, and writes the `xdevs_snapshot_*` metrics.
- Object Storage Always Free is **20 GB**: 3 snapshots must fit. Watch
  `xdevs_snapshot_last_size_bytes`, and lower `snapshot_keep` or move the job to the
  `pruning = "minimal"` bootnode if they grow.

Run one by hand: `sudo xdevs-snapshot --env /etc/xdevs/snapshot.env` (`--no-upload --keep-local`
for a dry run). List snapshots: `sudo rclone --config /etc/xdevs/rclone.conf lsl oci:xdevs-testnet-snapshots/testnet`.

## When to restore

- A node's database is corrupted (it refuses to start on a head state-root mismatch, which
  `verify_head` checks at startup), or the disk died.
- A new or replacement machine should catch up faster than syncing from genesis.
- The **monthly restore test** (plan §24.8): restore onto rpc-1 or a scratch VM and confirm it
  syncs.

Not for: rolling back the **chain**. Finalized blocks are final. A snapshot only speeds up one
node's catch-up. Network-wide recovery: [emergency-restart.md](emergency-restart.md).

## Procedure

1. The target host must already be converged (`site.yml`): genesis, binary, config and keys.
2. **If it is a validator**: everything else must be healthy. The playbook's quorum check
   refuses otherwise.
3. Run:
   ```bash
   cd deploy/ansible
   ansible-playbook restore-from-snapshot.yml -e target=rpc-1                       # newest snapshot
   ansible-playbook restore-from-snapshot.yml -e target=validator-3 -e snapshot=xdevs-testnet-20261110T031700Z-h123456
   ```
   On the host, `xdevs-restore` downloads the archive and checks its SHA-256. It checks that the
   archive's `genesis.json` is byte-identical to the host's (same chain), stops the node, moves the
   old `data/` to `data.pre-restore-<UTC>`, unpacks, copies the host's **own**
   `data/consensus/` back, fixes ownership, starts the node, and waits until it is healthy
   (`--wait-healthy`, 900 s). For validators the playbook then runs the back-in-consensus check.
4. The node re-syncs from the snapshot height to the tip (ValueSync re-executes every block, so a
   bad snapshot cannot inject bad state without failing root checks).
5. Once healthy, delete `data.pre-restore-*` to free the disk.

### Fresh machine for an existing validator (no `data/consensus/` yet)

The restore refuses: an empty sign guard means the key has no memory of what it already
signed. Only continue if **all** of these are true:

- [ ] The old machine with this key is stopped, disabled and terminated (or its
      `/var/lib/xdevs/keys` is deleted). Confirmed in the Oracle console, not assumed.
- [ ] Nobody else runs this key (one operator, one key).

Then add `-e allow_empty_signguard=true`. The node only signs at the current height after it
has caught up, so the remaining risk is the old machine coming back, which the checklist rules
out.

## Restore by hand (no Ansible)

```bash
sudo xdevs-restore --env /etc/xdevs/snapshot.env --latest
sudo xdevs-restore --file /path/xdevs-testnet-...-h123.tar.zst    # .sha256 next to it
```

## Monthly restore test log

Record: date, snapshot name, target host, time to healthy, blocks re-synced, and any issue. A
failed test is a P1: backups you cannot restore are not backups.
