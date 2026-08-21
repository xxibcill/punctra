//! Bounded public-interface harnesses for persisted index and terrain bytes.

#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, LinearUnit, PositionTransform, SpatialAxes,
    SpatialReferenceProfile, SpatialReferenceProvenance, WorldBounds,
};
use point_index::{
    IndexRecipe, InspectionAttributeIds, PrepareLimits, prepare, prepare_with_recipe,
};
use point_source::Source;
use point_terrain::{SurfaceReadLimits, TerrainLimits, TerrainPrepareLimits, TerrainRecipe};
use point_workspace::{OpenLimits, PointRowLimits, Snapshot, WorkspaceSchema};
use source_memory::MemorySource;

/// Maximum retained persisted input explored by one fuzz iteration.
pub const MAX_INPUT_BYTES: usize = 256 * 1_024;

const MAX_MUTATIONS: usize = 4_096;
const MAX_RETAINED_FILES: usize = 6;
const MAX_RETAINED_BYTES: u64 = 4 * MAX_INPUT_BYTES as u64;
const TERRAIN_GROUND_CLASSIFICATION: u8 = 2;
const TERRAIN_FIXTURE_POINTS: u64 = 16;
const TERRAIN_POINT_BYTES: u64 = 33;
const TERRAIN_MAX_STREAM_BATCHES: usize = MAX_INPUT_BYTES / 12 + 1;
const TERRAIN_ARTIFACT_CHECKSUM_DOMAIN: &[u8] = b"punctra-terrain-disk-v1";
const TERRAIN_WORK_CHECKSUM_DOMAIN: &[u8] = b"punctra-terrain-work-v1";

struct Seeds {
    artifact: Vec<u8>,
    work: Vec<u8>,
    attributed_artifact: Vec<u8>,
    attributed_work: Vec<u8>,
}

/// Exercises complete-artifact and resumable-work decoding through `prepare`.
///
/// The input, mutation count, Source size, hierarchy size, working memory, and
/// retained filesystem state are all capped so malformed lengths cannot turn a
/// fuzz case into unbounded work.
pub fn exercise_persisted_bytes(input: &[u8]) {
    if input.len() > MAX_INPUT_BYTES {
        return;
    }
    let (selector, payload) = input
        .split_first()
        .map_or((0_u8, &[][..]), |(&selector, payload)| (selector, payload));
    let mode = selector % 14;
    let persisted = match mode {
        0 | 1 => payload.to_vec(),
        2 => mutated(&seeds().artifact, payload),
        3 => mutated(&seeds().work, payload),
        4 => checksum_valid_artifact_mutation(&seeds().artifact, payload),
        5 => checksum_valid_work_header_mutation(&seeds().work, payload),
        6 => checksum_valid_work_frame_mutation(&seeds().work, payload),
        7 => mutated(&seeds().attributed_artifact, payload),
        8 => mutated(&seeds().attributed_work, payload),
        9 => checksum_valid_artifact_mutation(&seeds().attributed_artifact, payload),
        10 => checksum_valid_work_header_mutation(&seeds().attributed_work, payload),
        11 => checksum_valid_work_frame_mutation(&seeds().attributed_work, payload),
        12 => seeds().attributed_artifact.clone(),
        _ => seeds().attributed_work.clone(),
    };

    let directory = tempfile::tempdir().expect("create isolated fuzz directory");
    let target = directory.path().join("fixture.pidx");
    let artifact_mode = matches!(mode, 0 | 2 | 4 | 7 | 9 | 12);
    let attributed_mode = mode >= 7;
    let persisted_path = if artifact_mode {
        target.clone()
    } else {
        directory.path().join("fixture.pidx.work")
    };
    fs::write(&persisted_path, persisted).expect("write bounded persisted fuzz input");

    if attributed_mode {
        let _ = prepare_with_recipe(
            attributed_fixture_source(),
            &target,
            inspection_recipe(),
            fuzz_limits(33),
        )
        .blocking_wait();
    } else {
        let _ = prepare(fixture_source(), &target, fuzz_limits(24)).blocking_wait();
    }
    assert_retained_state_is_bounded(directory.path());
}

fn mutated(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    mutate_region(&mut bytes, mutations, 0, seed.len());
    bytes
}

