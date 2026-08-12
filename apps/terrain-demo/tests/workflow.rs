//! Public-facade evidence for durable workflow restart and reconciliation.

mod support;

use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use point_contracts::{AttributeId, PointId};
use point_index::PrepareLimits;
use point_terrain::{
    CheckPoint, CheckPointId, CheckPointLimits, LandXmlLimits, LandXmlOptions, TerrainLimits,
    TerrainRecipe,
};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, OperationResolution,
    PointRowLimits, PointSetLimits, RevisionAuditLimits, Workspace, WorkspaceSchema, create, open,
};
use serde_json::Value;
use support::{
    RevisionDirectoryBlocker, TestDirectory, journal_frame_ends, overwrite_and_sync,
    restore_journal_prefix, semantic_report_projection, write_las_family_fixture,
};
use terrain_demo::{
    WorkflowLimits, WorkflowPaths, WorkflowPhase, WorkflowReceipt, WorkflowRunId,
    WorkflowRunIntent, inspect_run, resume_run, start_run,
};

const GROUND: u8 = 2;
const NON_GROUND: u8 = 1;
const CLASSIFICATION_ATTRIBUTE: u32 = 6;
const RETURN_NUMBER_ATTRIBUTE: u32 = 2;
const EXPECTED_FRAME_COUNT: usize = 8;
const EXPECTED_PHASES: [WorkflowPhase; EXPECTED_FRAME_COUNT] = [
    WorkflowPhase::IntentRecorded,
    WorkflowPhase::RevisionResolved,
    WorkflowPhase::AuditObserved,
    WorkflowPhase::SurfacesObserved,
    WorkflowPhase::QaObserved,
    WorkflowPhase::ExportEnsured,
    WorkflowPhase::ReportEnsured,
    WorkflowPhase::Complete,
];

#[test]
fn every_checkpoint_prefix_resumes_to_one_revision_and_the_same_report() {
    let fixture = WorkflowFixture::new("prefixes", "las", 64, 11);
    let immutable_source = fs::read(&fixture.source).expect("read immutable Source fixture");

    let expected = fixture.start();
    let expected_report = fixture.report_bytes();
    let expected_landxml = fixture.landxml_bytes();
    let complete_journal = fixture.journal_bytes();
    let frame_ends = journal_frame_ends(&complete_journal).expect("parse sealed workflow journal");
    assert_eq!(frame_ends.len(), EXPECTED_FRAME_COUNT);
    let caller_owned = fixture.run_root.join("caller-notes.txt");
    overwrite_and_sync(&caller_owned, b"preserve me").expect("create unknown caller-owned child");

    for (prefix, end) in frame_ends.iter().copied().enumerate() {
        restore_journal_prefix(&fixture.journal(), &complete_journal, end)
            .expect("durably restore completed-run journal prefix");
        let before = inspect_run(&fixture.run_root, WorkflowLimits::default())
            .expect("inspect verified journal prefix");
        assert_eq!(before.phase(), EXPECTED_PHASES[prefix]);
        assert_eq!(before.is_complete(), prefix + 1 == EXPECTED_FRAME_COUNT);

        let resumed = fixture.resume();
        assert_eq!(resumed, expected, "prefix {} receipt", prefix + 1);
        assert_eq!(fixture.report_bytes(), expected_report);
        assert_eq!(fixture.landxml_bytes(), expected_landxml);
        assert_eq!(
            journal_frame_ends(&fixture.journal_bytes())
                .expect("parse repaired and completed journal")
                .len(),
            EXPECTED_FRAME_COUNT,
        );
        fixture.assert_single_operation_revision(expected);
        assert_eq!(fs::read(&caller_owned).unwrap(), b"preserve me");
    }

    assert_eq!(
        fs::read(&fixture.source).expect("reread immutable Source fixture"),
        immutable_source,
    );
}

