//! Frozen `SourceRecord` schema-1 bytes owned by the Source contract crate.

use point_source::SourceRecord;

const RECORD_BYTES: &[u8] = include_bytes!("fixtures/v1/source-memory-record.json");
const EXPECTED_RECORD_BYTES_HASH: &str =
    "b2049e1e5b9d93e812757e9c7d477a5e9eae0d741869364691072d0fd89e7216";

#[test]
fn frozen_v1_record_deserializes_with_exact_identity_and_bounded_adapter_facts() {
    assert_eq!(
        blake3::hash(RECORD_BYTES).to_hex().as_str(),
        EXPECTED_RECORD_BYTES_HASH
    );
    let record: SourceRecord = serde_json::from_slice(RECORD_BYTES).expect("valid v1 SourceRecord");
    assert_eq!(record.version(), 1);
    assert_eq!(
        record.source().to_string(),
        "4fb837767b5fd89001059cf40291e17e52136d5e77466bb63804288111868487"
    );
    assert_eq!(
        record.content_hash().to_string(),
        "17a8438d6b9124fd6cdee150c13baaecc8a8e937a65f7328650bb30b61e8c90e"
    );
    assert_eq!(record.adapter_name(), "source-memory");
    assert_eq!(record.adapter_version(), "1");
    assert_eq!(record.logical_order(), "immutable input row order v1");
    assert_eq!(record.fast_token(), 0_u64.to_le_bytes());
    assert_eq!(record.metadata().point_count(), 3);
    assert_eq!(record.metadata().attributes().definitions().len(), 1);
    let mut reproduced = serde_json::to_vec_pretty(&record).expect("serialize v1 SourceRecord");
    reproduced.push(b'\n');
    assert_eq!(
        reproduced, RECORD_BYTES,
        "the schema-1 writer must reproduce the frozen record bytes"
    );
}

#[test]
fn malformed_frozen_bytes_are_rejected_before_a_record_is_published() {
    let mut truncated = RECORD_BYTES.to_vec();
    truncated.truncate(truncated.len() - 2);
    assert!(serde_json::from_slice::<SourceRecord>(&truncated).is_err());

    let mut malformed: serde_json::Value = serde_json::from_slice(RECORD_BYTES).unwrap();
    malformed["adapter_name"] = serde_json::json!("");
    assert!(serde_json::from_value::<SourceRecord>(malformed).is_err());
    assert_eq!(
        blake3::hash(RECORD_BYTES).to_hex().as_str(),
        EXPECTED_RECORD_BYTES_HASH,
        "negative parsing changed the committed fixture"
    );
}
