//! Frozen Surface disk-v1 artifact and input-checkpoint compatibility evidence.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, LinearUnit, PositionTransform, SpatialAxes,
    SpatialReferenceProfile, SpatialReferenceProvenance, WorldBounds,
};
use point_index::{PrepareLimits, PreparedIndex};
use point_terrain::{
    PreparedTerrainSurface, SurfaceArtifactDescriptor, SurfaceReadLimits,
    TerrainPrepareDisposition, TerrainPrepareLimits, TerrainRecipe,
};
use point_workspace::{OpenLimits, Snapshot, WorkspaceSchema};
use source_memory::{MemoryFaultControl, MemorySource};

const FIXTURE_ROOT: &str = "tests/fixtures/v1";
const COMPLETE_ARTIFACT: &str = "bounded-six-point.pterr";
const WORK_CHECKPOINT: &str = "bounded-six-point.pterr.surface-work-v1";
const WORKSPACE_MANIFEST: &str = "workspace/manifest.pwm";
const FIXTURE_SCHEMA: &str = "punctra.point-terrain.fixture-manifest.v1";
const REGENERATION_GATE: &str = "PUNCTRA_REGENERATE_TERRAIN_V1";
const BOOTSTRAP_GATE: &str = "PUNCTRA_BOOTSTRAP_TERRAIN_V1_WORKSPACE";
const CLASSIFICATION_ATTRIBUTE: u32 = 301;
const GROUND_CLASSIFICATION: u8 = 2;
const WORKSPACE_ID: &str = "0a1d38f09738b17ac8f8a2d672d49ec0";
const SOURCE_ID: &str = "55ac5993c191ad69af6bba3282e0376bbd6a3d8cd639bb9906fb2cbf194f8803";
const REVISION_ID: &str = "0e37dff4a9502d0d1dc4567d6d599b67232b8e4646bb7d99c0626a039c61b213";

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one manifest assertion keeps all frozen disk-v1 facts reviewable together"
)]
fn v1_manifest_pins_paths_lengths_hashes_and_semantic_facts() {
    let manifest_path = fixture_path("manifest.json");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(&manifest_path).expect("read frozen Surface fixture manifest"),
    )
    .expect("frozen Surface fixture manifest is valid JSON");

    assert_eq!(manifest["schema"], FIXTURE_SCHEMA);
    assert_eq!(manifest["owner"], "point-terrain");
    assert_eq!(manifest["support_class"], "rebuildable");
    assert_eq!(manifest["path_base"], "manifest_directory");
    assert_eq!(manifest["disk_version"], 1);
    assert_eq!(manifest["algorithm_version"], 1);
    assert_eq!(point_terrain::SURFACE_DISK_VERSION, 1);
    assert_eq!(point_terrain::ALGORITHM_VERSION, 1);
    assert_eq!(manifest["source"]["point_count"], 6);
    assert_eq!(manifest["source"]["classification_attribute"], 301);
    assert_eq!(
        manifest["source"]["classification_values"],
        serde_json::json!([2, 2, 2, 2, 2, 2])
    );
    assert_eq!(
        manifest["source"]["position_transform"],
        serde_json::json!({
            "offset": [400_000.0, 1_500_000.0, 10.0],
            "scale": [0.25, 0.25, 0.1]
        })
    );
    assert_eq!(
        manifest["source"]["coordinate_reference"],
        serde_json::json!({
            "horizontal_epsg": 32647,
            "vertical_epsg": 5703,
            "axes": "easting_northing_elevation",
            "horizontal_unit": "metre",
            "vertical_unit": "metre",
            "provenance": "caller_declaration"
        })
    );
    assert_eq!(
        manifest["recipe"],
        serde_json::json!({
            "ground_classification": 2,
            "bounds": {
                "minimum": [399_999.0, 1_499_999.0, 0.0],
                "maximum": [400_004.0, 1_500_004.0, 20.0]
            }
        })
    );
    assert_eq!(
        manifest["snapshot"],
        serde_json::json!({
            "workspace": WORKSPACE_ID,
            "source": SOURCE_ID,
            "revision": REVISION_ID
        })
    );
    assert_eq!(manifest["source"]["identity"], SOURCE_ID);
    assert_eq!(
        manifest["surface"],
        serde_json::json!({
            "recipe_hash": "8306b53b31c0f1bccd1ddb17d45be09ab652e1a6bd1e19ec25cbc5363bbe2b7d",
            "input_hash": "2e87d01015b87e5d93a8d9a0a409f36e44caa9f63ff614550163d80b4505c238",
            "geometry_hash": "7f5f08f59b22d96b4b47477f9d44ada99a321f937fac0e8a0865b5d98872bc3a",
            "topology_hash": "fb8c17a2984dbb0cf0d87444c9f9b66e28c2ffbbfa4e3aca2bef9f362e33d63e",
            "artifact_hash": "5680cdd1765d9f950cd7d2c0429a0e668cbe9a701e0a459fc8986e30e92ca1f0",
            "input_point_count": 6,
            "vertex_count": 6,
            "face_count": 6,
            "hull_vertex_count": 4,
            "bounds": {
                "minimum": [400_000.0, 1_500_000.0, 10.0],
                "maximum": [400_002.5, 1_500_002.5, 10.6]
            }
        })
    );

    let files = manifest["files"].as_array().expect("fixture file array");
    assert_eq!(files.len(), 3);
    for file in files {
        let relative = file["path"].as_str().expect("relative fixture path");
        let bytes = fs::read(fixture_path(relative))
            .unwrap_or_else(|error| panic!("read frozen fixture {relative}: {error}"));
        assert_eq!(
            u64::try_from(bytes.len()).unwrap(),
            file["byte_length"].as_u64().unwrap(),
            "{relative} byte length"
        );
        assert_eq!(
            blake3::hash(&bytes).to_hex().as_str(),
            file["blake3"].as_str().unwrap(),
            "{relative} BLAKE3"
        );
    }

    assert_eq!(files[0]["path"], COMPLETE_ARTIFACT);
    assert_eq!(files[0]["role"], "complete");
    assert_eq!(files[0]["byte_length"], 936);
    assert_eq!(
        files[0]["blake3"],
        "39d8f1decc8999f590d6dbefb3ea96748550a55dceea0a8ffdcf75a55f6c9b6c"
    );
    assert_eq!(files[1]["path"], WORK_CHECKPOINT);
    assert_eq!(files[1]["role"], "resumable-input");
    assert_eq!(files[1]["byte_length"], 608);
    assert_eq!(
        files[1]["blake3"],
        "49b24812750cac6cf3f54f074ad8f9eea877a9f61fe6fd3c98871e66a9d5d2ef"
    );
    assert_eq!(files[2]["path"], WORKSPACE_MANIFEST);
    assert_eq!(files[2]["role"], "snapshot-binding");
    assert_eq!(files[2]["byte_length"], 1_252);
    assert_eq!(
        files[2]["blake3"],
        "b116198600aa5f5f9403925c26c576c24b2e634cb88cf65db767ec3d1fa5c161"
    );
    assert_eq!(files[0]["semantic_facts"], manifest["surface"]);
    assert_eq!(
        files[1]["semantic_facts"],
        serde_json::json!({
            "durable_input_points": 6,
            "resume_disposition": "resumed_input",
            "source_points_read": 0,
            "rebuild_equals_complete": true
        })
    );
}

