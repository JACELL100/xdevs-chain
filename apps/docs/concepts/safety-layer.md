# The user-safety layer

xDEVS Chain's signature feature is a set of protections built from standard EVM pieces (system contracts at fixed addresses) plus one opt-in node rule and a set of RPC extensions. Together they address the ways ordinary users actually lose money.

::: tip Three rules hold for every feature
1. **Opt-in.** A user who turns nothing on gets ordinary Ethereum behaviour.
2. **Never reverses a finalized transaction.** Protections decide what can happen *next*; they never undo what already happened.
3. **No admin key.** No protocol key, validator or governance vote can freeze, seize or move user funds. A freeze can only come from guardians the account owner picked.
:::

## The pieces

| Threat | Protection | Where it lives | Status |
|---|---|---|---|
| Lost seed phrase | [Smart accounts with passkeys](/concepts/smart-accounts) and social recovery | `XdevsSmartAccount` contract | <span class="status ok">Contracts built</span> |
| Stolen key or device | [Guardian Vault](/concepts/guardian-vault): time-locked transfers with a cancel window | Account contract | <span class="status ok">Contracts built</span> |
| Thief and owner both hold keys | [Guardian freeze](/concepts/guardian-vault#guardian-freeze) | Account contract | <span class="status ok">Contracts built</span> |
| Stolen key of an existing MetaMask address | [EOA lock (XIP-0001)](/concepts/eoa-lock) | Account contract + node validity rule | <span class="status ok">Built (node enforces it)</span> |
| Forgotten unlimited approvals | [Expiring approvals](/concepts/approvals): XDRC-20, Permit2Expiring, vault allowance policy | Contracts | <span class="status ok">Contracts built</span> |
| Look-alike names, address poisoning | [XNS `.xdevs` names](/concepts/xns) with confusable-skeleton checks | `XNSRegistry` contract | <span class="status ok">Contracts built</span> |
| Look-alike recipient addresses | `xdevs_checkRecipient` RPC; SDK offline helper | Node RPC / SDK | <span class="status planned">RPC planned (Phase 4)</span>, SDK helper built |
| Blind signing | `xdevs_simulateTransaction`: asset changes and plain-language warnings | Node RPC | <span class="status planned">Planned (Phase 4)</span> |
| Front-running (MEV) | Encrypted mempool | Consensus / mempool | <span class="status planned">Research (Phase 5)</span> |
| Bridge hacks | Light-client bridges only | Policy | <span class="status planned">Planned (Phase 7)</span> |

"Contracts built" means the Solidity is written and covered by unit, fuzz and invariant tests in Foundry. The contracts are **not yet deployed on any network** (they are not in the local dev genesis either) and have **not been externally audited**.

## How a protected account fits together

1. **Get a smart account.** A new user creates a passkey account through the factory (no seed phrase at all). An existing MetaMask user delegates their address to the xDEVS Smart Account with an EIP-7702 transaction and keeps the same address.
2. **Add guardians.** Friends, a second device, a hardware key or a recovery service. A threshold of them (for example 2 of 3) can recover the account after a delay.
3. **Turn on the Guardian Vault.** Everyday small payments stay instant. Anything above your threshold, any approval, and any call to an unknown contract waits out a delay (default 24 h) during which you or your guardians can cancel it.
4. **For a 7702 account, enable the EOA lock.** Without it, the old private key could still sign raw transactions that skip the vault. With it, the node accepts only calls that go through the account's own code.
5. **Use expiring approvals and `.xdevs` names.** Approvals die on their own, and names can't be impersonated by look-alikes.

If a key is stolen, the thief is limited to the instant allowance (daily limit). Everything bigger is queued, your guardians can cancel it, freeze the account and recover it to your new device.

## Read the specs

- [Smart accounts, Guardian Vault & paymaster spec](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/smart-accounts.md)
- [XIP-0001: EOA lock](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xip-0001-eoa-lock.md)
- [XNS and expiring approvals spec](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/xns-and-approvals.md)
- [ADR-0006: Safety in accounts and RPC, never in finality](https://github.com/JACELL100/xdevs-chain/blob/main/docs/adr/0006-safety-in-accounts-and-rpc.md)
