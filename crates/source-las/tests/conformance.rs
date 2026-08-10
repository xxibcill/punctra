//! Caller-facing conformance evidence shared by memory, LAS, and LAZ Sources.

use std::fs::{self, OpenOptions as FsOpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use las::point::{Classification, Format, ScanDirection};
use las::raw::point::ScanAngle;
use las::{Builder, Color, Point, Transform, Vector, Vlr, Writer};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeSchema, AttributeValues, CoordinateReference, MAX_METADATA_RECORDS, MetadataRecord,
    PointBatch, PositionTransform, SourceMetadata, WorldBounds,
};
use point_source::{
    AttributeSelection, OpenOptions, ReadBudget, ReadRequest, Source, SourceError, SourceRecord,
    SourceSpan, VerificationPolicy,
};
use source_las::{open as open_file, open_with as open_file_with};
use source_memory::{MemorySource, open as open_memory, open_with as open_memory_with};

const POINT_COUNT: usize = 9;
const EXTRA_BYTES_WIDTH: usize = 3;
const GENEROUS_BYTES: u64 = 1024 * 1024;
const WKT: &str = "LOCAL_CS[\"Punctra conformance fixture\"]";
const FIRST_VLR: &[u8] = b"first regular VLR";
const SECOND_VLR: &[u8] = b"second regular VLR";
const LAST_EVLR: &[u8] = b"last extended VLR";

const INTENSITY: u32 = 1;
const RETURN_NUMBER: u32 = 2;
const NUMBER_OF_RETURNS: u32 = 3;
const SCAN_DIRECTION: u32 = 4;
const EDGE_OF_FLIGHT_LINE: u32 = 5;
const CLASSIFICATION: u32 = 6;
const SYNTHETIC: u32 = 7;
const KEY_POINT: u32 = 8;
const WITHHELD: u32 = 9;
const OVERLAP: u32 = 10;
const SCANNER_CHANNEL: u32 = 11;
const SCAN_ANGLE: u32 = 12;
const USER_DATA: u32 = 13;
const POINT_SOURCE_ID: u32 = 14;
const GPS_TIME: u32 = 15;
const RED: u32 = 16;
const GREEN: u32 = 17;
const BLUE: u32 = 18;
const NIR: u32 = 26;
const EXTRA_BYTES: u32 = 4096;

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceKind {
    Memory,
    Las,
    Laz,
}

#[derive(Clone)]
enum PublicInput {
    Memory(MemorySource),
    File(PathBuf),
}

#[derive(Clone)]
struct FixtureInput {
    kind: SourceKind,
    input: PublicInput,
}

impl FixtureInput {
    fn open(&self) -> Result<Source, SourceError> {
        match &self.input {
            PublicInput::Memory(input) => open_memory(input.clone()).blocking_wait(),
            PublicInput::File(path) => open_file(path).blocking_wait(),
        }
    }

    fn open_with(
        &self,
        record: SourceRecord,
        policy: VerificationPolicy,
    ) -> Result<Source, SourceError> {
        let options = OpenOptions::match_record(record, policy);
        match &self.input {
            PublicInput::Memory(input) => open_memory_with(input.clone(), options).blocking_wait(),
            PublicInput::File(path) => open_file_with(path, options).blocking_wait(),
        }
    }

    fn source(&self) -> Source {
        let kind = self.kind;
        self.open()
            .unwrap_or_else(|error| panic!("failed to open {kind:?} fixture: {error}"))
    }
}

struct FixtureSet {
    directory: PathBuf,
    memory: MemorySource,
    uncompressed: PathBuf,
    compressed: PathBuf,
}

impl FixtureSet {
    fn new() -> Self {
        let directory = unique_temp_directory();
        fs::create_dir(&directory).unwrap();
        let uncompressed = directory.join("equivalent.las");
        let compressed = directory.join("equivalent.laz");
        let rows = fixture_rows();
        write_las_fixture(&uncompressed, &rows);
        write_las_fixture(&compressed, &rows);
        Self {
            directory,
            memory: memory_fixture(&rows),
            uncompressed,
            compressed,
        }
    }

    fn inputs(&self) -> [FixtureInput; 3] {
        [
            FixtureInput {
                kind: SourceKind::Memory,
                input: PublicInput::Memory(self.memory.clone()),
            },
            FixtureInput {
                kind: SourceKind::Las,
                input: PublicInput::File(self.uncompressed.clone()),
            },
            FixtureInput {
                kind: SourceKind::Laz,
                input: PublicInput::File(self.compressed.clone()),
            },
        ]
    }
}

