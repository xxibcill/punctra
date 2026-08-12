use std::sync::Arc;

use foundation_runtime::{Job, OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::{AttributeId, PointBatch, PointId, SourceId};
use point_index::CandidateLimits;
use point_source::{AttributeSelection, ReadBudget, ReadRequest, SourceSpan};

use crate::{
    PointQuery, PointSet, PointSetLimits, Snapshot, SnapshotProvenance, WorkspaceError,
    point_set::{PointSetBuilder, PointSetRecord},
    query::{SpanFacts, matches_query},
    util::allocation_bytes,
    workspace::{EffectiveClassificationBudget, OverlayUsage, Session},
};

const CANCELLATION_STRIDE: usize = 4_096;
const MIN_WORKING_GROWTH_RECORDS: usize = 4_096;

/// Background exact selection whose success publishes one immutable Point Set.
pub type PointSetJob = Job<PointSet, WorkspaceError>;

pub(crate) fn select(
    snapshot: &Snapshot,
    query: PointQuery,
    limits: PointSetLimits,
) -> PointSetJob {
    let snapshot = snapshot.clone();
    Job::spawn(move |control| run_select(&snapshot, query, limits, &control))
}

fn run_select(
    snapshot: &Snapshot,
    query: PointQuery,
    limits: PointSetLimits,
    control: &OperationControl,
) -> Result<PointSet, WorkspaceError> {
    let provenance = *snapshot.provenance();
    let session = snapshot.session();
    let spans = plan_query(
        &session,
        query,
        limits.candidate_limits(),
        limits.max_working_bytes(),
        control,
    )?;
    materialize(&session, provenance, spans, query, limits, control)
}

pub(crate) fn select_point_ids<I>(
    snapshot: &Snapshot,
    ids: I,
    limits: PointSetLimits,
) -> PointSetJob
where
    I: IntoIterator<Item = PointId>,
{
    // The public iterator need not be Send: consume only the bounded prefix
    // synchronously while the caller-provided selection ledger is in scope.
    let collected = collect_point_ids(ids, limits);
    let snapshot = snapshot.clone();
    Job::spawn(move |control| {
        let provenance = *snapshot.provenance();
        let session = snapshot.session();
        let ids = collected?;
        let spans = normalize_point_ids(
            ids,
            provenance.source(),
            session.index().descriptor().source_point_count(),
            limits,
            &control,
        )?;
        materialize(
            &session,
            provenance,
            spans,
            PointQuery::all(),
            limits,
            &control,
        )
    })
}

pub(crate) fn plan_query(
    session: &Session,
    query: PointQuery,
    candidate_limits: CandidateLimits,
    max_working_bytes: u64,
    control: &OperationControl,
) -> Result<Vec<SourceSpan>, WorkspaceError> {
    control.check_cancelled()?;
    require_peak(
        candidate_limits.max_working_bytes(),
        max_working_bytes,
        "candidate planning working bytes",
    )?;

    let descriptor = session.index().descriptor();
    if descriptor.source_point_count() == 0 {
        if descriptor.world_bounds().is_some() {
            return Err(WorkspaceError::incompatible(
                "empty Spatial Index unexpectedly has world bounds",
            ));
        }
        return Ok(Vec::new());
    }
    let Some(index_bounds) = query.bounds().or(descriptor.world_bounds()) else {
        return Err(WorkspaceError::incompatible(
            "nonempty Spatial Index has no world bounds",
        ));
    };
    let cancellation = control.token();
    let plan = session.index().candidates_with_cancellation(
        index_bounds,
        candidate_limits,
        &cancellation,
    )?;
    control.check_cancelled()?;
    validate_candidate_plan(
        plan.spans(),
        plan.candidate_point_count(),
        descriptor.source_point_count(),
        query.bounds().is_none(),
    )?;
    copy_candidate_spans(
        plan.spans(),
        candidate_limits.max_working_bytes(),
        max_working_bytes,
    )
}

fn validate_candidate_plan(
    spans: &[SourceSpan],
    candidate_count: u64,
    source_count: u64,
    selects_all: bool,
) -> Result<(), WorkspaceError> {
    let mut previous_end = None;
    let mut calculated_count = 0_u64;
    for &span in spans {
        if span.end_ordinal() > source_count
            || previous_end.is_some_and(|end| span.first_ordinal() <= end)
        {
            return Err(WorkspaceError::incompatible(
                "Spatial Index returned invalid candidate Source spans",
            ));
        }
        previous_end = Some(span.end_ordinal());
        calculated_count = calculated_count
            .checked_add(span.point_count())
            .ok_or_else(|| WorkspaceError::incompatible("candidate Point count overflowed"))?;
    }
    if calculated_count != candidate_count {
        return Err(WorkspaceError::incompatible(
            "Spatial Index candidate count differs from its spans",
        ));
    }
    if selects_all && calculated_count != source_count {
        return Err(WorkspaceError::incompatible(
            "Spatial Index omitted Points from an All selection",
        ));
    }
    Ok(())
}

fn copy_candidate_spans(
    source: &[SourceSpan],
    candidate_working_bytes: u64,
    max_working_bytes: u64,
) -> Result<Vec<SourceSpan>, WorkspaceError> {
    let mut spans = Vec::new();
    let available_copy_bytes = max_working_bytes.saturating_sub(candidate_working_bytes);
    if !source.is_empty() {
        grow_working_vec(
            &mut spans,
            source.len(),
            available_copy_bytes,
            "candidate plan copy working bytes",
        )?;
    }
    spans.extend_from_slice(source);
    Ok(spans)
}

fn collect_point_ids<I>(ids: I, limits: PointSetLimits) -> Result<Vec<PointId>, WorkspaceError>
where
    I: IntoIterator<Item = PointId>,
{
    let input = ids.into_iter();
    let lower_bound = u64::try_from(input.size_hint().0).unwrap_or(u64::MAX);
    if lower_bound > limits.max_input_point_ids() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "input Point Identities",
            required: lower_bound,
            allowed: limits.max_input_point_ids(),
        });
    }

    let mut collected = Vec::new();
    for (index, point_id) in input.enumerate() {
        let count = u64::try_from(index).unwrap_or(u64::MAX);
        if count == limits.max_input_point_ids() {
            return Err(WorkspaceError::ResourceLimit {
                limit: "input Point Identities",
                required: count.saturating_add(1),
                allowed: limits.max_input_point_ids(),
            });
        }
        if collected.len() == collected.capacity() {
            let next_len = collected.len().saturating_add(1);
            grow_working_vec(
                &mut collected,
                next_len,
                limits.max_working_bytes(),
                "input Point Identity working bytes",
            )?;
        }
        collected.push(point_id);
    }
    Ok(collected)
}

