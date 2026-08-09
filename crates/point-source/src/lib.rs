//! Verified, bounded canonical access to one immutable point-cloud Source.
//!
//! Concrete format adapters produce [`SourceCandidate`] values. Opening a
//! candidate returns a runtime-neutral [`Job`] and publishes [`Source`] only
//! after identity, metadata, and adapter verification succeed.
//!
//! ```
//! use point_source::{ReadBudget, ReadRequest, SourceSpan};
//!
//! let request = ReadRequest::all()
//!     .spans([SourceSpan::new(40, 20)?])
//!     .budget(ReadBudget::new(4_096, 8 * 1024 * 1024)?);
//! # let _ = request;
//! # Ok::<(), point_source::SourceError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use blake3::Hasher;
use foundation_runtime::{Job, OperationControl};
use point_contracts::{AttributeId, ContentHash, SourceId, SourceMetadata, SourceProvenance};
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

pub mod adapter;
mod error;
mod stream;

use adapter::{AdapterVerified, CandidateAdapter, FullVerification, ReadAdapter};
pub use error::{MAX_SOURCE_DIAGNOSTIC_BYTES, SourceDiagnostic, SourceError};
pub use point_contracts::{MAX_ATTRIBUTE_DEFINITIONS, MAX_LOGICAL_ORDER_BYTES};
pub use stream::{PointBatches, SourceReadSummary};

/// Runtime-neutral verification job that publishes one verified [`Source`].
pub type SourceJob = Job<Source, SourceError>;

// Persisted wire schema and deterministic Source semantics evolve separately.
const SOURCE_RECORD_VERSION: u32 = 1;
const SOURCE_CONTRACT_VERSION: u32 = 1;
const SOURCE_ID_DOMAIN: &[u8] = b"punctra-source-id-v1";
const DEFAULT_BATCH_POINTS: u64 = 65_536;
const DEFAULT_BATCH_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_MAX_SPANS: u64 = 65_536;
const DEFAULT_ADAPTER_WORKING_BYTES: u64 = 16 * 1024 * 1024;

/// Hard safety cap on raw Source spans accepted before normalization.
///
/// [`ReadBudget::max_spans`] separately limits the normalized, disjoint spans
/// delivered to an adapter.
pub const MAX_INPUT_SOURCE_SPANS: usize = 1_048_576;

/// Hard safety cap on raw Attribute identities accepted by one selection.
///
/// A Source schema cannot contain more than [`MAX_ATTRIBUTE_DEFINITIONS`], so
/// retaining additional caller input could never make a read more expressive.
pub const MAX_INPUT_ATTRIBUTE_IDS: usize = MAX_ATTRIBUTE_DEFINITIONS;

/// Maximum UTF-8 bytes in a serialized adapter name.
pub const MAX_ADAPTER_NAME_BYTES: usize = 256;

/// Maximum UTF-8 bytes in a serialized adapter contract version.
pub const MAX_ADAPTER_VERSION_BYTES: usize = 256;

/// Maximum opaque adapter Fast-evidence bytes in one [`SourceRecord`].
pub const MAX_FAST_TOKEN_BYTES: usize = 64 * 1024;

/// Cheap, explicitly unverified information about a candidate Source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePreview {
    format: String,
    display_name: Option<String>,
}

impl SourcePreview {
    /// Creates an adapter-owned preview.
    #[must_use]
    pub fn new(format: impl Into<String>, display_name: Option<String>) -> Self {
        Self {
            format: format.into(),
            display_name,
        }
    }

    /// Returns the adapter's format label.
    #[must_use]
    pub fn format(&self) -> &str {
        &self.format
    }

    /// Returns the optional caller-facing display name.
    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }
}

/// Verification work used when matching a recorded Source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationPolicy {
    /// Use only the adapter's recorded fast token.
    FastOnly,
    /// Recompute complete content verification.
    Full,
    /// Try Fast verification and run Full when Fast is inconclusive.
    FastThenFull,
}

/// Valid-by-construction options for opening a candidate Source.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    expectation: OpenExpectation,
}

