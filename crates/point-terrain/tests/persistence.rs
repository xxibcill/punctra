//! Durable disk-v1 Surface preparation, recovery, and bounded-read evidence.

mod support;

use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use point_contracts::{
    CoordinateReference, LinearUnit, SpatialAxes, SpatialReferenceProfile,
    SpatialReferenceProvenance, WorldBounds,
};
use point_terrain::{
    SurfaceReadLimits, TerrainError, TerrainPrepareDisposition, TerrainPrepareLimits,
    TerrainRecipe, prepare,
};
use point_workspace::{CommitLimits, CommitRequest, Snapshot};

use support::{TerrainFixture, committed, operation};

#[test]
fn cold_and_warm_prepare_share_canonical_file_backed_surface() {
    let fixture = fixture("persistence-cold-warm");
    let target = fixture.terrain_path("surface.pterr");
    let snapshot = fixture.snapshot();
    let recipe = bounded_recipe(2);

    let cold = prepare_surface(
        snapshot.clone(),
        &target,
        recipe,
        TerrainPrepareLimits::default(),
    )
    .expect("cold Surface prepares");
    assert_eq!(
        cold.report().disposition(),
        TerrainPrepareDisposition::Built
    );
    assert_eq!(cold.descriptor().snapshot(), *snapshot.provenance());
    assert_eq!(cold.descriptor().recipe(), recipe);
    assert_eq!(
        cold.report().artifact_bytes(),
        fs::metadata(&target).unwrap().len()
    );
    assert!(cold.report().accounted_peak_working_bytes().is_some());
    assert!(cold.report().topology_steps().is_some());
    assert_eq!(
        cold.report().source_points_read(),
        cold.descriptor().input_point_count()
    );
    assert!(
        cold.report().peak_temporary_disk_bytes() > cold.report().artifact_bytes(),
        "cold preparation temporarily owns both work and artifact bytes"
    );
    let cold_bytes = fs::read(&target).unwrap();
    let cold_vertices = collect_vertices(&cold, 2);
    let cold_faces = collect_faces(&cold, 1);
    assert_eq!(cold_vertices.len() as u64, cold.descriptor().vertex_count());
    assert_eq!(cold_faces.len() as u64, cold.descriptor().face_count());

    let warm = prepare_surface(snapshot, &target, recipe, TerrainPrepareLimits::default())
        .expect("matching Surface opens warm");
    assert_eq!(
        warm.report().disposition(),
        TerrainPrepareDisposition::Opened
    );
    assert_eq!(warm.report().reused_input_points(), 0);
    assert_eq!(warm.report().source_points_read(), 0);
    assert_eq!(warm.report().peak_temporary_disk_bytes(), 0);
    assert_eq!(warm.report().accounted_peak_working_bytes(), None);
    assert_eq!(warm.report().topology_steps(), None);
    assert_eq!(warm.descriptor(), cold.descriptor());
    assert_eq!(collect_vertices(&warm, 3), cold_vertices);
    assert_eq!(collect_faces(&warm, 2), cold_faces);
    assert_eq!(fs::read(&target).unwrap(), cold_bytes);
}

#[test]
fn prepared_surface_matches_the_legacy_deterministic_derivation_oracle() {
    let fixture = fixture("persistence-legacy-oracle");
    let target = fixture.terrain_path("surface.pterr");
    let snapshot = fixture.snapshot();
    let recipe = bounded_recipe(2);
    let limits = TerrainPrepareLimits::default();
    let legacy = point_terrain::derive(snapshot.clone(), recipe, limits.derivation())
        .blocking_wait()
        .expect("legacy bounded AOI derives");
    let prepared = prepare_surface(snapshot, &target, recipe, limits).unwrap();

    assert_eq!(collect_vertices(&prepared, 2), legacy.vertices());
    assert_eq!(collect_faces(&prepared, 2), legacy.faces());
    let persisted = prepared.descriptor();
    let in_memory = legacy.descriptor();
    assert_eq!(persisted.snapshot(), in_memory.snapshot());
    assert_eq!(persisted.recipe_hash(), in_memory.recipe_hash());
    assert_eq!(persisted.input_hash(), in_memory.input_hash());
    assert_eq!(persisted.geometry_hash(), in_memory.geometry_hash());
    assert_eq!(persisted.topology_hash(), in_memory.topology_hash());
    assert_eq!(persisted.artifact_hash(), in_memory.artifact_hash());
    assert_eq!(persisted.vertex_count(), in_memory.vertex_count());
    assert_eq!(persisted.face_count(), in_memory.face_count());
    assert_eq!(persisted.bounds(), in_memory.bounds());
}

