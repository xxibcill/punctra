use std::{
    collections::BinaryHeap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::{PositionTransform, SourceId, WorldBounds};
use point_source::{Source, SourceSpan};

use crate::{
    DisplayCoverage, IndexDescriptor, IndexError, IndexHierarchy, IndexNode, IndexNodeId,
    PrepareLimits,
    model::{DISK_VERSION, RECIPE_VERSION},
    read::IndexSample,
    tree::{BLOCK_POINTS, LeafRecord, MAX_NODE_SAMPLES, SAMPLE_BYTES, TreePlan},
};

const WORK_MAGIC: &[u8; 8] = b"PNWRK004";
const ARTIFACT_MAGIC: &[u8; 8] = b"PNIDX004";
const FRAME_MAGIC: &[u8; 4] = b"BLK1";
const WORK_HEADER_BODY_BYTES: usize = 168;
const WORK_HEADER_BYTES: u64 = 200;
const FRAME_PREFIX_BYTES: u64 = 40;
const FRAME_FIXED_PAYLOAD_BYTES: u64 = 72;
const ARTIFACT_HEADER_BYTES: u64 = 208;
const NODE_RECORD_BYTES: u64 = 168;
const ARTIFACT_CHECKSUM_BYTES: u64 = 32;
const HASH_BUFFER_BYTES: u64 = 64 * 1024;
const SAMPLE_HASH_DOMAIN: &[u8] = b"punctra-index-samples-v1";
const ORDINAL_HASH_DOMAIN: u64 = 0x706e_6374_7261_0401;

#[derive(Clone)]
pub(crate) struct ArtifactReader {
    file: Arc<Mutex<File>>,
    path: Arc<PathBuf>,
}

impl ArtifactReader {
    pub(crate) fn read_sample_block(
        &self,
        offset: u64,
        count: u64,
        expected_checksum: [u8; 32],
        max_buffer_bytes: u64,
    ) -> Result<Vec<IndexSample>, IndexError> {
        let byte_count = count
            .checked_mul(SAMPLE_BYTES)
            .ok_or(IndexError::CorruptArtifact {
                reason: "sample block length overflowed",
            })?;
        let capacity = usize::try_from(count).map_err(|_| IndexError::ResourceLimit {
            limit: "addressable sample Points",
            required: count,
            allowed: usize::MAX as u64,
        })?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(capacity)
            .map_err(|_| IndexError::ResourceLimit {
                limit: "sample buffer bytes",
                required: byte_count,
                allowed: byte_count,
            })?;
        let actual_bytes = u64::try_from(samples.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<IndexSample>()).unwrap_or(u64::MAX));
        require(actual_bytes, max_buffer_bytes, "index sample buffer bytes")?;
        let mut hasher = Hasher::new();
        hasher.update(SAMPLE_HASH_DOMAIN);
        let mut file = lock_recovering(&self.file);
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| IndexError::io("seek in", self.path.as_ref().clone(), error))?;
        for _ in 0..count {
            let mut encoded = [0_u8; 32];
            file.read_exact(&mut encoded).map_err(|error| {
                if error.kind() == std::io::ErrorKind::UnexpectedEof {
                    IndexError::CorruptArtifact {
                        reason: "node sample block was truncated after open",
                    }
                } else {
                    IndexError::io("read", self.path.as_ref().clone(), error)
                }
            })?;
            hasher.update(&encoded);
            let mut decoder = Decoder::artifact(&encoded);
            let ordinal = decoder.u64("sample ordinal")?;
            let ticks = [
                decoder.i64("sample x ticks")?,
                decoder.i64("sample y ticks")?,
                decoder.i64("sample z ticks")?,
            ];
            samples.push(IndexSample::new(ordinal, ticks));
        }
        if *hasher.finalize().as_bytes() != expected_checksum {
            return Err(IndexError::CorruptArtifact {
                reason: "node sample checksum differs after open",
            });
        }
        if samples
            .windows(2)
            .any(|pair| pair[0].ordinal() >= pair[1].ordinal())
        {
            return Err(IndexError::CorruptArtifact {
                reason: "samples are not sorted and unique",
            });
        }
        Ok(samples)
    }
}

pub(crate) struct OpenArtifact {
    pub(crate) descriptor: IndexDescriptor,
    pub(crate) hierarchy: IndexHierarchy,
    pub(crate) reader: ArtifactReader,
    pub(crate) artifact_bytes: u64,
}

pub(crate) struct WorkFile {
    file: File,
    path: PathBuf,
    leaves: Vec<LeafRecord>,
    durable_points: u64,
}

impl WorkFile {
    pub(crate) fn durable_points(&self) -> u64 {
        self.durable_points
    }

    pub(crate) fn leaves(&self) -> &[LeafRecord] {
        &self.leaves
    }

    pub(crate) fn retained_metadata_bytes(&self) -> u64 {
        u64::try_from(self.leaves.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX))
    }

    pub(crate) fn append_block(
        &mut self,
        span: SourceSpan,
        bounds: WorldBounds,
        samples: &[IndexSample],
        limits: PrepareLimits,
    ) -> Result<(), IndexError> {
        let retained = span.point_count().min(MAX_NODE_SAMPLES);
        let validation_bytes = retained
            .saturating_mul(u64::try_from(mem::size_of::<(u64, u64)>()).unwrap_or(u64::MAX))
            .saturating_add(
                retained.saturating_mul(u64::try_from(mem::size_of::<u64>()).unwrap_or(8)),
            );
        let payload_bytes = FRAME_FIXED_PAYLOAD_BYTES.saturating_add(
            u64::try_from(samples.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(SAMPLE_BYTES),
        );
        require(
            self.retained_metadata_bytes()
                .saturating_add(validation_bytes)
                .saturating_add(payload_bytes)
                .saturating_add(FRAME_PREFIX_BYTES),
            limits.max_build_working_bytes(),
            "build working bytes",
        )?;
        validate_frame_values(span, bounds, samples)?;
        if span.first_ordinal() != self.durable_points {
            return Err(IndexError::CorruptWork {
                reason: "new work frame is not ordinal-contiguous",
            });
        }
        let leaf_bytes = u64::try_from(self.leaves.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
        let sample_bytes = u64::try_from(samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(SAMPLE_BYTES);
        require(
            leaf_bytes
                .saturating_add(sample_bytes)
                .saturating_add(payload_bytes)
                .saturating_add(FRAME_PREFIX_BYTES),
            limits.max_build_working_bytes(),
            "build working bytes",
        )?;
        if self.leaves.len() == self.leaves.capacity() {
            return Err(IndexError::CorruptWork {
                reason: "work contains more frames than the canonical Source block count",
            });
        }
        let payload = encode_frame_payload(span, bounds, samples);
        let payload_length =
            u32::try_from(payload.len()).map_err(|_| IndexError::ResourceLimit {
                limit: "work frame payload bytes",
                required: u64::try_from(payload.len()).unwrap_or(u64::MAX),
                allowed: u64::from(u32::MAX),
            })?;
        let frame_bytes = FRAME_PREFIX_BYTES
            .checked_add(u64::from(payload_length))
            .ok_or(IndexError::ResourceLimit {
                limit: "incomplete index bytes",
                required: u64::MAX,
                allowed: limits.max_incomplete_bytes(),
            })?;
        let frame_offset = self
            .file
            .seek(SeekFrom::End(0))
            .map_err(|error| IndexError::io("seek to end of", &self.path, error))?;
        require(
            frame_offset.saturating_add(frame_bytes),
            limits.max_incomplete_bytes(),
            "incomplete index bytes",
        )?;

        let mut prefix = Vec::with_capacity(usize::try_from(FRAME_PREFIX_BYTES).unwrap_or(40));
        prefix.extend_from_slice(FRAME_MAGIC);
        push_u32(&mut prefix, payload_length);
        prefix.extend_from_slice(blake3::hash(&payload).as_bytes());
        self.file
            .write_all(&prefix)
            .and_then(|()| self.file.write_all(&payload))
            .and_then(|()| self.file.sync_data())
            .map_err(|error| IndexError::io("append and flush", &self.path, error))?;

        let sample_offset = frame_offset + FRAME_PREFIX_BYTES + FRAME_FIXED_PAYLOAD_BYTES;
        let sample_bytes = &payload[usize::try_from(FRAME_FIXED_PAYLOAD_BYTES).unwrap_or(72)..];
        self.leaves.push(LeafRecord {
            span,
            bounds,
            sample_offset,
            sample_count: u64::try_from(samples.len()).unwrap_or(u64::MAX),
            sample_checksum: sample_checksum(sample_bytes),
        });
        self.durable_points = span.end_ordinal();
        Ok(())
    }
}

pub(crate) fn target_exists(target: &Path) -> Result<bool, IndexError> {
    target
        .try_exists()
        .map_err(|error| IndexError::io("inspect", target, error))
}

pub(crate) fn open_or_create_work(
    source: &Source,
    target: &Path,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    preflight_work_initialization(source, limits, control)?;
    let path = sibling_path(target, ".work")?;
    reject_symlink(&path, "work path is a symbolic link")?;
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&path)
                        .map_err(|error| IndexError::io("open raced", &path, error))?
                }
                Err(error) => return Err(IndexError::io("create", path, error)),
            }
        }
        Err(error) => return Err(IndexError::io("open", path, error)),
    };
    acquire_work_ownership(&file, target)?;
    let file_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &path, error))?
        .len();
    if file_bytes == 0 {
        initialize_work(source, path, file, limits)
    } else {
        scan_work(source, path, file, limits, control)
    }
}

