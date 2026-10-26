//! Shared ids, errors and config for Atlas.
use serde::{Deserialize, Serialize};
use std::time::Duration;

mod auth;
pub use auth::{
    clip_echo, clip_text, AuthContext, Authenticator, EffectiveScope, KeyScope, KeyVerifier,
    PermSet, Principal, TenantId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VectorId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Cosine,
    Dot,
    Euclidean,
}

/// When an insert may be acknowledged.
///
/// Both modes ack only after `File::sync_all` returned for the record.
/// Covered: process crash and OS-level flush via sync_all.
/// KNOWN RISK (not yet proven): power-loss behaviour for directory metadata
/// (the WAL file's directory entry is not fsynced; no power-cut test exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Concurrent writers share one fsync; every ack waits for that flush. DEFAULT.
    GroupCommit,
    /// One fsync per commit (batch size forced to 1).
    SyncPerCommit,
}

/// How `open` treats a WAL that was left in a not-provably-valid state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecoverMode {
    /// Refuse to open (RECOVERY_REQUIRED) if a recovery marker exists.
    #[default]
    Normal,
    /// Opt-in repair: discard the failed (never acknowledged) batch recorded in the
    /// recovery marker, then open normally. Does not repair mid-log corruption.
    Repair,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub recover_mode: RecoverMode,
    /// Required when creating a database; optional (recovered from metadata) on reopen.
    pub dim: Option<usize>,
    pub durability: Durability,
    /// GroupCommit: how long the committer waits for more writers after the first.
    pub batch_window: Duration,
    /// GroupCommit: flush early once this many records are queued.
    pub max_batch: usize,
    /// Upper bound on one WAL record payload.
    pub max_record_len: u32,
}

impl Config {
    /// Config for reopening an existing database; the dimension is read from metadata.
    pub fn reopen() -> Self {
        Self {
            dim: None,
            ..Self::new(0)
        }
    }

