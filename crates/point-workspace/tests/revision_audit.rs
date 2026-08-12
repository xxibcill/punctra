//! Exact public Revision Audit behavior and resource contracts.

mod support;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use foundation_runtime::ProgressPhase;
use point_contracts::{ContentHash, PointId, WorldBounds};
use point_source::ReadBudget;
use point_workspace::{
    ClassificationTransition, CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId,
    RevisionAuditLimits, RevisionInfo, RevisionKind, SnapshotProvenance, Workspace, WorkspaceError,
    WorkspaceSchema, create, open,
};

use support::{classification_attribute, prepare_fixture, transform};

const REVISION_FOOTER_BYTES: u64 = 48;
const REVISION_HEADER_BYTES: u64 = 384;

#[derive(Clone, Copy, Debug)]
enum RevisionCorruption {
    Header,
    BlockHeader,
    Footer,
    TrailingByte,
}

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).expect("nonzero deterministic Operation Identity")
}

fn committed_revision(outcome: CommitOutcome) -> point_workspace::RevisionId {
    match outcome {
        CommitOutcome::Committed(receipt) => receipt.revision(),
        other => panic!("Edit unexpectedly did not commit: {other:?}"),
    }
}

fn commit_classification(
    workspace: &Workspace,
    ordinals: &[u64],
    value: u8,
    operation_byte: u8,
) -> point_workspace::RevisionId {
    let points = workspace
        .head()
        .select_point_ids(
            ordinals
                .iter()
                .copied()
                .map(|ordinal| PointId::new(workspace.source(), ordinal)),
            point_workspace::PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("audit target materializes");
    committed_revision(
        workspace
            .commit(
                CommitRequest::set_classification(operation(operation_byte), points, value),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("audit fixture Edit completes"),
    )
}

fn expected_footprint(ticks: &[[i64; 3]], ordinals: &[u64]) -> WorldBounds {
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for &ordinal in ordinals {
        let world = transform().world_f64(ticks[usize::try_from(ordinal).unwrap()]);
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(world[axis]);
            maximum[axis] = maximum[axis].max(world[axis]);
        }
    }
    WorldBounds::new(minimum, maximum).expect("fixture footprint is finite")
}

fn expected_point_id_hash(source: point_contracts::SourceId, ordinals: &[u64]) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"punctra-point-set-ids-v1");
    hasher.update(source.as_bytes());
    for ordinal in ordinals {
        hasher.update(&ordinal.to_le_bytes());
    }
    ContentHash::new(*hasher.finalize().as_bytes())
}

fn expected_content_hash(
    provenance: SnapshotProvenance,
    revision: RevisionInfo,
    ticks: &[[i64; 3]],
    rows: &[(u64, u8, u8)],
) -> ContentHash {
    let ordinals = rows
        .iter()
        .map(|(ordinal, _, _)| *ordinal)
        .collect::<Vec<_>>();
    let point_id_hash = expected_point_id_hash(provenance.source(), &ordinals);
    let mut row_hasher = blake3::Hasher::new();
    row_hasher.update(b"punctra-revision-audit-rows-v1");
    row_hasher.update(provenance.source().as_bytes());
    let mut position_hasher = blake3::Hasher::new();
    position_hasher.update(b"punctra-revision-audit-positions-v1");
    position_hasher.update(provenance.source().as_bytes());
    let mut transitions = BTreeMap::new();
    for &(ordinal, before, after) in rows {
        row_hasher.update(&ordinal.to_le_bytes());
        row_hasher.update(&[before, after]);
        position_hasher.update(&ordinal.to_le_bytes());
        for coordinate in ticks[usize::try_from(ordinal).unwrap()] {
            position_hasher.update(&coordinate.to_le_bytes());
        }
        *transitions.entry((before, after)).or_insert(0_u64) += 1;
    }
    let row_hash = ContentHash::new(*row_hasher.finalize().as_bytes());
    let position_hash = ContentHash::new(*position_hasher.finalize().as_bytes());
    let footprint = (!ordinals.is_empty()).then(|| expected_footprint(ticks, &ordinals));

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"punctra-revision-audit-content-v1");
    hasher.update(provenance.workspace().as_bytes());
    hasher.update(provenance.source().as_bytes());
    hasher.update(provenance.revision().as_bytes());
    hash_revision(&mut hasher, revision);
    hasher.update(&u64::try_from(rows.len()).unwrap().to_le_bytes());
    hasher.update(point_id_hash.as_bytes());
    hasher.update(row_hash.as_bytes());
    hasher.update(position_hash.as_bytes());
    hasher.update(&u64::try_from(transitions.len()).unwrap().to_le_bytes());
    for ((before, after), count) in transitions {
        hasher.update(&[before, after]);
        hasher.update(&count.to_le_bytes());
    }
    hash_footprint(&mut hasher, footprint);
    ContentHash::new(*hasher.finalize().as_bytes())
}

