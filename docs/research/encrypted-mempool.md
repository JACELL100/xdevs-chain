# Encrypted mempool: research and prototype (Phase 5)

Status: **prototype, research-grade, not reviewed.** It is off unless a genesis sets
`encryptedMempoolBlock`, and no running network sets it. The plan (§17.2, §25, §29 risk 9)
requires an external cryptographic review before it is enabled anywhere that holds value.

Code: `crates/crypto/src/threshold.rs` (scheme), `crates/crypto/src/encrypted_tx.rs`
(envelope format), `crates/node/src/encrypted.rs` (share pool, block rules),
`crates/node/src/app.rs` (proposal, validation, commit), RPC in `crates/rpc` /
`crates/node/src/backend.rs`. Exit-criterion test:
`crates/testutils/tests/phase5_encrypted_mempool.rs`.

## 1. Problem and goal

A transaction sitting in a public mempool tells everyone what it will do before its
position in a block is fixed. A searcher who sees a large AMM buy places a buy just before
it and a sell just after it (a sandwich); the victim pays the price impact, the searcher
keeps it. Proposers can do the same with their own blocks.

Goal: **the contents of a transaction stay hidden until its position in the chain is
final**, so nobody (peers, RPC operators, the proposer) can place a transaction around it
on the basis of what it does. Fees must still be payable and spam must still cost money.