#[test]
fn frozen_complete_artifact_warm_opens_without_snapshot_reads_or_fixture_mutation() {
    let frozen_before = frozen_tree_hashes();
    let temporary = TemporaryFixture::new("v1-complete-open");
    let (index, faults) = prepare_fixed_index(&temporary);
    let workspace_root = temporary.path().join("workspace-copy.pcw");
    copy_frozen_workspace(&workspace_root);
    let copied_workspace_before = fs::read(workspace_root.join("manifest.pwm")).unwrap();
    let workspace = point_workspace::open(&workspace_root, index, OpenLimits::default())
        .blocking_wait()
        .expect("copied frozen Workspace opens through its public owner");
    let target = temporary.path().join(COMPLETE_ARTIFACT);
    fs::copy(fixture_path(COMPLETE_ARTIFACT), &target).unwrap();
    let copied_before = fs::read(&target).unwrap();
    let oracle = point_terrain::derive(
        workspace.head(),
        recipe(),
        TerrainPrepareLimits::default().derivation(),
    )
    .blocking_wait()
    .expect("legacy in-memory oracle derives before Source invalidation");

    faults.mark_changed();
    faults.fail_at_ordinal(0);
    let surface = prepare_surface(workspace.head(), &target)
        .expect("complete frozen Surface warm-opens without Snapshot rows");
    assert_eq!(
        surface.report().disposition(),
        TerrainPrepareDisposition::Opened
    );
    assert_eq!(surface.report().source_points_read(), 0);
    assert_eq!(surface.report().reused_input_points(), 0);
    assert_eq!(surface.report().peak_temporary_disk_bytes(), 0);
    assert_surface_facts(surface.descriptor());
    assert_eq!(collect_vertices(&surface), oracle.vertices());
    assert_eq!(collect_faces(&surface), oracle.faces());
    assert_eq!(fs::read(&target).unwrap(), copied_before);
    drop(workspace);
    assert_eq!(
        fs::read(workspace_root.join("manifest.pwm")).unwrap(),
        copied_workspace_before
    );
    assert_eq!(frozen_tree_hashes(), frozen_before);
}