fn hash_revision(hasher: &mut blake3::Hasher, revision: RevisionInfo) {
    hasher.update(revision.id().as_bytes());
    if let Some(parent) = revision.parent() {
        hasher.update(&[1]);
        hasher.update(parent.as_bytes());
    } else {
        hasher.update(&[0]);
    }
    hasher.update(&revision.sequence().to_le_bytes());
    if let Some(operation) = revision.operation() {
        hasher.update(&[1]);
        hasher.update(operation.as_bytes());
    } else {
        hasher.update(&[0]);
    }
    match revision.kind() {
        RevisionKind::Root => {
            hasher.update(&[0]);
        }
        RevisionKind::SetClassification {
            value,
            changed_points,
        } => {
            hasher.update(&[1, value]);
            hasher.update(&changed_points.to_le_bytes());
        }
        RevisionKind::Revert {
            reverted_revision,
            changed_points,
        } => {
            hasher.update(&[2]);
            hasher.update(reverted_revision.as_bytes());
            hasher.update(&changed_points.to_le_bytes());
        }
    }
}

fn hash_footprint(hasher: &mut blake3::Hasher, footprint: Option<WorldBounds>) {
    if let Some(bounds) = footprint {
        hasher.update(&[1]);
        for coordinate in bounds.min().into_iter().chain(bounds.max()) {
            hasher.update(&coordinate.to_bits().to_le_bytes());
        }
    } else {
        hasher.update(&[0]);
    }
}

fn transition_map(transitions: &[ClassificationTransition]) -> BTreeMap<(u8, u8), u64> {
    transitions
        .iter()
        .map(|transition| {
            (
                (transition.before(), transition.after()),
                transition.count(),
            )
        })
        .collect()
}

fn classification_rows(classifications: &[u8], ordinals: &[u64], after: u8) -> Vec<(u64, u8, u8)> {
    ordinals
        .iter()
        .map(|&ordinal| {
            (
                ordinal,
                classifications[usize::try_from(ordinal).unwrap()],
                after,
            )
        })
        .collect()
}

fn inverse_rows(rows: &[(u64, u8, u8)]) -> Vec<(u64, u8, u8)> {
    rows.iter()
        .map(|&(ordinal, before, after)| (ordinal, after, before))
        .collect()
}

fn audit_limit_cases(defaults: RevisionAuditLimits) -> [RevisionAuditLimits; 6] {
    [
        RevisionAuditLimits::new(
            defaults.source_read_budget(),
            0,
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            defaults.source_read_budget(),
            defaults.max_revision_blocks(),
            0,
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            defaults.source_read_budget(),
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            1,
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            defaults.source_read_budget(),
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            0,
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            defaults.source_read_budget(),
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            0,
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            defaults.source_read_budget(),
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            0,
        ),
    ]
}

