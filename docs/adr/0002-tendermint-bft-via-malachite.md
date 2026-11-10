# ADR-0002: Tendermint-style BFT via Malachite

- Status: Accepted (re-confirmed 2026-11-10 at the start of Phase 2; see docs/research/phase2-consensus.md)
- Plan reference: §6

## Context
A PoS L1 needs fast, deterministic finality with a simple mental model. The Phase-2 check of
license, maintenance and API found that Malachite moved from Informal Systems to Circle: the
crates are now published as `arc-malachitebft-*` (0.8.2, 2026-10-19) from
github.com/circlefin/malachite, license Apache-2.0, used in production by Circle's Arc L1.

## Decision
Use Malachite (`arc-malachitebft-app-channel` + `arc-malachitebft-signing-ed25519`, pinned
`=0.8.2`) through its channel-based app API, in its default `ProposalAndParts` mode:
- the application builds and executes the proposal; validators re-execute every received block
  and answer with `Validity` before prevoting (ADR-0003);
- Malachite's WAL, ValueSync (block sync with commit certificates) and libp2p 0.56 consensus
  network are used as-is; tx/evidence gossip is our own libp2p 0.56 swarm (`xdevs-network`);
- Ed25519 consensus keys (ADR-0007); certificates are lists of signatures stored per height;
- an xdevs `SignGuard` (last signed height/round/step, durable) wraps the signer in addition to
  the WAL (plan §11.8);
- all Malachite types stay inside `xdevs-consensus::malachite`, behind the `ConsensusEngine` /
  `ConsensusApp` traits.

## Alternatives
- commonware (`commonware-consensus` simplex, 2026.10.0): strong and in production (Tempo), but
  pipelined (proposals extend notarized, not finalized, blocks; needs speculative fork state),
  its own runtime and non-libp2p p2p, extra C crypto (aws-lc, blst) and a second RustCrypto
  stack. Remains the fallback and the candidate for BLS/pipelining work later.
- CometBFT (Go) over ABCI: mature, but not a Rust library (conflicts with ADR-0001).
- Writing our own BFT (rejected: high risk). Probabilistic finality (rejected).

## Consequences
- Deterministic finality with CometBFT semantics; block sync and crash recovery come built in.
- Malachite has broken its API in every minor release of 2026 and its public repo is a periodic
  mirror of Circle's fork; it is labelled alpha and unaudited. We pin exact versions, upgrade
  deliberately, test with our own e2e/chaos suite, and can fork (Apache-2.0).
- Our libp2p version is tied to Malachite's (0.56.x).
- Upstream does not test Windows; we add a windows-gnu check to CI.

## Revisit
On each Malachite minor release; if upstream stops releasing for > 6 months or changes license;
in Phase 5 if BLS aggregate/threshold certificates or pipelined execution are needed.