fn checksum_valid_artifact_mutation(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    const CHECKSUM_BYTES: usize = 32;

    let mut bytes = seed.to_vec();
    let Some(checksum_offset) = bytes.len().checked_sub(CHECKSUM_BYTES) else {
        return bytes;
    };
    mutate_region(&mut bytes, mutations, 0, checksum_offset);
    let checksum = blake3::hash(&bytes[..checksum_offset]);
    bytes[checksum_offset..].copy_from_slice(checksum.as_bytes());
    bytes
}

fn checksum_valid_work_header_mutation(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    let Some((header_body_bytes, header_bytes)) = work_header_layout(&bytes) else {
        return bytes;
    };
    if bytes.len() < header_bytes {
        return bytes;
    }
    mutate_region(&mut bytes, mutations, 0, header_body_bytes);
    let checksum = blake3::hash(&bytes[..header_body_bytes]);
    bytes[header_body_bytes..header_bytes].copy_from_slice(checksum.as_bytes());
    bytes
}

fn checksum_valid_work_frame_mutation(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    const FRAME_PREFIX_BYTES: usize = 40;

    let mut bytes = seed.to_vec();
    let Some((_, header_bytes)) = work_header_layout(&bytes) else {
        return bytes;
    };
    let mut frame_offset = header_bytes;
    let mut mutation_index = 0;
    while frame_offset
        .checked_add(FRAME_PREFIX_BYTES)
        .is_some_and(|prefix_end| prefix_end <= bytes.len())
    {
        if bytes.get(frame_offset..frame_offset + 4) != Some(b"BLK1") {
            break;
        }
        let payload_length = u32::from_le_bytes(
            bytes[frame_offset + 4..frame_offset + 8]
                .try_into()
                .expect("bounded frame prefix has a length field"),
        ) as usize;
        let payload_start = frame_offset + FRAME_PREFIX_BYTES;
        let Some(payload_end) = payload_start.checked_add(payload_length) else {
            break;
        };
        if payload_end > bytes.len() {
            break;
        }
        let frame_mutations = mutations
            .chunks_exact(3)
            .skip(mutation_index)
            .take(MAX_MUTATIONS.saturating_sub(mutation_index));
        for mutation in frame_mutations {
            let payload_offset =
                usize::from(u16::from_le_bytes([mutation[0], mutation[1]])) % payload_length.max(1);
            if payload_length != 0 {
                bytes[payload_start + payload_offset] ^= mutation[2];
            }
            mutation_index += 1;
        }
        let checksum = blake3::hash(&bytes[payload_start..payload_end]);
        bytes[frame_offset + 8..frame_offset + FRAME_PREFIX_BYTES]
            .copy_from_slice(checksum.as_bytes());
        frame_offset = payload_end;
        if mutation_index >= MAX_MUTATIONS {
            break;
        }
    }
    bytes
}

fn work_header_layout(bytes: &[u8]) -> Option<(usize, usize)> {
    let disk = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    match disk {
        1 => Some((168, 200)),
        2 => Some((200, 232)),
        _ => None,
    }
}

fn mutate_region(bytes: &mut [u8], mutations: &[u8], start: usize, end: usize) {
    let Some(region_len) = end.checked_sub(start).filter(|length| *length != 0) else {
        return;
    };
    for mutation in mutations.chunks_exact(3).take(MAX_MUTATIONS) {
        let offset =
            start + usize::from(u16::from_le_bytes([mutation[0], mutation[1]])) % region_len;
        bytes[offset] ^= mutation[2];
    }
}