#[test]
fn frozen_work_resumes_without_snapshot_reads_to_exact_complete_bytes() {
    let frozen_before = frozen_tree_hashes();
    let temporary = TemporaryFixture::new("v1-work-resume");
    let (index, faults) = prepare_fixed_index(&temporary);
    let workspace_root = temporary.path().join("workspace-copy.pcw");
    copy_frozen_workspace(&workspace_root);
    let copied_workspace_before = fs::read(workspace_root.join("manifest.pwm")).unwrap();
    let workspace = point_workspace::open(&workspace_root, index, OpenLimits::default())
        .blocking_wait()
        .expect("copied frozen Workspace opens through its public owner");
    let target = temporary.path().join(COMPLETE_ARTIFACT);
    let work = work_path(&target);
    fs::copy(fixture_path(WORK_CHECKPOINT), &work).unwrap();

    faults.mark_changed();
    faults.fail_at_ordinal(0);
    let surface = prepare_surface(workspace.head(), &target)
        .expect("frozen checkpoint resumes without rereading Snapshot rows");
    assert_eq!(
        surface.report().disposition(),
        TerrainPrepareDisposition::ResumedInput
    );
    assert_eq!(surface.report().source_points_read(), 0);
    assert_eq!(surface.report().reused_input_points(), 6);
    assert_surface_facts(surface.descriptor());
    assert_eq!(
        fs::read(&target).unwrap(),
        fs::read(fixture_path(COMPLETE_ARTIFACT)).unwrap(),
        "resumed work must rebuild byte-identical canonical output"
    );
    assert_eq!(
        fs::read(&work).unwrap(),
        fs::read(fixture_path(WORK_CHECKPOINT)).unwrap(),
        "verified checkpoint copy is retained unchanged for race-safe recovery"
    );
    drop(workspace);
    assert_eq!(
        fs::read(workspace_root.join("manifest.pwm")).unwrap(),
        copied_workspace_before
    );
    assert_eq!(frozen_tree_hashes(), frozen_before);
}