#[derive(Clone, Debug)]
enum OpenExpectation {
    Identify,
    Match {
        record: Box<SourceRecord>,
        policy: VerificationPolicy,
    },
}

impl OpenOptions {
    /// Identifies a Source through mandatory Full verification.
    #[must_use]
    pub const fn identify() -> Self {
        Self {
            expectation: OpenExpectation::Identify,
        }
    }

    /// Requires a candidate to match a serialized Source record.
    #[must_use]
    pub fn match_record(record: SourceRecord, policy: VerificationPolicy) -> Self {
        Self {
            expectation: OpenExpectation::Match {
                record: Box::new(record),
                policy,
            },
        }
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::identify()
    }
}

/// Versioned, serializable evidence for reopening the same immutable Source.
///
/// Deserialization enforces the exported adapter-name, adapter-version,
/// logical-order, and Fast-token byte limits before this value is published.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceRecord {
    version: u32,
    source: SourceId,
    content_hash: ContentHash,
    adapter_name: String,
    adapter_version: String,
    logical_order: String,
    metadata: Arc<SourceMetadata>,
    fast_token: Vec<u8>,
}

#[derive(Deserialize)]
struct SourceRecordWire {
    version: u32,
    source: SourceId,
    content_hash: ContentHash,
    adapter_name: BoundedString<MAX_ADAPTER_NAME_BYTES>,
    adapter_version: BoundedString<MAX_ADAPTER_VERSION_BYTES>,
    logical_order: BoundedString<MAX_LOGICAL_ORDER_BYTES>,
    metadata: SourceMetadata,
    fast_token: BoundedBytes<MAX_FAST_TOKEN_BYTES>,
}

impl<'de> Deserialize<'de> for SourceRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SourceRecordWire::deserialize(deserializer)?;
        Ok(Self {
            version: wire.version,
            source: wire.source,
            content_hash: wire.content_hash,
            adapter_name: wire.adapter_name.0,
            adapter_version: wire.adapter_version.0,
            logical_order: wire.logical_order.0,
            metadata: Arc::new(wire.metadata),
            fast_token: wire.fast_token.0,
        })
    }
}

struct BoundedString<const MAX_BYTES: usize>(String);

impl<'de, const MAX_BYTES: usize> Deserialize<'de> for BoundedString<MAX_BYTES> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(BoundedStringVisitor::<MAX_BYTES>(PhantomData))
    }
}

struct BoundedStringVisitor<const MAX_BYTES: usize>(PhantomData<()>);

impl<const MAX_BYTES: usize> BoundedStringVisitor<MAX_BYTES> {
    fn accept<E>(value: String) -> Result<BoundedString<MAX_BYTES>, E>
    where
        E: serde::de::Error,
    {
        if value.trim().is_empty() {
            return Err(E::custom("SourceRecord strings must not be empty"));
        }
        if value.len() > MAX_BYTES {
            return Err(E::custom(format_args!(
                "string exceeds the {MAX_BYTES}-byte SourceRecord limit"
            )));
        }
        Ok(BoundedString(value))
    }
}

impl<const MAX_BYTES: usize> Visitor<'_> for BoundedStringVisitor<MAX_BYTES> {
    type Value = BoundedString<MAX_BYTES>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a UTF-8 string no longer than {MAX_BYTES} bytes")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value.trim().is_empty() {
            return Err(E::custom("SourceRecord strings must not be empty"));
        }
        if value.len() > MAX_BYTES {
            return Err(E::custom(format_args!(
                "string exceeds the {MAX_BYTES}-byte SourceRecord limit"
            )));
        }
        Ok(BoundedString(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Self::accept(value)
    }
}

struct BoundedBytes<const MAX_BYTES: usize>(Vec<u8>);

impl<'de, const MAX_BYTES: usize> Deserialize<'de> for BoundedBytes<MAX_BYTES> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(BoundedBytesVisitor::<MAX_BYTES>(PhantomData))
    }
}

struct BoundedBytesVisitor<const MAX_BYTES: usize>(PhantomData<()>);