#[test]
fn torn_suffix_repairs_but_version_reserved_and_sequence_corruption_do_not() {
    let fixture = WorkflowFixture::new("journal-variants", "las", 64, 121);
    fixture.start();
    let complete = fixture.journal_bytes();

    let mut torn = complete.clone();
    torn.extend_from_slice(b"torn final frame prefix");
    overwrite_and_sync(&fixture.journal(), &torn).expect("install torn final suffix");
    let status = inspect_run(&fixture.run_root, WorkflowLimits::default())
        .expect("reader repairs a provably torn final suffix");
    assert!(status.is_complete());
    assert_eq!(fixture.journal_bytes(), complete);

    for (label, offset, replacement) in [
        ("disk version", 8_usize, 2_u32.to_le_bytes()),
        ("reserved header", 20, 1_u32.to_le_bytes()),
        ("first sequence", 80 + 8, 1_u32.to_le_bytes()),
    ] {
        let mut corrupt = complete.clone();
        corrupt[offset..offset + replacement.len()].copy_from_slice(&replacement);
        overwrite_and_sync(&fixture.journal(), &corrupt)
            .unwrap_or_else(|error| panic!("install {label} corruption: {error}"));
        let Err(error) = inspect_run(&fixture.run_root, WorkflowLimits::default()) else {
            panic!("{label} corruption must fail closed");
        };
        assert_eq!(error.code(), "PWF_JOURNAL_CORRUPT", "{label}: {error}");
        assert_eq!(fixture.journal_bytes(), corrupt);
    }
    overwrite_and_sync(&fixture.journal(), &complete).expect("restore complete journal");
}

#[test]
fn exact_report_reconciles_and_a_conflict_is_never_overwritten() {
    let fixture = WorkflowFixture::new("report", "las", 64, 21);
    let expected = fixture.start();
    let report = fixture.report_bytes();
    let complete_journal = fixture.journal_bytes();
    let frame_ends = journal_frame_ends(&complete_journal).expect("parse complete journal");
    let export_prefix = frame_ends[5];

    restore_journal_prefix(&fixture.journal(), &complete_journal, export_prefix)
        .expect("restore ExportEnsured prefix");
    assert_eq!(fixture.resume(), expected, "exact report must reconcile");
    assert_eq!(fixture.report_bytes(), report);

    restore_journal_prefix(&fixture.journal(), &complete_journal, export_prefix)
        .expect("restore ExportEnsured prefix for conflict");
    let conflict = br#"{"caller_owned":"different"}\n"#;
    overwrite_and_sync(&fixture.report(), conflict).expect("install caller-owned report conflict");
    let journal_before = fixture.journal_bytes();
    let error = resume_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("conflicting report must fail closed");
    assert_eq!(error.code(), "PWF_OUTPUT_CONFLICT");
    assert!(error.to_string().contains("remove or rename"));
    assert_eq!(fixture.report_bytes(), conflict);
    assert_eq!(fixture.journal_bytes(), journal_before);
    fixture.assert_single_operation_revision(expected);

    overwrite_and_sync(&fixture.report(), &report).expect("restore exact canonical report");
    assert_eq!(fixture.resume(), expected);
}

#[test]
fn journal_corruption_limits_locking_and_path_binding_fail_closed() {
    let fixture = WorkflowFixture::new("fail-closed", "las", 64, 31);
    let expected = fixture.start();
    let complete_journal = fixture.journal_bytes();

    let limit = WorkflowLimits::default().with_max_journal_bytes(
        u64::try_from(complete_journal.len() - 1).expect("journal size fits u64"),
    );
    let error = inspect_run(&fixture.run_root, limit).expect_err("journal byte ceiling must bind");
    assert_eq!(error.code(), "PWF_RESOURCE_LIMIT");
    assert_eq!(fixture.journal_bytes(), complete_journal);

    let mut corrupt = complete_journal.clone();
    *corrupt.last_mut().expect("journal is nonempty") ^= 1;
    overwrite_and_sync(&fixture.journal(), &corrupt).expect("install corrupt complete frame");
    let error = inspect_run(&fixture.run_root, WorkflowLimits::default())
        .expect_err("complete frame corruption must fail closed");
    assert_eq!(error.code(), "PWF_JOURNAL_CORRUPT");
    assert_eq!(fixture.journal_bytes(), corrupt);
    overwrite_and_sync(&fixture.journal(), &complete_journal).expect("restore valid journal");

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.lock())
        .expect("open workflow lock file");
    lock.try_lock().expect("acquire independent exclusive lock");
    let error = inspect_run(&fixture.run_root, WorkflowLimits::default())
        .expect_err("concurrent inspection must not pass the Run lock");
    assert_eq!(error.code(), "PWF_IO");
    assert!(error.to_string().contains("lock"));
    drop(lock);
    assert!(inspect_run(&fixture.run_root, WorkflowLimits::default()).is_ok());

    overwrite_and_sync(&fixture.lock(), b"unexpected lock payload")
        .expect("install nonempty workflow lock file");
    let error = inspect_run(&fixture.run_root, WorkflowLimits::default())
        .expect_err("nonempty run.lock must fail its fixed schema");
    assert_eq!(error.code(), "PWF_IO");
    assert_eq!(error.stage(), "lock");
    overwrite_and_sync(&fixture.lock(), b"").expect("restore empty workflow lock file");
    assert!(inspect_run(&fixture.run_root, WorkflowLimits::default()).is_ok());

    let aggregate_limit = WorkflowLimits::default().with_max_aggregate_working_bytes(1);
    let error = inspect_run(&fixture.run_root, aggregate_limit)
        .expect_err("aggregate ceiling must bind before journal inspection");
    assert_eq!(error.code(), "PWF_RESOURCE_LIMIT");
    assert_eq!(fixture.journal_bytes(), complete_journal);

    let alternate_source = fixture
        .directory
        .path()
        .join("same-bytes-different-path.las");
    fs::copy(&fixture.source, &alternate_source).expect("copy byte-identical Source");
    let mismatched = WorkflowPaths::new(
        alternate_source,
        fixture.index.clone(),
        fixture.workspace.clone(),
        fixture.run_root.clone(),
    );
    let journal_before = fixture.journal_bytes();
    let error = resume_run(
        mismatched,
        fixture.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("different path binding must not resume");
    assert_eq!(error.code(), "PWF_JOURNAL_CONFLICT");
    assert_eq!(fixture.journal_bytes(), journal_before);
    fixture.assert_single_operation_revision(expected);
}