fn acquire_work_ownership(file: &File, target: &Path) -> Result<(), IndexError> {
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(IndexError::PreparationInProgress {
                path: target.to_path_buf(),
            });
        }
        Err(std::fs::TryLockError::Error(error)) => {
            return Err(IndexError::io("lock work for", target, error));
        }
    }
    if target_exists(target)? {
        return Err(IndexError::IncompatibleArtifact {
            reason: "target appeared while its index was being built",
        });
    }
    Ok(())
}

fn initialize_work(
    source: &Source,
    path: PathBuf,
    mut file: File,
    limits: PrepareLimits,
) -> Result<WorkFile, IndexError> {
    let header = encode_work_header(source);
    file.write_all(&header)
        .and_then(|()| file.sync_data())
        .map_err(|error| IndexError::io("write and flush", &path, error))?;
    sync_parent(&path)?;
    let leaves = reserve_leaf_metadata(source.metadata().point_count(), limits)?;
    Ok(WorkFile {
        file,
        path,
        leaves,
        durable_points: 0,
    })
}

fn preflight_work_initialization(
    source: &Source,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<(), IndexError> {
    control.check_cancelled()?;
    require(
        WORK_HEADER_BYTES,
        limits.max_incomplete_bytes(),
        "incomplete index bytes",
    )?;
    let leaf_count = canonical_leaf_count(source.metadata().point_count());
    let leaf_bytes =
        leaf_count.saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes.saturating_add(WORK_HEADER_BYTES),
        limits.max_build_working_bytes(),
        "build working bytes",
    )
}

fn scan_work(
    source: &Source,
    path: PathBuf,
    mut file: File,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    control.check_cancelled()?;
    let file_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &path, error))?
        .len();
    require(
        file_bytes,
        limits.max_incomplete_bytes(),
        "incomplete index bytes",
    )?;
    if file_bytes < WORK_HEADER_BYTES {
        return Err(IndexError::CorruptWork {
            reason: "work header is truncated",
        });
    }
    let expected_leaf_count = canonical_leaf_count(source.metadata().point_count());
    let leaf_bytes = expected_leaf_count
        .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    let maximum_payload =
        FRAME_FIXED_PAYLOAD_BYTES.saturating_add(MAX_NODE_SAMPLES.saturating_mul(SAMPLE_BYTES));
    let scan_buffers = maximum_payload
        .saturating_add(MAX_NODE_SAMPLES.saturating_mul(SAMPLE_BYTES))
        .saturating_add(
            MAX_NODE_SAMPLES
                .saturating_mul(u64::try_from(mem::size_of::<(u64, u64)>()).unwrap_or(u64::MAX)),
        )
        .saturating_add(
            MAX_NODE_SAMPLES.saturating_mul(u64::try_from(mem::size_of::<u64>()).unwrap_or(8)),
        )
        .saturating_add(WORK_HEADER_BYTES)
        .saturating_add(FRAME_PREFIX_BYTES);
    require(
        leaf_bytes.saturating_add(scan_buffers),
        limits.max_build_working_bytes(),
        "build working bytes",
    )?;
    let mut header = vec![0; usize::try_from(WORK_HEADER_BYTES).unwrap_or(200)];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| IndexError::io("read", &path, error))?;
    validate_work_header(source, &header)?;

    let mut leaves = reserve_leaf_metadata(source.metadata().point_count(), limits)?;
    let leaf_bytes = u64::try_from(leaves.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes.saturating_add(scan_buffers),
        limits.max_build_working_bytes(),
        "build working bytes",
    )?;

    let mut next_frame = WORK_HEADER_BYTES;
    let mut durable_points = 0_u64;
    while next_frame < file_bytes && durable_points < source.metadata().point_count() {
        control.check_cancelled()?;
        match scan_frame(
            &mut file,
            &path,
            next_frame,
            file_bytes,
            durable_points,
            source,
        )? {
            Some((leaf, frame_end)) => {
                leaves.push(leaf);
                durable_points = leaf.span.end_ordinal();
                next_frame = frame_end;
            }
            None => break,
        }
    }
    if next_frame != file_bytes {
        file.set_len(next_frame)
            .and_then(|()| file.sync_data())
            .map_err(|error| IndexError::io("truncate invalid suffix of", &path, error))?;
    }
    Ok(WorkFile {
        file,
        path,
        leaves,
        durable_points,
    })
}

fn scan_frame(
    file: &mut File,
    path: &Path,
    offset: u64,
    file_bytes: u64,
    expected_first: u64,
    source: &Source,
) -> Result<Option<(LeafRecord, u64)>, IndexError> {
    if file_bytes.saturating_sub(offset) < FRAME_PREFIX_BYTES {
        return Ok(None);
    }
    let mut prefix = [0_u8; 40];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut prefix))
        .map_err(|error| IndexError::io("read work frame from", path, error))?;
    if &prefix[..4] != FRAME_MAGIC {
        return Ok(None);
    }
    let mut decoder = Decoder::work(&prefix[4..8]);
    let Ok(payload_length) = decoder.u32("work frame payload length") else {
        return Ok(None);
    };
    let payload_length = u64::from(payload_length);
    let frame_end = offset
        .checked_add(FRAME_PREFIX_BYTES)
        .and_then(|value| value.checked_add(payload_length));
    let Some(frame_end) = frame_end.filter(|end| *end <= file_bytes) else {
        return Ok(None);
    };
    let maximum_payload =
        FRAME_FIXED_PAYLOAD_BYTES.saturating_add(MAX_NODE_SAMPLES.saturating_mul(SAMPLE_BYTES));
    if payload_length < FRAME_FIXED_PAYLOAD_BYTES || payload_length > maximum_payload {
        return Ok(None);
    }
    let payload_size = usize::try_from(payload_length).map_err(|_| IndexError::CorruptWork {
        reason: "work frame payload is not addressable",
    })?;
    let mut payload = vec![0; payload_size];
    file.read_exact(&mut payload)
        .map_err(|error| IndexError::io("read work frame from", path, error))?;
    if blake3::hash(&payload).as_bytes() != &prefix[8..40] {
        return Ok(None);
    }
    let leaf = match decode_frame(&payload, offset, expected_first, source) {
        Ok(Some(leaf)) => leaf,
        Err(error @ (IndexError::ResourceLimit { .. } | IndexError::Io { .. })) => {
            return Err(error);
        }
        Ok(None) | Err(_) => return Ok(None),
    };
    Ok(Some((leaf, frame_end)))
}

fn decode_frame(
    payload: &[u8],
    frame_offset: u64,
    expected_first: u64,
    source: &Source,
) -> Result<Option<LeafRecord>, IndexError> {
    let mut decoder = Decoder::work(payload);
    let first = decoder.u64("work frame first ordinal")?;
    let count = decoder.u64("work frame Point count")?;
    let bounds = decoder.bounds("work frame bounds")?;
    let sample_count = u64::from(decoder.u32("work frame sample count")?);
    let reserved = decoder.u32("work frame reserved value")?;
    if reserved != 0 || first != expected_first {
        return Ok(None);
    }
    let remaining = source
        .metadata()
        .point_count()
        .saturating_sub(expected_first);
    let expected_count = remaining.min(BLOCK_POINTS);
    if count != expected_count || sample_count != count.min(MAX_NODE_SAMPLES) {
        return Ok(None);
    }
    let expected_payload = FRAME_FIXED_PAYLOAD_BYTES
        .checked_add(sample_count.saturating_mul(SAMPLE_BYTES))
        .ok_or(IndexError::CorruptWork {
            reason: "work frame sample length overflowed",
        })?;
    if u64::try_from(payload.len()).unwrap_or(u64::MAX) != expected_payload {
        return Ok(None);
    }
    let span = SourceSpan::new(first, count)?;
    let sample_bytes = &payload[usize::try_from(FRAME_FIXED_PAYLOAD_BYTES).unwrap_or(72)..];
    let samples = decode_samples(sample_bytes, sample_count, "work frame")?;
    match validate_frame_values(span, bounds, &samples) {
        Ok(()) => {}
        Err(error @ IndexError::ResourceLimit { .. }) => return Err(error),
        Err(_) => return Ok(None),
    }
    if !samples_within_bounds(&samples, source.metadata().position_transform(), bounds) {
        return Ok(None);
    }
    let Some(sample_offset) = frame_offset
        .checked_add(FRAME_PREFIX_BYTES)
        .and_then(|value| value.checked_add(FRAME_FIXED_PAYLOAD_BYTES))
    else {
        return Ok(None);
    };
    Ok(Some(LeafRecord {
        span,
        bounds,
        sample_offset,
        sample_count,
        sample_checksum: sample_checksum(sample_bytes),
    }))
}