impl<const MAX_BYTES: usize> BoundedBytesVisitor<MAX_BYTES> {
    fn accept<E>(value: Vec<u8>) -> Result<BoundedBytes<MAX_BYTES>, E>
    where
        E: serde::de::Error,
    {
        if value.len() > MAX_BYTES {
            return Err(E::custom(format_args!(
                "byte sequence exceeds the {MAX_BYTES}-byte SourceRecord limit"
            )));
        }
        Ok(BoundedBytes(value))
    }
}

impl<'de, const MAX_BYTES: usize> Visitor<'de> for BoundedBytesVisitor<MAX_BYTES> {
    type Value = BoundedBytes<MAX_BYTES>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "at most {MAX_BYTES} bytes")
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value.len() > MAX_BYTES {
            return Err(E::custom(format_args!(
                "byte sequence exceeds the {MAX_BYTES}-byte SourceRecord limit"
            )));
        }
        Ok(BoundedBytes(value.to_vec()))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Self::accept(value)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if sequence.size_hint().is_some_and(|size| size > MAX_BYTES) {
            return Err(serde::de::Error::custom(format_args!(
                "byte sequence exceeds the {MAX_BYTES}-byte SourceRecord limit"
            )));
        }
        let capacity = sequence.size_hint().unwrap_or(0).min(MAX_BYTES);
        let mut bytes = Vec::with_capacity(capacity);
        while let Some(byte) = sequence.next_element()? {
            if bytes.len() == MAX_BYTES {
                return Err(serde::de::Error::custom(format_args!(
                    "byte sequence exceeds the {MAX_BYTES}-byte SourceRecord limit"
                )));
            }
            bytes.push(byte);
        }
        Ok(BoundedBytes(bytes))
    }
}

impl SourceRecord {
    /// Returns the persisted record schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the immutable Source Identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Returns the complete content fingerprint.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Returns the concrete adapter name.
    #[must_use]
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Returns the concrete adapter contract version.
    #[must_use]
    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }

    /// Returns the adapter's canonical Point ordering rule.
    #[must_use]
    pub fn logical_order(&self) -> &str {
        &self.logical_order
    }

    /// Returns verified canonical Source metadata.
    #[must_use]
    pub fn metadata(&self) -> &SourceMetadata {
        self.metadata.as_ref()
    }

    /// Returns opaque adapter evidence used for Fast verification.
    #[must_use]
    pub fn fast_token(&self) -> &[u8] {
        &self.fast_token
    }
}

/// Unverified input supplied by one concrete Source adapter.
#[derive(Clone)]
pub struct SourceCandidate {
    adapter: Arc<dyn CandidateAdapter>,
}

impl SourceCandidate {
    /// Wraps a concrete candidate adapter.
    #[must_use]
    pub fn new_adapter(adapter: impl CandidateAdapter) -> Self {
        Self {
            adapter: Arc::new(adapter),
        }
    }

    /// Returns cheap information that has not established Source Identity.
    #[must_use]
    pub fn preview(&self) -> &SourcePreview {
        self.adapter.preview()
    }

    /// Starts runtime-neutral Source verification.
    #[must_use]
    pub fn open(self, options: OpenOptions) -> SourceJob {
        Job::spawn(move |control| open_source(self.adapter.as_ref(), options, &control))
    }
}

impl fmt::Debug for SourceCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceCandidate")
            .field("preview", self.preview())
            .finish_non_exhaustive()
    }
}

/// Verified immutable Source with bounded canonical Point reads.
#[derive(Clone)]
pub struct Source {
    inner: Arc<SourceInner>,
}

struct SourceInner {
    identity: SourceId,
    provenance: SourceProvenance,
    record: SourceRecord,
    reader: Arc<dyn ReadAdapter>,
}

impl Source {
    /// Returns the stable immutable Source Identity.
    #[must_use]
    pub fn identity(&self) -> SourceId {
        self.inner.identity
    }

    /// Returns canonical Source and Attribute metadata.
    #[must_use]
    pub fn metadata(&self) -> &SourceMetadata {
        self.inner.record.metadata()
    }

    /// Returns detached immutable Source provenance.
    #[must_use]
    pub fn provenance(&self) -> &SourceProvenance {
        &self.inner.provenance
    }