fn seeds() -> &'static Seeds {
    static SEEDS: OnceLock<Seeds> = OnceLock::new();
    SEEDS.get_or_init(|| {
        let directory = tempfile::tempdir().expect("create seed directory");

        let artifact_path = directory.path().join("artifact.pidx");
        prepare(fixture_source(), &artifact_path, PrepareLimits::default())
            .blocking_wait()
            .expect("build valid artifact seed");
        let artifact = fs::read(&artifact_path).expect("read valid artifact seed");

        let work_target = directory.path().join("work.pidx");
        let result = prepare(
            fixture_source(),
            &work_target,
            PrepareLimits::default().with_max_artifact_bytes(400),
        )
        .blocking_wait();
        assert!(result.is_err(), "seed build must stop before finalization");
        let work = fs::read(directory.path().join("work.pidx.work"))
            .expect("read valid resumable-work seed");

        let attributed_artifact_path = directory.path().join("attributed-artifact.pidx");
        prepare_with_recipe(
            attributed_fixture_source(),
            &attributed_artifact_path,
            inspection_recipe(),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("build valid attributed artifact seed");
        let attributed_artifact =
            fs::read(&attributed_artifact_path).expect("read valid attributed artifact seed");

        let attributed_work_target = directory.path().join("attributed-work.pidx");
        let result = prepare_with_recipe(
            attributed_fixture_source(),
            &attributed_work_target,
            inspection_recipe(),
            PrepareLimits::default().with_max_artifact_bytes(400),
        )
        .blocking_wait();
        assert!(
            result.is_err(),
            "attributed seed build must stop before finalization"
        );
        let attributed_work = fs::read(directory.path().join("attributed-work.pidx.work"))
            .expect("read valid attributed resumable-work seed");

        Seeds {
            artifact,
            work,
            attributed_artifact,
            attributed_work,
        }
    })
}

fn attributed_fixture_source() -> Source {
    let definitions = [
        (
            1,
            "intensity",
            AttributeDataType::U16,
            AttributeValues::u16(vec![321]),
        ),
        (
            6,
            "classification",
            AttributeDataType::U8,
            AttributeValues::u8(vec![7]),
        ),
        (
            16,
            "red",
            AttributeDataType::U16,
            AttributeValues::u16(vec![1_000]),
        ),
        (
            17,
            "green",
            AttributeDataType::U16,
            AttributeValues::u16(vec![2_000]),
        ),
        (
            18,
            "blue",
            AttributeDataType::U16,
            AttributeValues::u16(vec![3_000]),
        ),
    ];
    let columns = definitions
        .into_iter()
        .map(|(id, name, data_type, values)| {
            AttributeColumn::new(
                AttributeDefinition::new(AttributeId::new(id).unwrap(), name, data_type).unwrap(),
                values,
            )
            .unwrap()
        })
        .collect();
    let input = MemorySource::from_columns(
        PositionTransform::new([1_000_000.0, -2_000_000.0, 50.0], [0.01; 3]).unwrap(),
        CoordinateReference::Unknown,
        vec![[1, 2, 3]],
        AttributeColumns::new(columns, 1).unwrap(),
    )
    .unwrap();
    source_memory::open(input).blocking_wait().unwrap()
}

fn inspection_recipe() -> IndexRecipe {
    IndexRecipe::InspectionV1(
        InspectionAttributeIds::new(
            AttributeId::new(1).unwrap(),
            AttributeId::new(6).unwrap(),
            [
                AttributeId::new(16).unwrap(),
                AttributeId::new(17).unwrap(),
                AttributeId::new(18).unwrap(),
            ],
        )
        .unwrap(),
    )
}

