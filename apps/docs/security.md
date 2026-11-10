# Security

xDEVS Chain is **pre-mainnet software**. No network built from this repository holds real value yet, and nothing has had an external security audit.

## Reporting a vulnerability

**Do not open a public issue for security problems.** Follow [SECURITY.md](https://github.com/JACELL100/xdevs-chain/blob/main/SECURITY.md) in the repository. Until a dedicated security contact is published there, use GitHub's private vulnerability reporting ("Report a vulnerability" under the repository's **Security** tab).

Please include the affected version or commit, reproduction steps, and the impact (for example a consensus split, fund loss, denial of service or an RPC crash). We aim to acknowledge reports within 72 hours (best effort; small team) and coordinate a fix and a disclosure date with you.

**Scope:** the node (`xdevs`), the CLI (`xdevs-cli`), all crates in the repository, and the system contracts. Out of scope: the public dev keys of the local dev chain (chain id 7117) and third-party services.

**Rewards** (free version for now): credit in the release notes, tXDS on the public testnet once it exists, and a "Security Contributor" role. Paid bounties come only with a budget or a mainnet.

## How the project approaches security

- **No admin keys over user funds.** No protocol key, validator or governance vote can freeze, seize or move user funds, and no finalized transaction is ever reversed ([ADR-0006](https://github.com/JACELL100/xdevs-chain/blob/main/docs/adr/0006-safety-in-accounts-and-rpc.md)).
- **Determinism first.** Consensus-critical code avoids floats, unordered iteration and wall-clock reads; every node must compute the same state root.
- **Tested hard.** Unit and end-to-end tests for the node; unit, fuzz and invariant tests for every contract area; a Docker localnet with chaos scenarios (crashes, partitions, packet loss) and a fork scan on every pull request; soak tests under load.
- **Honest limits.** Each spec documents what a feature does *not* protect against.
- **Double-sign protection.** Validators keep a persistent sign guard, separate from the chain database.

## Before any mainnet

Planned, not done: static analysis (Slither, Aderyn) of all contracts, independent external audits of the node and contracts, a public bug bounty, and months of public testnet operation without critical incidents.

## Staying safe as a user

- Never share a seed phrase or private key. xDEVS team members will never ask for one.
- The dev keys shown in these docs are **public**. Never send anything of value to them on any network.
- tXDS has no value. Nobody legitimately sells it.
- Official links are on this site and in the [GitHub repository](https://github.com/JACELL100/xdevs-chain).
