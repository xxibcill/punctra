//! Checked-in v1 Workspace compatibility fixtures.

mod support;

use std::{
    fs,
    path::{Path, PathBuf},
};

use point_contracts::PointId;
use point_index::{PrepareLimits, PreparedIndex, prepare};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRejection, CommitRequest, OpenLimits, OperationId,
    OperationResolution, PointIdReadLimits, PointQuery, PointSetLimits, RevisionId, RevisionKind,
    WorkspaceError, WorkspaceId, WorkspaceSchema, create, open,
};
use serde_json::json;

use support::{
    TemporaryFixture, classification_attribute, classification_for_ordinal, fixture_rows,
    open_source, prepare_fixture,
};

const POINT_COUNT: usize = 16;
const COMMITTED_REVISION_FILE: &str = concat!(
    "revisions/00000000000000000001-",
    "002a7c622f202132150e7f68053714c22d7ef1182f8b02e684f509a674250b86.pwr"
);

#[derive(Clone, Copy)]
struct GoldenFile {
    path: &'static str,
    bytes: &'static [u8],
    digest: &'static str,
}

fn golden_files() -> [GoldenFile; 12] {
    [
        GoldenFile {
            path: "committed/manifest.pwm",
            bytes: include_bytes!("fixtures/workspace-v1/committed/manifest.pwm"),
            digest: "97e2b2ac59f12105aaa8a9dc1ea2cf5689f0557d8bf465614696309c93689a77",
        },
        GoldenFile {
            path: "committed/operations/22222222222222222222222222222222.ready",
            bytes: include_bytes!(
                "fixtures/workspace-v1/committed/operations/22222222222222222222222222222222.ready"
            ),
            digest: "42eec168847d4875d9079e0a6cf365e051ec2c9ec2a82b0f8cf7c95a57ed7bd0",
        },
        GoldenFile {
            path: "committed/revisions/00000000000000000001-002a7c622f202132150e7f68053714c22d7ef1182f8b02e684f509a674250b86.pwr",
            bytes: include_bytes!(
                "fixtures/workspace-v1/committed/revisions/00000000000000000001-002a7c622f202132150e7f68053714c22d7ef1182f8b02e684f509a674250b86.pwr"
            ),
            digest: "42eec168847d4875d9079e0a6cf365e051ec2c9ec2a82b0f8cf7c95a57ed7bd0",
        },
        GoldenFile {
            path: "committed/workspace.lock",
            bytes: include_bytes!("fixtures/workspace-v1/committed/workspace.lock"),
            digest: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        },
        GoldenFile {
            path: "recorded-rejection/manifest.pwm",
            bytes: include_bytes!("fixtures/workspace-v1/recorded-rejection/manifest.pwm"),
            digest: "97e2b2ac59f12105aaa8a9dc1ea2cf5689f0557d8bf465614696309c93689a77",
        },
        GoldenFile {
            path: "recorded-rejection/operations/33333333333333333333333333333333.reject",
            bytes: include_bytes!(
                "fixtures/workspace-v1/recorded-rejection/operations/33333333333333333333333333333333.reject"
            ),
            digest: "205364e6858e0755154e52d4e24060b72775e8b897af99f920966d26c4e26435",
        },
        GoldenFile {
            path: "recorded-rejection/workspace.lock",
            bytes: include_bytes!("fixtures/workspace-v1/recorded-rejection/workspace.lock"),
            digest: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        },
        GoldenFile {
            path: "retryable-ready/manifest.pwm",
            bytes: include_bytes!("fixtures/workspace-v1/retryable-ready/manifest.pwm"),
            digest: "97e2b2ac59f12105aaa8a9dc1ea2cf5689f0557d8bf465614696309c93689a77",
        },
        GoldenFile {
            path: "retryable-ready/operations/22222222222222222222222222222222.ready",
            bytes: include_bytes!(
                "fixtures/workspace-v1/retryable-ready/operations/22222222222222222222222222222222.ready"
            ),
            digest: "42eec168847d4875d9079e0a6cf365e051ec2c9ec2a82b0f8cf7c95a57ed7bd0",
        },
        GoldenFile {
            path: "retryable-ready/workspace.lock",
            bytes: include_bytes!("fixtures/workspace-v1/retryable-ready/workspace.lock"),
            digest: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        },
        GoldenFile {
            path: "root/manifest.pwm",
            bytes: include_bytes!("fixtures/workspace-v1/root/manifest.pwm"),
            digest: "97e2b2ac59f12105aaa8a9dc1ea2cf5689f0557d8bf465614696309c93689a77",
        },
        GoldenFile {
            path: "root/workspace.lock",
            bytes: include_bytes!("fixtures/workspace-v1/root/workspace.lock"),
            digest: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        },
    ]
}

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).expect("nonzero fixture Operation Identity")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("create fixture directory");
    for entry in fs::read_dir(source).expect("read fixture source directory") {
        let entry = entry.expect("read fixture source entry");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry
            .file_type()
            .expect("inspect fixture source entry")
            .is_dir()
        {
            copy_tree(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("copy fixture file");
        }
    }
}

