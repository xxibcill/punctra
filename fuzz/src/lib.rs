//! Bounded public-interface harness for persisted point-index bytes.

#![forbid(unsafe_code)]

use std::{fs, sync::OnceLock};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_index::{
    IndexRecipe, InspectionAttributeIds, PrepareLimits, prepare, prepare_with_recipe,
};
use point_source::Source;
use source_memory::MemorySource;

/// Maximum retained persisted input explored by one fuzz iteration.
pub const MAX_INPUT_BYTES: usize = 256 * 1_024;

const MAX_MUTATIONS: usize = 4_096;
const MAX_RETAINED_FILES: usize = 6;
const MAX_RETAINED_BYTES: u64 = 4 * MAX_INPUT_BYTES as u64;

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
}
