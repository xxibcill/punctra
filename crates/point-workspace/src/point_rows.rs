use std::sync::Arc;

use blake3::Hasher;
use foundation_runtime::{
    BatchStream, OperationControl, OperationHandle, ProgressPhase, ProgressSnapshot,
};
use point_contracts::{
    ContentHash, PointBatch, PointId, QuantizedPositions, SourceId, SourceMetadata,
};
use point_source::{
    AttributeSelection, PointBatches, ReadBudget, ReadRequest, SourceReadSummary, SourceSpan,
};

use crate::{
    PointQuery, PointRowLimits, Snapshot, SnapshotProvenance, WorkspaceError,
    selection::plan_query,
    util::allocation_bytes,
    workspace::{EffectiveClassificationBudget, OverlayUsage, Session},
};

const CANCELLATION_STRIDE: usize = 4_096;
const ORDINAL_PAYLOAD_BYTES: u64 = 8;
const POSITION_PAYLOAD_BYTES: u64 = 24;
const CLASSIFICATION_PAYLOAD_BYTES: u64 = 1;
const ROW_PAYLOAD_BYTES: u64 =
    ORDINAL_PAYLOAD_BYTES + POSITION_PAYLOAD_BYTES + CLASSIFICATION_PAYLOAD_BYTES;
const POINT_ID_HASH_DOMAIN: &[u8] = b"punctra-point-set-ids-v1";
const CONTENT_HASH_DOMAIN: &[u8] = b"punctra-snapshot-point-rows-v1";
const SPAN_HASH_DOMAIN: &[u8] = b"punctra-selection-spans-v1";

/// One nonempty bounded batch of exact rows from an immutable Snapshot.
///
/// Every column has the same length. Ordinals are strictly increasing within
/// a batch and across every batch from the same completed stream.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotPointBatch {
    source: SourceId,
    ordinals: Vec<u64>,
    positions: QuantizedPositions,
    effective_classifications: Vec<u8>,
}

impl SnapshotPointBatch {
    /// Returns the immutable Source identity shared by every row.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Returns strictly increasing canonical Source ordinals.
    #[must_use]
    pub fn ordinals(&self) -> &[u64] {
        &self.ordinals
    }

    /// Returns exact signed position ticks and the verified Source transform.
    #[must_use]
    pub const fn positions(&self) -> &QuantizedPositions {
        &self.positions
    }

    /// Returns classification values after every overlay through the Snapshot.
    #[must_use]
    pub fn effective_classifications(&self) -> &[u8] {
        &self.effective_classifications
    }

    /// Returns one stable Point Identity, or `None` for an invalid row.
    #[must_use]
    pub fn point_id(&self, row: usize) -> Option<PointId> {
        self.ordinals
            .get(row)
            .copied()
            .map(|ordinal| PointId::new(self.source, ordinal))
    }

    /// Returns the exact row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ordinals.len()
    }

    /// Reports whether this batch is empty.
    ///
    /// A successfully constructed batch always returns `false`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ordinals.is_empty()
    }
}

/// Exact terminal facts for one successfully completed Snapshot Point read.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotPointSummary {
    provenance: SnapshotProvenance,
    query: PointQuery,
    candidate_point_count: u64,
    exact_count: u64,
    point_id_hash: ContentHash,
    content_hash: ContentHash,
}

impl SnapshotPointSummary {
    /// Returns the immutable Workspace, Source, and Revision identity chain.
    #[must_use]
    pub const fn provenance(&self) -> &SnapshotProvenance {
        &self.provenance
    }

    /// Returns the immutable Source identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.provenance.source()
    }

    /// Returns the exact normalized Query evaluated by the stream.
    #[must_use]
    pub const fn query(&self) -> PointQuery {
        self.query
    }

    /// Returns the exact number of conservative candidate Points examined.
    #[must_use]
    pub const fn candidate_point_count(&self) -> u64 {
        self.candidate_point_count
    }

    /// Returns the exact number of rows emitted by the complete stream.
    #[must_use]
    pub const fn exact_count(&self) -> u64 {
        self.exact_count
    }

    /// Returns the canonical hash of ordered Point Identities.
    ///
    /// This uses the same domain and encoding as Point Set membership hashes.
    #[must_use]
    pub const fn point_id_hash(&self) -> ContentHash {
        self.point_id_hash
    }

    /// Returns the provenance-bound hash of every ordinal, tick, and value.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }
}

