//! Conversions from xDEVS primitives to RPC shapes. **Owner: rpc agent** (bodies);
//! used by the node's [`crate::RpcBackend`] implementation.
//!
//! The output types are the `alloy-rpc-types-eth` ones, so the JSON matches geth:
//! hex quantities, `null` for pending positions, `withdrawals: []` on post-Shanghai
//! headers, `effectiveGasPrice = base fee + effective tip`.

use crate::{
    events::BlockEvent,
    types::{BlockTransactions, RpcBlock, RpcHeader, RpcLog, RpcReceipt, RpcTransaction},
};
use alloy_rpc_types_eth::Withdrawals;
use xdevs_primitives::{
    reexports::{
        alloy_consensus::transaction::{Recovered, TransactionInfo},
        alloy_rlp::{self, Encodable},
    },
    Address, Block, Header, ReceiptEnvelope, RecoveredTx, Sealed, SealedHeader, Transaction,
    TxEnvelope, B256, U256,
};

/// Position of a mined transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxMeta {
    /// Hash of the containing block.
    pub block_hash: B256,
    /// Number of the containing block.
    pub block_number: u64,
    /// Index in the block.
    pub index: u64,
    /// Base fee of the containing block (for `effectiveGasPrice`).
    pub base_fee: Option<u64>,
    /// Timestamp of the containing block.
    pub timestamp: u64,
}

/// Receipt context that is not stored in the consensus receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiptMeta {
    /// Gas used by this tx alone (`cumulative - previous cumulative`).
    pub gas_used: u64,
    /// Block-wide index of this receipt's first log.
    pub first_log_index: u64,
    /// Created contract address for create txs (`sender`, `nonce` → CREATE address).
    pub contract_address: Option<Address>,
}

/// RPC header (with hash) from a sealed header. `size` is not set (it needs the body;
/// [`rpc_block`] sets it).
pub fn rpc_header(header: &SealedHeader) -> RpcHeader {
    RpcHeader::from_sealed(header.clone())
}

/// RLP size of the full block (header, txs, no ommers, withdrawals list when the
/// header carries a withdrawals root) — the `size` field geth reports.
pub fn block_rlp_size(header: &Header, body: &[TxEnvelope]) -> usize {
    let empty_list = alloy_rlp::list_length::<Header, Header>(&[]);
    let mut payload = header.length() + alloy_rlp::list_length::<TxEnvelope, TxEnvelope>(body);
    payload += empty_list; // ommers
    if header.withdrawals_root.is_some() {
        payload += empty_list; // withdrawals (always empty on xDEVS)
    }
    payload + alloy_rlp::length_of_length(payload)
}

/// RPC block; `senders[i]` is the sender of `block.body[i]`. `full = false` lists only
/// tx hashes.
pub fn rpc_block(block: &Block, hash: B256, senders: &[Address], full: bool) -> RpcBlock {
    debug_assert!(
        !full || senders.len() == block.body.len(),
        "rpc_block: {} senders for {} txs",
        senders.len(),
        block.body.len()
    );
    let header = &block.header;
    let transactions = if full {
        BlockTransactions::Full(
            block
                .body
                .iter()
                .enumerate()
                .map(|(i, tx)| {
                    let sender = senders.get(i).copied().unwrap_or_default();
                    let meta = TxMeta {
                        block_hash: hash,
                        block_number: header.number,
                        index: i as u64,
                        base_fee: header.base_fee_per_gas,
                        timestamp: header.timestamp,
                    };
                    rpc_transaction(&RecoveredTx::new_unchecked(tx.clone(), sender), Some(meta))
                })
                .collect(),
        )
    } else {
        BlockTransactions::Hashes(block.tx_hashes().collect())
    };
    let size = block_rlp_size(header, &block.body);
    let rpc_header = RpcHeader::from_consensus(
        Sealed::new_unchecked(header.clone(), hash),
        None,
        Some(U256::from(size)),
    );
    let withdrawals = header.withdrawals_root.map(|_| Withdrawals::default());
    RpcBlock::new(rpc_header, transactions).with_withdrawals(withdrawals)
}

