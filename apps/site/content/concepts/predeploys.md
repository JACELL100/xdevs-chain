# Predeployed contracts

xDEVS Chain's system and user-safety contracts live at **fixed addresses** that are the same on every xDEVS network. They are written into the genesis state as runtime bytecode (constructors never run) and configured by a one-time `initialize` call from the protocol system address.

The canonical list is [`contracts/src/common/Predeploys.sol`](https://github.com/JACELL100/xdevs-chain/blob/main/contracts/src/common/Predeploys.sol). The [SDK](/developers/sdk) exports the same addresses as `predeploys`, and a unit test keeps the two in sync.

:::tip[Live on Enclave]
The Enclave genesis contains every contract below, built by [`tools/genesis-builder`](https://github.com/JACELL100/xdevs-chain/tree/main/tools/genesis-builder) from [`configs/enclave.json`](https://github.com/JACELL100/xdevs-chain/blob/main/tools/genesis-builder/configs/enclave.json). Look any of them up in the [explorer](https://explorer.xdevschain.tech).
:::

:::info[Local dev chain]
The **built-in dev genesis** (`xdevs run --dev`, chain id 7117) does not contain them: there these addresses are empty accounts (the Treasury address only holds a balance). Start the dev node with `--genesis deploy/configs/dev-genesis-with-predeploys.json` to get all of them locally. None of the contracts has had an external audit.
:::

## Address table

| Address | Contract | Purpose | Spec |
|---|---|---|---|
| `0x0000000000000000000000000000000000001000` | `Staking` | Validators, delegation, rewards, slashing, epoch transitions | [system](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/predeploys-system.md) |
| `0x0000000000000000000000000000000000001001` | `Governance` | Stake-weighted proposals with a timelock | [system](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/predeploys-system.md) |
| `0x0000000000000000000000000000000000001002` | `Treasury` | Receives 20% of base fees; grants by governance | [system](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/predeploys-system.md) |
| `0x0000000000000000000000000000000000001003` | `FeeConfig` | Fee parameters, changeable only by governance | [system](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/predeploys-system.md) |
| `0x0000000000000000000000000000000000001004` | `XNSRegistry` | `.xdevs` names: registry, registrar, resolver, reverse | [XNS](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xns-and-approvals.md) |
| `0x0000000000000000000000000000000000001005` | `XdevsSmartAccount` | Smart account implementation, EIP-7702 delegation target | [accounts](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/smart-accounts.md) |
| `0x0000000000000000000000000000000000001006` | `SmartAccountFactory` | CREATE2 account factory (also hosts the account extension code) | [accounts](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/smart-accounts.md) |
| `0x0000000000000000000000000000000000001007` | `XdevsPaymaster` | Shared gas sponsorship (ERC-4337 v0.9) | [accounts](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/smart-accounts.md) |
| `0x0000000000000000000000000000000000001008` | `Permit2Expiring` | Permit2-style allowances with mandatory expiry | [approvals](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xns-and-approvals.md) |
| `0x0000000000000000000000000000000000001009` | `WXDS` | Wrapped XDS (WETH9-style) | [system](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/predeploys-system.md) |

## Precompile and system addresses

| Address | What | Status |
|---|---|---|
| `0x0000000000000000000000000000000000000100` | `P256VERIFY` precompile (secp256r1 / passkeys, EIP-7951) | <span class="status ok">Live</span> (native in the node, revm Osaka) |
| `0x0A00000000000000000000000000000000000001` | `ED25519VERIFY` precompile (about 3,000 gas), used by the [bridge](/concepts/bridges) light clients | Activates at block 97,200 on Enclave (`ed25519VerifyBlock`); off elsewhere by default |
| `0xffffFFFfFFffffffffffffffFfFFFfffFFFfFFfE` | `SYSTEM_ADDRESS`: caller of protocol system calls (no signature, no gas charge) | Used by the node |

## Genesis extras

The genesis builder also places **Multicall3** at `0xcA11bde05977b3631167028862bE2a173976CA11` (the SDK chain definitions reference it), the deterministic CREATE2 deployer at `0x4e59b44847b379578588920cA78FbF26c0B4956C`, and the canonical **ERC-4337 EntryPoint v0.9** at `0x433709009B8330FDa32311DF1C2AFA402eD8D009`, which the public [bundler](/network/endpoints) uses.

## Upgrades

The system contracts are **not proxies**. Their code changes only through a hard fork at a set block height (validators upgrade their binaries), with append-only storage layouts. Every parameter change goes through Governance (vote + timelock). No runtime admin key exists.
