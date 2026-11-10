# Runbooks

Operational runbooks (plan §33.8). The testnet's infrastructure as code is in
[deploy/ansible](../../deploy/ansible/README.md), with [Cloudflare](../../deploy/cloudflare/README.md),
[monitoring](../../deploy/monitoring/README.md) and [scripts](../../deploy/scripts/) next to it.

1. Bootstrapping a new validator, and launching the network: [bootstrap-validator.md](bootstrap-validator.md)
2. Rotating a validator's consensus key, including compromise response: [rotate-consensus-key.md](rotate-consensus-key.md)
3. Upgrading the network, non-fork and fork-height releases (rolling, never more than 1/3 of the power down): [upgrade-network.md](upgrade-network.md)
4. Chain halted: diagnosis checklist (peers, clocks, more than 1/3 offline, state-root mismatch, determinism bug and emergency halt): [chain-halted.md](chain-halted.md)
5. Restoring a node from a snapshot (and how the daily snapshots work): [restore-from-snapshot.md](restore-from-snapshot.md)
6. Faucet wallet refill and compromise response: [faucet.md](faucet.md)
7. Responding to a security report: [security-report.md](security-report.md)
8. Emergency coordinated restart from a known-good height: [emergency-restart.md](emergency-restart.md)

## Infrastructure

- Oracle Cloud Always Free gotchas (capacity, idle reclamation, Pay-As-You-Go, storage limits, two firewalls): [oracle-always-free.md](oracle-always-free.md)
- Validator key backup (offline, encrypted): [deploy/scripts/backup-keys.md](../../deploy/scripts/backup-keys.md)
- Public RPC: Cloudflare Tunnel, DNS, WAF and rate limits: [deploy/cloudflare/](../../deploy/cloudflare/README.md)
- Dashboards, alerts, metric names and UptimeRobot: [deploy/monitoring/](../../deploy/monitoring/README.md)

## Testing

- Local Docker 4-validator network and chaos commands: [localnet.md](localnet.md)
- Soak test (CI up to 5.5 h; 24 h on the Oracle VMs): [soak.md](soak.md)
- Preparing and running the 24 h soak on the Oracle VMs with Ansible: [soak-oracle.md](soak-oracle.md)
