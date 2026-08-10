//! Implementer-facing interface for concrete Source adapters.
//!
//! Ordinary callers use [`crate::SourceCandidate`] and [`crate::Source`].
//! This public module exists for Punctra's workspace adapters. Its interface is
//! version-coupled to those official adapters and is not a stable third-party
//! plugin promise in v0.3.

use std::fmt;
use std::sync::Arc;

use foundation_runtime::OperationReporter;
use point_contracts::{ContentHash, PointBatch, SourceId, SourceMetadata};
use serde::Serialize;

use crate::{
    AttributeSelection, MAX_ADAPTER_NAME_BYTES, MAX_ADAPTER_VERSION_BYTES, MAX_LOGICAL_ORDER_BYTES,
    ReadBudget, SourceError, SourcePreview, SourceSpan,
};

/// Caller intent supplied to complete Source verification.
///
/// A recorded expected hash lets an adapter classify malformed input: matching
/// immutable content is corrupt, while different content has changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullVerification {
    /// Identify the current input without a prior Source record.
    Identify,
    /// Require the current input to match recorded immutable content.
    Match {
        /// Full content hash retained in the recorded Source evidence.
        expected_content_hash: ContentHash,
    },
}

impl FullVerification {
    /// Returns the recorded content hash for a reopen, if present.
    #[must_use]
    pub const fn expected_content_hash(self) -> Option<ContentHash> {
        match self {
            Self::Identify => None,
            Self::Match {
                expected_content_hash,
            } => Some(expected_content_hash),
        }
    }
}

/// Validated identity contract for one concrete Source adapter.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct AdapterContract {
    #[serde(rename = "adapter_name")]
    name: String,
    #[serde(rename = "adapter_version")]
    version: String,
    logical_order: String,
}

impl AdapterContract {
    /// Creates a bounded, non-empty adapter identity contract.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::SourceContractMismatch`] when any field is empty
    /// or exceeds its persisted byte limit.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        logical_order: impl Into<String>,
    ) -> Result<Self, SourceError> {
        let name = validated_contract_text("adapter name", name.into(), MAX_ADAPTER_NAME_BYTES)?;
        let version =
            validated_contract_text("adapter version", version.into(), MAX_ADAPTER_VERSION_BYTES)?;
        let logical_order = validated_contract_text(
            "logical order",
            logical_order.into(),
            MAX_LOGICAL_ORDER_BYTES,
        )?;
        Ok(Self {
            name,
            version,
            logical_order,
        })
    }

    /// Returns the concrete adapter name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the concrete adapter contract version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the adapter's canonical Point ordering rule.
    #[must_use]
    pub fn logical_order(&self) -> &str {
        &self.logical_order
    }
}

fn validated_contract_text(
    field: &str,
    value: String,
    max_bytes: usize,
) -> Result<String, SourceError> {
    if value.trim().is_empty() {
        return Err(SourceError::contract(format!("{field} is empty")));
    }
    if value.len() > max_bytes {
        return Err(SourceError::contract(format!(
            "{field} exceeds its {max_bytes}-byte limit"
        )));
    }
    Ok(value)
}

/// Untrusted verification result supplied by a concrete adapter.
///
/// `point-source` validates this value and is the only module that can publish
/// an always-verified [`crate::Source`].
pub struct AdapterVerified {
    contract: AdapterContract,
    metadata: Arc<SourceMetadata>,
    content_hash: ContentHash,
    fast_token: Vec<u8>,
    reader: Arc<dyn ReadAdapter>,
}

impl AdapterVerified {
    /// Creates an adapter verification result.
    #[must_use]
    pub fn new(
        contract: AdapterContract,
        metadata: Arc<SourceMetadata>,
        content_hash: ContentHash,
        fast_token: Vec<u8>,
        reader: Arc<dyn ReadAdapter>,
    ) -> Self {
        Self {
            contract,
            metadata,
            content_hash,
            fast_token,
            reader,
        }
    }

    pub(crate) const fn contract(&self) -> &AdapterContract {
        &self.contract
    }

    pub(crate) fn metadata(&self) -> &SourceMetadata {
        self.metadata.as_ref()
    }

    pub(crate) const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    pub(crate) fn fast_token(&self) -> &[u8] {
        &self.fast_token
    }

    pub(crate) fn into_parts(self) -> AdapterVerifiedParts {
        AdapterVerifiedParts {
            contract: self.contract,
            metadata: self.metadata,
            content_hash: self.content_hash,
            fast_token: self.fast_token,
            reader: self.reader,
        }
    }
}

