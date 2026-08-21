//! Public acceptance boundaries for durable Surface determinism and resources.

mod support;

use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem,
    path::{Path, PathBuf},
};

use point_contracts::WorldBounds;
use point_terrain::{
    PreparedTerrainSurface, SurfaceFace, SurfaceReadLimits, SurfaceVertex, TerrainError,
    TerrainLimits, TerrainPrepareDisposition, TerrainPrepareLimits, TerrainRecipe, prepare,
};
use point_workspace::Snapshot;

use support::{TerrainFixture, terrain_limits_with_row_batch};

const ARTIFACT_HEADER_BYTES: u64 = 576;
const VERTEX_DISK_BYTES: u64 = 32;
const FACE_DISK_BYTES: u64 = 12;
const RECORDS_PER_BLOCK: u64 = 4_096;

#[test]
fn cold_artifact_bytes_do_not_depend_on_point_row_batching() {
    let fixture = small_fixture("persistence-acceptance-batching");
    let snapshot = fixture.snapshot();
    let recipe = small_recipe();
    let mut artifacts = Vec::new();

    for (index, row_batch) in [1, 2, 6].into_iter().enumerate() {
        let target = fixture.terrain_path(&format!("batched-{index}.pterr"));
        let limits = PrepareCeilings::default()
            .with_derivation(terrain_limits_with_row_batch(row_batch))
            .build();
        let surface = prepare_surface(snapshot.clone(), &target, recipe, limits)
            .expect("differently batched cold Surface prepares");
        assert_eq!(
            surface.report().disposition(),
            TerrainPrepareDisposition::Built
        );
        artifacts.push((surface.descriptor().clone(), fs::read(target).unwrap()));
    }

    for candidate in &artifacts[1..] {
        assert_eq!(candidate, &artifacts[0]);
    }
}

#[test]
fn every_prepare_storage_ceiling_is_inclusive_and_rejects_one_under() {
    let fixture = small_fixture("persistence-acceptance-prepare-limits");
    let snapshot = fixture.snapshot();
    let recipe = small_recipe();
    let baseline_target = fixture.terrain_path("baseline.pterr");
    let baseline = prepare_surface(
        snapshot.clone(),
        &baseline_target,
        recipe,
        TerrainPrepareLimits::default(),
    )
    .unwrap();
    let artifact_bytes = baseline.report().artifact_bytes();
    let temporary_bytes = baseline.report().peak_temporary_disk_bytes();
    let handle_bytes = baseline.report().accounted_handle_bytes();
    let work_bytes = fs::metadata(sibling(&baseline_target, ".surface-work-v1"))
        .unwrap()
        .len();

    assert_prepare_boundary(
        &fixture,
        &snapshot,
        recipe,
        "work",
        "Surface work checkpoint bytes",
        work_bytes,
        PrepareCeilings::with_work_bytes,
    );
    assert_prepare_boundary(
        &fixture,
        &snapshot,
        recipe,
        "artifact",
        "Surface artifact bytes",
        artifact_bytes,
        PrepareCeilings::with_artifact_bytes,
    );
    assert_prepare_boundary(
        &fixture,
        &snapshot,
        recipe,
        "temporary",
        "Surface temporary bytes",
        temporary_bytes,
        PrepareCeilings::with_temporary_bytes,
    );
    assert_prepare_boundary(
        &fixture,
        &snapshot,
        recipe,
        "verify",
        "Surface checksum verification buffer bytes",
        32,
        PrepareCeilings::with_verify_buffer_bytes,
    );
    assert_prepare_handle_boundary(snapshot.clone(), &baseline_target, recipe, handle_bytes);
    assert_prepare_path_boundary(&fixture, snapshot, recipe);
}

#[test]
fn every_surface_read_ceiling_is_inclusive_and_rejects_one_under() {
    let fixture = small_fixture("persistence-acceptance-read-limits");
    let target = fixture.terrain_path("surface.pterr");
    let surface = prepare_surface(
        fixture.snapshot(),
        &target,
        small_recipe(),
        TerrainPrepareLimits::default(),
    )
    .unwrap();
    let record_bytes = u64::try_from(mem::size_of::<SurfaceVertex>()).unwrap();
    let generous_work = 1_000_000;

    assert_read_record_boundary(&surface, generous_work);
    let exact_payload_bytes = assert_read_payload_boundary(&surface, record_bytes, generous_work);
    let exact_verify_bytes = assert_read_verify_boundary(&surface, generous_work);
    assert_read_working_boundary(
        &surface,
        exact_payload_bytes,
        exact_verify_bytes,
        generous_work,
    );
    assert_read_work_unit_boundary(&surface, record_bytes);
}

