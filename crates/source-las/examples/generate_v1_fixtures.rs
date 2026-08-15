//! Maintainer-only generator for the frozen `SourceRecord` v1 LAS fixture.

use std::fs;
use std::path::Path;

use las::point::{Classification, Format};
use las::{Builder, Point, Transform, Vector, Writer};
use source_las::open;

fn main() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1");
    fs::create_dir_all(&fixture_dir).expect("create fixture directory");
    let source_path = fixture_dir.join("tiny.las");
    write_source(&source_path);
    let source_bytes = fs::read(&source_path).expect("read generated LAS");

    let source = open(&source_path)
        .blocking_wait()
        .expect("open fixture LAS");
    let mut record = serde_json::to_vec_pretty(source.record()).expect("serialize SourceRecord");
    record.push(b'\n');
    let record_path = fixture_dir.join("source-record.json");
    fs::write(&record_path, &record).expect("write SourceRecord fixture");

    let manifest = serde_json::json!({
        "owner": "source-las",
        "support_class": "authoritative",
        "path_base": "manifest_directory",
        "source_contract_version": 1,
        "source_record_version": source.record().version(),
        "las_version": "1.2",
        "point_format": 0,
        "files": [
            {
                "path": "tiny.las",
                "byte_length": source_bytes.len(),
                "blake3": blake3::hash(&source_bytes).to_hex().to_string()
            },
            {
                "path": "source-record.json",
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

fn write_source(path: &Path) {
    let mut builder = Builder::from((1, 2));
    builder.file_source_id = 9;
    "Punctra v1 fixture".clone_into(&mut builder.generating_software);
    "Punctra tests".clone_into(&mut builder.system_identifier);
    builder.point_format = Format::new(0).unwrap();
    builder.transforms = Vector {
        x: Transform {
            scale: 0.25,
            offset: 100.0,
        },
        y: Transform {
            scale: 0.5,
            offset: -50.0,
        },
        z: Transform {
            scale: 2.0,
            offset: 1_000.0,
        },
    };
    let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for (position, intensity, classification) in [
        ([99.5, -48.0, 1_018.0], 100, 2),
        ([100.0, -47.5, 1_020.0], 200, 7),
        ([101.75, -51.5, 1_022.0], 300, 2),
    ] {
        writer
            .write_point(Point {
                x: position[0],
                y: position[1],
                z: position[2],
                intensity,
                classification: Classification::new(classification).unwrap(),
                ..Point::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
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
