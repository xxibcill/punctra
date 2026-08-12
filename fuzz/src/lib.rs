//! Bounded public-interface harness for persisted point-index bytes.

#![forbid(unsafe_code)]

use std::{fs, sync::OnceLock};

use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
use point_index::{PrepareLimits, prepare};
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
    let mode = selector & 7;
    let persisted = match mode {
        0 | 1 => payload.to_vec(),
        2 => mutated(&seeds().artifact, payload),
        3 => mutated(&seeds().work, payload),
        4 => checksum_valid_artifact_mutation(&seeds().artifact, payload),
        5 => checksum_valid_work_header_mutation(&seeds().work, payload),
        _ => checksum_valid_work_frame_mutation(&seeds().work, payload),
    };

    let directory = tempfile::tempdir().expect("create isolated fuzz directory");
    let target = directory.path().join("fixture.pidx");
    let persisted_path = if matches!(mode, 0 | 2 | 4) {
        target.clone()
    } else {
        directory.path().join("fixture.pidx.work")
    };
    fs::write(&persisted_path, persisted).expect("write bounded persisted fuzz input");

    let _ = prepare(fixture_source(), &target, fuzz_limits()).blocking_wait();
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
    const HEADER_BODY_BYTES: usize = 168;
    const HEADER_BYTES: usize = 200;

    let mut bytes = seed.to_vec();
    if bytes.len() < HEADER_BYTES {
        return bytes;
    }
    mutate_region(&mut bytes, mutations, 0, HEADER_BODY_BYTES);
    let checksum = blake3::hash(&bytes[..HEADER_BODY_BYTES]);
    bytes[HEADER_BODY_BYTES..HEADER_BYTES].copy_from_slice(checksum.as_bytes());
    bytes
}

fn checksum_valid_work_frame_mutation(seed: &[u8], mutations: &[u8]) -> Vec<u8> {
    const HEADER_BYTES: usize = 200;
    const FRAME_PREFIX_BYTES: usize = 40;

    let mut bytes = seed.to_vec();
    let mut frame_offset = HEADER_BYTES;
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

        Seeds { artifact, work }
    })
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

fn fuzz_limits() -> PrepareLimits {
    PrepareLimits::new(1, 24)
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
    }
}
