//! Source-scale cold, resumed, warm, candidate, and node-read benchmarks.
//!
//! The default fixture contains 1,000,000 Points. Set
//! `PUNCTRA_POINT_INDEX_BENCH_POINTS` to run 10^5, 10^6, 10^7, or 10^8 scale.

#![allow(missing_docs)]

use std::{
    hint::black_box,
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{
    AttributeColumns, AttributeSchema, CoordinateReference, PositionTransform, SourceMetadata,
    WorldBounds,
};
use point_index::{
    CandidateLimits, IndexRecipe, NodeReadBudget, PrepareDisposition, PrepareLimits, PreparedIndex,
    prepare, prepare_fresh_with_recipe,
};
use point_source::Source;
use source_memory::{MemoryFaultControl, MemorySource};

const POINT_COUNT_ENV: &str = "PUNCTRA_POINT_INDEX_BENCH_POINTS";
const DEFAULT_POINT_COUNT: usize = 1_000_000;
const BLOCK_POINTS: u64 = 65_536;
const MAX_QUERY_READ_PEAK_HEAP_BYTES: u64 = 32 * 1024 * 1024;

fn benchmark_index(criterion: &mut Criterion) {
    let fixture = Fixture::new(configured_point_count());
    let limits = PrepareLimits::default();
    eprintln!(
        "point-index fixture: points={} source_batch_points={} source_batch_bytes={} adapter_bytes={} build_bytes={} incomplete_bytes={} artifact_bytes={} hierarchy_nodes={} metadata_bytes={}",
        fixture.point_count,
        limits.max_source_batch_points(),
        limits.max_source_batch_payload_bytes(),
        limits.max_adapter_working_bytes(),
        limits.max_build_working_bytes(),
        limits.max_incomplete_bytes(),
        limits.max_artifact_bytes(),
        limits.max_hierarchy_nodes(),
        limits.max_resident_metadata_bytes()
    );
    let cold_target = fixture.directory.path().join("cold.pidx");
    let warm_target = fixture.directory.path().join("warm.pidx");
    let resume_target = fixture.directory.path().join("resume.pidx");

    report_resume(&fixture, &resume_target);
    let built = cold_prepare(&fixture.source, &warm_target);
    eprintln!(
        "point-index initial prepare: disposition={:?} reused={} read={} artifact={} bytes",
        built.prepare_report().disposition(),
        built.prepare_report().durable_points_reused(),
        built.prepare_report().source_points_read(),
        built.prepare_report().artifact_bytes()
    );
    drop(built);
    let prepared = prepare(
        fixture.source.clone(),
        &warm_target,
        PrepareLimits::default(),
    )
    .blocking_wait()
    .expect("initial warm benchmark open succeeds");
    assert_eq!(
        prepared.prepare_report().disposition(),
        PrepareDisposition::Opened
    );
    eprintln!(
        "point-index initial warm open: disposition={:?} reused={} read={} artifact={} bytes",
        prepared.prepare_report().disposition(),
        prepared.prepare_report().durable_points_reused(),
        prepared.prepare_report().source_points_read(),
        prepared.prepare_report().artifact_bytes()
    );
    assert_query_read_peak_heap(&prepared);
    benchmark_prepare(criterion, &fixture, &cold_target, &warm_target);
    benchmark_queries(criterion, &fixture, &prepared);
}

fn assert_query_read_peak_heap(index: &PreparedIndex) {
    let root = index
        .hierarchy()
        .root()
        .expect("benchmark root exists")
        .id();
    let leaf = index
        .hierarchy()
        .nodes()
        .iter()
        .find(|node| node.coverage_complete())
        .expect("benchmark leaf exists")
        .id();
    let bounds = index
        .descriptor()
        .world_bounds()
        .expect("benchmark bounds exist");
    let allocations = allocation_counter::measure(|| {
        let candidates = index
            .candidates(bounds, CandidateLimits::default())
            .expect("measured candidates succeed");
        black_box(candidates.spans());
        black_box(drain_node(index, root));
        black_box(drain_node(index, leaf));
    });
    assert!(
        allocations.bytes_max <= MAX_QUERY_READ_PEAK_HEAP_BYTES,
        "query/read peak heap {} exceeded {} bytes",
        allocations.bytes_max,
        MAX_QUERY_READ_PEAK_HEAP_BYTES
    );
    assert_eq!(
        allocations.bytes_current, 0,
        "query/read retained measured heap allocations"
    );
    eprintln!(
        "point-index measured synchronous candidate/root/leaf peak heap: {} bytes (ceiling: {})",
        allocations.bytes_max, MAX_QUERY_READ_PEAK_HEAP_BYTES
    );
}

fn report_resume(fixture: &Fixture, target: &Path) {
    cleanup_target(target);
    fixture.fault.fail_at_ordinal(BLOCK_POINTS);
    let failed = prepare(fixture.source.clone(), target, PrepareLimits::default()).blocking_wait();
    assert!(failed.is_err(), "faulted build must leave resumable work");
    fixture.fault.clear_read_fault();
    let resumed = prepare(fixture.source.clone(), target, PrepareLimits::default())
        .blocking_wait()
        .expect("fault-cleared build resumes");
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert_eq!(
        resumed.prepare_report().durable_points_reused(),
        BLOCK_POINTS
    );
    eprintln!(
        "point-index resume: disposition={:?} reused={} read={} artifact={} bytes",
        resumed.prepare_report().disposition(),
        resumed.prepare_report().durable_points_reused(),
        resumed.prepare_report().source_points_read(),
        resumed.prepare_report().artifact_bytes()
    );
}

fn benchmark_prepare(
    criterion: &mut Criterion,
    fixture: &Fixture,
    cold_target: &Path,
    warm_target: &Path,
) {
    let mut group = criterion.benchmark_group("point-index/prepare");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5))
        .throughput(Throughput::Elements(fixture.point_count));
    group.bench_function("cold-build", |bencher| {
        bencher.iter(|| {
            cleanup_target(cold_target);
            let prepared = cold_prepare(&fixture.source, cold_target);
            assert_eq!(
                prepared.prepare_report().disposition(),
                PrepareDisposition::Built
            );
            black_box(prepared.descriptor().artifact_checksum())
        });
    });
    group.bench_function("warm-open", |bencher| {
        bencher.iter(|| {
            let prepared = prepare(
                fixture.source.clone(),
                warm_target,
                PrepareLimits::default(),
            )
            .blocking_wait()
            .expect("warm benchmark open succeeds");
            assert_eq!(
                prepared.prepare_report().disposition(),
                PrepareDisposition::Opened
            );
            black_box(prepared.descriptor().artifact_checksum())
        });
    });
    group.finish();
}

