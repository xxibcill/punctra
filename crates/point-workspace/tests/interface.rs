//! Public-interface construction and exact-selection conformance.

mod support;

use point_contracts::{
    AttributeId, CoordinateReference, LinearUnit, SpatialAxes, SpatialReferenceProfile,
    SpatialReferenceProvenance,
};
use point_index::{PrepareLimits, prepare};
use point_workspace::{OpenLimits, RevisionKind, WorkspaceError, WorkspaceSchema, create, open};

use support::{
    classification_attribute, fixture_rows, open_source_with_reference, prepare_fixture,
};

#[test]
fn commit_job_is_a_nameable_public_capability() {
    fn accepts_public_commit_job(_: Option<point_workspace::CommitJob>) {}

    accepts_public_commit_job(None);
}

#[test]
fn deterministic_fixture_retains_complete_source_and_schema() {
    let (_temporary, index, ticks, classifications) = prepare_fixture("fixture", 257);
    assert_eq!(
        index.descriptor().source_point_count(),
        u64::try_from(ticks.len()).unwrap()
    );
    assert_eq!(ticks.len(), classifications.len());
    assert!(
        index
            .source()
            .metadata()
            .attributes()
            .get(classification_attribute())
            .is_some()
    );
}

#[test]
fn create_lock_drop_and_reopen_preserve_root_identity() {
    let (temporary, index, _ticks, _classifications) = prepare_fixture("lifecycle", 1_003);
    let schema = WorkspaceSchema::new(classification_attribute());
    let workspace = create(
        temporary.workspace_path(),
        index.clone(),
        schema,
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let identity = workspace.identity();
    let source = workspace.source();
    assert_eq!(workspace.schema(), schema);
    let root = workspace.head();
    let root_provenance = *root.provenance();
    let root_info = workspace.revision_info(root_provenance.revision()).unwrap();
    assert_eq!(root_provenance.workspace(), identity);
    assert_eq!(root_provenance.source(), source);
    assert_eq!(root_info.id(), root_provenance.revision());
    assert_eq!(root_info.parent(), None);
    assert_eq!(root_info.sequence(), 0);
    assert_eq!(root_info.operation(), None);
    assert_eq!(root_info.kind(), RevisionKind::Root);
    assert_eq!(identity.to_string().len(), 32);

    let locked = open(
        temporary.workspace_path(),
        index.clone(),
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();
    assert!(matches!(locked, WorkspaceError::Locked));

    drop(root);
    drop(workspace);
    let reopened = open(temporary.workspace_path(), index, OpenLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(reopened.identity(), identity);
    assert_eq!(reopened.source(), source);
    assert_eq!(reopened.schema(), schema);
    assert_eq!(reopened.head().provenance(), &root_provenance);
}

#[test]
fn create_rejects_a_missing_classification_attribute_without_publishing() {
    let (temporary, index, _ticks, _classifications) = prepare_fixture("schema", 19);
    let missing = AttributeId::new(999).unwrap();

    let error = create(
        temporary.workspace_path(),
        index,
        WorkspaceSchema::new(missing),
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();

    assert!(matches!(error, WorkspaceError::Incompatible { .. }));
    assert!(!temporary.workspace_path().exists());
}

#[test]
fn reopen_rejects_changed_spatial_reference_with_unchanged_point_rows() {
    let temporary = support::TemporaryFixture::new("spatial-reference");
    let (ticks, classifications) = fixture_rows(67);
    let profile = |vertical_epsg| {
        CoordinateReference::profile(
            SpatialReferenceProfile::new(
                32_647,
                vertical_epsg,
                SpatialAxes::EastingNorthingElevation,
                LinearUnit::Metre,
                LinearUnit::Metre,
                SpatialReferenceProvenance::CallerDeclaration,
            )
            .unwrap(),
        )
    };
    let original_index = prepare(
        open_source_with_reference(ticks.clone(), classifications.clone(), profile(5_703)),
        temporary.index_path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let workspace = create(
        temporary.workspace_path(),
        original_index,
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    drop(workspace);

    let changed_index = prepare(
        open_source_with_reference(ticks, classifications, profile(5_704)),
        temporary.path().join("changed-reference.pidx"),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let error = open(
        temporary.workspace_path(),
        changed_index,
        OpenLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();
    assert!(matches!(error, WorkspaceError::Incompatible { .. }));
}
