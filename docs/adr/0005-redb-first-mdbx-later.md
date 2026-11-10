# ADR-0005: redb first, MDBX later

- Status: Accepted
- Plan reference: §6, §14

## Context
The node needs an embedded ACID key-value store that cross-compiles easily to ARM64.

## Decision
Use `redb` (pure Rust, ACID, simple). All access goes through the `Database` trait in `xdevs-storage`.

## Alternatives
- MDBX through libmdbx bindings: faster for a large state and used by Reth and Erigon, but it is a C dependency and harder to cross-compile.
- RocksDB: a C++ dependency with a heavy build and many tuning knobs.

## Consequences
- Builds are simple on every target, and storage needs no C toolchain.
- Performance may be lower at large state sizes.

## Revisit
Benchmark against MDBX in Phase 5.
