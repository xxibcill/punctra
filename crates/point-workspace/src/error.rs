use std::{fmt, io};

use foundation_runtime::RuntimeError;
use point_contracts::ContractError;
use point_index::IndexError;
use point_source::SourceError;
use thiserror::Error;

use crate::{OperationId, RevisionId};

/// Maximum UTF-8 bytes retained in one Workspace-owned diagnostic.
pub const MAX_WORKSPACE_DIAGNOSTIC_BYTES: usize = 1_024;

const ELLIPSIS: &str = "...";

/// Bounded text carried by Workspace failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceDiagnostic(String);

impl WorkspaceDiagnostic {
    /// Retains a bounded, valid UTF-8 prefix of a diagnostic.
    #[must_use]
    pub fn new(message: impl AsRef<str>) -> Self {
        let message = message.as_ref();
        if message.len() <= MAX_WORKSPACE_DIAGNOSTIC_BYTES {
            return Self(message.to_owned());
        }

        let mut end = MAX_WORKSPACE_DIAGNOSTIC_BYTES - ELLIPSIS.len();
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        Self(format!("{}{}", &message[..end], ELLIPSIS))
    }

    /// Returns the bounded diagnostic text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for WorkspaceDiagnostic {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for WorkspaceDiagnostic {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// Failure while creating, opening, selecting from, or editing a Workspace.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WorkspaceError {
    /// Caller input violates a Workspace interface contract.
    #[error("invalid {argument}: {reason}")]
    InvalidArgument {
        /// Stable argument category.
        argument: &'static str,
        /// Bounded failure explanation.
        reason: WorkspaceDiagnostic,
    },

    /// Work or retained output exceeded a caller-selected hard limit.
    #[error("Workspace exceeded {limit}: required {required}, limit {allowed}")]
    ResourceLimit {
        /// Stable resource category.
        limit: &'static str,
        /// Minimum resource amount required.
        required: u64,
        /// Caller-selected ceiling.
        allowed: u64,
    },

    /// Persisted Workspace bytes failed structural or checksum validation.
    #[error("corrupt Workspace: {reason}")]
    Corrupt {
        /// Bounded corruption category.
        reason: WorkspaceDiagnostic,
    },

    /// Persisted facts do not match the supplied Source or implementation.
    #[error("incompatible Workspace: {reason}")]
    Incompatible {
        /// Bounded mismatch category.
        reason: WorkspaceDiagnostic,
    },

    /// Another process holds the Workspace's exclusive file lock.
    #[error("Workspace is locked by another process")]
    Locked,

    /// A prior uncertain commit requires the Workspace to be reopened.
    #[error("Workspace commit state is uncertain; reopen before continuing")]
    Poisoned,

    /// Open could not durably reconcile a visible post-crash operation state.
    #[error("Workspace recovery is indeterminate: {reason}")]
    RecoveryIndeterminate {
        /// Operation being reconciled when its identity is known.
        operation: Option<OperationId>,
        /// Bounded durability diagnostic.
        reason: WorkspaceDiagnostic,
    },

    /// A requested immutable Revision does not exist.
    #[error("unknown Workspace Revision {revision}")]
    UnknownRevision {
        /// Missing Revision identity.
        revision: RevisionId,
    },

    /// A process-scoped Point Set cannot be used by this operation.
    #[error("invalid Point Set: {reason}")]
    InvalidPointSet {
        /// Bounded provenance or storage mismatch.
        reason: WorkspaceDiagnostic,
    },

    /// An Operation Identity was reused for different canonical intent.
    #[error("Operation {operation} is already bound to another request")]
    OperationConflict {
        /// Conflicting caller-owned identity.
        operation: OperationId,
    },

    /// An operation has no durable intent that can be resumed.
    #[error("Operation {operation} has no retryable recorded intent")]
    OperationNotRetryable {
        /// Operation requested for resume.
        operation: OperationId,
    },

    /// Cryptographic system randomness was unavailable.
    #[error("failed to generate an opaque identity: {reason}")]
    RandomnessUnavailable {
        /// Bounded entropy-provider diagnostic.
        reason: WorkspaceDiagnostic,
    },