#[test]
fn persistent_prepare_forwards_each_observable_derivation_ceiling() {
    let fixture = small_fixture("persistence-acceptance-derivation-limits");
    let snapshot = fixture.snapshot();
    let recipe = small_recipe();
    let baseline = point_terrain::derive(snapshot.clone(), recipe, TerrainLimits::default())
        .blocking_wait()
        .unwrap();
    let descriptor = baseline.descriptor();
    let exact_input = descriptor.input_point_count();
    let exact_vertices = descriptor.vertex_count();
    let exact_faces = descriptor.face_count();
    let exact_working = descriptor.accounted_peak_working_bytes();
    let exact_surface = descriptor.retained_surface_bytes();
    let exact_work_units = minimum_derivation_work_units(&snapshot, recipe);
    let working_resource = derivation_resource_at_one_under(
        snapshot.clone(),
        recipe,
        DerivationCeilings::default().with_working_bytes(exact_working - 1),
        exact_working,
    );
    let work_unit_resource = derivation_resource_at_one_under(
        snapshot.clone(),
        recipe,
        DerivationCeilings::default().with_work_units(exact_work_units - 1),
        exact_work_units,
    );

    assert_derivation_boundary(
        &fixture,
        &snapshot,
        recipe,
        "input",
        "Ground Input Points",
        exact_input,
        DerivationCeilings::with_input_points,
    );
    assert_derivation_boundary(
        &fixture,
        &snapshot,
        recipe,
        "vertices",
        "Terrain vertices",
        exact_vertices,
        DerivationCeilings::with_vertices,
    );
    assert_derivation_boundary(
        &fixture,
        &snapshot,
        recipe,
        "faces",
        "Terrain faces",
        exact_faces,
        DerivationCeilings::with_faces,
    );
    assert_derivation_boundary(
        &fixture,
        &snapshot,
        recipe,
        "working",
        working_resource,
        exact_working,
        DerivationCeilings::with_working_bytes,
    );
    assert_derivation_boundary(
        &fixture,
        &snapshot,
        recipe,
        "surface",
        "retained Terrain Surface bytes",
        exact_surface,
        DerivationCeilings::with_surface_bytes,
    );
    assert_derivation_boundary(
        &fixture,
        &snapshot,
        recipe,
        "work-units",
        work_unit_resource,
        exact_work_units,
        DerivationCeilings::with_work_units,
    );
}