pub(crate) fn merge_samples(
    left: &[IndexSample],
    right: &[IndexSample],
    child_capacities: [usize; 2],
    retained_build_bytes: u64,
    limits: PrepareLimits,
) -> Result<Vec<IndexSample>, IndexError> {
    let retained = left
        .len()
        .saturating_add(right.len())
        .min(usize::try_from(MAX_NODE_SAMPLES).unwrap_or(4_096));
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve_exact(retained)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "build working bytes",
            required: u64::try_from(retained).unwrap_or(u64::MAX).saturating_mul(
                u64::try_from(mem::size_of::<(u64, u64, [i64; 3])>()).unwrap_or(u64::MAX),
            ),
            allowed: limits.max_build_working_bytes(),
        })?;
    let allocated_bytes = |capacity: usize, item_bytes: usize| {
        u64::try_from(capacity)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(item_bytes).unwrap_or(u64::MAX))
    };
    let before_output = retained_build_bytes
        .saturating_add(allocated_bytes(
            child_capacities[0],
            mem::size_of::<IndexSample>(),
        ))
        .saturating_add(allocated_bytes(
            child_capacities[1],
            mem::size_of::<IndexSample>(),
        ))
        .saturating_add(allocated_bytes(
            selected.capacity(),
            mem::size_of::<(u64, u64, [i64; 3])>(),
        ))
        .saturating_add(
            u64::try_from(retained)
                .unwrap_or(u64::MAX)
                .saturating_mul(SAMPLE_BYTES),
        );
    require(
        before_output,
        limits.max_build_working_bytes(),
        "build working bytes",
    )?;
    for sample in left.iter().chain(right.iter()).copied() {
        retain_bottom_k(
            &mut selected,
            (
                ordinal_priority(sample.ordinal()),
                sample.ordinal(),
                sample.ticks(),
            ),
            retained,
        );
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(retained)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "build working bytes",
            required: u64::try_from(retained)
                .unwrap_or(u64::MAX)
                .saturating_mul(SAMPLE_BYTES),
            allowed: limits.max_build_working_bytes(),
        })?;
    let actual_peak = retained_build_bytes
        .saturating_add(allocated_bytes(
            child_capacities[0],
            mem::size_of::<IndexSample>(),
        ))
        .saturating_add(allocated_bytes(
            child_capacities[1],
            mem::size_of::<IndexSample>(),
        ))
        .saturating_add(allocated_bytes(
            selected.capacity(),
            mem::size_of::<(u64, u64, [i64; 3])>(),
        ))
        .saturating_add(allocated_bytes(
            samples.capacity(),
            mem::size_of::<IndexSample>(),
        ));
    require(
        actual_peak,
        limits.max_build_working_bytes(),
        "build working bytes",
    )?;
    samples.extend(
        selected
            .into_iter()
            .map(|(_, ordinal, ticks)| IndexSample::new(ordinal, ticks)),
    );
    samples.sort_unstable_by_key(|sample| sample.ordinal());
    Ok(samples)
}

pub(crate) fn ordinal_priority(ordinal: u64) -> u64 {
    let mut value = ordinal ^ ORDINAL_HASH_DOMAIN;
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn encode_work_header(source: &Source) -> Vec<u8> {
    let mut body = Vec::with_capacity(WORK_HEADER_BODY_BYTES);
    body.extend_from_slice(WORK_MAGIC);
    push_u32(&mut body, DISK_VERSION);
    push_u32(&mut body, RECIPE_VERSION);
    push_u32(
        &mut body,
        u32::try_from(BLOCK_POINTS).expect("block size fits u32"),
    );
    push_u32(
        &mut body,
        u32::try_from(MAX_NODE_SAMPLES).expect("sample size fits u32"),
    );
    body.extend_from_slice(source.identity().as_bytes());
    push_u64(&mut body, source.metadata().point_count());
    push_transform(&mut body, source.metadata().position_transform());
    push_optional_bounds(&mut body, source.metadata().world_bounds());
    debug_assert_eq!(body.len(), WORK_HEADER_BODY_BYTES);
    let checksum = blake3::hash(&body);
    body.extend_from_slice(checksum.as_bytes());
    body
}

fn validate_work_header(source: &Source, header: &[u8]) -> Result<(), IndexError> {
    if header.len() != usize::try_from(WORK_HEADER_BYTES).unwrap_or(200) {
        return Err(IndexError::CorruptWork {
            reason: "work header has an invalid length",
        });
    }
    let (body, expected_checksum) = header.split_at(WORK_HEADER_BODY_BYTES);
    if blake3::hash(body).as_bytes() != expected_checksum {
        return Err(IndexError::CorruptWork {
            reason: "work header checksum differs",
        });
    }
    let mut decoder = Decoder::work(body);
    if decoder.array::<8>("work magic")? != *WORK_MAGIC {
        return Err(IndexError::CorruptWork {
            reason: "work header magic differs",
        });
    }
    let disk = decoder.u32("work disk version")?;
    let recipe = decoder.u32("work recipe version")?;
    if disk != DISK_VERSION {
        return Err(IndexError::UnsupportedVersion {
            kind: "incomplete-index disk",
            version: disk,
        });
    }
    if recipe != RECIPE_VERSION {
        return Err(IndexError::UnsupportedVersion {
            kind: "index recipe",
            version: recipe,
        });
    }
    if decoder.u32("work block size")? != u32::try_from(BLOCK_POINTS).unwrap_or(u32::MAX)
        || decoder.u32("work sample size")? != u32::try_from(MAX_NODE_SAMPLES).unwrap_or(u32::MAX)
    {
        return Err(IndexError::IncompatibleWork {
            reason: "fixed recipe parameters differ",
        });
    }
    let source_id = SourceId::new(decoder.array("work Source identity")?);
    let point_count = decoder.u64("work Source Point count")?;
    let transform = decoder.transform("work Source transform")?;
    let bounds = decoder.optional_bounds("work Source bounds")?;
    if source_id != source.identity() {
        return Err(IndexError::IncompatibleWork {
            reason: "Source identity differs",
        });
    }
    if point_count != source.metadata().point_count()
        || !same_transform_bits(transform, source.metadata().position_transform())
        || !same_optional_bounds_bits(bounds, source.metadata().world_bounds())
    {
        return Err(IndexError::IncompatibleWork {
            reason: "Source count, transform, or bounds differ",
        });
    }
    Ok(())
}

fn encode_frame_payload(span: SourceSpan, bounds: WorldBounds, samples: &[IndexSample]) -> Vec<u8> {
    let capacity = FRAME_FIXED_PAYLOAD_BYTES.saturating_add(
        u64::try_from(samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(SAMPLE_BYTES),
    );
    let mut payload = Vec::with_capacity(usize::try_from(capacity).unwrap_or(0));
    push_u64(&mut payload, span.first_ordinal());
    push_u64(&mut payload, span.point_count());
    push_bounds(&mut payload, bounds);
    push_u32(
        &mut payload,
        u32::try_from(samples.len()).expect("bounded sample count fits u32"),
    );
    push_u32(&mut payload, 0);
    push_samples(&mut payload, samples);
    payload
}

fn validate_frame_values(
    span: SourceSpan,
    _bounds: WorldBounds,
    samples: &[IndexSample],
) -> Result<(), IndexError> {
    let expected_samples = span.point_count().min(MAX_NODE_SAMPLES);
    if u64::try_from(samples.len()).unwrap_or(u64::MAX) != expected_samples {
        return Err(IndexError::CorruptWork {
            reason: "work frame sample count differs from recipe",
        });
    }
    if samples
        .windows(2)
        .any(|pair| pair[0].ordinal() >= pair[1].ordinal())
        || samples.iter().any(|sample| {
            sample.ordinal() < span.first_ordinal() || sample.ordinal() >= span.end_ordinal()
        })
    {
        return Err(IndexError::CorruptWork {
            reason: "work frame samples are not sorted unique members",
        });
    }
    if !has_expected_sample_ordinals(span, samples)? {
        return Err(IndexError::CorruptWork {
            reason: "work frame samples differ from stable bottom-k recipe",
        });
    }
    Ok(())
}

fn has_expected_sample_ordinals(
    span: SourceSpan,
    samples: &[IndexSample],
) -> Result<bool, IndexError> {
    let capacity = usize::try_from(span.point_count().min(MAX_NODE_SAMPLES)).unwrap_or(4_096);
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "build working bytes",
            required: u64::try_from(capacity)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::try_from(mem::size_of::<(u64, u64)>()).unwrap_or(u64::MAX)),
            allowed: u64::MAX,
        })?;
    for row in 0..span.point_count() {
        let ordinal = span.first_ordinal() + row;
        retain_bottom_k(
            &mut selected,
            (ordinal_priority(ordinal), ordinal),
            capacity,
        );
    }
    let mut ordinals = Vec::new();
    ordinals
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "build working bytes",
            required: u64::try_from(capacity)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::try_from(mem::size_of::<u64>()).unwrap_or(8)),
            allowed: u64::MAX,
        })?;
    ordinals.extend(selected.into_iter().map(|(_, ordinal)| ordinal));
    ordinals.sort_unstable();
    Ok(ordinals
        .iter()
        .copied()
        .eq(samples.iter().map(|sample| sample.ordinal())))
}

fn retain_bottom_k<T: Ord>(heap: &mut BinaryHeap<T>, value: T, capacity: usize) {
    if heap.len() < capacity {
        heap.push(value);
    } else if heap.peek().is_some_and(|largest| &value < largest) {
        let _ = heap.pop();
        heap.push(value);
    }
}

