use std::{fmt, io};

use foundation_runtime::RuntimeError;
use point_contracts::{ContentHash, ContractError, PointId};
use point_workspace::WorkspaceError;
use thiserror::Error;

/// Maximum UTF-8 bytes retained in a terrain-owned diagnostic.
pub const MAX_TERRAIN_DIAGNOSTIC_BYTES: usize = 1_024;

const ELLIPSIS: &str = "...";

/// Bounded text carried by terrain failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainDiagnostic(String);

impl TerrainDiagnostic {
    /// Retains a bounded, valid UTF-8 prefix of a diagnostic.
    #[must_use]
    pub fn new(message: impl AsRef<str>) -> Self {
        let message = message.as_ref();
        if message.len() <= MAX_TERRAIN_DIAGNOSTIC_BYTES {
            return Self(message.to_owned());
        }

        let mut end = MAX_TERRAIN_DIAGNOSTIC_BYTES - ELLIPSIS.len();
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

impl fmt::Display for TerrainDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Failure while deriving, evaluating, or exporting a Terrain Surface.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TerrainError {
    /// Caller input violates a terrain interface contract.
    #[error("invalid {argument}: {reason}")]
    InvalidArgument {
        /// Stable argument category.
        argument: &'static str,
        /// Bounded failure explanation.
        reason: TerrainDiagnostic,
    },

    /// Work or retained output exceeded a caller-selected hard limit.
    #[error("terrain exceeded {limit}: required {required}, limit {allowed}")]
    ResourceLimit {
        /// Stable resource category.
        limit: &'static str,
        /// Minimum amount required.
        required: u64,
        /// Caller-selected ceiling.
        allowed: u64,
    },

    /// Fewer than three Ground Input Points were available.
    #[error("terrain requires at least three Ground Input Points; found {actual}")]
    InsufficientGroundInput {
        /// Exact available input count.
        actual: u64,
    },

    /// Two Ground Input Points occupy the same horizontal position.
    #[error("Ground Input Points {first:?} and {second:?} share one XY position")]
    DuplicateHorizontalPosition {
        /// First canonical Point Identity.
        first: PointId,
        /// Second canonical Point Identity.
        second: PointId,
        /// Whether their elevation ticks differ.
        conflicting_elevation: bool,
    },

    /// Every Ground Input Point is collinear in XY.
    #[error("Ground Input is collinear in XY")]
    CollinearGroundInput,

    /// Exact ticks cannot be represented safely by the supported terrain kernel.
    #[error("unsupported terrain numeric range: {reason}")]
    UnsupportedNumericRange {
        /// Bounded numeric failure explanation.
        reason: TerrainDiagnostic,
    },

    /// Completed topology failed an internal deterministic invariant.
    #[error("invalid terrain topology: {reason}")]
    TopologyInvariant {
        /// Bounded invariant explanation.
        reason: TerrainDiagnostic,
    },

    /// A Workspace Point-row operation failed.
    #[error("Workspace Point-row operation failed: {diagnostic}")]
    Workspace {
        /// Bounded rendering of the underlying failure.
        diagnostic: TerrainDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: Box<WorkspaceError>,
    },

    /// A point-contract operation failed.
    #[error("point contract failed: {diagnostic}")]
    Contract {
        /// Bounded rendering of the underlying failure.
        diagnostic: TerrainDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: ContractError,
    },

    /// An export target already exists and was not replaced.
    #[error("terrain export target already exists: {path}")]
    TargetExists {
        /// Bounded target path.
        path: TerrainDiagnostic,
    },

    /// A terrain-owned filesystem operation failed.
    #[error("failed to {operation} {path}: {source}")]
    Io {
        /// Stable operation description.
        operation: &'static str,
        /// Bounded path diagnostic.
        path: TerrainDiagnostic,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },

    /// Export publication may have created the complete target.
    #[error("LandXML publication is indeterminate for expected hash {expected_hash}")]
    ExportIndeterminate {
        /// Expected complete output content hash.
        expected_hash: ContentHash,
    },

    /// Cooperative cancellation was requested before result publication.
    #[error("terrain operation was cancelled")]
    Cancelled,

    /// Runtime-neutral worker or progress handling failed.
    #[error("terrain runtime failed: {diagnostic}")]
    Runtime {
        /// Bounded rendering of the runtime failure.
        diagnostic: TerrainDiagnostic,
        /// Structured underlying failure.
        #[source]
        source: RuntimeError,
    },
}

impl TerrainError {
    pub(crate) fn invalid(argument: &'static str, reason: impl AsRef<str>) -> Self {
        Self::InvalidArgument {
            argument,
            reason: TerrainDiagnostic::new(reason),
        }
    }

    pub(crate) fn resource(limit: &'static str, required: u64, allowed: u64) -> Self {
        Self::ResourceLimit {
            limit,
            required,
            allowed,
        }
    }

    pub(crate) fn numeric(reason: impl AsRef<str>) -> Self {
        Self::UnsupportedNumericRange {
            reason: TerrainDiagnostic::new(reason),
        }
    }

    pub(crate) fn topology(reason: impl AsRef<str>) -> Self {
        Self::TopologyInvariant {
            reason: TerrainDiagnostic::new(reason),
        }
    }

    pub(crate) fn io(operation: &'static str, path: impl fmt::Display, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: TerrainDiagnostic::new(path.to_string()),
            source,
        }
    }
}

impl From<WorkspaceError> for TerrainError {
    fn from(source: WorkspaceError) -> Self {
        match source {
            WorkspaceError::Cancelled => Self::Cancelled,
            source => Self::Workspace {
                diagnostic: TerrainDiagnostic::new(source.to_string()),
                source: Box::new(source),
            },
        }
    }
}

impl From<ContractError> for TerrainError {
    fn from(source: ContractError) -> Self {
        Self::Contract {
            diagnostic: TerrainDiagnostic::new(source.to_string()),
            source,
        }
    }
}

impl From<RuntimeError> for TerrainError {
    fn from(source: RuntimeError) -> Self {
        match source {
            RuntimeError::Cancelled => Self::Cancelled,
            source => Self::Runtime {
                diagnostic: TerrainDiagnostic::new(source.to_string()),
                source,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TERRAIN_DIAGNOSTIC_BYTES, TerrainDiagnostic};

    #[test]
    fn diagnostics_are_utf8_safe_and_bounded() {
        let diagnostic = TerrainDiagnostic::new("ก".repeat(MAX_TERRAIN_DIAGNOSTIC_BYTES));
        assert!(diagnostic.as_str().len() <= MAX_TERRAIN_DIAGNOSTIC_BYTES);
        assert!(diagnostic.as_str().ends_with("..."));
    }
}