/// RPC transaction; `meta = None` for pending transactions (then `gasPrice` =
/// `maxFeePerGas` for EIP-1559 txs, like geth).
pub fn rpc_transaction(tx: &RecoveredTx, meta: Option<TxMeta>) -> RpcTransaction {
    let info = match meta {
        Some(m) => TransactionInfo {
            hash: Some(tx.hash()),
            index: Some(m.index),
            block_hash: Some(m.block_hash),
            block_number: Some(m.block_number),
            base_fee: m.base_fee,
            block_timestamp: Some(m.timestamp),
        },
        None => TransactionInfo {
            hash: Some(tx.hash()),
            ..Default::default()
        },
    };
    RpcTransaction::from_transaction(Recovered::new_unchecked(tx.tx.clone(), tx.sender), info)
}

/// RPC receipt for a mined transaction.
pub fn rpc_receipt(
    tx: &RecoveredTx,
    receipt: &ReceiptEnvelope,
    meta: TxMeta,
    receipt_meta: ReceiptMeta,
) -> RpcReceipt {
    let tx_hash = tx.hash();
    let mut log_index = receipt_meta.first_log_index;
    let inner = receipt.clone().map_logs(|log| {
        let rpc = rpc_log(log, tx_hash, &meta, log_index);
        log_index += 1;
        rpc
    });
    RpcReceipt {
        inner,
        transaction_hash: tx_hash,
        transaction_index: Some(meta.index),
        block_hash: Some(meta.block_hash),
        block_number: Some(meta.block_number),
        gas_used: receipt_meta.gas_used,
        effective_gas_price: tx.effective_gas_price(meta.base_fee),
        blob_gas_used: None,
        blob_gas_price: None,
        from: tx.sender,
        to: tx.to(),
        contract_address: receipt_meta.contract_address,
    }
}

/// RPC logs of one receipt (with block/tx context and block-wide log indices).
pub fn rpc_logs(
    receipt: &ReceiptEnvelope,
    tx_hash: B256,
    meta: TxMeta,
    first_log_index: u64,
) -> Vec<RpcLog> {
    receipt
        .logs()
        .iter()
        .enumerate()
        .map(|(i, log)| rpc_log(log.clone(), tx_hash, &meta, first_log_index + i as u64))
        .collect()
}

/// The [`BlockEvent`] of a committed block: its header exactly as
/// `eth_getBlockByNumber(n, false)` returns it (`hash`, `size`; no transactions) and all
/// its logs exactly as `eth_getLogs` returns them. `receipts[i]` belongs to
/// `block.body[i]`.
pub fn block_event(block: &Block, hash: B256, receipts: &[ReceiptEnvelope]) -> BlockEvent {
    let header = rpc_block(block, hash, &[], false).header;
    let mut logs = Vec::new();
    let mut log_index = 0u64;
    for (i, (tx, receipt)) in block.body.iter().zip(receipts).enumerate() {
        let meta = TxMeta {
            block_hash: hash,
            block_number: block.header.number,
            index: i as u64,
            base_fee: block.header.base_fee_per_gas,
            timestamp: block.header.timestamp,
        };
        logs.extend(rpc_logs(receipt, *tx.tx_hash(), meta, log_index));
        log_index += receipt.logs().len() as u64;
    }
    BlockEvent {
        header,
        logs,
        tx_count: block.body.len(),
    }
}