fn normalize_point_ids(
    mut ids: Vec<PointId>,
    source: SourceId,
    source_point_count: u64,
    limits: PointSetLimits,
    control: &OperationControl,
) -> Result<Vec<SourceSpan>, WorkspaceError> {
    control.check_cancelled()?;
    for (index, point_id) in ids.iter().copied().enumerate() {
        if index % CANCELLATION_STRIDE == 0 {
            control.check_cancelled()?;
        }
        if point_id.source() != source {
            return Err(WorkspaceError::invalid(
                "Point Identities",
                "an identity belongs to another Source",
            ));
        }
        if point_id.ordinal() >= source_point_count {
            return Err(WorkspaceError::invalid(
                "Point Identities",
                "an ordinal is outside the immutable Source",
            ));
        }
    }
    ids.sort_unstable();
    ids.dedup();
    control.check_cancelled()?;

    let input_bytes = allocation_bytes::<PointId>(ids.capacity());
    let mut spans = Vec::new();
    let mut run_start = None;
    let mut run_end = 0_u64;
    for point_id in ids.iter().copied() {
        let ordinal = point_id.ordinal();
        match run_start {
            None => {
                run_start = Some(ordinal);
                run_end = ordinal + 1;
            }
            Some(_) if ordinal == run_end => run_end += 1,
            Some(first) => {
                push_span(&mut spans, first, run_end, input_bytes, limits)?;
                run_start = Some(ordinal);
                run_end = ordinal + 1;
            }
        }
    }
    if let Some(first) = run_start {
        push_span(&mut spans, first, run_end, input_bytes, limits)?;
    }
    drop(ids);
    control.check_cancelled()?;
    Ok(spans)
}

