# Guardian Vault & guardian freeze

**Goal:** if a key or device is stolen, the thief can't instantly drain everything.

The Guardian Vault is a policy inside the [xDEVS Smart Account](/concepts/smart-accounts). Small everyday actions stay instant. Risky actions are **queued** for a delay, during which the owner or the guardians can cancel them.

<span class="status ok">Contracts built and tested</span> · Not yet in the dev genesis · Not audited

## The policy

| Setting | Meaning | Default / bounds |
|---|---|---|
| `enabled` | Master switch | Off (opt-in) |
| `delay` | How long queued actions wait | 24 h; 1 h – 7 days |
| `nativeThreshold` | Max tXDS for one instant transaction | Set by the user |
| `dailyLimit` | Max tXDS of instant spending per UTC day | Set by the user |
| `blockUnlimitedApprovals` | Reject (instead of queue) unlimited approvals to non-whitelisted spenders | Recommended on |
| `allowInstantSignatures` | Accept off-chain (ERC-1271) signatures without on-chain pre-approval | Off |
| `dailyGasBudget` | Max ERC-4337 gas prefund per UTC day | 1 tXDS |
| Whitelist | Up to 64 trusted contracts, recipients and spenders | Empty |
| Token limits | Per-transaction and daily limits per token, in token units | None |

## What is instant and what is queued

A batch is instant only if **every** call in it is instant; otherwise the whole batch is queued atomically.

| Action | Instant when |
|---|---|
| Sending tXDS to a non-whitelisted address | Within `nativeThreshold` and today's `dailyLimit` |
| Token transfers out | Recipient whitelisted, or within that token's configured limits |
| Approvals | Revocations (always), or spender whitelisted. Unlimited approvals to others are rejected or queued |
| Calls with data to other contracts | Target whitelisted |
| Account management | Only a short list: add a signer or session key, revoke keys, cancel, enable the EOA lock |
| Everything else | Queued |

## Queue lifecycle

1. A queued action emits `VaultActionQueued(id, target, value, data, executeAfter)`. Wallets and an alert bot watch for it.
2. **During the delay**, the owner (any signer) can `cancel(id)` alone. Guardians can cancel too: each guardian's call is a vote, and the action is cancelled when the votes reach the guardian threshold, so one rogue guardian can't veto everything.
3. **After the delay**, anyone can `execute(id)`, within a 14-day window.

### A thief can't switch it off

Changing the policy is itself queued with the current delay: disabling the vault, editing the whitelist or limits, removing signers, changing guardians or the recovery delay, disabling the EOA lock, and upgrades. Turning the vault **on** is instant.

## Guardian freeze

If both the owner and a thief hold signer keys, the thief could cancel every recovery while guardians cancel the thief's queued actions: a stalemate. The guardian freeze breaks it.

- When a **threshold of guardians** calls `freeze()`, the account is frozen for its freeze duration (default **30 days**, configurable 3–60 days, and always at least the recovery delay plus 3 days).
- **While frozen, nothing executes**: no signer, session key or EOA-key action, no queued action, no owner cancel, no recovery cancel, and every off-chain signature is invalid. For a 7702 account the address is also **sealed** at the protocol level by [XIP-0001](/concepts/eoa-lock).
- Guardians can still cancel queued actions, approve and execute a recovery, and unfreeze.
- A freeze ends with `unfreeze()` at the threshold, a completed recovery, or expiry.
- **Anti-hostage cooldown:** after a freeze ends, a new one is only possible once the account has been unfrozen for as long as it was frozen. Guardians can't keep an account frozen more than about half the time without executing a public, attributable recovery.

### Example: stolen key

An illustrative flow. Its steps are covered by the Foundry tests `test_demo_stolenKey_guardianCancelsWithinWindow` and `test_demo_ownerVsThief_freezeBreaksStalemate`:

1. A thief steals an owner key and tries to send a large amount. It is above the threshold, so it's queued for 24 h.
2. The owner gets an alert. Two of three guardians vote to cancel it.
3. Guardians freeze the account and recover it to the owner's new passkey. The stolen key is now useless.

## Trade-offs

:::warning[Guardians are trusted for safety, not only for recovery]
A guardian threshold can freeze the account and recover it to keys of its choice without the owner being able to cancel. This is the standard social-recovery trust model (as in Argent or Safe recovery modules). Choose guardians you trust; a single guardian below the threshold can do nothing alone.
:::

- The vault only protects assets **held by the account**.
- The delay doesn't stop you from willingly paying a scammer after it ends; it gives you time to notice.
- A thief can still spend up to the daily limit (and token daily limits) until guardians freeze the account.

Full details: [smart-accounts spec, sections 7–8](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/smart-accounts.md).
