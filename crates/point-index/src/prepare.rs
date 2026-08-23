use std::{collections::BinaryHeap, mem, path::PathBuf};

use foundation_runtime::{Job, OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::{PositionTransform, WorldBounds};
use point_source::{AttributeSelection, ReadBudget, ReadRequest, Source, SourceSpan};

use crate::{
    DisplayAttributes, DisplaySampleContract, IndexError, IndexLimit, IndexRecipe,
    PrepareDisposition, PrepareLimits, PrepareReport, PreparedIndex,
    persistence::{
        WorkFile, finalize, open_complete, open_or_create_work, ordinal_priority, target_exists,
    },
    read::{StoredSample, attributes_at},
    tree::{self, BLOCK_POINTS, MAX_NODE_SAMPLES},
};

pub(crate) fn start(
    source: Source,
    target: PathBuf,
    recipe: IndexRecipe,
    limits: PrepareLimits,
) -> crate::IndexJob {
    Job::spawn(move |control| {
        run_with_allocation_probe(|| {
            run(
                source,
                &target,
                recipe,
                limits,
                PreparationPolicy::ResumeOrOpen,
                &control,
            )
        })
    })
}

pub(crate) fn start_fresh(
    source: Source,
    target: PathBuf,
    recipe: IndexRecipe,
    limits: PrepareLimits,
) -> crate::IndexJob {
    Job::spawn(move |control| {
        run_with_allocation_probe(|| {
            run(
                source,
                &target,
                recipe,
                limits,
                PreparationPolicy::Fresh,
                &control,
            )
        })
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PreparationPolicy {
    ResumeOrOpen,
    Fresh,
}

#[cfg(not(test))]
fn run_with_allocation_probe(
    run: impl FnOnce() -> Result<PreparedIndex, IndexError>,
) -> Result<PreparedIndex, IndexError> {
    run()
}

#[cfg(test)]
fn run_with_allocation_probe(
    run: impl FnOnce() -> Result<PreparedIndex, IndexError>,
) -> Result<PreparedIndex, IndexError> {
    allocation_probe::measure_if_armed(run)
}

fn run(
    source: Source,
    target: &std::path::Path,
    recipe: IndexRecipe,
    limits: PrepareLimits,
    policy: PreparationPolicy,
    control: &OperationControl,
) -> Result<PreparedIndex, IndexError> {
    run_with_completion_hook(source, target, recipe, limits, policy, control, |_| Ok(()))
}

fn run_with_completion_hook(
    source: Source,
    target: &std::path::Path,
    recipe: IndexRecipe,
    limits: PrepareLimits,
    policy: PreparationPolicy,
    control: &OperationControl,
    mut before_terminal_binding: impl FnMut(&std::path::Path) -> Result<(), IndexError>,
) -> Result<PreparedIndex, IndexError> {
    control.check_cancelled()?;
    let contract = recipe.resolve_contract(source.metadata())?;
    if target_exists(target)? {
        if policy == PreparationPolicy::Fresh {
            return Err(IndexError::IncompatibleArtifact {
                reason: "fresh preparation target already exists",
            });
        }
        let opened = open_complete(&source, target, recipe, limits, control)?;
        let artifact_bytes = opened.artifact_bytes;
        control.complete_progress(source.metadata().point_count())?;
        before_terminal_binding(target)?;
        opened.verify_path_binding()?;
        return Ok(publish(
            source,
            opened,
            PrepareReport {
                disposition: PrepareDisposition::Opened,
                durable_points_reused: 0,
                source_points_read: 0,
                artifact_bytes,
                peak_temporary_disk_bytes: 0,
            },
        ));
    }

    let mut work = open_or_create_work(
        &source,
        target,
        recipe,
        contract,
        limits,
        policy == PreparationPolicy::ResumeOrOpen,
        control,
    )?;
    let durable_points = work.durable_points();
    report_progress(control, durable_points, source.metadata().point_count())?;
    build_missing_blocks(&source, &mut work, contract, limits, control)?;
    let plan = tree::plan(
        work.leaves(),
        work.leaf_capacity(),
        recipe.sample_bytes(),
        limits,
        control,
    )?;
    finalize(&source, target, &mut work, &plan, limits, control)?;
    let peak_temporary_disk_bytes = work.peak_temporary_disk_bytes();
    drop(plan);
    let opened = open_complete(&source, target, recipe, limits, control)?;
    control.complete_progress(source.metadata().point_count())?;
    before_terminal_binding(work.path())?;
    opened.verify_path_binding()?;
    work.verify_path_binding()?;
    drop(work);
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
        peak_temporary_disk_bytes,
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
    contract: Option<DisplaySampleContract>,
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
            contract,
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
    contract: Option<DisplaySampleContract>,
    limits: PrepareLimits,
    retained_build_bytes: u64,
    control: &OperationControl,
) -> Result<(WorldBounds, Vec<StoredSample>), IndexError> {
    let budget = ReadBudget::new(
        limits.max_source_batch_points(),
        limits.max_source_batch_payload_bytes(),
    )?
    .with_max_spans(1)
    .with_max_points(span.point_count())
    .with_max_adapter_working_bytes(limits.max_adapter_working_bytes());
    let attributes = match contract {
        Some(contract) => AttributeSelection::only(contract.selected_ids()),
        None => AttributeSelection::only([]),
    };
    let request = ReadRequest::all()
        .spans([span])
        .attributes(attributes)
        .budget(budget);
    let mut batches = source.read(request)?;
    let mut accumulator = BlockAccumulator::new(
        source.metadata().position_transform(),
        span.point_count(),
        retained_build_bytes,
        contract.is_some(),
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
                    let attributes = contract
                        .map(|contract| attributes_at(batch.attributes(), row, contract))
                        .transpose()?;
                    let ordinal_row = u64::try_from(row).expect("Source batch rows fit u64");
                    accumulator.push(first + ordinal_row, ticks, attributes)?;
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
        || !selected_attributes_match(contract, summary.attributes())
    {
        return Err(IndexError::CorruptWork {
            reason: "Source block summary differs from its index request",
        });
    }
    accumulator.finish()
}

fn selected_attributes_match(
    contract: Option<DisplaySampleContract>,
    actual: &[point_contracts::AttributeId],
) -> bool {
    match contract {
        Some(contract) => actual.iter().copied().eq(contract.selected_ids()),
        None => actual.is_empty(),
    }
}

struct BlockAccumulator {
    transform: PositionTransform,
    expected_points: u64,
    accepted_points: u64,
    minimum: [f64; 3],
    maximum: [f64; 3],
    selected: SampleHeap,
    selection_limit: usize,
    retained_build_bytes: u64,
    max_build_working_bytes: u64,
}

enum SampleHeap {
    Position(BinaryHeap<(u64, u64, [i64; 3])>),
    Attributed(BinaryHeap<(u64, u64, [i64; 3], DisplayAttributes)>),
}

impl BlockAccumulator {
    fn new(
        transform: PositionTransform,
        expected_points: u64,
        retained_build_bytes: u64,
        attributed: bool,
        limits: PrepareLimits,
    ) -> Result<Self, IndexError> {
        let retained = expected_points.min(MAX_NODE_SAMPLES);
        let heap_item_bytes = if attributed {
            mem::size_of::<(u64, u64, [i64; 3], DisplayAttributes)>()
        } else {
            mem::size_of::<(u64, u64, [i64; 3])>()
        };
        let heap_bytes =
            retained.saturating_mul(u64::try_from(heap_item_bytes).unwrap_or(u64::MAX));
        let output_bytes = retained
            .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX));
        let required = retained_build_bytes
            .saturating_add(heap_bytes)
            .saturating_add(output_bytes);
        if required > limits.max_build_working_bytes() {
            return Err(IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required,
                allowed: limits.max_build_working_bytes(),
            });
        }
        let capacity = usize::try_from(retained).map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::AddressableSamplePoints,
            required: retained,
            allowed: usize::MAX as u64,
        })?;
        let selected = if attributed {
            let mut heap = BinaryHeap::new();
            heap.try_reserve_exact(capacity)
                .map_err(|_| IndexError::ResourceLimit {
                    limit: IndexLimit::BuildWorkingBytes,
                    required: heap_bytes,
                    allowed: limits.max_build_working_bytes(),
                })?;
            SampleHeap::Attributed(heap)
        } else {
            let mut heap = BinaryHeap::new();
            heap.try_reserve_exact(capacity)
                .map_err(|_| IndexError::ResourceLimit {
                    limit: IndexLimit::BuildWorkingBytes,
                    required: heap_bytes,
                    allowed: limits.max_build_working_bytes(),
                })?;
            SampleHeap::Position(heap)
        };
        let actual_heap_bytes = match &selected {
            SampleHeap::Position(heap) => heap_capacity_bytes(heap),
            SampleHeap::Attributed(heap) => heap_capacity_bytes(heap),
        };
        let actual_required = retained_build_bytes
            .saturating_add(actual_heap_bytes)
            .saturating_add(output_bytes);
        if actual_required > limits.max_build_working_bytes() {
            return Err(IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
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

    fn push(
        &mut self,
        ordinal: u64,
        ticks: [i64; 3],
        attributes: Option<DisplayAttributes>,
    ) -> Result<(), IndexError> {
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
        match (&mut self.selected, attributes) {
            (SampleHeap::Position(heap), None) => retain_selected(
                heap,
                (ordinal_priority(ordinal), ordinal, ticks),
                self.selection_limit,
            ),
            (SampleHeap::Attributed(heap), Some(attributes)) => retain_selected(
                heap,
                (ordinal_priority(ordinal), ordinal, ticks, attributes),
                self.selection_limit,
            ),
            (SampleHeap::Position(heap), Some(attributes)) if heap.is_empty() => {
                let capacity = heap.capacity();
                let mut attributed_heap = BinaryHeap::new();
                attributed_heap.try_reserve_exact(capacity).map_err(|_| {
                    IndexError::ResourceLimit {
                        limit: IndexLimit::BuildWorkingBytes,
                        required:
                            u64::try_from(capacity).unwrap_or(u64::MAX).saturating_mul(
                                u64::try_from(mem::size_of::<(
                                    u64,
                                    u64,
                                    [i64; 3],
                                    DisplayAttributes,
                                )>())
                                .unwrap_or(u64::MAX),
                            ),
                        allowed: self.max_build_working_bytes,
                    }
                })?;
                retain_selected(
                    &mut attributed_heap,
                    (ordinal_priority(ordinal), ordinal, ticks, attributes),
                    self.selection_limit,
                );
                self.selected = SampleHeap::Attributed(attributed_heap);
            }
            _ => {
                return Err(IndexError::CorruptWork {
                    reason: "Source block mixed attributed and position-only rows",
                });
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<(WorldBounds, Vec<StoredSample>), IndexError> {
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
                limit: IndexLimit::BuildWorkingBytes,
                required: u64::try_from(selection_limit)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(
                        u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX),
                    ),
                allowed: self.max_build_working_bytes,
            })?;
        let heap_bytes = match &self.selected {
            SampleHeap::Position(heap) => heap_capacity_bytes(heap),
            SampleHeap::Attributed(heap) => heap_capacity_bytes(heap),
        };
        let output_bytes = u64::try_from(samples.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX));
        let required = self
            .retained_build_bytes
            .saturating_add(heap_bytes)
            .saturating_add(output_bytes);
        if required > self.max_build_working_bytes {
            return Err(IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required,
                allowed: self.max_build_working_bytes,
            });
        }
        match self.selected {
            SampleHeap::Position(heap) => samples.extend(
                heap.into_iter()
                    .map(|(_, ordinal, ticks)| StoredSample::position_only(ordinal, ticks)),
            ),
            SampleHeap::Attributed(heap) => {
                samples.extend(heap.into_iter().map(|(_, ordinal, ticks, attributes)| {
                    StoredSample::attributed(ordinal, ticks, attributes)
                }));
            }
        }
        samples.sort_unstable_by_key(|sample| sample.ordinal());
        Ok((bounds, samples))
    }
}

fn retain_selected<T: Ord>(heap: &mut BinaryHeap<T>, value: T, capacity: usize) {
    if heap.len() < capacity {
        heap.push(value);
    } else if heap.peek().is_some_and(|largest| &value < largest) {
        let _ = heap.pop();
        heap.push(value);
    }
}

fn heap_capacity_bytes<T>(heap: &BinaryHeap<T>) -> u64 {
    u64::try_from(heap.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
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
mod allocation_probe {
    use std::sync::{Mutex, mpsc};

    use super::*;

    static REPORTER: Mutex<Option<mpsc::SyncSender<allocation_counter::AllocationInfo>>> =
        Mutex::new(None);

    pub(super) fn arm() -> mpsc::Receiver<allocation_counter::AllocationInfo> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut reporter = REPORTER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            reporter.replace(sender).is_none(),
            "allocation probe was already armed"
        );
        receiver
    }

    pub(super) fn measure_if_armed(
        run: impl FnOnce() -> Result<PreparedIndex, IndexError>,
    ) -> Result<PreparedIndex, IndexError> {
        let sender = REPORTER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(sender) = sender else {
            return run();
        };

        let mut result = None;
        let allocations = allocation_counter::measure(|| result = Some(run()));
        sender
            .send(allocations)
            .expect("allocation-test receiver remains available");
        result.expect("measured preparation produced a result")
    }
}

#[cfg(test)]
mod allocation_tests {
    use std::{
        hint::black_box,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
    use source_memory::MemorySource;

    use super::*;

    const TEST_POINTS: usize = 131_073;
    const MAX_PREPARE_PEAK_HEAP_BYTES: u64 = 64 * 1024 * 1024;
    static TEMPORARY_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn fresh_prepare_rejects_a_work_path_replaced_before_acknowledgement() {
        let source = empty_source();
        let directory = temporary_directory();
        std::fs::create_dir(&directory).expect("create binding-test directory");
        let target = directory.join("fresh.pidx");
        let moved_work = directory.join("owned-work-moved-aside");
        let sentinel = b"caller replacement installed before acknowledgement";
        let control = OperationControl::new();

        let result = run_with_completion_hook(
            source,
            &target,
            IndexRecipe::PositionOnlyV1,
            PrepareLimits::default(),
            PreparationPolicy::Fresh,
            &control,
            |work_path| {
                std::fs::rename(work_path, &moved_work)
                    .expect("move the open owned work file aside");
                std::fs::write(work_path, sentinel).expect("install caller replacement");
                Ok(())
            },
        );
        let Err(error) = result else {
            panic!("fresh preparation must reject a replaced work path")
        };

        assert!(matches!(
            error,
            IndexError::IncompatibleWork {
                reason: "work path changed before preparation acknowledgement"
            }
        ));
        let work_path = target.with_extension("pidx.work");
        assert_eq!(
            std::fs::read(&work_path).expect("caller replacement remains"),
            sentinel
        );
        assert_eq!(
            std::fs::metadata(&moved_work)
                .expect("owned work inode remains")
                .len(),
            200
        );
        std::fs::remove_dir_all(&directory).expect("remove binding-test directory");
    }

    #[test]
    fn warm_prepare_rejects_an_artifact_replaced_before_acknowledgement() {
        let source = empty_source();
        let directory = temporary_directory();
        std::fs::create_dir(&directory).expect("create binding-test directory");
        let target = directory.join("warm.pidx");
        let moved_target = directory.join("opened-artifact-moved-aside");
        crate::prepare(source.clone(), &target, PrepareLimits::default())
            .blocking_wait()
            .expect("fixture index builds");
        let artifact_bytes = std::fs::metadata(&target)
            .expect("inspect fixture artifact")
            .len();
        let control = OperationControl::new();

        let result = run_with_completion_hook(
            source,
            &target,
            IndexRecipe::PositionOnlyV1,
            PrepareLimits::default(),
            PreparationPolicy::ResumeOrOpen,
            &control,
            |_| {
                std::fs::rename(&target, &moved_target).expect("move the verified artifact aside");
                let replacement =
                    std::fs::File::create(&target).expect("create same-length caller replacement");
                replacement
                    .set_len(artifact_bytes)
                    .expect("size caller replacement");
                Ok(())
            },
        );
        let Err(error) = result else {
            panic!("warm preparation must reject a replaced artifact path")
        };

        assert!(matches!(
            error,
            IndexError::CorruptArtifact {
                reason: "artifact path changed before preparation acknowledgement"
            }
        ));
        assert_eq!(
            std::fs::metadata(&target)
                .expect("caller replacement remains")
                .len(),
            artifact_bytes
        );
        assert_eq!(
            std::fs::metadata(&moved_target)
                .expect("verified artifact remains")
                .len(),
            artifact_bytes
        );
        std::fs::remove_dir_all(&directory).expect("remove binding-test directory");
    }

    #[test]
    fn cold_and_warm_public_prepare_respect_measured_worker_peak_heap() {
        let source = measured_source();
        let directory = temporary_directory();
        std::fs::create_dir(&directory).expect("create allocation-test directory");
        let target = directory.join("measured.pidx");

        let cold_measurement = allocation_probe::arm();
        let prepared = crate::prepare(source.clone(), &target, PrepareLimits::default())
            .blocking_wait()
            .expect("measured cold prepare succeeds");
        assert_eq!(
            prepared.prepare_report().disposition(),
            PrepareDisposition::Built
        );
        black_box(prepared.descriptor().artifact_checksum());
        drop(prepared);
        let cold = cold_measurement
            .recv()
            .expect("cold worker publishes its allocation measurement");
        assert_allocation_gate("cold prepare", cold);

        let warm_measurement = allocation_probe::arm();
        let prepared = crate::prepare(source.clone(), &target, PrepareLimits::default())
            .blocking_wait()
            .expect("measured warm open succeeds");
        assert_eq!(
            prepared.prepare_report().disposition(),
            PrepareDisposition::Opened
        );
        black_box(prepared.hierarchy().nodes().len());
        drop(prepared);
        let warm = warm_measurement
            .recv()
            .expect("warm worker publishes its allocation measurement");
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
        eprintln!(
            "{label} measured worker peak heap: {} bytes (ceiling: {}; published result: {} bytes)",
            allocations.bytes_max,
            MAX_PREPARE_PEAK_HEAP_BYTES,
            allocations.bytes_current.max(0),
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

    fn empty_source() -> Source {
        let input = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [1.0; 3]).expect("identity transform is valid"),
            CoordinateReference::Unknown,
            Vec::new(),
            AttributeColumns::empty(0),
        )
        .expect("empty Source fixture is valid");
        source_memory::open(input)
            .blocking_wait()
            .expect("empty Source fixture opens")
    }

    fn temporary_directory() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = TEMPORARY_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "punctra-point-index-allocation-{}-{timestamp}-{sequence}",
            std::process::id()
        ))
    }
}
