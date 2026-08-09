//! Source-scale sequential-read and Full-open benchmarks for LAS and LAZ.
//!
//! Fixtures contain 1,000,000 Points by default. Set
//! `PUNCTRA_SOURCE_LAS_BENCH_POINTS` to exercise a larger source-scale count.

#![allow(missing_docs)] // Criterion generates a public benchmark-group function.

use std::error::Error;
use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use las::point::{Classification, Format};
use las::{Builder, Color, Point, Transform, Vector, Writer};
use point_source::{ReadBudget, ReadRequest, Source};

const DEFAULT_POINT_COUNT: u64 = 1_000_000;
const POINT_COUNT_ENV: &str = "PUNCTRA_SOURCE_LAS_BENCH_POINTS";
const MAX_BATCH_POINTS: u64 = 65_536;
const MAX_BATCH_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ADAPTER_WORKING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_READ_PEAK_HEAP_BYTES: u64 = 32 * 1024 * 1024;
const POSITION_SCALE: f64 = 0.001;
const X_OFFSET: f64 = 500_000.0;
const Y_OFFSET: f64 = 4_600_000.0;
const Z_OFFSET: f64 = 100.0;

fn benchmark_source_las(criterion: &mut Criterion) {
    let point_count = configured_point_count();
    let fixtures = FixtureSet::new(point_count);

    assert_peak_heap(&fixtures);
    benchmark_preopened_reads(criterion, &fixtures);
    benchmark_full_open(criterion, &fixtures);
}

fn assert_peak_heap(fixtures: &FixtureSet) {
    for fixture in fixtures.all() {
        let allocations = allocation_counter::measure(|| {
            black_box(read_all(fixture));
        });
        assert!(
            allocations.bytes_max <= MAX_READ_PEAK_HEAP_BYTES,
            "{} read peak heap was {} bytes, above the {} byte ceiling",
            fixture.label,
            allocations.bytes_max,
            MAX_READ_PEAK_HEAP_BYTES
        );
        assert_eq!(
            allocations.bytes_current, 0,
            "{} read retained measured heap allocations",
            fixture.label
        );
        eprintln!(
            "{} measured read peak heap: {} bytes (ceiling: {} bytes)",
            fixture.label, allocations.bytes_max, MAX_READ_PEAK_HEAP_BYTES
        );
    }
}

fn benchmark_preopened_reads(criterion: &mut Criterion, fixtures: &FixtureSet) {
    let mut group = criterion.benchmark_group("source-las/pre-opened-sequential-read");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));

    for fixture in fixtures.all() {
        group.throughput(Throughput::Elements(fixture.point_count));
        group.bench_with_input(
            BenchmarkId::new("points-per-second", fixture.label),
            fixture,
            |bencher, fixture| {
                bencher.iter(|| black_box(read_all(fixture)));
            },
        );

        group.throughput(Throughput::Bytes(fixture.source_file_bytes));
        group.bench_with_input(
            BenchmarkId::new("source-bytes-per-second", fixture.label),
            fixture,
            |bencher, fixture| {
                bencher.iter(|| black_box(read_all(fixture)));
            },
        );
    }
    group.finish();
}

fn benchmark_full_open(criterion: &mut Criterion, fixtures: &FixtureSet) {
    let mut group = criterion.benchmark_group("source-las/full-open");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));

    for fixture in fixtures.all() {
        group.throughput(Throughput::Bytes(fixture.source_file_bytes));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.label),
            fixture,
            |bencher, fixture| {
                bencher.iter(|| {
                    let source = source_las::open(black_box(&fixture.path))
                        .blocking_wait()
                        .expect("Full benchmark open must succeed");
                    assert_eq!(source.metadata().point_count(), fixture.point_count);
                    black_box(source.identity())
                });
            },
        );
    }
    group.finish();
}

fn read_all(fixture: &Fixture) -> ReadObservation {
    let budget = read_budget();
    assert_eq!(
        budget.max_adapter_working_bytes(),
        MAX_ADAPTER_WORKING_BYTES
    );
    let mut batches = fixture
        .source
        .read(ReadRequest::all().budget(budget))
        .expect("pre-opened benchmark read must start");
    let mut observation = ReadObservation::default();

    while let Some(batch) = batches.next().expect("benchmark batch must decode") {
        assert!(batch.point_count() <= MAX_BATCH_POINTS);
        assert!(batch.estimated_payload_bytes() <= MAX_BATCH_PAYLOAD_BYTES);
        observation.batch_count = observation
            .batch_count
            .checked_add(1)
            .expect("batch count fits u64");
        observation.point_count = observation
            .point_count
            .checked_add(batch.point_count())
            .expect("Point count fits u64");
        observation.canonical_payload_bytes = observation
            .canonical_payload_bytes
            .checked_add(batch.estimated_payload_bytes())
            .expect("canonical payload byte count fits u64");
        black_box(batch.last_ordinal());
    }

    let summary = batches
        .summary()
        .expect("successful benchmark exhaustion has an exact summary");
    assert_eq!(summary.exact_count(), fixture.point_count);
    assert_eq!(observation.point_count, summary.exact_count());
    assert_eq!(summary.source(), fixture.source.identity());
    observation
}

