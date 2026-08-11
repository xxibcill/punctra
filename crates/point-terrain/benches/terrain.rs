//! Public-composition benchmark for derivation, detached QA, and `LandXML`.
//!
//! The manageable default is 10,000 Points. Set
//! `PUNCTRA_TERRAIN_BENCH_POINTS` to a positive count up to 1,000,000; the
//! intended evidence scales are 10,000, 100,000, and 1,000,000 Points.

#![allow(missing_docs)]

use std::{
    fs,
    hint::black_box,
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_index::{PrepareLimits, prepare};
use point_terrain::{
    CheckPoint, CheckPointId, CheckPointLimits, CheckPointReport, LandXmlLimits, LandXmlOptions,
    LandXmlReceipt, TerrainDescriptor, TerrainLimits, TerrainRecipe, TerrainSurface,
};
use point_workspace::{OpenLimits, Snapshot, Workspace, WorkspaceSchema, create};
use source_memory::MemorySource;

const POINT_COUNT_ENV: &str = "PUNCTRA_TERRAIN_BENCH_POINTS";
const MACHINE_NAME_ENV: &str = "PUNCTRA_TERRAIN_BENCH_MACHINE";
const DEFAULT_POINT_COUNT: usize = 10_000;
const MAXIMUM_POINT_COUNT: usize = 1_000_000;
const CLASSIFICATION_ATTRIBUTE_ID: u32 = 301;
const GROUND_CLASSIFICATION: u8 = 2;
const QA_POINT_COUNT: u64 = 3;

fn benchmark_terrain(criterion: &mut Criterion) {
    let fixture = Fixture::new(configured_point_count());
    let terrain_limits = TerrainLimits::default();
    let qa_limits = CheckPointLimits::default();
    let landxml_limits = LandXmlLimits::default();
    let derive_started = Instant::now();
    let baseline = derive_surface(fixture.snapshot(), terrain_limits);
    let derive_elapsed_us = derive_started.elapsed().as_micros();
    let expected_descriptor = baseline.descriptor().clone();
    assert_surface(
        &baseline,
        &expected_descriptor,
        terrain_limits,
        fixture.point_count,
    );

    let check_points = benchmark_check_points(&baseline);
    let qa_started = Instant::now();
    let expected_qa = evaluate_check_points(&baseline, &check_points, qa_limits);
    let qa_elapsed_us = qa_started.elapsed().as_micros();
    assert_qa(&expected_qa, &expected_qa, qa_limits);

    let landxml_options = asserted_landxml_options();
    let baseline_target = fixture.next_export_target();
    let landxml_started = Instant::now();
    let expected_receipt = export_surface(
        &baseline,
        &baseline_target,
        &landxml_options,
        landxml_limits,
    );
    let landxml_elapsed_us = landxml_started.elapsed().as_micros();
    assert_export(
        &expected_receipt,
        &expected_receipt,
        &baseline_target,
        landxml_limits,
    );
    fs::remove_file(&baseline_target).expect("remove baseline LandXML output");

    report_resource_facts(
        &expected_descriptor,
        &expected_qa,
        expected_receipt,
        terrain_limits,
        qa_limits,
        landxml_limits,
        EvidenceTimings {
            derive: derive_elapsed_us,
            qa: qa_elapsed_us,
            landxml: landxml_elapsed_us,
        },
    );
    benchmark_derivation(criterion, &fixture, &expected_descriptor, terrain_limits);
    benchmark_qa(criterion, &baseline, &check_points, &expected_qa, qa_limits);
    benchmark_landxml(
        criterion,
        &fixture,
        &baseline,
        &landxml_options,
        expected_receipt,
        landxml_limits,
    );
}

fn benchmark_derivation(
    criterion: &mut Criterion,
    fixture: &Fixture,
    expected: &TerrainDescriptor,
    limits: TerrainLimits,
) {
    let mut group = criterion.benchmark_group("point_terrain/derive");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3))
        .throughput(Throughput::Elements(fixture.point_count));
    group.bench_function("complete_snapshot", |bencher| {
        bencher.iter(|| {
            let surface = derive_surface(fixture.snapshot(), limits);
            assert_surface(&surface, expected, limits, fixture.point_count);
            black_box(surface)
        });
    });
    group.finish();
}

fn benchmark_qa(
    criterion: &mut Criterion,
    surface: &TerrainSurface,
    check_points: &[CheckPoint],
    expected: &CheckPointReport,
    limits: CheckPointLimits,
) {
    let mut group = criterion.benchmark_group("point_terrain/detached_qa");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3))
        .throughput(Throughput::Elements(QA_POINT_COUNT));
    group.bench_function("two_samples_one_gap", |bencher| {
        bencher.iter(|| {
            let report = evaluate_check_points(surface, check_points, limits);
            assert_qa(&report, expected, limits);
            black_box(report)
        });
    });
    group.finish();
}