#[test]
#[ignore = "reproduces immutable fixtures only with explicit regeneration gates"]
fn regenerate_v1_surface_fixtures() {
    assert_eq!(
        std::env::var(REGENERATION_GATE).as_deref(),
        Ok("1"),
        "set {REGENERATION_GATE}=1 to regenerate frozen Terrain bytes"
    );
    assert_eq!(
        point_terrain::SURFACE_DISK_VERSION,
        1,
        "the v1 generator must never overwrite fixtures with a later disk version"
    );
    assert_eq!(
        point_terrain::ALGORITHM_VERSION,
        1,
        "the v1 generator must never overwrite fixtures with later algorithm semantics"
    );
    fs::create_dir_all(fixture_path("workspace")).unwrap();
    bootstrap_workspace_manifest_if_explicitly_requested();
    assert!(
        fixture_path(WORKSPACE_MANIFEST).is_file(),
        "the frozen Workspace anchor is missing; bootstrapping it also requires {BOOTSTRAP_GATE}=1"
    );

    let temporary = TemporaryFixture::new("v1-regeneration");
    let (index, _) = prepare_fixed_index(&temporary);
    let workspace_root = temporary.path().join("workspace-copy.pcw");
    copy_frozen_workspace(&workspace_root);
    let workspace = point_workspace::open(&workspace_root, index, OpenLimits::default())
        .blocking_wait()
        .expect("frozen Workspace anchor reopens for regeneration");
    let snapshot = workspace.head();

    let complete_target = temporary.path().join("generated-complete.pterr");
    let complete = prepare_surface(snapshot.clone(), &complete_target)
        .expect("generate complete disk-v1 Surface");
    assert_eq!(
        complete.report().disposition(),
        TerrainPrepareDisposition::Built
    );

    let work_target = temporary.path().join("generated-work.pterr");
    let defaults = TerrainPrepareLimits::default();
    let work_only_limits = TerrainPrepareLimits::new(
        defaults.derivation(),
        defaults.max_work_bytes(),
        1,
        defaults.max_temporary_bytes(),
        defaults.max_verify_buffer_bytes(),
        defaults.max_retained_handle_bytes(),
        defaults.max_path_bytes(),
    );
    let error = point_terrain::prepare(snapshot.clone(), &work_target, recipe(), work_only_limits)
        .blocking_wait()
        .expect_err("artifact ceiling leaves one complete input checkpoint");
    assert!(matches!(
        error,
        point_terrain::TerrainError::ResourceLimit {
            limit: "Surface artifact bytes",
            ..
        }
    ));
    let generated_work = work_path(&work_target);
    assert!(generated_work.is_file());

    let verification_target = temporary.path().join("verified-resume.pterr");
    fs::copy(&generated_work, work_path(&verification_target)).unwrap();
    let resumed = prepare_surface(snapshot, &verification_target)
        .expect("generated work resumes before fixture publication");
    assert_eq!(
        resumed.report().disposition(),
        TerrainPrepareDisposition::ResumedInput
    );
    assert_eq!(
        fs::read(&verification_target).unwrap(),
        fs::read(&complete_target).unwrap(),
        "generator refuses noncanonical work bytes"
    );

    publish_reproduced_file(
        &complete_target,
        &fixture_path(COMPLETE_ARTIFACT),
        "complete Surface artifact",
    );
    publish_reproduced_file(
        &generated_work,
        &fixture_path(WORK_CHECKPOINT),
        "Surface input checkpoint",
    );
    write_manifest(complete.descriptor());
}