fn benchmark_queries(criterion: &mut Criterion, fixture: &Fixture, index: &PreparedIndex) {
    let bounds = index
        .descriptor()
        .world_bounds()
        .expect("nonempty benchmark Source has bounds");
    let root = index
        .hierarchy()
        .root()
        .expect("nonempty benchmark Source has a root")
        .id();
    let leaf = index
        .hierarchy()
        .nodes()
        .iter()
        .find(|node| node.coverage_complete())
        .expect("fixed-block hierarchy has a leaf")
        .id();
    let mut group = criterion.benchmark_group("point-index/query-and-read");
    group.throughput(Throughput::Elements(fixture.point_count));
    group.bench_function("whole-bounds-candidates", |bencher| {
        bencher.iter(|| {
            let plan = index
                .candidates(black_box(bounds), CandidateLimits::default())
                .expect("candidate benchmark succeeds");
            assert_eq!(plan.candidate_point_count(), fixture.point_count);
            black_box(plan.spans().len())
        });
    });
    group.bench_function("internal-root-display-read", |bencher| {
        bencher.iter(|| black_box(drain_node(index, root)));
    });
    group.bench_function("complete-leaf-read", |bencher| {
        bencher.iter(|| black_box(drain_node(index, leaf)));
    });
    group.finish();
}