fn reserve_leaf_metadata(
    point_count: u64,
    limits: PrepareLimits,
) -> Result<Vec<LeafRecord>, IndexError> {
    let expected_leaf_count = canonical_leaf_count(point_count);
    let leaf_capacity =
        usize::try_from(expected_leaf_count).map_err(|_| IndexError::ResourceLimit {
            limit: "addressable work frames",
            required: expected_leaf_count,
            allowed: usize::MAX as u64,
        })?;
    let leaf_bytes = expected_leaf_count
        .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes,
        limits.max_build_working_bytes(),
        "build working bytes",
    )?;
    let mut leaves = Vec::new();
    leaves
        .try_reserve_exact(leaf_capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "build working bytes",
            required: leaf_bytes,
            allowed: limits.max_build_working_bytes(),
        })?;
    require(
        u64::try_from(leaves.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX)),
        limits.max_build_working_bytes(),
        "build working bytes",
    )?;
    Ok(leaves)
}

fn samples_within_bounds(
    samples: &[IndexSample],
    transform: PositionTransform,
    bounds: WorldBounds,
) -> bool {
    samples.iter().all(|sample| {
        let world = sample.world_position(transform);
        (0..3).all(|axis| {
            world[axis].is_finite()
                && bounds.min()[axis] <= world[axis]
                && world[axis] <= bounds.max()[axis]
        })
    })
}

fn sample_checksum(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(SAMPLE_HASH_DOMAIN);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn decode_samples(
    bytes: &[u8],
    count: u64,
    kind: &'static str,
) -> Result<Vec<IndexSample>, IndexError> {
    let expected_bytes = count.saturating_mul(SAMPLE_BYTES);
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != expected_bytes {
        return Err(corrupt(kind, "sample byte length differs"));
    }
    let capacity =
        usize::try_from(count).map_err(|_| corrupt(kind, "sample count is not addressable"))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "sample buffer bytes",
            required: expected_bytes,
            allowed: expected_bytes,
        })?;
    let mut decoder = if kind == "artifact" {
        Decoder::artifact(bytes)
    } else {
        Decoder::work(bytes)
    };
    for _ in 0..count {
        let ordinal = decoder.u64("sample ordinal")?;
        let ticks = [
            decoder.i64("sample x ticks")?,
            decoder.i64("sample y ticks")?,
            decoder.i64("sample z ticks")?,
        ];
        samples.push(IndexSample::new(ordinal, ticks));
    }
    if samples
        .windows(2)
        .any(|pair| pair[0].ordinal() >= pair[1].ordinal())
    {
        return Err(corrupt(kind, "samples are not sorted and unique"));
    }
    Ok(samples)
}

fn corrupt(kind: &'static str, reason: &'static str) -> IndexError {
    if kind == "artifact" {
        IndexError::CorruptArtifact { reason }
    } else {
        IndexError::CorruptWork { reason }
    }
}

fn canonical_leaf_count(point_count: u64) -> u64 {
    point_count.div_ceil(BLOCK_POINTS)
}

fn sibling_path(target: &Path, suffix: &str) -> Result<PathBuf, IndexError> {
    let file_name = target.file_name().ok_or_else(|| {
        IndexError::io(
            "derive sidecar for",
            target,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "index target must have a file name",
            ),
        )
    })?;
    let mut sibling_name = file_name.to_os_string();
    sibling_name.push(suffix);
    Ok(target.with_file_name(sibling_name))
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i64(bytes: &mut Vec<u8>, value: i64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f64(bytes: &mut Vec<u8>, value: f64) {
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn push_transform(bytes: &mut Vec<u8>, transform: PositionTransform) {
    for value in transform.offset().into_iter().chain(transform.scale()) {
        push_f64(bytes, value);
    }
}

fn push_bounds(bytes: &mut Vec<u8>, bounds: WorldBounds) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        push_f64(bytes, value);
    }
}

fn push_optional_bounds(bytes: &mut Vec<u8>, bounds: Option<WorldBounds>) {
    push_u64(bytes, u64::from(bounds.is_some()));
    if let Some(bounds) = bounds {
        push_bounds(bytes, bounds);
    } else {
        bytes.resize(bytes.len() + 48, 0);
    }
}

fn push_samples(bytes: &mut Vec<u8>, samples: &[IndexSample]) {
    for sample in samples {
        push_u64(bytes, sample.ordinal());
        for ticks in sample.ticks() {
            push_i64(bytes, ticks);
        }
    }
}

#[derive(Clone, Copy)]
enum DecodeKind {
    Artifact,
    Work,
}

struct Decoder<'bytes> {
    bytes: &'bytes [u8],
    position: usize,
    kind: DecodeKind,
}

impl<'bytes> Decoder<'bytes> {
    const fn artifact(bytes: &'bytes [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            kind: DecodeKind::Artifact,
        }
    }

    const fn work(bytes: &'bytes [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            kind: DecodeKind::Work,
        }
    }

    fn invalid(&self, reason: &'static str) -> IndexError {
        match self.kind {
            DecodeKind::Artifact => IndexError::CorruptArtifact { reason },
            DecodeKind::Work => IndexError::CorruptWork { reason },
        }
    }

    fn array<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N], IndexError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| self.invalid("persisted field offset overflowed"))?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| self.invalid(field))?;
        self.position = end;
        Ok(value.try_into().expect("slice length was checked"))
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, IndexError> {
        Ok(u32::from_le_bytes(self.array(field)?))
    }

    fn u64(&mut self, field: &'static str) -> Result<u64, IndexError> {
        Ok(u64::from_le_bytes(self.array(field)?))
    }

    fn i64(&mut self, field: &'static str) -> Result<i64, IndexError> {
        Ok(i64::from_le_bytes(self.array(field)?))
    }

    fn f64(&mut self, field: &'static str) -> Result<f64, IndexError> {
        Ok(f64::from_bits(self.u64(field)?))
    }

    fn transform(&mut self, field: &'static str) -> Result<PositionTransform, IndexError> {
        let offset = [self.f64(field)?, self.f64(field)?, self.f64(field)?];
        let scale = [self.f64(field)?, self.f64(field)?, self.f64(field)?];
        PositionTransform::new(offset, scale)
            .map_err(|_| self.invalid("persisted position transform is invalid"))
    }

    fn bounds(&mut self, field: &'static str) -> Result<WorldBounds, IndexError> {
        let minimum = [self.f64(field)?, self.f64(field)?, self.f64(field)?];
        let maximum = [self.f64(field)?, self.f64(field)?, self.f64(field)?];
        WorldBounds::new(minimum, maximum)
            .map_err(|_| self.invalid("persisted world bounds are invalid"))
    }

    fn optional_bounds(&mut self, field: &'static str) -> Result<Option<WorldBounds>, IndexError> {
        let present = self.u64(field)?;
        let raw = self.array::<48>(field)?;
        match present {
            0 if raw == [0; 48] => Ok(None),
            1 => {
                let mut decoder = match self.kind {
                    DecodeKind::Artifact => Decoder::artifact(&raw),
                    DecodeKind::Work => Decoder::work(&raw),
                };
                decoder.bounds(field).map(Some)
            }
            _ => Err(self.invalid("optional bounds marker or padding is invalid")),
        }
    }
}

fn same_transform_bits(left: PositionTransform, right: PositionTransform) -> bool {
    left.offset()
        .into_iter()
        .chain(left.scale())
        .zip(right.offset().into_iter().chain(right.scale()))
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn same_optional_bounds_bits(left: Option<WorldBounds>, right: Option<WorldBounds>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left
            .min()
            .into_iter()
            .chain(left.max())
            .zip(right.min().into_iter().chain(right.max()))
            .all(|(left, right)| left.to_bits() == right.to_bits()),
        _ => false,
    }
}

fn require(required: u64, allowed: u64, limit: &'static str) -> Result<(), IndexError> {
    if required > allowed {
        return Err(IndexError::ResourceLimit {
            limit,
            required,
            allowed,
        });
    }
    Ok(())
}

fn lock_recovering<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy)]
enum SampleStorage {
    Work,
    Spool,
}

#[derive(Clone, Copy)]
struct SampleLocation {
    storage: SampleStorage,
    offset: u64,
    count: u64,
    checksum: [u8; 32],
}

