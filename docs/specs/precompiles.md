# xDEVS Chain precompiles

Plan §10.3. xDEVS Chain runs the Ethereum Osaka precompile set unchanged (`0x01`–`0x11` and `P256VERIFY`
at `0x100`, EIP-7951; KZG point evaluation is unavailable because blob transactions are not supported). xDEVS
additions live in the `0x0A00…` range and are each switched on by a genesis activation block.

| Address | Name | Activation | Status |
|---|---|---|---|
| `0x0A00000000000000000000000000000000000001` | `ED25519VERIFY` | `config.ed25519VerifyBlock` | Implemented (Phase 7 prototype), off by default |
| `0x0A00000000000000000000000000000000000002` | `XDEVS_CONTEXT` | — | Planned |

## `ED25519VERIFY` (`0x0A00…0001`)

Verifies an Ed25519 signature. Used by light clients and bridges that check Ed25519-signed headers, such as
xDEVS commit certificates themselves (`docs/research/light-client-bridges.md`).

- **Input:** `pubkey (32 bytes) ‖ R (32) ‖ S (32) ‖ message (any length)`.
- **Output:** the 32-byte word `0x…01` if the signature is valid. Empty return data otherwise: input shorter
  than 96 bytes, a key that is not a curve point, or a wrong signature. The call never reverts (the same
  convention as `P256VERIFY`), so callers check `returndatasize() == 32`.
- **Verification rule:** ZIP-215, implemented by the `ed25519-consensus` crate. This is the rule the xDEVS
  consensus engine applies to votes, so a contract that verifies commit certificates with this precompile
  agrees with the validators bit for bit. Non-canonical key encodings and small-order components are handled
  the way consensus handles them.
- **Gas:** `3000 + 12 × ceil(len(message) / 32)`. The fee is charged for valid and invalid signatures alike. If
  the call's gas is below the fee, the call halts out of gas. For a 66-byte vote message the fee is 3,036 gas.
  Verifying the same signature in Solidity costs about 430,000 gas (`contracts/src/bridge/lib/Ed25519.sol`).
- **Activation:** present from block `config.ed25519VerifyBlock` on (decimal or hex). If the key is absent, the
  precompile never activates and the address behaves as an empty account (a call succeeds with empty return
  data). The activation block can be scheduled or moved on a live chain only while the chain is below both the
  stored and the new activation height (`crates/node/src/chain.rs`), as for the other fork schedules.
- **Implementation:** `crates/execution/src/precompiles.rs`, with unit tests and an EVM-level gating test
  in `crates/execution/src/tests.rs`.
