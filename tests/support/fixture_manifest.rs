//! Mechanical verification shared by persisted-fixture manifest tests.

use std::{fs, path::Path};

pub(crate) fn assert_manifest_files(base: &Path, files: &[serde_json::Value]) {
    for file in files {
        assert_manifest_file(base, file);
    }
}

pub(crate) fn assert_manifest_file(base: &Path, file: &serde_json::Value) {
    let path = base.join(file["path"].as_str().expect("relative fixture path"));
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert_eq!(
        u64::try_from(bytes.len()).unwrap(),
        file["byte_length"].as_u64().unwrap(),
        "{} byte length",
        path.display()
    );
    assert_eq!(
        blake3::hash(&bytes).to_hex().as_str(),
        file["blake3"].as_str().unwrap(),
        "{} BLAKE3",
        path.display()
    );
}
