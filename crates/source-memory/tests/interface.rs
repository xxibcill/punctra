//! Public-interface conformance tests for the in-memory Source adapter.

use foundation_runtime::ProgressPhase;
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeSchema, AttributeValues, CoordinateReference, MetadataRecord, PointId,
    PositionTransform, SourceMetadata, WorldBounds,
};
use point_source::{
    AttributeSelection, OpenOptions, ReadBudget, ReadRequest, Source, SourceError, SourceSpan,
    VerificationPolicy,
};
use source_memory::{MemoryFaultControl, MemorySource, open, open_with};

const POINT_COUNT: usize = 12;
const BYTES_PER_POINT: u64 = 24 + 2 + 1;

#[derive(Debug, Eq, PartialEq)]
struct CanonicalRow {
    id: PointId,
    ticks: [i64; 3],
    intensity: u16,
    classification: u8,
}

#[test]
fn equivalent_partitioned_and_overlapping_reads_preserve_identity_and_values() {
    let input = fixture_input();
    let source = open(input.clone()).blocking_wait().unwrap();
    let reopened = open(input).blocking_wait().unwrap();
    assert_eq!(reopened.identity(), source.identity());

    let narrow_batches = ReadRequest::all()
        .spans([SourceSpan::new(0, u64::try_from(POINT_COUNT).unwrap()).unwrap()])
        .budget(ReadBudget::new(2, 1_000_000).unwrap());
    let overlapping_spans = ReadRequest::all()
        .spans([
            SourceSpan::new(6, 6).unwrap(),
            SourceSpan::new(0, 4).unwrap(),
            SourceSpan::new(3, 4).unwrap(),
        ])
        .budget(ReadBudget::new(5, 1_000_000).unwrap());

    let expected = canonical_rows(&source, narrow_batches);
    let actual = canonical_rows(&reopened, overlapping_spans);
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), POINT_COUNT);

    for (ordinal, row) in actual.iter().enumerate() {
        assert_eq!(row.id.source(), source.identity());
        assert_eq!(row.id.ordinal(), u64::try_from(ordinal).unwrap());
        assert_eq!(row.ticks, ticks_for(ordinal));
        assert_eq!(row.intensity, intensity_for(ordinal));
        assert_eq!(row.classification, classification_for(ordinal));
    }
}

#[test]
fn attribute_projection_returns_only_exact_requested_columns() {
    let source = open(fixture_input()).blocking_wait().unwrap();
    let selection = AttributeSelection::only([classification_id(), classification_id()]);
    let mut batches = source
        .read(ReadRequest::all().attributes(selection.clone()))
        .unwrap();
    let mut classifications = Vec::new();

    while let Some(batch) = batches.next().unwrap() {
        assert_eq!(batch.attributes().columns().len(), 1);
        let column = &batch.attributes().columns()[0];
        assert_eq!(column.id(), classification_id());
        classifications.extend_from_slice(column.values().as_u8().unwrap());
    }

    let expected = (0..POINT_COUNT).map(classification_for).collect::<Vec<_>>();
    assert_eq!(classifications, expected);
    assert_eq!(
        batches.summary().unwrap().attributes(),
        &[classification_id()]
    );

    let no_attributes = AttributeSelection::only(Vec::<AttributeId>::new());
    let mut positions_only = source
        .read(ReadRequest::all().attributes(no_attributes.clone()))
        .unwrap();
    while let Some(batch) = positions_only.next().unwrap() {
        assert!(batch.attributes().is_empty());
        assert_eq!(batch.attributes().row_count(), batch.len());
        assert_eq!(batch.estimated_payload_bytes(), 24 * batch.point_count());
    }
    assert!(positions_only.summary().unwrap().attributes().is_empty());
}

