//! Frozen `Workspace` disk-1/semantic-1 compatibility and recovery evidence.

mod support;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_index::{PrepareLimits, PreparedIndex, prepare};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRejection, OpenLimits, OperationId, OperationResolution,
    PointQuery, PointRowLimits, RevisionAuditLimits, RevisionId, RevisionKind, WorkspaceError,
    WorkspaceId, open,
};
use source_memory::MemorySource;

use support::TemporaryFixture;

const FIXTURE_ROOT: &str = "tests/fixtures/v1/workspace";
const MANIFEST_DOMAIN: &[u8] = b"punctra-workspace-manifest-v1";
const WORKSPACE_ID: &str = "5a74dd496f026424f735244581c93064";
const SOURCE_ID: &str = "c59395f7e623418147be771e09ff6b6b86195fe1c0444b10b6efb2fdac217240";
const ROOT_REVISION: &str = "f0acf1bbbd2186bbe2441a1c6400362510dae1630347fb1e42befe4298c59ae3";
const COMMITTED_REVISION: &str = "4cf1a675aa841c6e804d10917eea98a9f908f31885e1b379788923e7c2dfeb68";
const RETRYABLE_REVISION: &str = "47723f465e92756cbbcdbe75044f28e2b466e2e8c616741eca501ed9d9ced122";
const COMMITTED_AUDIT_POINT_ID_HASH: &str =
    "b1d6972fa0e1a9dee2ba5d920b9a1c1a171235805c89c88a07defdf94c8d83fa";
const COMMITTED_AUDIT_CONTENT_HASH: &str =
    "1bd5af535b21866e4563829455c698805a301d2f8a1e7275e9b822f5c3a483d4";

#[test]
fn v1_manifest_paths_lengths_and_hashes_are_exact() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1/manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).expect("valid fixture manifest");
    assert_eq!(manifest["owner"], "point-workspace");
    assert_eq!(manifest["support_class"], "authoritative");
    assert_eq!(manifest["path_base"], "manifest_directory");
    assert_eq!(manifest["disk_version"], 1);
    assert_eq!(manifest["semantic_version"], 1);
    assert_eq!(manifest["source_contract_version"], 1);
    assert_eq!(
        manifest["expected"],
        serde_json::json!({
            "committed_audit_content_hash": COMMITTED_AUDIT_CONTENT_HASH,
            "committed_audit_point_id_hash": COMMITTED_AUDIT_POINT_ID_HASH,
            "committed_changed_ordinals": [1, 4, 7],
            "committed_operation": "01010101010101010101010101010101",
            "committed_revision": COMMITTED_REVISION,
            "committed_revision_sequence": 1,
            "committed_value": 42,
            "rejected_operation": "03030303030303030303030303030303",
            "rejection": "NoChanges",
            "retryable_changed_ordinals": [2, 5],
            "retryable_operation": "02020202020202020202020202020202",
            "retryable_revision": RETRYABLE_REVISION,
            "retryable_revision_sequence": 2,
            "retryable_value": 43,
            "root_revision": ROOT_REVISION,
            "workspace_id": WORKSPACE_ID
        })
    );
    assert_eq!(
        manifest["source_fixture"],
        serde_json::json!({
            "classification_values_in_logical_order": classifications(),
            "point_count": 12,
            "source_id": SOURCE_ID
        })
    );
    let base = path.parent().unwrap();
    let files = manifest["files"].as_array().expect("files array");
    assert_eq!(files.len(), 5);
    for file in files {
        let fixture = base.join(file["path"].as_str().expect("relative fixture path"));
        let bytes = fs::read(&fixture)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture.display()));
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