struct PendingBatch {
    batch: PointBatch,
    effective: Vec<u8>,
    next_row: usize,
    base_working_bytes: u64,
}

enum PendingResult {
    Batch(SnapshotPointBatch),
    Exhausted,
}

#[derive(Clone, Copy)]
struct ScanRange {
    start: usize,
    end: usize,
    matched: u64,
}

struct OutputColumns {
    ordinals: Vec<u64>,
    ticks: Vec<[i64; 3]>,
    classifications: Vec<u8>,
}

/// Pull-based bounded stream of exact effective Snapshot Point rows.
pub struct SnapshotPointBatches {
    session: Arc<Session>,
    provenance: SnapshotProvenance,
    query: PointQuery,
    limits: PointRowLimits,
    classification: point_contracts::AttributeId,
    source_budget: ReadBudget,
    expected_spans: SpanFacts,
    retained_span_bytes: u64,
    source: Option<PointBatches>,
    pending: Option<PendingBatch>,
    overlay_usage: OverlayUsage,
    control: OperationControl,
    examined_points: u64,
    emitted_points: u64,
    previous_ordinal: Option<u64>,
    point_id_hasher: Hasher,
    content_hasher: Hasher,
    summary: Option<SnapshotPointSummary>,
    terminal: bool,
}

impl SnapshotPointBatches {
    /// Returns verified canonical Source metadata for every possible row.
    #[must_use]
    pub fn source_metadata(&self) -> &SourceMetadata {
        self.session.source().metadata()
    }

    /// Returns a cloneable observation and cancellation handle.
    #[must_use]
    pub fn handle(&self) -> OperationHandle {
        self.control.handle()
    }