impl fmt::Debug for AdapterVerified {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdapterVerified")
            .field("contract", &self.contract)
            .field("metadata", &self.metadata)
            .field("content_hash", &self.content_hash)
            .field("fast_token_bytes", &self.fast_token.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct AdapterVerifiedParts {
    pub(crate) contract: AdapterContract,
    pub(crate) metadata: Arc<SourceMetadata>,
    pub(crate) content_hash: ContentHash,
    pub(crate) fast_token: Vec<u8>,
    pub(crate) reader: Arc<dyn ReadAdapter>,
}

/// Adapter behavior needed to verify one Source candidate.
pub trait CandidateAdapter: Send + Sync + 'static {
    /// Returns cheap, explicitly unverified descriptive information.
    fn preview(&self) -> &SourcePreview;

    /// Computes complete immutable content verification.
    ///
    /// Adapters may publish monotonic progress while verification is running,
    /// but [`foundation_runtime::ProgressPhase::COMPLETE`] is reserved for the
    /// `point-source` wrapper after verified Source construction succeeds.
    /// Full verification may run after an inconclusive Fast attempt on the same
    /// reporter, so its first snapshot must advance from (or exactly repeat)
    /// [`OperationReporter::progress`] rather than restart counters.
    ///
    /// # Errors
    ///
    /// Returns an adapter, corruption, cancellation, or resource error.
    fn full_verify(
        &self,
        verification: FullVerification,
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError>;

    /// Verifies the adapter's recorded fast token.
    ///
    /// `Ok` is permitted only when the token conclusively proves the complete
    /// current adapter contract. Missing, expired, ambiguous, or mismatched
    /// evidence must return [`SourceError::VerificationRequired`]; Fast
    /// verification must not report [`SourceError::SourceChanged`] on token
    /// evidence alone. The `point-source` wrapper defensively normalizes such
    /// mismatch errors and, when requested, falls back to Full verification.
    /// Progress published here remains visible to a fallback Full attempt and
    /// therefore must use counters that Full verification can advance.
    ///
    /// Adapters may publish monotonic progress while verification is running,
    /// but [`foundation_runtime::ProgressPhase::COMPLETE`] is reserved for the
    /// `point-source` wrapper after verified Source construction succeeds.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::VerificationRequired`] for failed proof, or an
    /// adapter, corruption, cancellation, or resource error.
    fn fast_verify(
        &self,
        expected_fast_token: &[u8],
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError>;
}

/// Canonical, normalized request delivered to a verified read adapter.
#[derive(Clone, Debug)]
pub struct AdapterReadRequest {
    spans: Arc<[SourceSpan]>,
    attributes: AttributeSelection,
    budget: ReadBudget,
}

impl AdapterReadRequest {
    pub(crate) fn new(
        spans: Arc<[SourceSpan]>,
        attributes: AttributeSelection,
        budget: ReadBudget,
    ) -> Self {
        Self {
            spans,
            attributes,
            budget,
        }
    }

    /// Returns sorted, disjoint, non-adjacent Source spans.
    #[must_use]
    pub fn spans(&self) -> &[SourceSpan] {
        &self.spans
    }

    /// Returns exact sorted, duplicate-free Attribute identities.
    ///
    /// The value always contains a bounded exact selection; an all-Attributes
    /// request is resolved against verified metadata before an adapter sees it.
    #[must_use]
    pub const fn attributes(&self) -> &AttributeSelection {
        &self.attributes
    }

    /// Returns the hard whole-read and per-batch budget.
    #[must_use]
    pub const fn budget(&self) -> ReadBudget {
        self.budget
    }
}

/// Verified adapter behavior that starts one bounded canonical read.
pub trait ReadAdapter: Send + Sync + 'static {
    /// Starts a pull-based read for an already validated request.
    ///
    /// `reporter` is supplied for cooperative cancellation checks. Accepted
    /// Point progress and terminal completion are published by `point-source`
    /// only after it validates each batch and the exact summary.
    ///
    /// # Errors
    ///
    /// Returns an adapter or resource error before any Point is published.
    fn start_read(
        &self,
        request: AdapterReadRequest,
        source: SourceId,
        reporter: OperationReporter,
    ) -> Result<Box<dyn AdapterRead>, SourceError>;
}

/// Pull-based adapter read that emits a Point Batch or terminal end.
pub trait AdapterRead: Send {
    /// Returns the next Point Batch, or `None` at terminal end.
    ///
    /// # Errors
    ///
    /// Returns an adapter, corrupt-input, changed-input, or cancellation error.
    fn next(&mut self) -> Result<Option<PointBatch>, SourceError>;
}