#[test]
fn multi_block_surface_streams_partition_and_revalidate_later_blocks() {
    let fixture = multi_block_fixture("persistence-acceptance-multi-block");
    let target = fixture.terrain_path("surface.pterr");
    let surface = prepare_surface(
        fixture.snapshot(),
        &target,
        multi_block_recipe(),
        TerrainPrepareLimits::default(),
    )
    .unwrap();
    assert_eq!(surface.descriptor().vertex_count(), 4_100);

    let expected = collect_vertices(&surface, read_limits(4_096)).unwrap();
    let (unit_vertices, unit_batch_count) = collect_unit_vertex_batches(&surface).unwrap();
    assert_eq!(unit_batch_count, 4_100);
    assert_eq!(unit_vertices, expected);
    for (batch_records, expected_lengths) in [
        (4_095, vec![4_095, 5]),
        (4_096, vec![4_096, 4]),
        (4_097, vec![4_097, 3]),
    ] {
        let (vertices, lengths) =
            collect_vertices_with_lengths(&surface, read_limits(batch_records))
                .expect("multi-block Surface stream reads");
        assert_eq!(lengths, expected_lengths);
        assert_eq!(vertices, expected);
    }

    let expected_faces = collect_faces(&surface, read_limits(4_096)).unwrap();
    assert!(u64::try_from(expected_faces.len()).unwrap() > RECORDS_PER_BLOCK);
    let (unit_faces, unit_face_batch_count) = collect_unit_face_batches(&surface).unwrap();
    assert_eq!(unit_face_batch_count, expected_faces.len());
    assert_eq!(unit_faces, expected_faces);
    for batch_records in [4_095, 4_096, 4_097] {
        let (faces, lengths) = collect_faces_with_lengths(&surface, read_limits(batch_records))
            .expect("multi-block face stream reads");
        assert_eq!(
            lengths,
            expected_batch_lengths(surface.descriptor().face_count(), batch_records)
        );
        assert_eq!(faces, expected_faces);
    }

    let face_target = fixture.terrain_path("face-mutation.pterr");
    fs::copy(&target, &face_target).unwrap();
    let face_surface = prepare_surface(
        fixture.snapshot(),
        &face_target,
        multi_block_recipe(),
        TerrainPrepareLimits::default(),
    )
    .unwrap();

    let mut vertex_stream = surface
        .vertex_batches(read_limits(RECORDS_PER_BLOCK))
        .unwrap();
    let first = vertex_stream.next().unwrap().unwrap();
    assert_eq!(first.len(), usize::try_from(RECORDS_PER_BLOCK).unwrap());
    let second_block_record = ARTIFACT_HEADER_BYTES + RECORDS_PER_BLOCK * VERTEX_DISK_BYTES;
    flip_byte(&target, second_block_record + 7);
    assert!(matches!(
        vertex_stream.next().unwrap().unwrap_err(),
        TerrainError::CorruptSurfaceArtifact { .. }
    ));

    let mut face_stream = face_surface
        .face_batches(read_limits(RECORDS_PER_BLOCK))
        .unwrap();
    let first = face_stream.next().unwrap().unwrap();
    assert_eq!(first.len(), usize::try_from(RECORDS_PER_BLOCK).unwrap());
    let face_offset =
        ARTIFACT_HEADER_BYTES + surface.descriptor().vertex_count() * VERTEX_DISK_BYTES;
    let second_face_block = face_offset + RECORDS_PER_BLOCK * FACE_DISK_BYTES;
    flip_byte(&face_target, second_face_block + 5);
    assert!(matches!(
        face_stream.next().unwrap().unwrap_err(),
        TerrainError::CorruptSurfaceArtifact { .. }
    ));
}

fn assert_prepare_handle_boundary(
    snapshot: Snapshot,
    target: &Path,
    recipe: TerrainRecipe,
    exact: u64,
) {
    prepare_surface(
        snapshot.clone(),
        target,
        recipe,
        PrepareCeilings::default()
            .with_retained_handle_bytes(exact)
            .build(),
    )
    .expect("exact retained-handle ceiling warm-opens");
    let error = prepare_surface(
        snapshot,
        target,
        recipe,
        PrepareCeilings::default()
            .with_retained_handle_bytes(exact - 1)
            .build(),
    )
    .unwrap_err();
    assert_resource(error, "Surface retained handle bytes", exact, exact - 1);
}

fn assert_prepare_path_boundary(
    fixture: &TerrainFixture,
    snapshot: Snapshot,
    recipe: TerrainRecipe,
) {
    let exact_target = fixture.terrain_path("path-exact.pterr");
    let under_target = fixture.terrain_path("path-under.pterr");
    let exact = retained_family_path_bytes(&exact_target);
    assert_eq!(exact, retained_family_path_bytes(&under_target));
    prepare_surface(
        snapshot.clone(),
        &exact_target,
        recipe,
        PrepareCeilings::default().with_path_bytes(exact).build(),
    )
    .expect("exact retained-path ceiling prepares");
    let error = prepare_surface(
        snapshot,
        &under_target,
        recipe,
        PrepareCeilings::default()
            .with_path_bytes(exact - 1)
            .build(),
    )
    .unwrap_err();
    assert_resource(error, "Surface retained path bytes", exact, exact - 1);
}

fn assert_read_record_boundary(surface: &PreparedTerrainSurface, generous: u64) {
    let exact = SurfaceReadLimits::new(1, generous, 1, generous, generous);
    drain_vertices(surface, exact).expect("one-record vertex batch ceiling is inclusive");
    drain_faces(surface, exact).expect("one-record face batch ceiling is inclusive");

    let under = SurfaceReadLimits::new(0, generous, 1, generous, generous);
    assert_resource_result(surface.vertex_batches(under), "Surface vertices", 1, 0);
    assert_resource_result(surface.face_batches(under), "Surface faces", 1, 0);
}