impl Drop for FixtureSet {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalRow {
    ticks: [i64; 3],
    intensity: u16,
    return_number: u8,
    number_of_returns: u8,
    scan_direction: u8,
    edge_of_flight_line: u8,
    classification: u8,
    synthetic: u8,
    key_point: u8,
    withheld: u8,
    overlap: u8,
    scanner_channel: u8,
    scan_angle: i16,
    user_data: u8,
    point_source_id: u16,
    gps_time_bits: u64,
    red: u16,
    green: u16,
    blue: u16,
    nir: u16,
    extra_bytes: [u8; EXTRA_BYTES_WIDTH],
}

struct ReadOutcome {
    rows: Vec<CanonicalRow>,
    batch_sizes: Vec<usize>,
    spans: Vec<SourceSpan>,
    attributes: Vec<AttributeId>,
    exact_count: u64,
    budget: ReadBudget,
}

#[test]
fn equivalent_sources_preserve_rows_identity_and_normalized_order_across_read_shapes() {
    let fixtures = FixtureSet::new();
    let expected = fixture_rows();
    let mut identities = Vec::new();

    for input in fixtures.inputs() {
        let kind = input.kind;
        let source = input.source();
        let repeated_open = input.source();
        assert_eq!(source.identity(), repeated_open.identity(), "{kind:?}");
        assert_metadata_contract(&source);

        let narrow_budget = read_budget(2, GENEROUS_BYTES);
        let narrow = read_all(&source, ReadRequest::all().budget(narrow_budget));
        assert_eq!(narrow.rows, expected, "{kind:?}");
        assert_eq!(narrow.batch_sizes, vec![2, 2, 2, 2, 1], "{kind:?}");
        assert_complete_summary(&narrow, narrow_budget);

        let partitioned_budget = read_budget(5, GENEROUS_BYTES);
        let partitioned = read_all(
            &repeated_open,
            ReadRequest::all()
                .spans([span(4, 5), span(0, 3), span(2, 4), span(4, 5)])
                .budget(partitioned_budget),
        );
        assert_eq!(partitioned.rows, expected, "{kind:?}");
        assert_eq!(partitioned.batch_sizes, vec![5, 4], "{kind:?}");
        assert_eq!(partitioned.spans, vec![span(0, POINT_COUNT as u64)]);
        assert_complete_summary(&partitioned, partitioned_budget);

        let repeated = read_all(
            &source,
            ReadRequest::all().budget(read_budget(3, GENEROUS_BYTES)),
        );
        assert_eq!(repeated.rows, expected, "{kind:?}");
        identities.push((kind, source.identity()));
    }

    let uncompressed_id = identity_for(&identities, SourceKind::Las);
    let compressed_id = identity_for(&identities, SourceKind::Laz);
    assert_ne!(
        uncompressed_id, compressed_id,
        "compression changes exact file-byte identity"
    );
}

#[test]
fn projection_and_every_hard_read_budget_are_enforced_through_the_public_source() {
    let fixtures = FixtureSet::new();

    for input in fixtures.inputs() {
        let kind = input.kind;
        let source = input.source();
        assert_projection(&source, kind);

        let point_limited = read_all(
            &source,
            ReadRequest::all().budget(read_budget(3, GENEROUS_BYTES)),
        );
        assert_eq!(point_limited.batch_sizes, vec![3, 3, 3], "{kind:?}");

        let bytes_per_point = canonical_bytes_per_point();
        let payload_limited = read_all(
            &source,
            ReadRequest::all().budget(read_budget(99, bytes_per_point * 2)),
        );
        assert_eq!(payload_limited.batch_sizes, vec![2, 2, 2, 2, 1], "{kind:?}");
        assert!(
            payload_limited
                .batch_sizes
                .iter()
                .all(|&points| points as u64 * bytes_per_point <= bytes_per_point * 2)
        );

        let payload_error = read_error(
            &source,
            ReadRequest::all().budget(read_budget(99, bytes_per_point - 1)),
        );
        assert!(matches!(
            payload_error,
            SourceError::ResourceLimit {
                limit: "batch payload bytes",
                required,
                allowed,
            } if required == bytes_per_point && allowed == bytes_per_point - 1
        ));

        let max_points = ReadBudget::new(99, GENEROUS_BYTES)
            .unwrap()
            .with_max_points((POINT_COUNT - 1) as u64)
            .with_max_adapter_working_bytes(GENEROUS_BYTES);
        assert!(matches!(
            source.read(ReadRequest::all().budget(max_points)),
            Err(SourceError::ResourceLimit {
                limit: "requested Point count",
                required,
                allowed,
            }) if required == POINT_COUNT as u64 && allowed == (POINT_COUNT - 1) as u64
        ));

        let tiny_working = ReadBudget::new(1, GENEROUS_BYTES)
            .unwrap()
            .with_max_adapter_working_bytes(1);
        match kind {
            SourceKind::Memory => {
                let outcome = read_all(&source, ReadRequest::all().budget(tiny_working));
                assert_eq!(outcome.rows, fixture_rows());
            }
            SourceKind::Las | SourceKind::Laz => {
                assert!(matches!(
                    read_error(&source, ReadRequest::all().budget(tiny_working)),
                    SourceError::ResourceLimit { allowed: 1, .. }
                ));
            }
        }
    }
}

#[test]
fn persisted_full_records_reopen_and_cancelled_reads_fuse_for_every_source() {
    let fixtures = FixtureSet::new();

    for input in fixtures.inputs() {
        let kind = input.kind;
        let source = input.source();
        let encoded = serde_json::to_vec(source.record()).unwrap();
        let record: SourceRecord = serde_json::from_slice(&encoded).unwrap();
        let reopened = input
            .open_with(record.clone(), VerificationPolicy::Full)
            .unwrap_or_else(|error| panic!("Full reopen failed for {kind:?}: {error}"));
        assert_eq!(reopened.identity(), source.identity());
        assert_eq!(reopened.metadata(), source.metadata());
        assert_eq!(reopened.provenance(), source.provenance());

        match kind {
            SourceKind::Memory => {
                let fast = input
                    .open_with(record, VerificationPolicy::FastOnly)
                    .unwrap();
                assert_eq!(fast.identity(), source.identity());
            }
            SourceKind::Las | SourceKind::Laz => assert!(matches!(
                input.open_with(record, VerificationPolicy::FastOnly),
                Err(SourceError::VerificationRequired)
            )),
        }

        let mut batches = source.points().unwrap();
        let handle = batches.handle();
        handle.cancel();
        assert!(matches!(batches.next(), Err(SourceError::Cancelled)));
        assert!(batches.next().unwrap().is_none());
        assert!(batches.summary().is_none());
    }
}

#[test]
fn metadata_payload_order_and_unambiguous_wkt_survive_las_and_laz() {
    let fixtures = FixtureSet::new();

    for input in fixtures.inputs() {
        let kind = input.kind;
        let source = input.source();
        let metadata = source.metadata();
        assert_eq!(metadata.coordinate_reference().as_wkt(), Some(WKT));
        let records = metadata.metadata_records();
        let first = record_index(records, FIRST_VLR);
        let wkt = record_index(records, &wkt_payload());
        let second = record_index(records, SECOND_VLR);
        let last = record_index(records, LAST_EVLR);
        assert!(first < wkt && wkt < second && second < last, "{kind:?}");

        if kind != SourceKind::Memory {
            assert_eq!(records[first].namespace(), "las.vlr");
            assert_eq!(records[second].namespace(), "las.vlr");
            assert_eq!(records[last].namespace(), "las.evlr");
            assert!(records[first].name().starts_with("vendor.first:101:"));
            assert!(records[last].name().starts_with("vendor.last:303:"));
        }
    }
}

#[test]
fn every_raw_duplicate_wkt_record_makes_the_coordinate_reference_unknown() {
    let fixtures = FixtureSet::new();
    let rows = fixture_rows();
    let valid_payload = wkt_payload();

    for (label, second_payload) in [("invalid", vec![0xff, 0xfe]), ("empty", vec![0])] {
        let path = fixtures.directory.join(format!("ambiguous-{label}.las"));
        write_wkt_ambiguity_fixture(&path, &rows, second_payload.clone());

        let source = open_file(&path).blocking_wait().unwrap();
        assert_eq!(source.metadata().coordinate_reference().as_wkt(), None);
        let projection_records = source
            .metadata()
            .metadata_records()
            .iter()
            .filter(|record| record.name().starts_with("LASF_Projection:2112:"))
            .collect::<Vec<_>>();
        assert_eq!(projection_records.len(), 2);
        assert_eq!(projection_records[0].namespace(), "las.vlr");
        assert_eq!(projection_records[0].payload(), valid_payload);
        assert_eq!(projection_records[1].namespace(), "las.evlr");
        assert_eq!(projection_records[1].payload(), second_payload);
    }
}

#[test]
fn oversized_wkt_remains_unknown_while_its_raw_payload_is_preserved() {
    let fixtures = FixtureSet::new();
    let path = fixtures.directory.join("oversized-wkt.las");
    let payload = vec![b'A'; point_contracts::MAX_COORDINATE_REFERENCE_WKT_BYTES + 1];
    write_las_fixture_with_metadata(
        &path,
        &fixture_rows(),
        Vec::new(),
        vec![vlr(
            "LASF_Projection",
            2112,
            "oversized WKT",
            payload.clone(),
        )],
    );

    let source = open_file(&path).blocking_wait().unwrap();
    assert_eq!(source.metadata().coordinate_reference().as_wkt(), None);
    let record = source
        .metadata()
        .metadata_records()
        .first()
        .expect("the raw WKT record is retained");
    assert_eq!(record.payload(), payload);
}

#[test]
fn missing_truncated_corrupt_and_changed_files_have_stable_public_failures() {
    let fixtures = FixtureSet::new();
    let missing = fixtures.directory.join("missing.las");
    assert!(matches!(
        open_file(&missing).blocking_wait(),
        Err(SourceError::SourceMissing { .. })
    ));

    for (source_path, extension) in [
        (&fixtures.uncompressed, "las"),
        (&fixtures.compressed, "laz"),
    ] {
        let bytes = fs::read(source_path).unwrap();
        let truncated = fixtures.directory.join(format!("truncated.{extension}"));
        fs::write(&truncated, &bytes[..100]).unwrap();
        assert!(matches!(
            open_file(&truncated).blocking_wait(),
            Err(SourceError::CorruptSource { .. })
        ));
    }

    let corrupt = fixtures.directory.join("corrupt.las");
    fs::write(&corrupt, b"this is not a LAS header").unwrap();
    assert!(matches!(
        open_file(&corrupt).blocking_wait(),
        Err(SourceError::CorruptSource { .. })
    ));

    let oversized_layer = fixtures.directory.join("oversized-layer.laz");
    let mut compressed_bytes = fs::read(&fixtures.compressed).unwrap();
    overwrite_first_layer_size(&mut compressed_bytes, u32::MAX);
    fs::write(&oversized_layer, compressed_bytes).unwrap();
    assert!(matches!(
        open_file(&oversized_layer).blocking_wait(),
        Err(SourceError::CorruptSource { .. })
    ));

    let changed = fixtures.directory.join("changed.las");
    fs::copy(&fixtures.uncompressed, &changed).unwrap();
    let source = open_file(&changed).blocking_wait().unwrap();
    let record = source.record().clone();
    FsOpenOptions::new()
        .append(true)
        .open(&changed)
        .unwrap()
        .write_all(&[0xa5])
        .unwrap();

    assert!(matches!(
        read_error(&source, ReadRequest::all()),
        SourceError::SourceChanged { .. }
    ));
    assert!(matches!(
        open_file_with(
            &changed,
            OpenOptions::match_record(record, VerificationPolicy::Full),
        )
        .blocking_wait(),
        Err(SourceError::SourceChanged { .. })
    ));
}

#[test]
fn malformed_metadata_reports_its_section_and_byte_offset() {
    let fixtures = FixtureSet::new();
    let path = fixtures.directory.join("bad-vlr-length.las");
    let mut bytes = fs::read(&fixtures.uncompressed).unwrap();
    let header_start = usize::from(u16::from_le_bytes(bytes[94..96].try_into().unwrap()));
    bytes[header_start + 20..header_start + 22].copy_from_slice(&u16::MAX.to_le_bytes());
    fs::write(&path, bytes).unwrap();

    let Err(SourceError::CorruptSource { reason }) = open_file(&path).blocking_wait() else {
        panic!("the malformed VLR must be rejected as corrupt");
    };
    assert!(
        reason
            .as_str()
            .contains(&format!("VLR payload at byte {header_start}")),
        "diagnostic did not retain the known context: {reason}"
    );
}

#[test]
fn combined_vlr_and_evlr_count_is_bounded_through_public_open() {
    let directory = unique_temp_directory();
    fs::create_dir(&directory).unwrap();
    let path = directory.join("too-many-metadata-records.las");
    let regular_count = MAX_METADATA_RECORDS / 2;
    let extended_count = MAX_METADATA_RECORDS - regular_count + 1;
    write_las_fixture_with_metadata(
        &path,
        &fixture_rows(),
        empty_vlrs(regular_count),
        empty_vlrs(extended_count),
    );

    assert!(matches!(
        open_file(&path).blocking_wait(),
        Err(SourceError::CorruptSource { .. })
    ));
    fs::remove_dir_all(directory).unwrap();
}

fn overwrite_first_layer_size(bytes: &mut [u8], layer_size: u32) {
    let point_offset = usize::try_from(u32::from_le_bytes(bytes[96..100].try_into().unwrap()))
        .expect("LAS point offset fits usize");
    let record_len = usize::from(u16::from_le_bytes(bytes[105..107].try_into().unwrap()));
    let layer_size_offset = point_offset
        .checked_add(8)
        .and_then(|offset| offset.checked_add(record_len))
        .and_then(|offset| offset.checked_add(4))
        .expect("fixture layer-size offset fits usize");
    bytes[layer_size_offset..layer_size_offset + 4].copy_from_slice(&layer_size.to_le_bytes());
}

fn read_all(source: &Source, request: ReadRequest) -> ReadOutcome {
    let mut batches = source.read(request).unwrap();
    let mut rows = Vec::new();
    let mut batch_sizes = Vec::new();

    while let Some(batch) = batches.next().unwrap() {
        assert_eq!(batch.source(), source.identity());
        assert_eq!(batch.first_ordinal(), rows.len() as u64);
        assert_eq!(batch.point_count(), batch.len() as u64);
        for row in 0..batch.len() {
            let point_id = batch.point_id(row).unwrap();
            assert_eq!(point_id.source(), source.identity());
            assert_eq!(point_id.ordinal(), rows.len() as u64);
            rows.push(canonical_row(&batch, row));
        }
        batch_sizes.push(batch.len());
    }

    let summary = batches
        .summary()
        .expect("successful read publishes a summary");
    ReadOutcome {
        rows,
        batch_sizes,
        spans: summary.spans().to_vec(),
        attributes: summary.attributes().to_vec(),
        exact_count: summary.exact_count(),
        budget: summary.budget(),
    }
}

fn read_error(source: &Source, request: ReadRequest) -> SourceError {
    match source.read(request) {
        Err(error) => error,
        Ok(mut batches) => {
            let error = match batches.next() {
                Err(error) => error,
                Ok(Some(_)) => panic!("read unexpectedly emitted a batch before failing"),
                Ok(None) => panic!("read unexpectedly completed successfully"),
            };
            assert!(
                batches.next().unwrap().is_none(),
                "failed streams are fused"
            );
            assert!(batches.summary().is_none());
            error
        }
    }
}

fn canonical_row(batch: &PointBatch, row: usize) -> CanonicalRow {
    let attributes = batch.attributes();
    CanonicalRow {
        ticks: batch.positions().ticks()[row],
        intensity: u16_value(attributes, INTENSITY, row),
        return_number: u8_value(attributes, RETURN_NUMBER, row),
        number_of_returns: u8_value(attributes, NUMBER_OF_RETURNS, row),
        scan_direction: u8_value(attributes, SCAN_DIRECTION, row),
        edge_of_flight_line: u8_value(attributes, EDGE_OF_FLIGHT_LINE, row),
        classification: u8_value(attributes, CLASSIFICATION, row),
        synthetic: u8_value(attributes, SYNTHETIC, row),
        key_point: u8_value(attributes, KEY_POINT, row),
        withheld: u8_value(attributes, WITHHELD, row),
        overlap: u8_value(attributes, OVERLAP, row),
        scanner_channel: u8_value(attributes, SCANNER_CHANNEL, row),
        scan_angle: i16_value(attributes, SCAN_ANGLE, row),
        user_data: u8_value(attributes, USER_DATA, row),
        point_source_id: u16_value(attributes, POINT_SOURCE_ID, row),
        gps_time_bits: f64_value(attributes, GPS_TIME, row).to_bits(),
        red: u16_value(attributes, RED, row),
        green: u16_value(attributes, GREEN, row),
        blue: u16_value(attributes, BLUE, row),
        nir: u16_value(attributes, NIR, row),
        extra_bytes: fixed_bytes_value(attributes, EXTRA_BYTES, row),
    }
}

fn assert_projection(source: &Source, kind: SourceKind) {
    let selection = AttributeSelection::only([
        attribute_id(NIR),
        attribute_id(INTENSITY),
        attribute_id(NIR),
    ]);
    let mut batches = source
        .read(
            ReadRequest::all()
                .attributes(selection)
                .budget(read_budget(2, GENEROUS_BYTES)),
        )
        .unwrap();
    let expected = fixture_rows();
    let mut ordinal = 0;

    while let Some(batch) = batches.next().unwrap() {
        let actual_ids = batch
            .attributes()
            .columns()
            .iter()
            .map(AttributeColumn::id)
            .collect::<Vec<_>>();
        assert_eq!(actual_ids, vec![attribute_id(INTENSITY), attribute_id(NIR)]);
        for row in 0..batch.len() {
            assert_eq!(batch.positions().ticks()[row], expected[ordinal].ticks);
            assert_eq!(
                u16_value(batch.attributes(), INTENSITY, row),
                expected[ordinal].intensity
            );
            assert_eq!(
                u16_value(batch.attributes(), NIR, row),
                expected[ordinal].nir
            );
            ordinal += 1;
        }
    }
    let summary = batches.summary().unwrap();
    assert_eq!(ordinal, POINT_COUNT, "{kind:?}");
    assert_eq!(
        summary.attributes(),
        &[attribute_id(INTENSITY), attribute_id(NIR)]
    );

    let mut positions_only = source
        .read(
            ReadRequest::all()
                .attributes(AttributeSelection::only(Vec::<AttributeId>::new()))
                .budget(read_budget(2, 48)),
        )
        .unwrap();
    while let Some(batch) = positions_only.next().unwrap() {
        assert!(batch.attributes().is_empty());
        assert_eq!(batch.attributes().row_count(), batch.len());
        assert_eq!(batch.estimated_payload_bytes(), 24 * batch.point_count());
    }
    assert!(positions_only.summary().unwrap().attributes().is_empty());
}

fn assert_complete_summary(outcome: &ReadOutcome, budget: ReadBudget) {
    assert_eq!(outcome.exact_count, POINT_COUNT as u64);
    assert_eq!(outcome.spans, vec![span(0, POINT_COUNT as u64)]);
    assert_eq!(outcome.attributes, attribute_ids());
    assert_eq!(outcome.budget, budget);
}

fn assert_metadata_contract(source: &Source) {
    let metadata = source.metadata();
    assert_eq!(metadata.point_count(), POINT_COUNT as u64);
    assert_eq!(metadata.position_transform(), fixture_transform());
    assert_eq!(metadata.coordinate_reference().as_wkt(), Some(WKT));
    assert_eq!(
        metadata.world_bounds(),
        Some(fixture_bounds(&fixture_rows()))
    );
    assert_eq!(metadata.attributes(), &attribute_schema());
}

fn fixture_rows() -> Vec<CanonicalRow> {
    (0..POINT_COUNT)
        .map(|ordinal| {
            let small = u8::try_from(ordinal).unwrap();
            let wide = u16::from(small);
            let scan_angle = i16::from(small) * 321 - 1_000;
            CanonicalRow {
                ticks: ticks_for(ordinal),
                intensity: 1_000 + wide * 17,
                return_number: small % 3 + 1,
                number_of_returns: 3,
                scan_direction: small % 2,
                edge_of_flight_line: u8::from(ordinal % 3 == 0),
                classification: [1, 2, 3, 5, 6, 7, 9, 17, 18][ordinal],
                synthetic: u8::from(ordinal % 2 == 0),
                key_point: u8::from(ordinal % 3 == 1),
                withheld: u8::from(ordinal == POINT_COUNT - 1),
                overlap: u8::from(ordinal == 4),
                scanner_channel: small % 4,
                scan_angle,
                user_data: 20 + small,
                point_source_id: 300 + wide,
                gps_time_bits: (1_000_000.25 + f64::from(small) * 0.5).to_bits(),
                red: 10_000 + wide,
                green: 20_000 + wide * 2,
                blue: 30_000 + wide * 3,
                nir: 40_000 + wide * 5,
                extra_bytes: [small, 255 - small, small ^ 0x5a],
            }
        })
        .collect()
}

fn write_las_fixture(path: &Path, rows: &[CanonicalRow]) {
    write_las_fixture_with_metadata(
        path,
        rows,
        vec![
            vlr("vendor.first", 101, "first", FIRST_VLR.to_vec()),
            vlr("LASF_Projection", 2112, "fixture WKT", wkt_payload()),
            vlr("vendor.second", 202, "second", SECOND_VLR.to_vec()),
        ],
        vec![vlr("vendor.last", 303, "last", LAST_EVLR.to_vec())],
    );
}

fn write_wkt_ambiguity_fixture(path: &Path, rows: &[CanonicalRow], second_payload: Vec<u8>) {
    write_las_fixture_with_metadata(
        path,
        rows,
        vec![vlr("LASF_Projection", 2112, "valid WKT", wkt_payload())],
        vec![vlr(
            "LASF_Projection",
            2112,
            "ambiguous WKT",
            second_payload,
        )],
    );
}

fn write_las_fixture_with_metadata(
    path: &Path,
    rows: &[CanonicalRow],
    vlrs: Vec<Vlr>,
    evlrs: Vec<Vlr>,
) {
    let mut format = Format::new(8).unwrap();
    format.extra_bytes = u16::try_from(EXTRA_BYTES_WIDTH).unwrap();
    let mut builder = Builder::from((1, 4));
    builder.point_format = format;
    builder.has_wkt_crs = true;
    builder.transforms = las_transforms();
    builder.vlrs = vlrs;
    builder.evlrs = evlrs;
    let header = builder.into_header().unwrap();
    let mut writer = Writer::from_path(path, header).unwrap();
    for row in rows {
        writer.write_point(las_point(row)).unwrap();
    }
    writer.close().unwrap();
}

fn las_point(row: &CanonicalRow) -> Point {
    let world = fixture_transform().world_f64(row.ticks);
    let scan_angle = encoded_scan_angle(row.scan_angle);
    Point {
        x: world[0],
        y: world[1],
        z: world[2],
        intensity: row.intensity,
        return_number: row.return_number,
        number_of_returns: row.number_of_returns,
        scan_direction: if row.scan_direction == 0 {
            ScanDirection::RightToLeft
        } else {
            ScanDirection::LeftToRight
        },
        is_edge_of_flight_line: row.edge_of_flight_line != 0,
        classification: Classification::new(row.classification).unwrap(),
        is_synthetic: row.synthetic != 0,
        is_key_point: row.key_point != 0,
        is_withheld: row.withheld != 0,
        is_overlap: row.overlap != 0,
        scanner_channel: row.scanner_channel,
        scan_angle,
        user_data: row.user_data,
        point_source_id: row.point_source_id,
        gps_time: Some(f64::from_bits(row.gps_time_bits)),
        color: Some(Color::new(row.red, row.green, row.blue)),
        waveform: None,
        nir: Some(row.nir),
        extra_bytes: row.extra_bytes.to_vec(),
    }
}

fn encoded_scan_angle(raw: i16) -> f32 {
    let nudge = if raw < 0 { -0.25 } else { 0.25 };
    let angle = (f32::from(raw) + nudge) * 0.006;
    assert_eq!(i16::from(ScanAngle::from(angle)), raw);
    angle
}

fn memory_fixture(rows: &[CanonicalRow]) -> MemorySource {
    let ticks = rows.iter().map(|row| row.ticks).collect::<Vec<_>>();
    let attributes = memory_attributes(rows);
    let metadata = SourceMetadata::new(
        rows.len() as u64,
        fixture_transform(),
        CoordinateReference::wkt(WKT).unwrap(),
        attribute_schema(),
        Some(fixture_bounds(rows)),
        "memory LAS-equivalent",
        fixture_metadata_records(),
    )
    .unwrap();
    MemorySource::new(metadata, ticks, attributes).unwrap()
}

fn memory_attributes(rows: &[CanonicalRow]) -> AttributeColumns {
    let mut columns = core_attribute_columns(rows);
    columns.extend(color_and_misc_attribute_columns(rows));
    AttributeColumns::new(columns, rows.len()).unwrap()
}

fn core_attribute_columns(rows: &[CanonicalRow]) -> Vec<AttributeColumn> {
    vec![
        column(
            INTENSITY,
            "intensity",
            AttributeValues::u16(rows.iter().map(|row| row.intensity).collect()),
        ),
        column(
            RETURN_NUMBER,
            "return_number",
            AttributeValues::u8(rows.iter().map(|row| row.return_number).collect()),
        ),
        column(
            NUMBER_OF_RETURNS,
            "number_of_returns",
            AttributeValues::u8(rows.iter().map(|row| row.number_of_returns).collect()),
        ),
        column(
            SCAN_DIRECTION,
            "scan_direction",
            AttributeValues::u8(rows.iter().map(|row| row.scan_direction).collect()),
        ),
        column(
            EDGE_OF_FLIGHT_LINE,
            "edge_of_flight_line",
            AttributeValues::u8(rows.iter().map(|row| row.edge_of_flight_line).collect()),
        ),
        column(
            CLASSIFICATION,
            "classification",
            AttributeValues::u8(rows.iter().map(|row| row.classification).collect()),
        ),
        column(
            SYNTHETIC,
            "synthetic",
            AttributeValues::u8(rows.iter().map(|row| row.synthetic).collect()),
        ),
        column(
            KEY_POINT,
            "key_point",
            AttributeValues::u8(rows.iter().map(|row| row.key_point).collect()),
        ),
        column(
            WITHHELD,
            "withheld",
            AttributeValues::u8(rows.iter().map(|row| row.withheld).collect()),
        ),
        column(
            OVERLAP,
            "overlap",
            AttributeValues::u8(rows.iter().map(|row| row.overlap).collect()),
        ),
        column(
            SCANNER_CHANNEL,
            "scanner_channel",
            AttributeValues::u8(rows.iter().map(|row| row.scanner_channel).collect()),
        ),
        column(
            SCAN_ANGLE,
            "scan_angle",
            AttributeValues::i16(rows.iter().map(|row| row.scan_angle).collect()),
        ),
        column(
            USER_DATA,
            "user_data",
            AttributeValues::u8(rows.iter().map(|row| row.user_data).collect()),
        ),
        column(
            POINT_SOURCE_ID,
            "point_source_id",
            AttributeValues::u16(rows.iter().map(|row| row.point_source_id).collect()),
        ),
    ]
}

fn color_and_misc_attribute_columns(rows: &[CanonicalRow]) -> Vec<AttributeColumn> {
    vec![
        column(
            GPS_TIME,
            "gps_time",
            AttributeValues::f64(
                rows.iter()
                    .map(|row| f64::from_bits(row.gps_time_bits))
                    .collect(),
            ),
        ),
        column(
            RED,
            "red",
            AttributeValues::u16(rows.iter().map(|row| row.red).collect()),
        ),
        column(
            GREEN,
            "green",
            AttributeValues::u16(rows.iter().map(|row| row.green).collect()),
        ),
        column(
            BLUE,
            "blue",
            AttributeValues::u16(rows.iter().map(|row| row.blue).collect()),
        ),
        column(
            NIR,
            "nir",
            AttributeValues::u16(rows.iter().map(|row| row.nir).collect()),
        ),
        column(
            EXTRA_BYTES,
            "extra_bytes",
            AttributeValues::fixed_bytes(
                extra_bytes_width(),
                rows.iter().flat_map(|row| row.extra_bytes).collect(),
            )
            .unwrap(),
        ),
    ]
}

fn attribute_schema() -> AttributeSchema {
    AttributeSchema::new(attribute_definitions()).unwrap()
}

fn attribute_definitions() -> Vec<AttributeDefinition> {
    vec![
        definition(INTENSITY, "intensity", AttributeDataType::U16),
        definition(RETURN_NUMBER, "return_number", AttributeDataType::U8),
        definition(
            NUMBER_OF_RETURNS,
            "number_of_returns",
            AttributeDataType::U8,
        ),
        definition(SCAN_DIRECTION, "scan_direction", AttributeDataType::U8),
        definition(
            EDGE_OF_FLIGHT_LINE,
            "edge_of_flight_line",
            AttributeDataType::U8,
        ),
        definition(CLASSIFICATION, "classification", AttributeDataType::U8),
        definition(SYNTHETIC, "synthetic", AttributeDataType::U8),
        definition(KEY_POINT, "key_point", AttributeDataType::U8),
        definition(WITHHELD, "withheld", AttributeDataType::U8),
        definition(OVERLAP, "overlap", AttributeDataType::U8),
        definition(SCANNER_CHANNEL, "scanner_channel", AttributeDataType::U8),
        definition(SCAN_ANGLE, "scan_angle", AttributeDataType::I16),
        definition(USER_DATA, "user_data", AttributeDataType::U8),
        definition(POINT_SOURCE_ID, "point_source_id", AttributeDataType::U16),
        definition(GPS_TIME, "gps_time", AttributeDataType::F64),
        definition(RED, "red", AttributeDataType::U16),
        definition(GREEN, "green", AttributeDataType::U16),
        definition(BLUE, "blue", AttributeDataType::U16),
        definition(NIR, "nir", AttributeDataType::U16),
        definition(
            EXTRA_BYTES,
            "extra_bytes",
            AttributeDataType::fixed_bytes(extra_bytes_width()).unwrap(),
        ),
    ]
}

fn definition(id: u32, name: &str, data_type: AttributeDataType) -> AttributeDefinition {
    AttributeDefinition::new(attribute_id(id), name, data_type).unwrap()
}

fn column(id: u32, name: &str, values: AttributeValues) -> AttributeColumn {
    let definition = definition(id, name, values.data_type());
    AttributeColumn::new(definition, values).unwrap()
}

fn attribute_ids() -> Vec<AttributeId> {
    attribute_definitions()
        .iter()
        .map(AttributeDefinition::id)
        .collect()
}

fn canonical_bytes_per_point() -> u64 {
    24 + attribute_definitions()
        .iter()
        .map(|definition| u64::from(definition.data_type().element_bytes()))
        .sum::<u64>()
}

fn fixture_metadata_records() -> Vec<MetadataRecord> {
    vec![
        MetadataRecord::new("las.vlr", "vendor.first:101:first", FIRST_VLR.to_vec()).unwrap(),
        MetadataRecord::new("las.vlr", "LASF_Projection:2112:fixture WKT", wkt_payload()).unwrap(),
        MetadataRecord::new("las.vlr", "vendor.second:202:second", SECOND_VLR.to_vec()).unwrap(),
        MetadataRecord::new("las.evlr", "vendor.last:303:last", LAST_EVLR.to_vec()).unwrap(),
    ]
}

fn fixture_transform() -> PositionTransform {
    PositionTransform::new([100.0, -25.0, 2.0], [0.25, 0.5, 2.0]).unwrap()
}

fn las_transforms() -> Vector<Transform> {
    let transform = fixture_transform();
    let offset = transform.offset();
    let scale = transform.scale();
    Vector {
        x: Transform {
            scale: scale[0],
            offset: offset[0],
        },
        y: Transform {
            scale: scale[1],
            offset: offset[1],
        },
        z: Transform {
            scale: scale[2],
            offset: offset[2],
        },
    }
}

fn fixture_bounds(rows: &[CanonicalRow]) -> WorldBounds {
    let transform = fixture_transform();
    let mut min = transform.world_f64(rows[0].ticks);
    let mut max = min;
    for row in &rows[1..] {
        let world = transform.world_f64(row.ticks);
        for axis in 0..3 {
            min[axis] = min[axis].min(world[axis]);
            max[axis] = max[axis].max(world[axis]);
        }
    }
    WorldBounds::new(min, max).unwrap()
}

fn ticks_for(ordinal: usize) -> [i64; 3] {
    let ordinal = i64::try_from(ordinal).unwrap();
    [ordinal * 3 - 10, 7 - ordinal * 2, ordinal * ordinal - 5]
}

fn vlr(user_id: &str, record_id: u16, description: &str, data: Vec<u8>) -> Vlr {
    Vlr {
        user_id: user_id.to_owned(),
        record_id,
        description: description.to_owned(),
        data,
    }
}

fn empty_vlrs(count: usize) -> Vec<Vlr> {
    std::iter::repeat_with(|| vlr("limit", 1, "limit", Vec::new()))
        .take(count)
        .collect()
}

fn wkt_payload() -> Vec<u8> {
    let mut payload = WKT.as_bytes().to_vec();
    payload.push(0);
    payload
}

fn unique_temp_directory() -> PathBuf {
    let counter = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "punctra-source-las-conformance-{}-{timestamp}-{counter}",
        std::process::id()
    ))
}

