# Phase 2 research: BFT consensus library and P2P networking

- Status: research note and recommendation, input to ADR-0002 (see §9 for the draft ADR update)
- Date: 2026-11-10
- Plan references: §6 (ADR-0002/0003/0007), §7.3, §11, §12, §27 Phase 2
- Method: crate metadata from `cargo search` / `cargo info`; published crate sources read under
  `~/.cargo/registry/src/*/arc-malachitebft-*-0.8.2/` and `commonware-*-2026.10.0/`; dependency
  graphs resolved with `cargo generate-lockfile` in throw-away probe manifests (nothing was
  built); GitHub repos, release notes and CI files fetched on 2026-11-10. Sources are in §11.

---

## 1. TL;DR

**Keep Malachite.** It has moved: the crates are now published as
**`arc-malachitebft-*` 0.8.2** (2026-10-19), the repo is **`circlefin/malachite`**, and the team
from Informal Systems is now part of Circle. It is still Apache-2.0. It runs Circle's Arc L1,
which launched a permissioned mainnet in September 2026. Malachite matches plan §11 almost
exactly:

- Tendermint propose/prevote/precommit with locking.
- Linear timeouts per round.
- A validator set and timeouts per height (`HeightParams`).
- Proposer selection defined by the application.
- A per-height `target_time`, which gives us the "~2 s block time" knob.

ADR-0003 (execute-before-vote) is supported directly. The application answers each received
proposal with a `ProposedValue { validity }`, so validators re-execute the block before they
prevote. Malachite also provides the parts we would otherwise have to write ourselves:

- A WAL that is fsynced before any signed message leaves the node.
- Value sync: block sync with commit certificates, for lagging or late-joining nodes.
- A libp2p 0.56 network for consensus traffic (gossipsub, Kademlia discovery, persistent peers,
  validator proofs).
- An Ed25519 signing scheme.

It runs on tokio, so it fits our runtime.

**Main risks:**

- **API churn.** There were three breaking releases in 2026 (0.7.0 in June, 0.8.0 in August,
  0.8.1 in October).
- **Upstream is a mirror.** The public repo now receives periodic "sync vX to malachite" drops
  from Circle's internal fork.
- **Alpha and unaudited.** The README calls it "alpha software" and says it "has not been
  externally audited".
- **Old libp2p.** It pins libp2p **0.56.0** (2025-06); 0.57.1 is current.
- **No Windows CI upstream.**

We contain these risks by pinning exact versions, by keeping all Malachite types inside one
crate behind the `ConsensusEngine` trait, by adding our own Windows-gnu CI job, and, because the
license is Apache-2.0, by being able to fork if we have to.

**commonware stays the fallback, but it is a worse fit for Phase 2:**

- Simplex is *pipelined*: a block builds on a notarized parent that is not yet final. Our
  linear redb chain store would need speculative fork state.
- It brings its own runtime and its own (non-libp2p) p2p stack.
- It adds a second RustCrypto major-version stack and C dependencies (`aws-lc-sys`, `blst`,
  `zstd-sys`).
- It ships monthly CalVer releases with breaking changes.
- Its CI covers Linux and macOS only.

## 2. Comparison table

| | **Malachite** (`arc-malachitebft-*`) | **commonware** (`commonware-consensus` + `-p2p`) | Others (§5) |
|---|---|---|---|
| Latest version | **0.8.2** (2026-10-19); 0.8.1 (2026-10-06); 0.7.0-pre (2026-02-27) on crates.io under the new name; old `informalsystems-malachitebft-*` stops at 0.5.0 | **2026.10.0** (2026-10-31); monthly CalVer since 2026.2.0; 0.0.x before that | tendermint-rs 0.41.0 (2026-10-26, types and ABCI only); hotstuff_rs 0.4.0 (2024-12); hotmint 0.8.14 (2026-10, GPL-3.0) |
| Repo / org | `github.com/circlefin/malachite` (moved from informalsystems; team joined Circle in Aug 2025) | `github.com/commonwarexyz/monorepo` | |
| License | Apache-2.0 | MIT OR Apache-2.0 | |
| Activity (last 6 months) | Active through 2026-05/06 (sync fixes, WAL fixes, peer scoring). Since then mostly periodic "sync vX" drops from Circle's internal fork: 2026-08-27, 10-06, 10-19 | Very active, with daily commits (last seen 2026-11-09) | |
| Production users | **Circle Arc**: permissioned PoA mainnet with 11 institutional validators, launch targeted for and reported on 2026-09-16 | **Tempo** (Stripe/Paradigm L1) runs commonware Simplex in production | |
| Algorithm | Tendermint (propose / prevote / precommit, locking, POL), the same as plan §11.1 | Simplex: 2-hop block time, 3-hop finality, pipelined, with notarize / nullify / finalize | |
| Formal spec | English and **Quint** specs in `specs/`; the Tendermint core was co-designed with the Quint spec and model-based tests | Quint workflow present; per-primitive stability levels; simplex is **BETA** ("wire and storage formats stable") | |
| App API | `app-channel`: one tokio `mpsc::Receiver<AppMsg>`. Variants: `ConsensusReady`, `StartedRound`, `GetValue`, `ReceivedProposalPart`, `Decided`, `Finalized`, `GetDecidedValues`, `ProcessSyncedValue`, `RestreamProposal`, `GetHistoryMinHeight`, `ExtendVote`, `VerifyVoteExtensions`. Each carries a oneshot reply | Traits: `Automaton::{propose, verify}`, `CertifiableAutomaton::certify`, `Relay`, `Reporter`, plus the `marshal` actor for block dissemination and finalized-block delivery | |
| Execute-before-vote (ADR-0003) | **Native.** The app validates a full proposal and replies `ProposedValue { validity: Valid / Invalid }`; `Invalid` leads to a nil prevote. Decided blocks are always ones the app validated | `verify()` may execute, but the parent is only *notarized and certified*, not final. The app must execute on top of unfinalized ancestors, which means a speculative state per fork branch | |
| What the app implements | `Context` types (Height, Address, Value / ValueId, Vote, Proposal, ProposalPart, Validator, ValidatorSet, Timeouts, SigningScheme), `select_proposer`, codecs (network, WAL, sync), `Signer` / `Verifier`, proposal streaming | Digest / Block types, `Automaton`, signing scheme selection, a `marshal` storage config | |
| Proposal transport | App streams `ProposalPart`s (Init / Data / Fin) through Malachite's network (`ProposalAndParts`, the default and tested mode) | `broadcast` / `marshal` disseminate blocks; consensus orders digests | |
| WAL / crash safety | Built in (`arc-malachitebft-wal`, CRC32, `sync_all`). The engine **flushes the WAL before publishing any signed message** and before deciding | Journals in `commonware-storage`; the voter persists its own votes | |
| Block sync | Built in (**ValueSync**): status gossip, range requests, peer scoring. App serves `GetDecidedValues` and validates `ProcessSyncedValue`; the engine verifies the commit certificate | `marshal` plus `resolver` backfill finalized blocks with certificates | |
| Networking | **Brings its own libp2p 0.56** (tcp/quic, noise, yamux, gossipsub or broadcast, kad discovery, identify, request-response, persistent peers, validator proofs). No tx or mempool channel | `commonware-p2p::authenticated`: its own encrypted protocol between peers known by public key. Not libp2p, and no DHT | |
| Ed25519 | `arc-malachitebft-signing-ed25519` (on `ed25519-consensus` 2.1, ZIP-215 rules); secp256k1 / ECDSA variant too | `simplex::scheme::ed25519`; also secp256r1, BLS multisig, BLS threshold (+VRF) | |
| Timeouts | `LinearTimeouts { propose, propose_delta, prevote, prevote_delta, precommit, precommit_delta, rebroadcast, max_timeout }`, set per height | Leader / advance / retry timeouts, skip policy | |
| Validator set changes | Per height via `HeightParams { validator_set, timeouts, target_time, vote_extension_policy }` | Per epoch (epoch-aware types; `reshare` / `dkg` for threshold keys) | |
| Runtime | tokio (needs ≥ 1.47.1) plus `ractor` 0.15 actors inside the engine | Own `commonware-runtime` abstraction (tokio backend BETA, io_uring ALPHA, deterministic simulator) | |
| MSRV / edition | `rust-version = 1.88`, edition 2021 | Workspace `rust-version = 1.95.0`, edition 2024 | |
| Windows | Upstream CI is Linux only. The WAL uses `advisory-lock` (works on Windows). libp2p is compile-tested on windows-msvc only. **Not yet verified on our windows-gnu** | CI is Ubuntu and macOS only. Pulls `aws-lc-sys` (cmake + C), `blst` (C/asm), `zstd-sys` (C) | |
| Extra crates vs our current graph (probe lockfile) | +248 packages (401 to 649). One libp2p (0.56.0); tokio stays 1.54.1; `ed25519-dalek` 2.2 shared with libp2p-identity; new duplicates are minor (`thiserror` 1+2, `yamux` 0.12+0.13, `unsigned-varint`, `socket2`, `hashlink`, `uint`, `chacha20`) | +186 packages (401 to 587), but duplicates the whole RustCrypto stack (`digest`, `elliptic-curve`, `ecdsa`, `p256`, `sha2`, `hmac`, `der`, … new majors), plus `curve25519-dalek` 5 and `rand` 0.10 | |
| Fit for xdevs Phase 2 | **High** | Medium-low now; revisit for BLS, pipelining or sub-second finality (Phase 5+) | |

