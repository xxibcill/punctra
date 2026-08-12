use std::{fmt, ops::Deref};

use foundation_runtime::RuntimeError;
use point_contracts::{AttributeId, SourceId};
use thiserror::Error;

/// Maximum UTF-8 bytes retained in one adapter-owned Source diagnostic.
pub const MAX_SOURCE_DIAGNOSTIC_BYTES: usize = 4 * 1024;

/// Bounded adapter-owned text attached to a [`SourceError`].
///
/// Oversized text is truncated at a UTF-8 boundary with an ellipsis so Source
/// errors never retain an unbounded adapter diagnostic.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceDiagnostic(String);

impl SourceDiagnostic {
    /// Retains bounded diagnostic text, truncating oversized input.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        const ELLIPSIS: &str = "…";

        let mut message = message.into();
        if message.len() <= MAX_SOURCE_DIAGNOSTIC_BYTES {
            return Self(message);
        }

        let mut end = MAX_SOURCE_DIAGNOSTIC_BYTES - ELLIPSIS.len();
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str(ELLIPSIS);
        Self(message)
    }

    /// Returns the retained diagnostic text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for SourceDiagnostic {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<String> for SourceDiagnostic {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for SourceDiagnostic {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// Stable identity of a caller or adapter Source-read limit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReadLimit {
    /// Caller maximum Points in one batch.
    MaxBatchPoints,
    /// Caller maximum canonical payload bytes in one batch.
    MaxBatchPayloadBytes,
    /// Total Points selected by a normalized request.
    RequestedPoints,
    /// Raw Source spans supplied before normalization.
    InputSourceSpans,
    /// Normalized disjoint Source spans.
    NormalizedSourceSpans,
    /// Raw Attribute identities supplied before resolution.
    InputAttributeIdentities,
    /// Points in one emitted batch.
    BatchPoints,
    /// Canonical bytes required by one Point.
    PointPayloadBytes,
    /// Canonical payload bytes in one emitted batch.
    BatchPayloadBytes,
    /// Adapter decoder working memory.
    AdapterWorkingBytes,
    /// Full-verification decoder working memory.
    VerificationWorkingBytes,
    /// Points accepted from adapter batches.
    EmittedPoints,
}

impl fmt::Display for ReadLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MaxBatchPoints => "max batch Points",
            Self::MaxBatchPayloadBytes => "max batch payload bytes",
            Self::RequestedPoints => "requested Point count",
            Self::InputSourceSpans => "input Source spans",
            Self::NormalizedSourceSpans => "normalized Source spans",
            Self::InputAttributeIdentities => "input Attribute identities",
            Self::BatchPoints => "batch Points",
            Self::PointPayloadBytes => "Point payload bytes",
            Self::BatchPayloadBytes => "batch payload bytes",
            Self::AdapterWorkingBytes => "adapter working bytes",
            Self::VerificationWorkingBytes => "verification working bytes",
            Self::EmittedPoints => "emitted Point count",
        })
    }
}

