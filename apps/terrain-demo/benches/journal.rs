//! Generated end-to-end benchmark for the recoverable v0.7 workflow.
//!
//! The default fixture contains 10,000 Points. Set
//! `PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS=100000` or `1000000` for the
//! documented larger modes. The measurements use only generated local data
//! and are not partner, production, or worker-heap evidence.

#![allow(missing_docs)]

#[path = "../tests/support/mod.rs"]
mod support;

use std::{
    fs,
    hint::black_box,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{AttributeId, PointId};
use point_index::PrepareLimits;
use point_terrain::{CheckPoint, CheckPointId, LandXmlOptions, TerrainRecipe};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, OperationResolution,
    PointSetLimits, Workspace, WorkspaceSchema, create, open,
};
use support::{
    RevisionDirectoryBlocker, TestDirectory, journal_frame_ends, restore_journal_prefix,
    write_las_family_fixture,
};
use terrain_demo::{
    WorkflowLimits, WorkflowPaths, WorkflowReceipt, WorkflowRunId, WorkflowRunIntent, resume_run,
    start_run,
};

const POINT_COUNT_ENV: &str = "PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS";
const DEFAULT_POINT_COUNT: usize = 10_000;
const CLASSIFICATION_ATTRIBUTE: u32 = 6;

fn benchmark_workflow(criterion: &mut Criterion) {
    let point_count = configured_point_count();
    report_resource_evidence(point_count);
    let mut group = criterion.benchmark_group("terrain_workflow_generated");
    group.throughput(Throughput::Elements(
        u64::try_from(point_count).expect("benchmark Point count fits u64"),
    ));
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("cold_start", |bencher| {
        bencher.iter_batched(
            || BenchFixture::new(point_count),
            |fixture| black_box(fixture.start()),
            BatchSize::PerIteration,
        );
    });
    group.bench_function("resume_after_committed_edit", |bencher| {
        bencher.iter_batched(
            || BenchFixture::completed_at_prefix(point_count, 1),
            |fixture| black_box(fixture.resume()),
            BatchSize::PerIteration,
        );
    });
    group.bench_function("resume_from_retryable_workspace_intent", |bencher| {
        bencher.iter_batched(
            || BenchFixture::retryable(point_count),
            |fixture| black_box(fixture.resume()),
            BatchSize::PerIteration,
        );
    });
    group.bench_function("landxml_and_report_reconciliation", |bencher| {
        bencher.iter_batched(
            || BenchFixture::completed_at_prefix(point_count, 5),
            |fixture| black_box(fixture.resume()),
            BatchSize::PerIteration,
        );
    });
    group.bench_function("complete_revalidation", |bencher| {
        bencher.iter_batched(
            || {
                let fixture = BenchFixture::new(point_count);
                fixture.start();
                fixture
            },
            |fixture| black_box(fixture.resume()),
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

fn report_resource_evidence(point_count: usize) {
    let fixture = BenchFixture::new(point_count);
    let receipt = fixture.start();
    let journal_bytes = fs::metadata(fixture.run_root.join("run.pwf"))
        .expect("measure benchmark journal")
        .len();
    let report =
        fs::read(fixture.run_root.join("audit.json")).expect("read benchmark resource report");
    let report_json: serde_json::Value =
        serde_json::from_slice(&report).expect("parse benchmark resource report");
    eprintln!(
        "terrain-workflow generated evidence: points={point_count} journal_bytes={journal_bytes} report_bytes={} frames={} semantic_limits={}",
        receipt.report_bytes(),
        receipt.frame_count(),
        report_json["limits"],
    );
    eprintln!(
        "terrain-workflow worker peak heap: unclaimed; accounted algorithm ceilings are reported above"
    );
    eprintln!(
        "terrain-workflow external evidence: generated local data only; partner, production, downstream round-trip, and human-time acceptance are unclaimed"
    );
    eprintln!(
        "terrain-workflow retryable-intent mode: fixture obstructs only the test-owned revisions directory after Workspace open, restores it, reopens, and requires OperationResolution::Retryable before timing"
    );
}

fn configured_point_count() -> usize {
    match std::env::var(POINT_COUNT_ENV) {
        Err(std::env::VarError::NotPresent) => DEFAULT_POINT_COUNT,
        Ok(value) => match value.parse::<usize>() {
            Ok(point_count @ (10_000 | 100_000 | 1_000_000)) => point_count,
            _ => panic!(
                "{POINT_COUNT_ENV} must be one documented generated mode: 10000, 100000, or 1000000"
            ),
        },
        Err(error) => panic!("read {POINT_COUNT_ENV}: {error}"),
    }
}

struct BenchFixture {
    _directory: TestDirectory,
    source: PathBuf,
    index: PathBuf,
    workspace: PathBuf,
    paths: WorkflowPaths,
    intent: WorkflowRunIntent,
    run_root: PathBuf,
}

impl BenchFixture {
    fn new(point_count: usize) -> Self {
        let sequence = next_sequence();
        let directory = TestDirectory::new("benchmark").expect("create benchmark directory");
        let source = directory.path().join("fixture.las");
        let index = directory.path().join("fixture.pidx");
        let workspace = directory.path().join("fixture.pcw");
        let run_root = directory.path().join("run");
        fs::create_dir(&run_root).expect("create benchmark Run root");
        write_las_family_fixture(&source, point_count).expect("write generated benchmark Source");

        let source_handle = source_las::open(&source)
            .blocking_wait()
            .expect("open benchmark Source");
        let prepared = point_index::prepare(source_handle, &index, PrepareLimits::default())
            .blocking_wait()
            .expect("prepare benchmark index");
        let workspace_handle = create(
            &workspace,
            prepared,
            WorkspaceSchema::new(
                AttributeId::new(CLASSIFICATION_ATTRIBUTE)
                    .expect("classification Attribute ID is nonzero"),
            ),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create benchmark Workspace");
        let baseline = workspace_handle.head().provenance().revision();
        drop(workspace_handle);

        let paths = WorkflowPaths::new(&source, &index, &workspace, &run_root);
        let intent = WorkflowRunIntent::new(
            WorkflowRunId::new(identity(sequence, 1)).expect("nonzero benchmark Run ID"),
            OperationId::from_bytes(identity(sequence, 2)).expect("nonzero benchmark Operation ID"),
            baseline,
            [9_u64, 10],
            1,
            TerrainRecipe::new(2),
            [
                CheckPoint::new(
                    CheckPointId::new(1).expect("nonzero Check Point ID"),
                    [500_002.0, 4_600_002.0, 121.6],
                )
                .expect("finite sampled Check Point"),
                CheckPoint::new(
                    CheckPointId::new(2).expect("nonzero Check Point ID"),
                    [600_000.0, 4_600_000.0, 120.0],
                )
                .expect("finite gap Check Point"),
            ],
            LandXmlOptions::metric_metres("Punctra Generated Benchmark", "2026-08-10", "00:00:00Z")
                .expect("valid benchmark LandXML options")
                .allow_unknown_coordinate_reference_as_metric_metres(),
        )
        .expect("construct benchmark workflow Intent");
        Self {
            _directory: directory,
            source,
            index,
            workspace,
            paths,
            intent,
            run_root,
        }
    }

    fn completed_at_prefix(point_count: usize, frames: usize) -> Self {
        let fixture = Self::new(point_count);
        fixture.start();
        let journal = fixture.run_root.join("run.pwf");
        let complete = fs::read(&journal).expect("read complete benchmark journal");
        let ends = journal_frame_ends(&complete).expect("parse complete benchmark journal");
        let end = *ends
            .get(frames - 1)
            .expect("benchmark prefix names one durable frame");
        restore_journal_prefix(&journal, &complete, end)
            .expect("durably restore benchmark journal prefix");
        fixture
    }

    fn retryable(point_count: usize) -> Self {
        let fixture = Self::new(point_count);
        start_run(
            fixture.paths.clone(),
            fixture.intent.clone(),
            WorkflowLimits::default().with_selection_limits(one_input_point_selection_limits()),
        )
        .blocking_wait()
        .expect_err("selection ceiling stops benchmark after durable Intent");

        let workspace = fixture.open_workspace();
        let points = workspace
            .head()
            .select_point_ids(
                [
                    PointId::new(workspace.source(), 9),
                    PointId::new(workspace.source(), 10),
                ],
                PointSetLimits::default(),
            )
            .blocking_wait()
            .expect("materialize benchmark retryable Points");
        let operation = fixture.intent.operation();
        let obstruction = RevisionDirectoryBlocker::install(&fixture.workspace)
            .expect("obstruct benchmark Revision publication");
        let outcome = workspace
            .commit(
                CommitRequest::set_classification(operation, points, 1),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("ready publication failure has a certainty-preserving outcome");
        assert!(matches!(outcome, CommitOutcome::Indeterminate(_)));
        obstruction
            .restore()
            .expect("restore benchmark revisions directory");
        drop(workspace);
        let reopened = fixture.open_workspace();
        assert!(matches!(
            reopened
                .resolve_operation(operation)
                .expect("resolve benchmark ready intent"),
            OperationResolution::Retryable(_),
        ));
        drop(reopened);
        fixture
    }

    fn start(&self) -> WorkflowReceipt {
        start_run(
            self.paths.clone(),
            self.intent.clone(),
            WorkflowLimits::default(),
        )
        .blocking_wait()
        .expect("complete benchmark workflow")
    }

    fn resume(&self) -> WorkflowReceipt {
        resume_run(
            self.paths.clone(),
            self.intent.clone(),
            WorkflowLimits::default(),
        )
        .blocking_wait()
        .expect("resume benchmark workflow")
    }

    fn open_workspace(&self) -> Workspace {
        let source = source_las::open(&self.source)
            .blocking_wait()
            .expect("reopen benchmark Source");
        let index = point_index::prepare(source, &self.index, PrepareLimits::default())
            .blocking_wait()
            .expect("reopen benchmark index");
        open(&self.workspace, index, OpenLimits::default())
            .blocking_wait()
            .expect("reopen benchmark Workspace")
    }
}

fn one_input_point_selection_limits() -> PointSetLimits {
    let defaults = PointSetLimits::default();
    PointSetLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        1,
        defaults.max_output_points(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        defaults.max_working_bytes(),
        defaults.max_resident_bytes(),
        defaults.max_temporary_bytes(),
    )
}

fn next_sequence() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn identity(sequence: u64, domain: u64) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    bytes[8..].copy_from_slice(&domain.to_le_bytes());
    bytes
}

criterion_group!(benches, benchmark_workflow);
criterion_main!(benches);