#[test]
fn every_batch_obeys_point_and_payload_byte_budgets() {
    let source = open(fixture_input()).blocking_wait().unwrap();

    let point_limited = ReadBudget::new(3, 1_000_000).unwrap();
    assert_eq!(batch_sizes(&source, point_limited), vec![3, 3, 3, 3]);

    let byte_limited = ReadBudget::new(99, BYTES_PER_POINT * 2).unwrap();
    assert_eq!(batch_sizes(&source, byte_limited), vec![2; 6]);

    let too_small = ReadBudget::new(99, BYTES_PER_POINT - 1).unwrap();
    let mut failed = source.read(ReadRequest::all().budget(too_small)).unwrap();
    assert!(matches!(
        failed.next(),
        Err(SourceError::ResourceLimit {
            limit: "batch payload bytes",
            required: BYTES_PER_POINT,
            allowed,
        }) if allowed == BYTES_PER_POINT - 1
    ));
    assert!(failed.next().unwrap().is_none());
    assert!(failed.summary().is_none());
}

#[test]
fn fast_reopen_matches_the_recorded_source() {
    let input = fixture_input();
    let source = open(input.clone()).blocking_wait().unwrap();

    let reopened = open_with(
        input,
        OpenOptions::match_record(source.record().clone(), VerificationPolicy::FastOnly),
    )
    .blocking_wait()
    .unwrap();

    assert_eq!(reopened.identity(), source.identity());
    assert_eq!(reopened.metadata(), source.metadata());
    assert_eq!(reopened.provenance(), source.provenance());
}

#[test]
fn metadata_records_survive_open_and_record_publication_exactly() {
    let (transform, ticks, attributes) = fixture_columns();
    let expected = vec![
        MetadataRecord::new("las", "vlr-34735", vec![0, 255, 17, 34]).unwrap(),
        MetadataRecord::new("vendor.example", "opaque", vec![128, 0, 128]).unwrap(),
    ];
    let metadata = fixture_metadata_with_records(transform, &ticks, &attributes, expected.clone());
    let input = MemorySource::new(metadata, ticks, attributes).unwrap();

    let source = open(input).blocking_wait().unwrap();

    assert_eq!(source.metadata().metadata_records(), expected);
    assert_eq!(source.record().metadata().metadata_records(), expected);
    assert_eq!(source.record().metadata(), source.metadata());
}

#[test]
fn every_attribute_representation_round_trips_without_coercion() {
    let ticks = vec![[0, 0, 0], [1, 1, 1], [2, 2, 2]];
    let f32_bits = [0x8000_0000, 0x7fc0_0042, 0x3f80_0000];
    let f64_bits = [
        0x8000_0000_0000_0000,
        0x7ff8_0000_0000_0042,
        0x3ff0_0000_0000_0000,
    ];
    let fixed_bytes = vec![0, 1, 2, 3, 254, 255];
    let attributes = every_attribute_columns(f32_bits, f64_bits, &fixed_bytes);
    let input = MemorySource::from_columns(
        fixture_transform(),
        CoordinateReference::Unknown,
        ticks,
        attributes,
    )
    .unwrap();
    let source = open(input).blocking_wait().unwrap();
    let mut batches = source.points().unwrap();
    let batch = batches.next().unwrap().unwrap();

    assert_integer_attribute_values(batch.attributes());
    assert_float_and_fixed_attribute_values(batch.attributes(), f32_bits, f64_bits, &fixed_bytes);
    assert!(batches.next().unwrap().is_none());
    assert_eq!(batches.summary().unwrap().exact_count(), 3);
}

#[test]
fn successful_open_and_read_publish_terminal_progress() {
    let job = open(fixture_input());
    let open_handle = job.handle();
    let source = job.blocking_wait().unwrap();
    let open_progress = open_handle.progress();
    assert_eq!(open_progress.phase(), ProgressPhase::COMPLETE);
    assert_eq!(
        open_progress.completed_units(),
        u64::try_from(POINT_COUNT).unwrap()
    );
    assert_eq!(
        open_progress.total_units(),
        Some(u64::try_from(POINT_COUNT).unwrap())
    );

    let mut batches = source.points().unwrap();
    let read_handle = batches.handle();
    while batches.next().unwrap().is_some() {}
    let read_progress = read_handle.progress();
    assert_eq!(read_progress.phase(), ProgressPhase::COMPLETE);
    assert_eq!(
        read_progress.completed_units(),
        u64::try_from(POINT_COUNT).unwrap()
    );
    assert_eq!(
        read_progress.total_units(),
        Some(u64::try_from(POINT_COUNT).unwrap())
    );
}

