//! CPU benchmark for exact public screen-through composition.
//!
//! Timed scope starts at [`point_review::screen_through`] and ends after its
//! terminal Inspection is joined. It includes exact Snapshot row streaming,
//! f64 projection, identity collection, and Point Set construction. Generated
//! Source, index, and Workspace creation happen once outside the measured
//! scope. Before timing, retained resident and zero-resident-limit results are
//! fully traversed through the public checked reader. Their storage evidence
//! compares recursive logical file lengths under the fixture-owned temporary
//! root against a repeated stable baseline. No renderer, GPU, window,
//! presentation, allocator, heap, or process-memory work is measured.

#![allow(missing_docs)]

use std::{fs, hint::black_box, io, path::Path, time::Duration};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_index::{PrepareLimits, prepare};
use point_review::{Inspection, ScreenRect, ScreenReviewLimits, ScreenSelection, screen_through};
use point_workspace::{OpenLimits, PointIdReadLimits, PointSetLimits, WorkspaceSchema, create};
use render_protocol::{Camera, Viewport};
use source_memory::MemorySource;

const CLASSIFICATION: u32 = 101;
const POINT_COUNT: usize = 20_000;
const EXPECTED_MATCHES: u64 = 5_151;

fn review_benchmark(criterion: &mut Criterion) {
    let fixture = Fixture::new();
    let snapshot = fixture.workspace.head();
    let camera = Camera::orthographic(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        100.0,
        1.0,
        200.0,
    )
    .unwrap();
    let selection = ScreenSelection::new(
        ScreenRect::new([200.0, 100.0], [600.0, 300.0]).unwrap(),
        camera,
        Viewport::new(800, 400).unwrap(),
    )
    .unwrap();
    let resident = ScreenReviewLimits::default();
    let forced_spill = forced_spill_limits();
    let baseline = fixture.stable_owned_file_footprint();
    let resident_inspection = review_result(&snapshot, selection, &resident);
    verify_complete_result(&resident_inspection);
    let with_resident = fixture.stable_owned_file_footprint();
    assert_eq!(
        with_resident, baseline,
        "a retained resident result must not create owned fixture files"
    );
    let spilled_inspection = review_result(&snapshot, selection, &forced_spill);
    verify_complete_result(&spilled_inspection);
    let with_spill = fixture.stable_owned_file_footprint();
    let spill_delta = with_spill
        .checked_delta(with_resident)
        .expect("retained spill storage only grows the owned fixture footprint");
    assert!(
        spill_delta.files > 0 && spill_delta.bytes > 0,
        "zero-resident non-empty Point Set must retain non-empty owned spill storage"
    );
    report_resource_facts(
        "resident",
        &resident,
        &resident_inspection,
        OwnedFileFootprint::default(),
        "verified-no-owned-file-growth",
    );
    report_resource_facts(
        "forced-spill",
        &forced_spill,
        &spilled_inspection,
        spill_delta,
        "verified-zero-resident-limit-and-owned-file-growth",
    );
    let mut group = criterion.benchmark_group("point_review_screen_through");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(250));
    group.measurement_time(Duration::from_millis(750));
    group.throughput(Throughput::Elements(
        u64::try_from(POINT_COUNT).expect("benchmark point count fits u64"),
    ));

    group.bench_function("20000_points_resident", |bencher| {
        bencher.iter(|| review_once(&snapshot, black_box(selection), &resident));
    });
    group.bench_function("20000_points_forced_spill", |bencher| {
        bencher.iter(|| review_once(&snapshot, black_box(selection), &forced_spill));
    });
    group.finish();
    black_box(resident_inspection.summary());
    black_box(spilled_inspection.summary());
}

fn report_resource_facts(
    disposition: &str,
    limits: &ScreenReviewLimits,
    inspection: &Inspection,
    temporary_delta: OwnedFileFootprint,
    storage_evidence: &str,
) {
    let rows = limits.point_row_limits();
    let point_set = limits.point_set_limits();
    eprintln!(
        "point-review observed benchmark facts: source_points={POINT_COUNT} exact_matches={} disposition={disposition} storage_evidence={storage_evidence} accounted_peak_working_bytes={} composition_working_ceiling={} row_working_ceiling={} point_set_working_ceiling={} resident_point_set_ceiling={} temporary_point_set_ceiling={} observed_owned_temporary_file_delta={} observed_owned_temporary_file_length_byte_delta={} accounting_kind=algorithm-conservative-not-measured-heap",
        inspection.summary().exact_count(),
        inspection.summary().accounted_peak_working_bytes(),
        limits.max_working_bytes(),
        rows.max_working_bytes(),
        point_set.max_working_bytes(),
        point_set.max_resident_bytes(),
        point_set.max_temporary_bytes(),
        temporary_delta.files,
        temporary_delta.bytes,
    );
}

fn review_once(
    snapshot: &point_workspace::Snapshot,
    selection: ScreenSelection,
    limits: &ScreenReviewLimits,
) {
    let inspection = review_result(snapshot, selection, limits);
    black_box(inspection.points().metadata());
    black_box(inspection.summary().accounted_peak_working_bytes());
}