#[allow(clippy::too_many_lines)]
pub(crate) fn finalize(
    source: &Source,
    target: &Path,
    work: &mut WorkFile,
    plan: &TreePlan,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<(), IndexError> {
    control.check_cancelled()?;
    preflight_finalization(work, plan, limits)?;
    if target_exists(target)? {
        return Err(IndexError::IncompatibleArtifact {
            reason: "target appeared while its index was being built",
        });
    }
    let spool_path = sibling_path(target, ".samples")?;
    let temporary_path = sibling_path(target, ".tmp")?;
    reject_symlink(&spool_path, "sample spool")?;
    reject_symlink(&temporary_path, "temporary artifact")?;
    let mut spool = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&spool_path)
        .map_err(|error| IndexError::io("create", &spool_path, error))?;
    let mut locations = Vec::new();
    locations
        .try_reserve_exact(plan.nodes.len())
        .map_err(|_| IndexError::ResourceLimit {
            limit: "build working bytes",
            required: finalization_working_bytes(work, plan),
            allowed: limits.max_build_working_bytes(),
        })?;
    locations.resize(plan.nodes.len(), None);
    preflight_finalization_with_locations(work, plan, locations.capacity(), limits)?;
    for index in (0..plan.nodes.len()).rev() {
        control.check_cancelled()?;
        let node = plan.nodes[index];
        let location = if let Some(leaf) = node.leaf {
            SampleLocation {
                storage: SampleStorage::Work,
                offset: leaf.sample_offset,
                count: leaf.sample_count,
                checksum: leaf.sample_checksum,
            }
        } else {
            let [left, right] = node
                .children
                .expect("planned internal nodes have two children");
            let left = locations[left].expect("reverse root-first order resolves child samples");
            let right = locations[right].expect("reverse root-first order resolves child samples");
            let left_samples = read_location(work, &mut spool, &spool_path, left)?;
            let right_samples = read_location(work, &mut spool, &spool_path, right)?;
            let retained_bytes =
                retained_finalization_metadata_bytes(work, plan, locations.capacity());
            let samples = merge_samples(
                &left_samples,
                &right_samples,
                [left_samples.capacity(), right_samples.capacity()],
                retained_bytes,
                limits,
            )?;
            if u64::try_from(samples.len()).unwrap_or(u64::MAX) != node.display_point_count {
                return Err(IndexError::CorruptWork {
                    reason: "merged internal sample count differs from recipe",
                });
            }
            append_spool_samples(work, &mut spool, &spool_path, &samples, limits)?
        };
        locations[index] = Some(location);
    }
    spool
        .sync_data()
        .map_err(|error| IndexError::io("flush", &spool_path, error))?;

    let internal_sample_count = plan
        .nodes
        .iter()
        .filter(|node| node.leaf.is_none())
        .try_fold(0_u64, |count, node| {
            count.checked_add(node.display_point_count)
        })
        .ok_or(IndexError::ResourceLimit {
            limit: "artifact sample Points",
            required: u64::MAX,
            allowed: limits.max_artifact_bytes() / SAMPLE_BYTES,
        })?;
    let node_count = u64::try_from(plan.nodes.len()).unwrap_or(u64::MAX);
    let node_table_bytes =
        node_count
            .checked_mul(NODE_RECORD_BYTES)
            .ok_or(IndexError::ResourceLimit {
                limit: "artifact bytes",
                required: u64::MAX,
                allowed: limits.max_artifact_bytes(),
            })?;
    let sample_bytes =
        internal_sample_count
            .checked_mul(SAMPLE_BYTES)
            .ok_or(IndexError::ResourceLimit {
                limit: "artifact bytes",
                required: u64::MAX,
                allowed: limits.max_artifact_bytes(),
            })?;
    let sample_offset =
        ARTIFACT_HEADER_BYTES
            .checked_add(node_table_bytes)
            .ok_or(IndexError::ResourceLimit {
                limit: "artifact bytes",
                required: u64::MAX,
                allowed: limits.max_artifact_bytes(),
            })?;
    let artifact_bytes = sample_offset
        .checked_add(sample_bytes)
        .and_then(|value| value.checked_add(ARTIFACT_CHECKSUM_BYTES))
        .ok_or(IndexError::ResourceLimit {
            limit: "artifact bytes",
            required: u64::MAX,
            allowed: limits.max_artifact_bytes(),
        })?;
    require(
        artifact_bytes,
        limits.max_artifact_bytes(),
        "artifact bytes",
    )?;

    let header = encode_artifact_header(
        source,
        node_count,
        plan.leaf_count,
        node_table_bytes,
        sample_offset,
        sample_bytes,
    );
    let mut temporary = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary_path)
        .map_err(|error| IndexError::io("create", &temporary_path, error))?;
    let mut artifact_hasher = Hasher::new();
    write_hashed(
        &mut temporary,
        &temporary_path,
        &mut artifact_hasher,
        &header,
    )?;
    let mut next_artifact_sample = sample_offset;
    for (index, node) in plan.nodes.iter().copied().enumerate() {
        let location = locations[index].expect("every planned node has a sample location");
        let persisted_location = node.leaf.is_none().then_some(location);
        let node_sample_offset = if node.leaf.is_some() {
            0
        } else {
            let offset = next_artifact_sample;
            next_artifact_sample = next_artifact_sample
                .checked_add(node.display_point_count.saturating_mul(SAMPLE_BYTES))
                .ok_or(IndexError::CorruptArtifact {
                    reason: "artifact sample offsets overflowed",
                })?;
            offset
        };
        let record = encode_node_record(
            index,
            node,
            node_sample_offset,
            persisted_location.map_or([0; 32], |location| location.checksum),
        );
        write_hashed(
            &mut temporary,
            &temporary_path,
            &mut artifact_hasher,
            &record,
        )?;
    }
    for (index, node) in plan.nodes.iter().enumerate() {
        if node.leaf.is_some() {
            continue;
        }
        control.check_cancelled()?;
        let location = locations[index].expect("internal node sample location exists");
        let samples = read_location(work, &mut spool, &spool_path, location)?;
        write_samples_hashed(
            &mut temporary,
            &temporary_path,
            &mut artifact_hasher,
            &samples,
        )?;
    }
    let checksum = artifact_hasher.finalize();
    temporary
        .write_all(checksum.as_bytes())
        .and_then(|()| temporary.sync_all())
        .map_err(|error| IndexError::io("finish and flush", &temporary_path, error))?;
    let actual_bytes = temporary
        .metadata()
        .map_err(|error| IndexError::io("inspect", &temporary_path, error))?
        .len();
    if actual_bytes != artifact_bytes {
        return Err(IndexError::CorruptArtifact {
            reason: "new artifact length differs from its deterministic layout",
        });
    }
    control.check_cancelled()?;
    if target_exists(target)? {
        return Err(IndexError::IncompatibleArtifact {
            reason: "target appeared before atomic publication",
        });
    }
    match fs::hard_link(&temporary_path, target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(IndexError::IncompatibleArtifact {
                reason: "target appeared before atomic publication",
            });
        }
        Err(error) => return Err(IndexError::io("atomically publish", target, error)),
    }
    sync_parent(target)?;
    fs::remove_file(&temporary_path)
        .map_err(|error| IndexError::io("remove published temporary", &temporary_path, error))?;
    fs::remove_file(&work.path)
        .map_err(|error| IndexError::io("remove completed work file", &work.path, error))?;
    fs::remove_file(&spool_path)
        .map_err(|error| IndexError::io("remove sample spool", &spool_path, error))?;
    sync_parent(target)?;
    Ok(())
}

fn append_spool_samples(
    work: &WorkFile,
    spool: &mut File,
    spool_path: &Path,
    samples: &[IndexSample],
    limits: PrepareLimits,
) -> Result<SampleLocation, IndexError> {
    let offset = spool
        .seek(SeekFrom::End(0))
        .map_err(|error| IndexError::io("seek to end of", spool_path, error))?;
    let bytes = u64::try_from(samples.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(SAMPLE_BYTES);
    let work_bytes = work
        .file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &work.path, error))?
        .len();
    require(
        work_bytes.saturating_add(offset).saturating_add(bytes),
        limits.max_incomplete_bytes(),
        "incomplete and sample-spool bytes",
    )?;
    let mut hasher = Hasher::new();
    hasher.update(SAMPLE_HASH_DOMAIN);
    for sample in samples {
        let encoded = encode_sample(*sample);
        hasher.update(&encoded);
        spool
            .write_all(&encoded)
            .map_err(|error| IndexError::io("append to", spool_path, error))?;
    }
    Ok(SampleLocation {
        storage: SampleStorage::Spool,
        offset,
        count: u64::try_from(samples.len()).unwrap_or(u64::MAX),
        checksum: *hasher.finalize().as_bytes(),
    })
}

fn read_location(
    work: &mut WorkFile,
    spool: &mut File,
    spool_path: &Path,
    location: SampleLocation,
) -> Result<Vec<IndexSample>, IndexError> {
    match location.storage {
        SampleStorage::Work => read_samples_from(
            &mut work.file,
            &work.path,
            location.offset,
            location.count,
            location.checksum,
            "work",
        ),
        SampleStorage::Spool => read_samples_from(
            spool,
            spool_path,
            location.offset,
            location.count,
            location.checksum,
            "work",
        ),
    }
}

fn read_samples_from(
    file: &mut File,
    path: &Path,
    offset: u64,
    count: u64,
    expected_checksum: [u8; 32],
    kind: &'static str,
) -> Result<Vec<IndexSample>, IndexError> {
    let capacity =
        usize::try_from(count).map_err(|_| corrupt(kind, "sample count is not addressable"))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "sample buffer bytes",
            required: count.saturating_mul(SAMPLE_BYTES),
            allowed: count.saturating_mul(SAMPLE_BYTES),
        })?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| IndexError::io("seek in", path, error))?;
    let mut hasher = Hasher::new();
    hasher.update(SAMPLE_HASH_DOMAIN);
    for _ in 0..count {
        let mut encoded = [0_u8; 32];
        file.read_exact(&mut encoded)
            .map_err(|error| IndexError::io("read", path, error))?;
        hasher.update(&encoded);
        let mut decoder = if kind == "artifact" {
            Decoder::artifact(&encoded)
        } else {
            Decoder::work(&encoded)
        };
        let ordinal = decoder.u64("sample ordinal")?;
        let ticks = [
            decoder.i64("sample x ticks")?,
            decoder.i64("sample y ticks")?,
            decoder.i64("sample z ticks")?,
        ];
        samples.push(IndexSample::new(ordinal, ticks));
    }
    if *hasher.finalize().as_bytes() != expected_checksum
        || samples
            .windows(2)
            .any(|pair| pair[0].ordinal() >= pair[1].ordinal())
    {
        return Err(corrupt(kind, "sample block checksum or order differs"));
    }
    Ok(samples)
}

