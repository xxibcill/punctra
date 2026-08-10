//! Deterministic in-memory adapter for verified Punctra Sources.
//!
//! The adapter owns one immutable columnar input and exposes it through the
//! same verified, bounded [`point_source::Source`] interface used by file
//! adapters. The opt-in `test-support` feature exposes deterministic fault
//! control for conformance tests without adding faults to the production
//! Source interface.
//!
//! ```
//! use point_contracts::{
//!     AttributeColumns, CoordinateReference, PositionTransform,
//! };
//! use source_memory::{MemorySource, open};
//!
//! let input = MemorySource::from_columns(
//!     PositionTransform::new([0.0; 3], [0.001; 3])?,
//!     CoordinateReference::Unknown,
//!     Vec::new(),
//!     AttributeColumns::empty(0),
//! )?;
//! let source = open(input).blocking_wait()?;
//! assert_eq!(source.metadata().point_count(), 0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use blake3::Hasher;
use foundation_runtime::{OperationReporter, ProgressPhase, ProgressSnapshot};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeId, AttributeSchema,
    AttributeValues, ContentHash, ContractError, CoordinateReference, PointBatch,
    PositionTransform, QuantizedPositions, SourceMetadata, WorldBounds,
};
use point_source::adapter::{
    AdapterRead, AdapterReadRequest, AdapterVerified, CandidateAdapter, FullVerification,
    ReadAdapter,
};
use point_source::{
    AttributeSelection, OpenOptions, ReadBudget, SourceCandidate, SourceError, SourceJob,
    SourcePreview,
};
use thiserror::Error;

const ADAPTER_NAME: &str = "source-memory";
const ADAPTER_VERSION: &str = "1";
const LOGICAL_ORDER: &str = "immutable input row order v1";
const HASH_DOMAIN: &[u8] = b"punctra-memory-source-v1";
const NO_READ_FAULT: u64 = u64::MAX;
const HASH_ROW_QUANTUM: usize = 4_096;
const HASH_VALUE_QUANTUM: usize = 4_096;
const HASH_BYTE_QUANTUM: usize = 64 * 1_024;

/// Validated immutable columnar input for one in-memory Source.
#[derive(Clone)]
pub struct MemorySource {
    state: Arc<MemoryState>,
    preview: SourcePreview,
}

impl MemorySource {
    /// Creates input from explicit canonical metadata and columns.
    ///
    /// # Errors
    ///
    /// Returns an error when the Point count, Attribute schema, row counts, or
    /// finite bounds do not describe the supplied position ticks exactly.
    pub fn new(
        metadata: SourceMetadata,
        ticks: Vec<[i64; 3]>,
        attributes: AttributeColumns,
    ) -> Result<Self, MemoryError> {
        Self::build(metadata, ticks, attributes)
    }

    /// Creates canonical memory metadata from exact position and Attribute columns.
    ///
    /// The format name is `memory`, Coordinate Reference is explicit, and
    /// finite bounds are derived from the supplied ticks.
    ///
    /// # Errors
    ///
    /// Returns an error for mismatched rows, invalid bounds, or a Source-scale
    /// count that cannot be represented.
    pub fn from_columns(
        transform: PositionTransform,
        coordinate_reference: CoordinateReference,
        ticks: Vec<[i64; 3]>,
        attributes: AttributeColumns,
    ) -> Result<Self, MemoryError> {
        let metadata = inferred_metadata(transform, coordinate_reference, &ticks, &attributes)?;
        Self::new(metadata, ticks, attributes)
    }

    /// Creates input plus deterministic change and corruption controls.
    ///
    /// This constructor is intended for conformance and fault tests. The
    /// returned control does not exist on the caller-facing Source interface.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`MemorySource::new`].
    #[cfg(feature = "test-support")]
    pub fn with_fault_control(
        metadata: SourceMetadata,
        ticks: Vec<[i64; 3]>,
        attributes: AttributeColumns,
    ) -> Result<(Self, MemoryFaultControl), MemoryError> {
        let source = Self::build(metadata, ticks, attributes)?;
        let control = MemoryFaultControl {
            state: Arc::clone(&source.state),
        };
        Ok((source, control))
    }