fn only_file(directory: &Path) -> PathBuf {
    let files = fs::read_dir(directory)
        .expect("read single-file directory")
        .map(|entry| entry.expect("read single-file entry").path())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1, "fixture directory has one file");
    files.into_iter().next().expect("fixture file exists")
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn fixture_payloads(root: &Path) -> Vec<PathBuf> {
    fn collect(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).expect("read captured fixture directory") {
            let entry = entry.expect("read captured fixture entry");
            let path = entry.path();
            if entry
                .file_type()
                .expect("inspect captured fixture entry")
                .is_dir()
            {
                collect(root, &path, paths);
            } else {
                paths.push(
                    path.strip_prefix(root)
                        .expect("captured fixture remains below its root")
                        .to_path_buf(),
                );
            }
        }
    }

    let mut paths = Vec::new();
    collect(root, root, &mut paths);
    paths.sort();
    paths
}

fn write_manifest(
    fixture_root: &Path,
    workspace: WorkspaceId,
    source: point_contracts::SourceId,
    root_revision: RevisionId,
    committed_revision: RevisionId,
) {
    let files = fixture_payloads(fixture_root)
        .into_iter()
        .map(|relative| {
            let bytes = fs::read(fixture_root.join(&relative)).expect("read captured fixture file");
            json!({
                "path": relative.to_string_lossy(),
                "bytes": bytes.len(),
                "blake3": blake3::hash(&bytes).to_hex().to_string(),
            })
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "corpus": "punctra-point-workspace-owner-local-v1",
        "generated_data": "synthetic quantized positions and U8 classifications only",
        "disk_version": 1,
        "semantic_version": 1,
        "point_count": POINT_COUNT,
        "classification_attribute": { "id": 101, "name": "classification", "type": "U8" },
        "workspace_id": hex(workspace.as_bytes()),
        "source_id": hex(source.as_bytes()),
        "root_revision": hex(root_revision.as_bytes()),
        "committed_revision": hex(committed_revision.as_bytes()),
        "committed_operation": hex(operation(0x22).as_bytes()),
        "rejected_operation": hex(operation(0x33).as_bytes()),
        "required_directories_per_state": ["operations", "revisions", "scratch"],
        "files": files,
    });
    let mut bytes = serde_json::to_vec_pretty(&manifest).expect("serialize fixture manifest");
    bytes.push(b'\n');
    fs::write(fixture_root.join("manifest.json"), bytes).expect("write fixture manifest");
}

fn materialize(state: &str, root: &Path) {
    for directory in ["operations", "revisions", "scratch"] {
        fs::create_dir_all(root.join(directory)).expect("create fixture state directory");
    }
    let prefix = format!("{state}/");
    for file in golden_files()
        .into_iter()
        .filter(|file| file.path.starts_with(&prefix))
    {
        let relative = file
            .path
            .strip_prefix(&prefix)
            .expect("state prefix was checked");
        let target = root.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create fixture payload parent");
        }
        fs::write(target, file.bytes).expect("materialize immutable fixture bytes");
    }
}

fn decode_hex<const N: usize>(encoded: &str) -> [u8; N] {
    assert_eq!(encoded.len(), N * 2, "fixture identity width is exact");
    let mut bytes = [0_u8; N];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |value: u8| match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            _ => panic!("fixture identity is canonical lowercase hex"),
        };
        bytes[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    bytes
}

fn manifest_value() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/workspace-v1/manifest.json"))
        .expect("checked fixture manifest is valid JSON")
}