fn encode_artifact_header(
    source: &Source,
    node_count: u64,
    leaf_count: u64,
    node_table_bytes: u64,
    sample_offset: u64,
    sample_bytes: u64,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(usize::try_from(ARTIFACT_HEADER_BYTES).unwrap_or(208));
    bytes.extend_from_slice(ARTIFACT_MAGIC);
    push_u32(&mut bytes, DISK_VERSION);
    push_u32(&mut bytes, RECIPE_VERSION);
    bytes.extend_from_slice(source.identity().as_bytes());
    push_u64(&mut bytes, source.metadata().point_count());
    push_transform(&mut bytes, source.metadata().position_transform());
    push_optional_bounds(&mut bytes, source.metadata().world_bounds());
    push_u64(&mut bytes, node_count);
    push_u64(&mut bytes, leaf_count);
    push_u64(&mut bytes, ARTIFACT_HEADER_BYTES);
    push_u64(&mut bytes, node_table_bytes);
    push_u64(&mut bytes, sample_offset);
    push_u64(&mut bytes, sample_bytes);
    debug_assert_eq!(
        bytes.len(),
        usize::try_from(ARTIFACT_HEADER_BYTES).unwrap_or(208)
    );
    bytes
}

fn encode_node_record(
    index: usize,
    node: crate::tree::PlannedNode,
    sample_offset: u64,
    sample_checksum: [u8; 32],
) -> Vec<u8> {
    let stable_id = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::with_capacity(usize::try_from(NODE_RECORD_BYTES).unwrap_or(168));
    push_u64(&mut bytes, stable_id);
    push_u64(
        &mut bytes,
        node.parent.map_or(0, |parent| {
            u64::try_from(parent).unwrap_or(u64::MAX).saturating_add(1)
        }),
    );
    let children = node.children.unwrap_or([usize::MAX; 2]);
    for child in children {
        push_u64(
            &mut bytes,
            if child == usize::MAX {
                0
            } else {
                u64::try_from(child).unwrap_or(u64::MAX).saturating_add(1)
            },
        );
    }
    push_bounds(&mut bytes, node.bounds);
    push_u64(&mut bytes, node.covered_point_count);
    push_u64(&mut bytes, node.display_point_count);
    push_u64(
        &mut bytes,
        node.leaf.map_or(0, |leaf| leaf.span.first_ordinal()),
    );
    push_u64(
        &mut bytes,
        node.leaf.map_or(0, |leaf| leaf.span.point_count()),
    );
    push_u64(&mut bytes, sample_offset);
    push_f64(&mut bytes, node.geometric_error);
    bytes.push(u8::from(node.leaf.is_some()));
    bytes.resize(bytes.len() + 7, 0);
    bytes.extend_from_slice(&sample_checksum);
    debug_assert_eq!(
        bytes.len(),
        usize::try_from(NODE_RECORD_BYTES).unwrap_or(168)
    );
    bytes
}

fn write_hashed(
    file: &mut File,
    path: &Path,
    hasher: &mut Hasher,
    bytes: &[u8],
) -> Result<(), IndexError> {
    file.write_all(bytes)
        .map_err(|error| IndexError::io("write", path, error))?;
    hasher.update(bytes);
    Ok(())
}

fn write_samples_hashed(
    file: &mut File,
    path: &Path,
    hasher: &mut Hasher,
    samples: &[IndexSample],
) -> Result<(), IndexError> {
    for sample in samples {
        let encoded = encode_sample(*sample);
        write_hashed(file, path, hasher, &encoded)?;
    }
    Ok(())
}

fn encode_sample(sample: IndexSample) -> [u8; 32] {
    let mut encoded = [0_u8; 32];
    encoded[..8].copy_from_slice(&sample.ordinal().to_le_bytes());
    for (axis, ticks) in sample.ticks().into_iter().enumerate() {
        let start = 8 + axis * 8;
        encoded[start..start + 8].copy_from_slice(&ticks.to_le_bytes());
    }
    encoded
}

fn reject_symlink(path: &Path, kind: &'static str) -> Result<(), IndexError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(IndexError::CorruptWork { reason: kind })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(IndexError::io("inspect", path, error)),
    }
}

fn sync_parent(path: &Path) -> Result<(), IndexError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| IndexError::io("flush parent directory of", path, error))
}

fn preflight_finalization(
    work: &WorkFile,
    plan: &TreePlan,
    limits: PrepareLimits,
) -> Result<(), IndexError> {
    preflight_finalization_with_locations(work, plan, plan.nodes.len(), limits)
}

fn preflight_finalization_with_locations(
    work: &WorkFile,
    plan: &TreePlan,
    location_capacity: usize,
    limits: PrepareLimits,
) -> Result<(), IndexError> {
    let required = finalization_working_bytes_with_locations(work, plan, location_capacity);
    require(
        required,
        limits.max_build_working_bytes(),
        "build working bytes",
    )
}

fn finalization_working_bytes(work: &WorkFile, plan: &TreePlan) -> u64 {
    finalization_working_bytes_with_locations(work, plan, plan.nodes.len())
}

fn finalization_working_bytes_with_locations(
    work: &WorkFile,
    plan: &TreePlan,
    location_capacity: usize,
) -> u64 {
    let retained_metadata = retained_finalization_metadata_bytes(work, plan, location_capacity);
    let child_samples = MAX_NODE_SAMPLES
        .saturating_mul(u64::try_from(mem::size_of::<IndexSample>()).unwrap_or(u64::MAX))
        .saturating_mul(2);
    let selection_heap = MAX_NODE_SAMPLES
        .saturating_mul(u64::try_from(mem::size_of::<(u64, u64, [i64; 3])>()).unwrap_or(u64::MAX));
    let merged_samples = MAX_NODE_SAMPLES
        .saturating_mul(u64::try_from(mem::size_of::<IndexSample>()).unwrap_or(u64::MAX));
    retained_metadata
        .saturating_add(child_samples)
        .saturating_add(selection_heap)
        .saturating_add(merged_samples)
        .saturating_add(1_024)
}

fn retained_finalization_metadata_bytes(
    work: &WorkFile,
    plan: &TreePlan,
    location_capacity: usize,
) -> u64 {
    let allocated = |capacity: usize, item_bytes: usize| {
        u64::try_from(capacity)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(item_bytes).unwrap_or(u64::MAX))
    };
    allocated(work.leaves.capacity(), mem::size_of::<LeafRecord>())
        .saturating_add(allocated(
            plan.nodes.capacity(),
            mem::size_of::<crate::tree::PlannedNode>(),
        ))
        .saturating_add(allocated(
            location_capacity,
            mem::size_of::<Option<SampleLocation>>(),
        ))
}

struct ArtifactHeader {
    source: SourceId,
    point_count: u64,
    transform: PositionTransform,
    bounds: Option<WorldBounds>,
    node_count: u64,
    leaf_count: u64,
    node_table_offset: u64,
    node_table_bytes: u64,
    sample_offset: u64,
    sample_bytes: u64,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn open_complete(
    source: &Source,
    target: &Path,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<OpenArtifact, IndexError> {
    control.check_cancelled()?;
    reject_complete_symlink(target)?;
    let mut file = File::open(target).map_err(|error| IndexError::io("open", target, error))?;
    let artifact_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", target, error))?
        .len();
    require(
        artifact_bytes,
        limits.max_artifact_bytes(),
        "artifact bytes",
    )?;
    let minimum_bytes = ARTIFACT_HEADER_BYTES.saturating_add(ARTIFACT_CHECKSUM_BYTES);
    if artifact_bytes < minimum_bytes {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact is shorter than its fixed header and checksum",
        });
    }
    require(
        artifact_verification_buffer_bytes(artifact_bytes),
        limits.max_build_working_bytes(),
        "artifact verification working bytes",
    )?;
    let artifact_checksum =
        verify_artifact_checksum(&mut file, target, artifact_bytes, limits, control)?;