fn review_result(
    snapshot: &point_workspace::Snapshot,
    selection: ScreenSelection,
    limits: &ScreenReviewLimits,
) -> Inspection {
    let inspection = screen_through(snapshot, selection, *limits)
        .blocking_wait()
        .expect("benchmark review completes");
    assert_eq!(inspection.summary().exact_count(), EXPECTED_MATCHES);
    assert!(inspection.summary().accounted_peak_working_bytes() > 0);
    assert!(inspection.summary().accounted_peak_working_bytes() <= limits.max_working_bytes());
    inspection
}

fn verify_complete_result(inspection: &Inspection) {
    let mut batches = inspection
        .points()
        .ids(PointIdReadLimits::default())
        .expect("benchmark Point Set opens through its public checked reader");
    let mut count = 0_u64;
    while let Some(batch) = batches
        .next()
        .expect("benchmark Point Set storage verifies during complete traversal")
    {
        count = count
            .checked_add(u64::try_from(batch.ids().len()).expect("batch count fits u64"))
            .expect("benchmark traversal count does not overflow");
    }
    assert_eq!(count, EXPECTED_MATCHES);
}

fn forced_spill_limits() -> ScreenReviewLimits {
    let defaults = ScreenReviewLimits::default();
    let point_set = defaults.point_set_limits();
    let forced_spill = PointSetLimits::new(
        point_set.candidate_limits(),
        point_set.source_read_budget(),
        point_set.max_input_point_ids(),
        point_set.max_output_points(),
        point_set.max_overlay_segments(),
        point_set.max_overlay_bytes(),
        point_set.max_working_bytes(),
        0,
        point_set.max_temporary_bytes(),
    );
    ScreenReviewLimits::new(
        defaults.point_row_limits(),
        forced_spill,
        defaults.max_screen_matches(),
        defaults.max_working_bytes(),
    )
}

struct Fixture {
    workspace: point_workspace::Workspace,
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("punctra-point-review-bench-")
            .tempdir()
            .unwrap();
        let ticks = (0..POINT_COUNT)
            .map(|ordinal| {
                let x = i64::try_from(ordinal % 200).unwrap() - 100;
                let y = i64::try_from((ordinal / 200) % 100).unwrap() - 50;
                [x, y, -100]
            })
            .collect::<Vec<_>>();
        let definition = AttributeDefinition::new(
            AttributeId::new(CLASSIFICATION).unwrap(),
            "classification",
            AttributeDataType::U8,
        )
        .unwrap();
        let classification =
            AttributeColumn::new(definition, AttributeValues::u8(vec![2; POINT_COUNT])).unwrap();
        let columns = AttributeColumns::new(vec![classification], POINT_COUNT).unwrap();
        let memory = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [1.0; 3]).unwrap(),
            CoordinateReference::Unknown,
            ticks,
            columns,
        )
        .unwrap();
        let source = source_memory::open(memory).blocking_wait().unwrap();
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        let workspace = create(
            directory.path().join("fixture.pcw"),
            index,
            WorkspaceSchema::new(AttributeId::new(CLASSIFICATION).unwrap()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        Self {
            workspace,
            directory,
        }
    }

    fn stable_owned_file_footprint(&self) -> OwnedFileFootprint {
        let first = owned_file_footprint(self.directory.path())
            .expect("benchmark reads its complete owned fixture footprint");
        let second = owned_file_footprint(self.directory.path())
            .expect("benchmark repeats its complete owned fixture footprint");
        assert_eq!(
            first, second,
            "owned index and Workspace baseline must be stable before comparison"
        );
        first
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OwnedFileFootprint {
    files: u64,
    bytes: u64,
}

impl OwnedFileFootprint {
    fn checked_delta(self, baseline: Self) -> Option<Self> {
        Some(Self {
            files: self.files.checked_sub(baseline.files)?,
            bytes: self.bytes.checked_sub(baseline.bytes)?,
        })
    }
}

fn owned_file_footprint(root: &Path) -> io::Result<OwnedFileFootprint> {
    let mut footprint = OwnedFileFootprint::default();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let child = owned_file_footprint(&entry.path())?;
            footprint.files = footprint
                .files
                .checked_add(child.files)
                .ok_or_else(|| io::Error::other("owned fixture file-count footprint overflowed"))?;
            footprint.bytes = footprint
                .bytes
                .checked_add(child.bytes)
                .ok_or_else(|| io::Error::other("owned fixture byte footprint overflowed"))?;
        } else if file_type.is_file() {
            footprint.files = footprint
                .files
                .checked_add(1)
                .ok_or_else(|| io::Error::other("owned fixture file-count footprint overflowed"))?;
            footprint.bytes = footprint
                .bytes
                .checked_add(entry.metadata()?.len())
                .ok_or_else(|| io::Error::other("owned fixture byte footprint overflowed"))?;
        } else {
            return Err(io::Error::other(
                "owned fixture contains an unsupported non-file entry",
            ));
        }
    }
    Ok(footprint)
}

criterion_group!(benches, review_benchmark);
criterion_main!(benches);