## 3. Findings per question

### 3.1 Malachite

- **Crates and versions.** Malachite is published as `arc-malachitebft-*` at 0.8.2.
  - Crates: `core-types`, `core-state-machine`, `core-votekeeper`, `core-driver`,
    `core-consensus`, `engine`, `app`, `app-channel`, `network`, `discovery`, `sync`, `wal`,
    `codec`, `config`, `metrics`, `peer`, `signing`, `signing-ed25519`, `signing-ecdsa`,
    `engine-byzantine`, `proto`.
  - `cargo info` reports `rust-version: 1.88` and `license: Apache-2.0` for all the crates we need.
  - The library crates have no `build.rs`, so `protoc` is not needed. The protobuf codec lives
    only in the test app.
- **Maintenance.** These are the release tags (from GitHub releases and `BREAKING_CHANGES.md`):

  | Tag | Date | Change |
  |---|---|---|
  | v0.5.0 | 2025-08-01 | |
  | v0.7.0 | 2026-06-22 | Crates renamed to `arc-malachitebft-*` |
  | v0.8.0 | 2026-08-27 | |
  | v0.8.1 | 2026-10-06 | |
  | v0.8.2 | 2026-10-19 | No breaking changes |

  - Commits on `main` from April to June 2026 include real work: sync rollback fixes, "reject
    certificates with any invalid signature", WAL reset fix, peer-scoring-based connection
    prioritisation, and "port downstream fork commits upstream".
  - Since then, commits are periodic `chore: sync v0.8.x to malachite`. Development now happens
    in Circle's fork and is pushed to the public repo in batches.
  - The README still says *"alpha software and under heavy development"* and *"has not been
    externally audited"*.
- **Production.** Arc (Circle) uses Malachite. Arc's mainnet launch was announced for
  2026-09-16 with 11 permissioned institutional validators, run as PoA with PoS planned later.
- **Formal spec.** The `specs/` directory holds English and Quint specs. CI has `quint.yml` and
  `mbt.yml` (model-based testing) workflows.
- **Integration (app-channel).** `start_engine(ctx, cfg, WalContext, NetworkContext,
  ConsensusContext, SyncContext, RequestContext)` returns `Channels { consensus:
  Receiver<AppMsg>, network: Sender<NetworkMsg>, events, requests, net_requests }`. Some details
  from the 0.8.2 sources:
  - **What the app must do.** It must define the `Context` types and codecs, and implement
    `Signer` / `Verifier`. `Signer` is an *async* trait (`sign_vote`, `sign_proposal`,
    `sign_vote_extension`, `sign_validator_proof`), which gives us a clean hook for the
    double-sign guard.
  - **Proposal streaming.** The app builds the value and also publishes its parts through
    `NetworkMsg::PublishProposalPart`. Received parts arrive as `AppMsg::ReceivedProposalPart`.
    The app reassembles them and answers with `Some(ProposedValue{..validity})` once the
    proposal is complete.
  - **Host connector is serial.** The connector awaits each app reply in turn, so a slow
    `validate` delays other host messages. The consensus actor itself is not blocked
    (`call_and_forward`).
  - **Decided vs Finalized.** `Decided { certificate }` asks the app to commit and acknowledge.
    `Finalized { certificate (possibly more signatures), evidence }` must be answered with
    `Next::Start(h+1, HeightParams)` or `Next::Restart`.
  - **Sync.** `GetDecidedValues(range)` → `Vec<RawDecidedValue { certificate, value_bytes }>`.
    `ProcessSyncedValue` → `SyncedValueOutcome::{Verdict, PeerFault, LocalTransientError}`.
  - **WAL.** The default WAL actor writes `<path>` with the app's `WalCodec` and stores signed
    votes and proposals, timeouts, `ProposedValue`s and polka certificates.
    `wal_flush` runs before `PublishConsensusMsg`, before `StartRound` and before `Decide`
    (`engine/src/consensus.rs`). On restart, the WAL is replayed immediately (since 0.8.1).
  - **Networking.** Malachite brings its own libp2p network.
    - Channels: `Consensus`, `ProposalParts`, `Sync` and `Liveness`. Their names can be
      configured, e.g. `/xdevs/<chain>/consensus`.
    - Pubsub is gossipsub or a "broadcast" protocol (`libp2p-scatter`).
    - Kademlia discovery is optional, persistent peers are supported, and a
      `persistent_peers_only` mode exists.
    - Validator proofs (Malachite ADR-006) bind the consensus key to the libp2p `PeerId`.
    - It can be swapped through `NetworkBuilder::Custom`.
    - **There is no transaction or mempool channel**, so tx gossip is ours to build (§6.3).
  - **Ed25519.** The `signing-ed25519` crate uses `ed25519-consensus` 2.1.0. Keys are 32-byte
    seeds, so one seed produces the same public key as `ed25519-dalek`. Consensus paths must
    verify with `ed25519-consensus`, which applies ZIP-215 rules.
  - **Timeouts.** `LinearTimeouts` gives `base + delta × round`, capped by `max_timeout`
    (default 60 s). Defaults are propose 3 s, prevote 1 s, precommit 1 s. We set the plan §11.2
    values per height.
  - **Block time.** `HeightParams.target_time`: if a decision comes earlier, the engine waits
    for the rest of the period (collecting late precommits) before `Finalized`. This plays the
    role of `timeout_commit`. Decisions reached through sync skip the wait.
  - **Validator set per height.** Supported through `HeightParams`, returned from
    `ConsensusReady` and `Finalized`.
  - **Execute-before-vote.** Supported (see the table). Validity is the app's verdict on the
    full block.