fn assert_surface_facts(descriptor: &SurfaceArtifactDescriptor) {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture_path("manifest.json")).expect("read Surface fixture manifest"),
    )
    .expect("parse Surface fixture manifest");
    assert_eq!(descriptor.algorithm_version(), 1);
    assert_eq!(descriptor.recipe(), recipe());
    assert_eq!(descriptor.position_transform(), fixture_transform());
    assert_eq!(descriptor.coordinate_reference(), &fixture_reference());
    assert_eq!(
        manifest["snapshot"]["workspace"],
        descriptor.snapshot().workspace().to_string()
    );
    assert_eq!(
        manifest["snapshot"]["source"],
        descriptor.snapshot().source().to_string()
    );
    assert_eq!(
        manifest["snapshot"]["revision"],
        descriptor.snapshot().revision().to_string()
    );
    assert_eq!(
        manifest["surface"]["recipe_hash"],
        descriptor.recipe_hash().to_string()
    );
    assert_eq!(
        manifest["surface"]["input_hash"],
        descriptor.input_hash().to_string()
    );
    assert_eq!(
        manifest["surface"]["geometry_hash"],
        descriptor.geometry_hash().to_string()
    );
    assert_eq!(
        manifest["surface"]["topology_hash"],
        descriptor.topology_hash().to_string()
    );
    assert_eq!(
        manifest["surface"]["artifact_hash"],
        descriptor.artifact_hash().to_string()
    );
    assert_eq!(
        manifest["surface"]["input_point_count"],
        descriptor.input_point_count()
    );
    assert_eq!(
        manifest["surface"]["vertex_count"],
        descriptor.vertex_count()
    );
    assert_eq!(manifest["surface"]["face_count"], descriptor.face_count());
    assert_eq!(
        manifest["surface"]["hull_vertex_count"],
        descriptor.hull_vertex_count()
    );
    assert_eq!(
        manifest["surface"]["bounds"],
        serde_json::json!({
            "minimum": descriptor.bounds().min(),
            "maximum": descriptor.bounds().max()
        })
    );
}

fn prepare_surface(
    snapshot: Snapshot,
    target: &Path,
) -> Result<PreparedTerrainSurface, point_terrain::TerrainError> {
    point_terrain::prepare(snapshot, target, recipe(), TerrainPrepareLimits::default())
        .blocking_wait()
}

fn collect_vertices(surface: &PreparedTerrainSurface) -> Vec<point_terrain::SurfaceVertex> {
    surface
        .vertex_batches(read_limits())
        .expect("open frozen vertex stream")
        .flat_map(|batch| batch.expect("read frozen vertex batch"))
        .collect()
}

fn collect_faces(surface: &PreparedTerrainSurface) -> Vec<point_terrain::SurfaceFace> {
    surface
        .face_batches(read_limits())
        .expect("open frozen face stream")
        .flat_map(|batch| batch.expect("read frozen face batch"))
        .collect()
}

fn read_limits() -> SurfaceReadLimits {
    SurfaceReadLimits::new(2, 1024, 128 * 1024, 1024 * 1024, 1_000_000)
}

fn recipe() -> TerrainRecipe {
    TerrainRecipe::new(GROUND_CLASSIFICATION).within(
        WorldBounds::new(
            [399_999.0, 1_499_999.0, 0.0],
            [400_004.0, 1_500_004.0, 20.0],
        )
        .unwrap(),
    )
}