#[test]
fn injected_change_invalidates_fast_reopen_and_fuses_an_active_read() {
    let (input, faults) = controlled_fixture();
    let source = open(input.clone()).blocking_wait().unwrap();
    let record = source.record().clone();
    let mut batches = source
        .read(ReadRequest::all().budget(ReadBudget::new(2, 1_000_000).unwrap()))
        .unwrap();
    assert!(batches.next().unwrap().is_some());

    faults.mark_changed();
    assert!(matches!(
        batches.next(),
        Err(SourceError::SourceChanged { .. })
    ));
    assert!(batches.next().unwrap().is_none());
    assert!(batches.summary().is_none());

    let reopened = open_with(
        input,
        OpenOptions::match_record(record, VerificationPolicy::FastOnly),
    )
    .blocking_wait();
    assert!(matches!(reopened, Err(SourceError::VerificationRequired)));
}

#[test]
fn full_reopen_reports_source_changed_after_an_injected_change() {
    let (input, faults) = controlled_fixture();
    let source = open(input.clone()).blocking_wait().unwrap();
    let record = source.record().clone();

    faults.mark_changed();
    let reopened = open_with(
        input,
        OpenOptions::match_record(record, VerificationPolicy::Full),
    )
    .blocking_wait();

    assert!(matches!(reopened, Err(SourceError::SourceChanged { .. })));
}

#[test]
fn injected_corruption_is_contextual_and_fuses_without_a_summary() {
    let (input, faults) = controlled_fixture();
    let source = open(input).blocking_wait().unwrap();
    faults.fail_at_ordinal(3);
    let mut batches = source
        .read(ReadRequest::all().budget(ReadBudget::new(2, 1_000_000).unwrap()))
        .unwrap();

    let mut ordinals = Vec::new();
    for _ in 0..2 {
        let batch = batches.next().unwrap().unwrap();
        ordinals.extend(batch.point_ids().map(PointId::ordinal));
    }
    assert_eq!(ordinals, vec![0, 1, 2]);

    assert!(matches!(
        batches.next(),
        Err(SourceError::CorruptSource { reason }) if reason.contains("ordinal 3")
    ));
    assert!(batches.next().unwrap().is_none());
    assert!(batches.summary().is_none());
}

