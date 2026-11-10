# Runbook: responding to a security report

Runbook index item 7 (plan §25, `SECURITY.md`). The policy is public in
[SECURITY.md](../../SECURITY.md): acknowledgement within 72 hours, coordinated disclosure, and
free-tier rewards (credit, tXDS, Discord role).

## 1. Intake (day 0)

- Channels: GitHub **private vulnerability reporting** (Security tab), and the security email
  once it exists (SECURITY.md TODO). Reports in public issues or Discord: thank the reporter,
  move it to a private advisory, then **hide or delete the public post** if it holds exploit
  details.
- Create a **GitHub Security Advisory (draft)**. Everything below happens inside it, including
  the private fork for the fix.
- Acknowledge within 72 h, even if only "received, triaging". Agree on a channel and a rough
  timeline with the reporter.

## 2. Triage (target: 72 h)

Reproduce on a local devnet ([localnet.md](localnet.md)), **never** on the public testnet.

| Severity | Examples (plan §25.1 threat model) | Response |
|---|---|---|
| **Critical** | Consensus split / non-determinism, double-sign bypass of the sign guard, remote crash of validators, forging blocks or signatures, state corruption, unauthorized mint/transfer in system contracts (Staking, Vault, Governance) | Fix immediately. Consider the emergency halt ([chain-halted.md](chain-halted.md) step 5) if it is being exploited |
| **High** | Remote crash of RPC / full nodes, P2P DoS or eclipse that stalls the chain, bypass of XIP-0001 / guardian protections, faucet draining at scale | Fix within days. Rolling or fork-height release |
| **Medium** | RPC resource abuse past the limits, information leaks, issues that need unusual config | Next scheduled release |
| **Low** | Hardening, docs | Normal issue after disclosure |

Ask:

- Is it being exploited now? Check Grafana and the logs: crashes, rounds, odd RPC methods,
  faucet drain rate.
- Does it need a **fork height**? Anything that changes validity or execution does
  ([upgrade-network.md](upgrade-network.md)).
- Are keys or secrets exposed? Rotate them first: [rotate-consensus-key.md](rotate-consensus-key.md),
  the faucet ([faucet.md](faucet.md)), and the vault secrets (Grafana token, Object Storage key,
  tunnel credentials) via `vault.yml` + `site.yml`.

## 3. Mitigate now, fix properly next

Stopgaps that need no release:

- RPC abuse: Cloudflare WAF block or a tighter rate limit
  ([waf-and-rate-limits.md](../../deploy/cloudflare/waf-and-rate-limits.md)). Disable the RPC
  namespace on rpc-1 (`xdevs_rpc_namespaces`, then `site.yml --limit rpc`), or stop
  `cloudflared`.
- Faucet: pause it (set `MIN_FAUCET_BALANCE` very high), see faucet.md.
- P2P: tighten `xdevs_gossip_max_peers` / `max_inbound_peers`, or block source IPs in
  `firewall_extra_rules` (nftables).
- Consensus-critical and exploited: emergency halt, then
  [emergency-restart.md](emergency-restart.md).

Fix in the advisory's **private fork**. Add a regression test that reproduces the report. Run
the full CI and a devnet soak. Get a second review for consensus or crypto code.

## 4. Release and deploy

- Non-consensus: tag a release and run `rolling-upgrade.yml`. Release notes say "security
  fix", with details after disclosure.
- Consensus: fork-height release. Coordinate privately with any external validator operators
  first: share the binary and the fork height, not the exploit.
- Before announcing, check that every node runs the fix (`ansible all -b -m command -a "xdevs --version"`).

## 5. Disclose and credit

- Publish the advisory (request a CVE through GitHub if it applies) once the fix is deployed,
  or at the agreed date (default: 30 days after the fix, 90 days max from the report).
- Credit the reporter in the advisory and the release notes (if they want it). Send the tXDS
  reward and grant the Discord role (plan §25.3).

## 6. Post-mortem (within 1 week)

Timeline, root cause, how it got past review / tests / fuzzing, detection gaps, and follow-up
issues (new invariant tests, fuzz targets, alerts). Link it from the advisory.
