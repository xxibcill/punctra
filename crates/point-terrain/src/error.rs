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

    /// The Source coordinate contract cannot establish the supported Terrain profile.
    #[error("unsupported Terrain spatial reference: {reason}")]
    UnsupportedSpatialReference {
        /// Bounded coordinate-assumption failure explanation.
        reason: TerrainDiagnostic,
    },

    /// The Surface coordinate contract does not support metric-metre export.
    #[error("unsupported metric-metre LandXML export: {reason}")]
    UnsupportedMetricExport {
        /// Bounded coordinate-assumption failure explanation.
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

    /// A durable Surface file belongs to different immutable terrain input.
    #[error("stale {kind}: {binding} does not match at {path}")]
    StaleSurfaceArtifact {
        /// Stable complete-artifact or work-checkpoint category.
        kind: &'static str,
        /// Stable mismatched binding category.
        binding: &'static str,
        /// Bounded artifact path.
        path: TerrainDiagnostic,
    },

    /// A durable Surface file fails structural or checksum validation.
    #[error("corrupt {kind} at {path}: {reason}")]
    CorruptSurfaceArtifact {
        /// Stable complete-artifact or work-checkpoint category.
        kind: &'static str,
        /// Bounded artifact path.
        path: TerrainDiagnostic,
        /// Bounded validation failure.
        reason: TerrainDiagnostic,
    },

    /// A durable Surface file uses an unsupported disk contract.
    #[error(
        "incompatible {kind} version at {path}: found {found_version}, supported {supported_version}"
    )]
    IncompatibleSurfaceArtifact {
        /// Stable complete-artifact or work-checkpoint category.
        kind: &'static str,
        /// Bounded artifact path.
        path: TerrainDiagnostic,
        /// Version found on disk, or zero when the magic is unknown.
        found_version: u32,
        /// Exact version supported by this crate.
        supported_version: u32,
    },

    /// An export target already exists and was not replaced.
    #[error("terrain export target already exists: {path}")]
    TargetExists {
        /// Bounded target path.
        path: TerrainDiagnostic,
    },

    /// An existing regular export target differs from the expected bytes.
    #[error(
        "LandXML target conflicts with expected content: expected {expected_hash}, actual {actual_hash} at {path}"
    )]
    ExportConflict {
        /// Bounded target path.
        path: TerrainDiagnostic,
        /// Deterministic hash of the complete expected output.
        expected_hash: ContentHash,
        /// Hash of the complete existing regular target.
        actual_hash: ContentHash,
    },

    /// An existing export target changed while it was being reconciled.
    #[error("LandXML target changed during verification at {path}")]
    TargetChanged {
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
    #[error("LandXML publication is indeterminate for expected hash {expected_hash}: {source}")]
    ExportIndeterminate {
        /// Expected complete output content hash.
        expected_hash: ContentHash,
        /// Structured failure observed after the no-replace target link.
        #[source]
        source: Box<TerrainError>,
    },

    /// Surface publication may have created the complete target.
    #[error(
        "Surface publication is indeterminate for expected complete checksum {expected_complete_checksum} at {path}: {source}"
    )]
    SurfacePublicationIndeterminate {
        /// Bounded target path.
        path: TerrainDiagnostic,
        /// Expected complete-payload checksum stored in the Surface footer.
        expected_complete_checksum: ContentHash,
        /// Structured failure observed after the no-replace link.
        #[source]
        source: Box<TerrainError>,
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

    pub(crate) fn unsupported_spatial_reference(reason: impl AsRef<str>) -> Self {
        Self::UnsupportedSpatialReference {
            reason: TerrainDiagnostic::new(reason),
        }
    }

    pub(crate) fn unsupported_metric_export(reason: impl AsRef<str>) -> Self {
        Self::UnsupportedMetricExport {
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

    pub(crate) fn export_indeterminate(expected_hash: ContentHash, source: TerrainError) -> Self {
        Self::ExportIndeterminate {
            expected_hash,
            source: Box::new(source),
        }
    }

    pub(crate) fn stale_surface(
        kind: &'static str,
        binding: &'static str,
        path: impl fmt::Display,
    ) -> Self {
        Self::StaleSurfaceArtifact {
            kind,
            binding,
            path: TerrainDiagnostic::new(path.to_string()),
        }
    }

    pub(crate) fn corrupt_surface(
        kind: &'static str,
        path: impl fmt::Display,
        reason: impl AsRef<str>,
    ) -> Self {
        Self::CorruptSurfaceArtifact {
            kind,
            path: TerrainDiagnostic::new(path.to_string()),
            reason: TerrainDiagnostic::new(reason),
        }
    }

    pub(crate) fn incompatible_surface(
        kind: &'static str,
        path: impl fmt::Display,
        found_version: u32,
        supported_version: u32,
    ) -> Self {
        Self::IncompatibleSurfaceArtifact {
            kind,
            path: TerrainDiagnostic::new(path.to_string()),
            found_version,
            supported_version,
        }
    }

    pub(crate) fn surface_publication_indeterminate(
        path: impl fmt::Display,
        expected_complete_checksum: ContentHash,
        source: TerrainError,
    ) -> Self {
        Self::SurfacePublicationIndeterminate {
            path: TerrainDiagnostic::new(path.to_string()),
            expected_complete_checksum,
            source: Box::new(source),
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
