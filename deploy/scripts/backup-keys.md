# Validator key backup (offline, encrypted)

Plan §24.8 and §11.8. Applies to every validator **consensus key**, its **keystore password**,
each node's **`node_key.json`** (the libp2p identity, so the peer id stays stable), the
**Ansible vault password**, and the faucet reserve key (see [faucet runbook](../../docs/runbooks/faucet.md)).

Rules:

- Never commit a key to the repo, not even vault-encrypted.
- Never put a key in cloud storage unencrypted. That includes Oracle Object Storage, Google
  Drive and password-manager attachments without client-side encryption.
- Never run the same consensus key on two machines. A restored backup is used **only** after
  the original host is stopped for good (§11.8; the sign guard protects one host only).
- Keys are generated on a trusted workstation, never on a server or in CI.

## What to back up

| Item | Where it is created | Secret? |
|---|---|---|
| `consensus_key.json` (keystore v3, Ed25519 seed inside) | `xdevs-cli testnet init --password-file ...` or `xdevs-cli keygen --consensus --out ...` | Yes. Encrypted with the keystore password |
| Keystore password | You choose it (strong, at least 5 random words) | **Yes** |
| `node_key.json` (libp2p identity, unencrypted) | `testnet init` / `xdevs-cli keygen --node-key` | Medium. Lets someone impersonate the peer id, but cannot sign blocks |
| Ansible vault password | You choose it | **Yes**. It decrypts `vault.yml` and every `*.vault` key file |
| `genesis.json` + its SHA-256 | `testnet init` | No (public once the network launches) |

## Procedure (once per key set, and after every rotation)

1. **Prepare two USB drives** (two different brands if possible). Format each one with an
   encrypted container: LUKS (`cryptsetup luksFormat`) on Linux, or VeraCrypt on
   Windows/macOS/Linux. Use a passphrase that differs from the keystore password.
2. On the workstation that generated the keys, copy to **each** drive:
   ```text
   xdevs-keys-<network>-<YYYY-MM-DD>/
     validator-1/consensus_key.json  node_key.json
     validator-2/...
     rpc-1/node_key.json
     ...
     genesis.json   genesis.sha256
     SHA256SUMS     (sha256sum of every file above)
     README.txt     (which host is which, and the date; NO passwords)
   ```
3. Verify each copy: `sha256sum -c SHA256SUMS` from the mounted drive.
4. **Paper copies** (plan §24.8): write the keystore password and the vault password on
   paper, plus the passphrase of the USB containers. Store them in a different place from the
   USB drives (for example a home safe, and a sealed envelope with someone you trust).
5. Store the two USB drives in two different physical places. Do not label them with what
   they contain.
6. **Remove the plaintext** from the workstation once the vault-encrypted copies exist
   (`deploy/scripts/import-testnet-keys.sh`) and the USB backups are verified:
   ```bash
   shred -u xdevs-testnet/node*/consensus_key.json xdevs-testnet/node*/node_key.json keystore-password.txt
   ```
   (On SSDs `shred` is best-effort. Prefer generating the keys inside an encrypted volume or
   on a live USB system in the first place.)
7. Write an entry in the ops log: date, key set, which drives, who holds the paper copies.
   Never write the secrets themselves.

## Restore test (with the monthly snapshot restore test, plan §24.8)

Once a month: mount one USB drive on an offline machine, run `sha256sum -c SHA256SUMS`, and
compare the files with the vault copies (`ansible-vault view <file>.vault | sha256sum`).
Unmount it, and log the result. Never load the key into a running node as part of a test.

There is no CLI command yet that only checks a consensus keystore password
(`xdevs-cli address` reads account keystores). Until one exists, the password is proven each
time a validator starts with it.

## When a key leaks or a host is compromised

Follow [rotate-consensus-key.md](../../docs/runbooks/rotate-consensus-key.md) (compromise path)
and [security-report.md](../../docs/runbooks/security-report.md). Make a new backup after the
rotation and destroy the old drives' copies of the retired key.