    /// Returns serializable verification evidence for a later reopen.
    #[must_use]
    pub fn record(&self) -> &SourceRecord {
        &self.inner.record
    }

    /// Reads every Point and every declared Attribute under hard defaults.
    ///
    /// # Errors
    ///
    /// Returns an invalid-request, resource, adapter, or cancellation error.
    pub fn points(&self) -> Result<PointBatches, SourceError> {
        self.read(ReadRequest::all())
    }

    /// Starts one validated, bounded canonical read.
    ///
    /// # Errors
    ///
    /// Returns an invalid-request, resource, adapter, or cancellation error.
    pub fn read(&self, request: ReadRequest) -> Result<PointBatches, SourceError> {
        PointBatches::start(
            self.inner.identity,
            Arc::clone(&self.inner.record.metadata),
            self.inner.provenance.clone(),
            self.inner.reader.as_ref(),
            request,
        )
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Source")
            .field("identity", &self.inner.identity)
            .field("metadata", &self.inner.record.metadata)
            .finish_non_exhaustive()
    }
}

/// One non-empty half-open interval of canonical Point ordinals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    first_ordinal: u64,
    point_count: u64,
}

impl SourceSpan {
    /// Creates a non-empty, non-overflowing Source span.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidSourceSpan`] for zero count or overflow.
    pub fn new(first_ordinal: u64, point_count: u64) -> Result<Self, SourceError> {
        if point_count == 0 || first_ordinal.checked_add(point_count).is_none() {
            return Err(SourceError::InvalidSourceSpan {
                first_ordinal,
                point_count,
            });
        }
        Ok(Self {
            first_ordinal,
            point_count,
        })
    }

    /// Returns the first included Point ordinal.
    #[must_use]
    pub const fn first_ordinal(self) -> u64 {
        self.first_ordinal
    }

    /// Returns the number of included Points.
    #[must_use]
    pub const fn point_count(self) -> u64 {
        self.point_count
    }

    /// Returns the exclusive end ordinal.
    #[must_use]
    pub const fn end_ordinal(self) -> u64 {
        self.first_ordinal + self.point_count
    }
}

/// Canonical Attributes requested from a Source read.
///
/// The representation is opaque so an oversized vector cannot bypass the
/// bounded [`AttributeSelection::only`] constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributeSelection {
    kind: AttributeSelectionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AttributeSelectionKind {
    All,
    Only(Vec<AttributeId>),
    TooManyInputAttributes { at_least: u64 },
}

impl AttributeSelection {
    /// Selects every Attribute declared by Source metadata.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            kind: AttributeSelectionKind::All,
        }
    }

    /// Creates an exact selection, sorting and deduplicating identities.
    ///
    /// Collection is capped at [`MAX_INPUT_ATTRIBUTE_IDS`]. A larger or
    /// infinite iterator is retained as an invalid bounded request and later
    /// rejected by [`Source::read`].
    #[must_use]
    pub fn only(attributes: impl IntoIterator<Item = AttributeId>) -> Self {
        let mut attributes = attributes.into_iter();
        let lower_bound = attributes.size_hint().0;
        if lower_bound > MAX_INPUT_ATTRIBUTE_IDS {
            return Self {
                kind: AttributeSelectionKind::TooManyInputAttributes {
                    at_least: u64::try_from(lower_bound).unwrap_or(u64::MAX),
                },
            };
        }

        let mut collected = Vec::with_capacity(lower_bound.min(MAX_INPUT_ATTRIBUTE_IDS));
        collected.extend(attributes.by_ref().take(MAX_INPUT_ATTRIBUTE_IDS));
        if attributes.next().is_some() {
            return Self {
                kind: AttributeSelectionKind::TooManyInputAttributes {
                    at_least: u64::try_from(MAX_INPUT_ATTRIBUTE_IDS)
                        .unwrap_or(u64::MAX)
                        .saturating_add(1),
                },
            };
        }
        collected.sort_unstable();
        collected.dedup();
        Self {
            kind: AttributeSelectionKind::Only(collected),
        }
    }

    /// Returns explicit identities for a bounded exact selection.
    ///
    /// Returns `None` for an all-Attributes request or an oversized request
    /// that [`Source::read`] will reject.
    #[must_use]
    pub fn explicit(&self) -> Option<&[AttributeId]> {
        match &self.kind {
            AttributeSelectionKind::Only(attributes) => Some(attributes),
            AttributeSelectionKind::All | AttributeSelectionKind::TooManyInputAttributes { .. } => {
                None
            }
        }
    }

    fn resolved(attributes: Vec<AttributeId>) -> Self {
        Self {
            kind: AttributeSelectionKind::Only(attributes),
        }
    }
}