#[cfg(unix)]
#[test]
fn bounded_streams_remain_bound_to_the_verified_open_file() {
    let fixture = fixture("persistence-open-file-binding");
    let target = fixture.terrain_path("surface.pterr");
    let moved = fixture.terrain_path("verified-original.pterr");
    let surface = prepare_surface(
        fixture.snapshot(),
        &target,
        bounded_recipe(2),
        TerrainPrepareLimits::default(),
    )
    .unwrap();
    let expected_vertices = collect_vertices(&surface, 2);
    let artifact_bytes = fs::metadata(&target).unwrap().len();
    fs::rename(&target, &moved).unwrap();
    fs::write(
        &target,
        vec![0xA5; usize::try_from(artifact_bytes).unwrap()],
    )
    .unwrap();

    assert_eq!(
        collect_vertices(&surface, 1),
        expected_vertices,
        "the handle reads the verified open inode, not a same-length path replacement"
    );
}

#[test]
fn prepare_requires_explicit_bounds_before_writing_any_file() {
    let fixture = fixture("persistence-explicit-aoi");
    let target = fixture.terrain_path("surface.pterr");
    let error = prepare_surface(
        fixture.snapshot(),
        &target,
        TerrainRecipe::new(2),
        TerrainPrepareLimits::default(),
    )
    .expect_err("persistent preparation requires an AOI");
    assert!(matches!(
        error,
        TerrainError::InvalidArgument {
            argument: "persistent Terrain Recipe bounds",
            ..
        }
    ));
    assert!(!target.exists());
}

#[test]
fn prepare_rejects_unsupported_spatial_references_before_writing_any_file() {
    let feet = SpatialReferenceProfile::new(
        2_230,
        5_703,
        SpatialAxes::EastingNorthingElevation,
        LinearUnit::UsSurveyFoot,
        LinearUnit::UsSurveyFoot,
        SpatialReferenceProvenance::CallerDeclaration,
    )
    .unwrap();
    for (label, reference) in [
        ("unknown", CoordinateReference::Unknown),
        (
            "opaque-wkt",
            CoordinateReference::wkt("LOCAL_CS[\"opaque\"]").unwrap(),
        ),
        ("feet", CoordinateReference::profile(feet)),
    ] {
        let fixture = TerrainFixture::with_reference(
            &format!("persistence-{label}-reference"),
            reference,
            vec![[0, 0, 0], [10, 0, 10], [0, 10, 20]],
            vec![2; 3],
        );
        let target = fixture.terrain_path("surface.pterr");

        let error = prepare_surface(
            fixture.snapshot(),
            &target,
            bounded_recipe(2),
            TerrainPrepareLimits::default(),
        )
        .expect_err("persistent Terrain requires the supported metre survey profile");
        let TerrainError::UnsupportedSpatialReference { reason } = error else {
            panic!("unsupported Terrain spatial references need a structured error, got {error:?}");
        };
        assert_eq!(
            reason.as_str(),
            "persistent Surface artifacts require the supported structured metre survey profile"
        );
        assert!(!target.exists());
    }
}

#[cfg(unix)]
#[test]
fn existing_symlink_target_is_rejected_without_touching_its_destination() {
    use std::os::unix::fs::symlink;

    let fixture = fixture("persistence-symlink");
    let target = fixture.terrain_path("surface.pterr");
    let destination = fixture.terrain_path("destination.bin");
    let original = b"caller-owned destination";
    fs::write(&destination, original).unwrap();
    symlink(&destination, &target).unwrap();

    let error = prepare_surface(
        fixture.snapshot(),
        &target,
        bounded_recipe(2),
        TerrainPrepareLimits::default(),
    )
    .expect_err("symbolic-link targets fail closed");
    assert!(matches!(
        error,
        TerrainError::CorruptSurfaceArtifact {
            kind: "Surface artifact",
            ..
        }
    ));
    assert_eq!(fs::read(&destination).unwrap(), original);
}

#[test]
fn checksum_corruption_is_rejected_and_preserved() {
    let fixture = fixture("persistence-corruption");
    let target = fixture.terrain_path("surface.pterr");
    let snapshot = fixture.snapshot();
    let recipe = bounded_recipe(2);
    drop(
        prepare_surface(
            snapshot.clone(),
            &target,
            recipe,
            TerrainPrepareLimits::default(),
        )
        .unwrap(),
    );
    let middle = fs::metadata(&target).unwrap().len() / 2;
    flip_byte(&target, middle);
    let corrupted = fs::read(&target).unwrap();

    let error = prepare_surface(snapshot, &target, recipe, TerrainPrepareLimits::default())
        .expect_err("checksum corruption fails closed");
    assert!(matches!(
        error,
        TerrainError::CorruptSurfaceArtifact {
            kind: "Surface artifact",
            ..
        }
    ));
    assert_eq!(fs::read(&target).unwrap(), corrupted);
}