#[test]
fn frozen_workspace_opens_exact_lineage_and_operations_without_byte_mutation() {
    let temporary = TemporaryFixture::new("v1-fixture-open");
    let index = prepare_fixed_index(&temporary);
    let fixture = fixture_root();
    let before = assert_frozen_files(&fixture);
    let root = temporary.path().join("workspace-copy.pcw");
    copy_workspace_fixture(&fixture, &root);
    let copied_before = authoritative_tree_hashes(&root);
    assert_committed_revision_alias(&root);
    let workspace = open(&root, index, OpenLimits::default())
        .blocking_wait()
        .expect("frozen Workspace opens through its public owner");

    assert_eq!(workspace.identity(), workspace_id());
    assert_eq!(workspace.source().to_string(), SOURCE_ID);
    assert_eq!(
        workspace.head().provenance().revision(),
        committed_revision()
    );
    let root_info = workspace.revision_info(root_revision()).unwrap();
    assert_eq!(root_info.sequence(), 0);
    assert_eq!(root_info.kind(), RevisionKind::Root);
    let committed_info = workspace.revision_info(committed_revision()).unwrap();
    assert_eq!(committed_info.parent(), Some(root_revision()));
    assert_eq!(committed_info.sequence(), 1);
    assert_eq!(committed_info.operation(), Some(operation(1)));
    assert_eq!(
        committed_info.kind(),
        RevisionKind::SetClassification {
            value: 42,
            changed_points: 3
        }
    );
    let audit = workspace
        .revision_audit(committed_revision(), RevisionAuditLimits::default())
        .blocking_wait()
        .expect("frozen Revision audits");
    assert_eq!(
        audit.point_id_hash().to_string(),
        COMMITTED_AUDIT_POINT_ID_HASH
    );
    assert_eq!(
        audit.content_hash().to_string(),
        COMMITTED_AUDIT_CONTENT_HASH
    );
    assert_eq!(audit.changed_point_count(), 3);
    assert_eq!(
        audit
            .transitions()
            .iter()
            .map(|transition| (transition.before(), transition.after(), transition.count()))
            .collect::<Vec<_>>(),
        [(1, 42, 1), (4, 42, 1), (7, 42, 1)]
    );

    match workspace.resolve_operation(operation(1)).unwrap() {
        OperationResolution::Committed(receipt) => {
            assert_eq!(receipt.operation(), operation(1));
            assert_eq!(receipt.revision(), committed_revision());
        }
        other => panic!("expected committed operation, got {other:?}"),
    }
    match workspace.resolve_operation(operation(2)).unwrap() {
        OperationResolution::Retryable(intent) => {
            assert_eq!(intent.operation(), operation(2));
            assert_eq!(intent.parent().revision(), committed_revision());
            assert_eq!(intent.revision(), retryable_revision());
            assert_eq!(intent.sequence(), 2);
            assert_eq!(
                intent.kind(),
                RevisionKind::SetClassification {
                    value: 43,
                    changed_points: 2
                }
            );
        }
        other => panic!("expected retryable operation, got {other:?}"),
    }
    match workspace.resolve_operation(operation(3)).unwrap() {
        OperationResolution::Rejected(rejection) => {
            assert_eq!(rejection.operation(), operation(3));
            assert_eq!(rejection.reason(), CommitRejection::NoChanges);
        }
        other => panic!("expected recorded rejection, got {other:?}"),
    }

    assert_eq!(snapshot_rows(&workspace), expected_committed_rows());
    drop(workspace);
    assert_eq!(
        authoritative_tree_hashes(&root),
        copied_before,
        "opening the copied fixture changed authoritative Workspace bytes"
    );
    assert_committed_revision_alias(&root);
    assert_eq!(assert_frozen_files(&fixture), before);
}

