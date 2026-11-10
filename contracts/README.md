# xDEVS Chain contracts

Solidity system contracts for xDEVS Chain. All of them are predeployed at fixed addresses in the genesis state (see `src/common/Predeploys.sol`), so none rely on constructors: each is configured by a one-time `initialize` call from the protocol system address, or needs no setup at all.

| Area | Contracts | Spec |
|---|---|---|
| `src/system` | Staking `0x…1000`, Governance `0x…1001`, Treasury `0x…1002`, FeeConfig `0x…1003`, WXDS `0x…1009` | [predeploys-system.md](../docs/specs/predeploys-system.md) |
| `src/accounts` | XdevsSmartAccount `0x…1005` (passkeys, social recovery, session keys, Guardian Vault), SmartAccountFactory `0x…1006`, Paymaster `0x…1007`, StablecoinPaymaster | [smart-accounts.md](../docs/specs/smart-accounts.md) |
| `src/periphery` | XNSRegistry `0x…1004` (`.xdevs` names), Permit2Expiring `0x…1008`, XDRC-20, CanonicalTokenFactory | [xns-and-approvals.md](../docs/specs/xns-and-approvals.md) |

## Build and test

Requires [Foundry](https://getfoundry.sh). Clone with submodules (`git clone --recurse-submodules`, or `git submodule update --init --recursive`).

```bash
forge build
forge test                            # everything
FOUNDRY_PROFILE=system forge test     # one area only: system | accounts | periphery
forge coverage
```

Each area has unit, fuzz and invariant tests. Static analysis (Slither, Aderyn) is clean, with every finding triaged in [docs/security/static-analysis.md](../docs/security/static-analysis.md) (`slither .` and `aderyn` run from this directory; CI: `.github/workflows/static-analysis.yml`). An external audit is still to do before any mainnet use.