fn assert_read_payload_boundary(
    surface: &PreparedTerrainSurface,
    candidate: u64,
    generous: u64,
) -> u64 {
    let exact = accepted_or_required(
        surface.vertex_batches(SurfaceReadLimits::new(
            1, candidate, generous, generous, generous,
        )),
        "Surface batch payload bytes",
        candidate,
    );
    drain_vertices(
        surface,
        SurfaceReadLimits::new(1, exact, generous, generous, generous),
    )
    .expect("exact batch-payload ceiling is inclusive");
    assert_resource_result(
        surface.vertex_batches(SurfaceReadLimits::new(
            1,
            exact - 1,
            generous,
            generous,
            generous,
        )),
        "Surface batch payload bytes",
        exact,
        exact - 1,
    );
    exact
}

fn assert_read_verify_boundary(surface: &PreparedTerrainSurface, generous: u64) -> u64 {
    let exact = accepted_or_required(
        surface.vertex_batches(SurfaceReadLimits::new(1, generous, 1, generous, generous)),
        "Surface read verification buffer bytes",
        1,
    );
    drain_vertices(
        surface,
        SurfaceReadLimits::new(1, generous, exact, generous, generous),
    )
    .expect("exact verification-buffer ceiling is inclusive");
    assert_resource_result(
        surface.vertex_batches(SurfaceReadLimits::new(
            1,
            generous,
            exact - 1,
            generous,
            generous,
        )),
        "Surface read verification buffer bytes",
        exact,
        exact - 1,
    );
    exact
}

fn assert_read_working_boundary(
    surface: &PreparedTerrainSurface,
    payload: u64,
    verify: u64,
    generous: u64,
) {
    let candidate = payload + 1;
    let exact = accepted_or_required(
        surface.vertex_batches(SurfaceReadLimits::new(
            1, payload, verify, candidate, generous,
        )),
        "Surface read working bytes",
        candidate,
    );
    drain_vertices(
        surface,
        SurfaceReadLimits::new(1, payload, verify, exact, generous),
    )
    .expect("exact stream working-byte ceiling is inclusive");
    assert_resource_result(
        surface.vertex_batches(SurfaceReadLimits::new(
            1,
            payload,
            verify,
            exact - 1,
            generous,
        )),
        "Surface read working bytes",
        exact,
        exact - 1,
    );
}

fn assert_read_work_unit_boundary(surface: &PreparedTerrainSurface, record_bytes: u64) {
    let zero_work = SurfaceReadLimits::new(2, record_bytes * 2, 1, record_bytes * 2 + 1, 0);
    let Err(probe) = surface.vertex_batches(zero_work) else {
        panic!("zero work-unit ceiling unexpectedly opened a stream");
    };
    let exact = resource_required(probe, "Surface read work units", 0);
    drain_vertices(
        surface,
        SurfaceReadLimits::new(2, record_bytes * 2, 1, record_bytes * 2 + 1, exact),
    )
    .expect("exact complete-stream work ceiling is inclusive");
    assert_resource_result(
        surface.vertex_batches(SurfaceReadLimits::new(
            2,
            record_bytes * 2,
            1,
            record_bytes * 2 + 1,
            exact - 1,
        )),
        "Surface read work units",
        exact,
        exact - 1,
    );
}

fn assert_prepare_boundary(
    fixture: &TerrainFixture,
    snapshot: &Snapshot,
    recipe: TerrainRecipe,
    name: &str,
    resource: &'static str,
    exact: u64,
    configure: impl Fn(PrepareCeilings, u64) -> PrepareCeilings,
) {
    let exact_target = fixture.terrain_path(&format!("{name}-exact.pterr"));
    prepare_surface(
        snapshot.clone(),
        &exact_target,
        recipe,
        configure(PrepareCeilings::default(), exact).build(),
    )
    .expect("exact preparation ceiling is inclusive");

    let under_target = fixture.terrain_path(&format!("{name}-under.pterr"));
    let error = prepare_surface(
        snapshot.clone(),
        &under_target,
        recipe,
        configure(PrepareCeilings::default(), exact - 1).build(),
    )
    .unwrap_err();
    assert_resource(error, resource, exact, exact - 1);
}

