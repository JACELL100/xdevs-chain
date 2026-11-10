# ADR-0007: Ed25519 for consensus/p2p keys, BLS later

- Status: Accepted
- Plan reference: §6

## Context
Validators sign votes and identify themselves on the p2p network. The signature scheme determines certificate size and verification cost.

## Decision
Use Ed25519 (`ed25519-dalek`) for consensus and p2p keys. At first, a commit certificate is a list of signatures.

## Alternatives
- BLS12-381 aggregate signatures from day one: smaller certificates and cheaper light clients, but slower and more complex, and the common implementation (`blst`) is a C dependency.
- secp256k1: the same scheme as user keys, but with no aggregation benefit and no speed advantage.

## Consequences
- Signing is fast, simple and pure Rust.
- Certificates grow linearly with the validator set.

## Revisit
Move to BLS12-381 aggregate signatures when the validator set grows.
