//! Machine-readable manifest verification for the frozen LAS Source.

use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn v1_manifest_paths_lengths_and_hashes_are_exact() {
    let path = manifest_path();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).expect("valid fixture manifest");
    assert_eq!(manifest["owner"], "source-las");
    assert_eq!(manifest["support_class"], "authoritative");
    assert_eq!(manifest["path_base"], "manifest_directory");
    assert_eq!(manifest["source_contract_version"], 1);
    assert_eq!(manifest["source_record_version"], 1);
    assert_eq!(manifest["las_version"], "1.2");
    assert_eq!(manifest["point_format"], 0);
    assert_eq!(
        manifest["expected"],
        serde_json::json!({
            "adapter_name": "source-las",
            "adapter_version": "1",
            "classification_values_in_logical_order": [2, 7, 2],
            "content_hash": "dbfbda7edebbac4050cd2de54cd752b851f7499262a1f8467b7b7d9d9f677121",
            "fast_token_hex": "66756c6c2d6f6e6c792d7631",
            "logical_order": "LAS/LAZ point-record order v1",
            "point_count": 3,
            "source_id": "f014512df07f3bf03245d337aedb6881fb93aacc7bdce3c9f8280c90aee612f3"
        })
    );
    let base = path.parent().unwrap();
    let files = manifest["files"].as_array().expect("files array");
    assert_eq!(files.len(), 2);
    for file in files {
        let file_path = base.join(file["path"].as_str().expect("relative fixture path"));
        let bytes = fs::read(&file_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", file_path.display()));
        assert_eq!(
            u64::try_from(bytes.len()).unwrap(),
            file["byte_length"].as_u64().unwrap()
        );
        assert_eq!(
            blake3::hash(&bytes).to_hex().as_str(),
            file["blake3"].as_str().unwrap()
        );
    }
}

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1/manifest.json")
}