fn read_budget() -> ReadBudget {
    ReadBudget::new(MAX_BATCH_POINTS, MAX_BATCH_PAYLOAD_BYTES)
        .expect("benchmark limits are nonzero")
        .with_max_adapter_working_bytes(MAX_ADAPTER_WORKING_BYTES)
}

#[derive(Clone, Copy, Debug, Default)]
struct ReadObservation {
    batch_count: u64,
    point_count: u64,
    canonical_payload_bytes: u64,
}

struct FixtureSet {
    las: Fixture,
    laz: Fixture,
    _directory: FixtureDirectory,
}

impl FixtureSet {
    fn new(point_count: u64) -> Self {
        let directory = FixtureDirectory::new().expect("create benchmark fixture directory");
        let las_path = directory.path().join("source-scale.las");
        let laz_path = directory.path().join("source-scale.laz");

        eprintln!("Generating {point_count} Point LAS and LAZ benchmark fixtures...");
        write_fixture(&las_path, point_count).expect("generate LAS benchmark fixture");
        write_fixture(&laz_path, point_count).expect("generate LAZ benchmark fixture");

        Self {
            las: Fixture::open("LAS", las_path, point_count),
            laz: Fixture::open("LAZ", laz_path, point_count),
            _directory: directory,
        }
    }

    fn all(&self) -> [&Fixture; 2] {
        [&self.las, &self.laz]
    }
}

struct Fixture {
    label: &'static str,
    path: PathBuf,
    point_count: u64,
    source_file_bytes: u64,
    source: Source,
}

impl Fixture {
    fn open(label: &'static str, path: PathBuf, point_count: u64) -> Self {
        let source_file_bytes = std::fs::metadata(&path)
            .expect("read fixture metadata")
            .len();
        let source = source_las::open(&path)
            .blocking_wait()
            .unwrap_or_else(|error| panic!("pre-open {label} benchmark fixture: {error:?}"));
        assert_eq!(source.metadata().point_count(), point_count);
        Self {
            label,
            path,
            point_count,
            source_file_bytes,
            source,
        }
    }
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
                "punctra-source-las-bench-{}-{timestamp}-{attempt}",
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
            "could not reserve a unique source-las benchmark directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!(
                "warning: could not remove benchmark fixtures at {}: {error}",
                self.path.display()
            );
        }
    }
}

fn configured_point_count() -> u64 {
    let Some(raw) = std::env::var_os(POINT_COUNT_ENV) else {
        return DEFAULT_POINT_COUNT;
    };
    let raw = raw
        .into_string()
        .unwrap_or_else(|_| panic!("{POINT_COUNT_ENV} must contain UTF-8 decimal digits"));
    let point_count = raw
        .parse::<u64>()
        .unwrap_or_else(|error| panic!("invalid {POINT_COUNT_ENV}={raw:?}: {error}"));
    assert!(
        point_count > 0,
        "{POINT_COUNT_ENV} must be greater than zero"
    );
    point_count
}

fn write_fixture(path: &Path, point_count: u64) -> Result<(), Box<dyn Error>> {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(3)?;
    builder.transforms = Vector {
        x: Transform {
            scale: POSITION_SCALE,
            offset: X_OFFSET,
        },
        y: Transform {
            scale: POSITION_SCALE,
            offset: Y_OFFSET,
        },
        z: Transform {
            scale: POSITION_SCALE,
            offset: Z_OFFSET,
        },
    };
    "Punctra benchmark".clone_into(&mut builder.system_identifier);
    "source-las Criterion fixture".clone_into(&mut builder.generating_software);

    let mut writer = Writer::from_path(path, builder.into_header()?)?;
    for ordinal in 0..point_count {
        writer.write_point(fixture_point(ordinal))?;
    }
    writer.close()?;
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn fixture_point(ordinal: u64) -> Point {
    let color = u16::try_from(ordinal % (u64::from(u16::MAX) + 1)).expect("modulo value fits u16");
    let x_ticks = (ordinal % 10_000) * 10;
    let y_ticks = (ordinal / 10_000) * 10;
    let z_ticks = ordinal % 4_096;
    Point {
        x: x_ticks as f64 * POSITION_SCALE + X_OFFSET,
        y: y_ticks as f64 * POSITION_SCALE + Y_OFFSET,
        z: z_ticks as f64 * POSITION_SCALE + Z_OFFSET,
        intensity: color,
        return_number: 1,
        number_of_returns: 1,
        is_edge_of_flight_line: ordinal % 10_000 == 9_999,
        classification: Classification::Ground,
        user_data: u8::try_from(ordinal % (u64::from(u8::MAX) + 1)).expect("modulo value fits u8"),
        point_source_id: u16::try_from((ordinal / 100_000) % 65_536)
            .expect("modulo value fits u16"),
        gps_time: Some(1_000_000.0 + ordinal as f64 * 0.000_001),
        color: Some(Color::new(
            color,
            color.rotate_left(5),
            color.rotate_left(11),
        )),
        ..Point::default()
    }
}

criterion_group!(benches, benchmark_source_las);
criterion_main!(benches);