fn expected_identity(field: &str) -> String {
    manifest_value()[field]
        .as_str()
        .expect("fixture identity is a JSON string")
        .to_owned()
}

fn ordinals(point_set: &point_workspace::PointSet) -> Vec<u64> {
    let mut batches = point_set
        .ids(PointIdReadLimits::default())
        .expect("read golden Point Set identities");
    let mut ordinals = Vec::new();
    while let Some(batch) = batches.next().expect("golden Point Set batch validates") {
        ordinals.extend(batch.ids().iter().map(|point| point.ordinal()));
    }
    ordinals
}

fn tree_digest(root: &Path) -> blake3::Hash {
    let mut payloads = fixture_payloads(root);
    payloads.sort();
    let mut hasher = blake3::Hasher::new();
    for relative in payloads {
        hasher.update(relative.as_os_str().as_encoded_bytes());
        hasher.update(&[0]);
        hasher.update(&fs::read(root.join(relative)).expect("read mutation fixture payload"));
    }
    hasher.finalize()
}

fn open_failure(root: &Path, index: PreparedIndex) -> WorkspaceError {
    let before = tree_digest(root);
    let error = open(root, index, OpenLimits::default())
        .blocking_wait()
        .expect_err("mutated fixture must fail closed");
    assert_eq!(
        tree_digest(root),
        before,
        "failed open must not partially publish or repair the fixture"
    );
    error
}

fn assert_corrupt(error: WorkspaceError, suffix: &str) {
    let WorkspaceError::Corrupt { reason } = error else {
        panic!("expected stable corrupt Workspace family")
    };
    assert!(
        reason.as_str().ends_with(suffix),
        "unexpected corruption diagnostic: {reason}"
    );
}

fn assert_incompatible(error: WorkspaceError, suffix: &str) {
    let WorkspaceError::Incompatible { reason } = error else {
        panic!("expected stable incompatible Workspace family")
    };
    assert!(
        reason.as_str().ends_with(suffix),
        "unexpected incompatibility diagnostic: {reason}"
    );
}

#[test]
fn checked_in_payloads_match_the_owner_local_manifest() {
    let manifest = manifest_value();
    assert_eq!(manifest["disk_version"], 1);
    assert_eq!(manifest["semantic_version"], 1);
    assert_eq!(manifest["point_count"], POINT_COUNT);
    assert_eq!(
        manifest["generated_data"],
        "synthetic quantized positions and U8 classifications only"
    );
    let entries = manifest["files"]
        .as_array()
        .expect("fixture manifest carries its file table");
    assert_eq!(entries.len(), golden_files().len());
    for file in golden_files() {
        let entry = entries
            .iter()
            .find(|entry| entry["path"] == file.path)
            .expect("every compiled fixture payload is manifested");
        assert_eq!(entry["bytes"], file.bytes.len());
        assert_eq!(entry["blake3"], file.digest);
        assert_eq!(blake3::hash(file.bytes).to_hex().as_str(), file.digest);
    }
}