fn push_span(
    spans: &mut Vec<SourceSpan>,
    first: u64,
    end: u64,
    input_bytes: u64,
    limits: PointSetLimits,
) -> Result<(), WorkspaceError> {
    if spans.len() == spans.capacity() {
        let next_len = spans.len().saturating_add(1);
        grow_working_vec(
            spans,
            next_len,
            limits.max_working_bytes().saturating_sub(input_bytes),
            "explicit Point Identity span working bytes",
        )?;
    }
    spans.push(SourceSpan::new(first, end - first)?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize(
    session: &Arc<Session>,
    provenance: SnapshotProvenance,
    spans: Vec<SourceSpan>,
    query: PointQuery,
    limits: PointSetLimits,
    control: &OperationControl,
) -> Result<PointSet, WorkspaceError> {
    control.check_cancelled()?;
    let span_facts = SpanFacts::new(&spans)?;
    let span_allocation = allocation_bytes::<SourceSpan>(spans.capacity());
    require_peak(
        span_allocation.saturating_mul(2),
        limits.max_working_bytes(),
        "Source span handoff working bytes",
    )?;

    let classification = session.classification_attribute();
    let budget = limits.source_read_budget();
    let retained_span_bytes = allocation_bytes::<SourceSpan>(span_facts.span_count());
    let declared_read_base = retained_span_bytes
        .saturating_add(budget.max_batch_payload_bytes())
        .saturating_add(budget.max_adapter_working_bytes())
        .saturating_add(budget.max_batch_points());
    require_peak(
        declared_read_base,
        limits.max_working_bytes(),
        "selection Source read working bytes",
    )?;
    let request = ReadRequest::all()
        .spans(spans)
        .attributes(AttributeSelection::only([classification]))
        .budget(budget);
    let mut batches = session.source().read(request)?;
    let read_handle = batches.handle();

    let mut builder = PointSetBuilder::new(Arc::clone(session), provenance, limits);
    let mut overlay_usage = OverlayUsage::default();
    let mut processed = 0_u64;
    loop {
        if let Err(error) = control.check_cancelled() {
            read_handle.cancel();
            return Err(error.into());
        }
        let next = batches.next();
        if let Err(error) = control.check_cancelled() {
            read_handle.cancel();
            return Err(error.into());
        }
        let Some(batch) = next? else {
            break;
        };
        process_batch(
            session,
            provenance,
            &batch,
            classification,
            query,
            retained_span_bytes,
            budget,
            limits,
            control,
            &mut builder,
            &mut overlay_usage,
        )?;
        processed = processed.saturating_add(batch.point_count());
        control.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            processed,
            Some(span_facts.point_count()),
        )?)?;
    }
    validate_summary(
        batches.summary(),
        provenance.source(),
        classification,
        budget,
        span_facts,
    )?;
    drop(batches);
    control.check_cancelled()?;
    let point_set = builder.finish()?;
    if let Err(error) = control.check_cancelled() {
        drop(point_set);
        return Err(error.into());
    }
    control.complete_progress(span_facts.point_count())?;
    Ok(point_set)
}