#[test]
fn aggregate_limit_preflights_before_rebuildable_index_publication() {
    let fixture = WorkflowFixture::new("aggregate-preflight", "las", 64, 30);
    fs::remove_file(&fixture.index).expect("remove rebuildable fixture index");
    assert!(!fixture.index.exists());

    let error = start_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default().with_max_aggregate_working_bytes(1),
    )
    .blocking_wait()
    .expect_err("aggregate ceiling must fail before index preparation");

    assert_eq!(error.code(), "PWF_RESOURCE_LIMIT");
    assert_eq!(error.stage(), "source");
    assert!(!fixture.index.exists());
    assert!(!fixture.journal().exists());
    fixture.assert_single_operation_revision(fixture.start());
}

#[test]
fn generated_las_and_laz_have_equal_semantic_projection_without_hiding_identity() {
    let plain_fixture = WorkflowFixture::new("projection-las", "las", 100, 41);
    let compressed_fixture = WorkflowFixture::new("projection-laz", "laz", 100, 51);
    let plain_source = fs::read(&plain_fixture.source).expect("read generated LAS bytes");
    let compressed_source = fs::read(&compressed_fixture.source).expect("read generated LAZ bytes");

    plain_fixture.start();
    compressed_fixture.start();
    let plain_report = plain_fixture.report_bytes();
    let compressed_report = compressed_fixture.report_bytes();
    let plain_json: Value = serde_json::from_slice(&plain_report).expect("parse LAS audit report");
    let compressed_json: Value =
        serde_json::from_slice(&compressed_report).expect("parse LAZ audit report");

    assert_ne!(
        plain_source, compressed_source,
        "LAS and LAZ storage encodings differ"
    );
    assert_ne!(
        plain_json["identities"]["source"], compressed_json["identities"]["source"],
        "raw-file-derived Source identities must remain honest",
    );
    assert_ne!(
        plain_report, compressed_report,
        "identity-bearing reports must differ"
    );
    assert_eq!(
        plain_json["request"]["semantic_results_hash"],
        compressed_json["request"]["semantic_results_hash"],
        "the named source-independent semantic digest must match",
    );
    assert_eq!(
        semantic_report_projection(&plain_report).expect("project LAS report"),
        semantic_report_projection(&compressed_report).expect("project LAZ report"),
    );
    assert_eq!(
        plain_fixture.landxml_bytes(),
        compressed_fixture.landxml_bytes(),
        "coordinate-domain LandXML is storage-encoding independent",
    );
    assert_eq!(fs::read(&plain_fixture.source).unwrap(), plain_source);
    assert_eq!(
        fs::read(&compressed_fixture.source).unwrap(),
        compressed_source
    );
}