- **Windows.** Malachite is pure Rust apart from what libp2p pulls in: `ring` 0.17.14 through
  rustls/quinn. `ring` is *already* in our `Cargo.lock`, and the dev machine has MinGW-w64 gcc
  13.2 in `C:/mingw64/bin`. The WAL locks files through `advisory-lock` 0.3, which supports
  Windows. Nothing in Malachite is gated on `cfg(unix)`. However, Malachite's CI runs only on
  Linux, and rust-libp2p's CI only *compiles* on `x86_64-pc-windows-msvc`. Our first build on
  windows-gnu is therefore unverified (no build was run for this note). That build is task 0 of
  WP1.
- **MSRV and dependency weight.** MSRV 1.88 is below our 1.91 floor. The probe resolution
  (§2) shows:
  - `tokio` 1.54.1, `rand` 0.8.8 and `ed25519-dalek` 2.2.0 are shared with our graph.
  - Malachite adds a single `libp2p` 0.56.0 (gossipsub 0.49.5, kad 0.48.0, quic 0.13.1,
    identity 0.2.14), `ractor` 0.15.13 and `prometheus-client` 0.23.1.
  - **Constraint:** our own libp2p code must use **libp2p 0.56.x**. Using 0.57 would compile
    two libp2p stacks.

### 3.2 commonware

- **Versions.** Crates are `commonware-*` **2026.10.0**, published 2026-10-31. They have moved
  to monthly CalVer (2026.2.0 → 2026.10.0). The license is MIT OR Apache-2.0. The repo is very
  active (daily commits; the last ones seen on 2026-11-09 are QMDB storage work).
- **Production.** Tempo uses "Simplex Consensus, implemented by Commonware" (Tempo docs).
  Tempo's validator set is permissioned and its block time is about 600 ms.
- **Stability.** Each primitive carries a stability label:
  - `consensus::simplex`, `marshal`, `p2p::authenticated` and `runtime::tokio` are **BETA**:
    "wire and storage formats stable; breaking changes include a migration path".
  - `aggregation`, `p2p::simulated`, `runtime::deterministic` and io_uring are **ALPHA**.
- **Primitives.** Available modules:
  - `consensus::simplex`, with Ed25519, secp256r1, BLS12-381 multisig and BLS12-381 threshold
    (+VRF) schemes.
  - `consensus::marshal` (block dissemination and backfill).
  - `consensus::aggregation`.
  - `broadcast`, `resolver`, `p2p` (authenticated, with a simulated network), `storage`
    (journals, archives, QMDB), `dkg`, `reshare` and `runtime`.
- **API style.** The library uses traits and actors over its own `Runtime` / `Spawner` /
  `Clock` context:
  - `Automaton::propose(context) -> oneshot::Receiver<Digest>`.
  - `verify(context, payload) -> oneshot::Receiver<bool>` (must be deterministic and must not
    report temporary failures as `false`).
  - `CertifiableAutomaton::certify` (after notarization, before finalize).
  - `Reporter` (receives finalizations).
- **Networking.** `commonware-p2p` is *not* libp2p. It is its own authenticated, encrypted
  protocol with peers identified by public key and peer sets fed by the app. It has no DHT, no
  gossipsub peer scoring, and no compatibility with libp2p tooling.
- **Fit with execute-before-vote.** `verify()` can re-execute, and Tempo does exactly that with
  reth. But Simplex lets view `v+1` build on a block that is only notarized and certified, while
  finalization lags by one or more views. Our Phase-1 storage is a single linear chain (flat
  state plus reverse diffs, one writer committing finalized blocks), so we would need in-memory
  state overlays for unfinalized branches and a way to discard them. That is a sizeable new
  subsystem. The "safety in finality" model of ADR-0006 also gets more complicated
  (notarized-but-not-final heads).
- **Windows and dependencies.** CI runs on Ubuntu and macOS only. `commonware-cryptography`
  always pulls `aws-lc-rs`/`aws-lc-sys` (needs cmake and a C toolchain on windows-gnu) and
  `blst`. `commonware-storage` pulls `zstd-sys`. These are exactly the kinds of C backends the
  workspace removed from revm. `rand` 0.10 and new majors of `digest`, `elliptic-curve` and
  `p256` would sit alongside the 0.13 stack that alloy and revm use (see the workspace comment
  on `k256`/`p256`).

### 3.3 Other Rust BFT options (brief)

- **CometBFT (Go) + `tendermint-abci` 0.41.0 (Rust).** Mature and maintained, with the same
  Tendermint semantics. However, the consensus engine is a Go process. That breaks the "own
  node, pure Rust" direction of ADR-0001 and doubles the operational surface.
- **hotstuff_rs 0.4.0** (ParallelChain, Apache-2.0). Last release 2024-12-29; effectively
  dormant.
- **hotmint 0.8.14** (HotStuff-2, first released 2026-03). Young (about 1.1k downloads) and
  **GPL-3.0-only**, which is incompatible with our Apache-2.0 workspace.
- **HotShot (Espresso), Aptos / Sui (Jolteon, Mysticeti), monad-bft.** These are embedded in
  large application monorepos rather than offered as standalone libraries.

None of these beats Malachite on spec, maintenance, license and fit together.

### 3.4 libp2p

- **Current versions.** `libp2p` 0.57.1 (2026-10-15, MSRV 1.88; 0.57.0 2026-09-11),
  `libp2p-gossipsub` 0.50.1, `libp2p-kad` 0.49.0, `libp2p-quic` 0.14.0.
- **Malachite pins `libp2p` 0.56.0** (2025-06-27, MSRV 1.83). That release contains
  gossipsub 0.49.x (v1.1/v1.2 with peer scoring), kad 0.48.0, quic 0.13.x (quinn 0.11,
  rustls 0.23), tcp 0.44, noise 0.46, yamux 0.47 and identity 0.2.12+ (ed25519-dalek 2).
- **Platforms.**
  - TCP, Noise and Yamux are pure Rust and work on Windows and Linux.
  - QUIC goes through quinn plus rustls/ring. quinn supports Windows. `ring` already builds in
    our workspace on windows-gnu with the installed MinGW gcc.
  - rust-libp2p CI runs tests on Linux only and compile-checks windows-msvc.
  - Recommendation: run **TCP as the default transport** in Phase 2 (simplest in Docker, NAT
    and Windows) and enable QUIC after a Linux soak.
