# Security Policy

xDEVS Chain is pre-mainnet software (Phase 1, local dev chain only). No network built from this repository holds real value yet.

## Reporting a vulnerability

Please **do not** open a public issue for security problems.

- Email: `security@<your-domain>`. **TODO: the security contact and domain are not set up yet (see plan §1.4). Replace this placeholder before the repository goes public.**
- PGP key: **TODO: publish a PGP key and its fingerprint here.**
- Until the contact exists, use GitHub's private vulnerability reporting ("Report a vulnerability" under the Security tab) if it is enabled.

Please include affected versions/commits, reproduction steps, and the impact (e.g. consensus split, fund loss, DoS, RPC crash).

## What to expect

- Acknowledgement within 72 hours (best effort; small team).
- We coordinate a fix and a disclosure date with you. Consensus-critical fixes may ship as a fork-height release (see plan §24.6).

## Rewards (free version, plan §25.3)

- Credit in the release notes.
- tXDS on the public testnet, once it exists.
- A "Security Contributor" role on Discord.

Paid bounties (for example through Immunefi) will come only once there is a budget or a mainnet.

## Scope

In scope: the node (`xdevs`), the CLI (`xdevs-cli`), all crates in this repository, and (later) system contracts.
Out of scope: the local dev chain's public dev keys (chain id 7117), and third-party services.