#[test]
fn cancelling_the_parent_run_never_publishes_a_false_complete_checkpoint() {
    let fixture = WorkflowFixture::new("cancel", "las", 10_000, 61);
    let source_before = fs::read(&fixture.source).expect("read cancellation Source fixture");
    let job = start_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    );
    job.handle().cancel();
    let error = job
        .blocking_wait()
        .expect_err("immediate parent cancellation must stop the workflow");
    assert_eq!(error.code(), "PWF_CANCELLED");

    if fixture.journal().exists() {
        let status = inspect_run(&fixture.run_root, WorkflowLimits::default())
            .expect("cancelled Run journal remains inspectable");
        assert!(
            !status.is_complete(),
            "a cancelled workflow must not claim Complete",
        );
        let receipt = fixture.resume();
        fixture.assert_single_operation_revision(receipt);
    } else {
        assert!(
            !fixture.report().exists(),
            "pre-Intent cancellation must not publish a report",
        );
        assert!(
            !fixture.run_root.join("terrain.xml").exists(),
            "pre-Intent cancellation must not publish LandXML",
        );
    }
    assert_eq!(fs::read(&fixture.source).unwrap(), source_before);
}

#[test]
fn run_root_validation_reports_known_intent_identities() {
    let fixture = WorkflowFixture::new("run-root-identities", "las", 64, 62);
    fs::remove_dir(&fixture.run_root).expect("remove empty caller-owned Run root");

    let error = start_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("a missing Run root must fail before publication");

    assert_eq!(error.stage(), "validate");
    assert_eq!(error.run(), Some(fixture.intent.run()));
    assert_eq!(error.operation(), Some(fixture.intent.operation()));
    assert_eq!(error.revision(), Some(fixture.intent.baseline_revision()));
}

#[test]
fn dropping_an_active_workflow_never_publishes_complete_and_can_resume() {
    let fixture = WorkflowFixture::new("drop-active", "las", 100_000, 63);
    let source_before = fs::read(&fixture.source).expect("read drop-test Source fixture");
    let job = start_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    );
    let handle = job.handle();
    wait_for_journal_frames(&fixture.journal(), 3);

    drop(job);

    assert!(handle.token().is_cancelled());
    let status = wait_for_unlocked_status(&fixture.run_root);
    assert!(
        !status.is_complete(),
        "dropping an active workflow must not publish Complete",
    );
    let receipt = fixture.resume();
    fixture.assert_single_operation_revision(receipt);
    assert_eq!(
        fs::read(&fixture.source).expect("reread drop-test Source fixture"),
        source_before,
    );
}