/// Hard limits covering one read and each Point Batch it returns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct ReadBudget {
    max_spans: u64,
    max_points: u64,
    max_batch_points: u64,
    max_batch_payload_bytes: u64,
    max_adapter_working_bytes: u64,
}

impl ReadBudget {
    /// Creates nonzero per-batch Point and payload limits.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidBudget`] when either limit is zero.
    pub const fn new(
        max_batch_points: u64,
        max_batch_payload_bytes: u64,
    ) -> Result<Self, SourceError> {
        if max_batch_points == 0 {
            return Err(SourceError::InvalidBudget {
                limit: "max_batch_points",
            });
        }
        if max_batch_payload_bytes == 0 {
            return Err(SourceError::InvalidBudget {
                limit: "max_batch_payload_bytes",
            });
        }
        Ok(Self {
            max_spans: DEFAULT_MAX_SPANS,
            max_points: u64::MAX,
            max_batch_points,
            max_batch_payload_bytes,
            max_adapter_working_bytes: DEFAULT_ADAPTER_WORKING_BYTES,
        })
    }

    /// Sets the maximum normalized spans in one request.
    ///
    /// Zero permits only a read whose normalized span selection is empty.
    #[must_use]
    pub const fn with_max_spans(mut self, max_spans: u64) -> Self {
        self.max_spans = max_spans;
        self
    }

    /// Sets the maximum exact Points in the normalized request.
    ///
    /// Zero permits only an empty read.
    #[must_use]
    pub const fn with_max_points(mut self, max_points: u64) -> Self {
        self.max_points = max_points;
        self
    }

    /// Sets the adapter's maximum separate decoder working memory.
    ///
    /// Zero is valid for adapters that need no separate decoding block.
    #[must_use]
    pub const fn with_max_adapter_working_bytes(mut self, bytes: u64) -> Self {
        self.max_adapter_working_bytes = bytes;
        self
    }

    /// Returns the maximum normalized Source spans.
    #[must_use]
    pub const fn max_spans(self) -> u64 {
        self.max_spans
    }

    /// Returns the maximum exact Points requested across all batches.
    #[must_use]
    pub const fn max_points(self) -> u64 {
        self.max_points
    }

    /// Returns the maximum Points in one emitted batch.
    #[must_use]
    pub const fn max_batch_points(self) -> u64 {
        self.max_batch_points
    }

    /// Returns the maximum canonical payload bytes in one emitted batch.
    #[must_use]
    pub const fn max_batch_payload_bytes(self) -> u64 {
        self.max_batch_payload_bytes
    }

    /// Returns the adapter's separate decoder working-memory limit.
    #[must_use]
    pub const fn max_adapter_working_bytes(self) -> u64 {
        self.max_adapter_working_bytes
    }
}

impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            max_spans: DEFAULT_MAX_SPANS,
            max_points: u64::MAX,
            max_batch_points: DEFAULT_BATCH_POINTS,
            max_batch_payload_bytes: DEFAULT_BATCH_BYTES,
            max_adapter_working_bytes: DEFAULT_ADAPTER_WORKING_BYTES,
        }
    }
}

/// Validated intent for one bounded Source read.
#[derive(Clone, Debug)]
pub struct ReadRequest {
    spans: SpanSelection,
    attributes: AttributeSelection,
    budget: ReadBudget,
}

#[derive(Clone, Debug)]
enum SpanSelection {
    All,
    Spans(Vec<SourceSpan>),
    TooManyInputSpans { at_least: u64 },
}