fn record_index(records: &[MetadataRecord], payload: &[u8]) -> usize {
    records
        .iter()
        .position(|record| record.payload() == payload)
        .unwrap_or_else(|| panic!("metadata payload not found: {payload:?}"))
}

fn read_budget(points: u64, payload_bytes: u64) -> ReadBudget {
    ReadBudget::new(points, payload_bytes)
        .unwrap()
        .with_max_adapter_working_bytes(GENEROUS_BYTES)
}

fn span(first_ordinal: u64, point_count: u64) -> SourceSpan {
    SourceSpan::new(first_ordinal, point_count).unwrap()
}

fn identity_for(
    identities: &[(SourceKind, point_contracts::SourceId)],
    kind: SourceKind,
) -> point_contracts::SourceId {
    identities
        .iter()
        .find_map(|&(actual, identity)| (actual == kind).then_some(identity))
        .unwrap()
}

fn attribute_id(value: u32) -> AttributeId {
    AttributeId::new(value).unwrap()
}

fn values(attributes: &AttributeColumns, id: u32) -> &AttributeValues {
    attributes.get(attribute_id(id)).unwrap().values()
}

fn u8_value(attributes: &AttributeColumns, id: u32, row: usize) -> u8 {
    values(attributes, id).as_u8().unwrap()[row]
}

fn u16_value(attributes: &AttributeColumns, id: u32, row: usize) -> u16 {
    values(attributes, id).as_u16().unwrap()[row]
}

fn i16_value(attributes: &AttributeColumns, id: u32, row: usize) -> i16 {
    values(attributes, id).as_i16().unwrap()[row]
}

fn f64_value(attributes: &AttributeColumns, id: u32, row: usize) -> f64 {
    values(attributes, id).as_f64().unwrap()[row]
}

fn fixed_bytes_value(
    attributes: &AttributeColumns,
    id: u32,
    row: usize,
) -> [u8; EXTRA_BYTES_WIDTH] {
    let (width, payload) = values(attributes, id).as_fixed_bytes().unwrap();
    assert_eq!(width, extra_bytes_width());
    let start = row * EXTRA_BYTES_WIDTH;
    payload[start..start + EXTRA_BYTES_WIDTH]
        .try_into()
        .unwrap()
}

fn extra_bytes_width() -> u32 {
    u32::try_from(EXTRA_BYTES_WIDTH).unwrap()
}
