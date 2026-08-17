//! Public Terrain Derivation behavior across immutable Workspace Snapshots.

mod support;

use point_contracts::WorldBounds;
use point_terrain::{ALGORITHM_VERSION, TerrainError, TerrainLimits, TerrainRecipe};
use point_workspace::{CommitLimits, CommitRequest};

use support::{
    TerrainFixture, committed, derive_surface, derive_with, operation,
    terrain_limits_with_row_batch,
};

#[test]
fn three_point_plane_publishes_exact_canonical_surface_facts() {
    let fixture = TerrainFixture::new(
        "three-point-plane",
        vec![[0, 0, 0], [10, 0, 10], [0, 10, 20]],
        vec![2; 3],
    );
    let snapshot = fixture.snapshot();
    let surface = derive_surface(snapshot.clone(), 2);

    let vertices = surface
        .vertices()
        .iter()
        .map(|vertex| (vertex.id().get(), vertex.point(), vertex.ticks()))
        .collect::<Vec<_>>();
    assert_eq!(
        vertices,
        vec![
            (1, fixture.point(0), [0, 0, 0]),
            (2, fixture.point(2), [0, 10, 20]),
            (3, fixture.point(1), [10, 0, 10]),
        ]
    );
    assert_eq!(surface.faces().len(), 1);
    assert_eq!(surface.faces()[0].id().get(), 1);
    assert_eq!(
        surface.faces()[0]
            .vertices()
            .map(point_terrain::SurfaceVertexId::get),
        [1, 3, 2]
    );

    let descriptor = surface.descriptor();
    assert_eq!(descriptor.snapshot(), *snapshot.provenance());
    assert_eq!(descriptor.recipe(), TerrainRecipe::new(2));
    assert_eq!(descriptor.algorithm_version(), ALGORITHM_VERSION);
    assert_eq!(
        descriptor.position_transform(),
        support::identity_transform()
    );
    assert_eq!(
        descriptor.coordinate_reference(),
        &support::supported_reference()
    );
    assert_eq!(descriptor.input_point_count(), 3);
    assert_eq!(descriptor.vertex_count(), 3);
    assert_eq!(descriptor.face_count(), 1);
    assert_eq!(descriptor.hull_vertex_count(), 3);
    assert_eq!(
        descriptor.bounds(),
        WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap()
    );
    assert!(descriptor.accounted_peak_working_bytes() > 0);
    assert!(descriptor.retained_surface_bytes() > 0);
    assert!(descriptor.topology_steps() > 0);
}

#[test]
fn inclusive_bounds_apply_to_effective_snapshot_classification() {
    let fixture = TerrainFixture::new(
        "bounds-effective-classification",
        vec![
            [-1, -1, 0],
            [1, -1, 0],
            [-1, 1, 0],
            [1, 1, 0],
            [2, 0, 0],
            [0, 0, 0],
        ],
        vec![2, 2, 9, 9, 2, 9],
    );
    let root = fixture.snapshot();
    let target = fixture.select_ordinals(&root, &[2]);
    let edit = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::set_classification(operation(1), target, 2),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("classification Edit is certain"),
    );
    let edited = fixture.workspace().snapshot(edit.revision()).unwrap();
    let bounds = WorldBounds::new([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]).unwrap();
    let recipe = TerrainRecipe::new(2).within(bounds);

    assert!(matches!(
        derive_with(root.clone(), recipe, TerrainLimits::default()),
        Err(TerrainError::InsufficientGroundInput { actual: 2 })
    ));
    let surface = derive_with(edited, recipe, TerrainLimits::default()).unwrap();
    let points = surface
        .vertices()
        .iter()
        .map(|vertex| vertex.point())
        .collect::<Vec<_>>();
    assert_eq!(
        points,
        vec![fixture.point(0), fixture.point(2), fixture.point(1)],
        "all three boundary Points are inclusive, the Edit is effective, and mismatched/outside rows are absent"
    );
    assert_eq!(surface.descriptor().recipe(), recipe);
    assert_eq!(
        surface.descriptor().bounds(),
        WorldBounds::new([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]).unwrap(),
        "descriptor bounds describe the selected Terrain geometry, not the wider query box"
    );
    assert!(matches!(
        derive_with(root, recipe, TerrainLimits::default()),
        Err(TerrainError::InsufficientGroundInput { actual: 2 })
    ));
}

