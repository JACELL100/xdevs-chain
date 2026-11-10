//! Key/value codecs. Integer keys are big-endian so byte order == numeric order.

use crate::{tables::StateChangeSet, DbError};
use xdevs_primitives::{
    reexports::alloy_rlp, AccountInfo, Address, Bytes, Decodable2718, Encodable2718, Header,
    ReceiptEnvelope, TxEnvelope, TxLocation, B256, U256,
};

/// Encode a key or value to bytes.
pub trait Encode {
    /// Encoded bytes.
    fn encode(&self) -> Vec<u8>;
}

/// Decode a key or value from bytes produced by [`Encode`].
pub trait Decode: Sized {
    /// Decode, failing with [`DbError::Decode`] on malformed input.
    fn decode(bytes: &[u8]) -> Result<Self, DbError>;
}

fn fixed<const N: usize>(bytes: &[u8], what: &str) -> Result<[u8; N], DbError> {
    bytes
        .try_into()
        .map_err(|_| DbError::Decode(format!("{what}: expected {N} bytes, got {}", bytes.len())))
}

impl Encode for u64 {
    fn encode(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }
}
impl Decode for u64 {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        Ok(u64::from_be_bytes(fixed(bytes, "u64")?))
    }
}

impl Encode for B256 {
    fn encode(&self) -> Vec<u8> {
        self.to_vec()
    }
}
impl Decode for B256 {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        Ok(B256::from(fixed::<32>(bytes, "B256")?))
    }
}

impl Encode for Address {
    fn encode(&self) -> Vec<u8> {
        self.to_vec()
    }
}
impl Decode for Address {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        Ok(Address::from(fixed::<20>(bytes, "Address")?))
    }
}

impl Encode for U256 {
    fn encode(&self) -> Vec<u8> {
        self.to_be_bytes::<32>().to_vec()
    }
}
impl Decode for U256 {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        Ok(U256::from_be_bytes(fixed::<32>(bytes, "U256")?))
    }
}

impl Encode for (u64, u64) {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(16);
        v.extend_from_slice(&self.0.to_be_bytes());
        v.extend_from_slice(&self.1.to_be_bytes());
        v
    }
}
impl Decode for (u64, u64) {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        let b = fixed::<16>(bytes, "(u64, u64)")?;
        Ok((u64::decode(&b[..8])?, u64::decode(&b[8..])?))
    }
}

impl Encode for (Address, B256) {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(52);
        v.extend_from_slice(self.0.as_slice());
        v.extend_from_slice(self.1.as_slice());
        v
    }
}
impl Decode for (Address, B256) {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        let b = fixed::<52>(bytes, "(Address, B256)")?;
        Ok((Address::decode(&b[..20])?, B256::decode(&b[20..])?))
    }
}

impl Encode for (Address, u64) {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(28);
        v.extend_from_slice(self.0.as_slice());
        v.extend_from_slice(&self.1.to_be_bytes());
        v
    }
}
impl Decode for (Address, u64) {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        let b = fixed::<28>(bytes, "(Address, u64)")?;
        Ok((Address::decode(&b[..20])?, u64::decode(&b[20..])?))
    }
}

impl Encode for (Address, B256, u64) {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(60);
        v.extend_from_slice(self.0.as_slice());
        v.extend_from_slice(self.1.as_slice());
        v.extend_from_slice(&self.2.to_be_bytes());
        v
    }
}
impl Decode for (Address, B256, u64) {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        let b = fixed::<60>(bytes, "(Address, B256, u64)")?;
        Ok((
            Address::decode(&b[..20])?,
            B256::decode(&b[20..52])?,
            u64::decode(&b[52..])?,
        ))
    }
}

/// `None` = empty bytes; `Some` = the account's RLP (a list, never empty).
impl Encode for Option<AccountInfo> {
    fn encode(&self) -> Vec<u8> {
        self.as_ref().map(Encode::encode).unwrap_or_default()
    }
}
impl Decode for Option<AccountInfo> {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        if bytes.is_empty() {
            Ok(None)
        } else {
            AccountInfo::decode(bytes).map(Some)
        }
    }
}

