# Light-client bridges: xDEVS Chain ↔ Ethereum Sepolia (Phase 7 prototype)

Status: **prototype**. It is tested on real Sepolia and Enclave-testnet data and in a two-chain local end-to-end
test. It is **not audited and not deployed**. Plan reference: §22 (Interoperability & Bridges), §10.3
(`ED25519VERIFY`), ADR-0007 (Ed25519 now, BLS later), Phase 7.

Code:

| Part | Path |
|---|---|
| Ethereum light client (on xDEVS) | `contracts/src/bridge/EthereumLightClient.sol` |
| xDEVS light client (on Sepolia) | `contracts/src/bridge/XdevsLightClient.sol` |
| Shared header store, freeze rule | `contracts/src/bridge/ExecutionHeaderStore.sol` |
| Endpoints | `contracts/src/bridge/SepoliaTokenVault.sol`, `contracts/src/bridge/XdevsBridge.sol`, `BridgeBase.sol` |
| Libraries | `contracts/src/bridge/lib/` (RLP, Merkle-Patricia, receipts and storage proofs, SSZ, BLS12-381 over EIP-2537, SHA-512, Ed25519) |
| `ED25519VERIFY` precompile | `crates/execution/src/precompiles.rs`, `docs/specs/precompiles.md` |
| Relayer | `tools/bridge-relayer` |
| Tests | `contracts/test/bridge/*.t.sol` (real fixtures in `contracts/test/bridge/fixtures`), `tools/bridge-relayer/test`, `tools/bridge-relayer/e2e` |

## 1. Why no multisig

The largest bridge losses (Ronin, Harmony Horizon, Multichain, Wormhole's guardian path) came from bridges that
trust a small signer set to attest to the other chain. The signer set is a second consensus, much smaller than
either chain's, whose keys sit in a few places. Plan §22.1 rules it out. In this design:

- Each side runs a **light client of the other chain's own consensus** in a contract. The security of a transfer
  is the security of the source chain's consensus, plus the correctness of the contracts.
- The **relayer is untrusted** and permissionless. It only carries data. Every byte it submits is checked: BLS and
  Ed25519 signatures, SSZ and Merkle-Patricia branches, `keccak256(header)`, the receipt's emitter, event and nonce.
  A malicious relayer can delay a transfer, and anyone can relay instead.
- **Governance cannot move funds.** It can pause the endpoints and, with a timelock, deploy a replacement.

## 2. Threat model

Assets at risk: tokens escrowed in `SepoliaTokenVault`, and the supply integrity of canonical bridged tokens on
xDEVS Chain.

| Assumption | Holds if | If it fails |
|---|---|---|
| Ethereum sync committee: ≥ 2/3 of the 512 members sign only canonical, finalized headers | Ethereum light-client security (the same assumption as every Ethereum light client) | A dishonest 2/3 of one committee could fake a Sepolia lock and mint on xDEVS |
| xDEVS: < 1/3 of the voting power is Byzantine | xDEVS BFT safety | > 1/3 equivocating validators could sign a fake block and release escrow on Sepolia |
| The initial checkpoints are correct | Deployers take them from several independent sources (weak subjectivity) | A wrong checkpoint means a client that follows a fake chain |
| The contracts and libraries are correct | Audit (§10) | Any bug in the verification code is a direct loss |
| The relayer is live | At least one honest relayer (anyone can be one) | Transfers wait; nothing is lost |

Out of scope: censorship by either chain's block producers (it delays proofs), application bugs in tokens bridged
through the vault (fee-on-transfer tokens are handled by crediting the amount received; rebasing tokens are not
supported), and the economics of light-client attacks (sync-committee members are not slashed for signing a
non-canonical header).

## 3. Ethereum → xDEVS (`EthereumLightClient` on xDEVS Chain)

Sepolia runs the **Gloas** fork (EIP-7732, version `0x90000076` from epoch 353024). The client implements the
Gloas light-client protocol (`consensus-specs/specs/gloas/light-client`):

1. **Committee.** The 512 keys of each period's sync committee are loaded once (`loadCommittee`, about 50 KB of
   calldata). The contract checks that every key is on the curve (`G1ADD`), recompresses each key and the sum of all
   keys, and checks them against the period's SSZ `hash_tree_root(SyncCommittee)`. It stores a keccak Merkle root of
   the uncompressed keys and their aggregate.
