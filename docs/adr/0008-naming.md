# ADR-0008: Naming (xDEVS Chain, XDS, tXDS, dev, .xdevs, XDRC-20, XNS)

- Status: Accepted (pending the external checks below)
- Plan reference: §1.4

## Context
The chain is built by the startup **THE xDEVS** and is named after the company, so every user of the chain learns the brand. Names must be consistent across code, RPC, docs and tooling.

## Decision
| Item | Value |
|---|---|
| Chain | **xDEVS Chain** |
| Native coin | **XDS**, 18 decimals; the testnet coin is **tXDS** |
| Smallest unit | **dev** (1 XDS = 10^18 dev) |
| Name service | **XNS** (xDEVS Name Service), suffix `.xdevs` (e.g. `alice.xdevs`) |
| Token standard | **XDRC-20** (ERC-20 compatible, with expiring approvals) |
| RPC namespace / code prefix | `xdevs_` / `xdevs-` (crates; binaries `xdevs` and `xdevs-cli`) |
| Local dev chain id | 7117 (local dev only, never a public network) |

## Alternatives
"XChain", "XCoin", "XDEV" and "XDV" were considered and rejected because they are already in use (web search, Nov 2026).

## Consequences
- The brand is visible everywhere. Sharing the company name makes it harder to show separation if the chain later moves to a separate foundation.
- Checks still open before the name is announced: XDS on CoinGecko and CoinMarketCap; trademark searches for "xDEVS", "xDEVS Chain" and "XDS" (IP India, USPTO, EUIPO, classes 9/36/42); the domain, GitHub org and social handles.

## Revisit
If any trademark or ticker check fails, or if a separate foundation is created (§30.2).
