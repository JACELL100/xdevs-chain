//! Sender accounts: the 10 well-known dev keys (Anvil / Hardhat mnemonic, funded by the
//! dev genesis), a private-key file, or fresh keys funded from a funder key.

use anyhow::{bail, Context};
use std::path::Path;
use xdevs_crypto::{generate_signer, signer_from_hex, PrivateKeySigner};
use xdevs_genesis::DEV_PRIVATE_KEYS;

/// Where sender keys come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderSource {
    /// The first N dev keys (N <= 10).
    Dev,
    /// Hex keys from a file, one per line (`#` comments and blank lines ignored).
    File(std::path::PathBuf),
    /// N fresh random keys funded from this key spec (see [`parse_key_spec`]).
    Fund(String),
}

/// A key given as `0x<64 hex>` / `<64 hex>`, or `dev:<i>` / `<i>` for dev key `i`.
pub fn parse_key_spec(spec: &str) -> anyhow::Result<PrivateKeySigner> {
    let s = spec.trim();
    let idx = s.strip_prefix("dev:").unwrap_or(s);
    if let Ok(i) = idx.parse::<usize>() {
        let Some(k) = DEV_PRIVATE_KEYS.get(i) else {
            bail!("dev key index {i} out of range (0..=9)");
        };
        return Ok(signer_from_hex(k)?);
    }
    signer_from_hex(s).map_err(|e| anyhow::anyhow!("invalid private key: {e}"))
}

/// Parse a key file's contents.
pub fn parse_key_file(contents: &str) -> anyhow::Result<Vec<PrivateKeySigner>> {
    contents
        .lines()
        .enumerate()
        .map(|(n, l)| (n, l.split('#').next().unwrap_or("").trim()))
        .filter(|(_, l)| !l.is_empty())
        .map(|(n, l)| {
            signer_from_hex(l)
                .map_err(|e| anyhow::anyhow!("line {}: invalid private key: {e}", n + 1))
        })
        .collect()
}

/// Load `count` sender keys (fresh keys for [`SenderSource::Fund`]; funding happens
/// later, over RPC).
pub fn load_senders(source: &SenderSource, count: usize) -> anyhow::Result<Vec<PrivateKeySigner>> {
    if count == 0 {
        bail!("--senders must be at least 1");
    }
    match source {
        SenderSource::Dev => {
            if count > DEV_PRIVATE_KEYS.len() {
                bail!(
                    "only {} dev keys exist; use --fund-from <key> to create {count} fresh senders \
                     or --private-keys <file>",
                    DEV_PRIVATE_KEYS.len()
                );
            }
            DEV_PRIVATE_KEYS[..count]
                .iter()
                .map(|k| Ok(signer_from_hex(k)?))
                .collect()
        }
        SenderSource::File(path) => {
            let keys = read_key_file(path)?;
            if keys.len() < count {
                bail!(
                    "{} has {} keys but --senders is {count}",
                    path.display(),
                    keys.len()
                );
            }
            Ok(keys.into_iter().take(count).collect())
        }
        SenderSource::Fund(_) => Ok((0..count).map(|_| generate_signer()).collect()),
    }
}

fn read_key_file(path: &Path) -> anyhow::Result<Vec<PrivateKeySigner>> {
    let s = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file {}", path.display()))?;
    parse_key_file(&s).with_context(|| format!("in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_genesis::DEV_ACCOUNTS;

    #[test]
    fn key_specs() {
        assert_eq!(parse_key_spec("0").unwrap().address(), DEV_ACCOUNTS[0]);
        assert_eq!(parse_key_spec("dev:3").unwrap().address(), DEV_ACCOUNTS[3]);
        assert_eq!(
            parse_key_spec(DEV_PRIVATE_KEYS[5]).unwrap().address(),
            DEV_ACCOUNTS[5]
        );
        assert!(parse_key_spec("10").is_err());
        assert!(parse_key_spec("0xzz").is_err());
    }

    #[test]
    fn key_files_and_sources() {
        let text = format!(
            "# senders\n{}\n\n  {}  # second\n",
            DEV_PRIVATE_KEYS[1], DEV_PRIVATE_KEYS[2]
        );
        let keys = parse_key_file(&text).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1].address(), DEV_ACCOUNTS[2]);
        assert!(parse_key_file("nothex\n").is_err());

        let dev = load_senders(&SenderSource::Dev, 10).unwrap();
        assert_eq!(dev[9].address(), DEV_ACCOUNTS[9]);
        assert!(load_senders(&SenderSource::Dev, 11).is_err());
        assert!(load_senders(&SenderSource::Dev, 0).is_err());
        let fresh = load_senders(&SenderSource::Fund("0".into()), 3).unwrap();
        assert_eq!(fresh.len(), 3);
        assert_ne!(fresh[0].address(), fresh[1].address());
    }
}
