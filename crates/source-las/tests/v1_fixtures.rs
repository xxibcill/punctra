//! Frozen `SourceRecord` schema-1 compatibility through the LAS adapter.

use std::{fs, path::PathBuf};

use point_contracts::AttributeId;
use point_source::{OpenOptions, SourceError, SourceRecord, VerificationPolicy};
use source_las::open_with;

const RECORD_BYTES: &[u8] = include_bytes!("fixtures/v1/source-record.json");

#[test]
fn frozen_v1_record_full_reopens_exact_las_rows_without_mutating_bytes() {
    let before = fixture_hash();
    let record: SourceRecord = serde_json::from_slice(RECORD_BYTES).expect("valid v1 SourceRecord");
    assert_record_facts(&record);
    let source = open_with(
        source_path(),
        OpenOptions::match_record(record, VerificationPolicy::Full),
    )
    .blocking_wait()
    .expect("frozen record reopens the fixed LAS Source");
    assert_eq!(source.identity().to_string(), EXPECTED_SOURCE_ID);
    assert_eq!(source.metadata().point_count(), 3);
    let mut batches = source.points().expect("open canonical rows");
    let batch = batches.next().expect("read rows").expect("one batch");
    assert_eq!(
        batch
            .attributes()
            .get(AttributeId::new(6).unwrap())
            .expect("classification column")
            .values()
            .as_u8()
            .expect("u8 classifications"),
        [2, 7, 2]
    );
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
        "the LAS adapter must reproduce the frozen record bytes"
    );
    assert_eq!(
        fixture_hash(),
        before,
        "fixture bytes changed during reopen"
    );
}

#[test]
fn fast_refusal_future_version_and_corrupt_evidence_are_non_destructive() {
    let before = fixture_hash();
    let record: SourceRecord = serde_json::from_slice(RECORD_BYTES).expect("valid v1 SourceRecord");
    let error = open_with(
        source_path(),
        OpenOptions::match_record(record, VerificationPolicy::FastOnly),
    )
    .blocking_wait()
    .expect_err("LAS has no stable Fast witness");
    assert!(matches!(error, SourceError::VerificationRequired));

    let mut future: serde_json::Value =
        serde_json::from_slice(RECORD_BYTES).expect("valid fixture JSON");
    future["version"] = serde_json::json!(2);
    let future: SourceRecord = serde_json::from_value(future).expect("bounded future record");
    let error = open_with(
        source_path(),
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
        source_path(),
        OpenOptions::match_record(corrupt, VerificationPolicy::Full),
    )
    .blocking_wait()
    .expect_err("different content evidence is rejected");
    assert!(matches!(error, SourceError::SourceChanged { .. }));
    assert_eq!(fixture_hash(), before, "rejection mutated fixture bytes");
}

const EXPECTED_SOURCE_ID: &str = "f014512df07f3bf03245d337aedb6881fb93aacc7bdce3c9f8280c90aee612f3";
const EXPECTED_CONTENT_HASH: &str =
    "dbfbda7edebbac4050cd2de54cd752b851f7499262a1f8467b7b7d9d9f677121";
const EXPECTED_SOURCE_BYTES_HASH: &str =
    "dbfbda7edebbac4050cd2de54cd752b851f7499262a1f8467b7b7d9d9f677121";
const EXPECTED_RECORD_BYTES_HASH: &str =
    "3151710a91dbc7d8a0654dd7086e2b818ca51f69356b852f2192e7e3a6c234f7";

fn assert_record_facts(record: &SourceRecord) {
    assert_eq!(record.version(), 1);
    assert_eq!(record.source().to_string(), EXPECTED_SOURCE_ID);
    assert_eq!(record.content_hash().to_string(), EXPECTED_CONTENT_HASH);
    assert_eq!(record.adapter_name(), "source-las");
    assert_eq!(record.adapter_version(), "1");
    assert_eq!(record.logical_order(), "LAS/LAZ point-record order v1");
    assert_eq!(record.fast_token(), b"full-only-v1");
    assert_eq!(record.metadata().point_count(), 3);
}

fn fixture_hash() -> (blake3::Hash, blake3::Hash) {
    let source = blake3::hash(&fs::read(source_path()).expect("read frozen LAS bytes"));
    let record = blake3::hash(
        &fs::read(record_path()).expect("read frozen SourceRecord bytes from the filesystem"),
    );
    assert_eq!(source.to_hex().as_str(), EXPECTED_SOURCE_BYTES_HASH);
    assert_eq!(record.to_hex().as_str(), EXPECTED_RECORD_BYTES_HASH);
    (source, record)
}

fn source_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1/tiny.las")
}

fn record_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1/source-record.json")
}