    /// A point-contract operation failed; its displayed diagnostic is bounded.
    #[error("point contract failed: {diagnostic}")]
    Contract {
        /// Bounded rendering of the underlying failure.
        diagnostic: WorkspaceDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: ContractError,
    },

    /// A complete-index operation failed; its displayed diagnostic is bounded.
    #[error("Spatial Index operation failed: {diagnostic}")]
    Index {
        /// Bounded rendering of the underlying failure.
        diagnostic: WorkspaceDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: IndexError,
    },

    /// A verified-Source operation failed; its displayed diagnostic is bounded.
    #[error("Source operation failed: {diagnostic}")]
    Source {
        /// Bounded rendering of the underlying failure.
        diagnostic: WorkspaceDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: SourceError,
    },

    /// A Workspace-owned filesystem operation failed.
    #[error("failed to {operation} {path}: {source}")]
    Io {
        /// Stable operation description.
        operation: &'static str,
        /// Bounded path diagnostic rather than an unbounded path value.
        path: WorkspaceDiagnostic,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },

    /// Cooperative cancellation was requested before an ambiguous commit point.
    #[error("Workspace operation was cancelled")]
    Cancelled,

    /// Runtime-neutral worker or progress handling failed.
    #[error("Workspace runtime failed: {diagnostic}")]
    Runtime {
        /// Bounded rendering of the runtime failure.
        diagnostic: WorkspaceDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: RuntimeError,
    },
}

impl WorkspaceError {
    pub(crate) fn invalid(argument: &'static str, reason: impl AsRef<str>) -> Self {
        Self::InvalidArgument {
            argument,
            reason: WorkspaceDiagnostic::new(reason),
        }
    }

    pub(crate) fn corrupt(reason: impl AsRef<str>) -> Self {
        Self::Corrupt {
            reason: WorkspaceDiagnostic::new(reason),
        }
    }

    pub(crate) fn incompatible(reason: impl AsRef<str>) -> Self {
        Self::Incompatible {
            reason: WorkspaceDiagnostic::new(reason),
        }
    }

    pub(crate) fn invalid_point_set(reason: impl AsRef<str>) -> Self {
        Self::InvalidPointSet {
            reason: WorkspaceDiagnostic::new(reason),
        }
    }

    pub(crate) fn random(source: impl fmt::Display) -> Self {
        Self::RandomnessUnavailable {
            reason: WorkspaceDiagnostic::new(source.to_string()),
        }
    }

    pub(crate) fn io(operation: &'static str, path: impl fmt::Display, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: WorkspaceDiagnostic::new(path.to_string()),
            source,
        }
    }
}

impl From<ContractError> for WorkspaceError {
    fn from(source: ContractError) -> Self {
        Self::Contract {
            diagnostic: WorkspaceDiagnostic::new(source.to_string()),
            source,
        }
    }
}

impl From<IndexError> for WorkspaceError {
    fn from(source: IndexError) -> Self {
        Self::Index {
            diagnostic: WorkspaceDiagnostic::new(source.to_string()),
            source,
        }
    }
}

impl From<SourceError> for WorkspaceError {
    fn from(source: SourceError) -> Self {
        match source {
            SourceError::Cancelled => Self::Cancelled,
            source => Self::Source {
                diagnostic: WorkspaceDiagnostic::new(source.to_string()),
                source,
            },
        }
    }
}

impl From<RuntimeError> for WorkspaceError {
    fn from(source: RuntimeError) -> Self {
        match source {
            RuntimeError::Cancelled => Self::Cancelled,
            source => Self::Runtime {
                diagnostic: WorkspaceDiagnostic::new(source.to_string()),
                source,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WORKSPACE_DIAGNOSTIC_BYTES, WorkspaceDiagnostic};

    #[test]
    fn diagnostics_are_utf8_safe_and_bounded() {
        let message = "ก".repeat(MAX_WORKSPACE_DIAGNOSTIC_BYTES);
        let diagnostic = WorkspaceDiagnostic::new(message);
        assert!(diagnostic.as_str().len() <= MAX_WORKSPACE_DIAGNOSTIC_BYTES);
        assert!(diagnostic.as_str().ends_with("..."));
    }
}
