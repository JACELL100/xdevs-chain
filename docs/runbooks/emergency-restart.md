# Runbook: emergency coordinated restart from a known-good height

Runbook index item 8 (plan §24.6 "emergency halt & restart", §25.1 non-determinism). Use it
after [chain-halted.md](chain-halted.md) step 5 (emergency halt), or when the chain halted at a
fork height and will not resume.

> BFT finality means **finalized blocks are never reverted** in normal operation. Every
> restart below keeps the finalized history, except the last-resort path (D), which is a
> deliberate, announced break of that promise. Only use it on the testnet.

## Terms

- **Last finalized height F**: the highest block with a commit certificate, the same on all
  honest nodes (`eth_blockNumber` on each; `scripts/check.sh` confirms they agree).
- **Known-good height G**: the highest height where every node agrees on block hash **and**
  state root. Normally G = F. With a determinism bug, G < F is possible on some nodes only.
- **Fixed release**: a binary that resolves the cause and was tested on a devnet that replays
  the recorded blocks on x86_64 **and** ARM64.

## A. Halt (if not already)

```bash
cd deploy/ansible
ansible validators -b -m systemd -a "name=xdevs state=stopped"
ansible rpc -b -m systemd -a "name=cloudflared state=stopped"    # optional: stop serving stale data
```

Post on Discord and the status page: "Testnet halted for an emergency fix, no action needed by
users; ETA ...". Keep the evidence: logs, plus a `tar` of `data/` from two nodes that disagree.

## B. Find F and G

With the nodes stopped, start them **one at a time, with the RPC on localhost only and the
consensus port blocked**, to read their stores. Or read the evidence copies on a scratch VM with
the same binary:

```bash
for n in v1 v2 v3 v4; do ...; done   # eth_getBlockByNumber at F, F-1, ... : compare hash + stateRoot
XDEVS_RPC_URLS="..." scripts/check.sh --samples 50   # first disagreeing height
```

Record F, G, and which nodes diverge.

## C. Restart paths (prefer the first that applies)

### C1. Nobody diverged; the network just stopped (bug in liveness, crash at height F+1)

1. Ship the fixed release (non-fork if the fix does not change validity).
2. Install it on all validators **while they are stopped**, and start none yet.
   `xdevs_service_state=stopped` installs the binary and config, keeps the service down, and
   skips the quorum and health checks:
   ```bash
   ansible-playbook site.yml --limit validators -e xdevs_version=vFIX -e xdevs_service_state=stopped
   ansible validators -b -m command -a "xdevs --version" -o     # all on vFIX
   ```
3. Start all validators within a short window. Like a genesis launch, F+1 needs more than 2/3:
   `ansible validators -b -m systemd -a "name=xdevs state=started"`.
4. Check: blocks resume from F+1, rounds settle, and `scripts/check.sh --expect progress`
   passes. Restart `cloudflared`.

### C2. Some nodes diverged locally, but the finalized chain (certificates) is consistent

The majority holds the canonical state at F. A minority node computed a different root, or
stored something bad.

1. Fixed release on all nodes (as in C1).
2. For each diverged node: [restore-from-snapshot.md](restore-from-snapshot.md) from a snapshot
   at a height at or below G. It keeps its own `data/consensus`. Then let it re-sync and
   re-execute up to F with the fixed binary. Root checks must pass.
3. Start the healthy majority first, then the restored nodes.

### C3. A bad block was finalized (execution rule bug), and all honest nodes agree on it

The chain is consistent but wrong. **Fix forward**: ship a fork-height release whose rules
correct the effect at height N > F (for example a state patch at the fork). History stays
intact. Use [upgrade-network.md](upgrade-network.md), fork-height section, with the network
restarted under C1 first if it is halted.

### D. Last resort (testnet only): roll the chain back to G

Only if C1 to C3 are impossible. For example, the finalized state is corrupt and cannot be
patched forward.

- **Open issue: no `xdevs db rollback --to <height>` tool exists yet.** Until it does, a
  rollback means a **relaunch**: a new genesis (optionally exporting state at G, which is also
  not implemented), new chain data on every node, and a public announcement that blocks after G
  are void.
- The sign guard on each validator refuses to sign heights at or below the last height it
  signed. After a rollback to G < F, validators would refuse heights G+1..F. A relaunch with a
  new genesis (and a new chain id or a new `name`) avoids reusing the old signing domain. Never
  delete `data/consensus/` to "unstick" a validator on the **same** chain: that is how
  double-signs happen.

## E. After the restart

- Remove the status-page notice and post a summary.
- Post-mortem (see [security-report.md](security-report.md) section 6 if it was a
  vulnerability). Add a determinism regression test that replays the offending block on both
  architectures.
- Publish a new trusted checkpoint (height:hash) after the restart (plan §11.7).