#[test]
fn historical_edit_and_revert_snapshots_derive_independently() {
    let fixture = TerrainFixture::new(
        "snapshot-history",
        vec![[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]],
        vec![7; 4],
    );
    let root = fixture.snapshot();
    let root_surface = derive_surface(root.clone(), 7);
    let target = fixture.select_ordinals(&root, &[0]);
    let edit = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::set_classification(operation(2), target, 9),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("classification Edit is certain"),
    );
    let edited = fixture.workspace().snapshot(edit.revision()).unwrap();
    let edited_surface_before_revert = derive_surface(edited.clone(), 7);
    assert_eq!(edited_surface_before_revert.vertices().len(), 3);
    assert_eq!(edited_surface_before_revert.faces().len(), 1);

    let revert = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::revert_head(operation(3), edit.revision()),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("Revert is certain"),
    );
    let reverted = fixture.workspace().snapshot(revert.revision()).unwrap();
    let reverted_surface = derive_surface(reverted, 7);
    let edited_surface_after_revert = derive_surface(edited, 7);

    assert_eq!(
        edited_surface_before_revert.descriptor().geometry_hash(),
        edited_surface_after_revert.descriptor().geometry_hash()
    );
    assert_eq!(
        edited_surface_before_revert.descriptor().artifact_hash(),
        edited_surface_after_revert.descriptor().artifact_hash(),
        "the historical edited Snapshot remains immutable after Revert"
    );
    assert_eq!(root_surface.vertices(), reverted_surface.vertices());
    assert_eq!(root_surface.faces(), reverted_surface.faces());
    assert_eq!(
        root_surface.descriptor().geometry_hash(),
        reverted_surface.descriptor().geometry_hash()
    );
    assert_eq!(
        root_surface.descriptor().topology_hash(),
        reverted_surface.descriptor().topology_hash()
    );
    assert_ne!(
        root_surface.descriptor().input_hash(),
        reverted_surface.descriptor().input_hash(),
        "Snapshot row content hashes bind Revision provenance"
    );
    assert_ne!(
        root_surface.descriptor().artifact_hash(),
        reverted_surface.descriptor().artifact_hash(),
        "Terrain Artifact hashes bind Snapshot provenance"
    );
}

#[test]
fn repeated_and_row_partitioned_derivations_have_identical_hashes() {
    let fixture = TerrainFixture::new(
        "partition-independent-hashes",
        vec![
            [0, 0, 0],
            [3, 0, 1],
            [7, 1, 2],
            [9, 5, 3],
            [8, 9, 4],
            [4, 11, 5],
            [0, 8, 6],
            [2, 4, 7],
            [6, 5, 8],
        ],
        vec![4; 9],
    );
    let snapshot = fixture.snapshot();
    let fine = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(4),
        terrain_limits_with_row_batch(1),
    )
    .unwrap();
    let coarse = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(4),
        terrain_limits_with_row_batch(7),
    )
    .unwrap();
    let repeated = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(4),
        terrain_limits_with_row_batch(7),
    )
    .unwrap();
    let covering_bounds = WorldBounds::new([-1.0; 3], [12.0; 3]).unwrap();
    let bounded = derive_with(
        snapshot,
        TerrainRecipe::new(4).within(covering_bounds),
        terrain_limits_with_row_batch(3),
    )
    .unwrap();

    for surface in [&coarse, &repeated] {
        assert_eq!(fine.vertices(), surface.vertices());
        assert_eq!(fine.faces(), surface.faces());
        assert_eq!(
            fine.descriptor().recipe_hash(),
            surface.descriptor().recipe_hash()
        );
        assert_eq!(
            fine.descriptor().input_hash(),
            surface.descriptor().input_hash()
        );
        assert_eq!(
            fine.descriptor().geometry_hash(),
            surface.descriptor().geometry_hash()
        );
        assert_eq!(
            fine.descriptor().topology_hash(),
            surface.descriptor().topology_hash()
        );
        assert_eq!(
            fine.descriptor().artifact_hash(),
            surface.descriptor().artifact_hash()
        );
    }
    assert_eq!(fine.vertices(), bounded.vertices());
    assert_eq!(fine.faces(), bounded.faces());
    assert_ne!(
        fine.descriptor().recipe_hash(),
        bounded.descriptor().recipe_hash(),
        "a semantically different bounds intent has a different Recipe hash even when it selects the same Points"
    );
    assert_eq!(
        fine.descriptor().geometry_hash(),
        bounded.descriptor().geometry_hash()
    );
    assert_eq!(
        fine.descriptor().topology_hash(),
        bounded.descriptor().topology_hash()
    );
    assert_ne!(
        fine.descriptor().artifact_hash(),
        bounded.descriptor().artifact_hash()
    );
}
