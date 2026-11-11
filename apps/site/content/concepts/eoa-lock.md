# EOA lock (XIP-0001)

**XIP-0001** is an opt-in protocol rule that closes the biggest hole in EIP-7702 smart accounts: the old private key.

<span class="status ok">Built and tested</span> in the node (block validity, block building, mempool, RPC) and in the account contract. Active from genesis on dev chains (`"xip0001Block": 0`).

## The problem

When you delegate an existing MetaMask address to the [xDEVS Smart Account](/concepts/smart-accounts) with EIP-7702, the address becomes a smart account, but its private key still works. On any other chain, that key can sign raw transactions that never enter the account code: plain transfers, direct token calls, or a new delegation that removes the account entirely. All of those skip the [Guardian Vault](/concepts/guardian-vault). A thief with the key defeats every protection.

## The rule

The account writes a **lock word** into a fixed storage slot. When the address is delegated to the canonical implementation at `0x…1005`, the node reads that word before accepting any transaction from the address:

| Lock word | Status | What the node accepts from the address |
|---|---|---|
| 0 | Unlocked | Everything (normal Ethereum behaviour) |
| 1 | **Locked** | Only **value-0 calls to itself**, at most one per block, gas ≤ 2,000,000, max fee ≤ 2 × base fee. EIP-7702 authorizations signed by the key are skipped |
| 2 | **Sealed** | Nothing at all. Authorizations are skipped |

A call to itself enters the account's own code, where the vault policy applies to the EOA key exactly as to any other signer. So with the lock on, the old key can only do what the vault allows instantly; everything else gets queued and can be cancelled.

The address is **sealed** automatically while guardians have [frozen](/concepts/guardian-vault#guardian-freeze) the account and after a social recovery revoked the EOA key.

## Turning it on and off

| Function | Who | Effect |
|---|---|---|
| `enableEoaLock()` | Owner | Instant. Requires an EIP-7702 delegation and an enabled vault |
| `disableEoaLock()` | Owner | **Delayed** like every policy change, and cancellable. Refused while the EOA key is revoked |
| `refreshEoaLock()` | Anyone | Re-derives the word, for example after a freeze expired on its own |

## What the node does

- **Block validity and building:** a block containing a transaction that breaks the rule is invalid, like one with a bad nonce. The block builder skips such transactions.
- **Mempool:** checked on admission and re-checked on every new head.
- **RPC:** `eth_sendRawTransaction` and `eth_estimateGas` return error **`-32003`** with `data.reason` set to `XIP0001_SEALED`, `XIP0001_NOT_SELF_CALL`, `XIP0001_VALUE_NOT_ZERO`, `XIP0001_GAS_LIMIT` or `XIP0001_FEE_CAP`. `eth_call` is unchanged.
- **`xdevs_getEoaLock(address, block)`** reports the state:

```json
{
  "active": true,
  "status": "locked",
  "word": "0x0000000000000000000000000000000000000000000000000000000000000001",
  "delegatedTo": "0x0000000000000000000000000000000000001005"
}
```

## Why this doesn't break xDEVS Chain's rules

XIP-0001 is a **validity rule for transactions not yet included**, of the same kind as nonce and balance checks. It never re-evaluates or reverses a finalized block. Its only input is a storage word the account's own code wrote, at the owner's request or through guardians the owner chose. No validator, governance or third party can lock anyone's address.

## Residual risks

- Protection starts when the owner enables the lock.
- **Bounded fee burn while locked:** a thief with the not-yet-revoked key can send one ≤ 2,000,000-gas self-call per block at ≤ 2 × base fee, mostly burned or paid to the Treasury, until guardians freeze the account.
- **Third-party tokens with raw-`ecrecover` permits** can still be authorized off-chain with the EOA key. XDRC-20 and Permit2Expiring are safe: they check signatures of accounts with code only through ERC-1271.
- Without guardians, a stolen EOA key can't be revoked; the lock limits the thief to the vault policy indefinitely. Factory (passkey) accounts with no private key remain the strongest option.

:::info[Dev chain note]
The rule is enforced by the dev node, but the account implementation is not yet placed at `0x…1005` in the dev genesis, so on a default dev chain no address can be in the locked state yet. See [Predeployed contracts](/concepts/predeploys).
:::

Full specification: [XIP-0001](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xip-0001-eoa-lock.md).
