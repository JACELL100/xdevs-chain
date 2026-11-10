# Contributing to xDEVS Chain

## Before you open a PR

All of these must pass, and CI runs the same checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Write unit tests for new code. Bug fixes come with a regression test.
- Add short doc comments to public items. No `unsafe` without an ADR.
- Read [docs/specs/phase1-interfaces.md](docs/specs/phase1-interfaces.md) before changing a shared interface, and keep such changes small and additive.
- revm and alloy APIs change between versions. Check the source of the resolved version and do not rely on memory.

## Determinism rules (plan §10.6)

Every node must compute exactly the same result from the same input. In consensus-critical code (execution, state, genesis, block building):

- No floating point.
- Never iterate a `HashMap`/`HashSet`; use `BTreeMap`/`BTreeSet` or sort explicitly.
- No system time except the block header timestamp. No randomness, no environment-dependent behaviour.
- No platform-dependent integer widths in encoded data (`usize` must not reach hashes or encodings).

## Ethereum compatibility

Transaction hashes, RLP, receipts, transaction and receipts roots, and state roots must match Ethereum byte for byte. Changes in these areas need a test against a known Ethereum vector.

## ADR process

Significant decisions (dependencies, storage, consensus, cryptography, protocol rules) are recorded in [docs/adr/](docs/adr/):

1. Copy the format of an existing ADR into `docs/adr/NNNN-short-title.md`, using the next free number.
2. Fill in Context / Decision / Alternatives / Consequences / Revisit.
3. Open a PR. The ADR is accepted when the PR is merged.
4. Never rewrite an accepted ADR's decision. Supersede it with a new ADR and mark the old one `Superseded by NNNN`.

## Protocol changes

Every consensus or execution rule change must be gated by a fork height in the chain config (plan §24.6).

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions are licensed under Apache-2.0.
