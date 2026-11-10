//! Conversions from xDEVS / alloy types to revm environments.

use crate::ExecutionError;
use alloy_rpc_types_eth::TransactionRequest;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    context_interface::{block::BlobExcessGasAndPrice, either::Either},
    primitives::{eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE, hardfork::SpecId},
};
use xdevs_primitives::{Header, RecoveredTx, Transaction, TxKind, Typed2718, U256};

/// EVM rules of xDEVS Chain v1: the latest activated Ethereum mainnet hardfork that
/// revm implements (Osaka / "Fusaka"). Blob transactions are rejected separately.
pub const SPEC: SpecId = SpecId::OSAKA;

/// EIP-4844 blob transaction type (unsupported on xDEVS Chain).
pub const BLOB_TX_TYPE: u8 = 3;

/// EVM config for `chain_id` with consensus (strict) settings.
pub(crate) fn cfg_env(chain_id: u64) -> CfgEnv {
    let mut cfg = CfgEnv::new_with_spec(SPEC);
    cfg.chain_id = chain_id;
    cfg.tx_chain_id_check = true;
    cfg
}

/// Block environment for a block with `header` (beneficiary = coinbase, basefee,
/// prevrandao = `mix_hash`). Blob gas is fixed at excess 0.
pub(crate) fn block_env(header: &Header) -> BlockEnv {
    BlockEnv {
        number: U256::from(header.number),
        beneficiary: header.beneficiary,
        timestamp: U256::from(header.timestamp),
        gas_limit: header.gas_limit,
        basefee: header.base_fee_per_gas.unwrap_or_default(),
        difficulty: U256::ZERO,
        prevrandao: Some(header.mix_hash),
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
            header.excess_blob_gas.unwrap_or_default(),
            BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        )),
        slot_num: 0,
    }
}

/// revm transaction for a signed, sender-recovered transaction.
pub(crate) fn tx_env(tx: &RecoveredTx) -> Result<TxEnv, ExecutionError> {
    let ty = tx.ty();
    if ty == BLOB_TX_TYPE {
        return Err(ExecutionError::UnsupportedTxType(ty));
    }
    let inner = &tx.tx;
    let mut env = TxEnv {
        tx_type: ty,
        caller: tx.sender,
        gas_limit: inner.gas_limit(),
        // Legacy/2930: gas price; 1559/7702: max fee per gas.
        gas_price: inner.max_fee_per_gas(),
        kind: inner.kind(),
        value: inner.value(),
        data: inner.input().clone(),
        nonce: inner.nonce(),
        chain_id: inner.chain_id(),
        access_list: inner.access_list().cloned().unwrap_or_default(),
        gas_priority_fee: inner.max_priority_fee_per_gas(),
        ..TxEnv::default()
    };
    if let Some(auths) = inner.authorization_list() {
        env.authorization_list = auths.iter().cloned().map(Either::Left).collect();
    }
    Ok(env)
}

/// revm transaction for an `eth_call` / `eth_estimateGas` request.
///
/// Missing `from` = zero address, missing `gas` = `gas_limit`, missing fee fields = 0
/// (base-fee check disabled by the caller's cfg), missing nonce = `state_nonce`.
pub(crate) fn call_tx_env(
    request: &TransactionRequest,
    chain_id: u64,
    gas_limit: u64,
    state_nonce: u64,
) -> Result<TxEnv, ExecutionError> {
    if request
        .blob_versioned_hashes
        .as_ref()
        .is_some_and(|h| !h.is_empty())
        || request.sidecar.is_some()
        || request.transaction_type == Some(BLOB_TX_TYPE)
    {
        return Err(ExecutionError::UnsupportedTxType(BLOB_TX_TYPE));
    }
    if request.gas_price.is_some()
        && (request.max_fee_per_gas.is_some() || request.max_priority_fee_per_gas.is_some())
    {
        return Err(ExecutionError::InvalidRequest(
            "both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified".into(),
        ));
    }
    let data = request
        .input
        .clone()
        .try_into_unique_input()
        .map_err(|e| ExecutionError::InvalidRequest(e.to_string()))?
        .unwrap_or_default();

    let is_1559 = request.max_fee_per_gas.is_some()
        || request.max_priority_fee_per_gas.is_some()
        || request.authorization_list.is_some();
    let (gas_price, gas_priority_fee) = if is_1559 {
        let max_fee = request.max_fee_per_gas.unwrap_or_default();
        let tip = request.max_priority_fee_per_gas.unwrap_or_default();
        // A request with only a tip behaves as if max fee = tip (like geth).
        (max_fee.max(tip), Some(tip))
    } else {
        (request.gas_price.unwrap_or_default(), None)
    };

    let mut env = TxEnv {
        tx_type: 0,
        caller: request.from.unwrap_or_default(),
        gas_limit,
        gas_price,
        kind: request.to.unwrap_or(TxKind::Create),
        value: request.value.unwrap_or_default(),
        data,
        nonce: request.nonce.unwrap_or(state_nonce),
        chain_id: Some(request.chain_id.unwrap_or(chain_id)),
        access_list: request.access_list.clone().unwrap_or_default(),
        gas_priority_fee,
        ..TxEnv::default()
    };
    if let Some(auths) = &request.authorization_list {
        env.authorization_list = auths.iter().cloned().map(Either::Left).collect();
    }
    env.derive_tx_type()
        .map_err(|e| ExecutionError::InvalidRequest(e.to_string()))?;
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::{Address, Bytes};

    #[test]
    fn call_env_defaults() {
        let req = TransactionRequest {
            to: Some(TxKind::Call(Address::repeat_byte(1))),
            ..Default::default()
        };
        let env = call_tx_env(&req, 7117, 1_000, 4).unwrap();
        assert_eq!(env.caller, Address::ZERO);
        assert_eq!(env.gas_limit, 1_000);
        assert_eq!(env.gas_price, 0);
        assert_eq!(env.nonce, 4);
        assert_eq!(env.chain_id, Some(7117));
        assert_eq!(env.tx_type, 0);
        assert_eq!(env.data, Bytes::new());
    }

    #[test]
    fn call_env_eip1559_and_conflicts() {
        let req = TransactionRequest {
            max_priority_fee_per_gas: Some(5),
            ..Default::default()
        };
        let env = call_tx_env(&req, 1, 1, 0).unwrap();
        assert_eq!(env.tx_type, 2);
        assert_eq!(env.gas_price, 5);
        assert_eq!(env.kind, TxKind::Create);

        let bad = TransactionRequest {
            gas_price: Some(1),
            max_fee_per_gas: Some(1),
            ..Default::default()
        };
        assert!(matches!(
            call_tx_env(&bad, 1, 1, 0),
            Err(ExecutionError::InvalidRequest(_))
        ));
        let blob = TransactionRequest {
            transaction_type: Some(3),
            ..Default::default()
        };
        assert_eq!(
            call_tx_env(&blob, 1, 1, 0),
            Err(ExecutionError::UnsupportedTxType(3))
        );
    }
}