fn prepare_fixed_index(temporary: &TemporaryFixture) -> (PreparedIndex, MemoryFaultControl) {
    let ticks = fixture_ticks();
    let attributes = fixture_attributes();
    let initial = MemorySource::from_columns(
        fixture_transform(),
        fixture_reference(),
        ticks.clone(),
        attributes.clone(),
    )
    .expect("fixed Terrain MemorySource is valid");
    let initial = source_memory::open(initial)
        .blocking_wait()
        .expect("fixed Terrain MemorySource verifies");
    let expected_identity = initial.identity();
    let (controlled, faults) =
        MemorySource::with_fault_control(initial.metadata().clone(), ticks, attributes)
            .expect("controlled Terrain MemorySource matches inferred metadata");
    let source = source_memory::open(controlled)
        .blocking_wait()
        .expect("controlled Terrain MemorySource verifies");
    assert_eq!(source.identity(), expected_identity);
    let index = point_index::prepare(
        source,
        temporary.path().join("fixture.pidx"),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .expect("fresh index prepares over fixed Terrain Source");
    (index, faults)
}

fn fixture_ticks() -> Vec<[i64; 3]> {
    vec![
        [0, 0, 0],
        [10, 0, 2],
        [10, 10, 4],
        [0, 10, 6],
        [5, 5, 3],
        [3, 7, 4],
    ]
}

fn fixture_attributes() -> AttributeColumns {
    let definition = AttributeDefinition::new(
        AttributeId::new(CLASSIFICATION_ATTRIBUTE).unwrap(),
        "classification",
        AttributeDataType::U8,
    )
    .unwrap();
    let values = AttributeValues::u8(vec![GROUND_CLASSIFICATION; fixture_ticks().len()]);
    let column = AttributeColumn::new(definition, values).unwrap();
    AttributeColumns::new(vec![column], fixture_ticks().len()).unwrap()
}

fn fixture_transform() -> PositionTransform {
    PositionTransform::new([400_000.0, 1_500_000.0, 10.0], [0.25, 0.25, 0.1]).unwrap()
}

fn fixture_reference() -> CoordinateReference {
    CoordinateReference::profile(
        SpatialReferenceProfile::new(
            32_647,
            5_703,
            SpatialAxes::EastingNorthingElevation,
            LinearUnit::Metre,
            LinearUnit::Metre,
            SpatialReferenceProvenance::CallerDeclaration,
        )
        .unwrap(),
    )
}

fn copy_frozen_workspace(target: &Path) {
    fs::create_dir(target).unwrap();
    for child in ["operations", "revisions", "scratch"] {
        fs::create_dir(target.join(child)).unwrap();
    }
    fs::copy(
        fixture_path(WORKSPACE_MANIFEST),
        target.join("manifest.pwm"),
    )
    .unwrap();
}

fn bootstrap_workspace_manifest_if_explicitly_requested() {
    let destination = fixture_path(WORKSPACE_MANIFEST);
    if destination.exists() {
        return;
    }
    assert_eq!(
        std::env::var(BOOTSTRAP_GATE).as_deref(),
        Ok("1"),
        "creating a new frozen Workspace identity requires {BOOTSTRAP_GATE}=1"
    );
    let temporary = TemporaryFixture::new("v1-workspace-bootstrap");
    let (index, _) = prepare_fixed_index(&temporary);
    let root = temporary.path().join("workspace-bootstrap.pcw");
    let workspace = point_workspace::create(
        &root,
        index,
        WorkspaceSchema::new(AttributeId::new(CLASSIFICATION_ATTRIBUTE).unwrap()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("bootstrap frozen Workspace anchor");
    assert_eq!(workspace.head().provenance().source(), workspace.source());
    drop(workspace);
    fs::copy(root.join("manifest.pwm"), destination).unwrap();
}

fn write_manifest(descriptor: &SurfaceArtifactDescriptor) {
    let complete = fs::read(fixture_path(COMPLETE_ARTIFACT)).unwrap();
    let work = fs::read(fixture_path(WORK_CHECKPOINT)).unwrap();
    let workspace = fs::read(fixture_path(WORKSPACE_MANIFEST)).unwrap();
    let surface = serde_json::json!({
        "recipe_hash": descriptor.recipe_hash().to_string(),
        "input_hash": descriptor.input_hash().to_string(),
        "geometry_hash": descriptor.geometry_hash().to_string(),
        "topology_hash": descriptor.topology_hash().to_string(),
        "artifact_hash": descriptor.artifact_hash().to_string(),
        "input_point_count": descriptor.input_point_count(),
        "vertex_count": descriptor.vertex_count(),
        "face_count": descriptor.face_count(),
        "hull_vertex_count": descriptor.hull_vertex_count(),
        "bounds": {
            "minimum": descriptor.bounds().min(),
            "maximum": descriptor.bounds().max()
        }
    });
    let manifest = serde_json::json!({
        "schema": FIXTURE_SCHEMA,
        "owner": "point-terrain",
        "support_class": "rebuildable",
        "path_base": "manifest_directory",
        "disk_version": point_terrain::SURFACE_DISK_VERSION,
        "algorithm_version": descriptor.algorithm_version(),
        "snapshot": {
            "workspace": descriptor.snapshot().workspace().to_string(),
            "source": descriptor.snapshot().source().to_string(),
            "revision": descriptor.snapshot().revision().to_string()
        },
        "source": {
            "generator": "fixed supported-profile MemorySource in tests/v1_fixtures.rs",
            "identity": descriptor.snapshot().source().to_string(),
            "point_count": 6,
            "position_transform": {
                "offset": fixture_transform().offset(),
                "scale": fixture_transform().scale()
            },
            "coordinate_reference": {
                "horizontal_epsg": 32647,
                "vertical_epsg": 5703,
                "axes": "easting_northing_elevation",
                "horizontal_unit": "metre",
                "vertical_unit": "metre",
                "provenance": "caller_declaration"
            },
            "classification_attribute": CLASSIFICATION_ATTRIBUTE,
            "classification_values": vec![GROUND_CLASSIFICATION; 6],
            "point_ticks": fixture_ticks()
        },
        "recipe": {
            "ground_classification": GROUND_CLASSIFICATION,
            "bounds": {
                "minimum": recipe().bounds().unwrap().min(),
                "maximum": recipe().bounds().unwrap().max()
            }
        },
        "surface": surface.clone(),
        "files": [
            file_manifest(COMPLETE_ARTIFACT, "complete", &complete, &surface),
            file_manifest(
                WORK_CHECKPOINT,
                "resumable-input",
                &work,
                &serde_json::json!({
                    "durable_input_points": descriptor.input_point_count(),
                    "resume_disposition": "resumed_input",
                    "source_points_read": 0,
                    "rebuild_equals_complete": true
                })
            ),
            file_manifest(
                WORKSPACE_MANIFEST,
                "snapshot-binding",
                &workspace,
                &serde_json::json!({
                    "workspace": descriptor.snapshot().workspace().to_string(),
                    "source": descriptor.snapshot().source().to_string(),
                    "revision": descriptor.snapshot().revision().to_string()
                })
            )
        ]
    });
    let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    bytes.push(b'\n');
    publish_reproduced_bytes(
        &fixture_path("manifest.json"),
        &bytes,
        "Surface fixture manifest",
    );
}

fn file_manifest(
    path: &str,
    role: &str,
    bytes: &[u8],
    semantic_facts: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "role": role,
        "byte_length": bytes.len(),
        "blake3": blake3::hash(bytes).to_hex().to_string(),
        "semantic_facts": semantic_facts
    })
}

fn publish_reproduced_file(generated: &Path, frozen: &Path, label: &str) {
    let bytes = fs::read(generated).unwrap();
    publish_reproduced_bytes(frozen, &bytes, label);
}

fn publish_reproduced_bytes(frozen: &Path, bytes: &[u8], label: &str) {
    match fs::read(frozen) {
        Ok(expected) => assert_eq!(
            bytes, expected,
            "{label} no longer reproduces frozen disk-v1 truth; preserve v1 and add a later version"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::write(frozen, bytes).unwrap();
        }
        Err(error) => panic!("read frozen {label}: {error}"),
    }
}

fn work_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap().to_os_string();
    name.push(".surface-work-v1");
    target.with_file_name(name)
}

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_ROOT)
        .join(relative)
}

fn frozen_tree_hashes() -> Vec<(String, u64, String)> {
    let root = fixture_path("");
    let mut files = Vec::new();
    collect_hashes(&root, &root, &mut files);
    files.sort();
    files
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

struct TemporaryFixture {
    path: PathBuf,
}

impl TemporaryFixture {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "punctra-terrain-v1-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create isolated Terrain v1 fixture directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
