# Genesis predeploy builder

Status: implemented · Source: `tools/genesis-builder/` · Plan: §18 (system contracts), §20.2 (allocation), §21 (genesis)

The builder computes the genesis state of every predeploy (the 10 xDEVS system contracts and the canonical
tooling contracts) and writes it in the node's genesis format (`crates/genesis/src/lib.rs`). It replaces the
`xdevs-cli genesis build` sketch of plan §21.2.

| Output | What it is |
|---|---|
| `deploy/configs/predeploys-alloc.json` | `{meta, alloc}`: only the predeploy and canonical-helper accounts, for the testnet config. Merge it into a genesis with `merge`. |
| `deploy/configs/testnet-genesis.template.json` | Full testnet genesis: §20.2 allocation buckets, predeploys and 4 genesis validators. **Placeholder** addresses, keys and p2p addresses. |
| `deploy/configs/dev-genesis-with-predeploys.json` | `deploy/configs/dev-genesis.json` (chain id 7117) plus every predeploy. `dev-genesis.json` itself is unchanged. |

## 1. Commands

Requirements: Node ≥ 22.18 (24 used; it runs the TypeScript directly, no `npm install`), Foundry (`forge`, `cast`,
`anvil` on `PATH` or `FORGE` / `CAST`). Run from the repository root.

```bash
# full testnet genesis + the predeploy alloc (template inputs: tools/genesis-builder/configs/testnet.json)
node tools/genesis-builder/genesis-builder.ts build --config tools/genesis-builder/configs/testnet.json \
  --out deploy/configs/testnet-genesis.template.json --alloc-out deploy/configs/predeploys-alloc.json
# dev variant
node tools/genesis-builder/genesis-builder.ts build --config tools/genesis-builder/configs/dev.json \
  --out deploy/configs/dev-genesis-with-predeploys.json

# local multi-validator network: add the predeploys to the genesis written by `xdevs-cli testnet init`
xdevs-cli testnet init --out net ...
node tools/genesis-builder/genesis-builder.ts build --config tools/genesis-builder/configs/dev.json \
  --validators-from net/genesis.json --alloc-out net/predeploys-alloc.json
node tools/genesis-builder/genesis-builder.ts merge --genesis net/genesis.json \
  --alloc net/predeploys-alloc.json --out net/genesis.json.new && mv net/genesis.json.new net/genesis.json

# checks (start a fresh anvil from the file, or pass an RPC URL of a node started from it)
bash tools/genesis-builder/verify.sh deploy/configs/testnet-genesis.template.json - 1000000000
bash tools/genesis-builder/verify.sh net/genesis.json http://127.0.0.1:8545

# re-derive the vendored canonical code (optional; network for --fetch-oz / --rpc)
node tools/genesis-builder/genesis-builder.ts verify-canonical --fetch-oz --rpc https://ethereum-rpc.publicnode.com
```