fn drain_node(index: &PreparedIndex, node: point_index::IndexNodeId) -> u64 {
    let budget =
        NodeReadBudget::new(65_536, 8 * 1024 * 1024).expect("benchmark limits are nonzero");
    let mut batches = index
        .read_node(node, budget)
        .expect("node benchmark starts");
    let mut emitted = 0_u64;
    while let Some(batch) = batches.next().expect("node benchmark reads") {
        assert!(batch.estimated_payload_bytes() <= budget.max_display_batch_bytes());
        emitted = emitted
            .checked_add(u64::try_from(batch.len()).expect("batch length fits u64"))
            .expect("emitted count fits u64");
        black_box(batch.samples().last());
    }
    let summary = batches.summary().expect("successful read has a summary");
    assert_eq!(summary.emitted_point_count(), emitted);
    assert_eq!(summary.source(), index.descriptor().source());
    emitted
}

fn cold_prepare(source: &Source, target: &Path) -> PreparedIndex {
    prepare_fresh_with_recipe(
        source.clone(),
        target,
        IndexRecipe::PositionOnlyV1,
        PrepareLimits::default(),
    )
    .blocking_wait()
    .expect("cold benchmark build succeeds")
}

fn configured_point_count() -> usize {
    std::env::var(POINT_COUNT_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|count| *count > usize::try_from(BLOCK_POINTS).unwrap_or(65_536))
        .unwrap_or(DEFAULT_POINT_COUNT)
}

struct Fixture {
    source: Source,
    fault: MemoryFaultControl,
    point_count: u64,
    directory: FixtureDirectory,
}

impl Fixture {
    fn new(point_count: usize) -> Self {
        let transform = PositionTransform::new([500_000.0, 4_600_000.0, 100.0], [0.001; 3])
            .expect("fixture transform is valid");
        let ticks = (0..point_count)
            .map(|ordinal| {
                let ordinal = i64::try_from(ordinal).expect("fixture ordinal fits i64");
                [ordinal, (ordinal * 97) % 131_071, ordinal / 1_024]
            })
            .collect::<Vec<_>>();
        let bounds = bounds(transform, &ticks);
        let metadata = SourceMetadata::new(
            u64::try_from(point_count).expect("fixture count fits u64"),
            transform,
            CoordinateReference::Unknown,
            AttributeSchema::empty(),
            Some(bounds),
            "memory",
            Vec::new(),
        )
        .expect("fixture metadata is valid");
        let attributes = AttributeColumns::empty(point_count);
        let (input, fault) = MemorySource::with_fault_control(metadata, ticks, attributes)
            .expect("fixture columns are valid");
        let source = source_memory::open(input)
            .blocking_wait()
            .expect("fixture Full verification succeeds");
        Self {
            source,
            fault,
            point_count: u64::try_from(point_count).expect("fixture count fits u64"),
            directory: FixtureDirectory::new().expect("benchmark directory is created"),
        }
    }
}

fn bounds(transform: PositionTransform, ticks: &[[i64; 3]]) -> WorldBounds {
    let first = transform.world_f64(ticks[0]);
    let mut minimum = first;
    let mut maximum = first;
    for ticks in &ticks[1..] {
        let world = transform.world_f64(*ticks);
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(world[axis]);
            maximum[axis] = maximum[axis].max(world[axis]);
        }
    }
    WorldBounds::new(minimum, maximum).expect("fixture bounds are valid")
}

fn cleanup_target(target: &Path) {
    for path in [
        target.to_path_buf(),
        sidecar(target, ".work"),
        sidecar(target, ".samples"),
        sidecar(target, ".tmp"),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove benchmark artifact {}: {error}", path.display()),
        }
    }
}

fn sidecar(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target
        .file_name()
        .expect("benchmark target has a file name")
        .to_os_string();
    name.push(suffix);
    target.with_file_name(name)
}

struct FixtureDirectory {
    path: PathBuf,
}

impl FixtureDirectory {
    fn new() -> io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..100_u32 {
            let path = std::env::temp_dir().join(format!(
                "punctra-point-index-bench-{}-{timestamp}-{attempt}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique point-index benchmark directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

criterion_group!(benches, benchmark_index);
criterion_main!(benches);
