# Expiring approvals

Stale unlimited token approvals are one of the main ways wallets get drained: you approve a contract "forever" once, forget about it, and years later the contract (or its upgraded version) is exploited. On xDEVS Chain, **approvals expire**.

<span class="status ok">Contracts built and tested</span> · Not yet deployed · Not audited

## Three layers

| Layer | For | How |
|---|---|---|
| **XDRC-20** token standard | New tokens | ERC-20 and EIP-2612 compatible, plus an expiry on every allowance |
| **Permit2Expiring** (`0x…1008`) | Any existing ERC-20 | A Permit2-style contract where every allowance has an expiry |
| **Vault allowance policy** | Smart accounts | Unlimited approvals to non-whitelisted spenders are blocked or queued |

## XDRC-20

`XDRC20` is a fully compatible ERC-20 / EIP-2612 token (built on OpenZeppelin v5) with these additions:

```solidity
function approve(address spender, uint256 value, uint64 expiry) external returns (bool);
function allowanceExpiry(address owner, address spender) external view returns (uint64);
function revokeAll() external;   // kills every allowance of the caller in O(1)
function permitWithExpiry(address owner, address spender, uint256 value, uint64 expiry,
                          uint256 deadline, bytes signature) external;
```

- The legacy `approve(spender, value)` and the standard `permit` also expire, after the token's **default duration** (recommended 30 days). Even `type(uint256).max` expires.
- `allowance()` returns 0 once an approval has expired. `transferFrom` then reverts with `AllowanceExpired`.
- `revokeAll()` bumps an epoch, so every earlier approval of the caller dies at once.
- Permit signatures from accounts with code (smart accounts, 7702-delegated EOAs) are checked **only** through ERC-1271. While the Guardian Vault is on, the account refuses off-chain signatures, so a thief with an EOA key can't sign a permit to drain it.

### For dApp developers

dApps that "approve the maximum once, forever" will see the allowance drop to 0 after the default duration. This is intended. Integrators should:

- read `allowance()` before every pull;
- prefer `approve(spender, amount, expiry)` or `permitWithExpiry`, detected with `supportsInterface(type(IXDRC20).interfaceId)`;
- treat `AllowanceExpired` as "ask the user again".

## Permit2Expiring

For tokens that are not XDRC-20: approve `0x…1008` on the token once, then keep per-spender allowances there. The interface follows Uniswap's `IAllowanceTransfer` (`approve(token, spender, amount, expiration)`, `permit`, `transferFrom`, `allowance`, `lockdown`), and every allowance needs an expiry. It also has `revokeAll()`.

## Canonical bridged tokens

A `CanonicalTokenFactory` (owned by governance) deploys at most **one** canonical XDRC-20 per origin asset, so wallets and explorers can tell the real bridged token from a copy with the same name or symbol (`isCanonical(token)`). Bridges themselves are Phase 7.

## Listing approvals

The planned `xdevs_getApprovals(owner)` RPC (Phase 4) will list all live approvals so wallets can show a one-click revoke screen. Until then, indexers can follow the `ApprovalWithExpiry` and `AllApprovalsRevoked` events described in the spec.

Full specification: [XNS and expiring approvals](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xns-and-approvals.md).