    /// Returns the next nonempty exact batch, or terminal `None`.
    ///
    /// A failure, including cancellation, is returned once. Later calls are
    /// fused to `Ok(None)`, and no terminal summary is published.
    ///
    /// # Errors
    ///
    /// Returns a Query, Source, overlay, cancellation, or resource failure.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<SnapshotPointBatch>, WorkspaceError> {
        self.pull()
    }

    /// Returns exact terminal facts only after successful completion.
    #[must_use]
    pub const fn summary(&self) -> Option<&SnapshotPointSummary> {
        self.summary.as_ref()
    }

    fn pull(&mut self) -> Result<Option<SnapshotPointBatch>, WorkspaceError> {
        if self.terminal {
            return Ok(None);
        }
        if let Err(error) = self.control.check_cancelled() {
            return self.fail(error.into());
        }

        loop {
            if let Some(mut pending) = self.pending.take() {
                let result = self.filter_pending(&mut pending);
                match result {
                    Ok(PendingResult::Batch(batch)) => {
                        if pending.next_row < pending.batch.len() {
                            self.pending = Some(pending);
                        }
                        return Ok(Some(batch));
                    }
                    Ok(PendingResult::Exhausted) => continue,
                    Err(error) => return self.fail(error),
                }
            }

            let next = match self.source.as_mut() {
                Some(source) => source.next(),
                None => {
                    return self.fail(WorkspaceError::incompatible(
                        "Snapshot Point Source stream disappeared before completion",
                    ));
                }
            };
            if let Err(error) = self.control.check_cancelled() {
                return self.fail(error.into());
            }
            match next {
                Ok(Some(batch)) => {
                    let pending = match self.prepare_batch(batch) {
                        Ok(pending) => pending,
                        Err(error) => return self.fail(error),
                    };
                    self.pending = Some(pending);
                }
                Ok(None) => return self.finish(),
                Err(error) => return self.fail(error.into()),
            }
        }
    }

    fn prepare_batch(&mut self, batch: PointBatch) -> Result<PendingBatch, WorkspaceError> {
        let base_without_effective = self
            .retained_span_bytes
            .saturating_add(batch.estimated_payload_bytes())
            .saturating_add(self.source_budget.max_adapter_working_bytes());
        let effective = self.session.effective_classifications(
            self.provenance.revision(),
            &batch,
            base_without_effective,
            EffectiveClassificationBudget::new(
                self.limits.max_working_bytes(),
                self.limits.max_overlay_segments(),
                self.limits.max_overlay_bytes(),
            ),
            &mut self.overlay_usage,
            &self.control,
        )?;
        let effective_bytes = allocation_bytes::<u8>(effective.capacity());
        let base_working_bytes = base_without_effective.saturating_add(effective_bytes);

        Ok(PendingBatch {
            batch,
            effective,
            next_row: 0,
            base_working_bytes,
        })
    }

    fn filter_pending(
        &mut self,
        pending: &mut PendingBatch,
    ) -> Result<PendingResult, WorkspaceError> {
        self.control.check_cancelled()?;
        let scan = self.scan_pending(pending)?;
        let examined = u64::try_from(scan.end.saturating_sub(scan.start)).unwrap_or(u64::MAX);
        if scan.matched == 0 {
            pending.next_row = scan.end;
            self.publish_examined(examined)?;
            debug_assert_eq!(pending.next_row, pending.batch.len());
            return Ok(PendingResult::Exhausted);
        }

        let row_count =
            usize::try_from(scan.matched).map_err(|_| WorkspaceError::ResourceLimit {
                limit: "Snapshot Point batch rows",
                required: scan.matched,
                allowed: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            })?;
        let mut columns = allocate_columns(row_count, pending.base_working_bytes, self.limits)?;
        self.fill_columns(pending, scan, &mut columns)?;
        let batch = self.seal_columns(pending, columns)?;
        self.control.check_cancelled()?;
        self.validate_and_hash_batch(&batch)?;
        self.emitted_points =
            self.emitted_points
                .checked_add(scan.matched)
                .ok_or(WorkspaceError::ResourceLimit {
                    limit: "emitted Snapshot Points",
                    required: u64::MAX,
                    allowed: self.limits.max_output_points(),
                })?;
        pending.next_row = scan.end;
        self.publish_examined(examined)?;
        Ok(PendingResult::Batch(batch))
    }

    fn scan_pending(&self, pending: &PendingBatch) -> Result<ScanRange, WorkspaceError> {
        let start = pending.next_row;
        let remaining_output = self
            .limits
            .max_output_points()
            .saturating_sub(self.emitted_points);
        let by_payload = self.limits.max_batch_payload_bytes() / ROW_PAYLOAD_BYTES;
        let batch_capacity = remaining_output
            .min(self.limits.max_batch_points())
            .min(by_payload);

        let mut end = start;
        let mut matched = 0_u64;
        while end < pending.batch.len() {
            if (end - start).is_multiple_of(CANCELLATION_STRIDE) {
                self.control.check_cancelled()?;
            }
            if matches_query(self.query, &pending.batch, &pending.effective, end) {
                if batch_capacity == 0 {
                    return Err(self.zero_output_capacity_error());
                }
                matched += 1;
            }
            end += 1;
            if matched != 0 && matched == batch_capacity {
                break;
            }
        }
        Ok(ScanRange {
            start,
            end,
            matched,
        })
    }

    fn fill_columns(
        &self,
        pending: &PendingBatch,
        scan: ScanRange,
        columns: &mut OutputColumns,
    ) -> Result<(), WorkspaceError> {
        for row in scan.start..scan.end {
            if (row - scan.start).is_multiple_of(CANCELLATION_STRIDE) {
                self.control.check_cancelled()?;
            }
            if !matches_query(self.query, &pending.batch, &pending.effective, row) {
                continue;
            }
            let row_offset = u64::try_from(row).map_err(|_| {
                WorkspaceError::incompatible("Source batch row index does not fit a Point ordinal")
            })?;
            let ordinal = pending
                .batch
                .first_ordinal()
                .checked_add(row_offset)
                .ok_or_else(|| {
                    WorkspaceError::incompatible("Source batch Point ordinal overflowed")
                })?;
            columns.ordinals.push(ordinal);
            columns.ticks.push(pending.batch.positions().ticks()[row]);
            columns.classifications.push(pending.effective[row]);
        }
        let expected = usize::try_from(scan.matched).unwrap_or(usize::MAX);
        if columns.ordinals.len() != expected
            || columns.ticks.len() != expected
            || columns.classifications.len() != expected
        {
            return Err(WorkspaceError::incompatible(
                "Snapshot Point Query changed during one Source batch",
            ));
        }
        Ok(())
    }

    fn seal_columns(
        &self,
        pending: &PendingBatch,
        columns: OutputColumns,
    ) -> Result<SnapshotPointBatch, WorkspaceError> {
        let tick_bytes = allocation_bytes::<[i64; 3]>(columns.ticks.capacity());
        let shrink_overlap = if columns.ticks.capacity() == columns.ticks.len() {
            0
        } else {
            allocation_bytes::<[i64; 3]>(columns.ticks.len())
        };
        require_working(
            pending
                .base_working_bytes
                .saturating_add(allocation_bytes::<u64>(columns.ordinals.capacity()))
                .saturating_add(allocation_bytes::<u8>(columns.classifications.capacity()))
                .saturating_add(tick_bytes)
                .saturating_add(shrink_overlap),
            self.limits,
            "Snapshot Point position sealing overlap",
        )?;
        let positions =
            QuantizedPositions::new(pending.batch.positions().transform(), columns.ticks)?;
        Ok(SnapshotPointBatch {
            source: self.provenance.source(),
            ordinals: columns.ordinals,
            positions,
            effective_classifications: columns.classifications,
        })
    }

    fn zero_output_capacity_error(&self) -> WorkspaceError {
        if self.emitted_points == self.limits.max_output_points() {
            return WorkspaceError::ResourceLimit {
                limit: "emitted Snapshot Points",
                required: self.emitted_points.saturating_add(1),
                allowed: self.limits.max_output_points(),
            };
        }
        if self.limits.max_batch_points() == 0 {
            return WorkspaceError::ResourceLimit {
                limit: "Snapshot Point batch rows",
                required: 1,
                allowed: 0,
            };
        }
        WorkspaceError::ResourceLimit {
            limit: "Snapshot Point batch payload bytes",
            required: ROW_PAYLOAD_BYTES,
            allowed: self.limits.max_batch_payload_bytes(),
        }
    }

    fn validate_and_hash_batch(
        &mut self,
        batch: &SnapshotPointBatch,
    ) -> Result<(), WorkspaceError> {
        for ((&ordinal, &ticks), &classification) in batch
            .ordinals()
            .iter()
            .zip(batch.positions().ticks())
            .zip(batch.effective_classifications())
        {
            if self
                .previous_ordinal
                .is_some_and(|previous| ordinal <= previous)
            {
                return Err(WorkspaceError::incompatible(
                    "Snapshot Point rows are not strictly Source-ordinal ordered",
                ));
            }
            let ordinal_bytes = ordinal.to_le_bytes();
            self.point_id_hasher.update(&ordinal_bytes);
            self.content_hasher.update(&ordinal_bytes);
            for coordinate in ticks {
                self.content_hasher.update(&coordinate.to_le_bytes());
            }
            self.content_hasher.update(&[classification]);
            self.previous_ordinal = Some(ordinal);
        }
        Ok(())
    }

    fn publish_examined(&mut self, added: u64) -> Result<(), WorkspaceError> {
        self.examined_points =
            self.examined_points
                .checked_add(added)
                .ok_or(WorkspaceError::ResourceLimit {
                    limit: "examined candidate Points",
                    required: u64::MAX,
                    allowed: self.expected_spans.point_count,
                })?;
        if self.examined_points > self.expected_spans.point_count {
            return Err(WorkspaceError::incompatible(
                "Snapshot Point stream examined too many candidate Points",
            ));
        }
        self.control.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            self.examined_points,
            Some(self.expected_spans.point_count),
        )?)?;
        Ok(())
    }

    fn finish(&mut self) -> Result<Option<SnapshotPointBatch>, WorkspaceError> {
        if self.pending.is_some() {
            return self.fail(WorkspaceError::incompatible(
                "Snapshot Point Source ended with an unfiltered batch",
            ));
        }
        let Some(source) = self.source.as_ref() else {
            return self.fail(WorkspaceError::incompatible(
                "Snapshot Point Source ended without stream state",
            ));
        };
        if let Err(error) = validate_source_summary(
            source.summary(),
            self.session.source().provenance(),
            self.classification,
            self.source_budget,
            self.expected_spans,
        ) {
            return self.fail(error);
        }
        if self.examined_points != self.expected_spans.point_count {
            return self.fail(WorkspaceError::incompatible(
                "Snapshot Point stream ended before every candidate was examined",
            ));
        }
        if let Err(error) = self.control.check_cancelled() {
            return self.fail(error.into());
        }
        if let Err(error) = self
            .control
            .complete_progress(self.expected_spans.point_count)
        {
            return self.fail(error.into());
        }
        self.source = None;
        self.summary = Some(SnapshotPointSummary {
            provenance: self.provenance,
            query: self.query,
            candidate_point_count: self.expected_spans.point_count,
            exact_count: self.emitted_points,
            point_id_hash: ContentHash::new(*self.point_id_hasher.finalize().as_bytes()),
            content_hash: ContentHash::new(*self.content_hasher.finalize().as_bytes()),
        });
        self.terminal = true;
        Ok(None)
    }

    fn fail<T>(&mut self, error: WorkspaceError) -> Result<T, WorkspaceError> {
        if let Some(source) = self.source.as_ref() {
            source.handle().cancel();
        }
        self.pending = None;
        self.source = None;
        self.summary = None;
        self.terminal = true;
        Err(error)
    }
}