#[test]
fn frozen_ready_operation_retries_to_exactly_one_revision_and_is_idempotent() {
    let temporary = TemporaryFixture::new("v1-fixture-retry");
    let root = temporary.path().join("workspace-copy.pcw");
    copy_workspace_fixture(&fixture_root(), &root);
    let workspace = open(
        &root,
        prepare_fixed_index(&temporary),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("copied frozen Workspace opens");
    let first = committed(
        workspace
            .retry_operation(operation(2), CommitLimits::default())
            .blocking_wait()
            .expect("ready payload retries"),
    );
    assert_eq!(first.operation(), operation(2));
    assert_eq!(first.revision(), retryable_revision());
    assert_eq!(first.revision_info().parent(), Some(committed_revision()));
    assert_eq!(first.revision_info().sequence(), 2);
    let second = committed(
        workspace
            .retry_operation(operation(2), CommitLimits::default())
            .blocking_wait()
            .expect("retry is idempotent"),
    );
    assert_eq!(second, first);
    assert_eq!(
        workspace.head().provenance().revision(),
        retryable_revision()
    );
    assert_eq!(snapshot_rows(&workspace), expected_retried_rows());
    let retried_revision = root.join(format!(
        "revisions/00000000000000000002-{RETRYABLE_REVISION}.pwr"
    ));
    let retryable_ready = root.join("operations/02020202020202020202020202020202.ready");
    assert_eq!(
        fs::read(&retried_revision).unwrap(),
        fs::read(&retryable_ready).unwrap(),
        "retry must publish the exact frozen ready payload as its Revision"
    );
    assert_same_file_identity(&retried_revision, &retryable_ready);
    assert_eq!(
        fs::read_dir(root.join("revisions")).unwrap().count(),
        2,
        "retry publishes exactly one new immutable Revision"
    );
}

#[test]
fn version_truncation_corruption_lineage_and_source_mismatch_fail_closed() {
    for (label, mutation, expected) in [
        (
            "future-version",
            mutate_manifest_version as fn(&Path),
            ErrorKind::Incompatible,
        ),
        (
            "truncated-manifest",
            truncate_manifest as fn(&Path),
            ErrorKind::Corrupt,
        ),
        (
            "checksum",
            corrupt_manifest as fn(&Path),
            ErrorKind::Corrupt,
        ),
        (
            "source-binding",
            mutate_manifest_source as fn(&Path),
            ErrorKind::Incompatible,
        ),
        (
            "lineage-fork",
            mutate_manifest_root_revision as fn(&Path),
            ErrorKind::Corrupt,
        ),
    ] {
        let temporary = TemporaryFixture::new(label);
        let root = temporary.path().join("workspace-copy.pcw");
        copy_workspace_fixture(&fixture_root(), &root);
        mutation(&root.join("manifest.pwm"));
        let before = tree_hashes(&root);
        let error = open(
            &root,
            prepare_fixed_index(&temporary),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect_err("mutated fixture is rejected");
        match expected {
            ErrorKind::Incompatible => {
                assert!(matches!(error, WorkspaceError::Incompatible { .. }));
            }
            ErrorKind::Corrupt => assert!(matches!(error, WorkspaceError::Corrupt { .. })),
        }
        let after = authoritative_tree_hashes(&root);
        assert_eq!(
            after, before,
            "failed open changed copied authoritative bytes"
        );
        assert_frozen_files(&fixture_root());
    }
}

#[derive(Clone, Copy)]
enum ErrorKind {
    Incompatible,
    Corrupt,
}

fn committed(outcome: CommitOutcome) -> point_workspace::CommitReceipt {
    match outcome {
        CommitOutcome::Committed(receipt) => receipt,
        other => panic!("expected committed outcome, got {other:?}"),
    }
}

fn snapshot_rows(workspace: &point_workspace::Workspace) -> Vec<(u64, u8)> {
    let mut stream = workspace
        .head()
        .point_rows(PointQuery::all(), PointRowLimits::default())
        .expect("open exact row stream");
    let mut rows = Vec::new();
    while let Some(batch) = stream.next().expect("read exact row batch") {
        rows.extend(
            batch
                .ordinals()
                .iter()
                .copied()
                .zip(batch.effective_classifications().iter().copied()),
        );
    }
    assert_eq!(
        stream
            .summary()
            .expect("complete row summary")
            .exact_count(),
        12
    );
    rows
}

fn expected_committed_rows() -> Vec<(u64, u8)> {
    classifications()
        .into_iter()
        .enumerate()
        .map(|(ordinal, value)| {
            let value = if [1, 4, 7].contains(&ordinal) {
                42
            } else {
                value
            };
            (u64::try_from(ordinal).unwrap(), value)
        })
        .collect()
}

fn expected_retried_rows() -> Vec<(u64, u8)> {
    expected_committed_rows()
        .into_iter()
        .map(|(ordinal, value)| {
            let value = if [2, 5].contains(&ordinal) { 43 } else { value };
            (ordinal, value)
        })
        .collect()
}

fn prepare_fixed_index(temporary: &TemporaryFixture) -> PreparedIndex {
    prepare(
        fixed_source(),
        temporary.index_path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .expect("prepare deterministic Source index")
}

fn fixed_source() -> point_source::Source {
    let ticks = (0..12_i64)
        .map(|ordinal| [ordinal - 6, ordinal.rem_euclid(5) - 2, ordinal * 2 - 7])
        .collect::<Vec<_>>();
    let definition = AttributeDefinition::new(
        AttributeId::new(101).unwrap(),
        "classification",
        AttributeDataType::U8,
    )
    .unwrap();
    let column = AttributeColumn::new(definition, AttributeValues::u8(classifications())).unwrap();
    let columns = AttributeColumns::new(vec![column], ticks.len()).unwrap();
    let input = MemorySource::from_columns(
        PositionTransform::new([100.25, -50.5, 1_000.0], [0.25, 0.5, 2.0]).unwrap(),
        CoordinateReference::Unknown,
        ticks,
        columns,
    )
    .unwrap();
    let source = source_memory::open(input).blocking_wait().unwrap();
    assert_eq!(source.identity().to_string(), SOURCE_ID);
    source
}

fn classifications() -> Vec<u8> {
    (0..12)
        .map(|ordinal| u8::try_from((ordinal * 7 + ordinal / 11) % 8).unwrap())
        .collect()
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT)
}

fn workspace_id() -> WorkspaceId {
    WorkspaceId::try_from_slice(&decode_hex::<16>(WORKSPACE_ID)).unwrap()
}

fn root_revision() -> RevisionId {
    RevisionId::try_from_slice(&decode_hex::<32>(ROOT_REVISION)).unwrap()
}

fn committed_revision() -> RevisionId {
    RevisionId::try_from_slice(&decode_hex::<32>(COMMITTED_REVISION)).unwrap()
}

fn retryable_revision() -> RevisionId {
    RevisionId::try_from_slice(&decode_hex::<32>(RETRYABLE_REVISION)).unwrap()
}

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).unwrap()
}

