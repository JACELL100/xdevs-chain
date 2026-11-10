# ADR-0002: Tendermint-style BFT via Malachite

- Status: Accepted (implementation in Phase 2)
- Plan reference: §6

## Context
A PoS L1 needs fast, deterministic finality with a simple mental model.

## Decision
Use Tendermint-style BFT consensus through Malachite, a Rust implementation with a formal specification. Consensus sits behind a `ConsensusEngine` trait so it can be swapped.

## Alternatives
- commonware consensus primitives (the fallback).
- Writing our own BFT from scratch (rejected: high risk).
- Longest-chain or probabilistic finality (rejected: no deterministic finality).

## Consequences
- Deterministic finality, using the model CometBFT has proven for a decade.
- We depend on an external library's license, maintenance and API. **Verify all three before starting Phase 2.**

## Revisit
At the start of Phase 2 (license and maintenance check), and if the library is abandoned or its API blocks us.