fn benchmark_landxml(
    criterion: &mut Criterion,
    fixture: &Fixture,
    surface: &TerrainSurface,
    options: &LandXmlOptions,
    expected: LandXmlReceipt,
    limits: LandXmlLimits,
) {
    let mut group = criterion.benchmark_group("point_terrain/landxml");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3))
        .throughput(Throughput::Bytes(expected.byte_length()));
    group.bench_function("atomic_metric_export", |bencher| {
        bencher.iter_custom(|iterations| {
            let mut measured = Duration::ZERO;
            for _ in 0..iterations {
                let target = fixture.next_export_target();
                let started = Instant::now();
                let receipt = export_surface(surface, &target, options, limits);
                measured = measured.saturating_add(started.elapsed());
                assert_export(&receipt, &expected, &target, limits);
                black_box(receipt);
                fs::remove_file(&target).expect("remove measured LandXML output");
            }
            measured
        });
    });
    group.finish();
}

fn derive_surface(snapshot: Snapshot, limits: TerrainLimits) -> TerrainSurface {
    point_terrain::derive(snapshot, TerrainRecipe::new(GROUND_CLASSIFICATION), limits)
        .blocking_wait()
        .expect("benchmark Terrain Derivation succeeds")
}

fn evaluate_check_points(
    surface: &TerrainSurface,
    check_points: &[CheckPoint],
    limits: CheckPointLimits,
) -> CheckPointReport {
    surface
        .check_points(check_points.iter().copied(), limits)
        .blocking_wait()
        .expect("benchmark detached QA succeeds")
}

fn export_surface(
    surface: &TerrainSurface,
    target: &Path,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
) -> LandXmlReceipt {
    surface
        .export_landxml(target, options.clone(), limits)
        .blocking_wait()
        .expect("benchmark LandXML export succeeds")
}

fn assert_surface(
    surface: &TerrainSurface,
    expected: &TerrainDescriptor,
    limits: TerrainLimits,
    point_count: u64,
) {
    let descriptor = surface.descriptor();
    assert_eq!(descriptor, expected);
    assert_eq!(descriptor.input_point_count(), point_count);
    assert_eq!(descriptor.vertex_count(), point_count);
    assert!(descriptor.face_count() > 0);
    assert!(descriptor.face_count() <= limits.max_faces());
    assert!(descriptor.input_point_count() <= limits.max_input_points());
    assert!(descriptor.accounted_peak_working_bytes() <= limits.max_working_bytes());
    assert!(descriptor.retained_surface_bytes() <= limits.max_surface_bytes());
    assert!(descriptor.topology_steps() <= limits.max_work_units());
    assert_eq!(surface.vertices().len() as u64, descriptor.vertex_count());
    assert_eq!(surface.faces().len() as u64, descriptor.face_count());
}

fn assert_qa(report: &CheckPointReport, expected: &CheckPointReport, limits: CheckPointLimits) {
    assert_eq!(report, expected);
    assert_eq!(report.results().len() as u64, QA_POINT_COUNT);
    assert_eq!(report.statistics().covered_count(), 2);
    assert_eq!(report.statistics().gap_count(), 1);
    assert!(report.face_tests() <= limits.max_face_tests());
    assert!(report.accounted_peak_working_bytes() <= limits.max_working_bytes());
    assert!(report.results().len() as u64 <= limits.max_check_points());
    assert!(
        (report.results().len() as u64)
            .saturating_mul(std::mem::size_of::<point_terrain::CheckPointResult>() as u64)
            <= limits.max_result_bytes()
    );
}

fn assert_export(
    receipt: &LandXmlReceipt,
    expected: &LandXmlReceipt,
    target: &Path,
    limits: LandXmlLimits,
) {
    assert_eq!(receipt, expected);
    assert_eq!(
        fs::metadata(target)
            .expect("published LandXML metadata is readable")
            .len(),
        receipt.byte_length()
    );
    assert!(receipt.vertex_count() <= limits.max_vertices());
    assert!(receipt.face_count() <= limits.max_faces());
    assert!(receipt.byte_length() <= limits.max_output_bytes());
    assert!(receipt.byte_length() <= limits.max_staging_bytes());
}

