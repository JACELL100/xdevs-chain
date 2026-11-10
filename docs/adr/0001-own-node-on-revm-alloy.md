# ADR-0001: Build our own node on revm + alloy (not a Reth fork)

- Status: Accepted
- Plan reference: §6

## Context
xDEVS Chain needs an EVM-equivalent execution layer with full control over consensus, transaction types and safety features, and the binary must run on free-tier ARM64 VMs.

## Decision
Build our own node in Rust, using `revm` for EVM execution and `alloy` for Ethereum types, RLP and tries. Storage, RPC and sync are written in this repository.

## Alternatives
- **Reth SDK + custom consensus via Engine API:** battle-tested storage, RPC and txpool, but heavy on RAM and disk, a large codebase to learn, and custom tx types are harder.
- **Cosmos SDK + EVM module (Go):** mature PoS and governance modules, but Go rather than Rust and less control over EVM details.

## Consequences
- Full control and a small binary; the team learns every layer.
- We must build and maintain storage, RPC and sync ourselves.
- revm and alloy APIs change often, so versions are pinned in `Cargo.lock` and upgraded deliberately.

## Revisit
At the end of Phase 3. If storage or RPC become the bottleneck, migrate execution to Reth components.
