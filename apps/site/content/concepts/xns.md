# XNS names & address poisoning

**XNS**, the xDEVS Name Service, gives addresses human-readable names ending in **`.xdevs`**, such as `alice.xdevs`. Unlike other name services, it refuses names that look like an existing one.

<span class="status ok">Contracts built and tested</span> · Not yet deployed · Not audited

## Look-alike protection

### Strict spelling

A label must be 3–63 characters of `a-z`, `0-9` and `-`, with no leading or trailing hyphen and no `--`. Invalid input is **rejected, never silently normalized**. Uppercase, spaces, every non-ASCII character (Cyrillic `а`, accented letters, zero-width characters, emoji) and punycode `xn--` are all refused, so Unicode homographs can't be registered at all.

### Confusable skeletons

Every label is mapped to a "skeleton" by replacing characters that look alike:

| Look-alike | Treated as |
|---|---|
| `0` | `o` |
| `1` | `l` |
| `rn` | `m` |
| `vv` | `w` |
| `cl` | `d` |

Only one active name may hold a given skeleton. While `paypal.xdevs` exists, `paypa1.xdevs` can't be registered; the same goes for `g00gle`/`google`, `rnetamask`/`metamask` and `vvallet`/`wallet`. The first registrant wins.

### Reserved names

Governance holds a short reserved list: `xdevs`, `the-xdevs`, `official`, `support`, `admin`, `faucet`.

## How names work

- **Registry, registrar, resolver and reverse registrar in one contract** at `0x…1004`, implementing the ENS (EIP-137) registry interface and standard namehash.
- **Registration** lasts 1–10 years; **renewal** is open to anyone. On testnet the fee is 0.
- **Expiry is safety-critical:** an expired name and all its subnames stop resolving immediately. After a 90-day grace period, anyone can register it again, and the new owner never inherits old records.
- **Primary names** are verified: `primaryName(addr)` re-checks the forward record at read time, so wallets should display it rather than the raw reverse record.

### Using XNS from libraries

- **ethers v6:** set the network's ENS registry address to `0x…1004`; resolution then works.
- **viem:** `getEnsAddress` uses ENS's UniversalResolver, which XNS does not include. Call the registry directly (`resolveAddr(node)`, `primaryName(addr)`) or use the [SDK](/developers/sdk) helpers.

## Address poisoning

Address poisoning means scammers send tiny or fake transfers from an address whose first and last characters match one you've used, hoping you copy the wrong one from your history.

| Defense | Status |
|---|---|
| SDK offline helper `isLookAlikeAddress(a, b)` (same first and last 4 hex characters, different address) | <span class="status ok">Built</span> |
| SDK name helpers: `normalizeXnsName`, `areConfusableXnsLabels`, `xnsNamehash` | <span class="status ok">Built</span> |
| RPC `xdevs_checkRecipient(from, to)`: first-time recipient? look-alike of a past one? has an XNS name? a contract? | <span class="status planned">Planned (Phase 4)</span> |
| RPC `xdevs_resolveName` / `xdevs_reverseName` | <span class="status planned">Planned (Phase 4)</span> |
| Explorer and wallet hiding zero-value and spam transfers, labelling look-alikes | <span class="status planned">Planned</span> |

```ts
import { normalizeXnsName, areConfusableXnsLabels, isLookAlikeAddress } from '@xdevs/sdk'

normalizeXnsName(' Alice ')                  // 'alice.xdevs'
normalizeXnsName('аlice')                    // throws: non-ASCII (Cyrillic 'а')
areConfusableXnsLabels('paypa1', 'paypal')   // true
isLookAlikeAddress(
  '0x1234567890abcdef1234567890abcdef1234abcd',
  '0x1234ffffffffffffffffffffffffffffffffabcd',
)                                            // true
```

Full specification: [XNS and expiring approvals](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xns-and-approvals.md).