#[test]
fn all_workspace_states_open_with_exact_lineage_order_and_recovery_results() {
    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("golden-v1-open", POINT_COUNT);
    let workspace_id = WorkspaceId::from_bytes(decode_hex(&expected_identity("workspace_id")))
        .expect("manifested Workspace Identity is valid");
    let root_revision = RevisionId::from_bytes(decode_hex(&expected_identity("root_revision")))
        .expect("manifested root Revision is valid");
    let committed_revision =
        RevisionId::from_bytes(decode_hex(&expected_identity("committed_revision")))
            .expect("manifested committed Revision is valid");

    let root_path = temporary.path().join("golden-root.pcw");
    materialize("root", &root_path);
    let workspace = open(&root_path, index.clone(), OpenLimits::default())
        .blocking_wait()
        .expect("v1 root fixture opens");
    assert_eq!(workspace.identity(), workspace_id);
    assert_eq!(
        hex(workspace.source().as_bytes()),
        expected_identity("source_id")
    );
    assert_eq!(workspace.head().provenance().revision(), root_revision);
    assert_eq!(
        workspace.revision_info(root_revision).unwrap().kind(),
        RevisionKind::Root
    );
    drop(workspace);

    let committed_path = temporary.path().join("golden-committed.pcw");
    materialize("committed", &committed_path);
    let workspace = open(&committed_path, index.clone(), OpenLimits::default())
        .blocking_wait()
        .expect("v1 committed fixture opens");
    assert_eq!(workspace.head().provenance().revision(), committed_revision);
    let info = workspace.revision_info(committed_revision).unwrap();
    assert_eq!(info.sequence(), 1);
    assert_eq!(info.parent(), Some(root_revision));
    assert_eq!(
        info.kind(),
        RevisionKind::SetClassification {
            value: 42,
            changed_points: 3,
        }
    );
    let selected = workspace
        .head()
        .select(
            PointQuery::all().classification_is(42),
            PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("golden committed semantics reproduce");
    assert_eq!(ordinals(&selected), [1, 3, 5]);
    assert!(matches!(
        workspace.resolve_operation(operation(0x22)).unwrap(),
        OperationResolution::Committed(receipt) if receipt.revision() == committed_revision
    ));
    drop(selected);
    drop(workspace);

    let retryable_path = temporary.path().join("golden-retryable.pcw");
    materialize("retryable-ready", &retryable_path);
    let workspace = open(&retryable_path, index.clone(), OpenLimits::default())
        .blocking_wait()
        .expect("v1 ready-only fixture opens");
    assert!(matches!(
        workspace.resolve_operation(operation(0x22)).unwrap(),
        OperationResolution::Retryable(intent)
            if intent.revision() == committed_revision && intent.sequence() == 1
    ));
    assert!(matches!(
        workspace
            .retry_operation(operation(0x22), CommitLimits::default())
            .blocking_wait()
            .expect("ready-only fixture retries deterministically"),
        CommitOutcome::Committed(receipt) if receipt.revision() == committed_revision
    ));
    drop(workspace);

    let rejected_path = temporary.path().join("golden-rejected.pcw");
    materialize("recorded-rejection", &rejected_path);
    let workspace = open(&rejected_path, index, OpenLimits::default())
        .blocking_wait()
        .expect("v1 recorded-rejection fixture opens");
    assert!(matches!(
        workspace.resolve_operation(operation(0x33)).unwrap(),
        OperationResolution::Rejected(recorded)
            if recorded.reason() == CommitRejection::NoChanges
    ));
    assert!(matches!(
        workspace
            .retry_operation(operation(0x33), CommitLimits::default())
            .blocking_wait()
            .expect("recorded rejection remains authoritative"),
        CommitOutcome::Rejected(CommitRejection::NoChanges)
    ));
}

#[test]
fn future_manifest_version_fails_as_incompatible_without_publication() {
    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("golden-v1-future", POINT_COUNT);
    let root = temporary.workspace_path();
    materialize("root", &root);
    let path = root.join("manifest.pwm");
    let mut bytes = fs::read(&path).expect("read manifested Workspace header");
    bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
    let payload_end = bytes.len() - 32;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"punctra-workspace-manifest-v1");
    hasher.update(&bytes[..payload_end]);
    bytes[payload_end..].copy_from_slice(hasher.finalize().as_bytes());
    fs::write(path, bytes).expect("write future-version mutation");
    assert_incompatible(
        open_failure(&root, index),
        "disk or semantic contract version differs",
    );
}

#[test]
fn truncated_manifest_fails_as_corrupt_without_publication() {
    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("golden-v1-truncated", POINT_COUNT);
    let root = temporary.workspace_path();
    materialize("root", &root);
    let path = root.join("manifest.pwm");
    let mut bytes = fs::read(&path).expect("read manifested Workspace header");
    bytes.pop();
    fs::write(path, bytes).expect("write truncation mutation");
    assert_corrupt(
        open_failure(&root, index),
        "published file length differs from its fixed format",
    );
}

#[test]
fn changed_manifest_checksum_fails_as_corrupt_without_publication() {
    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("golden-v1-checksum", POINT_COUNT);
    let root = temporary.workspace_path();
    materialize("root", &root);
    let path = root.join("manifest.pwm");
    let mut bytes = fs::read(&path).expect("read manifested Workspace header");
    bytes[40] ^= 1;
    fs::write(path, bytes).expect("write checksum mutation");
    assert_corrupt(open_failure(&root, index), "manifest checksum differs");
}