fn rpc_log(log: xdevs_primitives::Log, tx_hash: B256, meta: &TxMeta, log_index: u64) -> RpcLog {
    RpcLog {
        inner: log,
        block_hash: Some(meta.block_hash),
        block_number: Some(meta.block_number),
        block_timestamp: Some(meta.timestamp),
        transaction_hash: Some(tx_hash),
        transaction_index: Some(meta.index),
        log_index: Some(log_index),
        removed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use xdevs_primitives::{
        address, hex,
        reexports::alloy_consensus::{self, BlockBody, SignableTransaction},
        set_compat_header_fields, Bytes, Decodable2718, LogData, Receipt, ReceiptWithBloom,
        Signature, TxEip1559, TxKind,
    };

    const EIP155_RAW: &str = "f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

    fn legacy_tx() -> RecoveredTx {
        let raw = hex::decode(EIP155_RAW).unwrap();
        RecoveredTx::recover(TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap()).unwrap()
    }

    fn eip1559_tx(to: TxKind) -> RecoveredTx {
        let tx = TxEip1559 {
            chain_id: 7117,
            nonce: 3,
            gas_limit: 100_000,
            max_fee_per_gas: 3_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to,
            value: U256::from(5),
            input: Bytes::from_static(&[0xde, 0xad]),
            access_list: Default::default(),
        };
        let env = TxEnvelope::from(tx.into_signed(Signature::test_signature()));
        RecoveredTx::new_unchecked(env, address!("0x00000000000000000000000000000000000000aa"))
    }

    fn header(number: u64) -> Header {
        let mut h = Header {
            number,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            base_fee_per_gas: Some(1_500_000_000),
            ..Default::default()
        };
        set_compat_header_fields(&mut h);
        h
    }

    fn meta() -> TxMeta {
        TxMeta {
            block_hash: B256::repeat_byte(0xbb),
            block_number: 42,
            index: 1,
            base_fee: Some(1_500_000_000),
            timestamp: 1_700_000_000,
        }
    }

    fn receipt_with_logs(n: usize) -> ReceiptEnvelope {
        let logs = (0..n)
            .map(|i| xdevs_primitives::Log {
                address: address!("0x00000000000000000000000000000000000000cc"),
                data: LogData::new_unchecked(vec![B256::repeat_byte(i as u8)], Bytes::new()),
            })
            .collect();
        let receipt = Receipt {
            status: true.into(),
            cumulative_gas_used: 50_000,
            logs,
        };
        ReceiptEnvelope::Eip1559(ReceiptWithBloom::from(receipt))
    }

    #[test]
    fn block_size_matches_alloy_encoding() {
        let h = header(1);
        let body = vec![
            legacy_tx().tx.clone(),
            eip1559_tx(TxKind::Create).tx.clone(),
        ];
        let expected = alloy_consensus::Block::<TxEnvelope>::rlp_length_for(
            &h,
            &BlockBody {
                transactions: body.clone(),
                ommers: vec![],
                withdrawals: Some(Withdrawals::default()),
            },
        );
        assert_eq!(block_rlp_size(&h, &body), expected);
        assert_eq!(
            block_rlp_size(&h, &body),
            alloy_rlp::encode(alloy_consensus::Block {
                header: h.clone(),
                body: BlockBody {
                    transactions: body.clone(),
                    ommers: vec![],
                    withdrawals: Some(Withdrawals::default())
                },
            })
            .len()
        );
    }

    #[test]
    fn block_hashes_and_full_shapes() {
        let t0 = legacy_tx();
        let t1 = eip1559_tx(TxKind::Call(address!(
            "0x00000000000000000000000000000000000000dd"
        )));
        let block = Block::new(header(7), vec![t0.tx.clone(), t1.tx.clone()]);
        let hash = block.hash();
        let senders = [t0.sender, t1.sender];

        let light = rpc_block(&block, hash, &senders, false);
        let json = serde_json::to_value(&light).unwrap();
        assert_eq!(json["hash"], Value::String(format!("{hash}")));
        assert_eq!(json["number"], "0x7");
        assert_eq!(json["baseFeePerGas"], "0x59682f00");
        assert_eq!(json["difficulty"], "0x0");
        assert_eq!(json["nonce"], "0x0000000000000000");
        assert_eq!(json["withdrawals"], serde_json::json!([]));
        assert_eq!(json["uncles"], serde_json::json!([]));
        assert!(json["size"].as_str().unwrap().starts_with("0x"));
        assert_eq!(
            json["transactions"],
            serde_json::json!([format!("{}", t0.hash()), format!("{}", t1.hash())])
        );
        // Round-trips into the alloy type.
        let back: RpcBlock = serde_json::from_value(json).unwrap();
        assert_eq!(back.header.hash, hash);

        let full = rpc_block(&block, hash, &senders, true);
        let json = serde_json::to_value(&full).unwrap();
        let txs = json["transactions"].as_array().unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[1]["blockHash"], Value::String(format!("{hash}")));
        assert_eq!(txs[1]["transactionIndex"], "0x1");
        assert_eq!(txs[1]["from"], "0x00000000000000000000000000000000000000aa");
        assert_eq!(txs[1]["type"], "0x2");
        let back: RpcBlock = serde_json::from_value(json).unwrap();
        assert_eq!(back.transactions.len(), 2);
    }

    #[test]
    fn mined_and_pending_transaction() {
        let tx = eip1559_tx(TxKind::Create);
        let mined = rpc_transaction(&tx, Some(meta()));
        let json = serde_json::to_value(&mined).unwrap();
        assert_eq!(json["hash"], Value::String(format!("{}", tx.hash())));
        assert_eq!(json["blockNumber"], "0x2a");
        assert_eq!(json["nonce"], "0x3");
        assert_eq!(json["chainId"], "0x1bcd");
        // base 1.5 gwei + min(tip 1 gwei, max 3 - base 1.5) = 2.5 gwei
        assert_eq!(json["gasPrice"], "0x9502f900");
        assert_eq!(json["to"], Value::Null);
        let _: RpcTransaction = serde_json::from_value(json).unwrap();

        let pending = rpc_transaction(&tx, None);
        let json = serde_json::to_value(&pending).unwrap();
        assert_eq!(json["blockHash"], Value::Null);
        assert_eq!(json["blockNumber"], Value::Null);
        assert_eq!(json["transactionIndex"], Value::Null);
        assert_eq!(json["gasPrice"], "0xb2d05e00"); // maxFeePerGas
        let _: RpcTransaction = serde_json::from_value(json).unwrap();

        let legacy = rpc_transaction(&legacy_tx(), Some(meta()));
        let json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(json["type"], "0x0");
        assert_eq!(json["gasPrice"], "0x4a817c800");
        assert_eq!(json["v"], "0x25");
    }

    #[test]
    fn receipt_and_logs() {
        let tx = eip1559_tx(TxKind::Create);
        let receipt = receipt_with_logs(2);
        let created = address!("0x00000000000000000000000000000000000000ee");
        let r = rpc_receipt(
            &tx,
            &receipt,
            meta(),
            ReceiptMeta {
                gas_used: 29_000,
                first_log_index: 5,
                contract_address: Some(created),
            },
        );
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["status"], "0x1");
        assert_eq!(json["type"], "0x2");
        assert_eq!(json["gasUsed"], "0x7148");
        assert_eq!(json["cumulativeGasUsed"], "0xc350");
        assert_eq!(json["effectiveGasPrice"], "0x9502f900");
        assert_eq!(
            json["contractAddress"],
            serde_json::to_value(created).unwrap()
        );
        assert_eq!(json["to"], Value::Null);
        assert_eq!(json["from"], "0x00000000000000000000000000000000000000aa");
        let logs = json["logs"].as_array().unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0]["logIndex"], "0x5");
        assert_eq!(logs[1]["logIndex"], "0x6");
        assert_eq!(logs[1]["transactionIndex"], "0x1");
        assert_eq!(logs[1]["removed"], false);
        assert!(json["logsBloom"].as_str().unwrap().len() == 2 + 512);
        let back: RpcReceipt = serde_json::from_value(json).unwrap();
        assert_eq!(back.transaction_hash, tx.hash());

        let logs = rpc_logs(&receipt, tx.hash(), meta(), 5);
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[1].log_index, Some(6));
        assert_eq!(logs[0].transaction_hash, Some(tx.hash()));
        assert_eq!(logs[0].block_number, Some(42));
    }

    #[test]
    fn block_event_matches_block_and_get_logs_shapes() {
        let t0 = legacy_tx();
        let t1 = eip1559_tx(TxKind::Create);
        let block = Block::new(header(9), vec![t0.tx.clone(), t1.tx.clone()]);
        let hash = block.hash();
        let receipts = [receipt_with_logs(2), receipt_with_logs(1)];
        let ev = block_event(&block, hash, &receipts);
        assert_eq!(ev.header, rpc_block(&block, hash, &[], false).header);
        assert_eq!(ev.header.hash, hash);
        assert!(ev.header.size.is_some());
        assert_eq!(ev.logs.len(), 3);
        let at = |i: u64| TxMeta {
            block_hash: hash,
            block_number: 9,
            index: i,
            base_fee: block.header.base_fee_per_gas,
            timestamp: block.header.timestamp,
        };
        let mut expected = rpc_logs(&receipts[0], t0.hash(), at(0), 0);
        expected.extend(rpc_logs(&receipts[1], t1.hash(), at(1), 2));
        assert_eq!(ev.logs, expected);
        assert_eq!(ev.logs[2].log_index, Some(2));
        assert_eq!(ev.logs[2].transaction_index, Some(1));
    }

    #[test]
    fn header_conversion_keeps_hash() {
        let h = header(3);
        let sealed = Sealed::new(h.clone());
        let rpc = rpc_header(&sealed);
        assert_eq!(rpc.hash, h.hash_slow());
        assert_eq!(rpc.inner, h);
    }
}