fn source_limit_cases(defaults: RevisionAuditLimits) -> [RevisionAuditLimits; 3] {
    let limited_payload = ReadBudget::new(defaults.source_read_budget().max_batch_points(), 23)
        .expect("nonzero undersized Source payload ceiling")
        .with_max_points(defaults.source_read_budget().max_points())
        .with_max_spans(defaults.source_read_budget().max_spans())
        .with_max_adapter_working_bytes(defaults.source_read_budget().max_adapter_working_bytes());
    [
        RevisionAuditLimits::new(
            defaults.source_read_budget().with_max_points(1),
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            defaults.source_read_budget().with_max_spans(0),
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
        RevisionAuditLimits::new(
            limited_payload,
            defaults.max_revision_blocks(),
            defaults.max_revision_bytes(),
            defaults.max_changed_points(),
            defaults.max_transition_entries(),
            defaults.max_result_bytes(),
            defaults.max_working_bytes(),
        ),
    ]
}

fn only_file(directory: impl AsRef<Path>) -> PathBuf {
    let mut paths = fs::read_dir(directory)
        .expect("read fixture directory")
        .map(|entry| entry.expect("read fixture entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths.len(), 1, "fixture has exactly one Revision");
    paths.pop().unwrap()
}

fn corrupt_last_revision_payload(path: &Path) {
    let offset = fs::metadata(path)
        .unwrap()
        .len()
        .checked_sub(REVISION_FOOTER_BYTES + 1)
        .expect("Revision has a nonempty block payload");
    corrupt_revision_byte(path, offset);
}

fn corrupt_revision(path: &Path, corruption: RevisionCorruption) {
    let file_bytes = fs::metadata(path).unwrap().len();
    match corruption {
        RevisionCorruption::Header => corrupt_revision_byte(path, 0),
        RevisionCorruption::BlockHeader => corrupt_revision_byte(path, REVISION_HEADER_BYTES),
        RevisionCorruption::Footer => corrupt_revision_byte(path, file_bytes - 1),
        RevisionCorruption::TrailingByte => {
            make_revision_writable(path);
            let mut file = OpenOptions::new()
                .append(true)
                .open(path)
                .expect("open Revision for trailing-byte corruption injection");
            file.write_all(&[0xa5]).unwrap();
            file.sync_all().unwrap();
        }
    }
}

fn corrupt_revision_byte(path: &Path, offset: u64) {
    make_revision_writable(path);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open Revision for corruption injection");
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x80;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
}

fn make_revision_writable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[cfg(unix)]
    permissions.set_mode(permissions.mode() | 0o200);
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).expect("unlock temporary Revision inode");
}

#[test]
fn root_audit_is_canonical_empty_and_survives_reopen() {
    let (temporary, index, ticks, _classifications) = prepare_fixture("audit-root", 257);
    let workspace = create(
        temporary.workspace_path(),
        index.clone(),
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("Workspace creates");
    let root = workspace.head().provenance().revision();

    let first = workspace
        .revision_audit(root, RevisionAuditLimits::default())
        .blocking_wait()
        .expect("root audit succeeds");
    assert_eq!(first.provenance(), *workspace.head().provenance());
    assert_eq!(first.revision().kind(), RevisionKind::Root);
    assert_eq!(first.edit_footprint(), None);
    assert!(first.transitions().is_empty());
    assert_eq!(first.changed_point_count(), 0);
    assert_eq!(
        first.point_id_hash(),
        expected_point_id_hash(workspace.source(), &[])
    );
    assert_eq!(
        first.content_hash(),
        expected_content_hash(
            first.provenance(),
            workspace.revision_info(root).unwrap(),
            &ticks,
            &[],
        )
    );
    assert!(first.retained_result_bytes() > 0);
    assert!(
        first.accounted_peak_working_bytes() <= RevisionAuditLimits::default().max_working_bytes()
    );
    let defaults = RevisionAuditLimits::default();
    let root_only_limits = RevisionAuditLimits::new(
        defaults
            .source_read_budget()
            .with_max_spans(0)
            .with_max_points(0),
        0,
        0,
        0,
        0,
        defaults.max_result_bytes(),
        defaults.max_working_bytes(),
    );
    assert_eq!(
        workspace
            .revision_audit(root, root_only_limits)
            .blocking_wait()
            .expect("zero input ceilings still admit the canonical root"),
        first
    );

    drop(workspace);
    let reopened = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("Workspace reopens");
    let second = reopened
        .revision_audit(root, RevisionAuditLimits::default())
        .blocking_wait()
        .expect("reopened root audit succeeds");
    assert_eq!(second, first);
}

#[test]
fn audit_reports_only_changed_rows_with_exact_footprint_and_sorted_transitions() {
    let (temporary, index, ticks, classifications) = prepare_fixture("audit-edit", 1_025);
    let workspace = create(
        temporary.workspace_path(),
        index,
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("Workspace creates");
    let one = classifications
        .iter()
        .enumerate()
        .filter_map(|(ordinal, &value)| (value == 1).then_some(ordinal as u64))
        .take(2)
        .collect::<Vec<_>>();
    let already_two = classifications
        .iter()
        .position(|&value| value == 2)
        .map(|ordinal| ordinal as u64)
        .unwrap();
    let three = classifications
        .iter()
        .position(|&value| value == 3)
        .map(|ordinal| ordinal as u64)
        .unwrap();
    let mut selected = vec![one[0], one[1], already_two, three];
    selected.sort_unstable();
    let revision = commit_classification(&workspace, &selected, 2, 1);
    let mut changed = vec![one[0], one[1], three];
    changed.sort_unstable();

    let audit = workspace
        .revision_audit(revision, RevisionAuditLimits::default())
        .blocking_wait()
        .expect("classification audit succeeds");
    assert_eq!(audit.provenance().revision(), revision);
    assert_eq!(audit.changed_point_count(), 3);
    assert_eq!(
        audit.edit_footprint(),
        Some(expected_footprint(&ticks, &changed))
    );
    assert_eq!(
        audit.point_id_hash(),
        expected_point_id_hash(workspace.source(), &changed)
    );
    assert_eq!(
        transition_map(audit.transitions()),
        BTreeMap::from([((1, 2), 2), ((3, 2), 1)])
    );
    assert!(
        audit
            .transitions()
            .windows(2)
            .all(|pair| (pair[0].before(), pair[0].after()) < (pair[1].before(), pair[1].after()))
    );
    let changed_rows = classification_rows(&classifications, &changed, 2);
    assert_eq!(
        audit.content_hash(),
        expected_content_hash(
            audit.provenance(),
            workspace.revision_info(revision).unwrap(),
            &ticks,
            &changed_rows,
        )
    );
}

#[test]
fn source_partitioning_is_irrelevant() {
    let (temporary, index, _ticks, classifications) = prepare_fixture("audit-partition", 2_049);
    let workspace = create(
        temporary.workspace_path(),
        index,
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("Workspace creates");
    let selected = classifications
        .iter()
        .enumerate()
        .filter_map(|(ordinal, &value)| matches!(value, 1 | 3).then_some(ordinal as u64))
        .take(37)
        .collect::<Vec<_>>();
    let revision = commit_classification(&workspace, &selected, 42, 2);
    let defaults = RevisionAuditLimits::default();
    let single_point_budget = ReadBudget::new(1, 24)
        .expect("one exact position fits")
        .with_max_points(defaults.max_changed_points())
        .with_max_spans(defaults.source_read_budget().max_spans())
        .with_max_adapter_working_bytes(defaults.source_read_budget().max_adapter_working_bytes());
    let partitioned_limits = RevisionAuditLimits::new(
        single_point_budget,
        defaults.max_revision_blocks(),
        defaults.max_revision_bytes(),
        defaults.max_changed_points(),
        defaults.max_transition_entries(),
        defaults.max_result_bytes(),
        defaults.max_working_bytes(),
    );
    let default_audit = workspace
        .revision_audit(revision, defaults)
        .blocking_wait()
        .expect("default audit succeeds");
    let partitioned_audit = workspace
        .revision_audit(revision, partitioned_limits)
        .blocking_wait()
        .expect("one-Point Source batches succeed");
    assert_eq!(partitioned_audit.provenance(), default_audit.provenance());
    assert_eq!(partitioned_audit.revision(), default_audit.revision());
    assert_eq!(
        partitioned_audit.edit_footprint(),
        default_audit.edit_footprint()
    );
    assert_eq!(partitioned_audit.transitions(), default_audit.transitions());
    assert_eq!(
        partitioned_audit.point_id_hash(),
        default_audit.point_id_hash()
    );
    assert_eq!(
        partitioned_audit.content_hash(),
        default_audit.content_hash()
    );
}

#[test]
fn revert_inverts_audit_and_historical_revision_survives_reopen() {
    let (temporary, index, ticks, classifications) = prepare_fixture("audit-revert", 2_049);
    let workspace = create(
        temporary.workspace_path(),
        index.clone(),
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("Workspace creates");
    let selected = classifications
        .iter()
        .enumerate()
        .filter_map(|(ordinal, &value)| matches!(value, 1 | 3).then_some(ordinal as u64))
        .take(37)
        .collect::<Vec<_>>();
    let revision = commit_classification(&workspace, &selected, 42, 3);
    let committed_rows = classification_rows(&classifications, &selected, 42);
    let defaults = RevisionAuditLimits::default();
    let default_audit = workspace
        .revision_audit(revision, defaults)
        .blocking_wait()
        .expect("classification audit succeeds");
    let revert = committed_revision(
        workspace
            .commit(
                CommitRequest::revert_head(operation(4), revision),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("Revert completes"),
    );
    let reverted = workspace
        .revision_audit(revert, defaults)
        .blocking_wait()
        .expect("Revert audit succeeds");
    assert_eq!(reverted.edit_footprint(), default_audit.edit_footprint());
    assert_eq!(reverted.point_id_hash(), default_audit.point_id_hash());
    assert_ne!(reverted.content_hash(), default_audit.content_hash());
    let expected_inverse = transition_map(default_audit.transitions())
        .into_iter()
        .map(|((before, after), count)| ((after, before), count))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(transition_map(reverted.transitions()), expected_inverse);
    assert_eq!(
        reverted.edit_footprint(),
        Some(expected_footprint(&ticks, &selected))
    );
    let reverted_rows = inverse_rows(&committed_rows);
    assert_eq!(
        reverted.content_hash(),
        expected_content_hash(
            reverted.provenance(),
            workspace.revision_info(revert).unwrap(),
            &ticks,
            &reverted_rows,
        )
    );
    assert_eq!(
        workspace
            .revision_audit(revision, defaults)
            .blocking_wait()
            .expect("historical audit remains rebuildable after Revert"),
        default_audit
    );
    drop(workspace);
    let reopened = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .expect("Workspace with later Revert reopens");
    assert_eq!(
        reopened
            .revision_audit(revision, defaults)
            .blocking_wait()
            .expect("historical audit survives reopen after later Revision"),
        default_audit
    );
}

#[test]
fn audit_detects_revision_payload_corruption_after_workspace_open() {
    let (temporary, index, _ticks, _classifications) =
        prepare_fixture("audit-live-corruption", 257);
    let workspace = create(
        temporary.workspace_path(),
        index,
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("Workspace creates");
    let revision = commit_classification(&workspace, &[8, 13, 21], 55, 8);
    workspace
        .revision_audit(revision, RevisionAuditLimits::default())
        .blocking_wait()
        .expect("uncorrupted Revision audits");

    let revision_file = only_file(temporary.workspace_path().join("revisions"));
    corrupt_last_revision_payload(&revision_file);
    let job = workspace.revision_audit(revision, RevisionAuditLimits::default());
    let handle = job.handle();
    assert!(matches!(
        job.blocking_wait(),
        Err(WorkspaceError::Corrupt { .. })
    ));
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn audit_detects_all_revision_structure_corruption_after_workspace_open() {
    for corruption in [
        RevisionCorruption::Header,
        RevisionCorruption::BlockHeader,
        RevisionCorruption::Footer,
        RevisionCorruption::TrailingByte,
    ] {
        let label = format!("audit-live-structure-corruption-{corruption:?}");
        let (temporary, index, _ticks, _classifications) = prepare_fixture(&label, 257);
        let workspace = create(
            temporary.workspace_path(),
            index,
            WorkspaceSchema::new(classification_attribute()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("Workspace creates");
        let revision = commit_classification(&workspace, &[8, 13, 21], 55, 9);
        let revision_file = only_file(temporary.workspace_path().join("revisions"));
        corrupt_revision(&revision_file, corruption);

        let job = workspace.revision_audit(revision, RevisionAuditLimits::default());
        let handle = job.handle();
        assert!(
            matches!(job.blocking_wait(), Err(WorkspaceError::Corrupt { .. })),
            "{corruption:?} corruption must fail closed"
        );
        assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
    }
}

#[test]
fn every_audit_limit_and_cancellation_prevent_publication() {
    let (temporary, index, _ticks, classifications) = prepare_fixture("audit-limits", 8_193);
    let workspace = create(
        temporary.workspace_path(),
        index,
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("Workspace creates");
    let selected = classifications
        .iter()
        .enumerate()
        .filter_map(|(ordinal, &value)| (value != 42).then_some(ordinal as u64))
        .take(4_097)
        .collect::<Vec<_>>();
    let revision = commit_classification(&workspace, &selected, 42, 4);
    let defaults = RevisionAuditLimits::default();

    for limits in audit_limit_cases(defaults)
        .into_iter()
        .chain(source_limit_cases(defaults))
    {
        assert!(matches!(
            workspace.revision_audit(revision, limits).blocking_wait(),
            Err(WorkspaceError::ResourceLimit { .. } | WorkspaceError::Source { .. })
        ));
    }

    let job = workspace.revision_audit(revision, defaults);
    let handle = job.handle();
    handle.cancel();
    assert!(matches!(
        job.blocking_wait(),
        Err(WorkspaceError::Cancelled)
    ));
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}