fn benchmark_check_points(surface: &TerrainSurface) -> Vec<CheckPoint> {
    let transform = surface.descriptor().position_transform();
    let face = surface
        .faces()
        .first()
        .expect("benchmark Surface has a face");
    let world = face.vertices().map(|id| {
        let index = usize::try_from(id.get() - 1).expect("Surface identity fits usize");
        transform.world_f64(surface.vertices()[index].ticks())
    });
    let centroid = [
        (world[0][0] + world[1][0] + world[2][0]) / 3.0,
        (world[0][1] + world[1][1] + world[2][1]) / 3.0,
        (world[0][2] + world[1][2] + world[2][2]) / 3.0,
    ];
    let bounds = surface.descriptor().bounds();
    vec![
        check_point(1, world[0]),
        check_point(2, [centroid[0], centroid[1], centroid[2] + 1.0]),
        check_point(
            3,
            [bounds.max()[0] + 1.0, bounds.max()[1] + 1.0, centroid[2]],
        ),
    ]
}

fn check_point(id: u64, position: [f64; 3]) -> CheckPoint {
    CheckPoint::new(
        CheckPointId::new(id).expect("benchmark Check Point identity is valid"),
        position,
    )
    .expect("benchmark Check Point position is finite")
}

fn asserted_landxml_options() -> LandXmlOptions {
    LandXmlOptions::metric_metres("Punctra Terrain Benchmark", "2026-08-10", "12:34:56Z")
        .expect("benchmark LandXML options are valid")
        .assert_coordinates_are_metric_metres()
}

#[allow(clippy::too_many_arguments)]
fn report_resource_facts(
    descriptor: &TerrainDescriptor,
    qa: &CheckPointReport,
    landxml: LandXmlReceipt,
    terrain_limits: TerrainLimits,
    qa_limits: CheckPointLimits,
    landxml_limits: LandXmlLimits,
    timings: EvidenceTimings,
) {
    let machine = machine_name();
    eprintln!(
        concat!(
            "PUNCTRA_TERRAIN_RESOURCE_FACTS={{",
            "\"schema\":1,\"machine\":\"{}\",\"os\":\"{}\",\"arch\":\"{}\",",
            "\"input_points\":{},\"vertices\":{},\"faces\":{},\"hull_vertices\":{},",
            "\"recipe_hash\":\"{}\",\"input_hash\":\"{}\",",
            "\"geometry_hash\":\"{}\",\"topology_hash\":\"{}\",\"artifact_hash\":\"{}\",",
            "\"descriptor_accounted_peak_working_bytes\":{},",
            "\"descriptor_retained_surface_bytes\":{},\"descriptor_topology_steps\":{},",
            "\"terrain_limit_working_bytes\":{},\"terrain_limit_surface_bytes\":{},",
            "\"terrain_limit_work_units\":{},\"qa_points\":{},\"qa_face_tests\":{},",
            "\"qa_accounted_peak_working_bytes\":{},\"qa_limit_face_tests\":{},",
            "\"qa_limit_working_bytes\":{},\"landxml_output_bytes\":{},",
            "\"landxml_content_hash\":\"{}\",\"landxml_limit_output_bytes\":{},",
            "\"landxml_limit_working_bytes\":{},",
            "\"one_shot_derive_elapsed_us\":{},\"one_shot_qa_elapsed_us\":{},",
            "\"one_shot_landxml_elapsed_us\":{},",
            "\"worker_heap_measurement\":null,",
            "\"accounting_note\":\"descriptor accounting is not observed worker heap\"",
            "}}"
        ),
        machine,
        std::env::consts::OS,
        std::env::consts::ARCH,
        descriptor.input_point_count(),
        descriptor.vertex_count(),
        descriptor.face_count(),
        descriptor.hull_vertex_count(),
        descriptor.recipe_hash(),
        descriptor.input_hash(),
        descriptor.geometry_hash(),
        descriptor.topology_hash(),
        descriptor.artifact_hash(),
        descriptor.accounted_peak_working_bytes(),
        descriptor.retained_surface_bytes(),
        descriptor.topology_steps(),
        terrain_limits.max_working_bytes(),
        terrain_limits.max_surface_bytes(),
        terrain_limits.max_work_units(),
        qa.results().len(),
        qa.face_tests(),
        qa.accounted_peak_working_bytes(),
        qa_limits.max_face_tests(),
        qa_limits.max_working_bytes(),
        landxml.byte_length(),
        landxml.content_hash(),
        landxml_limits.max_output_bytes(),
        landxml_limits.max_working_bytes(),
        timings.derive,
        timings.qa,
        timings.landxml,
    );
}