#[allow(clippy::too_many_arguments)]
fn process_batch(
    session: &Session,
    provenance: SnapshotProvenance,
    batch: &PointBatch,
    classification: AttributeId,
    query: PointQuery,
    retained_span_bytes: u64,
    budget: ReadBudget,
    limits: PointSetLimits,
    control: &OperationControl,
    builder: &mut PointSetBuilder,
    overlay_usage: &mut OverlayUsage,
) -> Result<(), WorkspaceError> {
    let before_effective = retained_span_bytes
        .saturating_add(batch.estimated_payload_bytes())
        .saturating_add(budget.max_adapter_working_bytes())
        .saturating_add(builder.resident_bytes());
    debug_assert_eq!(classification, session.classification_attribute());
    let effective = session.effective_classifications(
        provenance.revision(),
        batch,
        before_effective,
        EffectiveClassificationBudget::new(
            limits.max_working_bytes(),
            limits.max_overlay_segments(),
            limits.max_overlay_bytes(),
        ),
        overlay_usage,
        control,
    )?;
    let batch_base = retained_span_bytes
        .saturating_add(batch.estimated_payload_bytes())
        .saturating_add(budget.max_adapter_working_bytes())
        .saturating_add(allocation_bytes::<u8>(effective.capacity()));

    for (row, &effective_classification) in effective.iter().enumerate() {
        if row % CANCELLATION_STRIDE == 0 {
            control.check_cancelled()?;
        }
        if !matches_query(query, batch, &effective, row)? {
            continue;
        }
        let row = u64::try_from(row).map_err(|_| {
            WorkspaceError::incompatible("Source batch row index does not fit a Point ordinal")
        })?;
        let ordinal = batch
            .first_ordinal()
            .checked_add(row)
            .ok_or_else(|| WorkspaceError::incompatible("Source batch Point ordinal overflowed"))?;
        builder.push(
            PointSetRecord {
                ordinal,
                effective_classification,
            },
            batch_base,
        )?;
    }
    Ok(())
}

fn validate_summary(
    summary: Option<&point_source::SourceReadSummary>,
    source: SourceId,
    classification: AttributeId,
    budget: ReadBudget,
    expected: SpanFacts,
) -> Result<(), WorkspaceError> {
    let summary = summary.ok_or_else(|| {
        WorkspaceError::incompatible("Source read ended without a success summary")
    })?;
    let actual = SpanFacts::new(summary.spans())?;
    if summary.source() != source
        || summary.exact_count() != expected.point_count()
        || summary.attributes() != [classification]
        || summary.budget() != budget
        || actual != expected
    {
        return Err(WorkspaceError::incompatible(
            "Source read summary differs from the complete selection plan",
        ));
    }
    Ok(())
}

fn require_peak(required: u64, allowed: u64, limit: &'static str) -> Result<(), WorkspaceError> {
    if required > allowed {
        return Err(WorkspaceError::ResourceLimit {
            limit,
            required,
            allowed,
        });
    }
    Ok(())
}

fn grow_working_vec<T>(
    values: &mut Vec<T>,
    next_len: usize,
    available_bytes: u64,
    limit: &'static str,
) -> Result<(), WorkspaceError> {
    let item_bytes = allocation_bytes::<T>(1).max(1);
    let old_bytes = allocation_bytes::<T>(values.capacity());
    let maximum_new_bytes = available_bytes.saturating_sub(old_bytes);
    let maximum_values = usize::try_from(maximum_new_bytes / item_bytes).unwrap_or(usize::MAX);
    if next_len > maximum_values {
        return Err(WorkspaceError::ResourceLimit {
            limit,
            required: old_bytes.saturating_add(allocation_bytes::<T>(next_len)),
            allowed: available_bytes,
        });
    }
    let target = values
        .capacity()
        .saturating_mul(2)
        .max(MIN_WORKING_GROWTH_RECORDS)
        .min(maximum_values)
        .max(next_len);
    values
        .try_reserve_exact(target.saturating_sub(values.len()))
        .map_err(|_| WorkspaceError::ResourceLimit {
            limit,
            required: old_bytes.saturating_add(allocation_bytes::<T>(target)),
            allowed: available_bytes,
        })?;
    let overlap = old_bytes.saturating_add(allocation_bytes::<T>(values.capacity()));
    require_peak(overlap, available_bytes, limit)
}

#[cfg(test)]
mod tests {
    use point_contracts::{PointId, SourceId};

    use super::collect_point_ids;
    use crate::PointSetLimits;

    #[test]
    fn explicit_input_stops_after_the_first_over_limit_identity() {
        let source = SourceId::new([7; 32]);
        let mut seen = 0_u64;
        let input = std::iter::from_fn(|| {
            let ordinal = seen;
            seen += 1;
            Some(PointId::new(source, ordinal))
        });
        let defaults = PointSetLimits::default();
        let limits = PointSetLimits::new(
            defaults.candidate_limits(),
            defaults.source_read_budget(),
            2,
            defaults.max_output_points(),
            defaults.max_overlay_segments(),
            defaults.max_overlay_bytes(),
            defaults.max_working_bytes(),
            defaults.max_resident_bytes(),
            defaults.max_temporary_bytes(),
        );

        let error = collect_point_ids(input, limits).unwrap_err();

        assert!(matches!(
            error,
            crate::WorkspaceError::ResourceLimit {
                limit: "input Point Identities",
                required: 3,
                allowed: 2
            }
        ));
        assert_eq!(seen, 3);
    }
}

