use std::{fmt, io, path::PathBuf, sync::Arc};

use foundation_runtime::RuntimeError;
use point_source::SourceError;
use thiserror::Error;

/// Stable identity of a caller or implementation index limit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IndexLimit {
    /// Nonzero Source batch Point ceiling accepted by [`crate::PrepareLimits`].
    MaxSourceBatchPoints,
    /// Nonzero Source batch payload ceiling accepted by [`crate::PrepareLimits`].
    MaxSourceBatchPayloadBytes,
    /// Nonzero emitted-Point ceiling accepted by [`crate::NodeReadBudget`].
    MaxEmittedPoints,
    /// Nonzero display-batch byte ceiling accepted by [`crate::NodeReadBudget`].
    MaxDisplayBatchBytes,
    /// Sample Point count representable by the current process.
    AddressableSamplePoints,
    /// Memory retained while reading persisted index samples.
    IndexSampleBufferBytes,
    /// Memory allocated for a sample buffer.
    SampleBufferBytes,
    /// Index construction working memory.
    BuildWorkingBytes,
    /// Encoded payload bytes in one incomplete-index frame.
    WorkFramePayloadBytes,
    /// Bytes retained in an incomplete index.
    IncompleteIndexBytes,
    /// Combined bytes retained by an incomplete index and sample spool.
    IncompleteAndSampleSpoolBytes,
    /// Work-frame count representable by the current process.
    AddressableWorkFrames,
    /// Sample Point count representable by the artifact format.
    ArtifactSamplePoints,
    /// Complete artifact bytes.
    ArtifactBytes,
    /// Hierarchy node count.
    HierarchyNodes,
    /// Hierarchy node count representable by the current process.
    AddressableHierarchyNodes,
    /// Resident index metadata bytes.
    ResidentIndexMetadataBytes,
    /// Memory used to verify an artifact checksum.
    ArtifactVerificationWorkingBytes,
    /// Memory used to validate an artifact after opening it.
    ArtifactValidationWorkingBytes,
    /// Hierarchy nodes visited by candidate planning.
    VisitedHierarchyNodes,
    /// Source Points covered by a candidate plan.
    CandidatePoints,
    /// Source spans emitted by a candidate plan.
    CandidateSourceSpans,
    /// Candidate-planning working memory.
    CandidateWorkingBytes,
    /// Display Points emitted by one node read.
    EmittedDisplayPoints,
    /// Source spans used by one node read.
    SourceSpans,
    /// Source Points in one batch used by a node read.
    SourceBatchPoints,
    /// Source payload bytes in one batch used by a node read.
    SourceBatchPayloadBytes,
    /// Display bytes in one emitted batch.
    DisplayBatchBytes,
    /// Memory used to buffer persisted index samples.
    IndexBufferBytes,
    /// Display Points in one emitted batch.
    DisplayBatchPoints,
}

impl fmt::Display for IndexLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MaxSourceBatchPoints => "max_source_batch_points",
            Self::MaxSourceBatchPayloadBytes => "max_source_batch_payload_bytes",
            Self::MaxEmittedPoints => "max_emitted_points",
            Self::MaxDisplayBatchBytes => "max_display_batch_bytes",
            Self::AddressableSamplePoints => "addressable sample Points",
            Self::IndexSampleBufferBytes => "index sample buffer bytes",
            Self::SampleBufferBytes => "sample buffer bytes",
            Self::BuildWorkingBytes => "build working bytes",
            Self::WorkFramePayloadBytes => "work frame payload bytes",
            Self::IncompleteIndexBytes => "incomplete index bytes",
            Self::IncompleteAndSampleSpoolBytes => "incomplete and sample-spool bytes",
            Self::AddressableWorkFrames => "addressable work frames",
            Self::ArtifactSamplePoints => "artifact sample Points",
            Self::ArtifactBytes => "artifact bytes",
            Self::HierarchyNodes => "hierarchy nodes",
            Self::AddressableHierarchyNodes => "addressable hierarchy nodes",
            Self::ResidentIndexMetadataBytes => "resident index metadata bytes",
            Self::ArtifactVerificationWorkingBytes => "artifact verification working bytes",
            Self::ArtifactValidationWorkingBytes => "artifact validation working bytes",
            Self::VisitedHierarchyNodes => "visited hierarchy nodes",
            Self::CandidatePoints => "candidate Points",
            Self::CandidateSourceSpans => "candidate Source spans",
            Self::CandidateWorkingBytes => "candidate working bytes",
            Self::EmittedDisplayPoints => "emitted display Points",
            Self::SourceSpans => "Source spans",
            Self::SourceBatchPoints => "Source batch Points",
            Self::SourceBatchPayloadBytes => "Source batch payload bytes",
            Self::DisplayBatchBytes => "display batch bytes",
            Self::IndexBufferBytes => "index buffer bytes",
            Self::DisplayBatchPoints => "display batch Points",
        })
    }
}

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
        /// Invalid limit identity.
        limit: IndexLimit,
    },

    /// The selected inspection recipe does not match the Source Attribute schema.
    #[error("invalid inspection Attribute profile: {reason}")]
    InvalidAttributeProfile {
        /// Bounded static profile mismatch category.
        reason: &'static str,
    },

    /// Work or retained output exceeded one caller-selected hard limit.
    #[error("index exceeded {limit}: required {required}, limit {allowed}")]
    ResourceLimit {
        /// Exceeded resource identity.
        limit: IndexLimit,
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

    /// Another preparation operation owns the writable state for this target.
    #[error("index preparation is already in progress for {path}")]
    PreparationInProgress {
        /// Requested complete-index path whose build is already owned.
        path: PathBuf,
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

    /// A filesystem operation on an already-owned path failed without copying
    /// the path after filesystem mutation began.
    #[error("failed to {operation} {path}: {source}")]
    SharedPathIo {
        /// Stable operation description.
        operation: &'static str,
        /// Exact path operated on, retained through shared ownership.
        path: Arc<std::path::Path>,
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

    pub(crate) fn io_shared(
        operation: &'static str,
        path: Arc<std::path::Path>,
        source: io::Error,
    ) -> Self {
        Self::SharedPathIo {
            operation,
            path,
            source,
        }
    }
}