#[test]
fn cancellation_is_terminal_and_has_no_summary() {
    let source = open(fixture_input()).blocking_wait().unwrap();
    let mut batches = source.points().unwrap();
    let handle = batches.handle();
    handle.cancel();

    assert!(matches!(batches.next(), Err(SourceError::Cancelled)));
    assert!(batches.next().unwrap().is_none());
    assert!(batches.summary().is_none());
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn empty_source_opens_and_completes_with_an_exact_zero_summary() {
    let transform = fixture_transform();
    let input = MemorySource::from_columns(
        transform,
        CoordinateReference::Unknown,
        Vec::new(),
        AttributeColumns::empty(0),
    )
    .unwrap();
    let source = open(input).blocking_wait().unwrap();

    assert_eq!(source.metadata().point_count(), 0);
    assert!(source.metadata().world_bounds().is_none());
    assert!(source.metadata().attributes().is_empty());

    let mut batches = source.points().unwrap();
    let handle = batches.handle();
    assert!(batches.next().unwrap().is_none());
    assert!(batches.next().unwrap().is_none());
    let summary = batches.summary().unwrap();
    assert_eq!(summary.source(), source.identity());
    assert_eq!(summary.exact_count(), 0);
    assert_eq!(handle.progress().phase(), ProgressPhase::COMPLETE);
    assert_eq!(handle.progress().total_units(), Some(0));
}

fn canonical_rows(source: &Source, request: ReadRequest) -> Vec<CanonicalRow> {
    let mut batches = source.read(request).unwrap();
    let mut rows = Vec::new();

    while let Some(batch) = batches.next().unwrap() {
        let intensities = batch
            .attributes()
            .get(intensity_id())
            .unwrap()
            .values()
            .as_u16()
            .unwrap();
        let classifications = batch
            .attributes()
            .get(classification_id())
            .unwrap()
            .values()
            .as_u8()
            .unwrap();
        for row in 0..batch.len() {
            rows.push(CanonicalRow {
                id: batch.point_id(row).unwrap(),
                ticks: batch.positions().ticks()[row],
                intensity: intensities[row],
                classification: classifications[row],
            });
        }
    }

    assert_eq!(
        batches.summary().unwrap().exact_count(),
        u64::try_from(rows.len()).unwrap()
    );
    rows
}

fn batch_sizes(source: &Source, budget: ReadBudget) -> Vec<usize> {
    let mut batches = source.read(ReadRequest::all().budget(budget)).unwrap();
    let mut sizes = Vec::new();

    while let Some(batch) = batches.next().unwrap() {
        assert!(batch.point_count() <= budget.max_batch_points());
        assert!(batch.estimated_payload_bytes() <= budget.max_batch_payload_bytes());
        sizes.push(batch.len());
    }

    assert_eq!(
        batches.summary().unwrap().exact_count(),
        u64::try_from(POINT_COUNT).unwrap()
    );
    sizes
}

fn fixture_input() -> MemorySource {
    let (transform, ticks, attributes) = fixture_columns();
    MemorySource::from_columns(transform, CoordinateReference::Unknown, ticks, attributes).unwrap()
}

fn controlled_fixture() -> (MemorySource, MemoryFaultControl) {
    let (transform, ticks, attributes) = fixture_columns();
    let metadata = fixture_metadata(transform, &ticks, &attributes);
    MemorySource::with_fault_control(metadata, ticks, attributes).unwrap()
}

fn fixture_columns() -> (PositionTransform, Vec<[i64; 3]>, AttributeColumns) {
    let ticks = (0..POINT_COUNT).map(ticks_for).collect::<Vec<_>>();
    let intensity = AttributeColumn::new(
        AttributeDefinition::new(intensity_id(), "intensity", AttributeDataType::U16).unwrap(),
        AttributeValues::u16((0..POINT_COUNT).map(intensity_for).collect()),
    )
    .unwrap();
    let classification = AttributeColumn::new(
        AttributeDefinition::new(classification_id(), "classification", AttributeDataType::U8)
            .unwrap(),
        AttributeValues::u8((0..POINT_COUNT).map(classification_for).collect()),
    )
    .unwrap();
    let attributes = AttributeColumns::new(vec![classification, intensity], POINT_COUNT).unwrap();
    (fixture_transform(), ticks, attributes)
}

fn fixture_metadata(
    transform: PositionTransform,
    ticks: &[[i64; 3]],
    attributes: &AttributeColumns,
) -> SourceMetadata {
    fixture_metadata_with_records(transform, ticks, attributes, Vec::new())
}

fn fixture_metadata_with_records(
    transform: PositionTransform,
    ticks: &[[i64; 3]],
    attributes: &AttributeColumns,
    metadata_records: Vec<MetadataRecord>,
) -> SourceMetadata {
    let schema = AttributeSchema::new(
        attributes
            .columns()
            .iter()
            .map(|column| column.definition().clone())
            .collect(),
    )
    .unwrap();
    let world_positions = ticks
        .iter()
        .copied()
        .map(|ticks| transform.world_f64(ticks))
        .collect::<Vec<_>>();
    let mut min = world_positions[0];
    let mut max = world_positions[0];
    for position in &world_positions[1..] {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    SourceMetadata::new(
        u64::try_from(POINT_COUNT).unwrap(),
        transform,
        CoordinateReference::Unknown,
        schema,
        Some(WorldBounds::new(min, max).unwrap()),
        "memory",
        metadata_records,
    )
    .unwrap()
}

fn fixture_transform() -> PositionTransform {
    PositionTransform::new([100.0, -25.0, 2.0], [0.25, 0.5, 2.0]).unwrap()
}

fn ticks_for(ordinal: usize) -> [i64; 3] {
    let ordinal = i64::try_from(ordinal).unwrap();
    [ordinal * 3 - 10, 7 - ordinal * 2, ordinal * ordinal]
}

fn intensity_for(ordinal: usize) -> u16 {
    1_000 + u16::try_from(ordinal).unwrap() * 17
}

fn classification_for(ordinal: usize) -> u8 {
    u8::try_from(ordinal % 5).unwrap()
}

fn intensity_id() -> AttributeId {
    AttributeId::new(1).unwrap()
}

fn classification_id() -> AttributeId {
    AttributeId::new(2).unwrap()
}

fn attribute_id(value: u32) -> AttributeId {
    AttributeId::new(value).unwrap()
}

fn fixture_attribute(id: u32, name: &str, values: AttributeValues) -> AttributeColumn {
    let definition = AttributeDefinition::new(attribute_id(id), name, values.data_type()).unwrap();
    AttributeColumn::new(definition, values).unwrap()
}

fn every_attribute_columns(
    f32_bits: [u32; 3],
    f64_bits: [u64; 3],
    fixed_bytes: &[u8],
) -> AttributeColumns {
    let columns = vec![
        fixture_attribute(1, "i8", AttributeValues::i8(vec![i8::MIN, -1, i8::MAX])),
        fixture_attribute(2, "u8", AttributeValues::u8(vec![0, 1, u8::MAX])),
        fixture_attribute(3, "i16", AttributeValues::i16(vec![i16::MIN, -1, i16::MAX])),
        fixture_attribute(4, "u16", AttributeValues::u16(vec![0, 1, u16::MAX])),
        fixture_attribute(5, "i32", AttributeValues::i32(vec![i32::MIN, -1, i32::MAX])),
        fixture_attribute(6, "u32", AttributeValues::u32(vec![0, 1, u32::MAX])),
        fixture_attribute(7, "i64", AttributeValues::i64(vec![i64::MIN, -1, i64::MAX])),
        fixture_attribute(8, "u64", AttributeValues::u64(vec![0, 1, u64::MAX])),
        fixture_attribute(
            9,
            "f32",
            AttributeValues::f32(f32_bits.map(f32::from_bits).to_vec()),
        ),
        fixture_attribute(
            10,
            "f64",
            AttributeValues::f64(f64_bits.map(f64::from_bits).to_vec()),
        ),
        fixture_attribute(
            11,
            "fixed-bytes",
            AttributeValues::fixed_bytes(2, fixed_bytes.to_vec()).unwrap(),
        ),
    ];
    AttributeColumns::new(columns, 3).unwrap()
}

fn assert_integer_attribute_values(attributes: &AttributeColumns) {
    assert_eq!(
        attribute_values(attributes, 1).as_i8(),
        Some(&[i8::MIN, -1, i8::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 2).as_u8(),
        Some(&[0, 1, u8::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 3).as_i16(),
        Some(&[i16::MIN, -1, i16::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 4).as_u16(),
        Some(&[0, 1, u16::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 5).as_i32(),
        Some(&[i32::MIN, -1, i32::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 6).as_u32(),
        Some(&[0, 1, u32::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 7).as_i64(),
        Some(&[i64::MIN, -1, i64::MAX][..])
    );
    assert_eq!(
        attribute_values(attributes, 8).as_u64(),
        Some(&[0, 1, u64::MAX][..])
    );
}

fn assert_float_and_fixed_attribute_values(
    attributes: &AttributeColumns,
    f32_bits: [u32; 3],
    f64_bits: [u64; 3],
    fixed_bytes: &[u8],
) {
    let actual_f32_bits = attribute_values(attributes, 9)
        .as_f32()
        .unwrap()
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();
    let actual_f64_bits = attribute_values(attributes, 10)
        .as_f64()
        .unwrap()
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();

    assert_eq!(actual_f32_bits, f32_bits);
    assert_eq!(actual_f64_bits, f64_bits);
    assert_eq!(
        attribute_values(attributes, 11).as_fixed_bytes(),
        Some((2, fixed_bytes))
    );
}

fn attribute_values(attributes: &AttributeColumns, id: u32) -> &AttributeValues {
    attributes.get(attribute_id(id)).unwrap().values()
}
