# ADR-0004: Ethereum MPT for state commitment

- Status: Accepted
- Plan reference: §6, §14.3

## Context
`eth_getProof`, light clients and bridges depend on Ethereum's Merkle-Patricia trie (MPT) commitments.

## Decision
Commit state with the Ethereum MPT. Use `alloy-trie` to compute roots over a **flat state** table, incrementally from changed keys.

## Alternatives
- Verkle trees or a custom sparse Merkle tree: possibly faster, but incompatible with Ethereum proofs and tooling.

## Consequences
- State roots and proofs match Ethereum exactly.
- Computing the MPT root is comparatively expensive. Phase 1 recomputes the root in full, which is fine for a small state; incremental computation is required before the state grows.

## Revisit
If root computation still dominates block time after incremental hashing is in place.
