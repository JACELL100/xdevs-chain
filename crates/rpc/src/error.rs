//! Backend errors and their JSON-RPC error codes (geth-compatible where tools care).

use jsonrpsee::types::ErrorObjectOwned;
use xdevs_primitives::{hex, Bytes};

/// Errors an [`crate::RpcBackend`] returns.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RpcBackendError {
    /// Bad parameters (code -32602).
    #[error("invalid params: {0}")]
    InvalidParams(String),
    /// Requested block / state does not exist (code -32000, "header not found").
    #[error("header not found")]
    UnknownBlock,
    /// `eth_call` / `eth_estimateGas` reverted (code 3, data = revert bytes).
    #[error("execution reverted{}", reason.as_ref().map(|r| format!(": {r}")).unwrap_or_default())]
    Reverted {
        /// Raw revert data.
        data: Bytes,
        /// Decoded `Error(string)` reason, if any.
        reason: Option<String>,
    },
    /// Transaction rejected by the pool or execution failed (code -32000, geth wording,
    /// e.g. "nonce too low", "insufficient funds for gas * price + value").
    #[error("{0}")]
    Rejected(String),
    /// Request exceeds a server limit (code -32005), e.g. log range too large.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// Method or feature not supported (code -32601).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Internal failure (code -32603).
    #[error("internal error: {0}")]
    Internal(String),
}

impl RpcBackendError {
    /// JSON-RPC error code.
    pub const fn code(&self) -> i32 {
        match self {
            RpcBackendError::InvalidParams(_) => -32602,
            RpcBackendError::UnknownBlock | RpcBackendError::Rejected(_) => -32000,
            RpcBackendError::Reverted { .. } => 3,
            RpcBackendError::LimitExceeded(_) => -32005,
            RpcBackendError::Unsupported(_) => -32601,
            RpcBackendError::Internal(_) => -32603,
        }
    }
}

impl From<RpcBackendError> for ErrorObjectOwned {
    fn from(e: RpcBackendError) -> Self {
        let code = e.code();
        let msg = e.to_string();
        match e {
            RpcBackendError::Reverted { data, .. } => {
                ErrorObjectOwned::owned(code, msg, Some(hex::encode_prefixed(&data)))
            }
            _ => ErrorObjectOwned::owned::<()>(code, msg, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_messages() {
        let e = RpcBackendError::Reverted {
            data: Bytes::from_static(&[1, 2]),
            reason: Some("nope".into()),
        };
        assert_eq!(e.code(), 3);
        assert_eq!(e.to_string(), "execution reverted: nope");
        let obj: ErrorObjectOwned = e.into();
        assert_eq!(obj.code(), 3);
        assert_eq!(obj.data().map(|d| d.get()), Some("\"0x0102\""));

        let e = RpcBackendError::Rejected("nonce too low".into());
        assert_eq!(e.code(), -32000);
        assert_eq!(e.to_string(), "nonce too low");
        assert_eq!(
            RpcBackendError::UnknownBlock.to_string(),
            "header not found"
        );
    }
}
