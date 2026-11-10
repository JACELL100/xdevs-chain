//! Storage errors.

/// Errors returned by [`crate::Database`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Backend (redb / IO) failure.
    #[error("storage backend error: {0}")]
    Backend(String),
    /// A stored key or value could not be decoded (corruption or schema mismatch).
    #[error("decode error: {0}")]
    Decode(String),
    /// The database was written by an incompatible schema version.
    #[error("schema version mismatch: on disk {found}, expected {expected}")]
    SchemaMismatch {
        /// Version found on disk.
        found: u64,
        /// Version this binary understands.
        expected: u64,
    },
}
