use std::{mem, sync::Arc};

use blake3::Hasher;
use foundation_runtime::{OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::{AttributeId, ContentHash, PointBatch, SourceId, WorldBounds};
use point_source::{
    AttributeSelection, MAX_INPUT_SOURCE_SPANS, ReadBudget, ReadRequest, SourceReadSummary,
    SourceSpan,
};

use crate::{
    ClassificationTransition, RevisionAudit, RevisionAuditLimits, RevisionId, RevisionInfo,
    RevisionKind, SnapshotProvenance, WorkspaceError,
    persistence::{RowReadLimits, ValidatedRevision},
    workspace::{Session, map_persistence},
};

const CANCELLATION_STRIDE: usize = 4_096;
const AUDIT_ROW_HASH_DOMAIN: &[u8] = b"punctra-revision-audit-rows-v1";
const AUDIT_POSITION_HASH_DOMAIN: &[u8] = b"punctra-revision-audit-positions-v1";
const AUDIT_CONTENT_HASH_DOMAIN: &[u8] = b"punctra-revision-audit-content-v1";
const SPAN_HASH_DOMAIN: &[u8] = b"punctra-revision-audit-spans-v1";

/// Background derivation of one complete immutable Revision Audit.
pub type RevisionAuditJob = foundation_runtime::Job<RevisionAudit, WorkspaceError>;

pub(crate) fn start(
    session: Arc<Session>,
    revision: RevisionId,
    limits: RevisionAuditLimits,
) -> RevisionAuditJob {
    RevisionAuditJob::spawn(move |control| run(&session, revision, limits, &control))
}

fn run(
    session: &Session,
    revision: RevisionId,
    limits: RevisionAuditLimits,
    control: &OperationControl,
) -> Result<RevisionAudit, WorkspaceError> {
    control.check_cancelled()?;
    let revision_info = session.revision_info(revision)?;
    let provenance = SnapshotProvenance::new(
        session.workspace_identity(),
        session.source().identity(),
        revision,
    );
    let mut memory = MemoryMeter::new(limits.max_working_bytes());

    if revision_info.kind() == RevisionKind::Root {
        return finish_empty(provenance, revision_info, limits, &mut memory, control);
    }

    let persisted = session.revision_for_audit(revision)?;
    preflight_revision(&persisted, limits, &mut memory)?;
    let scanned = scan_revision_rows(&persisted, provenance, limits, &mut memory, control)?;
    let ScannedRevision {
        spans,
        transitions,
        changed_point_count,
        point_id_hash,
        row_hash,
    } = scanned;
    let positions = read_source_positions(
        session,
        provenance,
        spans,
        transitions.retained_bytes(),
        changed_point_count,
        point_id_hash,
        limits,
        &mut memory,
        control,
    )?;
    let content_hash = content_hash(
        provenance,
        revision_info,
        changed_point_count,
        point_id_hash,
        row_hash,
        positions.position_hash,
        &transitions.entries,
        positions.footprint,
    );
    seal_report(
        provenance,
        revision_info,
        positions.footprint,
        transitions,
        changed_point_count,
        point_id_hash,
        content_hash,
        limits,
        &mut memory,
        control,
    )
}

fn finish_empty(
    provenance: SnapshotProvenance,
    revision: RevisionInfo,
    limits: RevisionAuditLimits,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<RevisionAudit, WorkspaceError> {
    let point_id_hash = ContentHash::new(
        *crate::hashes::point_id_hasher(provenance.source())
            .finalize()
            .as_bytes(),
    );
    let row_hash = empty_subhash(AUDIT_ROW_HASH_DOMAIN, provenance.source());
    let position_hash = empty_subhash(AUDIT_POSITION_HASH_DOMAIN, provenance.source());
    let transitions = TransitionAccumulator::default();
    let content_hash = content_hash(
        provenance,
        revision,
        0,
        point_id_hash,
        row_hash,
        position_hash,
        &transitions.entries,
        None,
    );
    seal_report(
        provenance,
        revision,
        None,
        transitions,
        0,
        point_id_hash,
        content_hash,
        limits,
        memory,
        control,
    )
}

fn preflight_revision(
    revision: &ValidatedRevision,
    limits: RevisionAuditLimits,
    memory: &mut MemoryMeter,
) -> Result<(), WorkspaceError> {
    require(
        revision.block_count(),
        limits.max_revision_blocks(),
        "Revision Audit blocks",
    )?;
    require(
        revision.file_bytes(),
        limits.max_revision_bytes(),
        "Revision Audit encoded Revision bytes",
    )?;
    require(
        revision.row_count(),
        limits.max_changed_points(),
        "Revision Audit changed Points",
    )?;
    memory.require(
        revision.max_block_bytes(),
        "Revision Audit row read working bytes",
    )
}

struct ScannedRevision {
    spans: Vec<SourceSpan>,
    transitions: TransitionAccumulator,
    changed_point_count: u64,
    point_id_hash: ContentHash,
    row_hash: ContentHash,
}

fn scan_revision_rows(
    revision: &ValidatedRevision,
    provenance: SnapshotProvenance,
    limits: RevisionAuditLimits,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<ScannedRevision, WorkspaceError> {
    let revision_buffer_bytes = revision.max_block_bytes();
    let mut rows = revision
        .rows(
            RowReadLimits {
                max_frames: limits.max_revision_blocks(),
                max_payload_bytes: limits.max_revision_bytes(),
                max_working_bytes: revision_buffer_bytes,
            },
            control,
        )
        .map_err(map_persistence)?;
    let mut spans = SpanAccumulator::default();
    let mut transitions = TransitionAccumulator::default();
    let mut point_ids = crate::hashes::point_id_hasher(provenance.source());
    let mut row_hasher = subhasher(AUDIT_ROW_HASH_DOMAIN, provenance.source());
    let mut changed_point_count = 0_u64;
    let mut previous_ordinal = None;

    for row in &mut rows {
        if usize::try_from(changed_point_count)
            .unwrap_or(usize::MAX)
            .is_multiple_of(CANCELLATION_STRIDE)
        {
            control.check_cancelled()?;
        }
        let row = row.map_err(map_persistence)?;
        if row.before == row.after {
            return Err(WorkspaceError::corrupt(
                "Revision Audit encountered a no-op persisted row",
            ));
        }
        if previous_ordinal.is_some_and(|previous| row.ordinal <= previous) {
            return Err(WorkspaceError::corrupt(
                "Revision Audit rows are not strictly Source-ordinal ordered",
            ));
        }
        changed_point_count =
            changed_point_count
                .checked_add(1)
                .ok_or(WorkspaceError::ResourceLimit {
                    limit: "Revision Audit changed Points",
                    required: u64::MAX,
                    allowed: limits.max_changed_points(),
                })?;
        require(
            changed_point_count,
            limits.max_changed_points(),
            "Revision Audit changed Points",
        )?;

        let span_bytes = vector_bytes::<SourceSpan>(spans.spans.capacity());
        transitions.observe(
            row.before,
            row.after,
            limits,
            revision_buffer_bytes.saturating_add(span_bytes),
            memory,
        )?;
        let transition_bytes = transitions.retained_bytes();
        spans.observe(
            row.ordinal,
            limits.source_read_budget(),
            revision_buffer_bytes.saturating_add(transition_bytes),
            memory,
        )?;
        memory.require(
            revision_buffer_bytes
                .saturating_add(spans.retained_bytes())
                .saturating_add(transition_bytes),
            "Revision Audit row scan working bytes",
        )?;

        let ordinal = row.ordinal.to_le_bytes();
        point_ids.update(&ordinal);
        row_hasher.update(&ordinal);
        row_hasher.update(&[row.before, row.after]);
        previous_ordinal = Some(row.ordinal);
    }
    drop(rows);
    spans.finish(
        limits.source_read_budget(),
        transitions.retained_bytes(),
        memory,
    )?;
    if changed_point_count != revision.row_count() {
        return Err(WorkspaceError::corrupt(
            "Revision Audit row count differs from immutable Revision facts",
        ));
    }
    transitions.validate_count(changed_point_count)?;
    control.check_cancelled()?;

    Ok(ScannedRevision {
        spans: spans.spans,
        transitions,
        changed_point_count,
        point_id_hash: ContentHash::new(*point_ids.finalize().as_bytes()),
        row_hash: ContentHash::new(*row_hasher.finalize().as_bytes()),
    })
}

struct PositionFacts {
    footprint: Option<WorldBounds>,
    position_hash: ContentHash,
}

struct SourceReadPlan {
    request: ReadRequest,
    expected: SpanFacts,
    retained_span_bytes: u64,
    budget: ReadBudget,
}

impl SourceReadPlan {
    fn new(
        spans: Vec<SourceSpan>,
        changed_point_count: u64,
        transition_bytes: u64,
        budget: ReadBudget,
        memory: &mut MemoryMeter,
    ) -> Result<Self, WorkspaceError> {
        let expected = SpanFacts::new(&spans)?;
        require(
            changed_point_count,
            budget.max_points(),
            "Revision Audit Source Points",
        )?;
        let span_count = u64::try_from(spans.len()).unwrap_or(u64::MAX);
        require(
            span_count,
            budget.max_spans(),
            "Revision Audit Source spans",
        )?;
        require(
            span_count,
            u64::try_from(MAX_INPUT_SOURCE_SPANS).unwrap_or(u64::MAX),
            "Revision Audit input Source spans",
        )?;

        let input_span_bytes = vector_bytes::<SourceSpan>(spans.capacity());
        memory.require(
            transition_bytes
                .saturating_add(input_span_bytes.saturating_mul(2))
                .saturating_add(budget.max_batch_payload_bytes())
                .saturating_add(budget.max_adapter_working_bytes()),
            "Revision Audit Source handoff working bytes",
        )?;
        let request = ReadRequest::all()
            .spans(spans)
            .attributes(AttributeSelection::only(std::iter::empty::<AttributeId>()))
            .budget(budget);
        Ok(Self {
            request,
            expected,
            retained_span_bytes: count_bytes::<SourceSpan>(expected.span_count),
            budget,
        })
    }
}

struct PositionAccumulator {
    point_ids: Hasher,
    positions: Hasher,
    footprint: FootprintAccumulator,
    observed: u64,
}

impl PositionAccumulator {
    fn new(source: SourceId) -> Self {
        Self {
            point_ids: crate::hashes::point_id_hasher(source),
            positions: subhasher(AUDIT_POSITION_HASH_DOMAIN, source),
            footprint: FootprintAccumulator::default(),
            observed: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_batch(
        &mut self,
        batch: &PointBatch,
        transition_bytes: u64,
        retained_span_bytes: u64,
        budget: ReadBudget,
        memory: &mut MemoryMeter,
        control: &OperationControl,
    ) -> Result<(), WorkspaceError> {
        if !batch.attributes().is_empty() {
            return Err(WorkspaceError::incompatible(
                "Revision Audit Source returned unrequested Attributes",
            ));
        }
        memory.require(
            transition_bytes
                .saturating_add(retained_span_bytes)
                .saturating_add(batch.estimated_payload_bytes())
                .saturating_add(budget.max_adapter_working_bytes()),
            "Revision Audit Source batch working bytes",
        )?;
        for (row, &ticks) in batch.positions().ticks().iter().enumerate() {
            if row.is_multiple_of(CANCELLATION_STRIDE) {
                control.check_cancelled()?;
            }
            let ordinal = batch
                .first_ordinal()
                .checked_add(u64::try_from(row).map_err(|_| {
                    WorkspaceError::incompatible("Revision Audit Source row index does not fit u64")
                })?)
                .ok_or_else(|| {
                    WorkspaceError::incompatible("Revision Audit Source ordinal overflowed")
                })?;
            self.observe_position(batch, row, ordinal, ticks)?;
            self.observed = self
                .observed
                .checked_add(1)
                .ok_or(WorkspaceError::ResourceLimit {
                    limit: "Revision Audit Source Points",
                    required: u64::MAX,
                    allowed: budget.max_points(),
                })?;
            require(
                self.observed,
                budget.max_points(),
                "Revision Audit Source Points",
            )?;
        }
        Ok(())
    }

    fn observe_position(
        &mut self,
        batch: &PointBatch,
        row: usize,
        ordinal: u64,
        ticks: [i64; 3],
    ) -> Result<(), WorkspaceError> {
        let ordinal_bytes = ordinal.to_le_bytes();
        self.point_ids.update(&ordinal_bytes);
        self.positions.update(&ordinal_bytes);
        for tick in ticks {
            self.positions.update(&tick.to_le_bytes());
        }
        let world = batch.positions().world_f64(row).ok_or_else(|| {
            WorkspaceError::incompatible("Revision Audit Source position row disappeared")
        })?;
        self.footprint.observe(world)
    }

    fn finish(
        self,
        expected_count: u64,
        expected_point_id_hash: ContentHash,
    ) -> Result<PositionFacts, WorkspaceError> {
        if self.observed != expected_count {
            return Err(WorkspaceError::incompatible(
                "Revision Audit Source count differs from immutable Revision rows",
            ));
        }
        let position_membership = ContentHash::new(*self.point_ids.finalize().as_bytes());
        if position_membership != expected_point_id_hash {
            return Err(WorkspaceError::incompatible(
                "Revision Audit Source membership differs from immutable Revision rows",
            ));
        }
        let footprint = self.footprint.finish()?;
        if footprint.is_none() {
            return Err(WorkspaceError::corrupt(
                "non-root Revision Audit has no changed Source position",
            ));
        }
        Ok(PositionFacts {
            footprint,
            position_hash: ContentHash::new(*self.positions.finalize().as_bytes()),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn read_source_positions(
    session: &Session,
    provenance: SnapshotProvenance,
    spans: Vec<SourceSpan>,
    transition_bytes: u64,
    changed_point_count: u64,
    expected_point_id_hash: ContentHash,
    limits: RevisionAuditLimits,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<PositionFacts, WorkspaceError> {
    let budget = limits.source_read_budget();
    let plan = SourceReadPlan::new(spans, changed_point_count, transition_bytes, budget, memory)?;
    let mut batches = session.source().read(plan.request)?;
    let handle = batches.handle();
    let mut positions = PositionAccumulator::new(provenance.source());

    loop {
        if let Err(error) = control.check_cancelled() {
            handle.cancel();
            return Err(error.into());
        }
        let next = batches.next();
        if let Err(error) = control.check_cancelled() {
            handle.cancel();
            return Err(error.into());
        }
        let Some(batch) = next? else {
            break;
        };
        positions.observe_batch(
            &batch,
            transition_bytes,
            plan.retained_span_bytes,
            budget,
            memory,
            control,
        )?;
        control.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            positions.observed,
            Some(changed_point_count),
        )?)?;
    }

    validate_source_summary(
        batches.summary(),
        session.source().provenance(),
        plan.budget,
        plan.expected,
    )?;
    drop(batches);
    control.check_cancelled()?;
    positions.finish(changed_point_count, expected_point_id_hash)
}

#[allow(clippy::too_many_arguments)]
fn seal_report(
    provenance: SnapshotProvenance,
    revision: RevisionInfo,
    footprint: Option<WorldBounds>,
    transitions: TransitionAccumulator,
    changed_point_count: u64,
    point_id_hash: ContentHash,
    content_hash: ContentHash,
    limits: RevisionAuditLimits,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<RevisionAudit, WorkspaceError> {
    let fixed_bytes = u64::try_from(mem::size_of::<RevisionAudit>()).unwrap_or(u64::MAX);
    let transition_result_bytes =
        count_bytes::<ClassificationTransition>(transitions.entries.len());
    let retained_result_bytes = fixed_bytes.saturating_add(transition_result_bytes);
    require(
        retained_result_bytes,
        limits.max_result_bytes(),
        "Revision Audit retained result bytes",
    )?;

    let builder_bytes = transitions.retained_bytes();
    memory.require(
        builder_bytes
            .saturating_add(transition_result_bytes)
            .saturating_add(fixed_bytes),
        "Revision Audit result sealing working bytes",
    )?;
    let mut sealed = Vec::new();
    sealed
        .try_reserve_exact(transitions.entries.len())
        .map_err(|_| WorkspaceError::ResourceLimit {
            limit: "Revision Audit transition result allocation",
            required: transition_result_bytes,
            allowed: limits.max_working_bytes(),
        })?;
    let sealed_bytes = vector_bytes::<ClassificationTransition>(sealed.capacity());
    memory.require(
        builder_bytes
            .saturating_add(sealed_bytes)
            .saturating_add(fixed_bytes),
        "Revision Audit transition result working bytes",
    )?;
    sealed.extend(
        transitions
            .entries
            .iter()
            .map(|entry| ClassificationTransition::new(entry.before, entry.after, entry.count)),
    );
    drop(transitions);
    memory.require(
        sealed_bytes
            .saturating_add(transition_result_bytes)
            .saturating_add(fixed_bytes),
        "Revision Audit boxed result working bytes",
    )?;
    let transitions = sealed.into_boxed_slice();
    let report = RevisionAudit::new(
        provenance,
        revision,
        footprint,
        transitions,
        changed_point_count,
        point_id_hash,
        content_hash,
        memory.peak,
        retained_result_bytes,
    );
    control.check_cancelled()?;
    control.complete_progress(changed_point_count)?;
    Ok(report)
}

#[derive(Default)]
struct SpanAccumulator {
    spans: Vec<SourceSpan>,
    run_start: Option<u64>,
    run_end: u64,
}

impl SpanAccumulator {
    fn observe(
        &mut self,
        ordinal: u64,
        budget: ReadBudget,
        retained_other: u64,
        memory: &mut MemoryMeter,
    ) -> Result<(), WorkspaceError> {
        match self.run_start {
            None => {
                self.run_start = Some(ordinal);
                self.run_end = ordinal.checked_add(1).ok_or_else(|| {
                    WorkspaceError::corrupt("Revision Audit ordinal range overflowed")
                })?;
            }
            Some(_) if ordinal == self.run_end => {
                self.run_end = ordinal.checked_add(1).ok_or_else(|| {
                    WorkspaceError::corrupt("Revision Audit ordinal range overflowed")
                })?;
            }
            Some(_) => {
                self.push_run(budget, retained_other, memory)?;
                self.run_start = Some(ordinal);
                self.run_end = ordinal.checked_add(1).ok_or_else(|| {
                    WorkspaceError::corrupt("Revision Audit ordinal range overflowed")
                })?;
            }
        }
        Ok(())
    }

    fn finish(
        &mut self,
        budget: ReadBudget,
        retained_other: u64,
        memory: &mut MemoryMeter,
    ) -> Result<(), WorkspaceError> {
        if self.run_start.is_some() {
            self.push_run(budget, retained_other, memory)?;
        }
        Ok(())
    }

    fn push_run(
        &mut self,
        budget: ReadBudget,
        retained_other: u64,
        memory: &mut MemoryMeter,
    ) -> Result<(), WorkspaceError> {
        let first = self.run_start.take().ok_or_else(|| {
            WorkspaceError::corrupt("Revision Audit Source span has no first ordinal")
        })?;
        let count = self.run_end.checked_sub(first).ok_or_else(|| {
            WorkspaceError::corrupt("Revision Audit Source span range is reversed")
        })?;
        let next_count = u64::try_from(self.spans.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        require(
            next_count,
            budget.max_spans(),
            "Revision Audit Source spans",
        )?;
        require(
            next_count,
            u64::try_from(MAX_INPUT_SOURCE_SPANS).unwrap_or(u64::MAX),
            "Revision Audit input Source spans",
        )?;
        reserve_next(
            &mut self.spans,
            usize::try_from(next_count).unwrap_or(usize::MAX),
            retained_other,
            memory,
            "Revision Audit Source span allocation",
        )?;
        self.spans.push(SourceSpan::new(first, count)?);
        Ok(())
    }

    fn retained_bytes(&self) -> u64 {
        vector_bytes::<SourceSpan>(self.spans.capacity())
    }
}

#[derive(Clone, Copy)]
struct TransitionCount {
    before: u8,
    after: u8,
    count: u64,
}

#[derive(Default)]
struct TransitionAccumulator {
    entries: Vec<TransitionCount>,
}

impl TransitionAccumulator {
    fn observe(
        &mut self,
        before: u8,
        after: u8,
        limits: RevisionAuditLimits,
        retained_other: u64,
        memory: &mut MemoryMeter,
    ) -> Result<(), WorkspaceError> {
        let key = (before, after);
        match self
            .entries
            .binary_search_by_key(&key, |entry| (entry.before, entry.after))
        {
            Ok(index) => {
                self.entries[index].count = self.entries[index].count.checked_add(1).ok_or(
                    WorkspaceError::ResourceLimit {
                        limit: "Revision Audit transition count",
                        required: u64::MAX,
                        allowed: limits.max_changed_points(),
                    },
                )?;
            }
            Err(index) => {
                let next_count = u64::try_from(self.entries.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(1);
                require(
                    next_count,
                    limits.max_transition_entries(),
                    "Revision Audit transition entries",
                )?;
                reserve_next(
                    &mut self.entries,
                    usize::try_from(next_count).unwrap_or(usize::MAX),
                    retained_other,
                    memory,
                    "Revision Audit transition allocation",
                )?;
                self.entries.insert(
                    index,
                    TransitionCount {
                        before,
                        after,
                        count: 1,
                    },
                );
            }
        }
        Ok(())
    }

    fn validate_count(&self, expected: u64) -> Result<(), WorkspaceError> {
        let actual = self.entries.iter().try_fold(0_u64, |count, entry| {
            count.checked_add(entry.count).ok_or_else(|| {
                WorkspaceError::corrupt("Revision Audit transition count overflowed")
            })
        })?;
        if actual != expected || self.entries.iter().any(|entry| entry.count == 0) {
            return Err(WorkspaceError::corrupt(
                "Revision Audit transitions differ from changed rows",
            ));
        }
        Ok(())
    }

    fn retained_bytes(&self) -> u64 {
        vector_bytes::<TransitionCount>(self.entries.capacity())
    }
}

#[derive(Default)]
struct FootprintAccumulator {
    minimum: Option<[f64; 3]>,
    maximum: [f64; 3],
}

impl FootprintAccumulator {
    fn observe(&mut self, world: [f64; 3]) -> Result<(), WorkspaceError> {
        if world.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(WorkspaceError::incompatible(
                "Revision Audit Source world position is non-finite",
            ));
        }
        match self.minimum.as_mut() {
            None => {
                self.minimum = Some(world);
                self.maximum = world;
            }
            Some(minimum) => {
                for axis in 0..3 {
                    minimum[axis] = minimum[axis].min(world[axis]);
                    self.maximum[axis] = self.maximum[axis].max(world[axis]);
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Option<WorldBounds>, WorkspaceError> {
        self.minimum
            .map(|minimum| WorldBounds::new(minimum, self.maximum).map_err(WorkspaceError::from))
            .transpose()
    }
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
                    limit: "Revision Audit Source Points",
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

fn validate_source_summary(
    summary: Option<&SourceReadSummary>,
    provenance: &point_contracts::SourceProvenance,
    budget: ReadBudget,
    expected: SpanFacts,
) -> Result<(), WorkspaceError> {
    let summary = summary.ok_or_else(|| {
        WorkspaceError::incompatible("Revision Audit Source ended without a success summary")
    })?;
    if summary.provenance() != provenance
        || summary.exact_count() != expected.point_count
        || !summary.attributes().is_empty()
        || summary.budget() != budget
        || SpanFacts::new(summary.spans())? != expected
    {
        return Err(WorkspaceError::incompatible(
            "Revision Audit Source summary differs from requested changed membership",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn content_hash(
    provenance: SnapshotProvenance,
    revision: RevisionInfo,
    changed_point_count: u64,
    point_id_hash: ContentHash,
    row_hash: ContentHash,
    position_hash: ContentHash,
    transitions: &[TransitionCount],
    footprint: Option<WorldBounds>,
) -> ContentHash {
    let mut hasher = domain_hasher(AUDIT_CONTENT_HASH_DOMAIN);
    hash_provenance(&mut hasher, provenance);
    hash_revision(&mut hasher, revision);
    hasher.update(&changed_point_count.to_le_bytes());
    hasher.update(point_id_hash.as_bytes());
    hasher.update(row_hash.as_bytes());
    hasher.update(position_hash.as_bytes());
    hasher.update(
        &u64::try_from(transitions.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for transition in transitions {
        hasher.update(&[transition.before, transition.after]);
        hasher.update(&transition.count.to_le_bytes());
    }
    match footprint {
        None => {
            hasher.update(&[0]);
        }
        Some(bounds) => {
            hasher.update(&[1]);
            for coordinate in bounds.min().into_iter().chain(bounds.max()) {
                hasher.update(&coordinate.to_bits().to_le_bytes());
            }
        }
    }
    ContentHash::new(*hasher.finalize().as_bytes())
}

fn hash_provenance(hasher: &mut Hasher, provenance: SnapshotProvenance) {
    hasher.update(provenance.workspace().as_bytes());
    hasher.update(provenance.source().as_bytes());
    hasher.update(provenance.revision().as_bytes());
}

fn hash_revision(hasher: &mut Hasher, revision: RevisionInfo) {
    hasher.update(revision.id().as_bytes());
    match revision.parent() {
        None => {
            hasher.update(&[0]);
        }
        Some(parent) => {
            hasher.update(&[1]);
            hasher.update(parent.as_bytes());
        }
    }
    hasher.update(&revision.sequence().to_le_bytes());
    match revision.operation() {
        None => {
            hasher.update(&[0]);
        }
        Some(operation) => {
            hasher.update(&[1]);
            hasher.update(operation.as_bytes());
        }
    }
    match revision.kind() {
        RevisionKind::Root => {
            hasher.update(&[0]);
        }
        RevisionKind::SetClassification {
            value,
            changed_points,
        } => {
            hasher.update(&[1, value]);
            hasher.update(&changed_points.to_le_bytes());
        }
        RevisionKind::Revert {
            reverted_revision,
            changed_points,
        } => {
            hasher.update(&[2]);
            hasher.update(reverted_revision.as_bytes());
            hasher.update(&changed_points.to_le_bytes());
        }
    }
}

fn subhasher(domain: &[u8], source: point_contracts::SourceId) -> Hasher {
    let mut hasher = domain_hasher(domain);
    hasher.update(source.as_bytes());
    hasher
}

fn empty_subhash(domain: &[u8], source: point_contracts::SourceId) -> ContentHash {
    ContentHash::new(*subhasher(domain, source).finalize().as_bytes())
}

fn domain_hasher(domain: &[u8]) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(domain);
    hasher
}

struct MemoryMeter {
    allowed: u64,
    peak: u64,
}

impl MemoryMeter {
    const fn new(allowed: u64) -> Self {
        Self { allowed, peak: 0 }
    }

    fn require(&mut self, required: u64, limit: &'static str) -> Result<(), WorkspaceError> {
        require(required, self.allowed, limit)?;
        self.peak = self.peak.max(required);
        Ok(())
    }
}

fn reserve_next<T>(
    values: &mut Vec<T>,
    next_len: usize,
    retained_other: u64,
    memory: &mut MemoryMeter,
    limit: &'static str,
) -> Result<(), WorkspaceError> {
    if next_len <= values.capacity() {
        return Ok(());
    }
    let target = values.capacity().saturating_mul(2).max(4).max(next_len);
    let old_bytes = vector_bytes::<T>(values.capacity());
    let requested_bytes = count_bytes::<T>(target);
    memory.require(
        retained_other
            .saturating_add(old_bytes)
            .saturating_add(requested_bytes),
        limit,
    )?;
    values
        .try_reserve_exact(target.saturating_sub(values.len()))
        .map_err(|_| WorkspaceError::ResourceLimit {
            limit,
            required: retained_other.saturating_add(requested_bytes),
            allowed: memory.allowed,
        })?;
    memory.require(
        retained_other
            .saturating_add(old_bytes)
            .saturating_add(vector_bytes::<T>(values.capacity())),
        limit,
    )
}

fn require(required: u64, allowed: u64, limit: &'static str) -> Result<(), WorkspaceError> {
    if required > allowed {
        Err(WorkspaceError::ResourceLimit {
            limit,
            required,
            allowed,
        })
    } else {
        Ok(())
    }
}

fn count_bytes<T>(count: usize) -> u64 {
    u64::try_from(count)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
}

fn vector_bytes<T>(capacity: usize) -> u64 {
    count_bytes::<T>(capacity)
}

#[cfg(test)]
mod tests {
    use point_source::ReadBudget;

    use super::{MemoryMeter, SpanAccumulator, TransitionAccumulator};
    use crate::RevisionAuditLimits;

    #[test]
    fn spans_coalesce_only_adjacent_ordinals() {
        let mut spans = SpanAccumulator::default();
        let mut memory = MemoryMeter::new(1_024 * 1_024);
        let budget = ReadBudget::default();
        for ordinal in [1, 2, 3, 8, 10, 11] {
            spans.observe(ordinal, budget, 0, &mut memory).unwrap();
        }
        spans.finish(budget, 0, &mut memory).unwrap();
        let facts = spans
            .spans
            .iter()
            .map(|span| (span.first_ordinal(), span.point_count()))
            .collect::<Vec<_>>();
        assert_eq!(facts, vec![(1, 3), (8, 1), (10, 2)]);
    }

    #[test]
    fn transitions_remain_sorted_and_counted() {
        let mut transitions = TransitionAccumulator::default();
        let mut memory = MemoryMeter::new(1_024 * 1_024);
        let limits = RevisionAuditLimits::default();
        for (before, after) in [(3, 8), (1, 8), (3, 8), (2, 4)] {
            transitions
                .observe(before, after, limits, 0, &mut memory)
                .unwrap();
        }
        transitions.validate_count(4).unwrap();
        let facts = transitions
            .entries
            .iter()
            .map(|entry| (entry.before, entry.after, entry.count))
            .collect::<Vec<_>>();
        assert_eq!(facts, vec![(1, 8, 1), (2, 4, 1), (3, 8, 2)]);
    }
}
