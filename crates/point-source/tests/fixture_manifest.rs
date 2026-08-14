//! Machine-readable manifest verification for frozen Source records.

use std::fs;
use std::path::{Path, PathBuf};

#[path = "../../../tests/support/fixture_manifest.rs"]
mod fixture_manifest_support;

use fixture_manifest_support::{assert_manifest_file, assert_manifest_files};

#[test]
fn v1_manifest_paths_lengths_and_hashes_are_exact() {
    validate_manifest(&manifest_path());
}

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1/manifest.json")
}

fn validate_manifest(path: &Path) {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(path).unwrap()).expect("valid fixture manifest");
    assert_eq!(manifest["owner"], "point-source");
    assert_eq!(manifest["support_class"], "authoritative");
    assert_eq!(manifest["path_base"], "manifest_directory");
    assert_eq!(manifest["source_contract_version"], 1);
    assert_eq!(manifest["source_record_version"], 1);
    assert_eq!(
        manifest["expected"],
        serde_json::json!({
            "adapter_name": "source-memory",
            "adapter_version": "1",
            "content_hash": "17a8438d6b9124fd6cdee150c13baaecc8a8e937a65f7328650bb30b61e8c90e",
            "fast_token_hex": "0000000000000000",
            "logical_order": "immutable input row order v1",
            "point_count": 3,
            "source_id": "4fb837767b5fd89001059cf40291e17e52136d5e77466bb63804288111868487"
        })
    );
    let base = path.parent().unwrap();
    let files = manifest["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1);
    assert_manifest_files(base, files);
    assert_eq!(
        manifest["source_bytes"]["path"],
        "../../../../source-memory/tests/fixtures/v1/memory-source.json"
    );
    assert_manifest_file(base, &manifest["source_bytes"]);
}
