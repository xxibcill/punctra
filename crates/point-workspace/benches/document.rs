//! One-machine benchmark for the narrow v0.5 durable document core.
//!
//! The default fixture contains 1,000,000 Points. Set
//! `PUNCTRA_POINT_WORKSPACE_BENCH_POINTS=10000000` (or another larger positive
//! count) for an opt-in larger generated run. Reported process RSS is sampled
//! around the worker-owned selection; it is not a caller-thread allocation
//! counter and is not presented as a universal memory bound. Worker peak heap
//! remains explicitly unclaimed until instrumentation runs inside that worker.

#![allow(missing_docs)]

use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform, WorldBounds,
};
use point_index::{CandidateLimits, PrepareLimits, PreparedIndex, prepare};
use point_source::ReadBudget;
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, PointIdReadLimits,
    PointQuery, PointSetLimits, RevisionKind, Workspace, WorkspaceSchema, create, open,
};
use source_memory::MemorySource;

const POINT_COUNT_ENV: &str = "PUNCTRA_POINT_WORKSPACE_BENCH_POINTS";
const DEFAULT_POINT_COUNT: usize = 1_000_000;
const CLASSIFICATION_ID: u32 = 101;
const MIB: u64 = 1024 * 1024;

fn benchmark_document(criterion: &mut Criterion) {
    let fixture = Fixture::new(configured_point_count());
    report_resource_and_revision_evidence(&fixture);

    let point_count = u64::try_from(fixture.point_count).expect("benchmark count fits u64");
    let half_count = point_count / 2;
    let one_percent = point_count.div_ceil(100);
    let empty_bounds = WorldBounds::new(
        [exact_f64(point_count) + 10.0, 0.0, 0.0],
        [exact_f64(point_count) + 20.0, 4_095.0, 2_047.0],
    )
    .expect("empty benchmark bounds are valid");
    let half_bounds = WorldBounds::new(
        [0.0, 0.0, 0.0],
        [exact_f64(half_count.saturating_sub(1)), 4_095.0, 2_047.0],
    )
    .expect("half benchmark bounds are valid");
    let root = fixture.workspace.head();
    let mut group = criterion.benchmark_group("point_workspace_exact_selection");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(5));

    for (name, query, expected) in [
        ("0_percent_resident", PointQuery::within(empty_bounds), 0),
        (
            "1_percent_resident",
            PointQuery::all().classification_is(0),
            one_percent,
        ),
        (
            "50_percent_resident",
            PointQuery::within(half_bounds),
            half_count,
        ),
        ("100_percent_resident", PointQuery::all(), point_count),
    ] {
        group.throughput(Throughput::Elements(point_count));
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                select_and_assert(&root, black_box(query), resident_limits(), expected);
            });
        });
    }
    group.throughput(Throughput::Elements(point_count));
    group.bench_function("100_percent_forced_spill", |bencher| {
        bencher.iter(|| {
            select_and_assert(&root, PointQuery::all(), forced_spill_limits(), point_count);
        });
    });
    group.finish();
}

fn select_and_assert(
    snapshot: &point_workspace::Snapshot,
    query: PointQuery,
    limits: PointSetLimits,
    expected: u64,
) {
    let point_set = snapshot
        .select(query, limits)
        .blocking_wait()
        .expect("benchmark exact selection succeeds");
    assert_eq!(point_set.metadata().exact_count(), expected);
    black_box(point_set.metadata());
}