#[test]
fn recipe_and_snapshot_mismatches_are_stale_and_preserve_target() {
    let fixture = fixture("persistence-stale");
    let target = fixture.terrain_path("surface.pterr");
    let root = fixture.snapshot();
    let recipe = bounded_recipe(2);
    drop(
        prepare_surface(
            root.clone(),
            &target,
            recipe,
            TerrainPrepareLimits::default(),
        )
        .unwrap(),
    );
    let original = fs::read(&target).unwrap();

    let narrower = TerrainRecipe::new(2)
        .within(WorldBounds::new([0.0, 0.0, -10.0], [8.0, 8.0, 20.0]).unwrap());
    let recipe_error = prepare_surface(
        root.clone(),
        &target,
        narrower,
        TerrainPrepareLimits::default(),
    )
    .expect_err("different AOI is stale");
    assert!(matches!(
        recipe_error,
        TerrainError::StaleSurfaceArtifact {
            binding: "Terrain Recipe",
            ..
        }
    ));
    assert_eq!(fs::read(&target).unwrap(), original);

    let selected = fixture.select_ordinals(&root, &[4]);
    let edit = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::set_classification(operation(41), selected, 9),
                CommitLimits::default(),
            )
            .blocking_wait()
            .unwrap(),
    );
    let edited = fixture.workspace().snapshot(edit.revision()).unwrap();
    let snapshot_error = prepare_surface(edited, &target, recipe, TerrainPrepareLimits::default())
        .expect_err("different Snapshot Revision is stale");
    assert!(matches!(
        snapshot_error,
        TerrainError::StaleSurfaceArtifact {
            binding: "Snapshot Revision",
            ..
        }
    ));
    assert_eq!(fs::read(&target).unwrap(), original);
    prepare_surface(root, &target, recipe, TerrainPrepareLimits::default())
        .expect("original binding remains warm-openable");
}

#[test]
fn verified_input_checkpoint_resumes_to_canonical_artifact() {
    let fixture = fixture("persistence-input-resume");
    let snapshot = fixture.snapshot();
    let recipe = bounded_recipe(2);
    let resumed_target = fixture.terrain_path("resumed.pterr");
    let clean_target = fixture.terrain_path("clean.pterr");
    let defaults = TerrainPrepareLimits::default();
    let too_small = TerrainPrepareLimits::new(
        defaults.derivation(),
        defaults.max_work_bytes(),
        1,
        defaults.max_temporary_bytes(),
        defaults.max_verify_buffer_bytes(),
        defaults.max_retained_handle_bytes(),
        defaults.max_path_bytes(),
    );

    let error = prepare_surface(snapshot.clone(), &resumed_target, recipe, too_small)
        .expect_err("artifact ceiling fails after durable input collection");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "Surface artifact bytes",
            ..
        }
    ));
    let work_path = resumed_target.with_file_name("resumed.pterr.surface-work-v1");
    assert!(work_path.is_file(), "complete input checkpoint is durable");
    assert!(!resumed_target.exists());

    let resumed = prepare_surface(snapshot.clone(), &resumed_target, recipe, defaults)
        .expect("retry resumes verified input");
    assert_eq!(
        resumed.report().disposition(),
        TerrainPrepareDisposition::ResumedInput
    );
    assert_eq!(
        resumed.report().reused_input_points(),
        resumed.descriptor().input_point_count()
    );
    assert_eq!(resumed.report().source_points_read(), 0);
    assert!(resumed.report().peak_temporary_disk_bytes() > 0);
    let clean = prepare_surface(snapshot, &clean_target, recipe, defaults)
        .expect("comparison target builds cleanly");
    assert_eq!(resumed.descriptor(), clean.descriptor());
    assert_eq!(
        fs::read(&resumed_target).unwrap(),
        fs::read(&clean_target).unwrap(),
        "attempt observations do not alter canonical artifact bytes"
    );
    assert!(
        work_path.is_file(),
        "verified work checkpoint is retained for race-safe recovery"
    );
}

