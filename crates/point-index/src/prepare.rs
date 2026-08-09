use std::{collections::BinaryHeap, mem, path::PathBuf};

use foundation_runtime::{Job, OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::{PositionTransform, WorldBounds};
use point_source::{AttributeSelection, ReadBudget, ReadRequest, Source, SourceSpan};

use crate::{
    IndexError, PrepareDisposition, PrepareLimits, PrepareReport, PreparedIndex,
    persistence::{
        WorkFile, finalize, open_complete, open_or_create_work, ordinal_priority, target_exists,
    },
    read::IndexSample,
    tree::{self, BLOCK_POINTS, MAX_NODE_SAMPLES, SAMPLE_BYTES},
};

pub(crate) fn start(source: Source, target: PathBuf, limits: PrepareLimits) -> crate::IndexJob {
    Job::spawn(move |control| run(source, &target, limits, &control))
}

fn run(
    source: Source,
    target: &std::path::Path,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<PreparedIndex, IndexError> {
    control.check_cancelled()?;
    if target_exists(target)? {
        let opened = open_complete(&source, target, limits, control)?;
        let artifact_bytes = opened.artifact_bytes;
        control.complete_progress(source.metadata().point_count())?;
        return Ok(publish(
            source,
            opened,
            PrepareReport {
                disposition: PrepareDisposition::Opened,
                durable_points_reused: 0,
                source_points_read: 0,
                artifact_bytes,
            },
        ));
    }

    let mut work = open_or_create_work(&source, target, limits, control)?;
    let durable_points = work.durable_points();
    report_progress(control, durable_points, source.metadata().point_count())?;
    build_missing_blocks(&source, &mut work, limits, control)?;
    let plan = tree::plan(work.leaves(), limits, control)?;
    finalize(&source, target, &mut work, &plan, limits, control)?;
    drop(plan);
    drop(work);
    let opened = open_complete(&source, target, limits, control)?;
    control.complete_progress(source.metadata().point_count())?;
    let disposition = if durable_points == 0 {
        PrepareDisposition::Built
    } else {
        PrepareDisposition::Resumed
    };
    let report = PrepareReport {
        disposition,
        durable_points_reused: durable_points,
        source_points_read: source
            .metadata()
            .point_count()
            .saturating_sub(durable_points),
        artifact_bytes: opened.artifact_bytes,
    };
    Ok(publish(source, opened, report))
}

fn publish(
    source: Source,
    opened: crate::persistence::OpenArtifact,
    prepare_report: PrepareReport,
) -> PreparedIndex {
    PreparedIndex {
        source,
        descriptor: opened.descriptor,
        hierarchy: opened.hierarchy,
        prepare_report,
        artifact: opened.reader,
    }
}

fn build_missing_blocks(
    source: &Source,
    work: &mut WorkFile,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<(), IndexError> {
    let point_count = source.metadata().point_count();
    while work.durable_points() < point_count {
        control.check_cancelled()?;
        let first = work.durable_points();
        let count = point_count.saturating_sub(first).min(BLOCK_POINTS);
        let span = SourceSpan::new(first, count)?;
        let (bounds, samples) = read_block(
            source,
            span,
            limits,
            work.retained_metadata_bytes(),
            control,
        )?;
        control.check_cancelled()?;
        work.append_block(span, bounds, &samples, limits)?;
        report_progress(control, work.durable_points(), point_count)?;
    }
    Ok(())
}

fn read_block(
    source: &Source,
    span: SourceSpan,
    limits: PrepareLimits,
    retained_build_bytes: u64,
    control: &OperationControl,
) -> Result<(WorldBounds, Vec<IndexSample>), IndexError> {
    let budget = ReadBudget::new(
        limits.max_source_batch_points(),
        limits.max_source_batch_payload_bytes(),
    )?
    .with_max_spans(1)
    .with_max_points(span.point_count())
    .with_max_adapter_working_bytes(limits.max_adapter_working_bytes());
    let request = ReadRequest::all()
        .spans([span])
        .attributes(AttributeSelection::only([]))
        .budget(budget);
    let mut batches = source.read(request)?;
    let mut accumulator = BlockAccumulator::new(
        source.metadata().position_transform(),
        span.point_count(),
        retained_build_bytes,
        limits,
    )?;
    loop {
        if let Err(error) = control.check_cancelled() {
            batches.handle().cancel();
            return Err(error.into());
        }
        let next = batches.next();
        if let Err(error) = control.check_cancelled() {
            batches.handle().cancel();
            return Err(error.into());
        }
        match next {
            Ok(Some(batch)) => {
                let first = batch.first_ordinal();
                for (row, ticks) in batch.positions().ticks().iter().copied().enumerate() {
                    if row % 4_096 == 0 {
                        control.check_cancelled()?;
                    }
                    let row = u64::try_from(row).expect("Source batch rows fit u64");
                    accumulator.push(first + row, ticks)?;
                }
            }
            Ok(None) => break,
            Err(error) => return Err(error.into()),
        }
    }
    let summary = batches.summary().ok_or(IndexError::CorruptWork {
        reason: "Source block read ended without a terminal summary",
    })?;
    if summary.source() != source.identity()
        || summary.provenance() != source.provenance()
        || summary.exact_count() != span.point_count()
        || summary.spans() != [span]
        || !summary.attributes().is_empty()
    {
        return Err(IndexError::CorruptWork {
            reason: "Source block summary differs from its index request",
        });
    }
    accumulator.finish()
}

struct BlockAccumulator {
    transform: PositionTransform,
    expected_points: u64,
    accepted_points: u64,
    minimum: [f64; 3],
    maximum: [f64; 3],
    selected: BinaryHeap<(u64, u64, [i64; 3])>,
    selection_limit: usize,
    retained_build_bytes: u64,
    max_build_working_bytes: u64,
}

impl BlockAccumulator {
    fn new(
        transform: PositionTransform,
        expected_points: u64,
        retained_build_bytes: u64,
        limits: PrepareLimits,
    ) -> Result<Self, IndexError> {
        let retained = expected_points.min(MAX_NODE_SAMPLES);
        let heap_bytes = retained.saturating_mul(
            u64::try_from(mem::size_of::<(u64, u64, [i64; 3])>()).unwrap_or(u64::MAX),
        );
        let output_bytes = retained.saturating_mul(SAMPLE_BYTES);
        let required = retained_build_bytes
            .saturating_add(heap_bytes)
            .saturating_add(output_bytes);
        if required > limits.max_build_working_bytes() {
            return Err(IndexError::ResourceLimit {
                limit: "build working bytes",
                required,
                allowed: limits.max_build_working_bytes(),
            });
        }
        let capacity = usize::try_from(retained).map_err(|_| IndexError::ResourceLimit {
            limit: "addressable sample Points",
            required: retained,
            allowed: usize::MAX as u64,
        })?;
        let mut selected = BinaryHeap::new();
        selected
            .try_reserve_exact(capacity)
            .map_err(|_| IndexError::ResourceLimit {
                limit: "build working bytes",
                required: heap_bytes,
                allowed: limits.max_build_working_bytes(),
            })?;
        let actual_heap_bytes = u64::try_from(selected.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(
                u64::try_from(mem::size_of::<(u64, u64, [i64; 3])>()).unwrap_or(u64::MAX),
            );
        let actual_required = retained_build_bytes
            .saturating_add(actual_heap_bytes)
            .saturating_add(output_bytes);
        if actual_required > limits.max_build_working_bytes() {
            return Err(IndexError::ResourceLimit {
                limit: "build working bytes",
                required: actual_required,
                allowed: limits.max_build_working_bytes(),
            });
        }
        Ok(Self {
            transform,
            expected_points,
            accepted_points: 0,
            minimum: [f64::INFINITY; 3],
            maximum: [f64::NEG_INFINITY; 3],
            selected,
            selection_limit: capacity,
            retained_build_bytes,
            max_build_working_bytes: limits.max_build_working_bytes(),
        })
    }

    fn push(&mut self, ordinal: u64, ticks: [i64; 3]) -> Result<(), IndexError> {
        self.accepted_points =
            self.accepted_points
                .checked_add(1)
                .ok_or(IndexError::CorruptWork {
                    reason: "Source block Point count overflowed",
                })?;
        if self.accepted_points > self.expected_points {
            return Err(IndexError::CorruptWork {
                reason: "Source block emitted too many Points",
            });
        }
        let world = self.transform.world_f64(ticks);
        for (axis, coordinate) in world.into_iter().enumerate() {
            if !coordinate.is_finite() {
                return Err(IndexError::CorruptWork {
                    reason: "Source block contains a non-finite world position",
                });
            }
            self.minimum[axis] = self.minimum[axis].min(coordinate);
            self.maximum[axis] = self.maximum[axis].max(coordinate);
        }
        let value = (ordinal_priority(ordinal), ordinal, ticks);
        if self.selected.len() < self.selection_limit {
            self.selected.push(value);
        } else if self.selected.peek().is_some_and(|largest| &value < largest) {
            let _ = self.selected.pop();
            self.selected.push(value);
        }
        Ok(())
    }

    fn finish(self) -> Result<(WorldBounds, Vec<IndexSample>), IndexError> {
        if self.accepted_points != self.expected_points {
            return Err(IndexError::CorruptWork {
                reason: "Source block ended before every Point was accepted",
            });
        }
        let bounds =
            WorldBounds::new(self.minimum, self.maximum).map_err(|_| IndexError::CorruptWork {
                reason: "Source block bounds are invalid",
            })?;
        let selection_limit = self.selection_limit;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(selection_limit)
            .map_err(|_| IndexError::ResourceLimit {
                limit: "build working bytes",
                required: u64::try_from(selection_limit)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(SAMPLE_BYTES),
                allowed: self.max_build_working_bytes,
            })?;
        let heap_bytes = u64::try_from(self.selected.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(
                u64::try_from(mem::size_of::<(u64, u64, [i64; 3])>()).unwrap_or(u64::MAX),
            );
        let output_bytes = u64::try_from(samples.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(SAMPLE_BYTES);
        let required = self
            .retained_build_bytes
            .saturating_add(heap_bytes)
            .saturating_add(output_bytes);
        if required > self.max_build_working_bytes {
            return Err(IndexError::ResourceLimit {
                limit: "build working bytes",
                required,
                allowed: self.max_build_working_bytes,
            });
        }
        samples.extend(
            self.selected
                .into_iter()
                .map(|(_, ordinal, ticks)| IndexSample::new(ordinal, ticks)),
        );
        samples.sort_unstable_by_key(|sample| sample.ordinal());
        Ok((bounds, samples))
    }
}

fn report_progress(
    control: &OperationControl,
    completed: u64,
    total: u64,
) -> Result<(), IndexError> {
    let progress = ProgressSnapshot::new(ProgressPhase::RUNNING, completed, Some(total))?;
    control.report_progress(progress)?;
    Ok(())
}

#[cfg(test)]
mod allocation_tests {
    use std::{hint::black_box, path::PathBuf, time::SystemTime};

    use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
    use source_memory::MemorySource;

    use super::*;

    const TEST_POINTS: usize = 131_073;
    const MAX_PREPARE_PEAK_HEAP_BYTES: u64 = 64 * 1024 * 1024;

    #[test]
    fn cold_and_warm_prepare_respect_measured_peak_heap_and_release_allocations() {
        let source = measured_source();
        let directory = temporary_directory();
        std::fs::create_dir(&directory).expect("create allocation-test directory");
        let target = directory.join("measured.pidx");

        let cold = allocation_counter::measure(|| {
            let prepared = run(
                source.clone(),
                &target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
            .expect("measured cold prepare succeeds");
            assert_eq!(
                prepared.prepare_report().disposition(),
                PrepareDisposition::Built
            );
            black_box(prepared.descriptor().artifact_checksum());
        });
        assert_allocation_gate("cold prepare", cold);

        let warm = allocation_counter::measure(|| {
            let prepared = run(
                source.clone(),
                &target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
            .expect("measured warm open succeeds");
            assert_eq!(
                prepared.prepare_report().disposition(),
                PrepareDisposition::Opened
            );
            black_box(prepared.hierarchy().nodes().len());
        });
        assert_allocation_gate("warm open", warm);
        std::fs::remove_dir_all(&directory).expect("remove allocation-test directory");
    }

    fn assert_allocation_gate(label: &str, allocations: allocation_counter::AllocationInfo) {
        assert!(
            allocations.bytes_max <= MAX_PREPARE_PEAK_HEAP_BYTES,
            "{label} peak heap {} exceeded {} bytes",
            allocations.bytes_max,
            MAX_PREPARE_PEAK_HEAP_BYTES
        );
        assert_eq!(
            allocations.bytes_current, 0,
            "{label} retained measured heap bytes"
        );
        assert_eq!(
            allocations.count_current, 0,
            "{label} retained measured allocations"
        );
        eprintln!(
            "{label} measured peak heap: {} bytes (ceiling: {})",
            allocations.bytes_max, MAX_PREPARE_PEAK_HEAP_BYTES
        );
    }

    fn measured_source() -> Source {
        let ticks = (0..TEST_POINTS)
            .map(|ordinal| {
                let ordinal = i64::try_from(ordinal).expect("test ordinal fits i64");
                [ordinal, ordinal % 4_093, ordinal / 257]
            })
            .collect::<Vec<_>>();
        let input = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [0.001; 3]).expect("test transform is valid"),
            CoordinateReference::Unknown,
            ticks,
            AttributeColumns::empty(TEST_POINTS),
        )
        .expect("allocation-test Source is valid");
        source_memory::open(input)
            .blocking_wait()
            .expect("allocation-test Source opens")
    }

    fn temporary_directory() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "punctra-point-index-allocation-{}-{timestamp}",
            std::process::id()
        ))
    }
}