fn decode_hex<const N: usize>(hex: &str) -> [u8; N] {
    assert_eq!(hex.len(), N * 2);
    let mut bytes = [0_u8; N];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    bytes
}

fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("fixture identity is lowercase hexadecimal"),
    }
}

fn assert_frozen_files(root: &Path) -> Vec<(String, u64, String)> {
    let actual = tree_hashes(root);
    let expected = vec![
        file(
            "manifest.pwm",
            1252,
            "acf0be994588cb339ef0cd09e0a87c9bea9525b9ce2595d60fe666ba6cbd7b33",
        ),
        file(
            "operations/01010101010101010101010101010101.ready",
            534,
            "0230f5d1a3e4268323d38ec7fe383cde2cdaee7a4ed75971c296694489f804b5",
        ),
        file(
            "operations/02020202020202020202020202020202.ready",
            524,
            "3a0b7bb03d6fe6955420f2c674c9ceaf4e487369c7198df0e9175a34901a67ad",
        ),
        file(
            "operations/03030303030303030303030303030303.reject",
            184,
            "cc6f7e4c76c4c36b959b1b74b0d2503d4403557a65810d76ef6421b83fbce245",
        ),
        file(
            "revisions/00000000000000000001-4cf1a675aa841c6e804d10917eea98a9f908f31885e1b379788923e7c2dfeb68.pwr",
            534,
            "0230f5d1a3e4268323d38ec7fe383cde2cdaee7a4ed75971c296694489f804b5",
        ),
    ];
    assert_eq!(actual, expected);
    actual
}