#[test]
fn report_limits_and_landxml_conflicts_resume_without_a_second_revision() {
    let limited = WorkflowFixture::new("report-limit", "las", 64, 71);
    let error = start_run(
        limited.paths.clone(),
        limited.intent.clone(),
        WorkflowLimits::default().with_max_report_bytes(1),
    )
    .blocking_wait()
    .expect_err("one-byte report ceiling must bind before report publication");
    assert_eq!(error.code(), "PWF_RESOURCE_LIMIT");
    assert!(!limited.report().exists());
    assert!(limited.run_root.join("terrain.xml").is_file());
    let status = inspect_run(&limited.run_root, WorkflowLimits::default())
        .expect("inspect Run stopped at the report limit");
    assert_eq!(status.phase(), WorkflowPhase::ExportEnsured);
    assert!(!status.is_complete());
    let recovered = limited.resume();
    limited.assert_single_operation_revision(recovered);

    let conflicting = WorkflowFixture::new("landxml-conflict", "las", 64, 81);
    let caller_bytes = b"caller-owned conflicting LandXML";
    overwrite_and_sync(&conflicting.run_root.join("terrain.xml"), caller_bytes)
        .expect("install caller-owned LandXML conflict");
    let error = start_run(
        conflicting.paths.clone(),
        conflicting.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("conflicting LandXML must fail closed");
    assert_eq!(error.code(), "PWF_OUTPUT_CONFLICT");
    assert_eq!(
        fs::read(conflicting.run_root.join("terrain.xml")).unwrap(),
        caller_bytes,
    );
    let status = inspect_run(&conflicting.run_root, WorkflowLimits::default())
        .expect("inspect Run stopped before LandXML checkpoint");
    assert_eq!(status.phase(), WorkflowPhase::QaObserved);
    assert!(!status.is_complete());
    fs::remove_file(conflicting.run_root.join("terrain.xml"))
        .expect("caller removes its conflicting target");
    let recovered = conflicting.resume();
    conflicting.assert_single_operation_revision(recovered);
}

#[test]
fn public_limit_families_fail_as_resource_limits_and_recover_without_source_mutation() {
    let defaults = WorkflowLimits::default();
    let cases = [
        (
            "prepare-limit",
            141,
            defaults.with_prepare_limits(one_byte_index_artifact_limits()),
        ),
        (
            "open-limit",
            143,
            defaults.with_open_limits(one_byte_workspace_manifest_limits()),
        ),
        (
            "row-limit",
            145,
            defaults.with_point_row_limits(one_output_point_row_limits()),
        ),
        ("intent-limit", 147, defaults.with_intent_count_limits(1, 2)),
        (
            "selection-limit",
            149,
            defaults.with_selection_limits(one_input_point_selection_limits()),
        ),
        (
            "commit-limit",
            151,
            defaults.with_commit_limits(one_selected_point_commit_limits()),
        ),
        (
            "audit-limit",
            153,
            defaults.with_audit_limits(one_changed_point_audit_limits()),
        ),
        (
            "terrain-limit",
            155,
            defaults.with_terrain_limits(sixty_three_input_point_terrain_limits()),
        ),
        (
            "qa-limit",
            157,
            defaults.with_check_point_limits(one_check_point_limits()),
        ),
        (
            "landxml-limit",
            159,
            defaults.with_landxml_limits(one_byte_landxml_limits()),
        ),
        ("envelope-limit", 161, defaults.with_envelope_limits(0, 1)),
        (
            "aggregate-limit",
            163,
            defaults.with_max_aggregate_working_bytes(1),
        ),
    ];

    let failures: Vec<_> = cases
        .into_iter()
        .filter_map(|(label, identity, limits)| resource_limit_problem(label, identity, &limits))
        .collect();
    assert!(
        failures.is_empty(),
        "public limit classification failures:\n{}",
        failures.join("\n"),
    );
}

#[test]
fn stale_head_and_a_differently_bound_rejection_do_not_mutate_the_run() {
    let stale = WorkflowFixture::new("stale-head", "las", 64, 91);
    let workspace = stale.open_workspace();
    let baseline = workspace.head();
    let point_set = baseline
        .select_point_ids(
            [PointId::new(workspace.source(), 11)],
            point_workspace::PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("materialize stale-head edit Point");
    let other_operation = OperationId::from_bytes([93; 16]).expect("valid other Operation ID");
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(other_operation, point_set, NON_GROUND),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("commit head-advancing edit");
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    drop(baseline);
    drop(workspace);
    let error = start_run(
        stale.paths.clone(),
        stale.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("stale baseline must fail before Intent publication");
    assert_eq!(error.code(), "PWF_STALE_BASELINE");
    assert!(!stale.journal().exists());

    let rejected = WorkflowFixture::new("recorded-rejection", "las", 64, 101);
    let workspace = rejected.open_workspace();
    let point_set = workspace
        .head()
        .select_point_ids(
            [
                PointId::new(workspace.source(), 9),
                PointId::new(workspace.source(), 10),
            ],
            point_workspace::PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("materialize rejection fixture Points");
    let intended_operation = rejected.intent.operation();
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(intended_operation, point_set, GROUND),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("publish definitive no-change rejection");
    assert!(matches!(outcome, CommitOutcome::Rejected(_)));
    assert_eq!(
        workspace.head().provenance().revision(),
        rejected.intent.baseline_revision(),
    );
    drop(workspace);

    let error = start_run(
        rejected.paths.clone(),
        rejected.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("same Operation bound to a different request must conflict");
    assert_eq!(error.code(), "PWF_JOURNAL_CONFLICT");
    let status = inspect_run(&rejected.run_root, WorkflowLimits::default())
        .expect("inspect rejected Run Intent");
    assert_eq!(status.phase(), WorkflowPhase::IntentRecorded);
    assert!(!status.is_complete());
}

#[test]
fn changed_source_and_workspace_identities_are_named_before_run_mutation() {
    let fixture = WorkflowFixture::new("identity-mismatch", "las", 64, 111);
    fixture.start();
    let source_bytes = fs::read(&fixture.source).expect("read expected Source bytes");
    let journal = fixture.journal_bytes();

    fs::remove_file(&fixture.source).expect("remove test-owned expected Source");
    write_las_family_fixture(&fixture.source, 65).expect("write changed Source meaning");
    let error = resume_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("changed Source must fail before index or Run mutation");
    assert_eq!(error.code(), "PWF_SOURCE_MISMATCH", "{error}");
    assert!(
        error
            .to_string()
            .contains("restore the expected immutable Source")
    );
    assert_eq!(fixture.journal_bytes(), journal);

    overwrite_and_sync(&fixture.source, &source_bytes).expect("restore exact immutable Source");
    let expected_workspace = fixture.directory.path().join("expected-workspace.pcw");
    fs::rename(&fixture.workspace, &expected_workspace).expect("retain expected Workspace");
    let source = source_las::open(&fixture.source)
        .blocking_wait()
        .expect("reopen restored Source");
    let index = point_index::prepare(source, &fixture.index, PrepareLimits::default())
        .blocking_wait()
        .expect("reopen expected index");
    drop(
        create(
            &fixture.workspace,
            index,
            WorkspaceSchema::new(AttributeId::new(CLASSIFICATION_ATTRIBUTE).unwrap()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create different Workspace lineage at the bound path"),
    );
    let error = resume_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("changed Workspace lineage must fail before Run mutation");
    assert_eq!(error.code(), "PWF_WORKSPACE_MISMATCH", "{error}");
    assert_eq!(fixture.journal_bytes(), journal);
}

#[test]
fn non_las_classification_workspace_is_rejected_before_run_or_workspace_mutation() {
    let fixture = WorkflowFixture::new_with_classification_attribute(
        "wrong-workspace-schema",
        "las",
        64,
        121,
        RETURN_NUMBER_ATTRIBUTE,
    );
    let baseline = fixture.open_workspace().head().provenance().revision();

    let error = start_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default(),
    )
    .blocking_wait()
    .expect_err("a different U8 editable Attribute must be rejected");

    assert_eq!(error.code(), "PWF_WORKSPACE_MISMATCH", "{error}");
    assert!(
        error
            .to_string()
            .contains("expected LAS classification Attribute 6"),
        "{error}"
    );
    assert!(!fixture.journal().exists());
    assert!(!fixture.report().exists());
    assert!(!fixture.run_root.join("terrain.xml").exists());
    let workspace = fixture.open_workspace();
    assert_eq!(workspace.head().provenance().revision(), baseline);
    assert!(matches!(
        workspace
            .resolve_operation(fixture.intent.operation())
            .expect("resolve untouched Operation"),
        OperationResolution::NotRecorded
    ));
}

#[test]
fn retryable_workspace_intent_resumes_with_the_recorded_operation() {
    let fixture = WorkflowFixture::new("retryable", "las", 64, 131);
    let source_bytes = fs::read(&fixture.source).expect("read retryable Source fixture");
    let error = start_run(
        fixture.paths.clone(),
        fixture.intent.clone(),
        WorkflowLimits::default().with_selection_limits(one_input_point_selection_limits()),
    )
    .blocking_wait()
    .expect_err("selection ceiling stops after durable Intent");
    assert_eq!(error.code(), "PWF_RESOURCE_LIMIT", "{error}");
    let status = inspect_run(&fixture.run_root, WorkflowLimits::default())
        .expect("inspect Intent-only workflow Run");
    assert_eq!(status.phase(), WorkflowPhase::IntentRecorded);
    assert!(!status.is_complete());

    let workspace = fixture.open_workspace();
    let points = workspace
        .head()
        .select_point_ids(
            [
                PointId::new(workspace.source(), 9),
                PointId::new(workspace.source(), 10),
            ],
            PointSetLimits::default(),
        )
        .blocking_wait()
        .expect("materialize retryable Operation Points");
    let operation = fixture.intent.operation();
    let obstruction = RevisionDirectoryBlocker::install(&fixture.workspace)
        .expect("replace test-owned revisions directory after Workspace open");
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(operation, points, NON_GROUND),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("post-ready publication failure preserves certainty in the outcome");
    assert!(
        matches!(outcome, CommitOutcome::Indeterminate(_)),
        "Revision publication obstruction follows durable ready intent",
    );
    obstruction
        .restore()
        .expect("restore test-owned revisions directory");
    drop(workspace);

    let reopened = fixture.open_workspace();
    let resolution = reopened
        .resolve_operation(operation)
        .expect("resolve retained Operation after reopen");
    assert!(matches!(resolution, OperationResolution::Retryable(_)));
    drop(reopened);

    let receipt = fixture.resume();
    assert_eq!(receipt.operation(), fixture.intent.operation());
    fixture.assert_single_operation_revision(receipt);
    assert_eq!(fs::read(&fixture.source).unwrap(), source_bytes);
}

struct WorkflowFixture {
    directory: TestDirectory,
    source: PathBuf,
    index: PathBuf,
    workspace: PathBuf,
    run_root: PathBuf,
    paths: WorkflowPaths,
    intent: WorkflowRunIntent,
}

impl WorkflowFixture {
    fn new(label: &str, extension: &str, point_count: usize, identity: u8) -> Self {
        Self::new_with_classification_attribute(
            label,
            extension,
            point_count,
            identity,
            CLASSIFICATION_ATTRIBUTE,
        )
    }

    fn new_with_classification_attribute(
        label: &str,
        extension: &str,
        point_count: usize,
        identity: u8,
        classification_attribute: u32,
    ) -> Self {
        let directory = TestDirectory::new(label).expect("create workflow fixture directory");
        let source = directory.path().join(format!("fixture.{extension}"));
        let index = directory.path().join("fixture.pidx");
        let workspace = directory.path().join("fixture.pcw");
        let run_root = directory.path().join("run");
        fs::create_dir(&run_root).expect("create caller-owned Run root");
        write_las_family_fixture(&source, point_count).expect("write generated LAS-family Source");

        let source_handle = source_las::open(&source)
            .blocking_wait()
            .expect("open generated Source");
        let prepared = point_index::prepare(source_handle, &index, PrepareLimits::default())
            .blocking_wait()
            .expect("prepare generated Source index");
        let workspace_handle = create(
            &workspace,
            prepared,
            WorkspaceSchema::new(
                AttributeId::new(classification_attribute)
                    .expect("Workspace classification Attribute ID is nonzero"),
            ),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create baseline Workspace");
        let baseline = workspace_handle.head().provenance().revision();
        drop(workspace_handle);

        let paths = WorkflowPaths::new(&source, &index, &workspace, &run_root);
        let intent = WorkflowRunIntent::new(
            WorkflowRunId::new([identity; 16]).expect("nonzero Workflow Run ID"),
            OperationId::from_bytes([identity.wrapping_add(1); 16])
                .expect("nonzero Workspace Operation ID"),
            baseline,
            [9_u64, 10],
            NON_GROUND,
            TerrainRecipe::new(GROUND),
            [
                CheckPoint::new(
                    CheckPointId::new(1).expect("nonzero Check Point ID"),
                    [500_002.0, 4_600_002.0, 121.6],
                )
                .expect("finite sampled Check Point"),
                CheckPoint::new(
                    CheckPointId::new(2).expect("nonzero Check Point ID"),
                    [600_000.0, 4_600_000.0, 120.0],
                )
                .expect("finite gap Check Point"),
            ],
            LandXmlOptions::metric_metres(
                "Punctra Technical Alpha Surface",
                "2026-08-10",
                "00:00:00Z",
            )
            .expect("valid deterministic LandXML options")
            .assert_coordinates_are_metric_metres(),
        )
        .expect("construct bounded workflow intent");
        Self {
            directory,
            source,
            index,
            workspace,
            run_root,
            paths,
            intent,
        }
    }

    fn start(&self) -> WorkflowReceipt {
        start_run(
            self.paths.clone(),
            self.intent.clone(),
            WorkflowLimits::default(),
        )
        .blocking_wait()
        .expect("complete new workflow Run")
    }

    fn resume(&self) -> WorkflowReceipt {
        resume_run(
            self.paths.clone(),
            self.intent.clone(),
            WorkflowLimits::default(),
        )
        .blocking_wait()
        .expect("resume workflow Run")
    }

    fn assert_single_operation_revision(&self, receipt: WorkflowReceipt) {
        let workspace = self.open_workspace();
        let head = workspace.head().provenance().revision();
        assert_eq!(head, receipt.revision());
        let info = workspace
            .revision_info(head)
            .expect("read workflow Revision facts");
        assert_eq!(info.sequence(), 1, "only Root and one edit Revision exist");
        assert_eq!(info.operation(), Some(receipt.operation()),);
        assert_eq!(receipt.revision(), info.id(),);
    }

    fn open_workspace(&self) -> Workspace {
        let source = source_las::open(&self.source)
            .blocking_wait()
            .expect("reopen immutable Source");
        let index = point_index::prepare(source, &self.index, PrepareLimits::default())
            .blocking_wait()
            .expect("reopen complete index");
        open(&self.workspace, index, OpenLimits::default())
            .blocking_wait()
            .expect("reopen Workspace after workflow")
    }

    fn journal(&self) -> PathBuf {
        self.run_root.join("run.pwf")
    }

    fn lock(&self) -> PathBuf {
        self.run_root.join("run.lock")
    }

    fn report(&self) -> PathBuf {
        self.run_root.join("audit.json")
    }

    fn journal_bytes(&self) -> Vec<u8> {
        read(&self.journal(), "journal")
    }

    fn report_bytes(&self) -> Vec<u8> {
        read(&self.report(), "audit report")
    }

    fn landxml_bytes(&self) -> Vec<u8> {
        read(&self.run_root.join("terrain.xml"), "LandXML")
    }
}

fn read(path: &Path, artifact: &str) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("read {artifact} {}: {error}", path.display()))
}

fn wait_for_journal_frames(path: &Path, minimum: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(bytes) = fs::read(path)
            && let Ok(ends) = journal_frame_ends(&bytes)
            && ends.len() >= minimum
        {
            assert!(
                ends.len() < EXPECTED_FRAME_COUNT,
                "workflow completed before the drop-test cancellation point",
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "workflow did not reach journal frame {minimum} before timeout",
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_unlocked_status(run_root: &Path) -> terrain_demo::WorkflowStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match inspect_run(run_root, WorkflowLimits::default()) {
            Ok(status) => return status,
            Err(error) if error.stage() == "lock" && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("inspect dropped workflow: {error}"),
        }
    }
}

fn one_input_point_selection_limits() -> PointSetLimits {
    let defaults = PointSetLimits::default();
    PointSetLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        1,
        defaults.max_output_points(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        defaults.max_working_bytes(),
        defaults.max_resident_bytes(),
        defaults.max_temporary_bytes(),
    )
}

fn one_byte_index_artifact_limits() -> PrepareLimits {
    PrepareLimits::default().with_max_artifact_bytes(1)
}

fn one_byte_workspace_manifest_limits() -> OpenLimits {
    OpenLimits::default().with_max_manifest_bytes(1)
}

fn one_output_point_row_limits() -> PointRowLimits {
    let defaults = PointRowLimits::default();
    PointRowLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        1,
        defaults.max_batch_points(),
        defaults.max_batch_payload_bytes(),
        defaults.max_working_bytes(),
    )
}

fn one_selected_point_commit_limits() -> CommitLimits {
    CommitLimits::default().with_max_selected_points(1)
}

fn one_changed_point_audit_limits() -> RevisionAuditLimits {
    let defaults = RevisionAuditLimits::default();
    RevisionAuditLimits::new(
        defaults.source_read_budget(),
        defaults.max_revision_blocks(),
        defaults.max_revision_bytes(),
        1,
        defaults.max_transition_entries(),
        defaults.max_result_bytes(),
        defaults.max_working_bytes(),
    )
}

fn sixty_three_input_point_terrain_limits() -> TerrainLimits {
    let defaults = TerrainLimits::default();
    TerrainLimits::new(
        defaults.point_rows(),
        63,
        defaults.max_faces(),
        defaults.max_working_bytes(),
        defaults.max_surface_bytes(),
        defaults.max_work_units(),
    )
}

fn one_check_point_limits() -> CheckPointLimits {
    let defaults = CheckPointLimits::default();
    CheckPointLimits::new(
        1,
        defaults.max_result_bytes(),
        defaults.max_face_tests(),
        defaults.max_working_bytes(),
    )
}

fn one_byte_landxml_limits() -> LandXmlLimits {
    let defaults = LandXmlLimits::default();
    LandXmlLimits::new(
        defaults.max_vertices(),
        defaults.max_faces(),
        1,
        1,
        defaults.max_write_buffer_bytes(),
        defaults.max_xml_token_bytes(),
        defaults.max_working_bytes(),
    )
}

fn resource_limit_problem(label: &str, identity: u8, limits: &WorkflowLimits) -> Option<String> {
    let fixture = WorkflowFixture::new(label, "las", 64, identity);
    let source = fs::read(&fixture.source).expect("read immutable resource-limit Source");
    let error = start_run(fixture.paths.clone(), fixture.intent.clone(), *limits)
        .blocking_wait()
        .unwrap_err();
    let diagnostic = error.to_string();
    let problem = (error.code() != "PWF_RESOURCE_LIMIT"
        || !diagnostic.contains("raise the named limit or narrow"))
    .then(|| format!("{label}: {diagnostic}"));
    assert!(
        !fixture.report().exists(),
        "{label}: a limited run must not publish its final report",
    );
    let receipt = if fixture.journal().exists() {
        let status = inspect_run(&fixture.run_root, WorkflowLimits::default())
            .unwrap_or_else(|error| panic!("{label}: inspect limited Run: {error}"));
        assert!(!status.is_complete(), "{label}: false Complete checkpoint");
        if label == "audit-limit" {
            assert_eq!(
                status.phase(),
                WorkflowPhase::RevisionResolved,
                "the resolved Revision must be durable before its audit starts",
            );
        }
        fixture.resume()
    } else {
        fixture.start()
    };
    fixture.assert_single_operation_revision(receipt);
    assert_eq!(
        fs::read(&fixture.source).expect("reread resource-limit Source"),
        source,
        "{label}: Source bytes changed",
    );
    problem
}
