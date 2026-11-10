# Predeployed contracts

xDEVS Chain's system and user-safety contracts live at **fixed addresses** that are the same on every xDEVS network. They are written into the genesis state as runtime bytecode (constructors never run) and configured by a one-time `initialize` call from the protocol system address.

The canonical list is [`contracts/src/common/Predeploys.sol`](https://github.com/JACELL100/xdevs-chain/blob/main/contracts/src/common/Predeploys.sol). The [SDK](/developers/sdk) exports the same addresses as `predeploys`, and a unit test keeps the two in sync.

::: warning Not in the dev genesis yet
The contracts are built and tested in Foundry, but the **local dev chain's genesis does not contain them yet**: on chain id 7117 these addresses are empty accounts (the Treasury address only holds a balance). Placing them in genesis is a Phase 3 task (the genesis builder). The precompile at `0x100` is native to the node and does work today.
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
| `0x0000000000000000000000000000000000000100` | `P256VERIFY` precompile (secp256r1 / passkeys, EIP-7951) | <span class="status ok">Works today</span> (native in the node, revm Osaka) |
| `0xffffFFFfFFffffffffffffffFfFFFfffFFFfFFfE` | `SYSTEM_ADDRESS`: caller of protocol system calls (no signature, no gas charge) | Used by the node |

## Planned genesis extras

The genesis builder is also planned to include **Multicall3** at `0xcA11bde05977b3631167028862bE2a173976CA11` (the SDK chain definitions already reference it), a CREATE2 deployer, and the canonical **ERC-4337 EntryPoint v0.9** at `0x433709009B8330FDa32311DF1C2AFA402eD8D009`.

## Upgrades

The system contracts are **not proxies**. Their code changes only through a hard fork at a set block height (validators upgrade their binaries), with append-only storage layouts. Every parameter change goes through Governance (vote + timelock). No runtime admin key exists.