    /// Wraps this input as an unverified Source candidate.
    #[must_use]
    pub fn candidate(self) -> SourceCandidate {
        SourceCandidate::new_adapter(MemoryCandidate { source: self })
    }

    fn build(
        metadata: SourceMetadata,
        ticks: Vec<[i64; 3]>,
        attributes: AttributeColumns,
    ) -> Result<Self, MemoryError> {
        validate_input(&metadata, &ticks, &attributes)?;
        let preview = SourcePreview::new(metadata.format_name(), None);
        let state = Arc::new(MemoryState {
            metadata: Arc::new(metadata),
            ticks: ticks.into_boxed_slice(),
            attributes,
            epoch: AtomicU64::new(0),
            corrupt_ordinal: AtomicU64::new(NO_READ_FAULT),
            verified_hash: Mutex::new(None),
        });
        Ok(Self { state, preview })
    }
}

impl std::fmt::Debug for MemorySource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemorySource")
            .field("metadata", &self.state.metadata)
            .finish_non_exhaustive()
    }
}

/// Test-only control for deterministic Source change and read faults.
#[cfg(feature = "test-support")]
#[derive(Clone, Debug)]
pub struct MemoryFaultControl {
    state: Arc<MemoryState>,
}

#[cfg(feature = "test-support")]
impl MemoryFaultControl {
    /// Makes existing verification records and open readers observe a changed Source.
    pub fn mark_changed(&self) {
        self.state.epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// Makes a read fail when it reaches the given logical Point ordinal.
    pub fn fail_at_ordinal(&self, ordinal: u64) {
        self.state.corrupt_ordinal.store(ordinal, Ordering::Release);
    }

    /// Removes an injected ordinal read fault.
    pub fn clear_read_fault(&self) {
        self.state
            .corrupt_ordinal
            .store(NO_READ_FAULT, Ordering::Release);
    }
}

/// Opens a new in-memory Source through mandatory Full verification.
#[must_use]
pub fn open(input: MemorySource) -> SourceJob {
    open_with(input, OpenOptions::identify())
}

/// Opens an in-memory Source using explicit identify or reopen options.
#[must_use]
pub fn open_with(input: MemorySource, options: OpenOptions) -> SourceJob {
    input.candidate().open(options)
}

struct MemoryState {
    metadata: Arc<SourceMetadata>,
    ticks: Box<[[i64; 3]]>,
    attributes: AttributeColumns,
    epoch: AtomicU64,
    corrupt_ordinal: AtomicU64,
    verified_hash: Mutex<Option<(u64, ContentHash)>>,
}

impl std::fmt::Debug for MemoryState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryState")
            .field("point_count", &self.metadata.point_count())
            .field("epoch", &self.epoch.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

struct MemoryCandidate {
    source: MemorySource,
}

impl CandidateAdapter for MemoryCandidate {
    fn preview(&self) -> &SourcePreview {
        &self.source.preview
    }

    fn full_verify(
        &self,
        verification: FullVerification,
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        reporter.check_cancelled()?;
        let epoch = self.source.state.epoch.load(Ordering::Acquire);
        let content_hash = hash_content(&self.source.state, epoch, reporter)?;
        if self.source.state.epoch.load(Ordering::Acquire) != epoch {
            return Err(SourceError::SourceChanged {
                reason: "in-memory content changed during Full verification".into(),
            });
        }
        if verification
            .expected_content_hash()
            .is_some_and(|expected| expected != content_hash)
        {
            return Err(SourceError::changed(
                "in-memory content hash differs from recorded Full evidence",
            ));
        }
        *lock_recovering(&self.source.state.verified_hash) = Some((epoch, content_hash));
        Ok(verified(&self.source.state, epoch, content_hash))
    }

    fn fast_verify(
        &self,
        expected_fast_token: &[u8],
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        reporter.check_cancelled()?;
        let epoch = self.source.state.epoch.load(Ordering::Acquire);
        if expected_fast_token != epoch.to_le_bytes() {
            return Err(SourceError::VerificationRequired);
        }
        let content_hash = lock_recovering(&self.source.state.verified_hash)
            .filter(|(verified_epoch, _)| *verified_epoch == epoch)
            .map(|(_, hash)| hash)
            .ok_or(SourceError::VerificationRequired)?;
        if self.source.state.epoch.load(Ordering::Acquire) != epoch {
            return Err(SourceError::VerificationRequired);
        }
        Ok(verified(&self.source.state, epoch, content_hash))
    }
}

fn verified(state: &Arc<MemoryState>, epoch: u64, content_hash: ContentHash) -> AdapterVerified {
    AdapterVerified::new(
        ADAPTER_NAME,
        ADAPTER_VERSION,
        LOGICAL_ORDER,
        Arc::clone(&state.metadata),
        content_hash,
        epoch.to_le_bytes().to_vec(),
        Arc::new(MemoryReadAdapter {
            state: Arc::clone(state),
            verified_epoch: epoch,
        }),
    )
}

struct MemoryReadAdapter {
    state: Arc<MemoryState>,
    verified_epoch: u64,
}

impl ReadAdapter for MemoryReadAdapter {
    fn start_read(
        &self,
        request: AdapterReadRequest,
        source: point_contracts::SourceId,
        reporter: OperationReporter,
    ) -> Result<Box<dyn AdapterRead>, SourceError> {
        ensure_unchanged(&self.state, self.verified_epoch)?;
        let selected_attributes = selected_attributes(&self.state.metadata, request.attributes());
        Ok(Box::new(MemoryRead {
            state: Arc::clone(&self.state),
            verified_epoch: self.verified_epoch,
            spans: request.spans().to_vec(),
            selected_attributes,
            budget: request.budget(),
            source,
            reporter,
            span_index: 0,
            next_ordinal: request
                .spans()
                .first()
                .map_or(0, |span| span.first_ordinal()),
            terminal: false,
        }))
    }
}

struct MemoryRead {
    state: Arc<MemoryState>,
    verified_epoch: u64,
    spans: Vec<point_source::SourceSpan>,
    selected_attributes: Vec<AttributeId>,
    budget: ReadBudget,
    source: point_contracts::SourceId,
    reporter: OperationReporter,
    span_index: usize,
    next_ordinal: u64,
    terminal: bool,
}

impl AdapterRead for MemoryRead {
    fn next(&mut self) -> Result<Option<PointBatch>, SourceError> {
        if self.terminal {
            return Ok(None);
        }
        self.reporter.check_cancelled()?;
        ensure_unchanged(&self.state, self.verified_epoch)?;
        let Some(span) = self.spans.get(self.span_index).copied() else {
            self.terminal = true;
            return Ok(None);
        };
        let corrupt_ordinal = self.state.corrupt_ordinal.load(Ordering::Acquire);
        if corrupt_ordinal == self.next_ordinal {
            self.terminal = true;
            return Err(SourceError::corrupt(format!(
                "injected corrupt Point at ordinal {corrupt_ordinal}"
            )));
        }

        let remaining = span.end_ordinal() - self.next_ordinal;
        let mut point_count = batch_point_count(
            remaining,
            self.budget,
            &self.state.metadata,
            &self.selected_attributes,
        )?;
        if corrupt_ordinal > self.next_ordinal && corrupt_ordinal < self.next_ordinal + point_count
        {
            point_count = corrupt_ordinal - self.next_ordinal;
        }
        let batch = self.make_batch(point_count)?;
        self.reporter.check_cancelled()?;
        ensure_unchanged(&self.state, self.verified_epoch)?;
        self.advance(point_count, span.end_ordinal());
        ensure_unchanged(&self.state, self.verified_epoch)?;
        Ok(Some(batch))
    }
}

impl MemoryRead {
    fn make_batch(&self, point_count: u64) -> Result<PointBatch, SourceError> {
        let start = usize::try_from(self.next_ordinal).map_err(|_| {
            SourceError::adapter("memory Source ordinal does not fit the host address space")
        })?;
        let count = usize::try_from(point_count)
            .map_err(|_| SourceError::adapter("memory batch count does not fit usize"))?;
        let end = start
            .checked_add(count)
            .ok_or_else(|| SourceError::adapter("memory batch row range overflow"))?;
        let positions = QuantizedPositions::new(
            self.state.metadata.position_transform(),
            self.state.ticks[start..end].to_vec(),
        )
        .map_err(contract_as_source)?;
        let attributes = projected_columns(
            &self.state.attributes,
            &self.selected_attributes,
            start..end,
        )?;
        PointBatch::new(self.source, self.next_ordinal, positions, attributes)
            .map_err(contract_as_source)
    }

    fn advance(&mut self, point_count: u64, span_end: u64) {
        self.next_ordinal += point_count;
        if self.next_ordinal == span_end {
            self.span_index += 1;
            if let Some(next_span) = self.spans.get(self.span_index) {
                self.next_ordinal = next_span.first_ordinal();
            }
        }
    }
}

fn selected_attributes(
    _metadata: &SourceMetadata,
    selection: &AttributeSelection,
) -> Vec<AttributeId> {
    selection
        .explicit()
        .expect("point-source resolves every adapter Attribute selection")
        .to_vec()
}

fn projected_columns(
    columns: &AttributeColumns,
    selected: &[AttributeId],
    rows: std::ops::Range<usize>,
) -> Result<AttributeColumns, SourceError> {
    let projected = selected
        .iter()
        .map(|&id| {
            let column = columns
                .get(id)
                .ok_or_else(|| SourceError::adapter("verified Attribute column is missing"))?;
            AttributeColumn::new(
                column.definition().clone(),
                column
                    .values()
                    .slice_rows(rows.clone())
                    .map_err(contract_as_source)?,
            )
            .map_err(contract_as_source)
        })
        .collect::<Result<Vec<_>, SourceError>>()?;
    AttributeColumns::new(projected, rows.end - rows.start).map_err(contract_as_source)
}

fn batch_point_count(
    remaining: u64,
    budget: ReadBudget,
    metadata: &SourceMetadata,
    selected: &[AttributeId],
) -> Result<u64, SourceError> {
    let bytes_per_point = selected.iter().try_fold(24_u64, |total, &id| {
        let definition = metadata.attributes().get(id).ok_or_else(|| {
            SourceError::unsupported_schema(format!(
                "Source does not contain requested Attribute {id:?}"
            ))
        })?;
        total
            .checked_add(u64::from(definition.data_type().element_bytes()))
            .ok_or(SourceError::ResourceLimit {
                limit: "Point payload bytes",
                required: u64::MAX,
                allowed: budget.max_batch_payload_bytes(),
            })
    })?;
    let points_by_bytes = budget.max_batch_payload_bytes() / bytes_per_point;
    if points_by_bytes == 0 {
        return Err(SourceError::ResourceLimit {
            limit: "batch payload bytes",
            required: bytes_per_point,
            allowed: budget.max_batch_payload_bytes(),
        });
    }
    Ok(remaining
        .min(budget.max_batch_points())
        .min(points_by_bytes))
}

fn ensure_unchanged(state: &MemoryState, verified_epoch: u64) -> Result<(), SourceError> {
    if state.epoch.load(Ordering::Acquire) == verified_epoch {
        Ok(())
    } else {
        Err(SourceError::SourceChanged {
            reason: "in-memory content changed after verification".into(),
        })
    }
}

fn inferred_metadata(
    transform: PositionTransform,
    coordinate_reference: CoordinateReference,
    ticks: &[[i64; 3]],
    attributes: &AttributeColumns,
) -> Result<SourceMetadata, MemoryError> {
    let point_count = u64::try_from(ticks.len()).map_err(|_| MemoryError::PointCountOverflow)?;
    let schema = AttributeSchema::new(
        attributes
            .columns()
            .iter()
            .map(|column| column.definition().clone())
            .collect(),
    )?;
    SourceMetadata::new(
        point_count,
        transform,
        coordinate_reference,
        schema,
        bounds_for_ticks(transform, ticks)?,
        "memory",
        Vec::new(),
    )
    .map_err(MemoryError::from)
}

fn validate_input(
    metadata: &SourceMetadata,
    ticks: &[[i64; 3]],
    attributes: &AttributeColumns,
) -> Result<(), MemoryError> {
    let actual_count = u64::try_from(ticks.len()).map_err(|_| MemoryError::PointCountOverflow)?;
    if metadata.point_count() != actual_count {
        return Err(MemoryError::PointCountMismatch {
            metadata: metadata.point_count(),
            actual: actual_count,
        });
    }
    if attributes.row_count() != ticks.len() {
        return Err(MemoryError::AttributeRowCountMismatch {
            positions: ticks.len(),
            attributes: attributes.row_count(),
        });
    }
    let actual_schema = AttributeSchema::new(
        attributes
            .columns()
            .iter()
            .map(|column| column.definition().clone())
            .collect(),
    )?;
    if metadata.attributes() != &actual_schema {
        return Err(MemoryError::AttributeSchemaMismatch);
    }
    let actual_bounds = bounds_for_ticks(metadata.position_transform(), ticks)?;
    if metadata.world_bounds() != actual_bounds {
        return Err(MemoryError::WorldBoundsMismatch);
    }
    Ok(())
}

fn bounds_for_ticks(
    transform: PositionTransform,
    ticks: &[[i64; 3]],
) -> Result<Option<WorldBounds>, MemoryError> {
    let Some(first) = ticks.first().copied() else {
        return Ok(None);
    };
    let first = transform.world_f64(first);
    let mut min = first;
    let mut max = first;
    for ticks in &ticks[1..] {
        let world = transform.world_f64(*ticks);
        for axis in 0..3 {
            min[axis] = min[axis].min(world[axis]);
            max[axis] = max[axis].max(world[axis]);
        }
    }
    WorldBounds::new(min, max)
        .map(Some)
        .map_err(MemoryError::from)
}

fn hash_content(
    state: &MemoryState,
    epoch: u64,
    reporter: &OperationReporter,
) -> Result<ContentHash, SourceError> {
    let mut hasher = Hasher::new();
    hasher.update(HASH_DOMAIN);
    hash_metadata(&mut hasher, &state.metadata, reporter)?;
    hash_len(&mut hasher, state.ticks.len())?;
    for (row, ticks) in state.ticks.iter().enumerate() {
        if row % HASH_ROW_QUANTUM == 0 {
            reporter.check_cancelled()?;
            report_progress(
                reporter,
                ProgressPhase::RUNNING,
                u64::try_from(row).unwrap_or(u64::MAX),
                state.metadata.point_count(),
            )?;
        }
        for tick in ticks {
            hasher.update(&tick.to_le_bytes());
        }
    }
    hash_columns(&mut hasher, &state.attributes, reporter)?;
    hasher.update(&epoch.to_le_bytes());
    report_progress(
        reporter,
        ProgressPhase::RUNNING,
        state.metadata.point_count(),
        state.metadata.point_count(),
    )?;
    Ok(ContentHash::new(*hasher.finalize().as_bytes()))
}

fn hash_metadata(
    hasher: &mut Hasher,
    metadata: &SourceMetadata,
    reporter: &OperationReporter,
) -> Result<(), SourceError> {
    reporter.check_cancelled()?;
    hasher.update(&metadata.point_count().to_le_bytes());
    for value in metadata.position_transform().offset() {
        hasher.update(&value.to_bits().to_le_bytes());
    }
    for value in metadata.position_transform().scale() {
        hasher.update(&value.to_bits().to_le_bytes());
    }
    hash_bytes(hasher, metadata.format_name().as_bytes(), reporter)?;
    match metadata.coordinate_reference().as_wkt() {
        Some(wkt) => {
            hasher.update(&[1]);
            hash_bytes(hasher, wkt.as_bytes(), reporter)?;
        }
        None => {
            hasher.update(&[0]);
        }
    }
    match metadata.world_bounds() {
        Some(bounds) => {
            hasher.update(&[1]);
            for value in bounds.min().into_iter().chain(bounds.max()) {
                hasher.update(&value.to_bits().to_le_bytes());
            }
        }
        None => {
            hasher.update(&[0]);
        }
    }
    hash_len(hasher, metadata.attributes().definitions().len())?;
    for definition in metadata.attributes().definitions() {
        reporter.check_cancelled()?;
        hasher.update(&definition.id().get().to_le_bytes());
        hash_bytes(hasher, definition.name().as_bytes(), reporter)?;
        hash_attribute_type(hasher, definition.data_type());
    }
    hash_len(hasher, metadata.metadata_records().len())?;
    for record in metadata.metadata_records() {
        reporter.check_cancelled()?;
        hash_bytes(hasher, record.namespace().as_bytes(), reporter)?;
        hash_bytes(hasher, record.name().as_bytes(), reporter)?;
        hash_bytes(hasher, record.payload(), reporter)?;
    }
    Ok(())
}

fn hash_columns(
    hasher: &mut Hasher,
    columns: &AttributeColumns,
    reporter: &OperationReporter,
) -> Result<(), SourceError> {
    hash_len(hasher, columns.columns().len())?;
    for column in columns.columns() {
        reporter.check_cancelled()?;
        hasher.update(&column.id().get().to_le_bytes());
        hash_values(hasher, column.values(), reporter)?;
    }
    Ok(())
}

fn hash_values(
    hasher: &mut Hasher,
    values: &AttributeValues,
    reporter: &OperationReporter,
) -> Result<(), SourceError> {
    hash_attribute_type(hasher, values.data_type());
    hash_len(hasher, values.len())?;
    match values.data_type() {
        AttributeDataType::I8 => {
            hash_numeric(
                hasher,
                values.as_i8().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::U8 => {
            hash_raw_bytes(
                hasher,
                values.as_u8().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::I16 => {
            hash_numeric(
                hasher,
                values.as_i16().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::U16 => {
            hash_numeric(
                hasher,
                values.as_u16().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::I32 => {
            hash_numeric(
                hasher,
                values.as_i32().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::U32 => {
            hash_numeric(
                hasher,
                values.as_u32().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::I64 => {
            hash_numeric(
                hasher,
                values.as_i64().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::U64 => {
            hash_numeric(
                hasher,
                values.as_u64().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::F32 => {
            hash_numeric(
                hasher,
                values.as_f32().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::F64 => {
            hash_numeric(
                hasher,
                values.as_f64().expect("type-matched values"),
                reporter,
            )?;
        }
        AttributeDataType::FixedBytes(_) => {
            let (_, payload) = values.as_fixed_bytes().expect("type-matched values");
            hash_raw_bytes(hasher, payload, reporter)?;
        }
    }
    Ok(())
}

trait LittleEndianBytes {
    fn update_hash(self, hasher: &mut Hasher);
}

macro_rules! impl_little_endian_bytes {
    ($($type:ty),+ $(,)?) => {$(
        impl LittleEndianBytes for $type {
            fn update_hash(self, hasher: &mut Hasher) {
                hasher.update(&self.to_le_bytes());
            }
        }
    )+};
}

impl_little_endian_bytes!(i8, i16, u16, i32, u32, i64, u64);

impl LittleEndianBytes for f32 {
    fn update_hash(self, hasher: &mut Hasher) {
        hasher.update(&self.to_bits().to_le_bytes());
    }
}

impl LittleEndianBytes for f64 {
    fn update_hash(self, hasher: &mut Hasher) {
        hasher.update(&self.to_bits().to_le_bytes());
    }
}

fn hash_numeric<T: Copy + LittleEndianBytes>(
    hasher: &mut Hasher,
    values: &[T],
    reporter: &OperationReporter,
) -> Result<(), SourceError> {
    for (index, value) in values.iter().copied().enumerate() {
        if index % HASH_VALUE_QUANTUM == 0 {
            reporter.check_cancelled()?;
        }
        value.update_hash(hasher);
    }
    Ok(())
}

fn hash_attribute_type(hasher: &mut Hasher, data_type: AttributeDataType) {
    let (tag, width) = match data_type {
        AttributeDataType::I8 => (0_u8, 1),
        AttributeDataType::U8 => (1, 1),
        AttributeDataType::I16 => (2, 2),
        AttributeDataType::U16 => (3, 2),
        AttributeDataType::I32 => (4, 4),
        AttributeDataType::U32 => (5, 4),
        AttributeDataType::I64 => (6, 8),
        AttributeDataType::U64 => (7, 8),
        AttributeDataType::F32 => (8, 4),
        AttributeDataType::F64 => (9, 8),
        AttributeDataType::FixedBytes(width) => (10, width.get()),
    };
    hasher.update(&[tag]);
    hasher.update(&width.to_le_bytes());
}

fn hash_bytes(
    hasher: &mut Hasher,
    bytes: &[u8],
    reporter: &OperationReporter,
) -> Result<(), SourceError> {
    hash_len(hasher, bytes.len())?;
    hash_raw_bytes(hasher, bytes, reporter)
}

fn hash_len(hasher: &mut Hasher, len: usize) -> Result<(), SourceError> {
    let len = u64::try_from(len)
        .map_err(|_| SourceError::adapter("canonical sequence length does not fit u64"))?;
    hasher.update(&len.to_le_bytes());
    Ok(())
}

fn hash_raw_bytes(
    hasher: &mut Hasher,
    bytes: &[u8],
    reporter: &OperationReporter,
) -> Result<(), SourceError> {
    for chunk in bytes.chunks(HASH_BYTE_QUANTUM) {
        reporter.check_cancelled()?;
        hasher.update(chunk);
    }
    Ok(())
}

fn report_progress(
    reporter: &OperationReporter,
    phase: ProgressPhase,
    completed: u64,
    total: u64,
) -> Result<(), SourceError> {
    reporter
        .report_progress(ProgressSnapshot::new(phase, completed, Some(total))?)
        .map_err(SourceError::from)
}

fn contract_as_source(error: ContractError) -> SourceError {
    SourceError::adapter(error.to_string())
}

fn lock_recovering<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Invalid in-memory Source input.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// A canonical value failed validation.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// The host cannot represent the input row count as a Source-scale count.
    #[error("memory Source Point count does not fit u64")]
    PointCountOverflow,
    /// Metadata and supplied position rows disagree.
    #[error("metadata declares {metadata} Points but input contains {actual}")]
    PointCountMismatch {
        /// Metadata Point count.
        metadata: u64,
        /// Supplied position count.
        actual: u64,
    },
    /// Position and Attribute columns have different row counts.
    #[error("input has {positions} positions but {attributes} Attribute rows")]
    AttributeRowCountMismatch {
        /// Position row count.
        positions: usize,
        /// Attribute row count.
        attributes: usize,
    },
    /// Metadata Attribute definitions differ from supplied columns.
    #[error("metadata Attribute schema differs from supplied columns")]
    AttributeSchemaMismatch,
    /// Metadata bounds do not exactly describe supplied positions.
    #[error("metadata world bounds differ from supplied positions")]
    WorldBoundsMismatch,
}