#[derive(Clone, Copy)]
struct EvidenceTimings {
    derive: u128,
    qa: u128,
    landxml: u128,
}

fn machine_name() -> String {
    let candidate = std::env::var(MACHINE_NAME_ENV)
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(system_hostname)
        .unwrap_or_else(|| "unnamed-local-machine".to_owned());
    assert!(
        !candidate.is_empty()
            && candidate.len() <= 128
            && candidate
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "{MACHINE_NAME_ENV} or the local hostname must contain 1-128 ASCII letters, digits, '.', '_', or '-'"
    );
    candidate
}

fn system_hostname() -> Option<String> {
    let output = Command::new("hostname").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let hostname = String::from_utf8(output.stdout).ok()?;
    let hostname = hostname.trim();
    (!hostname.is_empty()).then(|| hostname.to_owned())
}

fn configured_point_count() -> usize {
    let Some(value) = std::env::var_os(POINT_COUNT_ENV) else {
        return DEFAULT_POINT_COUNT;
    };
    let value = value
        .into_string()
        .expect("PUNCTRA_TERRAIN_BENCH_POINTS must be Unicode");
    let count = value
        .parse::<usize>()
        .expect("PUNCTRA_TERRAIN_BENCH_POINTS must be a positive integer");
    assert!(
        (3..=MAXIMUM_POINT_COUNT).contains(&count),
        "PUNCTRA_TERRAIN_BENCH_POINTS must be between 3 and {MAXIMUM_POINT_COUNT}"
    );
    count
}

struct Fixture {
    point_count: u64,
    snapshot: Snapshot,
    _workspace: Workspace,
    directory: FixtureDirectory,
    next_export: AtomicU64,
}

impl Fixture {
    fn new(point_count: usize) -> Self {
        let directory = FixtureDirectory::new().expect("create benchmark directory");
        let input = memory_fixture(point_count);
        let source = source_memory::open(input)
            .blocking_wait()
            .expect("benchmark Source opens");
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("benchmark index prepares");
        let workspace = create(
            directory.path().join("fixture.pcw"),
            index,
            WorkspaceSchema::new(classification_attribute()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("benchmark Workspace creates");
        let snapshot = workspace.head();
        Self {
            point_count: u64::try_from(point_count).expect("fixture count fits u64"),
            snapshot,
            _workspace: workspace,
            directory,
            next_export: AtomicU64::new(1),
        }
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    fn next_export_target(&self) -> PathBuf {
        let sequence = self.next_export.fetch_add(1, Ordering::Relaxed);
        self.directory
            .path()
            .join(format!("terrain-{sequence}.xml"))
    }
}

fn memory_fixture(point_count: usize) -> MemorySource {
    let width = integer_ceil_sqrt(point_count);
    let ticks = (0..point_count)
        .map(|ordinal| fixture_ticks(ordinal, width))
        .collect::<Vec<_>>();
    let definition = AttributeDefinition::new(
        classification_attribute(),
        "classification",
        AttributeDataType::U8,
    )
    .expect("benchmark classification definition is valid");
    let values = AttributeValues::u8(vec![GROUND_CLASSIFICATION; point_count]);
    let column = AttributeColumn::new(definition, values)
        .expect("benchmark classification values are valid");
    let attributes = AttributeColumns::new(vec![column], point_count)
        .expect("benchmark Attribute rows are aligned");
    MemorySource::from_columns(
        PositionTransform::new([0.0; 3], [1.0, 1.0, 0.01]).expect("benchmark transform is valid"),
        CoordinateReference::Unknown,
        ticks,
        attributes,
    )
    .expect("benchmark memory Source is valid")
}

fn fixture_ticks(ordinal: usize, width: usize) -> [i64; 3] {
    let x = i64::try_from(ordinal % width).expect("fixture x fits i64");
    let y = i64::try_from(ordinal / width).expect("fixture y fits i64");
    [x, y, (x * x + 3 * y * y + x * y).rem_euclid(100_000)]
}

fn integer_ceil_sqrt(value: usize) -> usize {
    let mut low = 1_usize;
    let mut high = value;
    while low < high {
        let middle = low + (high - low) / 2;
        if middle >= value / middle && middle.saturating_mul(middle) >= value {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

fn classification_attribute() -> AttributeId {
    AttributeId::new(CLASSIFICATION_ATTRIBUTE_ID)
        .expect("benchmark classification Attribute identity is nonzero")
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
                "punctra-point-terrain-bench-{}-{timestamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create isolated point-terrain benchmark directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

criterion_group!(benches, benchmark_terrain);
criterion_main!(benches);