    let mut header_bytes = [0_u8; 208];
    file.seek(SeekFrom::Start(0))
        .map_err(|error| IndexError::io("seek to header in", target, error))?;
    file.read_exact(&mut header_bytes)
        .map_err(|error| IndexError::io("read header from", target, error))?;
    let header = decode_artifact_header(&header_bytes)?;
    validate_artifact_layout(&header, artifact_bytes)?;
    validate_artifact_binding(source, &header)?;
    require(
        header.node_count,
        limits.max_hierarchy_nodes(),
        "hierarchy nodes",
    )?;
    let expected_leaf_count = canonical_leaf_count(header.point_count);
    let expected_node_count = if expected_leaf_count == 0 {
        0
    } else {
        expected_leaf_count
            .checked_mul(2)
            .and_then(|value| value.checked_sub(1))
            .ok_or(IndexError::CorruptArtifact {
                reason: "canonical hierarchy node count overflowed",
            })?
    };
    if header.leaf_count != expected_leaf_count || header.node_count != expected_node_count {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact node or leaf count differs from the fixed-block recipe",
        });
    }
    let metadata_bytes = header
        .node_count
        .saturating_mul(u64::try_from(mem::size_of::<IndexNode>()).unwrap_or(u64::MAX));
    require(
        metadata_bytes,
        limits.max_resident_metadata_bytes(),
        "resident index metadata bytes",
    )?;
    let node_capacity =
        usize::try_from(header.node_count).map_err(|_| IndexError::ResourceLimit {
            limit: "addressable hierarchy nodes",
            required: header.node_count,
            allowed: usize::MAX as u64,
        })?;
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "resident index metadata bytes",
            required: metadata_bytes,
            allowed: limits.max_resident_metadata_bytes(),
        })?;
    let actual_metadata_bytes = u64::try_from(nodes.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<IndexNode>()).unwrap_or(u64::MAX));
    require(
        actual_metadata_bytes,
        limits.max_resident_metadata_bytes(),
        "resident index metadata bytes",
    )?;
    file.seek(SeekFrom::Start(header.node_table_offset))
        .map_err(|error| IndexError::io("seek to node table in", target, error))?;
    let mut expected_sample_offset = header.sample_offset;
    for index in 0..header.node_count {
        if index % 4_096 == 0 {
            control.check_cancelled()?;
        }
        let mut record = [0_u8; 168];
        file.read_exact(&mut record)
            .map_err(|error| IndexError::io("read node table from", target, error))?;
        let node = decode_node_record(index, &record, &mut expected_sample_offset)?;
        nodes.push(node);
    }
    if expected_sample_offset != header.sample_offset.saturating_add(header.sample_bytes) {
        return Err(IndexError::CorruptArtifact {
            reason: "node sample ranges do not exactly cover the sample section",
        });
    }
    validate_topology(&nodes, source, header.leaf_count, limits, control)?;
    let hierarchy = IndexHierarchy::new(nodes);
    require(
        hierarchy.estimated_resident_bytes(),
        limits.max_resident_metadata_bytes(),
        "resident index metadata bytes",
    )?;
    let reader = ArtifactReader {
        file: Arc::new(Mutex::new(file)),
        path: Arc::new(target.to_path_buf()),
    };
    validate_persisted_samples(&reader, &hierarchy, source, limits, control)?;
    let final_checksum = verify_artifact_checksum(
        &mut lock_recovering(&reader.file),
        target,
        artifact_bytes,
        limits,
        control,
    )?;
    if final_checksum != artifact_checksum
        || fs::metadata(target)
            .map_err(|error| IndexError::io("reinspect", target, error))?
            .len()
            != artifact_bytes
    {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact changed while it was being opened",
        });
    }
    let descriptor = IndexDescriptor {
        source: header.source,
        source_point_count: header.point_count,
        position_transform: header.transform,
        world_bounds: header.bounds,
        recipe_version: RECIPE_VERSION,
        disk_version: DISK_VERSION,
        node_count: header.node_count,
        leaf_count: header.leaf_count,
        artifact_checksum,
    };
    Ok(OpenArtifact {
        descriptor,
        hierarchy,
        reader,
        artifact_bytes,
    })
}

fn decode_artifact_header(bytes: &[u8; 208]) -> Result<ArtifactHeader, IndexError> {
    let mut decoder = Decoder::artifact(bytes);
    if decoder.array::<8>("artifact magic")? != *ARTIFACT_MAGIC {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact magic differs",
        });
    }
    let disk = decoder.u32("artifact disk version")?;
    let recipe = decoder.u32("artifact recipe version")?;
    if disk != DISK_VERSION {
        return Err(IndexError::UnsupportedVersion {
            kind: "complete-index disk",
            version: disk,
        });
    }
    if recipe != RECIPE_VERSION {
        return Err(IndexError::UnsupportedVersion {
            kind: "index recipe",
            version: recipe,
        });
    }
    Ok(ArtifactHeader {
        source: SourceId::new(decoder.array("artifact Source identity")?),
        point_count: decoder.u64("artifact Source Point count")?,
        transform: decoder.transform("artifact Source transform")?,
        bounds: decoder.optional_bounds("artifact Source bounds")?,
        node_count: decoder.u64("artifact node count")?,
        leaf_count: decoder.u64("artifact leaf count")?,
        node_table_offset: decoder.u64("artifact node table offset")?,
        node_table_bytes: decoder.u64("artifact node table bytes")?,
        sample_offset: decoder.u64("artifact sample offset")?,
        sample_bytes: decoder.u64("artifact sample bytes")?,
    })
}

fn validate_artifact_layout(
    header: &ArtifactHeader,
    artifact_bytes: u64,
) -> Result<(), IndexError> {
    let expected_node_bytes =
        header
            .node_count
            .checked_mul(NODE_RECORD_BYTES)
            .ok_or(IndexError::CorruptArtifact {
                reason: "artifact node table length overflowed",
            })?;
    let checksum_offset = artifact_bytes.saturating_sub(ARTIFACT_CHECKSUM_BYTES);
    if header.node_table_offset != ARTIFACT_HEADER_BYTES
        || header.node_table_bytes != expected_node_bytes
        || header.sample_offset
            != header
                .node_table_offset
                .saturating_add(header.node_table_bytes)
        || header.sample_offset.saturating_add(header.sample_bytes) != checksum_offset
        || !header.sample_bytes.is_multiple_of(SAMPLE_BYTES)
    {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact offsets or lengths are not canonical",
        });
    }
    Ok(())
}

fn validate_artifact_binding(source: &Source, header: &ArtifactHeader) -> Result<(), IndexError> {
    if header.source != source.identity() {
        return Err(IndexError::IncompatibleArtifact {
            reason: "Source identity differs",
        });
    }
    if header.point_count != source.metadata().point_count()
        || !same_transform_bits(header.transform, source.metadata().position_transform())
        || !same_optional_bounds_bits(header.bounds, source.metadata().world_bounds())
    {
        return Err(IndexError::IncompatibleArtifact {
            reason: "Source count, transform, or bounds differ",
        });
    }
    Ok(())
}

fn verify_artifact_checksum(
    file: &mut File,
    target: &Path,
    artifact_bytes: u64,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<[u8; 32], IndexError> {
    let payload_bytes = artifact_bytes.saturating_sub(ARTIFACT_CHECKSUM_BYTES);
    let buffer_length =
        usize::try_from(artifact_verification_buffer_bytes(artifact_bytes)).unwrap_or(65_536);
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(buffer_length)
        .map_err(|_| IndexError::ResourceLimit {
            limit: "artifact verification working bytes",
            required: u64::try_from(buffer_length).unwrap_or(u64::MAX),
            allowed: limits.max_build_working_bytes(),
        })?;
    require(
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        limits.max_build_working_bytes(),
        "artifact verification working bytes",
    )?;
    buffer.resize(buffer_length, 0);
    file.seek(SeekFrom::Start(0))
        .map_err(|error| IndexError::io("seek in", target, error))?;
    let mut remaining = payload_bytes;
    let mut hasher = Hasher::new();
    while remaining != 0 {
        control.check_cancelled()?;
        let count = usize::try_from(remaining.min(u64::try_from(buffer.len()).unwrap_or(u64::MAX)))
            .unwrap_or(buffer.len());
        file.read_exact(&mut buffer[..count])
            .map_err(|error| IndexError::io("read", target, error))?;
        hasher.update(&buffer[..count]);
        remaining -= u64::try_from(count).unwrap_or(0);
    }
    let mut expected = [0_u8; 32];
    file.read_exact(&mut expected)
        .map_err(|error| IndexError::io("read checksum from", target, error))?;
    let actual = *hasher.finalize().as_bytes();
    if actual != expected {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact checksum differs",
        });
    }
    Ok(expected)
}

fn artifact_verification_buffer_bytes(artifact_bytes: u64) -> u64 {
    HASH_BUFFER_BYTES.min(
        artifact_bytes
            .saturating_sub(ARTIFACT_CHECKSUM_BYTES)
            .max(1),
    )
}

