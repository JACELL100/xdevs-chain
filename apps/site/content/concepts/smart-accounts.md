# Smart accounts & passkeys

The **xDEVS Smart Account** is a canonical account contract, predeployed at a fixed address, that gives every user passkey login, social recovery, session keys and the [Guardian Vault](/concepts/guardian-vault). It is compatible with ERC-4337 v0.9 (UserOperations, bundlers, paymasters), ERC-1271 (contract signatures) and EIP-7702 (delegating an existing address).

<span class="status ok">Contracts built and tested</span> · Not yet in the dev genesis · Not audited

## Two ways to get one

### New user: passkey only, no seed phrase

1. The wallet creates a **passkey** (Face ID, fingerprint, or a security key) and computes the counterfactual account address with the factory (`0x…1006`).
2. Someone funds that address, or a [paymaster](#paymasters-gas-sponsorship) sponsors the first transaction.
3. The first UserOperation, signed by the passkey, deploys the account (an ERC-1967 proxy to the implementation at `0x…1005`) with CREATE2. The address commits to the initial owners, so nobody can front-run it with other owners.

There is no private key anywhere: the account is controlled only by its passkeys and other signers.

### Existing MetaMask address: EIP-7702

1. The EOA signs an EIP-7702 authorization that delegates to `0x…1005` (a type-4 transaction; the node supports these).
2. The EOA initializes the account on itself, in the same transaction or later.
3. The address, its history and its names stay the same.

The old private key remains valid as an implicit signer. To stop it from bypassing the vault with raw transactions, turn on the [EOA lock (XIP-0001)](/concepts/eoa-lock).

## Signers

An account has up to **16 signers**, of two kinds:

| Kind | Key | Typical use |
|---|---|---|
| ECDSA (secp256k1) | An Ethereum address | MetaMask, hardware wallets |
| Passkey (P-256 / secp256r1) | A WebAuthn public key | Face ID, fingerprint, security keys |

Passkey signatures are verified with the **`P256VERIFY` precompile at `0x100`** (EIP-7951), which the node provides natively (revm Osaka rules). The WebAuthn checks match OpenZeppelin's (`User-Present` and `User-Verified` flags, challenge binding, low-s). The origin, rpId and signature counter are checked by the authenticator and wallet, not on-chain.

## Social recovery

- Choose up to **16 guardians** (other people's addresses, your other devices, a recovery service) and a threshold such as 2-of-3.
- Guardians approve a new signer set. When the threshold is reached, a **recovery delay** starts (default **48 hours**, configurable between 1 hour and 30 days).
- After the delay, anyone can execute the recovery. It replaces all signers, revokes all session keys, invalidates every queued vault action, and **revokes the old EOA key** of a 7702 account (unless the new signer set lists it).
- The owner can cancel a pending recovery at any time before it executes, **except while the account is frozen** (see [guardian freeze](/concepts/guardian-vault#guardian-freeze)).

## Session keys

Session keys are limited keys for games and dApps: a time window, a lifetime native spend cap, up to 32 allowed `(contract, function)` pairs and per-token caps. A session key can never call the account itself, and anything the vault would queue simply reverts for a session key. You can revoke one key or all of them instantly.

## Paymasters (gas sponsorship)

- **`XdevsPaymaster`** (`0x…1007`): apps deposit tXDS to sponsor their users' gas, with per-operation and per-account caps, an allowlist, or a signer that approves each operation.
- **`StablecoinPaymaster`** (example contract, normal deployment): pay gas in an approved ERC-20 at an operator-set rate, capped by a rate the user signs.

## Recommended wallet defaults

With the vault on: an instant threshold of 1,000 tXDS, a daily limit of a few thresholds, a 24 h delay, unlimited approvals blocked, instant off-chain signatures off, and 2-of-3 guardians.

## Limitations

The spec lists these honestly; the main ones:

- It protects only assets **held by the account**. Approvals given before the vault was enabled, or funds elsewhere, are out of scope.
- It doesn't stop you from **willingly paying a scammer after the delay**; the delay buys time to notice and cancel.
- **Whitelists are trust.** A whitelisted contract can be called instantly with any calldata.
- A thief with an owner key can still spend up to the **daily limit** per day until guardians freeze the account.
- Accounts without guardians have no recovery and no freeze.
- Daily limits use UTC calendar days, not rolling 24-hour windows.

Full details: [smart-accounts spec, §12 Limitations](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/smart-accounts.md#12-limitations-documented-honestly).