2. **Update** (`submitUpdate`). The update carries the attested beacon header, the finalized header, the finality
   branch (gindex 735), the finalized execution block hash with its branch in the body (gindex 2856, the
   `parent_block_hash` of the block's payload bid), the participation bits and the aggregate signature. The
   contract checks:
   - `signature_slot > attested.slot ≥ finalized.slot`, and that the signature slot is not in the future (block
     clock);
   - ≥ 2/3 participation (≥ 342 of 512). The aggregate key is the stored aggregate minus the listed non-signers.
     Each non-signer is proven against the stored key root, and the list must match the zero bits exactly;
   - the BLS signature over `signing_root(attested header, DOMAIN_SYNC_COMMITTEE, fork version at signature_slot −
     1, genesis_validators_root)`, with `hash_to_curve` (RFC 9380, `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_`) built
     on SHA-256, MODEXP and `MAP_FP2_TO_G2`, then `PAIRING_CHECK`;
   - both Merkle branches.
3. A rotation (`nextSyncCommitteeRoot` with its gindex-2946 branch) is accepted only from a finalized update whose
   finalized and attested headers are in the same period (the spec's `update_has_finalized_next_sync_committee`).
4. The finalized **execution block hash** is then trusted. Anyone submits the RLP header with that hash
   (`submitHeader`). The store keeps `stateRoot` and `receiptsRoot` by block number and trusts the parent hash, so a
   relayer can walk back to any earlier block for about 25k gas (warm) to 180k gas (cold) per header.
5. `XdevsBridge.mint` proves a receipt in that block, checks that the log is `Locked` from the registered Sepolia
   vault and that its nonce is unused, and mints the canonical token (`CanonicalTokenFactory`, plan §22.3).

Storage proofs (`eth_getProof`) are verified by the same libraries. The Foundry test checks a real Sepolia
account and storage proof against a verified state root.

## 4. xDEVS → Ethereum (`XdevsLightClient` on Sepolia)

1. **Commit certificate** (`submitCommit`). Ed25519 precommit signatures over
   `"XDEVS/VOTE/1" ‖ chainId:u64be ‖ 0x02 ‖ height:u64be ‖ round:u32be ‖ 0x01 ‖ blockHash`
   (`xdevs_consensus::vote_sign_bytes`). The caller passes the full validator set in force at that height. It must
   hash to the stored set, and the signers must be strictly increasing indexes holding more than 2/3 of the voting
   power (`signed × 3 > total × 2`, as in `ValidatorSet::is_quorum`). The block hash is the Ethereum-format header
   hash, so `submitHeader` gives the state and receipts roots directly. xDEVS finality is instant: one certificate
   per block, no waiting.
2. **Ed25519 in Solidity** (`lib/Ed25519.sol`, `lib/Sha512.sol`). RFC 8032 verification with extended
   twisted-Edwards coordinates, Shamir's trick, and MODEXP for the inversion and the square root. Measured cost:
   **~432k gas per signature** with a 66-byte vote message (SHA-512 is roughly 90k of it). A certificate costs
   about n × 450k gas: **1.37M gas for the Enclave testnet's 3-of-4 quorum**, and about 30M for 100 equal
   validators, which is impractical. The relayer submits only as many signatures as a quorum needs.
3. **Verification rule.** Keys are decoded the permissive ZIP-215 way (as consensus does), and the signature check
   is the cofactorless `encode([S]B − [k]A) = R` with `S < L`. For honest signatures this agrees with the ZIP-215
   rule of consensus. On adversarial edge cases (small-order components, non-canonical R) it can only be
   stricter. That can cost liveness, never safety.
4. `SepoliaTokenVault.release` proves the `BurnedForRelease` receipt, checks the emitter and nonce, and pays out
   from per-token escrow accounting, so a release can never exceed what was locked for that token.

### Cheaper paths (later)

- **BLS aggregate commits** (ADR-0007): one pairing check of about 150k gas on Sepolia, whatever the set size.
  This needs consensus to sign with BLS keys.
- **ZK-compressed certificates** (SP1 / RISC Zero, plan §22.2): a SNARK proving "> 2/3 Ed25519 signatures over
  this header" verifies for about 250–300k gas.
- On chains with an Ed25519 precompile, including xDEVS Chain itself from `ed25519VerifyBlock` on, the same check
  costs about 3k gas per signature.

## 5. Validator-set rotation (xDEVS)

xDEVS sets change only at epoch boundaries (`ChainConfig::validator_set_boundary`). The set fixed by boundary block
`B` (Staking `onEpoch()` runs in `B`'s system calls) governs heights `B+1 ..= B+epochLength`. The client mirrors this:

- `validatorSetBoundary(h)` uses the node's formula (`epochLength`, `stakingStart`; key 0 = the genesis set).
- `submitValidatorSet(B, proof)` requires the certified header of `B`, which was signed by the previous set. The
  account and storage proofs against `B`'s state root must show Staking slot 3's `lastEpochBlock == B` (`onEpoch`
  ran in `B`) and slot 4's `validatorSetHash == keccak256(abi.encode(operators, keys, powers))`. The contract then
  applies `set_from_staking`: power = stake / 10¹⁸ clamped to `[1, 2⁴⁸−1]`, keys that are not curve points are
  dropped, and the set is sorted by `sha256(key)[..20]`. If no member is usable, the previous set stays, as on
  the node.
- Sets are proven in order, one boundary at a time, so a long relayer outage needs one proof per missed epoch.

**Known gap (must close before value):** the node reads `consensusValidatorSet()` right after `onEpoch()`, before
`B`'s transactions. The storage proof is against `B`'s post-state. A transaction in `B` that removes an active
member (a validator exit) would change `validatorSetHash` after the node read it. The client would then adopt a
subset of the real set. The safety loss is bounded: forging a certificate for that subset still needs the removed
validator to hold more than half the power, if the removed validator is honest. A certificate can also fail to
reach the subset's quorum, which costs liveness. The fix is a protocol change: commit the next set's hash in
consensus data, for example in the boundary block's header or `BlockConsensusData`, or emit it from a system call
whose receipt is in the block.

The Enclave testnet has not activated staking-driven sets yet: the genesis set governs every height. Rotation is
tested with dev keys over a real-format state trie (`test_dev_validator_set_rotation`).

## 6. Latency and cost

| | Ethereum → xDEVS | xDEVS → Ethereum |
|---|---|---|
| Finality wait | ~2 epochs (~13 min) for Ethereum finality, plus up to one relay interval | none (instant BFT finality); one Sepolia block |
| Per-transfer gas | update ~500–660k (shared by all transfers it covers) + header ~180k + mint ~230k, on xDEVS | certificate ~1.37M today (3 signatures) + header ~180k + release ~150k, on Sepolia |
| Periodic gas | `loadCommittee` ~3.8M every ~27 h (on xDEVS, where gas is cheap) | `submitValidatorSet` ~400k per epoch once staking sets are live (plus that epoch's certificate) |

All numbers are measured in the Foundry tests on real data (`forge test --match-path 'test/bridge/*' -vv` prints
them) and in the local end-to-end test.

## 7. Failure modes

- **Source chain halts.** No new proofs arrive and transfers wait. Escrow is safe. When Ethereum's finality stalls,
  so does `submitUpdate`, by design (no optimistic headers).
- **Equivocation (safety failure).** Two valid certificates for different blocks at one height, two different
  verified headers for one number, or two different next-committee roots for one period. Any of these is proof
  that the source chain's finality failed. The light client **freezes permanently**: `verifiedRoots` reverts, and
  every mint and release depending on it stops. The freeze itself cannot be undone. Recovery is a new deployment
  from a fresh checkpoint, decided by governance through its timelock after a public review.
- **Relayer outage.** Liveness only. Ethereum committees must be rotated period by period, using the beacon
  node's `light_client/updates` history. xDEVS sets must be proven epoch by epoch, which needs archive
  `eth_getProof` at old boundary blocks.
- **Ethereum fork changes containers.** The gindices are Gloas constants, and fork versions are fixed at
  deployment. The next fork that changes the `BeaconState` or `BeaconBlockBody` layout, or adds a fork version,
  needs a new deployment, scheduled before that fork. A production version should take the fork schedule and
  gindices per fork from a timelocked governance list.
- **Governance.** `guardian` (the timelocked Governance contract on xDEVS; a timelock on Sepolia) may only pause
  and unpause. The counterpart address is set once by the deployer. There is no upgrade proxy and no admin
  override of light-client state.

## 8. Comparison with IBC

| | IBC (07-tendermint client) | This prototype |
|---|---|---|
| Header verification | Tendermint light client (skipping verification, trusting period) | xDEVS: sequential per epoch; Ethereum: Altair/Gloas sync-committee protocol |
| Messages | Packets with commitments in the provable store, acks and timeouts, ordered/unordered channels | Logs proven by receipt proofs, one-shot nonces, no acks or timeouts |
| Misbehaviour | Client frozen on submitted misbehaviour evidence | Frozen on conflicting verified data. Anyone can submit the second certificate |
| Trusting period / unbonding | Explicit, so long-range attacks need slashable stake | None yet: an old xDEVS checkpoint plus keys of validators that already unbonded could forge history. Deploy only from recent checkpoints. A production version needs a trusting period below the unbonding period |
| Ecosystem | Standard, many chains | Purpose-built for Ethereum, where IBC needs extra machinery (sync committee or ZK) anyway |

IBC to Cosmos chains (plan §22.2, `ibc-rs`) remains a separate later item. The xDEVS light client here is close to
an IBC client for xDEVS: what it still needs is ICS-23 store proofs, a trusting period and misbehaviour handling.

## 9. Testing

- **Real Ethereum data** (`EthereumLightClient.t.sol`, fixtures from `lodestar-sepolia` and a Sepolia RPC,
  regenerate with `tools/bridge-relayer/scripts/fixtures.ts`). Committee P−1 is a checkpoint. The real
  rotation update signed by committee P−1 proves committee P. The real finality update signed by committee P
  finalizes block 12162341. Its header, a real receipt with its log, and a real `eth_getProof` storage proof
  are then verified. Negative tests: forged signature, tampered attested header, bad finality, execution and
  next-committee branches, insufficient participation, wrong or hidden non-signers, wrong committee keys,
  unknown committee, future slot, stale or replayed update, untrusted header, bad receipt and storage proofs.
- **Real xDEVS data** (`XdevsLightClient.t.sol`). Real Enclave-testnet commit certificates (heights 67 and 43535)
  are verified against the validator set read from Staking, then the header and the testnet's only transaction
  receipt. Negative tests: forged signature, other block hash, height or round, 2 of 4 signatures (insufficient
  power), duplicate signers, inflated power (wrong set), untrusted header, tampered receipt proof, and replay as a
  no-op.
- **Dev-key fixtures** for what the testnet cannot show yet: epoch rotation from a Staking storage proof
  (including a dropped invalid key and power clamping), a rejected old set, bad staking proofs, the freeze on
  conflicting certificates, and the ancestor walk.
- **Endpoints** (`BridgeEndpoints.t.sol`): mint and release, replay, wrong emitter, failed transaction, a foreign
  event, a missing canonical token, release beyond escrow, and pause-only governance.
- **Local end to end** (`npm run e2e` in `tools/bridge-relayer`, CI job `bridge e2e`). Two anvils run the real
  contracts and relayer through lock → mint → burn → release, plus a dry run that sends nothing. A dev sync
  committee and dev validators sign the anvils' real block hashes.
- **Ed25519 / SHA-512**: RFC 8032 and NIST vectors, node:crypto vectors, tampering, non-reduced S. **Precompile**:
  Rust unit tests, plus an EVM-level test that `0x0A00…0001` exists only from `ed25519VerifyBlock` on.

## 10. Before real value: audit and open items

External audit, in priority order:

1. `Ed25519.sol` / `Sha512.sol` (hand-written curve and hash code; differential fuzzing against `ed25519-dalek` and
   `ed25519-consensus`), `BLS12381.sol` (hash-to-curve, compression flags, the non-signer subtraction),
   `MerklePatricia.sol` / `RLPReader.sol` (malformed and adversarial proofs; fuzzing against geth's trie).
2. `EthereumLightClient` against the consensus-specs light-client tests (spec test vectors), including the
   period-boundary and rotation rules, and the fork-schedule handling.
3. `XdevsLightClient` set conversion against `set_from_staking` (differential tests), and the boundary-block gap
   in §5. The protocol fix there is required.
4. The endpoints' accounting and nonce handling. ERC-20 edge cases.

Open items: a trusting period and long-range-attack protection, a governance-managed fork schedule, BLS or ZK
certificates for an affordable Sepolia cost, relayer persistence and fee economics, a cap on transfer rate (a
rate limit bounds the loss from a light-client bug), and monitoring that alerts on any freeze.

## 11. What a testnet deployment would need

1. Pick checkpoints from several independent beacon and xDEVS nodes: a recent finalized Sepolia block root (its
   bootstrap gives the current sync committee root), and the current Enclave validator set at a recent height.
2. Deploy `XdevsLightClient` and `SepoliaTokenVault` on Sepolia, with the Sepolia `guardian` as a timelock.
   Deploy `EthereumLightClient`, `XdevsBridge` and a `CanonicalTokenFactory` on Enclave, with the guardian and
   factory governance as the Governance contract. Then `setCounterpart` on both.
3. Load the current committee, and create canonical tokens for the test assets.
4. Run at least two independent relayers, funded on both chains (`node src/cli.ts … --dry-run` first).
5. Optionally schedule `ed25519VerifyBlock` on Enclave through the coordinated-upgrade runbook. Only contracts on
   xDEVS need it; the Sepolia side cannot use it.
6. Publish the addresses and checkpoints. Set alerts on `Frozen` events and keep a transfer cap until an audit is
   done.
