//! Frozen `SourceRecord` schema-1 compatibility through the memory adapter.

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_source::{OpenOptions, SourceError, SourceRecord, VerificationPolicy};
use source_memory::{MemorySource, open_with};

const SOURCE_BYTES: &[u8] = include_bytes!("fixtures/v1/memory-source.json");
const RECORD_BYTES: &[u8] =
    include_bytes!("../../point-source/tests/fixtures/v1/source-memory-record.json");

#[test]
fn frozen_v1_record_reopens_full_and_fast_without_mutating_fixture_bytes() {
    let before = fixture_hash();
    let record: SourceRecord = serde_json::from_slice(RECORD_BYTES).expect("valid v1 SourceRecord");
    assert_record_facts(&record);

    let input = fixture_input();
    for policy in [VerificationPolicy::Full, VerificationPolicy::FastOnly] {
        let source = open_with(
            input.clone(),
            OpenOptions::match_record(record.clone(), policy),
        )
        .blocking_wait()
        .expect("frozen record reopens the fixed memory Source");
        assert_eq!(source.identity().to_string(), EXPECTED_SOURCE_ID);
        assert_eq!(source.metadata().point_count(), 3);
        let mut batches = source.points().expect("open canonical rows");
        let batch = batches.next().expect("read rows").expect("one batch");
        let classifications = batch
            .attributes()
            .get(AttributeId::new(6).unwrap())
            .expect("classification column")
            .values()
            .as_u8()
            .expect("u8 classifications");
        assert_eq!(classifications, [2, 7, 2]);
        assert!(batches.next().expect("finish rows").is_none());
        assert_eq!(
            batches.summary().expect("complete summary").exact_count(),
            3
        );
        let mut reproduced =
            serde_json::to_vec_pretty(source.record()).expect("serialize reopened SourceRecord");
        reproduced.push(b'\n');
        assert_eq!(
            reproduced, RECORD_BYTES,
            "the memory adapter must reproduce the frozen record bytes"
        );
    }
    assert_eq!(
        fixture_hash(),
        before,
        "fixture bytes changed during reopen"
    );
}

#[test]
fn future_version_and_corrupt_content_evidence_fail_closed_without_mutation() {
    let before = fixture_hash();
    let mut future: serde_json::Value =
        serde_json::from_slice(RECORD_BYTES).expect("valid fixture JSON");
    future["version"] = serde_json::json!(2);
    let future: SourceRecord = serde_json::from_value(future).expect("bounded future record");
    let error = open_with(
        fixture_input(),
        OpenOptions::match_record(future, VerificationPolicy::Full),
    )
    .blocking_wait()
    .expect_err("future SourceRecord version is rejected");
    assert!(matches!(
        error,
        SourceError::UnsupportedRecordVersion { version: 2 }
    ));

    let mut corrupt: serde_json::Value =
        serde_json::from_slice(RECORD_BYTES).expect("valid fixture JSON");
    corrupt["content_hash"][0] = serde_json::json!(255);
    let corrupt: SourceRecord = serde_json::from_value(corrupt).expect("bounded corrupt record");
    let error = open_with(
        fixture_input(),
        OpenOptions::match_record(corrupt, VerificationPolicy::Full),
    )
    .blocking_wait()
    .expect_err("different content evidence is rejected");
    assert!(matches!(error, SourceError::SourceChanged { .. }));
    assert_eq!(fixture_hash(), before, "rejection mutated fixture bytes");
}

const EXPECTED_SOURCE_ID: &str = "4fb837767b5fd89001059cf40291e17e52136d5e77466bb63804288111868487";
const EXPECTED_CONTENT_HASH: &str =
    "17a8438d6b9124fd6cdee150c13baaecc8a8e937a65f7328650bb30b61e8c90e";
const EXPECTED_SOURCE_BYTES_HASH: &str =
    "0f8ef7c5c3edcbfb08f8309bf5e21ec8389f0e6c96f83381fa4e14ffd4bf517f";
const EXPECTED_RECORD_BYTES_HASH: &str =
    "b2049e1e5b9d93e812757e9c7d477a5e9eae0d741869364691072d0fd89e7216";

fn assert_record_facts(record: &SourceRecord) {
    assert_eq!(record.version(), 1);
    assert_eq!(record.source().to_string(), EXPECTED_SOURCE_ID);
    assert_eq!(record.content_hash().to_string(), EXPECTED_CONTENT_HASH);
    assert_eq!(record.adapter_name(), "source-memory");
    assert_eq!(record.adapter_version(), "1");
    assert_eq!(record.logical_order(), "immutable input row order v1");
    assert_eq!(record.fast_token(), 0_u64.to_le_bytes());
    assert_eq!(record.metadata().point_count(), 3);
    assert_eq!(record.metadata().attributes().definitions().len(), 1);
}

fn fixture_hash() -> (blake3::Hash, blake3::Hash) {
    let source = blake3::hash(SOURCE_BYTES);
    let record = blake3::hash(RECORD_BYTES);
    assert_eq!(source.to_hex().as_str(), EXPECTED_SOURCE_BYTES_HASH);
    assert_eq!(record.to_hex().as_str(), EXPECTED_RECORD_BYTES_HASH);
    (source, record)
}

fn fixture_input() -> MemorySource {
    let input: serde_json::Value =
        serde_json::from_slice(SOURCE_BYTES).expect("valid frozen Source bytes");
    let ticks = input["ticks"]
        .as_array()
        .expect("ticks array")
        .iter()
        .map(|row| {
            let values = row.as_array().expect("tick row");
            [
                values[0].as_i64().expect("x tick"),
                values[1].as_i64().expect("y tick"),
                values[2].as_i64().expect("z tick"),
            ]
        })
        .collect::<Vec<_>>();
    let classifications = input["classifications"]
        .as_array()
        .expect("classification array")
        .iter()
        .map(|value| u8::try_from(value.as_u64().expect("classification")).unwrap())
        .collect::<Vec<_>>();
    let transform = PositionTransform::new([100.0, -50.0, 1_000.0], [0.25, 0.5, 2.0]).unwrap();
    let definition = AttributeDefinition::new(
        AttributeId::new(6).unwrap(),
        "classification",
        AttributeDataType::U8,
    )
    .unwrap();
    let column = AttributeColumn::new(definition, AttributeValues::u8(classifications)).unwrap();
    let columns = AttributeColumns::new(vec![column], ticks.len()).unwrap();
    MemorySource::from_columns(transform, CoordinateReference::Unknown, ticks, columns).unwrap()
}