impl BatchStream for SnapshotPointBatches {
    type Batch = SnapshotPointBatch;
    type Summary = SnapshotPointSummary;
    type Error = WorkspaceError;

    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error> {
        Self::next(self)
    }

    fn summary(&self) -> Option<&Self::Summary> {
        Self::summary(self)
    }

    fn handle(&self) -> OperationHandle {
        Self::handle(self)
    }
}

impl Drop for SnapshotPointBatches {
    fn drop(&mut self) {
        if !self.terminal {
            self.control.cancel();
            if let Some(source) = self.source.as_ref() {
                source.handle().cancel();
            }
        }
    }
}

impl std::fmt::Debug for SnapshotPointBatches {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SnapshotPointBatches")
            .field("provenance", &self.provenance)
            .field("query", &self.query)
            .field("examined_points", &self.examined_points)
            .field("emitted_points", &self.emitted_points)
            .field("progress", &self.control.progress())
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

pub(crate) fn start(
    snapshot: &Snapshot,
    query: PointQuery,
    limits: PointRowLimits,
) -> Result<SnapshotPointBatches, WorkspaceError> {
    let control = OperationControl::new();
    let provenance = *snapshot.provenance();
    let session = snapshot.session();
    let spans = plan_query(
        &session,
        query,
        limits.candidate_limits(),
        limits.max_working_bytes(),
        &control,
    )?;
    let expected_spans = SpanFacts::new(&spans)?;
    let span_handoff_bytes = allocation_bytes::<SourceSpan>(spans.capacity()).saturating_mul(2);
    require_working(
        span_handoff_bytes,
        limits,
        "Source span handoff working bytes",
    )?;

    let retained_span_bytes = allocation_bytes::<SourceSpan>(expected_spans.span_count);
    let source_budget = limits.source_read_budget();
    require_working(
        retained_span_bytes.saturating_add(source_budget.max_adapter_working_bytes()),
        limits,
        "Snapshot Point Source read working bytes",
    )?;
    let classification = session.classification_attribute();
    let request = ReadRequest::all()
        .spans(spans)
        .attributes(AttributeSelection::only([classification]))
        .budget(source_budget);
    let source = session.source().read(request)?;

    let mut point_id_hasher = domain_hasher(POINT_ID_HASH_DOMAIN);
    point_id_hasher.update(provenance.source().as_bytes());
    let mut content_hasher = domain_hasher(CONTENT_HASH_DOMAIN);
    content_hasher.update(provenance.workspace().as_bytes());
    content_hasher.update(provenance.source().as_bytes());
    content_hasher.update(provenance.revision().as_bytes());

    Ok(SnapshotPointBatches {
        session,
        provenance,
        query,
        limits,
        classification,
        source_budget,
        expected_spans,
        retained_span_bytes,
        source: Some(source),
        pending: None,
        overlay_usage: OverlayUsage::default(),
        control,
        examined_points: 0,
        emitted_points: 0,
        previous_ordinal: None,
        point_id_hasher,
        content_hasher,
        summary: None,
        terminal: false,
    })
}

fn allocate_columns(
    rows: usize,
    base_working_bytes: u64,
    limits: PointRowLimits,
) -> Result<OutputColumns, WorkspaceError> {
    let requested_payload = u64::try_from(rows)
        .unwrap_or(u64::MAX)
        .saturating_mul(ROW_PAYLOAD_BYTES);
    if requested_payload > limits.max_batch_payload_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Snapshot Point batch payload bytes",
            required: requested_payload,
            allowed: limits.max_batch_payload_bytes(),
        });
    }
    require_working(
        base_working_bytes.saturating_add(requested_payload),
        limits,
        "Snapshot Point output working bytes",
    )?;

    let mut ordinals = Vec::new();
    ordinals
        .try_reserve_exact(rows)
        .map_err(|_| output_allocation_error(rows, limits))?;
    let ordinal_bytes = allocation_bytes::<u64>(ordinals.capacity());
    require_working(
        base_working_bytes.saturating_add(ordinal_bytes),
        limits,
        "Snapshot Point output working bytes",
    )?;

    let mut ticks = Vec::new();
    ticks
        .try_reserve_exact(rows)
        .map_err(|_| output_allocation_error(rows, limits))?;
    let tick_bytes = allocation_bytes::<[i64; 3]>(ticks.capacity());
    require_working(
        base_working_bytes
            .saturating_add(ordinal_bytes)
            .saturating_add(tick_bytes),
        limits,
        "Snapshot Point output working bytes",
    )?;

    let mut classifications = Vec::new();
    classifications
        .try_reserve_exact(rows)
        .map_err(|_| output_allocation_error(rows, limits))?;
    require_working(
        base_working_bytes
            .saturating_add(ordinal_bytes)
            .saturating_add(tick_bytes)
            .saturating_add(allocation_bytes::<u8>(classifications.capacity())),
        limits,
        "Snapshot Point output working bytes",
    )?;
    Ok(OutputColumns {
        ordinals,
        ticks,
        classifications,
    })
}