- **Reuse.** Consensus, proposal and sync traffic should use Malachite's built-in network. We
  should not build our own consensus gossip. Our `xdevs-network` crate covers only what
  Malachite lacks: tx gossip now, evidence gossip in Phase 3, and later snapshot sync. It runs
  a second, small libp2p **0.56** swarm on its own port. Merging both into one swarm with
  `NetworkBuilder::Custom` is a later optimisation (Phase 6), not a Phase-2 task.

## 4. Recommendation

**Keep Malachite (ADR-0002 stands, with the updates in §9).** The reasons, in order:

1. **Semantics match the plan 1:1.** Tendermint rounds, locking and POL (§11.1), linear
   timeouts per round (§11.2), proposer selection defined by the app (§11.3), and a validator
   set per height (§11.6). Using commonware would change the consensus model, not just the
   library.
2. **ADR-0003 needs no workaround.** Proposals are full blocks that each validator re-executes
   on the *finalized* parent. Our linear store and single-writer commit stay as they are.
3. **Much of Phase 2 is already built.** WAL with fsync-before-send, ValueSync block sync with
   certificate verification, libp2p consensus networking with discovery and peer scoring, and
   Ed25519 signing. We write the glue, the validation and the storage.
4. **It fits our stack.** tokio, `libp2p` (plan §12.1), `ed25519-dalek` 2 via libp2p-identity,
   no forced C crypto, MSRV 1.88, Apache-2.0.
5. **It is used in production** by a well-funded maintainer (Circle Arc), with a formal Quint
   spec and model-based tests for the core.

**Risks and mitigations:**

| Risk | Mitigation |
|---|---|
| Breaking API changes every minor release (0.7 → 0.8.0 → 0.8.1 all broke `AppMsg` / `HeightParams` / signing) | Pin `=0.8.2` and commit `Cargo.lock` (CI already uses `--locked`). All Malachite types live in `xdevs-consensus::malachite` behind the xdevs `ConsensusEngine` / `ConsensusApp` traits. Upgrade on purpose, one minor at a time, reading `BREAKING_CHANGES.md` |
| Public repo is a periodic mirror of Circle's fork; issues may get slow responses; "alpha, unaudited" | Apache-2.0 allows a vendored fork (`[patch.crates-io]`) if upstream stalls. Watch releases. Rely on our own e2e and chaos tests (WP5), not on upstream claims |
| libp2p held at 0.56.0 (16 months old) | Our tx-gossip swarm must also use 0.56. Track RUSTSEC through the existing `cargo audit` CI job. Plan the 0.57 move together with a Malachite release that adopts it |
| No upstream Windows CI; first windows-gnu build unverified | WP1 day 1: `cargo check -p xdevs-consensus -p xdevs-network` on the dev box. Add a `windows-latest` (gnu target) check job to CI. Docker/Linux stays the reference platform for the 4-node tests |
| Host connector serialises app replies; slow re-execution delays proposals | Keep `validate` well under `timeout_propose` (§7). Run execution in `spawn_blocking`. Watch the `validate_ms` metric |
| **Our state root is a full recompute** (`xdevs_state::root::db_state_root` walks all `PlainAccounts` and `PlainStorage` on every block). Each validator pays this cost in `validate`, so it grows with state and will eventually exceed `timeout_propose` in the 24 h soak | Track it as a Phase-2 blocker for the soak exit criterion. Either do incremental roots (ADR-0004 "compute roots incrementally from changed keys") before the soak, or raise `timeout_propose` / `target_time` and measure. Owner: state agent (outside the WPs below, or fold into WP2) |
| Stateless `select_proposer` (pure function of set, height and round) vs CometBFT's stateful proposer priority | Phase 2 uses a stateless weighted round-robin (§6.6). Phase 3 carries precomputed priorities inside our `ValidatorSet` for each height |

## 5. Not chosen, and when to revisit