#[test]
fn verified_complete_stage_resumes_publication_without_triangulation() {
    let fixture = fixture("persistence-publication-resume");
    let snapshot = fixture.snapshot();
    let recipe = bounded_recipe(2);
    let source_target = fixture.terrain_path("source.pterr");
    let resumed_target = fixture.terrain_path("resumed.pterr");
    let stage_path = resumed_target.with_file_name("resumed.pterr.surface-stage-v1");
    let source = prepare_surface(
        snapshot.clone(),
        &source_target,
        recipe,
        TerrainPrepareLimits::default(),
    )
    .unwrap();
    fs::copy(&source_target, &stage_path).unwrap();

    let resumed = prepare_surface(
        snapshot,
        &resumed_target,
        recipe,
        TerrainPrepareLimits::default(),
    )
    .expect("verified complete stage publishes");
    assert_eq!(
        resumed.report().disposition(),
        TerrainPrepareDisposition::ResumedPublication
    );
    assert_eq!(resumed.report().accounted_peak_working_bytes(), None);
    assert_eq!(resumed.report().topology_steps(), None);
    assert_eq!(resumed.report().source_points_read(), 0);
    assert_eq!(
        resumed.report().peak_temporary_disk_bytes(),
        resumed.report().artifact_bytes()
    );
    assert_eq!(resumed.descriptor(), source.descriptor());
    assert_eq!(
        fs::read(&resumed_target).unwrap(),
        fs::read(&source_target).unwrap()
    );
    assert!(
        stage_path.is_file(),
        "verified stage is retained because conditional unlink is unavailable"
    );
}

#[test]
fn work_and_read_limits_fail_before_unbounded_allocation() {
    let fixture = fixture("persistence-resource-limits");
    let target = fixture.terrain_path("surface.pterr");
    let defaults = TerrainPrepareLimits::default();
    let work_limited = TerrainPrepareLimits::new(
        defaults.derivation(),
        1,
        defaults.max_artifact_bytes(),
        defaults.max_temporary_bytes(),
        defaults.max_verify_buffer_bytes(),
        defaults.max_retained_handle_bytes(),
        defaults.max_path_bytes(),
    );
    let error = prepare_surface(fixture.snapshot(), &target, bounded_recipe(2), work_limited)
        .expect_err("work checkpoint limit is enforced");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "Surface work checkpoint bytes",
            ..
        }
    ));
    assert!(!target.exists());

    let surface =
        prepare_surface(fixture.snapshot(), &target, bounded_recipe(2), defaults).unwrap();
    let Err(read_error) = surface.vertex_batches(SurfaceReadLimits::new(1, 1024, 1, 0, 10)) else {
        panic!("zero working bytes must reject the stream");
    };
    assert!(matches!(
        read_error,
        TerrainError::ResourceLimit {
            limit: "Surface read working bytes",
            ..
        }
    ));
}

fn fixture(label: &str) -> TerrainFixture {
    TerrainFixture::new(
        label,
        vec![
            [0, 0, 0],
            [10, 0, 2],
            [10, 10, 4],
            [0, 10, 6],
            [5, 5, 3],
            [3, 7, 4],
        ],
        vec![2; 6],
    )
}

fn bounded_recipe(classification: u8) -> TerrainRecipe {
    TerrainRecipe::new(classification)
        .within(WorldBounds::new([-1.0, -1.0, -10.0], [11.0, 11.0, 20.0]).unwrap())
}

fn prepare_surface(
    snapshot: Snapshot,
    target: &Path,
    recipe: TerrainRecipe,
    limits: TerrainPrepareLimits,
) -> Result<point_terrain::PreparedTerrainSurface, TerrainError> {
    prepare(snapshot, target, recipe, limits).blocking_wait()
}

fn collect_vertices(
    surface: &point_terrain::PreparedTerrainSurface,
    batch_records: u64,
) -> Vec<point_terrain::SurfaceVertex> {
    surface
        .vertex_batches(SurfaceReadLimits::new(
            batch_records,
            1024 * 1024,
            128 * 1024,
            1024 * 1024,
            100_000_000,
        ))
        .unwrap()
        .flat_map(|batch| batch.expect("vertex batch reads"))
        .collect()
}

fn collect_faces(
    surface: &point_terrain::PreparedTerrainSurface,
    batch_records: u64,
) -> Vec<point_terrain::SurfaceFace> {
    surface
        .face_batches(SurfaceReadLimits::new(
            batch_records,
            1024 * 1024,
            128 * 1024,
            1024 * 1024,
            100_000_000,
        ))
        .unwrap()
        .flat_map(|batch| batch.expect("face batch reads"))
        .collect()
}

fn flip_byte(path: &Path, offset: u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut byte = [0];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x80;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
}