#[cfg(test)]
mod allocation_tests {
    use std::{
        hint::black_box,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use foundation_runtime::OperationControl;
    use point_contracts::{
        AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
        AttributeValues, CoordinateReference, PositionTransform,
    };
    use point_index::{PrepareLimits, prepare};
    use source_memory::MemorySource;

    use super::run_select;
    use crate::{OpenLimits, PointQuery, PointSetLimits, WorkspaceSchema, create};

    const TEST_POINTS: usize = 131_073;
    const MAX_SELECTION_PEAK_HEAP_BYTES: u64 = 64 * 1024 * 1024;
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn exact_selection_respects_measured_worker_equivalent_peak_heap() {
        let directory = temporary_directory();
        std::fs::create_dir(&directory).expect("create selection allocation-test directory");
        let classification = AttributeId::new(911).unwrap();
        let source = measured_source(classification);
        let index = prepare(
            source,
            directory.join("measured.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("prepare measured selection index");
        let workspace = create(
            directory.join("measured.pcw"),
            index,
            WorkspaceSchema::new(classification),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create measured selection Workspace");
        let snapshot = workspace.head();

        let allocations = allocation_counter::measure(|| {
            let selected = run_select(
                &snapshot,
                PointQuery::all(),
                PointSetLimits::default(),
                &OperationControl::new(),
            )
            .expect("measured exact selection succeeds");
            assert_eq!(
                selected.metadata().exact_count(),
                u64::try_from(TEST_POINTS).expect("test count fits u64")
            );
            black_box(selected.metadata());
        });

        assert!(
            allocations.bytes_max <= MAX_SELECTION_PEAK_HEAP_BYTES,
            "selection peak heap {} exceeded {} bytes",
            allocations.bytes_max,
            MAX_SELECTION_PEAK_HEAP_BYTES
        );
        assert_eq!(allocations.bytes_current, 0, "retained measured heap bytes");
        assert_eq!(
            allocations.count_current, 0,
            "retained measured heap allocations"
        );
        eprintln!(
            "point-workspace measured exact-selection peak heap: {} bytes (ceiling: {})",
            allocations.bytes_max, MAX_SELECTION_PEAK_HEAP_BYTES
        );

        drop(snapshot);
        drop(workspace);
        std::fs::remove_dir_all(&directory).expect("remove selection allocation-test directory");
    }

    fn measured_source(classification: AttributeId) -> point_source::Source {
        let ticks = (0..TEST_POINTS)
            .map(|ordinal| {
                let ordinal = i64::try_from(ordinal).expect("test ordinal fits i64");
                [ordinal, ordinal.rem_euclid(4_093), ordinal / 257]
            })
            .collect::<Vec<_>>();
        let values = (0..TEST_POINTS)
            .map(|ordinal| u8::try_from(ordinal % 8).expect("classification fits u8"))
            .collect::<Vec<_>>();
        let definition =
            AttributeDefinition::new(classification, "classification", AttributeDataType::U8)
                .expect("classification definition is valid");
        let column = AttributeColumn::new(definition, AttributeValues::u8(values))
            .expect("classification column is valid");
        let attributes =
            AttributeColumns::new(vec![column], TEST_POINTS).expect("classification rows align");
        let input = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [0.001; 3]).expect("test transform is valid"),
            CoordinateReference::Unknown,
            ticks,
            attributes,
        )
        .expect("measured memory Source is valid");
        source_memory::open(input)
            .blocking_wait()
            .expect("open measured memory Source")
    }

    fn temporary_directory() -> PathBuf {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "punctra-workspace-selection-allocation-{}-{sequence}",
            std::process::id()
        ))
    }
}