Non-goals for the prototype: hiding *who* sends (the envelope's sender is public),
hiding that a transaction exists, protecting against a coalition of `t` or more committee
members, and fair ordering among plain transactions.

## 2. Threat model

| Adversary | Capability | Prototype outcome |
|---|---|---|
| Mempool watcher / searcher bot | Sees every gossiped tx, sends its own with any tip | Sees only ciphertext, sender, nonce, fees, size and the committed inner gas. Cannot target a specific trade. |
| Malicious proposer (one validator) | Chooses order and inclusion in its block | Orders envelopes blind. Can censor an envelope (by sender) or delay a height, not read it. |
| Fewer than `t` colluding committee members | Pool their shares early | Learn nothing (Shamir threshold). |
| `t` or more colluding members | Combine shares before commit | **Break confidentiality** (inherent to threshold schemes). Mitigated only by choosing `t` and the committee well (§7). |
| Envelope griefer | Sends garbage / copied / oversized envelopes | Pays gas for each; nothing is decrypted; next block unaffected. |
| Network attacker | Spams fake decryption shares | Shares carry DLEQ proofs; invalid ones are rejected and the peer penalised. |

Assumptions: the consensus assumptions of Phase 2 (fewer than 1/3 Byzantine voting power,
partial synchrony), `t ≤ n − f` so the honest members alone can always decrypt, a trusted
dealer for the prototype key (§7), and the standard hardness of discrete logs on
secp256k1 (CDH, in the random-oracle model for the Fiat-Shamir proofs).

## 3. Options considered

| | A. Threshold encryption to a committee (chosen) | B. Commit-reveal by the sender | C. Private RPC / builder relay | D. Time-lock puzzles / VDF | E. TEEs (SGX/SEV) |
|---|---|---|---|---|---|
| Who can decrypt | `t` of `n` committee members, after commit | The sender, when they reveal | The relay operator, immediately | Anyone, after the puzzle time | The enclave |
| Liveness | As consensus (honest members ≥ `t`) | Sender must come back online; no reveal = no execution | Relay availability | Always (just slow) | Enclave availability |
| Griefing | Envelope pays gas; bad payloads skipped | Free option: commit, then reveal only if profitable (needs bonds) | n/a | Cheap puzzles, expensive solving | n/a |
| Latency | +1 block | +1 block at least, plus the sender's round trip | none | puzzle time (seconds+) | none |
| Trust | `< t` honest-majority committee | none | the operator | timing assumptions on hardware | the hardware vendor |
| Key management | DKG / dealer, rotation | none | none | none | attestation infrastructure |
| Leaks | sender, nonce, fees, size, gas commitment | same, plus whether the sender revealed | everything to the operator | same as A | side channels |
| Fit with xDEVS | Validators already exist and vote every block | Wallet UX burden; the free option is a known MEV vector | Centralising (plan §17.1 lists it as an interim only) | Too slow for 2 s blocks | Hardware dependency, contrary to the $0 budget |

**Why A.** It is the only option that keeps the property under an adversarial sender
*and* without a trusted operator, with the latency the chain already has. B is the
fallback worth keeping in mind: it needs no cryptography beyond hashing, but the sender
can choose not to reveal after seeing the order (a free option that is itself MEV) unless
non-reveal is slashed from a bond, and every user must stay online for a second step. The
envelope format here would support B with a different payload version.

Prior art: Shutter Network (threshold BLS, keypers), Ferveo / Anoma (DKG + TPKE over
BLS12-381, decryption shares in vote extensions), the Osmosis threshold-encryption work,
and Shoup–Gennaro TDH1/TDH2 (CCA-secure threshold ElGamal).

## 4. Cryptography

All primitives come from audited RustCrypto crates already in the dependency graph
(`k256`, `sha2`, `hkdf`, `chacha20poly1305`: the last two were pulled in by libp2p noise,
so no new crate was added). The module composes them; it does not implement a primitive.

Notation: `G` the secp256k1 generator, `H(…)` = SHA-256 over length-prefixed parts with a
domain string, reduced mod the group order.

* **Keys (trusted dealer, prototype).** Random polynomial `f` of degree `t − 1`; committee
  secret `s = f(0)`, public key `Y = s·G`; member `i` (1-based) holds `s_i = f(i)` and
  publishes `Y_i = s_i·G`. Genesis carries `Y`, all `Y_i` and `t`.
* **Encrypt** (hashed ElGamal KEM + AEAD DEM). Random `r`; `U = r·G`; `K = r·Y`;
  `(key, nonce) = HKDF-SHA256(salt = "xdevs/tpke/kdf/v1", ikm = K, info = U ‖ label)`;
  `body = ChaCha20-Poly1305(key, nonce, plaintext, aad = label)`.
* **Non-malleability** (TDH1-style). A Schnorr proof of knowledge of `r`: random `w`,
  `A = w·G`, `c = H(U, A, label, body)`, `z = w + c·r`. Ciphertext = `(U, c, z, body)`.
  Committee members release shares **only** for ciphertexts whose proof verifies for the
  label. Without it an attacker could copy a pending victim's `U` into its own envelope,
  get it decrypted one block early and learn the victim's key.
* **Label** = `"xdevs/encrypted-tx/v1" ‖ chainId ‖ keyId ‖ envelope sender ‖ inner gas
  commitment`. Binding the sender means a payload copied into another account's envelope
  fails the proof; binding the gas commitment means it cannot be changed.
* **Decryption share** of member `i`: `D_i = s_i·U` with a Chaum–Pedersen DLEQ proof that
  `log_G Y_i = log_U D_i` (`(c, z)`, 64 bytes). Anyone can verify it against `Y_i`.
* **Combine**: `K = Σ λ_i·D_i` over exactly `t` shares with distinct indices (Lagrange at
  0), then derive the key and open the AEAD.

Sizes: ciphertext overhead 113 bytes (`U` 33, proof 64, tag 16); a share is 101 bytes on
the wire.

## 5. Protocol

### 5.1 Envelopes

An envelope is an **ordinary signed transaction** to `ENCRYPTED_INBOX_ADDRESS`
(`0x…1100`, no code), so it reuses the pool, gossip, fee market, replacement and receipts
unchanged and plain transactions keep working. Its calldata:

```text
0x01 ‖ keyId (32) ‖ innerGasLimit (8, BE) ‖ U (33) ‖ c (32) ‖ z (32) ‖ AEAD body
```

The plaintext is a complete signed inner transaction (EIP-2718 bytes) for the same chain.
The envelope pays gas for its own inclusion (21,000 + calldata) in block N whatever
happens to the payload. If the envelope and the inner transaction come from the same
account, the inner nonce is the envelope's nonce + 1 (the envelope executes first).

### 5.2 Lifecycle

1. **Block N (order blind).** The proposer orders envelopes like any transaction (by tip),
   seeing only the visible fields. The *decryptable* envelopes of a block are those with
   payload version 1, the current key id, a proof that verifies for their sender, and an
   inner gas commitment within the remaining **budget** (half the block gas limit,
   cumulative in block order). Everything else is skipped after paying its gas.
2. **Commit N, release.** Every committee member that commits N computes one share per
   decryptable envelope and gossips them as one message (prefix byte `0xb1` on the tx
   topic; no EIP-2718 transaction starts with it). Nodes verify each share's proof against
   `Y_i` before keeping or relaying it; shares for a block the node has not committed yet
   are buffered and verified once it has.
3. **Block N+1 (execute first).** The proposer waits (at most half the time to its propose
   deadline) until it holds `t` shares for every decryptable envelope of N, puts exactly
   `t` shares per envelope (the lowest indices) into the block, decrypts, and builds the
   block with the decrypted transactions as the first candidates, in N's order, followed
   by pool transactions.

### 5.3 Block format and validity

From the activation on, `Block::consensus` (bound to the header through `requests_hash`)
is `rlp([stakingData, decryptionData])`; either part may be empty, not both, and a block
with neither keeps empty bytes, so blocks without envelopes look as before.
`decryptionData = rlp([parentHash, [[txIndex, [[shareIndex, D, c, z], …]], …]])`.

Block N+1 is valid only if (in addition to the Phase 2/3 rules):

* it carries decryption data **iff** block N has decryptable envelopes, for N's hash;
* the data lists exactly N's decryptable envelopes in order, each with exactly `t` shares
  with strictly increasing indices in `1..=n`, each proof valid;
* its body starts with exactly the transactions the executor includes when building from
  the decrypted list alone on the parent state (validators rebuild that prefix with the
  same skip rules: wrong nonce, insufficient funds, gas, XIP-0001…). A payload that does
  not open, does not decode, is for another chain or exceeds its gas commitment is
  skipped. The shares are in the block, so full nodes and syncing nodes verify the same
  way without holding a key.

The proposer of N+1 cannot reorder, drop or add decrypted transactions, nor put its own
transactions in front of them.

### 5.4 Activation and configuration

Same pattern as Phase 3's `stakingValidatorsBlock`:

```json
"config": {
  "encryptedMempoolBlock": 1000,
  "encryptedMempool": {
    "threshold": 3,
    "publicKey": "0x02…",
    "verificationKeys": ["0x03…", "0x02…", "0x03…", "0x02…"]
  }
}
```

Absent = never (the default; the live Enclave network is unaffected). A validator's share
is a text file `<index>:<hex scalar>` named by `[validator] mempool_share_file`; the node
checks it against `verificationKeys[index − 1]` at start-up. Full nodes need no share.
The dev-mode single-node producer does not decrypt (envelopes are only ordered).

### 5.5 RPC

* `xdevs_getMempoolEncryptionKey()` → key id, public key, `t`, `n`, inbox, max inner gas,
  payload version, activation block and whether the next block is in the encrypted era.
* `xdevs_sendEncryptedTransaction(rawEnvelope)` → checks the payload (key id, gas bound,
  proof for the sender) and then behaves like `eth_sendRawTransaction`. Encryption happens
  in the client: the RPC node never sees the plaintext.

## 6. Security properties (informal)

* **Confidentiality until order is final.** Shares for block N are released only after N
  is committed (BFT finality: N's order cannot change). Before that the payload is a
  hashed-ElGamal ciphertext under `Y`; fewer than `t` shares reveal nothing about `K`.
* **No early decryption by copying.** The proof of knowledge binds `U` to the label
  (including the sender), so a copied ciphertext is never decrypted; a fresh ciphertext
  needs the encryptor's `r`.
* **Execution integrity.** Every share is DLEQ-verified against the genesis verification
  keys, so a Byzantine member cannot make honest nodes decrypt to a wrong key; with exactly
  `t` valid shares the combination is unique, so all nodes derive the same plaintext and
  the same prefix (`assert_no_fork` in the test).
* **Committed order.** Decrypted transactions run at the start of N+1 in N's order, before
  anything a proposer or searcher could add after seeing them. A searcher who reads the
  plaintext only when shares are gossiped can at most back-run.
* **Griefing costs gas.** Invalid, copied, over-budget or undecryptable envelopes pay
  their inclusion gas and are skipped; the per-block budget keeps the next block's prefix
  bounded.
* **Liveness.** Under `t ≤ n − f`, honest members alone produce enough shares; a proposer
  without them fails to propose and the round times out, the next proposer usually has
  them.

## 7. Limitations (prototype)

1. **Trusted dealer.** Whoever generates the key knows `s` and can decrypt everything.
   Production needs a DKG (Pedersen / Gennaro et al., or Ferveo's) and per-epoch keys.
2. **Static committee.** The committee is fixed in genesis and does not follow the Phase 3
   staking set; a validator leaving the set keeps its share. Keys must rotate at epoch
   boundaries with a hand-over (old committee decrypts envelopes of the last epoch block).
3. **`t`-collusion.** `t` members can decrypt before commit, privately and undetectably.
   Choosing `t` trades this against liveness (`t ≤ n − f`).
4. **Metadata.** Sender, nonce, fees, calldata size and the inner gas commitment are
   visible. A known sender with a known habit can still be front-run *speculatively*
   (a blind sandwich that guesses the direction is a gamble the attacker can lose).
   Padding and relayer-paid envelopes reduce this.
5. **Revealed but skipped.** If a decrypted inner transaction is skipped (e.g. its nonce is
   too high), its signed plaintext is now public and could be replayed later as a plain
   transaction. Wallets must use the next nonce and sufficient funds; a protocol fix is to
   make inner transactions valid only through an envelope (a dedicated signing domain).
6. **Reserved gas is not charged.** The inner gas commitment reserves room in the next
   block but only the envelope's own gas is paid; production should charge
   `commitment × baseFee` up front (refunding unused gas when the inner tx runs).
7. **Censorship.** A proposer can still leave out envelopes from a given sender, as with
   plain transactions; inclusion lists or fair-ordering rules are separate work.
8. **Restart edge.** A member that restarts releases shares for its head block lazily
   (when it next proposes or validates). If more than `n − t` members restart at once,
   the next height waits until they do.
9. **Performance.** One share and proof per envelope per member, verified by every node
   (`t` DLEQ checks per envelope per block). Pairing-based schemes allow one share per
   block (batched TPKE) and constant-size combined keys.
10. **Randomness.** Encryption and proofs use the OS RNG; deterministic nonce derivation
    (RFC 6979-style) would remove that dependency.
11. **Dev mode.** The single-node producer orders envelopes but never decrypts them.

## 8. Evidence

* Unit tests (`xdevs-crypto`): encrypt / threshold decrypt for every `t`-subset, fewer
  than `t` shares, forged and duplicate indices, wrong label, tampered body, copied `U`,
  tampered shares, share file round trip, envelope seal / check / open, copied payload
  under another sender, wrong chain / key / gas commitment.
* Unit tests (`xdevs-node`): decryptable-envelope selection (garbage, copied payload,
  over-budget envelopes skipped), share pool (early buffering, duplicates, forged shares,
  stale blocks, lowest-index selection), deterministic decryption in committed order with
  a junk payload skipped, invalid decryption data (too few shares, unordered, missing,
  bad proof, reordered), inner tx over its gas commitment skipped, block data split / join,
  gossip dispatch.
* Integration (`phase5_encrypted_mempool.rs`, 4 validators, 3-of-4 key, 1 s blocks): a
  mempool bot sandwiches a plain AMM buy (front-run, victim, back-run in that order; the
  attacker ends with more tXDS, the victim gets fewer tokens than quoted). The same buy
  sent with `xdevs_sendEncryptedTransaction` is opaque to the bot, is ordered in block N,
  executes at index 0 of block N+1 and gets exactly the quote; the attacker gains nothing.
  A garbage envelope is refused by the RPC, included through `eth_sendRawTransaction`,
  pays gas and is skipped. All nodes end with the same blocks and state roots.

## 9. Needs external review before any mainnet use

* The composition in `threshold.rs`: KEM/DEM construction and key derivation, the
  proof-of-knowledge binding (TDH1 argument with this label and body), the DLEQ proof and
  its challenge inputs, Lagrange combination and index handling, encoding / identity /
  non-canonical scalar checks, constant-time behaviour of secret-dependent code paths.
* The protocol rules: when shares may be released (only after BFT commit), exactly-`t`
  selection, the validity rules of N+1, the budget, the interaction with the executor's
  skip rules and XIP-0001, and with Phase 3 consensus data in the same field.
* Key lifecycle: replace the dealer with a DKG, epoch rotation tied to the staking set,
  share storage (encrypted at rest like the consensus key), and compromise recovery.
* Economic analysis: speculative sandwiches using visible metadata, the free-option and
  griefing costs, charging for reserved gas, proposer withholding.
* DoS: share gossip rate limits, buffered early messages, per-block envelope counts.

## 10. Next steps

DKG and epoch keys tied to the staking set; shares carried in precommit vote extensions
(Malachite) instead of gossip so they are available exactly when a block is decided;
a pairing-based batched scheme (BLS12-381, as in Ferveo) to cut per-envelope work;
charging for committed gas; an SDK helper and wallet toggle; then a devnet trial and the
external review.