fn assert_derivation_boundary(
    fixture: &TerrainFixture,
    snapshot: &Snapshot,
    recipe: TerrainRecipe,
    name: &str,
    resource: &'static str,
    exact: u64,
    configure: impl Fn(DerivationCeilings, u64) -> DerivationCeilings,
) {
    let exact_limits = configure(DerivationCeilings::default(), exact).build();
    let exact_target = fixture.terrain_path(&format!("derive-{name}-exact.pterr"));
    prepare_surface(
        snapshot.clone(),
        &exact_target,
        recipe,
        PrepareCeilings::default()
            .with_derivation(exact_limits)
            .build(),
    )
    .expect("exact forwarded derivation ceiling prepares");

    let under_limits = configure(DerivationCeilings::default(), exact - 1).build();
    let under_target = fixture.terrain_path(&format!("derive-{name}-under.pterr"));
    let error = prepare_surface(
        snapshot.clone(),
        &under_target,
        recipe,
        PrepareCeilings::default()
            .with_derivation(under_limits)
            .build(),
    )
    .unwrap_err();
    assert_resource(error, resource, exact, exact - 1);
}

fn minimum_derivation_work_units(snapshot: &Snapshot, recipe: TerrainRecipe) -> u64 {
    let mut failing = 0;
    let mut passing = 1;
    while !work_limited_derivation_succeeds(snapshot.clone(), recipe, passing) {
        failing = passing;
        passing = passing.checked_mul(2).expect("fixture work fits u64");
    }
    while failing + 1 < passing {
        let candidate = failing + (passing - failing) / 2;
        if work_limited_derivation_succeeds(snapshot.clone(), recipe, candidate) {
            passing = candidate;
        } else {
            failing = candidate;
        }
    }
    passing
}

fn work_limited_derivation_succeeds(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
    max_work_units: u64,
) -> bool {
    match derive_with_work_units(snapshot, recipe, max_work_units) {
        Ok(_) => true,
        Err(TerrainError::ResourceLimit {
            limit: "Terrain Derivation work units" | "max_steps",
            ..
        }) => false,
        Err(other) => panic!("work-unit search found an unrelated failure: {other:?}"),
    }
}

fn derivation_resource_at_one_under(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
    limits: DerivationCeilings,
    required: u64,
) -> &'static str {
    let Err(error) = point_terrain::derive(snapshot, recipe, limits.build()).blocking_wait() else {
        panic!("one-under derivation ceiling unexpectedly succeeded");
    };
    match error {
        TerrainError::ResourceLimit {
            limit,
            required: actual_required,
            allowed,
        } => {
            assert_eq!(actual_required, required);
            assert_eq!(allowed, required - 1);
            limit
        }
        other => panic!("expected one-under derivation ResourceLimit, found {other:?}"),
    }
}

fn derive_with_work_units(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
    max_work_units: u64,
) -> Result<point_terrain::TerrainSurface, TerrainError> {
    point_terrain::derive(
        snapshot,
        recipe,
        DerivationCeilings::default()
            .with_work_units(max_work_units)
            .build(),
    )
    .blocking_wait()
}

fn prepare_surface(
    snapshot: Snapshot,
    target: &Path,
    recipe: TerrainRecipe,
    limits: TerrainPrepareLimits,
) -> Result<PreparedTerrainSurface, TerrainError> {
    prepare(snapshot, target, recipe, limits).blocking_wait()
}

