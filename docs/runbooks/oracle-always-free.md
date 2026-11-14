# Runbook: Oracle Cloud Always Free gotchas

Plan §24.1 / §24.2: 4 validators on Ampere A1 (1 OCPU / 6 GB each) and a bootnode on an
E2.1.Micro, all on Oracle Cloud Always Free. These are the traps that bite. Free-tier terms
change, so re-check Oracle's current "Always Free Resources" page before relying on a number.

## 1. "Out of host capacity" when creating A1 instances

A1 capacity is often exhausted in popular regions.

- Always Free compute exists only in your **home region**, which you pick at sign-up and
  cannot change. Prefer a less popular region near your users when you sign up.
- Try every **availability domain** (and fault domain) in the region. Retry at off-peak hours.
- Creating one 1-OCPU instance at a time succeeds more often than one 4-OCPU instance.
- A retry loop with the OCI CLI (`oci compute instance launch ...`, every few minutes, with
  back-off) is common practice. Keep it polite and stop when it succeeds.
- **Upgrading the account to Pay-As-You-Go** (below) noticeably improves A1 availability.

## 2. Idle instance reclamation

Oracle may **reclaim** (stop) Always Free instances it considers idle over 7 days. The criteria
on the docs page when this was written: 95th-percentile CPU < 20%, network < 20%, and memory
< 20% (memory applies to A1). A consensus validator at 2 s blocks on a small testnet can look
idle.

- **Best fix: upgrade to Pay-As-You-Go (PAYG).** Always Free resources stay free on PAYG, and
  PAYG accounts are not subject to idle reclamation. Then:
  - Set a **budget alert** at a tiny amount (e.g. USD 1) in Billing -> Budgets, so any paid
    resource shows up at once.
  - Only create shapes and sizes marked "Always Free-eligible". Delete trial leftovers.
- Without PAYG: the soak's continuous load and normal testnet traffic help. **Do not** run
  CPU-burning "keep-alive" loops on validators; they steal CPU from consensus and can cause
  extra rounds.
- Detection: the `XdevsHostRebooted` / `XdevsNodeDown` alerts, and an instance shown as
  *Stopped* in the console. Start it again (the node starts on boot), then check
  [chain-halted.md](chain-halted.md) if two went down.

## 3. Block storage: 200 GB total, about 47 GB minimum per boot volume

- 4 A1 boot volumes x ~47 GB = ~188 GB. That leaves ~12 GB, **not enough for a 5th boot
  volume** (the E2.1.Micro bootnode also needs ~47 GB). The plan's topology does not fit in
  200 GB as drawn. Options:
  - (a) Put the bootnode on the **GCP e2-micro** (its free tier includes a 30 GB standard
    disk) and skip the Oracle micro;
  - (b) run 3 A1 validators plus 1 micro; or
  - (c) on PAYG, pay for the extra ~35 GB (cents per month at block-volume prices; check the
    current price list).
- Keep the boot volumes at the minimum size. Disk alerts fire at 80% (`XdevsDiskUsageHigh`).
  Validators use `pruning = "full"` (10,000 blocks of state history); bootnodes use `minimal`.
- Object Storage: 20 GB Always Free. 3 snapshots must fit (see
  [restore-from-snapshot.md](restore-from-snapshot.md)).

## 4. Two firewalls

Oracle filters traffic **twice**: the **VCN security list** (or NSG) in the cloud, and the
**OS firewall** in the image (`/etc/iptables/rules.v4`, which ends in REJECT). A port must be
open in both. `roles/common` handles the OS side (nftables, and removes only the final INPUT
REJECT). The VCN side is in the console or the OCI CLI:
[`deploy/ansible/oci/security-list-ingress.example.json`](../../deploy/ansible/oci/security-list-ingress.example.json),
explained in [deploy/ansible/README.md](../../deploy/ansible/README.md#oracle-has-two-firewalls).

## 5. Other gotchas

- **Images**: use Canonical **Ubuntu 24.04** (aarch64 for A1). The release binaries need glibc
  2.39 and do not run on 22.04.
- **E2.1.Micro has 1 GB RAM**. `roles/common` adds a 2 GB swap file on hosts under 2 GB. Keep
  the bootnode's mempool small (`group_vars/bootnodes.yml`).
- **Public IPs**: an ephemeral public IP is released when the instance is terminated (and can
  change on some stop/start paths). Nodes dial `/dns4/*.xdevschain.tech`, so an IP change is
  one DNS edit plus `hosts.yml`. Reserved public IPs avoid this, if your free quota has them.
- **Private IPs** are stable for the life of the VNIC. Validators peer over them. If you
  re-create an instance, update `private_ip` and re-run `site.yml`.
- **Time**: use Oracle's NTP at 169.254.169.254 (the default in `hosts.yml.example`). Check
  with `chronyc tracking`.
- **Metadata service**: keep Oracle's `InstanceServices` iptables egress rules (Ansible does).
  They stop non-root processes, including a compromised node, from reaching instance metadata.
- **Maintenance reboots**: Oracle occasionally live-migrates or reboots hosts and emails the
  tenancy admin. Make sure that inbox is read. Two validators rebooted at once halt the chain
  until they are back.
- **Egress**: 10 TB/month free. P2P plus snapshots are far below that. Public RPC goes out
  through the home machine, not Oracle.
- **Account security**: enable MFA on the Oracle console. Use separate IAM users: one for you,
  one limited to the snapshot bucket (its Customer Secret Key is what the hosts hold; see
  `roles/snapshots/defaults/main.yml`).
