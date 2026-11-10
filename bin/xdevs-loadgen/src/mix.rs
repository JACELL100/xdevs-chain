//! Transaction mix: `--mix transfer=80,erc20=20[,storage=5]` and a deterministic
//! weighted picker (smooth weighted round-robin, so every window of `total` picks has
//! exactly the configured proportions).

use serde::Serialize;
use std::{fmt, str::FromStr};

/// Kind of load transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LoadKind {
    /// Native value transfer (21 000 gas).
    Transfer,
    /// `transfer(address,uint256)` on the embedded ERC-20-style token.
    Erc20,
    /// `stress(uint256)` on the token contract: writes fresh storage slots (state-root
    /// stress).
    Storage,
}

impl LoadKind {
    /// All kinds, in display order.
    pub const ALL: [LoadKind; 3] = [LoadKind::Transfer, LoadKind::Erc20, LoadKind::Storage];

    /// Lower-case name used on the command line and in reports.
    pub const fn name(self) -> &'static str {
        match self {
            LoadKind::Transfer => "transfer",
            LoadKind::Erc20 => "erc20",
            LoadKind::Storage => "storage",
        }
    }
}

impl fmt::Display for LoadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for LoadKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "transfer" | "native" => Ok(LoadKind::Transfer),
            "erc20" | "token" => Ok(LoadKind::Erc20),
            "storage" | "sstore" => Ok(LoadKind::Storage),
            other => Err(format!(
                "unknown tx kind {other:?} (expected transfer, erc20 or storage)"
            )),
        }
    }
}

/// Weighted set of tx kinds (only kinds with weight > 0 are kept).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mix {
    weights: Vec<(LoadKind, u32)>,
}

impl Mix {
    /// Weight of `kind` (0 if absent).
    pub fn weight(&self, kind: LoadKind) -> u32 {
        self.weights
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(0, |(_, w)| *w)
    }

    /// Whether `kind` is part of the mix.
    pub fn contains(&self, kind: LoadKind) -> bool {
        self.weight(kind) > 0
    }

    /// Sum of all weights.
    pub fn total(&self) -> u32 {
        self.weights.iter().map(|(_, w)| w).sum()
    }

    /// `(kind, weight)` pairs.
    pub fn weights(&self) -> &[(LoadKind, u32)] {
        &self.weights
    }

    /// A fresh picker over this mix.
    pub fn picker(&self) -> MixPicker {
        MixPicker {
            weights: self.weights.clone(),
            current: vec![0; self.weights.len()],
            total: i64::from(self.total()),
        }
    }
}

impl Default for Mix {
    fn default() -> Self {
        Self {
            weights: vec![(LoadKind::Transfer, 100)],
        }
    }
}

impl fmt::Display for Mix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .weights
            .iter()
            .map(|(k, w)| format!("{k}={w}"))
            .collect();
        f.write_str(&parts.join(","))
    }
}

impl FromStr for Mix {
    type Err = String;

    /// `kind=weight[,kind=weight...]`; a bare `kind` means weight 1.
    fn from_str(s: &str) -> Result<Self, String> {
        let mut weights: Vec<(LoadKind, u32)> = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (kind, weight) = match part.split_once('=') {
                Some((k, w)) => {
                    let w: u32 = w
                        .trim()
                        .parse()
                        .map_err(|_| format!("invalid weight in {part:?}"))?;
                    (k.parse::<LoadKind>()?, w)
                }
                None => (part.parse::<LoadKind>()?, 1),
            };
            if weights.iter().any(|(k, _)| *k == kind) {
                return Err(format!("{kind} listed twice in --mix"));
            }
            if weight > 1_000_000 {
                return Err(format!("weight of {kind} too large"));
            }
            weights.push((kind, weight));
        }
        weights.retain(|(_, w)| *w > 0);
        if weights.is_empty() {
            return Err("--mix needs at least one kind with a weight > 0".into());
        }
        Ok(Self { weights })
    }
}

/// Deterministic weighted picker (nginx smooth weighted round-robin).
#[derive(Debug, Clone)]
pub struct MixPicker {
    weights: Vec<(LoadKind, u32)>,
    current: Vec<i64>,
    total: i64,
}

impl MixPicker {
    /// Next kind.
    pub fn next_kind(&mut self) -> LoadKind {
        let mut best = 0;
        for (i, (_, w)) in self.weights.iter().enumerate() {
            self.current[i] += i64::from(*w);
            if self.current[i] > self.current[best] {
                best = i;
            }
        }
        self.current[best] -= self.total;
        self.weights[best].0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn parses_mixes() {
        let m: Mix = "transfer=80,erc20=20".parse().unwrap();
        assert_eq!(m.weight(LoadKind::Transfer), 80);
        assert_eq!(m.weight(LoadKind::Erc20), 20);
        assert!(!m.contains(LoadKind::Storage));
        assert_eq!(m.total(), 100);
        assert_eq!(m.to_string(), "transfer=80,erc20=20");

        let m: Mix = " transfer = 3 , storage ".parse().unwrap();
        assert_eq!(
            m.weights(),
            &[(LoadKind::Transfer, 3), (LoadKind::Storage, 1)]
        );

        // Zero weights are dropped.
        let m: Mix = "transfer=1,erc20=0".parse().unwrap();
        assert!(!m.contains(LoadKind::Erc20));

        assert_eq!(Mix::default().to_string(), "transfer=100");
    }

    #[test]
    fn rejects_bad_mixes() {
        assert!("".parse::<Mix>().is_err());
        assert!("transfer=0".parse::<Mix>().is_err());
        assert!("swap=10".parse::<Mix>().is_err());
        assert!("transfer=x".parse::<Mix>().is_err());
        assert!("transfer=-1".parse::<Mix>().is_err());
        assert!("transfer=1,transfer=2".parse::<Mix>().is_err());
    }

    #[test]
    fn picker_matches_weights_exactly() {
        let m: Mix = "transfer=80,erc20=15,storage=5".parse().unwrap();
        let mut p = m.picker();
        let mut counts: HashMap<LoadKind, u32> = HashMap::new();
        for _ in 0..1000 {
            *counts.entry(p.next_kind()).or_default() += 1;
        }
        assert_eq!(counts[&LoadKind::Transfer], 800);
        assert_eq!(counts[&LoadKind::Erc20], 150);
        assert_eq!(counts[&LoadKind::Storage], 50);

        // Smooth: a 1:1 mix alternates.
        let mut p = "transfer=1,erc20=1".parse::<Mix>().unwrap().picker();
        let seq: Vec<_> = (0..4).map(|_| p.next_kind()).collect();
        assert_ne!(seq[0], seq[1]);
        assert_ne!(seq[1], seq[2]);
    }
}