#[test]
fn lineage_fork_fails_as_corrupt_without_publication() {
    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("golden-v1-lineage", POINT_COUNT);
    let root = temporary.workspace_path();
    materialize("committed", &root);
    let original = root.join(COMMITTED_REVISION_FILE);
    let forked_name =
        COMMITTED_REVISION_FILE.replacen("00000000000000000001", "00000000000000000002", 1);
    fs::rename(original, root.join(forked_name)).expect("write lineage-fork mutation");
    assert_corrupt(
        open_failure(&root, index),
        "Revision sequence has a gap or fork",
    );
}

#[test]
fn mismatched_source_binding_fails_as_incompatible_without_publication() {
    let fixture = TemporaryFixture::new("golden-v1-source-binding");
    let root = fixture.workspace_path();
    materialize("root", &root);
    let (ticks, mut classifications) = fixture_rows(POINT_COUNT);
    classifications[0] ^= 1;
    let source = open_source(ticks, classifications);
    let index = prepare(source, fixture.index_path(), PrepareLimits::default())
        .blocking_wait()
        .expect("prepare deliberately mismatched fixture Source");
    assert_incompatible(
        open_failure(&root, index),
        "Workspace manifest does not match the verified Source contract",
    );
}

/// Captures the owner-local corpus once from generated, non-secret technical data.
///
/// Normal compatibility tests never invoke this capture path. Delete the existing
/// fixture directory intentionally before recapturing a new format generation.
#[test]
#[ignore = "owner-only fixture capture; checked tests consume immutable bytes"]
fn capture_workspace_v1_fixture_corpus() {
    let fixture_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace-v1");
    assert!(
        !fixture_root.exists(),
        "refusing to overwrite checked-in Workspace fixtures"
    );

    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("golden-v1-capture", POINT_COUNT);
    let source_root = temporary.workspace_path();
    let workspace = create(
        &source_root,
        index.clone(),
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("create fixture Workspace root");
    let workspace_id = workspace.identity();
    let source_id = workspace.source();
    let root_revision = workspace.head().provenance().revision();
    drop(workspace);
    copy_tree(&source_root, &fixture_root.join("root"));

    let committed_root = temporary.path().join("committed.pcw");
    copy_tree(&source_root, &committed_root);
    let workspace = open(&committed_root, index.clone(), OpenLimits::default())
        .blocking_wait()
        .expect("open committed fixture seed");
    let source = workspace.source();
    let target = workspace
        .head()
        .select_point_ids(
            [1_u64, 3, 5].map(|ordinal| PointId::new(source, ordinal)),
            PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("materialize committed fixture target");
    let committed_operation = operation(0x22);
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(committed_operation, target, 42),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("commit fixture operation");
    let CommitOutcome::Committed(receipt) = outcome else {
        panic!("fixture operation must commit")
    };
    assert_eq!(receipt.operation(), committed_operation);
    drop(workspace);
    copy_tree(&committed_root, &fixture_root.join("committed"));

    let retryable_root = temporary.path().join("retryable-ready.pcw");
    copy_tree(&committed_root, &retryable_root);
    fs::remove_file(only_file(&retryable_root.join("revisions")))
        .expect("remove only published Revision link");
    copy_tree(&retryable_root, &fixture_root.join("retryable-ready"));

    let rejected_root = temporary.path().join("recorded-rejection.pcw");
    copy_tree(&source_root, &rejected_root);
    let workspace = open(&rejected_root, index, OpenLimits::default())
        .blocking_wait()
        .expect("open rejection fixture seed");
    let rejected_ordinal = 2_u64;
    let target = workspace
        .head()
        .select_point_ids(
            [PointId::new(workspace.source(), rejected_ordinal)],
            PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("materialize rejection fixture target");
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(
                operation(0x33),
                target,
                classification_for_ordinal(
                    usize::try_from(rejected_ordinal).expect("fixture ordinal fits usize"),
                ),
            ),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("record fixture rejection");
    assert!(matches!(outcome, CommitOutcome::Rejected(_)));
    drop(workspace);
    copy_tree(&rejected_root, &fixture_root.join("recorded-rejection"));

    write_manifest(
        &fixture_root,
        workspace_id,
        source_id,
        root_revision,
        receipt.revision(),
    );
}