/// `[n_accounts: u32 BE][address; n_accounts][(address, slot); ..]`.
impl Encode for StateChangeSet {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(4 + 20 * self.accounts.len() + 52 * self.slots.len());
        v.extend_from_slice(&(self.accounts.len() as u32).to_be_bytes());
        for a in &self.accounts {
            v.extend_from_slice(a.as_slice());
        }
        for key in &self.slots {
            v.extend_from_slice(&key.encode());
        }
        v
    }
}
impl Decode for StateChangeSet {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        let bad = || DbError::Decode("StateChangeSet: bad length".into());
        let (count, rest) = bytes.split_at_checked(4).ok_or_else(bad)?;
        let n = u32::from_be_bytes(fixed(count, "u32")?) as usize;
        let split = n
            .checked_mul(20)
            .filter(|l| *l <= rest.len())
            .ok_or_else(bad)?;
        let (accounts, slots) = rest.split_at(split);
        let (accounts, _) = accounts.as_chunks::<20>();
        let (slots, tail) = slots.as_chunks::<52>();
        if !tail.is_empty() {
            return Err(bad());
        }
        Ok(StateChangeSet {
            accounts: accounts.iter().map(|a| Address::from(*a)).collect(),
            slots: slots
                .iter()
                .map(|k| <(Address, B256)>::decode(k))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl Encode for String {
    fn encode(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}
impl Decode for String {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        String::from_utf8(bytes.to_vec()).map_err(|e| DbError::Decode(format!("String: {e}")))
    }
}

impl Encode for Vec<u8> {
    fn encode(&self) -> Vec<u8> {
        self.clone()
    }
}
impl Decode for Vec<u8> {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        Ok(bytes.to_vec())
    }
}

impl Encode for Bytes {
    fn encode(&self) -> Vec<u8> {
        self.to_vec()
    }
}
impl Decode for Bytes {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        Ok(Bytes::copy_from_slice(bytes))
    }
}

/// Implement the codecs through RLP.
macro_rules! rlp_codec {
    ($($ty:ty),*) => {$(
        impl Encode for $ty {
            fn encode(&self) -> Vec<u8> {
                alloy_rlp::encode(self)
            }
        }
        impl Decode for $ty {
            fn decode(bytes: &[u8]) -> Result<Self, DbError> {
                alloy_rlp::decode_exact(bytes)
                    .map_err(|e| DbError::Decode(format!("{}: {e}", stringify!($ty))))
            }
        }
    )*};
}
rlp_codec!(Header, AccountInfo, TxLocation, Vec<TxEnvelope>);

impl Encode for ReceiptEnvelope {
    fn encode(&self) -> Vec<u8> {
        self.encoded_2718()
    }
}
impl Decode for ReceiptEnvelope {
    fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        let mut buf = bytes;
        let r = ReceiptEnvelope::decode_2718(&mut buf)
            .map_err(|e| DbError::Decode(format!("ReceiptEnvelope: {e}")))?;
        if !buf.is_empty() {
            return Err(DbError::Decode("ReceiptEnvelope: trailing bytes".into()));
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::{Log, Receipt, ReceiptWithBloom};

    fn rt<T: Encode + Decode + PartialEq + core::fmt::Debug>(v: T) {
        assert_eq!(T::decode(&v.encode()).unwrap(), v);
    }

    #[test]
    fn roundtrips() {
        rt(42u64);
        rt((7u64, 9u64));
        rt(B256::repeat_byte(3));
        rt(Address::repeat_byte(4));
        rt((Address::repeat_byte(1), B256::repeat_byte(2)));
        rt((Address::repeat_byte(1), 7u64));
        rt((Address::repeat_byte(1), B256::repeat_byte(2), 7u64));
        rt(None::<AccountInfo>);
        rt(Some(AccountInfo::new(0, U256::ZERO)));
        rt(StateChangeSet::default());
        rt(StateChangeSet {
            accounts: vec![Address::repeat_byte(1), Address::repeat_byte(2)],
            slots: vec![(Address::repeat_byte(3), B256::repeat_byte(4))],
        });
        rt(U256::from(123_456u64));
        rt(String::from("latest_block"));
        rt(vec![1u8, 2, 3]);
        rt(Bytes::from_static(&[0x60, 0x00]));
        rt(AccountInfo::new(1, U256::from(2u64)));
        rt(TxLocation {
            block_number: 1,
            index: 2,
        });
        rt(Header {
            number: 3,
            ..Default::default()
        });
        rt(Vec::<TxEnvelope>::new());
        let receipt = Receipt::<Log> {
            status: true.into(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        };
        rt(ReceiptEnvelope::Eip1559(ReceiptWithBloom::from(
            receipt.clone(),
        )));
        rt(ReceiptEnvelope::Legacy(ReceiptWithBloom::from(receipt)));
    }

    #[test]
    fn integer_keys_sort_numerically() {
        assert!(255u64.encode() < 256u64.encode());
        assert!((1u64, 300u64).encode() < (2u64, 0u64).encode());
        let a = Address::repeat_byte(1);
        assert!((a, 255u64).encode() < (a, 256u64).encode());
        assert!((a, u64::MAX).encode() < (Address::repeat_byte(2), 0u64).encode());
    }

    #[test]
    fn bad_lengths_fail() {
        assert!(u64::decode(&[1, 2]).is_err());
        assert!(Address::decode(&[0; 19]).is_err());
        assert!(StateChangeSet::decode(&[0, 0]).is_err());
        assert!(StateChangeSet::decode(&[0, 0, 0, 1, 1]).is_err());
        assert!(StateChangeSet::decode(&[0, 0, 0, 0, 1]).is_err());
    }
}
