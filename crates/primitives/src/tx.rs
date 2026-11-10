//! Transaction helpers (plan §8.2).

use alloy_consensus::{
    crypto::RecoveryError, transaction::SignerRecoverable, Transaction, TxEnvelope,
};
use alloy_eips::eip2718::{Encodable2718, Typed2718};
use alloy_primitives::{Address, TxHash};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

/// A signed transaction together with its recovered sender.
///
/// Signature recovery is expensive, so it is done once (mempool entry / block import)
/// and the sender is carried along. Derefs to [`TxEnvelope`], so all
/// [`Transaction`] getters (`nonce()`, `gas_limit()`, `max_fee_per_gas()`, ...) work directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredTx {
    /// The signed transaction (types 0x00, 0x01, 0x02; 0x04 later).
    pub tx: TxEnvelope,
    /// Address recovered from the signature.
    pub sender: Address,
}

impl RecoveredTx {
    /// Recover the sender from the signature (EIP-2 low-s enforced).
    pub fn recover(tx: TxEnvelope) -> Result<Self, RecoveryError> {
        let sender = tx.recover_signer()?;
        Ok(Self { tx, sender })
    }

    /// Pair a transaction with an already known sender. The caller guarantees correctness.
    pub const fn new_unchecked(tx: TxEnvelope, sender: Address) -> Self {
        Self { tx, sender }
    }

    /// Transaction hash (`keccak256` of the EIP-2718 encoding) — identical to Ethereum's.
    pub fn hash(&self) -> TxHash {
        *self.tx.tx_hash()
    }

    /// EIP-2718 transaction type byte (0 = legacy).
    pub fn tx_type_byte(&self) -> u8 {
        self.tx.ty()
    }

    /// Length of the EIP-2718 encoding (what `eth_sendRawTransaction` receives).
    pub fn encoded_len(&self) -> usize {
        self.tx.encode_2718_len()
    }

    /// Tip per gas actually paid at `base_fee`, or `None` if `max_fee < base_fee`.
    pub fn effective_tip(&self, base_fee: u64) -> Option<u128> {
        self.tx.effective_tip_per_gas(base_fee)
    }
}

impl core::ops::Deref for RecoveredTx {
    type Target = TxEnvelope;
    fn deref(&self) -> &TxEnvelope {
        &self.tx
    }
}

/// Where a transaction lives in the canonical chain (value of the `Transactions` table).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
#[serde(rename_all = "camelCase")]
pub struct TxLocation {
    /// Block number containing the transaction.
    pub block_number: u64,
    /// Index of the transaction in the block body.
    pub index: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::eip2718::Decodable2718;
    use alloy_primitives::{address, b256, hex};

    /// The signed transaction from the EIP-155 specification example
    /// (private key 0x4646..46, chain id 1).
    const EIP155_RAW: &str = "f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

    #[test]
    fn eip155_vector_hash_and_sender() {
        let raw = hex::decode(EIP155_RAW).unwrap();
        let tx = TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap();
        let rtx = RecoveredTx::recover(tx).unwrap();
        assert_eq!(
            rtx.sender,
            address!("0x9d8A62f656a8d1615C1294fd71e9CFb3E4855A4F")
        );
        assert_eq!(rtx.tx_type_byte(), 0);
        assert_eq!(rtx.nonce(), 9);
        assert_eq!(rtx.chain_id(), Some(1));
        assert_eq!(rtx.encoded_len(), raw.len());
        assert_eq!(rtx.hash(), alloy_primitives::keccak256(&raw));
        assert_eq!(
            rtx.hash(),
            b256!("0x33469b22e9f636356c4160a87eb19df52b7412e8eac32a4a55ffe88ea8350788")
        );
    }

    #[test]
    fn tx_location_rlp_roundtrip() {
        let loc = TxLocation {
            block_number: 12,
            index: 3,
        };
        let enc = alloy_rlp::encode(loc);
        assert_eq!(alloy_rlp::decode_exact::<TxLocation>(&enc).unwrap(), loc);
    }
}
