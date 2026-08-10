//! Durable classification, Revert, recovery, and fail-closed public behavior.

#[path = "support/evidence.rs"]
mod evidence;
mod support;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use point_contracts::PointId;
use point_workspace::{
    CommitLimits, CommitOutcome, CommitReceipt, CommitRejection, CommitRequest, OpenLimits,
    OperationId, OperationResolution, PointQuery, RevisionKind, WorkspaceError, open,
};

use evidence::{create_fixture_workspace, forced_spill_limits, ordinals, selection_limits};
const MIB: u64 = 1024 * 1024;

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).expect("nonzero deterministic Operation Identity")
}

fn committed(outcome: CommitOutcome) -> CommitReceipt {
    match outcome {
        CommitOutcome::Committed(receipt) => receipt,
        CommitOutcome::Rejected(reason) => panic!("commit unexpectedly rejected: {reason:?}"),
        CommitOutcome::Indeterminate(uncertainty) => {
            panic!("commit unexpectedly indeterminate: {uncertainty:?}")
        }
    }
}

fn only_file(directory: impl AsRef<Path>) -> PathBuf {
    let mut paths = fs::read_dir(directory)
        .expect("read fixture directory")
        .map(|entry| entry.expect("read fixture entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths.len(),
        1,
        "fixture contains exactly one published file"
    );
    paths.pop().expect("one fixture file exists")
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end scenario proves the same snapshots before and after reopen"
)]
fn mixed_classification_commit_revert_and_reopen_preserve_every_snapshot() {
    let (temporary, index, workspace, _ticks, classifications) =
        create_fixture_workspace("commit-revert", 1_025);
    let source = workspace.source();
    let root = workspace.head();
    let root_revision = root.provenance().revision();
    let target_ordinals = vec![0_u64, 1, 2, 11, 12, 98, 255, 512, 1_024];
    assert!(
        target_ordinals
            .iter()
            .map(|&ordinal| classifications[usize::try_from(ordinal).unwrap()])
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1,
        "the Edit target has mixed before classifications"
    );
    let target_ids = target_ordinals
        .iter()
        .map(|&ordinal| PointId::new(source, ordinal))
        .collect::<Vec<_>>();
    let points = root
        .select_point_ids(target_ids, forced_spill_limits(3))
        .blocking_wait()
        .expect("mixed target materializes");

    let set_operation = operation(1);
    let set_receipt = committed(
        workspace
            .commit(
                CommitRequest::set_classification(set_operation, points.clone(), 42),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("classification commit returns a certain outcome"),
    );
    let set_revision = set_receipt.revision();
    assert_eq!(set_receipt.operation(), set_operation);
    assert_eq!(set_receipt.revision_info().parent(), Some(root_revision));
    assert_eq!(set_receipt.revision_info().sequence(), 1);
    assert_eq!(
        set_receipt.revision_info().kind(),
        RevisionKind::SetClassification {
            value: 42,
            changed_points: u64::try_from(target_ordinals.len()).unwrap(),
        }
    );
    let set_snapshot = workspace.snapshot(set_revision).unwrap();
    let classified = set_snapshot
        .select(
            PointQuery::all().classification_is(42),
            selection_limits(7, 8 * MIB),
        )
        .blocking_wait()
        .expect("committed Snapshot applies its classification overlay");
    assert_eq!(ordinals(&classified, 2), target_ordinals);
    let root_still_has_no_42 = root
        .select(
            PointQuery::all().classification_is(42),
            selection_limits(17, 8 * MIB),
        )
        .blocking_wait()
        .expect("historical root remains readable");
    assert!(ordinals(&root_still_has_no_42, 1).is_empty());

    let revert_operation = operation(2);
    let revert_receipt = committed(
        workspace
            .commit(
                CommitRequest::revert_head(revert_operation, set_revision),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("Revert commit returns a certain outcome"),
    );
    let revert_revision = revert_receipt.revision();
    assert_eq!(revert_receipt.revision_info().parent(), Some(set_revision));
    assert_eq!(revert_receipt.revision_info().sequence(), 2);
    assert_eq!(
        revert_receipt.revision_info().kind(),
        RevisionKind::Revert {
            reverted_revision: set_revision,
            changed_points: u64::try_from(target_ordinals.len()).unwrap(),
        }
    );
    let reverted = workspace.snapshot(revert_revision).unwrap();
    for classification in 0..8_u8 {
        let expected = classifications
            .iter()
            .enumerate()
            .filter_map(|(ordinal, &value)| {
                (value == classification)
                    .then_some(u64::try_from(ordinal).expect("fixture ordinal fits u64"))
            })
            .collect::<Vec<_>>();
        let actual = reverted
            .select(
                PointQuery::all().classification_is(classification),
                selection_limits(31, 8 * MIB),
            )
            .blocking_wait()
            .expect("Reverted Snapshot remains exact");
        assert_eq!(ordinals(&actual, 19), expected);
    }
    assert_eq!(
        ordinals(
            &set_snapshot
                .select(
                    PointQuery::all().classification_is(42),
                    selection_limits(5, 8 * MIB)
                )
                .blocking_wait()
                .expect("older edited Snapshot is immutable after Revert"),
            3,
        ),
        target_ordinals
    );

    drop(root_still_has_no_42);
    drop(classified);
    drop(points);
    drop(reverted);
    drop(set_snapshot);
    drop(root);
    drop(workspace);

    let reopened = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("committed and Reverted Workspace reopens");
    assert_eq!(reopened.head().provenance().revision(), revert_revision);
    assert_eq!(
        reopened.revision_info(root_revision).unwrap().kind(),
        RevisionKind::Root
    );
    assert_eq!(
        ordinals(
            &reopened
                .snapshot(set_revision)
                .unwrap()
                .select(
                    PointQuery::all().classification_is(42),
                    selection_limits(11, 8 * MIB)
                )
                .blocking_wait()
                .expect("historical edited Snapshot survives reopen"),
            4,
        ),
        target_ordinals
    );
}

#[test]
fn operation_identity_is_idempotent_and_durable_rejections_remain_authoritative() {
    let (temporary, index, workspace, _ticks, _classifications) =
        create_fixture_workspace("operation-identity", 513);
    let root = workspace.head();
    let class_two = root
        .select(
            PointQuery::all().classification_is(2),
            selection_limits(23, 8 * MIB),
        )
        .blocking_wait()
        .expect("class-two target materializes");
    assert!(class_two.metadata().exact_count() > 0);

    let rejected_operation = operation(3);
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(rejected_operation, class_two.clone(), 2),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("no-change operation produces a durable outcome");
    assert!(matches!(
        outcome,
        CommitOutcome::Rejected(CommitRejection::NoChanges)
    ));
    match workspace.resolve_operation(rejected_operation).unwrap() {
        OperationResolution::Rejected(recorded) => {
            assert_eq!(recorded.operation(), rejected_operation);
            assert_eq!(recorded.reason(), CommitRejection::NoChanges);
        }
        other => panic!("expected recorded no-change rejection, got {other:?}"),
    }

    let conflict = workspace
        .commit(
            CommitRequest::set_classification(rejected_operation, class_two.clone(), 9),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("conflicting identity reuse has a definitive outcome");
    assert!(matches!(
        conflict,
        CommitOutcome::Rejected(CommitRejection::OperationConflict)
    ));
    assert!(matches!(
        workspace.resolve_operation(operation(99)).unwrap(),
        OperationResolution::NotRecorded
    ));

    drop(class_two);
    drop(root);
    drop(workspace);
    let reopened = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("Workspace with rejection record reopens");
    assert!(matches!(
        reopened
            .retry_operation(rejected_operation, CommitLimits::default())
            .blocking_wait()
            .expect("retrying a rejected operation remains definitive"),
        CommitOutcome::Rejected(CommitRejection::NoChanges)
    ));
}

#[test]
fn complete_ready_intent_reconciles_and_retries_without_the_original_point_set() {
    let (temporary, index, workspace, _ticks, _classifications) =
        create_fixture_workspace("ready-retry", 333);
    let root = workspace.head();
    let target = root
        .select_point_ids(
            [5, 7, 9, 11].map(|ordinal| PointId::new(workspace.source(), ordinal)),
            forced_spill_limits(2),
        )
        .blocking_wait()
        .expect("retry target materializes");
    let operation = operation(4);
    let first = committed(
        workspace
            .commit(
                CommitRequest::set_classification(operation, target, 40),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("initial classification commits"),
    );
    let revision = first.revision();

    drop(root);
    drop(workspace);
    let published_revision = only_file(temporary.workspace_path().join("revisions"));
    fs::remove_file(&published_revision)
        .expect("fault fixture removes only the Revision link while retaining ready intent");

    let reopened = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("ready-only crash shape is recoverable");
    match reopened.resolve_operation(operation).unwrap() {
        OperationResolution::Retryable(intent) => {
            assert_eq!(intent.operation(), operation);
            assert_eq!(intent.revision(), revision);
            assert_eq!(intent.sequence(), 1);
        }
        other => panic!("expected retryable durable intent, got {other:?}"),
    }
    let retry = committed(
        reopened
            .retry_operation(operation, CommitLimits::default())
            .blocking_wait()
            .expect("ready payload retries without a Point Set"),
    );
    assert_eq!(retry, first);
    let idempotent = committed(
        reopened
            .retry_operation(operation, CommitLimits::default())
            .blocking_wait()
            .expect("second retry is idempotent"),
    );
    assert_eq!(idempotent, first);
    assert_eq!(
        fs::read_dir(temporary.workspace_path().join("revisions"))
            .expect("read revisions")
            .count(),
        1
    );
}

#[test]
fn hard_commit_limits_leave_the_head_and_operation_catalog_unchanged() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("commit-limits", 129);
    let root = workspace.head();
    let root_revision = root.provenance().revision();
    let target = root
        .select_point_ids(
            [1, 2, 3].map(|ordinal| PointId::new(workspace.source(), ordinal)),
            selection_limits(2, 8 * MIB),
        )
        .blocking_wait()
        .expect("limit target materializes");
    let defaults = CommitLimits::default();
    let limits = CommitLimits::new(
        2,
        defaults.max_changed_points(),
        defaults.max_input_frames(),
        defaults.max_block_points(),
        defaults.max_block_bytes(),
        defaults.max_working_bytes(),
        defaults.max_temporary_bytes(),
        defaults.max_revision_bytes(),
        defaults.max_total_durable_bytes(),
    );
    let operation = operation(5);
    let error = workspace
        .commit(
            CommitRequest::set_classification(operation, target, 41),
            limits,
        )
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        WorkspaceError::ResourceLimit {
            limit: "selected Points",
            required: 3,
            allowed: 2
        }
    ));
    assert_eq!(workspace.head().provenance().revision(), root_revision);
    assert!(matches!(
        workspace.resolve_operation(operation).unwrap(),
        OperationResolution::NotRecorded
    ));
}

#[test]
fn point_set_retains_the_exclusive_session_lock_until_its_last_handle_drops() {
    let (temporary, index, workspace, _ticks, _classifications) =
        create_fixture_workspace("point-set-lock", 257);
    let root = workspace.head();
    let point_set = root
        .select(PointQuery::all(), forced_spill_limits(13))
        .blocking_wait()
        .expect("spilled Point Set materializes");
    let clone = point_set.clone();
    drop(root);
    drop(workspace);

    let locked = open(
        temporary.workspace_path(),
        index.clone(),
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();
    assert!(matches!(locked, WorkspaceError::Locked));
    drop(point_set);
    let still_locked = open(
        temporary.workspace_path(),
        index.clone(),
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();
    assert!(matches!(still_locked, WorkspaceError::Locked));
    drop(clone);

    open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("last Point Set handle releases the session lock");
}

#[test]
fn published_revision_corruption_fails_closed_on_reopen() {
    let (temporary, index, workspace, _ticks, _classifications) =
        create_fixture_workspace("revision-corruption", 257);
    let root = workspace.head();
    let target = root
        .select_point_ids(
            [8, 13, 21].map(|ordinal| PointId::new(workspace.source(), ordinal)),
            selection_limits(2, 8 * MIB),
        )
        .blocking_wait()
        .expect("corruption target materializes");
    committed(
        workspace
            .commit(
                CommitRequest::set_classification(operation(6), target, 55),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("fixture Revision commits"),
    );
    drop(root);
    drop(workspace);

    let revision = only_file(temporary.workspace_path().join("revisions"));
    let mut permissions = fs::metadata(&revision).unwrap().permissions();
    #[cfg(unix)]
    permissions.set_mode(permissions.mode() | 0o200);
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(&revision, permissions).expect("fault fixture unlocks its temporary inode");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&revision)
        .expect("open temporary Revision for corruption injection");
    let offset = file.metadata().unwrap().len() / 2;
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x80;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let error = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Corrupt { .. }));
}

#[test]
fn open_limits_fail_without_changing_a_valid_workspace() {
    let (temporary, index, workspace, _ticks, _classifications) =
        create_fixture_workspace("open-limits", 17);
    drop(workspace);
    let defaults = OpenLimits::default();
    let limits = OpenLimits::new(
        defaults.max_manifest_bytes(),
        defaults.max_operation_records(),
        0,
        defaults.max_revision_blocks(),
        defaults.max_revision_rows(),
        defaults.max_revision_block_bytes(),
        defaults.max_single_file_bytes(),
        defaults.max_total_persisted_bytes(),
        defaults.max_working_bytes(),
        defaults.max_resident_metadata_bytes(),
    );
    let error = open(temporary.workspace_path(), index.clone(), limits)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::ResourceLimit { .. }));
    open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("failed bounded open did not mutate the Workspace");
}