fn drain_vertices(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<(), TerrainError> {
    for batch in surface.vertex_batches(limits)? {
        drop(batch?);
    }
    Ok(())
}

fn drain_faces(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<(), TerrainError> {
    for batch in surface.face_batches(limits)? {
        drop(batch?);
    }
    Ok(())
}

fn collect_unit_vertex_batches(
    surface: &PreparedTerrainSurface,
) -> Result<(Vec<SurfaceVertex>, usize), TerrainError> {
    let mut vertices = Vec::new();
    let mut batch_count = 0;
    for batch in surface.vertex_batches(read_limits(1))? {
        let batch = batch?;
        assert_eq!(batch.len(), 1);
        batch_count += 1;
        vertices.extend(batch);
    }
    Ok((vertices, batch_count))
}

fn collect_unit_face_batches(
    surface: &PreparedTerrainSurface,
) -> Result<(Vec<SurfaceFace>, usize), TerrainError> {
    let mut faces = Vec::new();
    let mut batch_count = 0;
    for batch in surface.face_batches(read_limits(1))? {
        let batch = batch?;
        assert_eq!(batch.len(), 1);
        batch_count += 1;
        faces.extend(batch);
    }
    Ok((faces, batch_count))
}

fn collect_vertices(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<Vec<SurfaceVertex>, TerrainError> {
    Ok(collect_vertices_with_lengths(surface, limits)?.0)
}

fn collect_vertices_with_lengths(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<(Vec<SurfaceVertex>, Vec<usize>), TerrainError> {
    let mut vertices = Vec::new();
    let mut lengths = Vec::new();
    for batch in surface.vertex_batches(limits)? {
        let batch = batch?;
        lengths.push(batch.len());
        vertices.extend(batch);
    }
    Ok((vertices, lengths))
}

fn collect_faces(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<Vec<SurfaceFace>, TerrainError> {
    Ok(collect_faces_with_lengths(surface, limits)?.0)
}

fn collect_faces_with_lengths(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<(Vec<SurfaceFace>, Vec<usize>), TerrainError> {
    let mut faces = Vec::new();
    let mut lengths = Vec::new();
    for batch in surface.face_batches(limits)? {
        let batch = batch?;
        lengths.push(batch.len());
        faces.extend(batch);
    }
    Ok((faces, lengths))
}

fn expected_batch_lengths(total_records: u64, batch_records: u64) -> Vec<usize> {
    let mut remaining = total_records;
    let mut lengths = Vec::new();
    while remaining > 0 {
        let count = remaining.min(batch_records);
        lengths.push(usize::try_from(count).unwrap());
        remaining -= count;
    }
    lengths
}

fn read_limits(batch_records: u64) -> SurfaceReadLimits {
    SurfaceReadLimits::new(
        batch_records,
        1024 * 1024,
        128 * 1024,
        2 * 1024 * 1024,
        100_000_000,
    )
}

fn assert_resource_result<T>(
    result: Result<T, TerrainError>,
    resource: &'static str,
    required: u64,
    allowed: u64,
) {
    let Err(error) = result else {
        panic!("{resource} one-under ceiling unexpectedly succeeded");
    };
    assert_resource(error, resource, required, allowed);
}

fn accepted_or_required<T>(
    result: Result<T, TerrainError>,
    resource: &'static str,
    candidate: u64,
) -> u64 {
    match result {
        Ok(_) => candidate,
        Err(error) => resource_required(error, resource, candidate),
    }
}

fn resource_required(error: TerrainError, resource: &'static str, allowed: u64) -> u64 {
    match error {
        TerrainError::ResourceLimit {
            limit,
            required,
            allowed: actual_allowed,
        } => {
            assert_eq!(limit, resource);
            assert_eq!(actual_allowed, allowed);
            required
        }
        other => panic!("expected {resource} ResourceLimit, found {other:?}"),
    }
}

fn assert_resource(error: TerrainError, resource: &'static str, required: u64, allowed: u64) {
    match error {
        TerrainError::ResourceLimit {
            limit,
            required: actual_required,
            allowed: actual_allowed,
        } => {
            assert_eq!(limit, resource);
            assert_eq!(actual_required, required);
            assert_eq!(actual_allowed, allowed);
        }
        other => panic!("expected {resource} ResourceLimit, found {other:?}"),
    }
}

fn small_fixture(label: &str) -> TerrainFixture {
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

fn multi_block_fixture(label: &str) -> TerrainFixture {
    let ticks = (0_i64..4_100)
        .map(|index| {
            let column = index % 64;
            let row = index / 64;
            [
                column * 100 + (row * 17 + column * 13) % 23,
                row * 100 + (column * 19 + row * 11) % 29,
                (column * 7 + row * 5) % 41,
            ]
        })
        .collect::<Vec<_>>();
    TerrainFixture::new(label, ticks, vec![2; 4_100])
}

fn small_recipe() -> TerrainRecipe {
    TerrainRecipe::new(2).within(WorldBounds::new([-1.0, -1.0, -10.0], [11.0, 11.0, 20.0]).unwrap())
}

fn multi_block_recipe() -> TerrainRecipe {
    TerrainRecipe::new(2)
        .within(WorldBounds::new([-1.0, -1.0, -100.0], [7_000.0, 7_000.0, 100.0]).unwrap())
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target.file_name().unwrap().to_os_string();
    name.push(suffix);
    target.with_file_name(name)
}

fn retained_family_path_bytes(target: &Path) -> u64 {
    [
        target.to_path_buf(),
        sibling(target, ".surface-work-v1"),
        sibling(target, ".surface-stage-v1"),
    ]
    .into_iter()
    .map(|path| u64::try_from(path.as_os_str().as_encoded_bytes().len()).unwrap())
    .max()
    .unwrap()
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

#[derive(Clone, Copy)]
struct PrepareCeilings {
    derivation: TerrainLimits,
    work_bytes: u64,
    artifact_bytes: u64,
    temporary_bytes: u64,
    verify_buffer_bytes: u64,
    retained_handle_bytes: u64,
    path_bytes: u64,
}

impl PrepareCeilings {
    fn with_derivation(mut self, value: TerrainLimits) -> Self {
        self.derivation = value;
        self
    }

    fn with_work_bytes(mut self, value: u64) -> Self {
        self.work_bytes = value;
        self
    }

    fn with_artifact_bytes(mut self, value: u64) -> Self {
        self.artifact_bytes = value;
        self
    }

    fn with_temporary_bytes(mut self, value: u64) -> Self {
        self.temporary_bytes = value;
        self
    }

    fn with_verify_buffer_bytes(mut self, value: u64) -> Self {
        self.verify_buffer_bytes = value;
        self
    }

    fn with_retained_handle_bytes(mut self, value: u64) -> Self {
        self.retained_handle_bytes = value;
        self
    }

    fn with_path_bytes(mut self, value: u64) -> Self {
        self.path_bytes = value;
        self
    }

    const fn build(self) -> TerrainPrepareLimits {
        TerrainPrepareLimits::new(
            self.derivation,
            self.work_bytes,
            self.artifact_bytes,
            self.temporary_bytes,
            self.verify_buffer_bytes,
            self.retained_handle_bytes,
            self.path_bytes,
        )
    }
}

impl Default for PrepareCeilings {
    fn default() -> Self {
        let limits = TerrainPrepareLimits::default();
        Self {
            derivation: limits.derivation(),
            work_bytes: limits.max_work_bytes(),
            artifact_bytes: limits.max_artifact_bytes(),
            temporary_bytes: limits.max_temporary_bytes(),
            verify_buffer_bytes: limits.max_verify_buffer_bytes(),
            retained_handle_bytes: limits.max_retained_handle_bytes(),
            path_bytes: limits.max_path_bytes(),
        }
    }
}

#[derive(Clone, Copy)]
struct DerivationCeilings {
    limits: TerrainLimits,
    input_points: u64,
    vertices: u64,
    faces: u64,
    working_bytes: u64,
    surface_bytes: u64,
    work_units: u64,
}

impl DerivationCeilings {
    fn with_input_points(mut self, value: u64) -> Self {
        self.input_points = value;
        self
    }

    fn with_vertices(mut self, value: u64) -> Self {
        self.vertices = value;
        self
    }

    fn with_faces(mut self, value: u64) -> Self {
        self.faces = value;
        self
    }

    fn with_working_bytes(mut self, value: u64) -> Self {
        self.working_bytes = value;
        self
    }

    fn with_surface_bytes(mut self, value: u64) -> Self {
        self.surface_bytes = value;
        self
    }

    fn with_work_units(mut self, value: u64) -> Self {
        self.work_units = value;
        self
    }

    const fn build(self) -> TerrainLimits {
        TerrainLimits::new(
            self.limits.point_rows(),
            self.input_points,
            self.vertices,
            self.faces,
            self.working_bytes,
            self.surface_bytes,
            self.work_units,
        )
    }
}

impl Default for DerivationCeilings {
    fn default() -> Self {
        let limits = TerrainLimits::default();
        Self {
            limits,
            input_points: limits.max_input_points(),
            vertices: limits.max_vertices(),
            faces: limits.max_faces(),
            working_bytes: limits.max_working_bytes(),
            surface_bytes: limits.max_surface_bytes(),
            work_units: limits.max_work_units(),
        }
    }
}