    /// Config for creating (or reopening and asserting) a database of dimension `dim`.
    pub fn new(dim: usize) -> Self {
        Self {
            recover_mode: RecoverMode::Normal,
            dim: Some(dim),
            durability: Durability::GroupCommit,
            batch_window: Duration::from_millis(2),
            max_batch: 128,
            max_record_len: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AtlasError {
    #[error("INVALID_VECTOR_DIMENSION: expected {expected}, got {got}")]
    InvalidVectorDimension { expected: usize, got: usize },
    #[error("INVALID_VECTOR_VALUE: non-finite component")]
    InvalidVectorValue,
    #[error("RECORD_TOO_LARGE: {len} > {max}")]
    RecordTooLarge { len: usize, max: u32 },
    #[error("WAL_FLUSH_FAILED: {0}")]
    WalFlushFailed(String),
    #[error("WAL_CORRUPT: {0}")]
    WalCorrupt(String),
    #[error("UNSUPPORTED_WAL_VERSION: file has version {found}, this build supports {supported}")]
    UnsupportedWalVersion { found: u32, supported: u32 },
    #[error("RECOVERY_REQUIRED: {0}")]
    RecoveryRequired(String),
    #[error("OUTCOME_UNKNOWN: {0}")]
    OutcomeUnknown(String),
    #[error("OP_ID_CONFLICT: operation id {0} was already used with a different payload")]
    OpIdConflict(String),
    #[error("INVALID_OP_ID: operation id must be 1..=128 bytes")]
    InvalidOpId,
    #[error("WAL_FAILED: write path failed and needs recovery; reopen the database")]
    WalFailed,
    #[error("DIMENSION_MISMATCH: database has dimension {stored}, requested {requested}")]
    DimensionMismatch { stored: usize, requested: usize },
    #[error("INVALID_CONFIG: {0}")]
    InvalidConfig(String),
    #[error("META_CORRUPT: {0}")]
    MetaCorrupt(String),
    #[error("WAL_CLOSED")]
    WalClosed,
    #[error("CODEC: {0}")]
    Codec(String),
    #[error("INVALID_TENANT: {0}")]
    InvalidTenant(String),
    #[error("FORBIDDEN: missing permission")]
    Forbidden,
    #[error("NOT_FOUND")]
    NotFound,
    #[error("INCOMPLETE: stream ended early after {emitted} records, output is not a complete result: {reason}")]
    Incomplete { emitted: u64, reason: String },
    #[error("INVALID_METADATA: {0}")]
    InvalidMetadata(String),
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

impl AtlasError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidVectorDimension { .. } => "INVALID_VECTOR_DIMENSION",
            Self::InvalidVectorValue => "INVALID_VECTOR_VALUE",
            Self::RecordTooLarge { .. } => "RECORD_TOO_LARGE",
            Self::WalFlushFailed(_) => "WAL_FLUSH_FAILED",
            Self::WalCorrupt(_) => "WAL_CORRUPT",
            Self::UnsupportedWalVersion { .. } => "UNSUPPORTED_WAL_VERSION",
            Self::RecoveryRequired(_) => "RECOVERY_REQUIRED",
            Self::OutcomeUnknown(_) => "OUTCOME_UNKNOWN",
            Self::OpIdConflict(_) => "OP_ID_CONFLICT",
            Self::InvalidOpId => "INVALID_OP_ID",
            Self::WalFailed => "WAL_FAILED",
            Self::DimensionMismatch { .. } => "DIMENSION_MISMATCH",
            Self::InvalidConfig(_) => "INVALID_CONFIG",
            Self::MetaCorrupt(_) => "META_CORRUPT",
            Self::WalClosed => "WAL_CLOSED",
            Self::Codec(_) => "CODEC",
            Self::InvalidTenant(_) => "INVALID_TENANT",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::Incomplete { .. } => "INCOMPLETE",
            Self::InvalidMetadata(_) => "INVALID_METADATA",
            Self::Io(_) => "IO",
        }
    }
}

pub fn validate_vector(dim: usize, v: &[f32]) -> Result<(), AtlasError> {
    if v.len() != dim {
        return Err(AtlasError::InvalidVectorDimension {
            expected: dim,
            got: v.len(),
        });
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err(AtlasError::InvalidVectorValue);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dim_and_finiteness() {
        assert_eq!(
            validate_vector(3, &[1.0]).unwrap_err().code(),
            "INVALID_VECTOR_DIMENSION"
        );
        assert_eq!(
            validate_vector(1, &[f32::NAN]).unwrap_err().code(),
            "INVALID_VECTOR_VALUE"
        );
        assert!(validate_vector(2, &[1.0, 2.0]).is_ok());
        assert_eq!(Config::new(2).durability, Durability::GroupCommit);
        assert_eq!(Config::reopen().dim, None);
        for e in [
            AtlasError::WalClosed,
            AtlasError::WalFailed,
            AtlasError::OutcomeUnknown("x".into()),
            AtlasError::OpIdConflict("x".into()),
            AtlasError::InvalidOpId,
            AtlasError::UnsupportedWalVersion {
                found: 2,
                supported: 1,
            },
            AtlasError::RecoveryRequired("x".into()),
            AtlasError::DimensionMismatch {
                stored: 1,
                requested: 2,
            },
            AtlasError::InvalidConfig("x".into()),
            AtlasError::MetaCorrupt("x".into()),
            AtlasError::WalCorrupt("x".into()),
            AtlasError::WalFlushFailed("x".into()),
            AtlasError::Codec("x".into()),
            AtlasError::RecordTooLarge { len: 2, max: 1 },
            AtlasError::InvalidTenant("x".into()),
            AtlasError::Forbidden,
            AtlasError::NotFound,
            AtlasError::Incomplete {
                emitted: 1,
                reason: "x".into(),
            },
            AtlasError::InvalidMetadata("x".into()),
            AtlasError::Io(std::io::Error::other("x")),
        ] {
            assert!(!e.code().is_empty());
            assert!(!e.to_string().is_empty());
        }
    }
}
