use std::{io, path::PathBuf};

use foundation_runtime::RuntimeError;
use point_source::SourceError;
use thiserror::Error;

/// Failure while preparing, opening, querying, or reading an index.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IndexError {
    /// A caller attempted to construct the reserved zero node identity.
    #[error("index node identities must be nonzero")]
    ZeroNodeId,

    /// A required hard limit was zero.
    #[error("{limit} must be greater than zero")]
    InvalidLimit {
        /// Invalid limit name.
        limit: &'static str,
    },

    /// Work or retained output exceeded one caller-selected hard limit.
    #[error("index exceeded {limit}: required {required}, limit {allowed}")]
    ResourceLimit {
        /// Exceeded resource name.
        limit: &'static str,
        /// Minimum resource amount required.
        required: u64,
        /// Caller-selected maximum.
        allowed: u64,
    },

    /// An existing complete target belongs to another Source or recipe.
    #[error("incompatible complete index: {reason}")]
    IncompatibleArtifact {
        /// Bounded static mismatch category.
        reason: &'static str,
    },

    /// An existing incomplete target belongs to another Source or recipe.
    #[error("incompatible incomplete index: {reason}")]
    IncompatibleWork {
        /// Bounded static mismatch category.
        reason: &'static str,
    },

    /// A complete target failed structural or checksum validation.
    #[error("corrupt complete index: {reason}")]
    CorruptArtifact {
        /// Bounded static corruption category.
        reason: &'static str,
    },

    /// An incomplete target header failed structural or checksum validation.
    #[error("corrupt incomplete index: {reason}")]
    CorruptWork {
        /// Bounded static corruption category.
        reason: &'static str,
    },

    /// A persisted schema or recipe version is not supported.
    #[error("unsupported {kind} version {version}")]
    UnsupportedVersion {
        /// Versioned contract name.
        kind: &'static str,
        /// Unsupported version value.
        version: u32,
    },

    /// A requested hierarchy identity does not exist.
    #[error("unknown index node {node}")]
    UnknownNode {
        /// Requested nonzero identity value.
        node: u64,
    },

    /// A verified Source operation failed without losing its category.
    #[error(transparent)]
    Source(#[from] SourceError),

    /// A filesystem operation failed.
    #[error("failed to {operation} {path}: {source}")]
    Io {
        /// Stable operation description.
        operation: &'static str,
        /// Exact path operated on.
        path: PathBuf,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },

    /// Runtime-neutral cancellation or worker failure.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

impl IndexError {
    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}