fn file(path: &str, bytes: u64, hash: &str) -> (String, u64, String) {
    (path.to_owned(), bytes, hash.to_owned())
}

fn tree_hashes(root: &Path) -> Vec<(String, u64, String)> {
    let mut files = Vec::new();
    collect_hashes(root, root, &mut files);
    files.sort();
    files
}

fn authoritative_tree_hashes(root: &Path) -> Vec<(String, u64, String)> {
    tree_hashes(root)
        .into_iter()
        .filter(|(path, _, _)| path != "workspace.lock")
        .collect()
}

fn collect_hashes(root: &Path, directory: &Path, files: &mut Vec<(String, u64, String)>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_hashes(root, &path, files);
        } else {
            let bytes = fs::read(&path).unwrap();
            files.push((
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                u64::try_from(bytes.len()).unwrap(),
                blake3::hash(&bytes).to_hex().to_string(),
            ));
        }
    }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let source_path = entry.unwrap().path();
        let target_path = target.join(source_path.file_name().unwrap());
        if source_path.is_dir() {
            copy_tree(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).unwrap();
        }
    }
}

fn copy_workspace_fixture(source: &Path, target: &Path) {
    copy_tree(source, target);
    fs::create_dir_all(target.join("scratch")).unwrap();
    let ready = target.join("operations/01010101010101010101010101010101.ready");
    let revision = target.join(format!(
        "revisions/00000000000000000001-{COMMITTED_REVISION}.pwr"
    ));
    let mut permissions = fs::metadata(&ready).unwrap().permissions();
    #[cfg(unix)]
    permissions.set_mode(permissions.mode() | 0o200);
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(&ready, permissions).unwrap();
    fs::remove_file(&ready).unwrap();
    fs::hard_link(&revision, &ready).unwrap();
    assert_same_file_identity(&revision, &ready);
}

fn assert_committed_revision_alias(root: &Path) {
    assert_same_file_identity(
        &root.join(format!(
            "revisions/00000000000000000001-{COMMITTED_REVISION}.pwr"
        )),
        &root.join("operations/01010101010101010101010101010101.ready"),
    );
}

fn assert_same_file_identity(first: &Path, second: &Path) {
    assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let first = fs::metadata(first).unwrap();
        let second = fs::metadata(second).unwrap();
        assert_eq!((first.dev(), first.ino()), (second.dev(), second.ino()));
    }
}

fn mutate_manifest_version(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
    rewrite_manifest_checksum(&mut bytes);
    write_mutation(path, &bytes);
}

fn truncate_manifest(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    bytes.truncate(bytes.len() - 1);
    write_mutation(path, &bytes);
}

fn mutate_manifest_source(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    bytes[32] ^= 0x80;
    rewrite_manifest_checksum(&mut bytes);
    write_mutation(path, &bytes);
}

fn mutate_manifest_root_revision(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    let root_revision_start = bytes.len() - 3 * 32;
    bytes[root_revision_start] ^= 0x80;
    rewrite_manifest_checksum(&mut bytes);
    write_mutation(path, &bytes);
}

fn rewrite_manifest_checksum(bytes: &mut [u8]) {
    let payload_len = bytes.len() - 32;
    let mut hasher = blake3::Hasher::new();
    hasher.update(MANIFEST_DOMAIN);
    hasher.update(&bytes[..payload_len]);
    let checksum = *hasher.finalize().as_bytes();
    bytes[payload_len..].copy_from_slice(&checksum);
}

fn corrupt_manifest(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    bytes[32] ^= 0x80;
    write_mutation(path, &bytes);
}

fn write_mutation(path: &Path, bytes: &[u8]) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[cfg(unix)]
    permissions.set_mode(permissions.mode() | 0o200);
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
    fs::write(path, bytes).unwrap();
}