fn report_resource_and_revision_evidence(fixture: &Fixture) {
    let workspace_path = fixture.directory.path().join("resource.pcw");
    let workspace = create(
        &workspace_path,
        fixture.index.clone(),
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("resource-report Workspace creates");
    let point_count = u64::try_from(fixture.point_count).expect("benchmark count fits u64");
    report_selection_resources(&workspace, &workspace_path, point_count);
    report_revision_resources(fixture, workspace, &workspace_path);
}

fn report_selection_resources(workspace: &Workspace, workspace_path: &Path, point_count: u64) {
    let root = workspace.head();
    let resident = root
        .select(PointQuery::all(), resident_limits())
        .blocking_wait()
        .expect("resource-report resident selection succeeds");
    let resident_rss = process_rss_bytes().expect("sample resident-selection process RSS with ps");
    black_box(resident.metadata());
    let identity_allocations = allocation_counter::measure(|| {
        let mut batches = resident
            .ids(PointIdReadLimits::default())
            .expect("resource-report identity stream opens");
        while let Some(batch) = batches
            .next()
            .expect("resource-report identity batch validates")
        {
            black_box(batch);
        }
    });
    eprintln!(
        "point-workspace synchronous Point-ID iteration allocations: peak_bytes={} total_bytes={} retained_bytes={} (caller thread only; excludes worker-owned selection)",
        identity_allocations.bytes_max,
        identity_allocations.bytes_total,
        identity_allocations.bytes_current
    );
    assert_eq!(identity_allocations.count_current, 0);
    assert_eq!(identity_allocations.bytes_current, 0);
    drop(resident);

    let rss_before_spill = process_rss_bytes().expect("sample pre-spill process RSS with ps");
    let spill_job = root.select(PointQuery::all(), forced_spill_limits());
    let handle = spill_job.handle();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut peak_rss = rss_before_spill;
    while handle.progress().phase() != foundation_runtime::ProgressPhase::COMPLETE {
        peak_rss = peak_rss.max(process_rss_bytes().expect("sample in-flight process RSS with ps"));
        assert!(
            Instant::now() < deadline,
            "resource-report forced-spill selection exceeded 120 seconds"
        );
        std::thread::yield_now();
    }
    let spilled = spill_job
        .blocking_wait()
        .expect("resource-report forced-spill selection succeeds");
    peak_rss =
        peak_rss.max(process_rss_bytes().expect("sample completed-spill process RSS with ps"));
    let scratch_path = workspace_path.join("scratch");
    assert_eq!(
        fs::read_dir(&scratch_path)
            .expect("read completed spill directory")
            .count(),
        1,
        "one append-only sealed spill owns the monotonically grown temporary payload"
    );
    let peak_temporary_bytes = logical_directory_entry_bytes(&scratch_path);
    assert_eq!(spilled.metadata().exact_count(), point_count);
    eprintln!(
        "point-workspace resource evidence: points={} resident_process_rss={} spill_baseline_rss={} spill_peak_process_rss={} spill_peak_rss_delta={} spill_peak_temporary_bytes={}",
        point_count,
        resident_rss,
        rss_before_spill,
        peak_rss,
        peak_rss.saturating_sub(rss_before_spill),
        peak_temporary_bytes
    );
    eprintln!(
        "point-workspace worker peak heap: unclaimed (process RSS is reported separately; synchronous allocation-counter evidence covers only public Point-ID iteration)"
    );
    drop(spilled);
    assert_eq!(logical_directory_entry_bytes(&scratch_path), 0);
}

fn report_revision_resources(fixture: &Fixture, mut workspace: Workspace, workspace_path: &Path) {
    let initial_durable = logical_directory_entry_bytes(workspace_path);
    let mut depth = 0_u64;
    let plans = [
        ("sparse_1_percent", EditSelection::Classification(0), 254),
        ("dense_50_percent", EditSelection::FirstHalf, 253),
        (
            "sparse_1_percent_repeat",
            EditSelection::Classification(0),
            252,
        ),
        ("dense_50_percent_repeat", EditSelection::FirstHalf, 251),
    ];
    for (index, (label, selection, value)) in plans.into_iter().enumerate() {
        let before = logical_directory_entry_bytes(workspace_path);
        let pair = run_edit_pair(
            &workspace,
            workspace_path,
            fixture.point_count,
            selection,
            value,
            u8::try_from(index * 2 + 1).expect("benchmark Operation byte fits u8"),
        );
        depth += 2;
        let after_set_growth = pair.durable_after_set.saturating_sub(before);
        let changed = pair.changed.max(1);
        let bytes_per_changed_milli = after_set_growth.saturating_mul(1_000) / changed;
        eprintln!(
            "point-workspace revision evidence: label={} depth={} selected={} changed={} set_micros={} revert_micros={} logical_set_durable_growth={} logical_bytes_per_changed_point={}.{:03}",
            label,
            depth,
            pair.selected,
            pair.changed,
            pair.set_elapsed.as_micros(),
            pair.revert_elapsed.as_micros(),
            after_set_growth,
            bytes_per_changed_milli / 1_000,
            bytes_per_changed_milli % 1_000
        );

        if matches!(depth, 2 | 4 | 8) {
            drop(workspace);
            let started = Instant::now();
            workspace = open(workspace_path, fixture.index.clone(), OpenLimits::default())
                .blocking_wait()
                .expect("resource-report Workspace reopens at increasing depth");
            eprintln!(
                "point-workspace reopen evidence: depth={} elapsed_micros={} durable_bytes={}",
                depth,
                started.elapsed().as_micros(),
                logical_directory_entry_bytes(workspace_path)
            );
        }
    }
    eprintln!(
        "point-workspace logical durable-entry evidence (hard-linked payloads counted per directory entry): initial_bytes={} final_bytes={} total_growth={} physical_du_bytes={}",
        initial_durable,
        logical_directory_entry_bytes(workspace_path),
        logical_directory_entry_bytes(workspace_path).saturating_sub(initial_durable),
        physical_disk_bytes(workspace_path).expect("measure physical Workspace disk use with du")
    );
}

#[derive(Clone, Copy)]
enum EditSelection {
    Classification(u8),
    FirstHalf,
}

struct EditPairReport {
    selected: u64,
    changed: u64,
    set_elapsed: Duration,
    revert_elapsed: Duration,
    durable_after_set: u64,
}

fn run_edit_pair(
    workspace: &Workspace,
    workspace_path: &Path,
    point_count: usize,
    selection: EditSelection,
    value: u8,
    operation_byte: u8,
) -> EditPairReport {
    let point_count = u64::try_from(point_count).expect("benchmark count fits u64");
    let query = match selection {
        EditSelection::Classification(classification) => {
            PointQuery::all().classification_is(classification)
        }
        EditSelection::FirstHalf => PointQuery::within(
            WorldBounds::new(
                [0.0, 0.0, 0.0],
                [
                    exact_f64(point_count.saturating_div(2).saturating_sub(1)),
                    4_095.0,
                    2_047.0,
                ],
            )
            .expect("benchmark half bounds are valid"),
        ),
    };
    let selected = workspace
        .head()
        .select(query, resident_limits())
        .blocking_wait()
        .expect("benchmark Edit target materializes");
    let selected_count = selected.metadata().exact_count();
    let set_operation = operation_id(operation_byte);
    let set_started = Instant::now();
    let set_receipt = committed(
        workspace
            .commit(
                CommitRequest::set_classification(set_operation, selected, value),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("benchmark classification commit has a certain outcome"),
    );
    let set_elapsed = set_started.elapsed();
    let changed = match set_receipt.revision_info().kind() {
        RevisionKind::SetClassification { changed_points, .. } => changed_points,
        other => panic!("benchmark classification produced {other:?}"),
    };
    let durable_after_set = logical_directory_entry_bytes(workspace_path);
    let revert_started = Instant::now();
    committed(
        workspace
            .commit(
                CommitRequest::revert_head(
                    operation_id(operation_byte.saturating_add(1)),
                    set_receipt.revision(),
                ),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("benchmark Revert has a certain outcome"),
    );
    EditPairReport {
        selected: selected_count,
        changed,
        set_elapsed,
        revert_elapsed: revert_started.elapsed(),
        durable_after_set,
    }
}

fn committed(outcome: CommitOutcome) -> point_workspace::CommitReceipt {
    match outcome {
        CommitOutcome::Committed(receipt) => receipt,
        other => panic!("benchmark commit did not commit: {other:?}"),
    }
}

fn operation_id(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).expect("benchmark Operation Identity is nonzero")
}

fn resident_limits() -> PointSetLimits {
    selection_limits(512 * MIB)
}

fn forced_spill_limits() -> PointSetLimits {
    selection_limits(0)
}

fn selection_limits(resident_bytes: u64) -> PointSetLimits {
    PointSetLimits::new(
        CandidateLimits::default(),
        ReadBudget::default().with_max_points(100_000_000),
        100_000_000,
        100_000_000,
        10_000_000,
        8 * 1024 * MIB,
        768 * MIB,
        resident_bytes,
        8 * 1024 * MIB,
    )
}

fn configured_point_count() -> usize {
    let count = std::env::var(POINT_COUNT_ENV)
        .ok()
        .map_or(DEFAULT_POINT_COUNT, |value| {
            value.parse().unwrap_or_else(|_| {
                panic!("{POINT_COUNT_ENV} must be a positive integer, got {value:?}")
            })
        });
    assert!(count >= 100, "{POINT_COUNT_ENV} must be at least 100");
    count
}

struct Fixture {
    workspace: Workspace,
    index: PreparedIndex,
    point_count: usize,
    directory: BenchDirectory,
}

impl Fixture {
    fn new(point_count: usize) -> Self {
        let directory = BenchDirectory::new();
        let source = generated_source(point_count);
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("benchmark Spatial Index prepares");
        let workspace = create(
            directory.path().join("selection.pcw"),
            index.clone(),
            WorkspaceSchema::new(classification_attribute()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("benchmark selection Workspace creates");
        eprintln!(
            "point-workspace fixture: points={} index_bytes={} env_override={}",
            point_count,
            index.prepare_report().artifact_bytes(),
            POINT_COUNT_ENV
        );
        Self {
            workspace,
            index,
            point_count,
            directory,
        }
    }
}

fn generated_source(point_count: usize) -> point_source::Source {
    let ticks = (0..point_count)
        .map(|ordinal| {
            let ordinal = i64::try_from(ordinal).expect("benchmark ordinal fits i64");
            [
                ordinal,
                (ordinal * 17).rem_euclid(4_096),
                (ordinal * 31).rem_euclid(2_048),
            ]
        })
        .collect::<Vec<_>>();
    let classifications = (0..point_count)
        .map(|ordinal| u8::try_from(ordinal % 100).expect("classification fits u8"))
        .collect::<Vec<_>>();
    let definition = AttributeDefinition::new(
        classification_attribute(),
        "classification",
        AttributeDataType::U8,
    )
    .expect("benchmark classification definition is valid");
    let column = AttributeColumn::new(definition, AttributeValues::u8(classifications))
        .expect("benchmark classification column is valid");
    let attributes = AttributeColumns::new(vec![column], point_count)
        .expect("benchmark Attribute columns align");
    let input = MemorySource::from_columns(
        PositionTransform::new([0.0; 3], [1.0; 3]).unwrap(),
        CoordinateReference::Unknown,
        ticks,
        attributes,
    )
    .expect("benchmark memory Source input is valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("benchmark memory Source opens")
}

fn classification_attribute() -> AttributeId {
    AttributeId::new(CLASSIFICATION_ID).expect("benchmark Attribute Identity is nonzero")
}

#[allow(
    clippy::cast_precision_loss,
    reason = "the explicit 2^53 guard makes this ordinal conversion exact"
)]
fn exact_f64(value: u64) -> f64 {
    assert!(
        value <= (1_u64 << 53),
        "benchmark world ordinal exceeds exact f64 range"
    );
    value as f64
}

fn physical_disk_bytes(path: &Path) -> Option<u64> {
    let output = Command::new("du").args(["-sk"]).arg(path).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let kibibytes = stdout.split_whitespace().next()?.parse::<u64>().ok()?;
    kibibytes.checked_mul(1_024)
}

fn process_rss_bytes() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p"])
        .arg(std::process::id().to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kibibytes = String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1_024)
}

fn logical_directory_entry_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    fs::read_dir(path)
        .expect("read generated benchmark directory")
        .map(|entry| logical_directory_entry_bytes(&entry.expect("read benchmark entry").path()))
        .sum()
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct BenchDirectory(PathBuf);

impl BenchDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "punctra-point-workspace-benchmark-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create generated benchmark directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for BenchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

criterion_group!(benches, benchmark_document);
criterion_main!(benches);