fn decode_node_record(
    expected_index: u64,
    bytes: &[u8; 168],
    expected_sample_offset: &mut u64,
) -> Result<IndexNode, IndexError> {
    let mut decoder = Decoder::artifact(bytes);
    let id_value = decoder.u64("node identity")?;
    if id_value != expected_index.saturating_add(1) {
        return Err(IndexError::CorruptArtifact {
            reason: "node identities are not stable root-first values",
        });
    }
    let id = IndexNodeId::new(id_value).map_err(|_| IndexError::CorruptArtifact {
        reason: "node identity is zero",
    })?;
    let parent_value = decoder.u64("node parent")?;
    let left_value = decoder.u64("node left child")?;
    let right_value = decoder.u64("node right child")?;
    let bounds = decoder.bounds("node bounds")?;
    let covered_point_count = decoder.u64("node covered Point count")?;
    let display_point_count = decoder.u64("node display Point count")?;
    let first_ordinal = decoder.u64("node Source first ordinal")?;
    let span_count = decoder.u64("node Source span count")?;
    let sample_offset = decoder.u64("node sample offset")?;
    let geometric_error = decoder.f64("node geometric error")?;
    let coverage = decoder.array::<1>("node Coverage")?[0];
    let reserved = decoder.array::<7>("node reserved bytes")?;
    let sample_checksum = decoder.array::<32>("node sample checksum")?;
    if reserved != [0; 7]
        || covered_point_count == 0
        || display_point_count == 0
        || !geometric_error.is_finite()
        || geometric_error < 0.0
    {
        return Err(IndexError::CorruptArtifact {
            reason: "node counts, error, or reserved bytes are invalid",
        });
    }
    let parent = optional_node_id(parent_value)?;
    let is_leaf = coverage == 1;
    let (children, source_span, display_coverage) = if is_leaf {
        if left_value != 0
            || right_value != 0
            || span_count == 0
            || sample_offset != 0
            || sample_checksum != [0; 32]
            || geometric_error.to_bits() != 0.0_f64.to_bits()
            || display_point_count != covered_point_count
        {
            return Err(IndexError::CorruptArtifact {
                reason: "leaf node fields differ from the fixed recipe",
            });
        }
        let span = SourceSpan::new(first_ordinal, span_count).map_err(|_| {
            IndexError::CorruptArtifact {
                reason: "leaf Source span is invalid",
            }
        })?;
        (None, Some(span), DisplayCoverage::Complete)
    } else if coverage == 0 {
        if left_value == 0
            || right_value == 0
            || first_ordinal != 0
            || span_count != 0
            || sample_offset != *expected_sample_offset
            || display_point_count != covered_point_count.min(MAX_NODE_SAMPLES)
        {
            return Err(IndexError::CorruptArtifact {
                reason: "internal node fields differ from the fixed recipe",
            });
        }
        let left = IndexNodeId::new(left_value).map_err(|_| IndexError::CorruptArtifact {
            reason: "internal left child is zero",
        })?;
        let right = IndexNodeId::new(right_value).map_err(|_| IndexError::CorruptArtifact {
            reason: "internal right child is zero",
        })?;
        *expected_sample_offset = expected_sample_offset
            .checked_add(display_point_count.saturating_mul(SAMPLE_BYTES))
            .ok_or(IndexError::CorruptArtifact {
                reason: "node sample range overflowed",
            })?;
        (Some([left, right]), None, DisplayCoverage::Sampled)
    } else {
        return Err(IndexError::CorruptArtifact {
            reason: "node Coverage marker is invalid",
        });
    };
    Ok(IndexNode {
        id,
        parent,
        bounds,
        covered_point_count,
        display_point_count,
        geometric_error,
        coverage: display_coverage,
        children,
        source_span,
        sample_offset,
        sample_checksum,
    })
}

fn optional_node_id(value: u64) -> Result<Option<IndexNodeId>, IndexError> {
    if value == 0 {
        Ok(None)
    } else {
        IndexNodeId::new(value)
            .map(Some)
            .map_err(|_| IndexError::CorruptArtifact {
                reason: "persisted node identity is invalid",
            })
    }
}

#[allow(clippy::too_many_lines)]
fn validate_topology(
    nodes: &[IndexNode],
    source: &Source,
    leaf_count: u64,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<(), IndexError> {
    if nodes.is_empty() {
        if source.metadata().point_count() != 0 || source.metadata().world_bounds().is_some() {
            return Err(IndexError::CorruptArtifact {
                reason: "empty hierarchy is not bound to an empty Source",
            });
        }
        return Ok(());
    }
    if nodes[0].parent.is_some()
        || !same_optional_bounds_bits(Some(nodes[0].bounds), source.metadata().world_bounds())
    {
        return Err(IndexError::CorruptArtifact {
            reason: "root parent or bounds differ from the Source",
        });
    }
    let leaf_bytes =
        leaf_count.saturating_mul(u64::try_from(mem::size_of::<SourceSpan>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes.saturating_add(MAX_NODE_SAMPLES.saturating_mul(SAMPLE_BYTES)),
        limits.max_build_working_bytes(),
        "artifact validation working bytes",
    )?;
    let mut leaf_spans = Vec::new();
    leaf_spans
        .try_reserve_exact(usize::try_from(leaf_count).unwrap_or(usize::MAX))
        .map_err(|_| IndexError::ResourceLimit {
            limit: "artifact validation working bytes",
            required: leaf_bytes,
            allowed: limits.max_build_working_bytes(),
        })?;
    let actual_leaf_bytes = u64::try_from(leaf_spans.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<SourceSpan>()).unwrap_or(u64::MAX));
    require(
        actual_leaf_bytes.saturating_add(MAX_NODE_SAMPLES.saturating_mul(SAMPLE_BYTES)),
        limits.max_build_working_bytes(),
        "artifact validation working bytes",
    )?;
    for (index, node) in nodes.iter().enumerate() {
        if index % 4_096 == 0 {
            control.check_cancelled()?;
        }
        if let Some([left, right]) = node.children {
            let left = resolve_child(nodes, node, left)?;
            let right = resolve_child(nodes, node, right)?;
            if left.id == right.id
                || left.parent != Some(node.id)
                || right.parent != Some(node.id)
                || node.covered_point_count
                    != left
                        .covered_point_count
                        .checked_add(right.covered_point_count)
                        .ok_or(IndexError::CorruptArtifact {
                            reason: "child Point counts overflow",
                        })?
                || !same_bounds_bits(node.bounds, union_bounds(left.bounds, right.bounds)?)
                || node.geometric_error.to_bits() != finite_bounds_diagonal(node.bounds).to_bits()
            {
                return Err(IndexError::CorruptArtifact {
                    reason: "internal topology, bounds, count, or error differs",
                });
            }
        } else if let Some(span) = node.source_span {
            if node.covered_point_count != span.point_count() {
                return Err(IndexError::CorruptArtifact {
                    reason: "leaf covered Point count differs from its Source span",
                });
            }
            leaf_spans.push(span);
        } else {
            return Err(IndexError::CorruptArtifact {
                reason: "node is neither an internal node nor a leaf",
            });
        }
    }
    if u64::try_from(leaf_spans.len()).unwrap_or(u64::MAX) != leaf_count {
        return Err(IndexError::CorruptArtifact {
            reason: "observed leaf count differs from the header",
        });
    }
    leaf_spans.sort_unstable_by_key(|span| span.first_ordinal());
    let mut next = 0_u64;
    for span in leaf_spans {
        let expected = source
            .metadata()
            .point_count()
            .saturating_sub(next)
            .min(BLOCK_POINTS);
        if span.first_ordinal() != next || span.point_count() != expected {
            return Err(IndexError::CorruptArtifact {
                reason: "leaf spans do not exactly partition canonical Source blocks",
            });
        }
        next = span.end_ordinal();
    }
    if next != source.metadata().point_count() {
        return Err(IndexError::CorruptArtifact {
            reason: "leaf spans do not cover the complete Source",
        });
    }
    Ok(())
}

fn resolve_child<'nodes>(
    nodes: &'nodes [IndexNode],
    parent: &IndexNode,
    child: IndexNodeId,
) -> Result<&'nodes IndexNode, IndexError> {
    if child.get() <= parent.id.get() {
        return Err(IndexError::CorruptArtifact {
            reason: "child identity is not after its root-first parent",
        });
    }
    let index = usize::try_from(child.get().saturating_sub(1)).map_err(|_| {
        IndexError::CorruptArtifact {
            reason: "child identity is not addressable",
        }
    })?;
    nodes.get(index).ok_or(IndexError::CorruptArtifact {
        reason: "child identity is outside the node table",
    })
}

fn validate_persisted_samples(
    reader: &ArtifactReader,
    hierarchy: &IndexHierarchy,
    source: &Source,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<(), IndexError> {
    for node in hierarchy.nodes() {
        control.check_cancelled()?;
        if node.coverage_complete() {
            continue;
        }
        require(
            node.display_point_count.saturating_mul(SAMPLE_BYTES),
            limits.max_build_working_bytes(),
            "artifact validation working bytes",
        )?;
        let samples = reader.read_sample_block(
            node.sample_offset,
            node.display_point_count,
            node.sample_checksum,
            limits.max_build_working_bytes(),
        )?;
        if samples.iter().any(|sample| {
            sample.ordinal() >= source.metadata().point_count()
                || !samples_within_bounds(
                    std::slice::from_ref(sample),
                    source.metadata().position_transform(),
                    node.bounds,
                )
        }) {
            return Err(IndexError::CorruptArtifact {
                reason: "internal sample identity or position is outside its node",
            });
        }
    }
    Ok(())
}

fn union_bounds(left: WorldBounds, right: WorldBounds) -> Result<WorldBounds, IndexError> {
    WorldBounds::new(
        [
            left.min()[0].min(right.min()[0]),
            left.min()[1].min(right.min()[1]),
            left.min()[2].min(right.min()[2]),
        ],
        [
            left.max()[0].max(right.max()[0]),
            left.max()[1].max(right.max()[1]),
            left.max()[2].max(right.max()[2]),
        ],
    )
    .map_err(|_| IndexError::CorruptArtifact {
        reason: "child bounds cannot form a finite union",
    })
}

fn same_bounds_bits(left: WorldBounds, right: WorldBounds) -> bool {
    same_optional_bounds_bits(Some(left), Some(right))
}

fn finite_bounds_diagonal(bounds: WorldBounds) -> f64 {
    let dx = bounds.max()[0] - bounds.min()[0];
    let dy = bounds.max()[1] - bounds.min()[1];
    let dz = bounds.max()[2] - bounds.min()[2];
    let diagonal = dx.hypot(dy).hypot(dz);
    if diagonal.is_finite() {
        diagonal
    } else {
        f64::MAX
    }
}

fn reject_complete_symlink(path: &Path) -> Result<(), IndexError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(IndexError::CorruptArtifact {
            reason: "complete artifact path is a symbolic link",
        }),
        Ok(_) => Ok(()),
        Err(error) => Err(IndexError::io("inspect", path, error)),
    }
}
