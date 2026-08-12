//! Maintainer-only generator for the frozen `SourceRecord` v1 memory fixture.

use std::fs;
use std::path::Path;

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use source_memory::{MemorySource, open};

const SOURCE_BYTES: &[u8] = b"{\n  \"ticks\": [[-2, 4, 9], [0, 5, 10], [7, -3, 11]],\n  \"classifications\": [2, 7, 2]\n}\n";

fn main() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1");
    let record_fixture_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../point-source/tests/fixtures/v1");
    fs::create_dir_all(&fixture_dir).expect("create fixture directory");
    fs::create_dir_all(&record_fixture_dir).expect("create SourceRecord fixture directory");
    let source_path = fixture_dir.join("memory-source.json");
    fs::write(&source_path, SOURCE_BYTES).expect("write frozen memory Source bytes");

    let input = fixture_input();
    let source = open(input).blocking_wait().expect("open fixture Source");
    let mut record = serde_json::to_vec_pretty(source.record()).expect("serialize SourceRecord");
    record.push(b'\n');
    let record_path = record_fixture_dir.join("source-memory-record.json");
    fs::write(&record_path, &record).expect("write SourceRecord fixture");

    let record_manifest = serde_json::json!({
        "owner": "point-source",
        "support_class": "authoritative",
        "path_base": "manifest_directory",
        "source_contract_version": 1,
        "source_record_version": source.record().version(),
        "files": [{
            "path": "source-memory-record.json",
            "byte_length": record.len(),
            "blake3": blake3::hash(&record).to_hex().to_string()
        }],
        "expected": {
            "source_id": source.identity().to_string(),
            "content_hash": source.record().content_hash().to_string(),
            "adapter_name": source.record().adapter_name(),
            "adapter_version": source.record().adapter_version(),
            "logical_order": source.record().logical_order(),
            "fast_token_hex": hex(source.record().fast_token()),
            "point_count": source.metadata().point_count()
        },
        "source_bytes": {
            "path": "../../../../source-memory/tests/fixtures/v1/memory-source.json",
            "byte_length": SOURCE_BYTES.len(),
            "blake3": blake3::hash(SOURCE_BYTES).to_hex().to_string()
        }
    });
    let mut record_manifest_bytes =
        serde_json::to_vec_pretty(&record_manifest).expect("serialize SourceRecord manifest");
    record_manifest_bytes.push(b'\n');
    fs::write(
        record_fixture_dir.join("manifest.json"),
        record_manifest_bytes,
    )
    .expect("write SourceRecord manifest");

    let manifest = serde_json::json!({
        "owner": "source-memory",
        "support_class": "authoritative",
        "path_base": "manifest_directory",
        "source_contract_version": 1,
        "source_record_version": source.record().version(),
        "files": [
            {
                "path": "memory-source.json",
                "byte_length": SOURCE_BYTES.len(),
                "blake3": blake3::hash(SOURCE_BYTES).to_hex().to_string()
            },
            {
                "path": "../../../../point-source/tests/fixtures/v1/source-memory-record.json",
                "byte_length": record.len(),
                "blake3": blake3::hash(&record).to_hex().to_string()
            }
        ],
        "expected": {
            "source_id": source.identity().to_string(),
            "content_hash": source.record().content_hash().to_string(),
            "adapter_name": source.record().adapter_name(),
            "adapter_version": source.record().adapter_version(),
            "logical_order": source.record().logical_order(),
            "fast_token_hex": hex(source.record().fast_token()),
            "point_count": source.metadata().point_count(),
            "classification_values_in_logical_order": [2, 7, 2]
        }
    });
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).expect("serialize fixture manifest");
    manifest_bytes.push(b'\n');
    fs::write(fixture_dir.join("manifest.json"), manifest_bytes).expect("write fixture manifest");
}

fn fixture_input() -> MemorySource {
    let transform = PositionTransform::new([100.0, -50.0, 1_000.0], [0.25, 0.5, 2.0]).unwrap();
    let ticks = vec![[-2, 4, 9], [0, 5, 10], [7, -3, 11]];
    let definition = AttributeDefinition::new(
        AttributeId::new(6).unwrap(),
        "classification",
        AttributeDataType::U8,
    )
    .unwrap();
    let column = AttributeColumn::new(definition, AttributeValues::u8(vec![2, 7, 2])).unwrap();
    let columns = AttributeColumns::new(vec![column], ticks.len()).unwrap();
    MemorySource::from_columns(transform, CoordinateReference::Unknown, ticks, columns).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}