`package.json` has the same commands as npm scripts (`npm run build:testnet`, `build:dev`, `verify`, `verify:dev`,
`verify-canonical`). Forge always runs with `--root contracts --out out-genesis --cache-path cache-genesis`
(gitignored), so it never touches the default `contracts/out` and `contracts/cache`, which other tools (and
Slither's `forge clean`) use.

## 2. How it works

```
config.json ──▶ genesis-builder.ts ──GENESIS_PARAMS (JSON env)──▶ forge script GenesisPredeploys.s.sol
                                                                     (in-memory EVM, nothing broadcast)
             ◀── JSON return value: accounts{code,balance,nonce,storage}, validator set, hashes ──┘
             ──▶ checks ──▶ predeploys-alloc.json / genesis.json
```

`tools/genesis-builder/script/GenesisPredeploys.s.sol` runs in forge's in-memory EVM with `block.number = 0`,
`block.timestamp = genesis timestamp`, `block.chainid = genesis chain id`:

1. **Code.** `vm.etch` of the runtime code (`deployedBytecode` of the forge artifact, compiled with the contracts'
   own `foundry.toml`: solc 0.8.38, osaka, 200 runs, no metadata) at `0x…1000`–`0x…1009`. No constructor runs.
2. **Canonical helpers.** The deterministic deployment proxy `0x4e59b448…956C` (runtime code), then the **real
   CREATE2 deployment** of EntryPoint v0.9 through it (salt + canonical init code), so the EntryPoint lands at
   `0x433709009B8330FDa32311DF1C2AFA402eD8D009`, its constructor creates `SenderCreator` at
   `0x0A630a99Df908A81115A3022927Be82f9299987e`, and its immutables (cached EIP-712 domain) are computed for the
   target chain id. Multicall3 `0xcA11bde0…CA11` gets its runtime code (no immutables).
3. **Initialization**, as `SYSTEM_ADDRESS` (`vm.prank`), in the order of `predeploys-system.md`: Staking balance =
   Σ genesis stakes; `Staking.initialize`, `Governance.initialize`, `Treasury.initialize`, `FeeConfig.initialize`,
   then `XNSRegistry.initialize` (`xns-and-approvals.md` §1.1). Permit2Expiring, WXDS and the account predeploys
   (`0x…1005`–`0x…1007`, `smart-accounts.md` §2) get code only.
4. **Dump.** Storage is collected with `vm.startStateDiffRecording`: every slot written during steps 1–3 is read
   back with `vm.load` (zero values dropped), so no storage layout knowledge is needed and mappings / dynamic
   arrays / checkpoints are complete. A write to any account outside the predeploy list aborts the build.
   The result is returned as a JSON string (`forge script --json`, ABI-decoded by the TypeScript).

Default parameters come from the existing reference scripts: `contracts/script/system/SystemGenesis.s.sol`
(`stakingParams()`, `governanceParams()`, `feeParams()`) and `contracts/script/periphery/DeployPeriphery.s.sol`
(`genesisInitCalldata()`: XNS governance `0x…1001`, treasury `0x…1002`, fee 0, reserved labels). The config
may override single fields.

**Why forge's in-memory EVM and not anvil + RPC.** Staking writes `block.number` into storage during
`initialize` (`lastSetChangeBlock`, `lastEpochBlock`, vote checkpoints). In anvil every transaction mines a new
block, so initialization would run at blocks 1…n and leave non-zero block numbers in genesis storage (the signing
set logic `lastSetChangeBlock == block.number − 1` and `getPastVotes(·, 0)` would then misbehave). A forge script
runs at block 0 exactly, needs no gas, keys or impersonation, and is deterministic. anvil is used for
**verification** instead (`anvil --init <genesis>` then `cast`), which also proves the file is a valid
geth-style genesis.

### Checks performed by `build`

* every predeploy / helper has code; exactly the 14 expected accounts are produced; SenderCreator address;
* runtime code == `contracts/out-genesis` artifact; immutables: none except XNSRegistry's one deployer
  immutable, which must be zero-filled (only `SYSTEM_ADDRESS` can then initialize it);
* `Staking.consensusValidatorSet()` == configured validators (keys, operators, power = stake in wei);
* `FeeConfig.version() == 1` and FeeConfig slots 1–5 == genesis `gasLimit`, `baseFee.minimum`, `burnBps`,
  `maxChangeDenominator`, `elasticity` (the FeeConfig defaults are **derived** from the genesis config, so they
  can only differ through an explicit `fee` override, which then fails the build);
* `config.treasury == 0x…1002`, `config.systemAddress == 0xff…fe`;
* Σ balances == `totalSupply` (when configured);
* genesis `validators[].power == stake / 1e18` (whole tXDS), or, with `--validators-from`, powers proportional to
  the Staking stakes.

## 3. Config file (`tools/genesis-builder/configs/*.json`)

| Key | Meaning |
|---|---|
| `genesis` | Skeleton copied verbatim to the output: `config` (chainId, xip0001Block, baseFee, …), `timestamp`, `gasLimit`, `extraData`. |
| `totalSupply` | Optional, whole tXDS. The build fails unless Σ all balances (allocations + Staking stake) equals it. |
| `allocations[]` | `{name?, address, amount}` (amount in tXDS, decimals allowed). Same address twice = summed. An allocation on a predeploy address (Treasury) funds that contract. |
| `validators[]` | `{name?, operator, consensusKey: "ed25519:0x…", stake (tXDS), commissionBps?, p2p?}`, registered in Staking and written to genesis `validators[]`. |
| `emitValidators` | `false` = do not write `validators[]` (single-node dev chain: the node treats an empty list as dev mode). |
| `validatorDefaults` | `{operators[], stake, commissionBps}` used by `--validators-from` for entries without `operator` / `stake`. |
| `staking`, `governance` (+ `guardian`), `fee`, `xns` | Optional field overrides (names as in `IStaking.Params`, `IGovernance.Params`, `FeeConfig.FeeParams`; `xns`: `governance`, `treasury`, `feePerYear`, `reserved[]`). |

Testnet (`configs/testnet.json`, plan §20.2, 1,000,000,000 tXDS): faucet 300M → `0x…fa01`, ecosystem grants 250M →
`0x…fa02`, Treasury 200M → `0x…1002`, core development 150M → `0x…fa03`, reserve 50M → `0x…fa04`, genesis stake
4 × 12.5M = 50M → Staking `0x…1000`. Validators `val1`–`val4`: operators `0x…a11ce1`–`4`, commission 10%,
power 12,500,000 each. **Every `0x…fa0N`, `0x…a11ceN`, consensus key and p2p address is a placeholder**; the
consensus keys are valid Ed25519 keys derived from public seeds
(`sha256("xdevs-testnet-placeholder-validator-<i>")`) and must never be used. Guardian: none (`address(0)`,
pausing disabled) unless set.

Dev (`configs/dev.json`): the 10 Anvil accounts × 10,000 tXDS + Treasury 200M (as `dev-genesis.json`) + one Staking
validator (operator Anvil #0, stake 10,000 tXDS = `minSelfStake`, public placeholder key) whose stake is minted
into Staking; `validators[]` is not emitted, so the node stays a single-node dev chain. Total 200,110,000 tXDS.

## 4. `predeploys-alloc.json` and `merge`

`meta` records what the alloc is bound to: `chainId`, genesis `timestamp`, the Staking genesis validators
(operator, consensus key, stake), `validatorSetHash`, `stakingBalance`, and name / size / keccak code hash of
every account. The alloc is **chain-specific** (EntryPoint's EIP-712 domain immutable) and
**validator-specific** (Staking storage).

`merge --genesis in.json --alloc predeploys-alloc.json --out out.json` keeps `config`, `validators` and every other
top-level field of the input, adds the alloc, sorts it by address and fails on:

* `meta.chainId` ≠ genesis `config.chainId`;
* an **address collision**: an address present in both. The only accepted overlap (disabled by `--strict`) is
  *funding a predeploy*: one side is a balance-only entry and the other has balance 0 — e.g. `xdevs-cli testnet
  init` gives the Treasury `0x…1002` 200M tXDS while the alloc carries its code + storage. Nothing is ever summed
  or overwritten; two balances, two codes or two storages are errors;
* genesis `validators[]` non-empty and its consensus keys ≠ the Staking genesis validators
  (`--allow-validator-mismatch` downgrades this to a warning); an empty `validators[]` (dev chain) is accepted;
* FeeConfig slots ≠ the genesis `gasLimit` / `baseFee`.

### Genesis `validators[]` vs Staking

* **Phase 2** (now): the node's consensus set is genesis `validators[]` (`Genesis::validator_set`, sorted by
  consensus key, power as written). Staking is not read by consensus.
* **Phase 3**: the set comes from `Staking.consensusValidatorSet()` (operators, Ed25519 keys, power in wei,
  ordered by stake desc, operator asc) and changes at epochs; the node converts power as `power / 1e18`.
* The builder keeps both views identical at genesis: same consensus keys; `power == stake / 1e18` for configs
  that list validators (12,500,000 on the testnet), and for `--validators-from` (where `testnet init` writes
  `power: 1`) the stakes must be proportional to the powers (default: `validatorDefaults.stake` for everyone).
  `operator`, `stake` (wei, decimal string) and `commissionBps` are written as extra fields of each
  `validators[]` entry, which the Rust parser keeps verbatim (`GenesisValidator::extra`).
* The order differs (key order in Rust, stake/operator order in Staking); the set (key → power) is identical.

## 5. Canonical helpers

`tools/genesis-builder/script/CanonicalCode.sol` vendors three byte strings (with their keccak hashes, asserted
by the forge script):

| Contract | Address | Provenance |
|---|---|---|
| EntryPoint v0.9.0 init code (24,855 B) | `0x433709009B8330FDa32311DF1C2AFA402eD8D009` via CREATE2 salt `0x7702…bf61` | input of mainnet tx `0x65f7c530…9ccc` (block 23,783,136) minus the salt. Reproduced byte for byte from `contracts/lib/account-abstraction` (tag v0.9.0, LF line endings) + OpenZeppelin Contracts 5.1.0 with solc 0.8.28, cancun, viaIR, 1,000,000 runs (upstream `hardhat.config.ts`). |
| Multicall3 runtime (3,808 B) | `0xcA11bde05977b3631167028862bE2a173976CA11` | mainnet `eth_getCode`, codehash `0xd5c15df6…0891`. |
| Deterministic deployment proxy runtime (69 B) | `0x4e59b44847b379578588920cA78FbF26c0B4956C` | mainnet `eth_getCode`, codehash `0x2fa86add…4989`. |

`verify-canonical` re-checks all of it: hashes, the CREATE2 address, the source reproduction (`--solc`, `--oz` or
`--fetch-oz` = `npm pack @openzeppelin/contracts@5.1.0`), and with `--rpc` the mainnet code of Multicall3 and the
deployer, plus the EntryPoint and SenderCreator runtime code produced by the same forge pipeline at chain id 1,
which must equal mainnet's **including immutables**. A Windows checkout with `core.autocrlf` has CRLF sources
in `contracts/lib/account-abstraction`; the reproduction normalizes them to LF (otherwise only the metadata hash
differs).

Nonces: contract predeploys at `0x…1000`–`0x…1009` have nonce 0 (code placed, no constructor). The helpers have
the nonces of a real deployment on a fresh chain: deployer 2 (1 + the EntryPoint CREATE2), EntryPoint 2 (1 +
SenderCreator), SenderCreator 1, Multicall3 1.

## 6. Output format (node genesis, `crates/genesis`)

```json
{ "config": {...}, "timestamp": "0x0", "gasLimit": "0x1c9c380", "extraData": "0x7844455653",
  "alloc": { "0x…1000": { "balance": "50000000000000000000000000", "code": "0x…",
                          "storage": { "0x<64 hex>": "0x<64 hex>" }, "nonce": "0x2" } },
  "validators": [ { "name": "val1", "consensusKey": "ed25519:0x…", "power": 12500000, "p2p": "…",
                    "operator": "0x…", "stake": "12500000000000000000000000", "commissionBps": 1000 } ] }
```

Balances are decimal wei strings, storage keys and values are full 32-byte hex (zero values omitted), nonces are
hex strings and omitted when 0, accounts are sorted by address, `balance` is always present (anvil requires it).
The file is also a valid anvil / geth-style genesis (`anvil --init`).

## 7. Verification and determinism

`verify.sh` (anvil or any RPC): every alloc account's code / balance / nonce / storage slot at block 0, chain id,
total supply; every predeploy has code; `Staking.activeValidators()` / consensus keys / Σ powers = Staking balance /
epoch 0 / block numbers in slot 3 are 0; Governance and Treasury wiring; FeeConfig slots 0–10 as documented in
`predeploys-system.md` §4; XNS governance / treasury / reserved labels; EntryPoint `supportsInterface`,
`getNonce`, `senderCreator()`, `getDomainSeparatorV4()` == the EIP-712 domain of this chain id; Multicall3
`getChainId` and `aggregate3`; a CREATE2 through the deployer; WXDS.

Determinism: same inputs → byte-identical files (checked by building twice, the second time after deleting
`contracts/out-genesis` and `contracts/cache-genesis`, and comparing with `cmp`). Inputs = the config, the
contract sources and compiler settings, the vendored `CanonicalCode.sol`.

## 8. Notes and limitations

* The genesis timestamp feeds `block.timestamp` during initialization (nothing time-dependent is stored today).
  XNS `available()` returns false while `block.timestamp < 90 days` (expiry 0 + grace period): true on anvil at
  block 0 with timestamp 0, irrelevant on a live chain with wall-clock block timestamps.
* anvil does not see EIP-1559 on these files (no `baseFeePerGas` field; the node uses `config.baseFee.initial`):
  use `cast send --legacy` against anvil.
* The Staking stake is minted into Staking's balance at genesis (part of the total supply), so a merged
  `testnet init` genesis has Σ stakes more supply than the input.
* Large code is fine in genesis: Staking runtime is 24,099 bytes and EntryPoint 22,425 bytes; EIP-170 only limits
  CREATE, the node inserts genesis code as is.