impl ReadRequest {
    /// Selects every Point and every declared Attribute.
    #[must_use]
    pub fn all() -> Self {
        Self {
            spans: SpanSelection::All,
            attributes: AttributeSelection::all(),
            budget: ReadBudget::default(),
        }
    }

    /// Replaces the Point selection with ordinal spans.
    ///
    /// Collection is capped at [`MAX_INPUT_SOURCE_SPANS`]. A larger iterator is
    /// retained as an invalid request and later rejected by [`Source::read`].
    #[must_use]
    pub fn spans(mut self, spans: impl IntoIterator<Item = SourceSpan>) -> Self {
        let mut spans = spans.into_iter();
        let lower_bound = spans.size_hint().0;
        if lower_bound > MAX_INPUT_SOURCE_SPANS {
            self.spans = SpanSelection::TooManyInputSpans {
                at_least: u64::try_from(lower_bound).unwrap_or(u64::MAX),
            };
            return self;
        }

        let mut collected = Vec::with_capacity(lower_bound.min(MAX_INPUT_SOURCE_SPANS));
        collected.extend(spans.by_ref().take(MAX_INPUT_SOURCE_SPANS));
        self.spans = if spans.next().is_some() {
            SpanSelection::TooManyInputSpans {
                at_least: u64::try_from(MAX_INPUT_SOURCE_SPANS)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            }
        } else {
            SpanSelection::Spans(collected)
        };
        self
    }

    /// Replaces the Attribute selection.
    #[must_use]
    pub fn attributes(mut self, attributes: AttributeSelection) -> Self {
        self.attributes = attributes;
        self
    }

    /// Replaces the hard whole-read and per-batch budget.
    #[must_use]
    pub const fn budget(mut self, budget: ReadBudget) -> Self {
        self.budget = budget;
        self
    }
}

impl Default for ReadRequest {
    fn default() -> Self {
        Self::all()
    }
}

fn open_source(
    adapter: &dyn CandidateAdapter,
    options: OpenOptions,
    control: &OperationControl,
) -> Result<Source, SourceError> {
    control.check_cancelled()?;
    let (verified, expected) = verify_adapter(adapter, options, control)?;
    control.check_cancelled()?;
    validate_adapter_identity(&verified)?;
    let source_id = derive_source_id(&verified);

    if let Some(record) = expected.as_ref() {
        validate_record(record, source_id, &verified)?;
    }

    let source = publish_source(source_id, verified)?;
    control.check_cancelled()?;
    publish_complete(control, None)?;
    Ok(source)
}

fn verify_adapter(
    adapter: &dyn CandidateAdapter,
    options: OpenOptions,
    control: &OperationControl,
) -> Result<(AdapterVerified, Option<SourceRecord>), SourceError> {
    match options.expectation {
        OpenExpectation::Identify => Ok((
            adapter.full_verify(FullVerification::Identify, &control.reporter())?,
            None,
        )),
        OpenExpectation::Match { record, policy } => {
            if record.version != SOURCE_RECORD_VERSION {
                return Err(SourceError::UnsupportedRecordVersion {
                    version: record.version,
                });
            }
            let verified = match policy {
                VerificationPolicy::FastOnly => try_fast_match(adapter, &record, control)?
                    .ok_or(SourceError::VerificationRequired)?,
                VerificationPolicy::Full => adapter.full_verify(
                    FullVerification::Match {
                        expected_content_hash: record.content_hash(),
                    },
                    &control.reporter(),
                )?,
                VerificationPolicy::FastThenFull => {
                    if let Some(verified) = try_fast_match(adapter, &record, control)? {
                        verified
                    } else {
                        control.check_cancelled()?;
                        adapter.full_verify(
                            FullVerification::Match {
                                expected_content_hash: record.content_hash(),
                            },
                            &control.reporter(),
                        )?
                    }
                }
            };
            Ok((verified, Some(*record)))
        }
    }
}