Revisit commonware (or Malachite's own successor features) in Phase 5 if we need any of these:

- BLS threshold certificates (constant-size certificates for light clients and bridges).
- Sub-second pipelined finality.
- Erasure-coded block propagation.

Revisit as well if Malachite stops publishing releases for more than 6 months, or if Circle
changes the license.

---

## 6. Integration design

### 6.1 Crates and dependency direction

```
primitives ─┬─> crypto ───────────────┐
            ├─> consensus (NEW) <─────┘     (no dependency on execution/storage/state)
            ├─> network (NEW)               (libp2p 0.56 tx gossip; no consensus types)
            └─> storage ─> state ─> execution ─┬─> mempool ─┐
                                    genesis ───┴────────────┴─> rpc ─> node ─> bin/xdevs
node depends on consensus + network and implements ConsensusApp ("ChainApp").
testutils (NEW, dev-only) depends on node.
```

| Crate | Package | Contents |
|---|---|---|
| `crates/consensus` | `xdevs-consensus` | xdevs consensus types (`ValidatorSet`, `CommitCertificate`, sign-bytes); the `ConsensusApp` and `ConsensusEngine` traits; `malachite/` adapter (Context, codec, signer, streaming, engine loop); `SignGuard` and the undecided-block store (`consensus.redb`) |
| `crates/network` | `xdevs-network` | libp2p 0.56 swarm: gossipsub tx topic, identify, kad, bootnodes, peer limits; `NetworkHandle` |
| `crates/testutils` | `xdevs-testutils` | In-process N-node harness and assertions (no forks, progress, roots) |
| (changed) `crates/node` | `xdevs-node` | `ChainApp` (propose / validate / commit), validator / full-node wiring, config, tx-gossip glue |
| (changed) `crates/genesis`, `crates/storage`, `crates/crypto`, `bin/xdevs-cli` | | Typed genesis validators, `CommitCertificates` table, Ed25519 consensus key files, localnet generator |

### 6.2 The xdevs-side traits (refines plan §7.3; frozen on day 1)

The plan's `ConsensusApp` is kept but made precise. These are the interface changes: `validate`
gets the height and the origin; `propose` gets a deadline; `head`, `decided` and
`earliest_height` are added for start-up and sync.

```rust
// xdevs-consensus::app  (no Malachite types in this module)
pub enum Validity { Valid, Invalid }
pub enum BlockOrigin { Consensus, Sync }        // Sync = already-decided block from ValueSync

#[async_trait::async_trait]
pub trait ConsensusApp: Send + Sync + 'static {
    /// Last committed (height, block hash); the engine starts at height + 1.
    async fn head(&self) -> anyhow::Result<(u64, B256)>;
    /// Validator set active at `height` (Phase 2: the genesis set for every height).
    async fn validator_set(&self, height: u64) -> anyhow::Result<ValidatorSet>;
    /// Build AND execute a block on the committed head; must finish before `deadline`.
    /// The app caches the execution outcome under the block hash.
    async fn propose(&self, height: u64, round: u32, deadline: std::time::Instant)
        -> anyhow::Result<Block>;
    /// Re-execute a proposal on the committed head (ADR-0003). Err = local failure (no verdict).
    async fn validate(&self, height: u64, block: &Block, origin: BlockOrigin)
        -> anyhow::Result<Validity>;
    /// Persist a decided block + certificate atomically. Idempotent: called again for the same
    /// height with the same hash it only replaces the certificate if the new one has more signatures.
    async fn commit(&self, block: Block, cert: CommitCertificate) -> anyhow::Result<()>;
    /// Serve block sync.
    async fn decided(&self, height: u64) -> anyhow::Result<Option<(Block, CommitCertificate)>>;
    /// Lowest height `decided` can serve (1 while we keep all blocks).
    async fn earliest_height(&self) -> anyhow::Result<u64>;
    /// Misbehaviour seen by the engine (logged in Phase 2, slashed in Phase 3).
    async fn report_evidence(&self, _e: Evidence) -> anyhow::Result<()> { Ok(()) }
}

#[async_trait::async_trait]
pub trait ConsensusEngine: Send {
    async fn run(self, app: std::sync::Arc<dyn ConsensusApp>,
                 shutdown: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()>;
}
pub struct MalachiteEngine { /* config, keys, paths */ }   // impl ConsensusEngine
```

### 6.3 How the Malachite messages map onto `ConsensusApp`

The adapter is `xdevs-consensus::malachite::engine` (one task looping on `channels.consensus`).

| `AppMsg` | Adapter action |
|---|---|
| `ConsensusReady { reply }` | `(h, _) = app.head()`. Reply `(h+1, height_params(h+1))`, where `height_params` = `HeightParams::new(app.validator_set(h+1), timeouts, Some(block_time))`, extension policy `Disabled` |
| `StartedRound { height, round, proposer, role, reply_value }` | Record the round. Reply with the undecided `ProposedValue`s we hold for `(height, round)` from the undecided store (needed after a crash) |
| `GetValue { height, round, timeout, reply }` | Reuse the block already built for `height` if its parent is still the head; otherwise `app.propose(height, round, now + 0.8 × timeout)`. Store it in the undecided store. Reply `LocallyProposedValue { value: BlockValue(hash) }`, then stream the parts (§6.4) through `channels.network` |
| `ReceivedProposalPart { from, published_by, part, reply }` | Reassemble by `(published_by, stream_id)`. Check the per-peer byte budget on `from`. On `Fin`: verify that the proposer signature comes from `ctx.select_proposer(set, h, r)`, decode the block, and check `block.hash() == value_id`. Then `app.validate(h, &block, Consensus)` in `spawn_blocking`, store the block in the undecided store, and reply `Some(ProposedValue { validity, .. })`. If validate returns Err, reply `None` and log; we then prevote nil when `timeout_propose` fires |
| `RestreamProposal { height, round, valid_round, address, value_id }` | Load the block from the undecided store and re-stream it with the new round / POL round |
| `ExtendVote` / `VerifyVoteExtensions` | `None` / all valid (vote extensions are unused in Phase 2) |
| `Decided { certificate, extensions, reply }` | Load the block by `certificate.value_id` from the undecided store. Convert the certificate to the xdevs `CommitCertificate` (§6.5). `app.commit(block, cert)`, then `reply.send(())` |
| `Finalized { certificate, evidence, reply }` | `app.commit` again (certificate upgrade only). `app.report_evidence` for each piece of evidence. Prune the undecided store for `≤ h`. Reply `Next::Start(h+1, height_params(h+1))`, or `Next::Restart(h, …)` if the commit failed |
| `GetHistoryMinHeight { reply }` | `app.earliest_height()` |
| `GetDecidedValues { range, reply }` | Clamp the range to `[earliest, head]` (Malachite does not check it). For each h: `app.decided(h)` → `RawDecidedValue { certificate: to_malachite(cert), value_bytes: rlp(block) }` |
| `ProcessSyncedValue { height, round, proposer, value_bytes, reply }` | Decode the block; on failure → `PeerFault`. `app.validate(h, &block, Sync)`: Err → `LocalTransientError`. Store in the undecided store and reply `Verdict(ProposedValue { height, round, valid_round: Nil, proposer, value: BlockValue(hash), validity })`. The engine then verifies the certificate against `HeightParams.validator_set` and emits `Decided`, so commit goes through the same path |

**Where execution happens:**

- **Proposer:** `GetValue` → `ChainApp::propose` → `EvmExecutor::build` (the Phase-1
  `BlockProducer::produce` without the commit step).
- **Validators:** `ReceivedProposalPart` (complete) → `ChainApp::validate` →
  `EvmExecutor::execute` on the committed head, then compare `state_root`, `receipts_root`,
  `logs_bloom` and `gas_used` with the header.
- **Lagging nodes:** `ProcessSyncedValue` → the same `validate`.
- **Commit:** never re-executes. It uses the `ExecutionOutcome` cached by hash in `ChainApp`. On
  a cache miss (for example after a restart) it re-executes once.

**`ChainApp::validate` rules** (all deterministic except rule 3, which applies only to
`Consensus` origin):

1. `header.number == head + 1` and `parent_hash == head hash`.
2. Header shape:
   - `gas_limit == cfg.block_gas_limit` and `base_fee == next_base_fee(parent)`.
   - `timestamp > parent.timestamp`.
   - `extra_data.len() ≤ 32`.
   - The compat fields equal what `set_compat_header_fields` sets.
   - `prev_randao == parent hash` (Phase-2 rule, same as dev mode).
   - `beneficiary` is free in Phase 2 (proposer's `fee_recipient`); in Phase 3 it is checked
     against the Staking registry.
3. *Consensus origin only:* `timestamp ≤ local_unix_now + 2 s`, the CometBFT PBTS-style
   "timely" check. Blocks from sync skip it because they are already decided. This check does
   not affect execution results, so §10.6 determinism holds.
4. Every tx decodes, recovers, has the right chain id, is not of type 0x03, and the body is
   ≤ `MAX_BLOCK_BYTES` (Phase-2 constant: 4 MiB).
5. `executor.execute(parent, block, state@head)` succeeds, and the roots, bloom and gas match
   the header. Otherwise `Invalid`.

### 6.4 Context types, sign-bytes and proposal parts

The domain-separated encodings below are fixed-length or RLP, so they are canonical.

- `Height = u64`, `Address = [u8; 20]`, where `ValidatorAddress = sha256(ed25519_pubkey)[..20]`
  (the CometBFT convention, kept separate from EVM addresses).
- `Value = BlockValue { hash: B256 }` and `ValueId = B256` (the Ethereum block hash).
- `Validator { address, public_key: [u8;32], voting_power: u64 }`.
- `ValidatorSet`: sorted by address, with `total_voting_power`.
- `Timeouts = LinearTimeouts`.
- `SigningScheme = Ed25519` (from `arc-malachitebft-signing-ed25519`).
- **Vote sign-bytes** = `"XDEVS/VOTE/1" ‖ chain_id:u64be ‖ type:u8 (1 prevote, 2 precommit) ‖
  height:u64be ‖ round:u32be ‖ (0x00 | 0x01 ‖ block_hash)`.
- **Proposal sign-bytes** = `"XDEVS/PROPOSAL/1" ‖ chain_id ‖ height ‖ round:u32be ‖
  pol_round:i64be ‖ block_hash`.
- **Proposal parts**:
  - `Init { height, round, pol_round, proposer }`.
  - `Data { chunk }`: the RLP block (`header ‖ body` as stored in `Headers` / `Bodies`), in
    chunks of ≤ 256 KiB.
  - `Fin { signature }`, where the signature is over `"XDEVS/PARTS/1" ‖ chain_id ‖ height ‖
    round ‖ keccak256(all chunks)`.
  - Configure `pubsub_max_size_per_topic` to match.
- **Codec.** One `XdevsCodec` serves network, WAL and sync (alloy-rlp; no protobuf, no
  `protoc`). The WAL format is versioned with a leading tag byte.

### 6.5 Commit certificate format (stored and served)

```
CommitCertificate {            // RLP, stored in storage table CommitCertificates: u64 -> bytes
    height: u64,
    round: u32,
    block_hash: B256,
    signatures: Vec<CommitSig { validator: [u8;20], signature: [u8;64] }>,  // sorted by validator
}
```

- **Verification.** Rebuild the precommit sign-bytes from `(chain_id, height, round,
  block_hash)`. Verify each signature with `ed25519-consensus` against the validator set at
  `height`. Require the sum of voting power to be greater than 2/3 of the total. Duplicates or
  unknown validators make the certificate invalid.
- **Size.** About 84 B per signer (plus a few bytes of RLP), so roughly 350 B for 4 validators
  and roughly 8.4 KB for 100. That is the linear growth ADR-0007 accepts.
- **Where it lives.** It is not in the Ethereum header (no room; `extraData` is ≤ 32 B). It is
  written in **the same `WriteBatch` as the block** and served by `xdevs_getCommitCertificate`
  (RPC, WP2) and by sync. At `Finalized` it may be replaced with a superset (more signatures).
- **Phase-3 note.** Rewards and downtime need the *previous* block's signers inside the block
  (CometBFT `LastCommit`). That is a block-format decision for Phase 3, not Phase 2.

### 6.6 Proposer selection (Phase 2)

`select_proposer(set, h, r)` takes `k = (h + r) mod total_power` and walks the validators in
address order, accumulating power, until the running sum exceeds `k`. With equal powers this is
plain round-robin. It is a pure function, which is what `Context::select_proposer` needs, and it
is deterministic.

In Phase 3, the app computes CometBFT proposer priorities at each commit and puts the
priority-ordered queue into the `ValidatorSet` for the next height. `select_proposer` then
indexes by `r`, so it stays a pure function.

### 6.7 WAL and double-sign protection (§11.8)

There are two independent layers:

1. **Malachite WAL** at `<data_dir>/consensus/wal`. It is fsynced before every signed vote or
   proposal is published and before a decision, and replayed on restart. This prevents
   equivocation after a crash *within the engine's own view*.
2. **`SignGuard`** wraps the `Signer` and is stored in `<data_dir>/consensus/consensus.redb`.
   This is a separate redb file, so wiping or resyncing the chain DB never resets it. It
   persists `last = (height, round, step, keccak(sign_bytes))`, with step order
   Proposal 0 < Prevote 1 < Precommit 2. The rules, applied before any signature is returned:
   - If `(h, r, s) < last`, refuse.
   - If `(h, r, s) == last` with the same payload hash, sign again (Ed25519 is deterministic;
     WAL replay and rebroadcast need this).
   - If `(h, r, s) == last` with a different hash, refuse and raise an alert.
   - Otherwise, write `last` with a durable commit, then sign.
   - `sign_validator_proof` is exempt.

Startup checks:

- An exclusive `advisory-lock` is taken on the consensus dir, so two processes cannot share a
  data dir.
- Operators are warned that the same key must never run on two machines. Remote signing is
  Phase 6.

The same `consensus.redb` holds **`UndecidedBlocks: (height, hash) -> rlp(block)`**. The WAL
stores only `BlockValue(hash)`, so after a crash the block body for a replayed decision must
come from here. Entries are pruned at `Finalized`.

### 6.8 Block sync for lagging and late nodes

We use Malachite **ValueSync** (`[value_sync] enabled = true`), not a hand-written
`GetBlocks` protocol. It covers plan §12.3 item 1:

- Peers gossip their tip heights, and the node requests ranges.
- The app serves `GetDecidedValues` from `Headers` / `Bodies` / `CommitCertificates`.
- On the receiving side, `ProcessSyncedValue` → re-execute → `Decided` → commit.

A new node with only the genesis file syncs from height 1. Every block is verified twice: the
certificate (by the engine) and the execution (by the app). `eth_syncing` reports
`{currentBlock: head, highestBlock: max peer tip}` from the sync status or network state dump.
Trusted checkpoints (§11.7) and snapshot sync stay in Phase 6.

### 6.9 Static validator set from genesis (Phase 2)

`genesis.validators` (currently `Vec<serde_json::Value>`, ignored) becomes typed:

```json
"validators": [ { "name": "val1", "consensusKey": "ed25519:0x<32-byte hex>", "power": 1,
                  "p2p": "/dns4/val1/tcp/30333/p2p/12D3Koo..." } ]
```

- `power` must be ≥ 1. Duplicate keys are rejected.
- `operator` and staking fields are ignored until Phase 3.
- `validator_set(h)` returns the genesis set for every `h`.
- The genesis hash does **not** change, because validators are not in the header. The
  validator set *is* stored in `Meta[CHAIN_CONFIG]`'s sibling key `Meta["validators"]`, and
  `init_genesis` checks it, so a node started with a different set refuses to run (the same
  pattern as the chain-config check).

### 6.10 Node configuration additions

These follow plan §33.2 and are all additive. Unknown keys are already ignored.

```toml
[validator]            # absent/disabled => full node (ConsensusContext::new_full_node)
enabled = true
key_file = "/var/lib/xdevs/keys/consensus.json"   # Ed25519 seed, keystore v3 encrypted (WP4)
fee_recipient = "0x..."

[consensus]            # plan §11.2 defaults
block_time_ms = 2000   # -> HeightParams.target_time
timeout_propose_ms = 1000   ; timeout_propose_delta_ms = 500
timeout_prevote_ms = 500    ; timeout_prevote_delta_ms = 250
timeout_precommit_ms = 500  ; timeout_precommit_delta_ms = 250
max_timeout_ms = 10000

[p2p]                  # Malachite consensus network
listen = "/ip4/0.0.0.0/tcp/30333"      # QUIC later: /udp/30333/quic-v1
persistent_peers = ["/dns4/val2/tcp/30333", "..."]
node_key_file = "/var/lib/xdevs/keys/node_key.json"   # libp2p Ed25519 identity
discovery = false      # Phase 2: static peers; Phase 3 devnet: kad on

[gossip]               # xdevs-network tx gossip
listen = "/ip4/0.0.0.0/tcp/30334"
bootnodes = []
max_peers = 50
```

`--dev` keeps the Phase-1 single-producer loop unchanged. A single-validator `[validator]` node
also works through consensus (1 of 1 > 2/3) and is useful for testing.

### 6.11 Tx gossip (`xdevs-network`)

- **Topic:** `/xdevs/<chainId>/tx/1` (gossipsub, `ValidationMode::Strict`, message-id =
  keccak(raw tx)).
- **Validation:** inbound → `decode_and_recover(raw, chain_id)` → `pool.add(tx, External)`.
  `report_message_validation_result` accepts on `Ok` or `AlreadyKnown` and rejects otherwise,
  so invalid txs are never forwarded. A 128 KiB cap applies (plan §12.2).
- **Outbound:** `NodeBackend::send_raw_transaction` publishes after a successful local add.
- **Dedup:** the gossipsub seen-cache plus `pool.contains`.
- **Handle API:**
  `NetworkHandle { fn publish_tx(&self, Bytes); fn inbound(&self) -> mpsc::Receiver<(PeerId, Bytes)>; fn peer_count(&self) -> usize; async fn shutdown(self) }`.
  Channels are bounded (§12.5).

### 6.12 Local 4-node test harness

**In-process (`crates/testutils`, used by `tests/e2e`):**

- `LocalNet::start(n, opts)` generates `n` Ed25519 consensus keys and node keys, and writes a
  genesis with `n` equal-power validators plus the dev alloc.
- It picks `127.0.0.1` ports for each node (RPC: 0; consensus and gossip: pre-reserved), sets
  `persistent_peers` to all the others, and starts `n` full `Node`s in one tokio runtime, each
  in its own `tempdir`. Malachite's own integration framework (its ADR 009) also runs many
  engines in one process.
- Controls: `stop(i)` (graceful), `crash(i)` (abort the tasks and drop them without a graceful
  shutdown), `restart(i)` (same data dir), `add_node()` (late joiner, syncs from genesis).
- Assertions:
  - `wait_height(h, timeout)`.
  - `assert_no_forks()`: for every height ≤ min head, all nodes return the same block hash and
    certificate hash over RPC.
  - `assert_roots()`: every node's `verify_head` passes.
- Use a short `block_time_ms` (e.g. 300 ms) and timeouts scaled down for test speed.
- Scenarios mirror the exit criteria:
  - 4 nodes make progress.
  - Crash 1 → progress continues.
  - Crash 2 → no progress for 10 × block time, then restart both → progress resumes, no forks.
  - A late node joins and catches up.
  - A tx sent to node A is included and visible on node D.

**Docker (`deploy/local/`, already sketched):**

- `xdevs testnet init --validators 4 --out deploy/local/cfg` writes the keys, the genesis and
  `valN.toml` files (`persistent_peers` = `/dns4/valN/tcp/30333`).
- Compose profiles `validators` (val1–4) and `joiner` (val5, a non-validator full node).
- The image is built on GitHub Actions (`docker.yml` → `ghcr.io/jacell100/xdevs:<sha>`) and
  pulled locally, because the dev box is short on disk.
- `scripts/chaos/`:
  - `kill.sh N`: `docker kill`, a real crash with no graceful shutdown.
  - `pause.sh N`: `docker pause`, simulating a partition or GC stall.
  - `netem.sh N delay`: `tc qdisc` inside the container (needs `NET_ADMIN`).
  - `partition.sh`: `docker network disconnect`.
  - `check.sh`: polls every node's RPC and fails on hash divergence or a stalled head.
- The 24 h soak runs `xdevs-loadgen` against val1 with `check.sh` every 10 s, on a cloud VM or a
  CI self-hosted runner.

---

## 7. Timeouts and block time (initial values)

These are plan §11.2 mapped onto Malachite.

| Plan | Malachite | Value |
|---|---|---|
| `timeout_propose` | `LinearTimeouts.propose` / `propose_delta` | 1000 ms / +500 ms |
| `timeout_prevote` | `prevote` / `prevote_delta` | 500 ms / +250 ms |
| `timeout_precommit` | `precommit` / `precommit_delta` | 500 ms / +250 ms |
| `timeout_commit` | `HeightParams.target_time` (minimum height duration) | 2000 ms for the Phase-2 exit test (the plan's 2 s block time) |
| — | `max_timeout` | 10 s |
| — | `rebroadcast` | `propose + prevote + precommit` (Malachite default rule) |

Budget rule: proposer build plus streaming plus validator re-execution must fit inside
`timeout_propose`. Measure `build_ms` and `validate_ms` (WP6 metrics). Because of the
full-recompute state root (§4 risks), these will grow with state size.

## 8. Work packages (parallel build plan)

**Day-0 interface freeze, done by the WP1 lead within the first day:** merge the
`xdevs-consensus` public skeleton (`app.rs` traits, `types.rs` with `ValidatorSet`,
`CommitCertificate`, `Validator`, `Evidence`, all compiling with `todo!()` bodies). WP4 merges
`GenesisValidator` and the `CommitCertificates` table at the same time. After that, each WP
changes only the files it owns. Shared interfaces change only additively and are reported, the
same rule as `docs/specs/phase1-interfaces.md`.

| WP | Owns (exclusive write) | Delivers | Depends on |
|---|---|---|---|
| **WP1 Consensus engine adapter** | `crates/consensus/**` | Day-0 skeleton. Malachite `Context` and types; `XdevsCodec`; Ed25519 `Signer`/`Verifier` with `SignGuard`; proposal streaming and reassembly; `UndecidedBlocks` store; `MalachiteEngine` (AppMsg loop, §6.3); config mapping (`[consensus]`, `[p2p]` → Malachite `ConsensusConfig` / `ValueSyncConfig`, channel names `/xdevs/<chain>/…`). Tests: codec round-trips, SignGuard (lower, equal, conflicting, re-sign), certificate verification, and **4 in-process engines with a mock `ConsensusApp` (counter app)** reaching height 50 and surviving 1 crash. Day 1: windows-gnu `cargo check` | primitives, crypto |
| **WP2 Chain app and node wiring** | `crates/node/**`, `bin/xdevs/**` | `ChainApp: ConsensusApp`: `propose` (refactor `BlockProducer` into build-without-commit plus `commit_block_with_cert`), `validate` (§6.3 rules, `spawn_blocking`, outcome cache), `commit` (one `WriteBatch`: block, certificate, state diff, history; then pool updates), `decided`, `earliest_height`. Config sections `[validator]`, `[consensus]`, `[p2p]`, `[gossip]`. `Node::start` in validator / full-node / dev mode. Tx-gossip glue (RPC → publish; inbound → pool). `eth_syncing`, `xdevs_getCommitCertificate`, `net_peerCount` | WP1 skeleton, WP3 handle, WP4 table / genesis |
| **WP3 Tx gossip network** | `crates/network/**` | libp2p **0.56.0** swarm (tcp + noise + yamux; quic behind config), gossipsub tx topic with strict validation, identify, kad (off by default), bootnodes, `max_peers`, message-size caps, bounded channels, `NetworkHandle` (§6.11). Tests: 3 in-process swarms propagate a tx; invalid tx not forwarded; peer limit | primitives (no node dependency; validation via a callback `Fn(&[u8]) -> Verdict`) |
| **WP4 Genesis, keys, storage, CLI** | `crates/genesis/**`, `crates/storage/**`, `crates/crypto/**`, `bin/xdevs-cli/**` | Typed `GenesisValidator` + `Genesis::validator_set()` + stored-set check. Storage table `CommitCertificates: u64 -> Bytes` (additive; `SCHEMA_VERSION` unchanged, redb creates tables on open). Ed25519 consensus key file (keystore v3 around the 32-byte seed) and libp2p node key file. `xdevs-cli keygen --consensus`, `xdevs-cli keygen --node-key`, and `xdevs testnet init --validators N --out DIR` (keys, genesis, `valN.toml`, peer multiaddrs) | primitives |
| **WP5 Harness, e2e, Docker, CI** | `crates/testutils/**`, `tests/e2e/**`, `deploy/local/**`, `scripts/**`, `.github/workflows/**` | `LocalNet` harness (§6.12); e2e tests for each exit criterion; Docker compose (`validators`, `joiner` profiles) built in CI and pushed to GHCR; chaos scripts plus `check.sh`; CI jobs: Linux e2e 4-node, windows-gnu `cargo check -p xdevs-consensus -p xdevs-network`, `cargo audit`; soak runbook `docs/runbooks/soak.md` | node public API (WP2), CLI generator (WP4) |
| **WP6 RPC subscriptions, metrics, loadgen** | `crates/rpc/**`, new `crates/metrics/**`, new `bin/xdevs-loadgen/**` | WebSocket `eth_subscribe` (`newHeads`, `logs`) through a `RpcBackend` subscription hook; `eth_getLogs` log index; `xdevs-metrics` (prometheus-client **0.23.1**, the same registry type Malachite uses, on an HTTP `/metrics` endpoint, `[metrics] listen`); consensus metrics (height, round, `build_ms`, `validate_ms`, `commit_ms`, peers); `xdevs-loadgen` v1 (N senders, target TPS, latency percentiles) | rpc / primitives; node wires it (WP2) |

**Integration order:**

1. WP1 skeleton and WP4 table/genesis (day 0–1).
2. WP1, WP3, WP4 and WP6 in parallel.
3. WP2, against the mock-free WP1 engine.
4. WP5 e2e scenarios as each piece lands.

The critical path is WP1 → WP2 → WP5 (4-node e2e).

## 9. Draft ADR-0002 update (proposed text; the ADR file is not edited here)

```markdown
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
```

## 10. Exact versions to pin (workspace `Cargo.toml`)

```toml
# --- Consensus (xdevs-consensus only) --------------------------------------
# Pin exactly: Malachite breaks its API on every minor release.
arc-malachitebft-app-channel     = "=0.8.2"   # re-exports app, engine, core types, config, codec, sync, wal
arc-malachitebft-signing-ed25519 = { version = "=0.8.2", features = ["std", "rand", "serde", "zeroize"] }
ed25519-consensus = "2.1.0"                   # same as Malachite's signing scheme (ZIP-215)

# --- Networking (xdevs-network; must equal Malachite's libp2p) --------------
libp2p = { version = "=0.56.0", default-features = false, features = [
    "tokio", "tcp", "quic", "dns", "noise", "yamux", "gossipsub", "identify",
    "kad", "ping", "request-response", "macros", "ed25519", "metrics",
] }

# --- Metrics (xdevs-metrics; Malachite's registry type) ---------------------
prometheus-client = "0.23.1"
```

These were resolved together with our current graph by the probe on 2026-11-10:
`libp2p` 0.56.0 (gossipsub 0.49.5, kad 0.48.0, quic 0.13.1, identity 0.2.14), `ractor` 0.15.13
(transitive; do not depend on it directly), `ed25519-consensus` 2.1.0, `prometheus-client`
0.23.1, `tokio` 1.54.1 (unchanged), `ed25519-dalek` 2.2.0 (unchanged), `rand` 0.8.8
(unchanged), `redb` 4.5.0 (unchanged; also used for `consensus.redb`). `Cargo.lock` must be
committed after the first `cargo update -p` so that transitive `arc-malachitebft-*` crates stay
at 0.8.2.

## 11. Sources (fetched 2026-11-10)

- Malachite repo, README (alpha / unaudited; joined Circle; Quint specs):
  https://github.com/circlefin/malachite
- Malachite releases (v0.8.2 2026-10-19, v0.8.1 10-06, v0.8.0 08-27, v0.7.0, v0.5.0):
  https://github.com/circlefin/malachite/releases
- Malachite commit history (sync drops, April–June fixes):
  https://github.com/circlefin/malachite/commits/main
- Malachite breaking changes 0.7–0.8.1:
  https://raw.githubusercontent.com/circlefin/malachite/main/BREAKING_CHANGES.md
- Malachite workspace manifest (libp2p 0.56.0, ractor 0.15.10, ed25519-consensus 2.1.0,
  rust-version 1.88): https://raw.githubusercontent.com/circlefin/malachite/main/code/Cargo.toml
- Malachite CI (Linux only): https://raw.githubusercontent.com/circlefin/malachite/main/.github/workflows/rust.yml
- Malachite channel-app tutorial: https://raw.githubusercontent.com/circlefin/malachite/main/docs/tutorials/channels.md
- crates.io versions: https://crates.io/api/v1/crates/arc-malachitebft-engine/versions ;
  https://crates.io/crates/arc-malachitebft-app-channel/0.8.2
- Published sources read locally: `arc-malachitebft-{app-channel,app,engine,core-types,core-consensus,network,wal,signing,signing-ed25519,config}-0.8.2`
- Malachite acquisition by Circle (Informal Systems press release, Aug 2025):
  https://www.barchart.com/story/news/34223912/informal-systems-announces-malachite-acquisition-by-circle-to-power-new-arc-blockchain-network
- Arc mainnet on Malachite, 11 permissioned validators, 2026-09-16 (crypto.news, 2026-09-04):
  https://crypto.news/circle-arc-mainnet-usdc-chain-stablecoin-infrastructure/ ; The Defiant:
  https://thedefiant.io/news/blockchains/circle-launches-arc-mainnet-with-blackrock-dtcc-and-visa-producing-blocks
- commonware monorepo (stability levels, primitives, license): https://github.com/commonwarexyz/monorepo
- commonware versions: https://crates.io/api/v1/crates/commonware-consensus/versions
- commonware commits: https://github.com/commonwarexyz/monorepo/commits/main
- commonware CI (Ubuntu/macOS): https://raw.githubusercontent.com/commonwarexyz/monorepo/main/.github/workflows/fast.yml
- commonware workspace (rust-version 1.95.0, edition 2024, rand 0.10, blst):
  https://raw.githubusercontent.com/commonwarexyz/monorepo/main/Cargo.toml
- Simplex docs (pipelining, certification): https://docs.rs/commonware-consensus/latest/commonware_consensus/simplex/
- Tempo consensus (Simplex by Commonware, permissioned set): https://tempo.xyz/developers/docs/protocol/blockspace/consensus
- libp2p versions (0.57.1 2026-10-15, 0.56.0 2025-06-27): https://crates.io/api/v1/crates/libp2p/versions
- rust-libp2p CI (Windows/macOS compile-only): https://raw.githubusercontent.com/libp2p/rust-libp2p/master/.github/workflows/ci.yml
- Other BFT crates: https://crates.io/api/v1/crates/hotmint-consensus (GPL-3.0-only),
  https://crates.io/api/v1/crates/hotstuff_rs (0.4.0, 2024-12-29),
  https://crates.io/api/v1/crates/tendermint (0.41.0, 2026-10-26)