fn output_allocation_error(rows: usize, limits: PointRowLimits) -> WorkspaceError {
    WorkspaceError::ResourceLimit {
        limit: "Snapshot Point output allocation",
        required: u64::try_from(rows)
            .unwrap_or(u64::MAX)
            .saturating_mul(ROW_PAYLOAD_BYTES),
        allowed: limits.max_working_bytes(),
    }
}

fn matches_query(query: PointQuery, batch: &PointBatch, effective: &[u8], row: usize) -> bool {
    let matches_bounds = query.bounds().is_none_or(|bounds| {
        let world = batch
            .positions()
            .transform()
            .world_f64(batch.positions().ticks()[row]);
        let min = bounds.min();
        let max = bounds.max();
        (0..3).all(|axis| world[axis] >= min[axis] && world[axis] <= max[axis])
    });
    matches_bounds
        && query
            .classification_eq()
            .is_none_or(|value| effective[row] == value)
}

fn validate_source_summary(
    summary: Option<&SourceReadSummary>,
    provenance: &point_contracts::SourceProvenance,
    classification: point_contracts::AttributeId,
    budget: ReadBudget,
    expected: SpanFacts,
) -> Result<(), WorkspaceError> {
    let summary = summary.ok_or_else(|| {
        WorkspaceError::incompatible("Source read ended without a success summary")
    })?;
    let actual = SpanFacts::new(summary.spans())?;
    if summary.provenance() != provenance
        || summary.exact_count() != expected.point_count
        || summary.attributes() != [classification]
        || summary.budget() != budget
        || actual != expected
    {
        return Err(WorkspaceError::incompatible(
            "Source read summary differs from the complete Snapshot Point plan",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpanFacts {
    span_count: usize,
    point_count: u64,
    hash: [u8; 32],
}

impl SpanFacts {
    fn new(spans: &[SourceSpan]) -> Result<Self, WorkspaceError> {
        let mut point_count = 0_u64;
        let mut hasher = domain_hasher(SPAN_HASH_DOMAIN);
        for span in spans {
            point_count = point_count.checked_add(span.point_count()).ok_or(
                WorkspaceError::ResourceLimit {
                    limit: "candidate Points",
                    required: u64::MAX,
                    allowed: u64::MAX - 1,
                },
            )?;
            hasher.update(&span.first_ordinal().to_le_bytes());
            hasher.update(&span.point_count().to_le_bytes());
        }
        Ok(Self {
            span_count: spans.len(),
            point_count,
            hash: *hasher.finalize().as_bytes(),
        })
    }
}

fn require_working(
    required: u64,
    limits: PointRowLimits,
    limit: &'static str,
) -> Result<(), WorkspaceError> {
    if required > limits.max_working_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit,
            required,
            allowed: limits.max_working_bytes(),
        });
    }
    Ok(())
}

fn domain_hasher(domain: &[u8]) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(domain);
    hasher
}