fn fixture_source() -> Source {
    let input = MemorySource::from_columns(
        PositionTransform::new([1_000_000.0, -2_000_000.0, 50.0], [0.01; 3])
            .expect("fixture transform is valid"),
        CoordinateReference::Unknown,
        vec![[1, 2, 3]],
        AttributeColumns::empty(1),
    )
    .expect("fixture memory Source is valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("fixture memory Source opens")
}

fn fuzz_limits(source_point_bytes: u64) -> PrepareLimits {
    PrepareLimits::new(1, source_point_bytes)
        .expect("nonzero fuzz Source batch limits")
        .with_max_adapter_working_bytes(64 * 1_024)
        .with_max_build_working_bytes(512 * 1_024)
        .with_max_incomplete_bytes(MAX_INPUT_BYTES as u64)
        .with_max_artifact_bytes(MAX_INPUT_BYTES as u64)
        .with_max_hierarchy_nodes(1_024)
        .with_max_resident_metadata_bytes(512 * 1_024)
}

fn assert_retained_state_is_bounded(directory: &std::path::Path) {
    let mut files = 0;
    let mut bytes = 0_u64;
    for entry in fs::read_dir(directory).expect("inspect fuzz directory") {
        let metadata = entry
            .expect("read fuzz directory entry")
            .metadata()
            .expect("inspect fuzz file");
        if metadata.is_file() {
            files += 1;
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    assert!(files <= MAX_RETAINED_FILES);
    assert!(bytes <= MAX_RETAINED_BYTES);
}

struct TerrainSeeds {
    snapshot: Snapshot,
    artifact: Vec<u8>,
    work: Vec<u8>,
    _directory: tempfile::TempDir,
}

#[derive(Clone, Copy)]
enum TerrainPersistedKind {
    Artifact,
    Work,
}

/// Exercises complete-artifact and resumable-work terrain decoding through
/// public preparation and bounded stream interfaces.
///
/// The fuzz payload, mutation count, fixed Source, derivation limits, file
/// sizes, stream batch sizes, stream iterations, and retained filesystem state
/// are all capped. Valid seed files are built through the public API so this
/// harness does not construct private decoder state.
pub fn exercise_terrain_persisted_bytes(input: &[u8]) {
    if input.len() > MAX_INPUT_BYTES {
        return;
    }
    let (selector, payload) = input
        .split_first()
        .map_or((0_u8, &[][..]), |(&selector, payload)| (selector, payload));
    let mode = selector % 16;
    let (kind, persisted) = terrain_persisted_case(mode, payload);

    let directory = tempfile::tempdir().expect("create isolated terrain fuzz directory");
    let target = directory.path().join("fixture.pterr");
    let persisted_path = match kind {
        TerrainPersistedKind::Artifact => target.clone(),
        TerrainPersistedKind::Work => terrain_sibling_path(&target, ".surface-work-v1"),
    };
    fs::write(&persisted_path, persisted).expect("write bounded terrain fuzz input");

    if let Ok(surface) = point_terrain::prepare(
        terrain_seeds().snapshot.clone(),
        &target,
        terrain_recipe(),
        terrain_fuzz_limits(MAX_INPUT_BYTES as u64),
    )
    .blocking_wait()
    {
        exercise_terrain_streams(&surface);
    }
    assert_retained_state_is_bounded(directory.path());
}

fn terrain_persisted_case(mode: u8, payload: &[u8]) -> (TerrainPersistedKind, Vec<u8>) {
    match mode {
        0 => (TerrainPersistedKind::Artifact, payload.to_vec()),
        1 => (TerrainPersistedKind::Work, payload.to_vec()),
        2 => (
            TerrainPersistedKind::Artifact,
            mutated(&terrain_seeds().artifact, payload),
        ),
        3 => (
            TerrainPersistedKind::Work,
            mutated(&terrain_seeds().work, payload),
        ),
        4 => (
            TerrainPersistedKind::Artifact,
            terrain_tail_mutation(&terrain_seeds().artifact, payload),
        ),
        5 => (
            TerrainPersistedKind::Work,
            terrain_tail_mutation(&terrain_seeds().work, payload),
        ),
        6 => (
            TerrainPersistedKind::Artifact,
            terrain_checksum_valid_tail_mutation(
                &terrain_seeds().artifact,
                payload,
                TERRAIN_ARTIFACT_CHECKSUM_DOMAIN,
            ),
        ),
        7 => (
            TerrainPersistedKind::Work,
            terrain_checksum_valid_tail_mutation(
                &terrain_seeds().work,
                payload,
                TERRAIN_WORK_CHECKSUM_DOMAIN,
            ),
        ),
        8 => (
            TerrainPersistedKind::Artifact,
            terrain_truncated(&terrain_seeds().artifact, payload),
        ),
        9 => (
            TerrainPersistedKind::Work,
            terrain_truncated(&terrain_seeds().work, payload),
        ),
        10 => (
            TerrainPersistedKind::Artifact,
            terrain_extended(&terrain_seeds().artifact, payload),
        ),
        11 => (
            TerrainPersistedKind::Work,
            terrain_extended(&terrain_seeds().work, payload),
        ),
        12 => (
            TerrainPersistedKind::Artifact,
            terrain_seeds().artifact.clone(),
        ),
        13 => (TerrainPersistedKind::Work, terrain_seeds().work.clone()),
        14 => (
            TerrainPersistedKind::Artifact,
            terrain_checksum_mutation(&terrain_seeds().artifact, payload),
        ),
        _ => (
            TerrainPersistedKind::Work,
            terrain_checksum_mutation(&terrain_seeds().work, payload),
        ),
    }
}

fn terrain_tail_mutation(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    const TAIL_BYTES: usize = 96;

    let mut bytes = seed.to_vec();
    let start = bytes.len().saturating_sub(TAIL_BYTES);
    let end = bytes.len();
    mutate_region(&mut bytes, mutations, start, end);
    bytes
}

fn terrain_checksum_valid_tail_mutation(seed: &[u8], mutations: &[u8], domain: &[u8]) -> Vec<u8> {
    const CHECKSUM_BYTES: usize = 32;
    const DIRECTORY_ADJACENT_BYTES: usize = 64;

    let mut bytes = seed.to_vec();
    let Some(payload_end) = bytes.len().checked_sub(CHECKSUM_BYTES) else {
        return bytes;
    };
    let start = payload_end.saturating_sub(DIRECTORY_ADJACENT_BYTES);
    mutate_region(&mut bytes, mutations, start, payload_end);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    hasher.update(&bytes[..payload_end]);
    bytes[payload_end..].copy_from_slice(hasher.finalize().as_bytes());
    bytes
}

fn terrain_checksum_mutation(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    const CHECKSUM_BYTES: usize = 32;

    let mut bytes = seed.to_vec();
    let start = bytes.len().saturating_sub(CHECKSUM_BYTES);
    let end = bytes.len();
    mutate_region(&mut bytes, mutations, start, end);
    bytes
}

fn terrain_truncated(seed: &[u8], selector: &[u8]) -> Vec<u8> {
    let ordinal = selector.iter().take(8).fold(0_usize, |value, byte| {
        value.rotate_left(5) ^ usize::from(*byte)
    });
    let retained = ordinal % seed.len().saturating_add(1);
    seed[..retained].to_vec()
}

fn terrain_extended(seed: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    let retained = MAX_INPUT_BYTES
        .saturating_sub(bytes.len())
        .min(suffix.len());
    bytes.extend_from_slice(&suffix[..retained]);
    bytes
}

fn terrain_seeds() -> &'static TerrainSeeds {
    static SEEDS: OnceLock<TerrainSeeds> = OnceLock::new();
    SEEDS.get_or_init(|| {
        let directory = tempfile::tempdir().expect("create terrain seed directory");
        let source = terrain_fixture_source();
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            terrain_index_limits(),
        )
        .blocking_wait()
        .expect("build terrain fixture index");
        let workspace = point_workspace::create(
            directory.path().join("fixture.pcw"),
            index,
            WorkspaceSchema::new(terrain_classification_attribute()),
            terrain_workspace_limits(),
        )
        .blocking_wait()
        .expect("build terrain fixture Workspace");
        let snapshot = workspace.head();

        let artifact_path = directory.path().join("artifact.pterr");
        point_terrain::prepare(
            snapshot.clone(),
            &artifact_path,
            terrain_recipe(),
            terrain_fuzz_limits(MAX_INPUT_BYTES as u64),
        )
        .blocking_wait()
        .expect("build valid terrain artifact seed");
        let artifact = fs::read(&artifact_path).expect("read valid terrain artifact seed");

        let work_target = directory.path().join("work.pterr");
        let result = point_terrain::prepare(
            snapshot.clone(),
            &work_target,
            terrain_recipe(),
            terrain_fuzz_limits(64),
        )
        .blocking_wait();
        assert!(result.is_err(), "seed build must stop before publication");
        let work = fs::read(terrain_sibling_path(&work_target, ".surface-work-v1"))
            .expect("read valid terrain work seed");

        let validation_target = directory.path().join("validated-work.pterr");
        fs::write(
            terrain_sibling_path(&validation_target, ".surface-work-v1"),
            &work,
        )
        .expect("copy terrain work seed for public validation");
        point_terrain::prepare(
            snapshot.clone(),
            validation_target,
            terrain_recipe(),
            terrain_fuzz_limits(MAX_INPUT_BYTES as u64),
        )
        .blocking_wait()
        .expect("resume valid terrain work seed");

        assert!(artifact.len() <= MAX_INPUT_BYTES);
        assert!(work.len() <= MAX_INPUT_BYTES);
        TerrainSeeds {
            snapshot,
            artifact,
            work,
            _directory: directory,
        }
    })
}

fn terrain_fixture_source() -> Source {
    let mut ticks = Vec::with_capacity(TERRAIN_FIXTURE_POINTS as usize);
    for ordinal in 0..TERRAIN_FIXTURE_POINTS {
        let x = i64::try_from(ordinal % 4).expect("small fixture x tick");
        let y = i64::try_from(ordinal / 4).expect("small fixture y tick");
        ticks.push([x, y, x * x + 3 * y * y + x * y]);
    }
    let classification = AttributeDefinition::new(
        terrain_classification_attribute(),
        "classification",
        AttributeDataType::U8,
    )
    .expect("terrain classification definition is valid");
    let classification = AttributeColumn::new(
        classification,
        AttributeValues::u8(vec![
            TERRAIN_GROUND_CLASSIFICATION;
            TERRAIN_FIXTURE_POINTS as usize
        ]),
    )
    .expect("terrain classification column is valid");
    let attributes = AttributeColumns::new(vec![classification], TERRAIN_FIXTURE_POINTS as usize)
        .expect("terrain fixture attributes are row-aligned");
    let reference = SpatialReferenceProfile::new(
        32_647,
        5_703,
        SpatialAxes::EastingNorthingElevation,
        LinearUnit::Metre,
        LinearUnit::Metre,
        SpatialReferenceProvenance::CallerDeclaration,
    )
    .expect("terrain fixture reference is valid");
    let input = MemorySource::from_columns(
        PositionTransform::new([0.0; 3], [1.0, 1.0, 0.1])
            .expect("terrain fixture transform is valid"),
        CoordinateReference::profile(reference),
        ticks,
        attributes,
    )
    .expect("terrain fixture Source is valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("terrain fixture Source opens")
}

fn terrain_classification_attribute() -> AttributeId {
    AttributeId::new(301).expect("terrain classification Attribute identity is nonzero")
}

fn terrain_recipe() -> TerrainRecipe {
    TerrainRecipe::new(TERRAIN_GROUND_CLASSIFICATION).within(
        WorldBounds::new([-1.0, -1.0, -1.0], [4.0, 4.0, 10.0])
            .expect("terrain fixture bounds are valid"),
    )
}

fn terrain_fuzz_limits(max_artifact_bytes: u64) -> TerrainPrepareLimits {
    const MAX_POINTS: u64 = 32;
    const MAX_FACES: u64 = 64;
    const KIB: u64 = 1_024;

    let source_budget = point_source::ReadBudget::new(8, 8 * TERRAIN_POINT_BYTES)
        .expect("nonzero terrain Source batch limits")
        .with_max_spans(64)
        .with_max_points(MAX_POINTS)
        .with_max_adapter_working_bytes(64 * KIB);
    let rows = PointRowLimits::new(
        point_index::CandidateLimits::new(1_024, 64, MAX_POINTS, 64 * KIB),
        source_budget,
        64,
        64 * KIB,
        MAX_POINTS,
        8,
        8 * TERRAIN_POINT_BYTES,
        512 * KIB,
    );
    let derivation = TerrainLimits::new(
        rows,
        MAX_POINTS,
        MAX_POINTS,
        MAX_FACES,
        1_024 * KIB,
        1_024 * KIB,
        100_000,
    );
    TerrainPrepareLimits::new(
        derivation,
        MAX_INPUT_BYTES as u64,
        max_artifact_bytes,
        2 * MAX_INPUT_BYTES as u64,
        16 * KIB,
        64 * KIB,
        4 * KIB,
    )
}

fn terrain_index_limits() -> PrepareLimits {
    const KIB: u64 = 1_024;

    PrepareLimits::new(8, 8 * TERRAIN_POINT_BYTES)
        .expect("nonzero terrain index Source limits")
        .with_max_adapter_working_bytes(64 * KIB)
        .with_max_build_working_bytes(1_024 * KIB)
        .with_max_incomplete_bytes(MAX_INPUT_BYTES as u64)
        .with_max_artifact_bytes(MAX_INPUT_BYTES as u64)
        .with_max_hierarchy_nodes(1_024)
        .with_max_resident_metadata_bytes(512 * KIB)
}

fn terrain_workspace_limits() -> OpenLimits {
    const KIB: u64 = 1_024;

    OpenLimits::new()
        .with_max_manifest_bytes(64 * KIB)
        .with_max_operation_records(1)
        .with_max_revision_files(1)
        .with_max_revision_blocks(1)
        .with_max_revision_rows(0)
        .with_max_revision_block_bytes(64 * KIB)
        .with_max_single_file_bytes(MAX_INPUT_BYTES as u64)
        .with_max_total_persisted_bytes(2 * MAX_INPUT_BYTES as u64)
        .with_max_working_bytes(1_024 * KIB)
        .with_max_resident_metadata_bytes(128 * KIB)
}

fn exercise_terrain_streams(surface: &point_terrain::PreparedTerrainSurface) {
    let _ = surface.descriptor();
    let limits = SurfaceReadLimits::new(8, 8 * 32, 4 * 1_024, 8 * 1_024, 64 * 1_024);
    if let Ok(batches) = surface.vertex_batches(limits) {
        for batch in batches.take(TERRAIN_MAX_STREAM_BATCHES) {
            if batch.is_err() {
                break;
            }
        }
    }
    if let Ok(batches) = surface.face_batches(limits) {
        for batch in batches.take(TERRAIN_MAX_STREAM_BATCHES) {
            if batch.is_err() {
                break;
            }
        }
    }
}

fn terrain_sibling_path(target: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(target.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT_CORPUS: &[&[u8]] = &[
        include_bytes!("../corpus/index_persistence/raw_artifact"),
        include_bytes!("../corpus/index_persistence/raw_work"),
        include_bytes!("../corpus/index_persistence/valid_artifact"),
        include_bytes!("../corpus/index_persistence/valid_work"),
        include_bytes!("../corpus/index_persistence/mutated_artifact"),
        include_bytes!("../corpus/index_persistence/mutated_work"),
        include_bytes!("../corpus/index_persistence/checksummed_artifact"),
        include_bytes!("../corpus/index_persistence/checksummed_work_header"),
        include_bytes!("../corpus/index_persistence/checksummed_work_frame"),
    ];

    #[test]
    fn checked_in_short_corpus_stays_bounded_and_panic_free() {
        for input in SHORT_CORPUS {
            exercise_persisted_bytes(input);
        }
    }

    #[test]
    fn structured_mutations_refresh_their_enclosing_checksums() {
        let artifact = checksum_valid_artifact_mutation(&seeds().artifact, &[0, 0, 1]);
        let artifact_checksum = artifact.len() - 32;
        assert_eq!(
            blake3::hash(&artifact[..artifact_checksum]).as_bytes(),
            &artifact[artifact_checksum..]
        );

        let work_header = checksum_valid_work_header_mutation(&seeds().work, &[0, 0, 1]);
        assert_eq!(
            blake3::hash(&work_header[..168]).as_bytes(),
            &work_header[168..200]
        );

        let work_frame = checksum_valid_work_frame_mutation(&seeds().work, &[0, 0, 1]);
        let payload_length = u32::from_le_bytes(work_frame[204..208].try_into().unwrap()) as usize;
        let payload_end = 240 + payload_length;
        assert_eq!(
            blake3::hash(&work_frame[240..payload_end]).as_bytes(),
            &work_frame[208..240]
        );

        let attributed_header =
            checksum_valid_work_header_mutation(&seeds().attributed_work, &[0, 0, 1]);
        assert_eq!(
            blake3::hash(&attributed_header[..200]).as_bytes(),
            &attributed_header[200..232]
        );

        let attributed_frame =
            checksum_valid_work_frame_mutation(&seeds().attributed_work, &[0, 0, 1]);
        let payload_length =
            u32::from_le_bytes(attributed_frame[236..240].try_into().unwrap()) as usize;
        let payload_end = 272 + payload_length;
        assert_eq!(
            blake3::hash(&attributed_frame[272..payload_end]).as_bytes(),
            &attributed_frame[240..272]
        );
    }

    #[test]
    fn terrain_seed_and_malformed_modes_stay_bounded_and_panic_free() {
        for mode in 0_u8..16 {
            exercise_terrain_persisted_bytes(&[mode, 0, 0, 1, 31, 0, 0x80]);
        }
    }
}