fn try_fast_match(
    adapter: &dyn CandidateAdapter,
    record: &SourceRecord,
    control: &OperationControl,
) -> Result<Option<AdapterVerified>, SourceError> {
    let verified = match adapter.fast_verify(record.fast_token(), &control.reporter()) {
        Ok(verified) => verified,
        Err(
            SourceError::VerificationRequired
            | SourceError::SourceChanged { .. }
            | SourceError::SourceContractMismatch { .. },
        ) => return Ok(None),
        Err(error) => return Err(error),
    };
    validate_adapter_identity(&verified)?;
    let source = derive_source_id(&verified);
    match validate_record(record, source, &verified) {
        Ok(()) => Ok(Some(verified)),
        Err(SourceError::SourceChanged { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn validate_adapter_identity(verified: &AdapterVerified) -> Result<(), SourceError> {
    for (name, value, max_bytes) in [
        (
            "adapter name",
            verified.adapter_name(),
            MAX_ADAPTER_NAME_BYTES,
        ),
        (
            "adapter version",
            verified.adapter_version(),
            MAX_ADAPTER_VERSION_BYTES,
        ),
        (
            "logical order",
            verified.logical_order(),
            MAX_LOGICAL_ORDER_BYTES,
        ),
    ] {
        if value.trim().is_empty() {
            return Err(SourceError::contract(format!("{name} is empty")));
        }
        if value.len() > max_bytes {
            return Err(SourceError::contract(format!(
                "{name} exceeds its {max_bytes}-byte limit"
            )));
        }
    }
    if verified.fast_token().len() > MAX_FAST_TOKEN_BYTES {
        return Err(SourceError::contract(format!(
            "Fast evidence exceeds its {MAX_FAST_TOKEN_BYTES}-byte limit"
        )));
    }
    Ok(())
}

fn validate_record(
    record: &SourceRecord,
    derived_source: SourceId,
    verified: &AdapterVerified,
) -> Result<(), SourceError> {
    if verified.content_hash() != record.content_hash {
        return Err(SourceError::changed(
            "content fingerprint differs from the record",
        ));
    }
    if verified.adapter_name() != record.adapter_name
        || verified.adapter_version() != record.adapter_version
        || verified.logical_order() != record.logical_order
    {
        return Err(SourceError::changed(
            "adapter name, version, or logical-order rule differs from the record",
        ));
    }
    if verified.metadata() != record.metadata.as_ref() {
        return Err(SourceError::changed(
            "canonical metadata or Attribute schema differs from the record",
        ));
    }
    if derived_source != record.source {
        return Err(SourceError::changed(
            "content identity differs from the record",
        ));
    }
    Ok(())
}

fn derive_source_id(verified: &AdapterVerified) -> SourceId {
    let mut hasher = Hasher::new();
    hasher.update(SOURCE_ID_DOMAIN);
    hash_text(&mut hasher, verified.adapter_name());
    hash_text(&mut hasher, verified.adapter_version());
    hash_text(&mut hasher, verified.logical_order());
    hasher.update(verified.content_hash().as_bytes());
    SourceId::new(*hasher.finalize().as_bytes())
}

fn hash_text(hasher: &mut Hasher, value: &str) {
    let length = u64::try_from(value.len()).expect("string length fits u64");
    hasher.update(&length.to_le_bytes());
    hasher.update(value.as_bytes());
}

fn publish_source(source: SourceId, verified: AdapterVerified) -> Result<Source, SourceError> {
    let parts = verified.into_parts();
    let provenance = SourceProvenance::new(
        source,
        parts.content_hash,
        parts.logical_order.clone(),
        SOURCE_CONTRACT_VERSION,
    )
    .map_err(|error| SourceError::contract(error.to_string()))?;
    let record = SourceRecord {
        version: SOURCE_RECORD_VERSION,
        source,
        content_hash: parts.content_hash,
        adapter_name: parts.adapter_name,
        adapter_version: parts.adapter_version,
        logical_order: parts.logical_order,
        metadata: Arc::clone(&parts.metadata),
        fast_token: parts.fast_token,
    };

    Ok(Source {
        inner: Arc::new(SourceInner {
            identity: source,
            provenance,
            record,
            reader: parts.reader,
        }),
    })
}

fn publish_complete(
    control: &OperationControl,
    exact_total: Option<u64>,
) -> Result<(), SourceError> {
    let current = control.progress();
    let total = exact_total
        .or(current.total_units())
        .unwrap_or_else(|| current.completed_units());
    control.complete_progress(total).map_err(SourceError::from)
}

pub(crate) struct NormalizedRead {
    pub(crate) spans: Arc<[SourceSpan]>,
    pub(crate) expected_attributes: Vec<AttributeId>,
    pub(crate) attributes: AttributeSelection,
    pub(crate) budget: ReadBudget,
    pub(crate) exact_count: u64,
}

fn normalize_request(
    metadata: &SourceMetadata,
    request: ReadRequest,
) -> Result<NormalizedRead, SourceError> {
    let spans = normalize_spans(request.spans, metadata.point_count(), request.budget)?;
    let exact_count = spans.iter().try_fold(0_u64, |count, span| {
        count
            .checked_add(span.point_count())
            .ok_or(SourceError::ResourceLimit {
                limit: "requested Point count",
                required: u64::MAX,
                allowed: request.budget.max_points(),
            })
    })?;
    if exact_count > request.budget.max_points() {
        return Err(SourceError::ResourceLimit {
            limit: "requested Point count",
            required: exact_count,
            allowed: request.budget.max_points(),
        });
    }
    let expected_attributes = resolve_attributes(metadata, &request.attributes)?;
    let attributes = AttributeSelection::resolved(expected_attributes.clone());
    Ok(NormalizedRead {
        spans: Arc::from(spans),
        expected_attributes,
        attributes,
        budget: request.budget,
        exact_count,
    })
}

fn normalize_spans(
    selection: SpanSelection,
    point_count: u64,
    budget: ReadBudget,
) -> Result<Vec<SourceSpan>, SourceError> {
    let mut spans = match selection {
        SpanSelection::All if point_count == 0 => Vec::new(),
        SpanSelection::All => vec![SourceSpan::new(0, point_count)?],
        SpanSelection::Spans(spans) => spans,
        SpanSelection::TooManyInputSpans { at_least } => {
            return Err(SourceError::ResourceLimit {
                limit: "input Source spans",
                required: at_least,
                allowed: u64::try_from(MAX_INPUT_SOURCE_SPANS).unwrap_or(u64::MAX),
            });
        }
    };
    for span in &spans {
        if span.end_ordinal() > point_count {
            return Err(SourceError::InvalidSourceSpan {
                first_ordinal: span.first_ordinal(),
                point_count: span.point_count(),
            });
        }
    }

    spans.sort_unstable_by_key(|span| span.first_ordinal());
    let mut normalized: Vec<SourceSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(previous) = normalized.last_mut()
            && span.first_ordinal() <= previous.end_ordinal()
        {
            let end = previous.end_ordinal().max(span.end_ordinal());
            previous.point_count = end - previous.first_ordinal;
            continue;
        }
        normalized.push(span);
    }
    let normalized_count = u64::try_from(normalized.len()).unwrap_or(u64::MAX);
    if normalized_count > budget.max_spans() {
        return Err(SourceError::ResourceLimit {
            limit: "normalized Source spans",
            required: normalized_count,
            allowed: budget.max_spans(),
        });
    }
    Ok(normalized)
}

fn resolve_attributes(
    metadata: &SourceMetadata,
    selection: &AttributeSelection,
) -> Result<Vec<AttributeId>, SourceError> {
    match &selection.kind {
        AttributeSelectionKind::All => Ok(metadata
            .attributes()
            .definitions()
            .iter()
            .map(point_contracts::AttributeDefinition::id)
            .collect()),
        AttributeSelectionKind::Only(attributes) => {
            let mut attributes = attributes.clone();
            attributes.sort_unstable();
            attributes.dedup();
            for &attribute in &attributes {
                if metadata.attributes().get(attribute).is_none() {
                    return Err(SourceError::UnknownAttribute { attribute });
                }
            }
            Ok(attributes)
        }
        AttributeSelectionKind::TooManyInputAttributes { at_least } => {
            Err(SourceError::ResourceLimit {
                limit: "input Attribute identities",
                required: *at_least,
                allowed: u64::try_from(MAX_INPUT_ATTRIBUTE_IDS).unwrap_or(u64::MAX),
            })
        }
    }
}