/// Failure reported while verifying or reading a [`Source`](crate::Source).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SourceError {
    /// The candidate cannot be accessed at its recorded local location.
    #[error("Source is missing: {reason}")]
    SourceMissing {
        /// Bounded missing-input diagnostic.
        reason: SourceDiagnostic,
    },

    /// Stable candidate bytes violate the adapter's format contract.
    #[error("Source is corrupt: {reason}")]
    CorruptSource {
        /// Bounded corruption diagnostic.
        reason: SourceDiagnostic,
    },

    /// The concrete input format is not supported.
    #[error("unsupported Source format: {format}")]
    UnsupportedFormat {
        /// Adapter-observed format label.
        format: SourceDiagnostic,
    },

    /// The Source Attribute schema cannot be represented losslessly.
    #[error("unsupported Source schema: {reason}")]
    UnsupportedSchema {
        /// Bounded schema diagnostic.
        reason: SourceDiagnostic,
    },

    /// Fast verification could not prove that the recorded Source is unchanged.
    #[error("fast Source verification was inconclusive; Full verification is required")]
    VerificationRequired,

    /// The candidate no longer has the recorded Source content.
    #[error("the Source changed: {reason}")]
    SourceChanged {
        /// Bounded explanation of the mismatch.
        reason: SourceDiagnostic,
    },

    /// A recorded Source contract is incompatible with this implementation.
    #[error("the recorded Source contract is incompatible: {reason}")]
    SourceContractMismatch {
        /// Bounded explanation of the mismatch.
        reason: SourceDiagnostic,
    },

    /// The serialized record uses an unsupported schema version.
    #[error("unsupported SourceRecord schema version {version}")]
    UnsupportedRecordVersion {
        /// Unsupported version number.
        version: u32,
    },

    /// A Source span is empty, overflows, or falls outside the Source.
    #[error("invalid Source span starting at {first_ordinal} with count {point_count}")]
    InvalidSourceSpan {
        /// First requested Point ordinal.
        first_ordinal: u64,
        /// Requested Point count.
        point_count: u64,
    },

    /// A caller-supplied budget has an invalid zero limit.
    #[error("{limit} must be greater than zero")]
    InvalidBudget {
        /// Name of the invalid limit.
        limit: ReadLimit,
    },

    /// A valid Point cannot fit within the caller's hard read budget.
    #[error("the Source read exceeded {limit}: required {required}, limit {allowed}")]
    ResourceLimit {
        /// Name of the exceeded limit.
        limit: ReadLimit,
        /// Amount required by the rejected batch.
        required: u64,
        /// Hard caller-selected limit.
        allowed: u64,
    },

    /// An adapter returned a Point Batch for another Source.
    #[error("adapter returned Source {actual:?}; expected {expected:?}")]
    AdapterSourceMismatch {
        /// Verified Source identity.
        expected: SourceId,
        /// Identity returned by the adapter.
        actual: SourceId,
    },

    /// An adapter changed the verified position transform.
    #[error("adapter returned a position transform that differs from verified metadata")]
    AdapterTransformMismatch,

    /// An adapter returned a position outside verified Source bounds.
    #[error(
        "adapter returned an invalid position at Point ordinal {ordinal}, axis {axis}: {reason}"
    )]
    AdapterPositionOutOfBounds {
        /// Canonical Point ordinal of the invalid position.
        ordinal: u64,
        /// Zero-based position axis.
        axis: usize,
        /// Bounded position-contract diagnostic.
        reason: SourceDiagnostic,
    },

    /// An adapter returned a gap, duplicate, or out-of-request Point ordinal.
    #[error("adapter returned Point ordinal {actual}; expected {expected}")]
    AdapterOrdinalMismatch {
        /// Next required Point ordinal.
        expected: u64,
        /// First ordinal returned by the adapter.
        actual: u64,
    },

    /// An adapter batch extended beyond its current normalized Source span.
    #[error("adapter batch ending at {batch_end} exceeds requested span ending at {span_end}")]
    AdapterSpanOverflow {
        /// Exclusive batch end ordinal.
        batch_end: u64,
        /// Exclusive normalized span end ordinal.
        span_end: u64,
    },

    /// An adapter returned the wrong Attribute set or type.
    #[error("adapter returned an invalid column for Attribute {attribute:?}: {reason}")]
    AdapterAttributeMismatch {
        /// Invalid Attribute identity.
        attribute: AttributeId,
        /// Bounded mismatch description.
        reason: SourceDiagnostic,
    },

    /// An adapter returned an Attribute that was not requested.
    #[error("adapter returned unrequested Attribute {attribute:?}")]
    AdapterUnexpectedAttribute {
        /// Unexpected Attribute identity.
        attribute: AttributeId,
    },

    /// An adapter ended before emitting every requested Point.
    #[error("adapter ended after {emitted} Points; expected {expected}")]
    AdapterEndedEarly {
        /// Points accepted before terminal end.
        emitted: u64,
        /// Exact normalized request count.
        expected: u64,
    },

    /// An adapter-specific operation failed.
    #[error("Source adapter failed: {message}")]
    Adapter {
        /// Bounded adapter diagnostic.
        message: SourceDiagnostic,
    },

    /// The operation was cancelled before completion.
    #[error("Source operation was cancelled")]
    Cancelled,

    /// Runtime-neutral Job or control failure.
    #[error(transparent)]
    Runtime(RuntimeError),
}

impl From<RuntimeError> for SourceError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::Cancelled => Self::Cancelled,
            error => Self::Runtime(error),
        }
    }
}

impl SourceError {
    /// Creates a bounded adapter diagnostic.
    #[must_use]
    pub fn adapter(message: impl Into<SourceDiagnostic>) -> Self {
        Self::Adapter {
            message: message.into(),
        }
    }

    /// Creates an explicit immutable-content mismatch.
    #[must_use]
    pub fn changed(reason: impl Into<SourceDiagnostic>) -> Self {
        Self::SourceChanged {
            reason: reason.into(),
        }
    }

    /// Creates an explicit corrupt-input failure.
    #[must_use]
    pub fn corrupt(reason: impl Into<SourceDiagnostic>) -> Self {
        Self::CorruptSource {
            reason: reason.into(),
        }
    }

    /// Creates an explicit unsupported-schema failure.
    #[must_use]
    pub fn unsupported_schema(reason: impl Into<SourceDiagnostic>) -> Self {
        Self::UnsupportedSchema {
            reason: reason.into(),
        }
    }

    pub(crate) fn contract(reason: impl Into<SourceDiagnostic>) -> Self {
        Self::SourceContractMismatch {
            reason: reason.into(),
        }
    }
}
