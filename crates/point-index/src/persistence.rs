use std::{
    collections::BinaryHeap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::{AttributeId, PositionTransform, SourceId, WorldBounds};
use point_source::{Source, SourceSpan};

use crate::{
    DisplayAttributes, DisplayCoverage, DisplaySampleContract, IndexDescriptor, IndexError,
    IndexHierarchy, IndexLimit, IndexNode, IndexNodeId, IndexRecipe, InspectionAttributeIds,
    PrepareLimits,
    limits::require,
    model::{DISK_VERSION_V1, DISK_VERSION_V2, INSPECTION_RECIPE_VERSION, POSITION_RECIPE_VERSION},
    read::StoredSample,
    tree::{BLOCK_POINTS, LeafRecord, MAX_NODE_SAMPLES, TreePlan},
};

const WORK_MAGIC: &[u8; 8] = b"PNWRK004";
const ARTIFACT_MAGIC: &[u8; 8] = b"PNIDX004";
const FRAME_MAGIC: &[u8; 4] = b"BLK1";
const WORK_HEADER_V1_BODY_BYTES: usize = 168;
const WORK_HEADER_V1_BYTES: u64 = 200;
const WORK_HEADER_V2_BODY_BYTES: usize = 200;
const WORK_HEADER_V2_BYTES: u64 = 232;
const FRAME_PREFIX_BYTES: u64 = 40;
const FRAME_FIXED_PAYLOAD_BYTES: u64 = 72;
const ARTIFACT_HEADER_V1_BYTES: u64 = 208;
const ARTIFACT_HEADER_V2_BYTES: u64 = 240;
const NODE_RECORD_BYTES: u64 = 168;
const ARTIFACT_CHECKSUM_BYTES: u64 = 32;
const HASH_BUFFER_BYTES: u64 = 64 * 1024;
const SAMPLE_HASH_DOMAIN_V1: &[u8] = b"punctra-index-samples-v1";
const SAMPLE_HASH_DOMAIN_V2: &[u8] = b"punctra-index-samples-v2";
const ORDINAL_HASH_DOMAIN: u64 = 0x706e_6374_7261_0401;
const TEMPORARY_CREATE_ATTEMPTS: usize = 128;

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PersistenceProfile {
    recipe: IndexRecipe,
    contract: Option<DisplaySampleContract>,
}

impl PersistenceProfile {
    fn requested(
        recipe: IndexRecipe,
        contract: Option<DisplaySampleContract>,
    ) -> Result<Self, IndexError> {
        if matches!(recipe, IndexRecipe::PositionOnlyV1) != contract.is_none() {
            return Err(IndexError::InvalidAttributeProfile {
                reason: "inspection recipe and display contract differ",
            });
        }
        Ok(Self { recipe, contract })
    }

    const fn sample_bytes(self) -> u64 {
        self.recipe.sample_bytes()
    }

    const fn sample_width(self) -> usize {
        match self.recipe {
            IndexRecipe::PositionOnlyV1 => 32,
            IndexRecipe::InspectionV1(_) => 42,
        }
    }

    const fn work_header_body_bytes(self) -> usize {
        match self.recipe {
            IndexRecipe::PositionOnlyV1 => WORK_HEADER_V1_BODY_BYTES,
            IndexRecipe::InspectionV1(_) => WORK_HEADER_V2_BODY_BYTES,
        }
    }

    const fn work_header_bytes(self) -> u64 {
        match self.recipe {
            IndexRecipe::PositionOnlyV1 => WORK_HEADER_V1_BYTES,
            IndexRecipe::InspectionV1(_) => WORK_HEADER_V2_BYTES,
        }
    }

    const fn artifact_header_bytes(self) -> u64 {
        match self.recipe {
            IndexRecipe::PositionOnlyV1 => ARTIFACT_HEADER_V1_BYTES,
            IndexRecipe::InspectionV1(_) => ARTIFACT_HEADER_V2_BYTES,
        }
    }

    const fn sample_hash_domain(self) -> &'static [u8] {
        match self.recipe {
            IndexRecipe::PositionOnlyV1 => SAMPLE_HASH_DOMAIN_V1,
            IndexRecipe::InspectionV1(_) => SAMPLE_HASH_DOMAIN_V2,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ArtifactReader {
    file: Arc<Mutex<File>>,
    path: Arc<PathBuf>,
    profile: PersistenceProfile,
    identity: Arc<fs::Metadata>,
}

impl ArtifactReader {
    pub(crate) fn verify_path_binding(&self) -> Result<(), IndexError> {
        artifact_path_matches_open_file(
            &lock_recovering(&self.file),
            self.path.as_ref(),
            &self.identity,
        )?
        .then_some(())
        .ok_or(IndexError::CorruptArtifact {
            reason: "artifact path changed before preparation acknowledgement",
        })
    }

    pub(crate) fn read_sample_block(
        &self,
        offset: u64,
        count: u64,
        expected_checksum: [u8; 32],
        max_buffer_bytes: u64,
    ) -> Result<Vec<StoredSample>, IndexError> {
        let mut file = lock_recovering(&self.file);
        read_persisted_samples(
            &mut file,
            self.path.as_ref(),
            offset,
            count,
            expected_checksum,
            SampleReadContext::ArtifactAfterOpen { max_buffer_bytes },
            self.profile,
        )
    }

    pub(crate) fn read_position_sample_block(
        &self,
        offset: u64,
        count: u64,
        expected_checksum: [u8; 32],
        max_buffer_bytes: u64,
    ) -> Result<Vec<crate::IndexSample>, IndexError> {
        debug_assert!(matches!(self.profile.recipe, IndexRecipe::PositionOnlyV1));
        let mut file = lock_recovering(&self.file);
        read_persisted_position_samples(
            &mut file,
            self.path.as_ref(),
            offset,
            count,
            expected_checksum,
            max_buffer_bytes,
        )
    }
}

fn read_persisted_position_samples(
    file: &mut File,
    path: &Path,
    offset: u64,
    count: u64,
    expected_checksum: [u8; 32],
    max_buffer_bytes: u64,
) -> Result<Vec<crate::IndexSample>, IndexError> {
    let byte_count = count.checked_mul(32).ok_or(IndexError::CorruptArtifact {
        reason: "sample block length overflowed",
    })?;
    require(
        byte_count,
        max_buffer_bytes,
        IndexLimit::IndexSampleBufferBytes,
    )?;
    let capacity = usize::try_from(count).map_err(|_| IndexError::ResourceLimit {
        limit: IndexLimit::AddressableSamplePoints,
        required: count,
        allowed: usize::MAX as u64,
    })?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::SampleBufferBytes,
            required: byte_count,
            allowed: max_buffer_bytes,
        })?;
    require(
        allocated_bytes(samples.capacity(), mem::size_of::<crate::IndexSample>()),
        max_buffer_bytes,
        IndexLimit::IndexSampleBufferBytes,
    )?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| IndexError::io("seek in", path, error))?;
    let mut hasher = Hasher::new();
    hasher.update(SAMPLE_HASH_DOMAIN_V1);
    for _ in 0..count {
        let mut encoded = [0; 32];
        file.read_exact(&mut encoded).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                IndexError::CorruptArtifact {
                    reason: "node sample block was truncated after open",
                }
            } else {
                IndexError::io("read", path, error)
            }
        })?;
        hasher.update(&encoded);
        let mut decoder = Decoder::artifact(&encoded);
        samples.push(crate::IndexSample::new(
            decoder.u64("sample ordinal")?,
            [
                decoder.i64("sample x ticks")?,
                decoder.i64("sample y ticks")?,
                decoder.i64("sample z ticks")?,
            ],
        ));
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

#[derive(Clone, Copy)]
enum SampleReadContext {
    ArtifactAfterOpen {
        max_buffer_bytes: u64,
    },
    Work {
        retained_bytes: u64,
        max_build_working_bytes: u64,
    },
}

impl SampleReadContext {
    fn byte_count(self, count: u64, sample_bytes: u64) -> Result<u64, IndexError> {
        match self {
            Self::ArtifactAfterOpen { .. } => {
                count
                    .checked_mul(sample_bytes)
                    .ok_or(IndexError::CorruptArtifact {
                        reason: "sample block length overflowed",
                    })
            }
            Self::Work { .. } => Ok(count.saturating_mul(sample_bytes)),
        }
    }

    fn capacity(self, count: u64) -> Result<usize, IndexError> {
        usize::try_from(count).map_err(|_| match self {
            Self::ArtifactAfterOpen { .. } => IndexError::ResourceLimit {
                limit: IndexLimit::AddressableSamplePoints,
                required: count,
                allowed: usize::MAX as u64,
            },
            Self::Work { .. } => corrupt("work", "sample count is not addressable"),
        })
    }

    fn enforce_buffer_limit(self, capacity: usize) -> Result<(), IndexError> {
        let actual_bytes = u64::try_from(capacity)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX));
        match self {
            Self::ArtifactAfterOpen { max_buffer_bytes } => require(
                actual_bytes,
                max_buffer_bytes,
                IndexLimit::IndexSampleBufferBytes,
            ),
            Self::Work {
                retained_bytes,
                max_build_working_bytes,
            } => require(
                retained_bytes.saturating_add(actual_bytes),
                max_build_working_bytes,
                IndexLimit::BuildWorkingBytes,
            ),
        }
    }

    fn allocation_error(self, capacity: usize) -> IndexError {
        let requested_bytes = allocated_bytes(capacity, mem::size_of::<StoredSample>());
        match self {
            Self::ArtifactAfterOpen { max_buffer_bytes } => IndexError::ResourceLimit {
                limit: IndexLimit::SampleBufferBytes,
                required: requested_bytes,
                allowed: max_buffer_bytes,
            },
            Self::Work {
                retained_bytes,
                max_build_working_bytes,
            } => IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required: retained_bytes.saturating_add(requested_bytes),
                allowed: max_build_working_bytes,
            },
        }
    }

    fn decoder(self, bytes: &[u8]) -> Decoder<'_> {
        match self {
            Self::ArtifactAfterOpen { .. } => Decoder::artifact(bytes),
            Self::Work { .. } => Decoder::work(bytes),
        }
    }

    fn read_error(self, path: &Path, error: std::io::Error) -> IndexError {
        if matches!(self, Self::ArtifactAfterOpen { .. })
            && error.kind() == std::io::ErrorKind::UnexpectedEof
        {
            IndexError::CorruptArtifact {
                reason: "node sample block was truncated after open",
            }
        } else {
            IndexError::io("read", path, error)
        }
    }

    fn checksum_error(self) -> IndexError {
        match self {
            Self::ArtifactAfterOpen { .. } => IndexError::CorruptArtifact {
                reason: "node sample checksum differs after open",
            },
            Self::Work { .. } => corrupt("work", "sample block checksum or order differs"),
        }
    }

    fn order_error(self) -> IndexError {
        match self {
            Self::ArtifactAfterOpen { .. } => IndexError::CorruptArtifact {
                reason: "samples are not sorted and unique",
            },
            Self::Work { .. } => corrupt("work", "sample block checksum or order differs"),
        }
    }
}

fn read_persisted_samples(
    file: &mut File,
    path: &Path,
    offset: u64,
    count: u64,
    expected_checksum: [u8; 32],
    context: SampleReadContext,
    profile: PersistenceProfile,
) -> Result<Vec<StoredSample>, IndexError> {
    let _byte_count = context.byte_count(count, profile.sample_bytes())?;
    let capacity = context.capacity(count)?;
    context.enforce_buffer_limit(capacity)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(capacity)
        .map_err(|_| context.allocation_error(capacity))?;
    context.enforce_buffer_limit(samples.capacity())?;

    file.seek(SeekFrom::Start(offset))
        .map_err(|error| IndexError::io("seek in", path, error))?;
    let mut hasher = Hasher::new();
    hasher.update(profile.sample_hash_domain());
    for _ in 0..count {
        let mut encoded = [0_u8; 42];
        let width = usize::try_from(profile.sample_bytes()).expect("sample width fits usize");
        file.read_exact(&mut encoded[..width])
            .map_err(|error| context.read_error(path, error))?;
        hasher.update(&encoded[..width]);
        let mut decoder = context.decoder(&encoded[..width]);
        let ordinal = decoder.u64("sample ordinal")?;
        let ticks = [
            decoder.i64("sample x ticks")?,
            decoder.i64("sample y ticks")?,
            decoder.i64("sample z ticks")?,
        ];
        let sample = match profile.recipe {
            IndexRecipe::PositionOnlyV1 => StoredSample::position_only(ordinal, ticks),
            IndexRecipe::InspectionV1(_) => {
                let intensity = decoder.u16("sample intensity")?;
                let classification = decoder.array::<1>("sample classification")?[0];
                if decoder.array::<1>("sample reserved byte")?[0] != 0 {
                    return Err(context.order_error());
                }
                let rgb = [
                    decoder.u16("sample red")?,
                    decoder.u16("sample green")?,
                    decoder.u16("sample blue")?,
                ];
                if profile
                    .contract
                    .is_some_and(|contract| contract.rgb().is_none())
                    && rgb != [0; 3]
                {
                    return Err(context.order_error());
                }
                StoredSample::attributed(
                    ordinal,
                    ticks,
                    DisplayAttributes::new(intensity, classification, rgb),
                )
            }
        };
        samples.push(sample);
    }
    if *hasher.finalize().as_bytes() != expected_checksum {
        return Err(context.checksum_error());
    }
    if samples
        .windows(2)
        .any(|pair| pair[0].ordinal() >= pair[1].ordinal())
    {
        return Err(context.order_error());
    }
    Ok(samples)
}

pub(crate) struct OpenArtifact {
    pub(crate) descriptor: IndexDescriptor,
    pub(crate) hierarchy: IndexHierarchy,
    pub(crate) reader: ArtifactReader,
    pub(crate) artifact_bytes: u64,
}

impl OpenArtifact {
    pub(crate) fn verify_path_binding(&self) -> Result<(), IndexError> {
        self.reader.verify_path_binding()
    }
}

pub(crate) struct WorkFile {
    file: File,
    path: PathBuf,
    leaves: Vec<LeafRecord>,
    durable_points: u64,
    profile: PersistenceProfile,
    peak_temporary_disk_bytes: u64,
}

impl WorkFile {
    pub(crate) fn durable_points(&self) -> u64 {
        self.durable_points
    }

    pub(crate) fn leaves(&self) -> &[LeafRecord] {
        &self.leaves
    }

    pub(crate) fn leaf_capacity(&self) -> usize {
        self.leaves.capacity()
    }

    pub(crate) const fn peak_temporary_disk_bytes(&self) -> u64 {
        self.peak_temporary_disk_bytes
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn verify_path_binding(&self) -> Result<(), IndexError> {
        let opened = self
            .file
            .metadata()
            .map_err(|error| IndexError::io("reinspect open work file", &self.path, error))?;
        let current = fs::symlink_metadata(&self.path)
            .map_err(|error| IndexError::io("reinspect work path", &self.path, error))?;
        if opened.file_type().is_file()
            && current.file_type().is_file()
            && same_file_state(&opened, &current)
        {
            Ok(())
        } else {
            Err(IndexError::IncompatibleWork {
                reason: "work path changed before preparation acknowledgement",
            })
        }
    }

    fn observe_temporary_disk_bytes(
        &mut self,
        spool_bytes: u64,
        artifact_temporary_bytes: u64,
    ) -> Result<(), IndexError> {
        let work_bytes = self
            .file
            .metadata()
            .map_err(|error| IndexError::io("inspect", &self.path, error))?
            .len();
        self.peak_temporary_disk_bytes = self.peak_temporary_disk_bytes.max(
            work_bytes
                .saturating_add(spool_bytes)
                .saturating_add(artifact_temporary_bytes),
        );
        Ok(())
    }

    pub(crate) fn retained_metadata_bytes(&self) -> u64 {
        u64::try_from(self.leaves.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX))
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn append_block(
        &mut self,
        span: SourceSpan,
        bounds: WorldBounds,
        samples: &[StoredSample],
        limits: PrepareLimits,
    ) -> Result<(), IndexError> {
        let retained = span.point_count().min(MAX_NODE_SAMPLES);
        let live_sample_bytes = u64::try_from(samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX));
        let validation_bytes = live_sample_bytes.saturating_add(
            retained
                .saturating_mul(u64::try_from(mem::size_of::<(u64, u64)>()).unwrap_or(u64::MAX))
                .saturating_add(
                    retained.saturating_mul(u64::try_from(mem::size_of::<u64>()).unwrap_or(8)),
                ),
        );
        let payload_bytes = FRAME_FIXED_PAYLOAD_BYTES.saturating_add(
            u64::try_from(samples.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(self.profile.sample_bytes()),
        );
        require(
            self.retained_metadata_bytes()
                .saturating_add(validation_bytes)
                .saturating_add(payload_bytes)
                .saturating_add(FRAME_PREFIX_BYTES),
            limits.max_build_working_bytes(),
            IndexLimit::BuildWorkingBytes,
        )?;
        validate_frame_values(
            span,
            bounds,
            samples,
            self.retained_metadata_bytes()
                .saturating_add(live_sample_bytes),
            limits.max_build_working_bytes(),
        )?;
        if span.first_ordinal() != self.durable_points {
            return Err(IndexError::CorruptWork {
                reason: "new work frame is not ordinal-contiguous",
            });
        }
        let leaf_bytes = u64::try_from(self.leaves.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
        require(
            leaf_bytes
                .saturating_add(live_sample_bytes)
                .saturating_add(payload_bytes)
                .saturating_add(FRAME_PREFIX_BYTES),
            limits.max_build_working_bytes(),
            IndexLimit::BuildWorkingBytes,
        )?;
        if self.leaves.len() == self.leaves.capacity() {
            return Err(IndexError::CorruptWork {
                reason: "work contains more frames than the canonical Source block count",
            });
        }
        let retained_while_encoding = leaf_bytes
            .saturating_add(live_sample_bytes)
            .saturating_add(FRAME_PREFIX_BYTES);
        let payload = encode_frame_payload(
            span,
            bounds,
            samples,
            self.profile,
            retained_while_encoding,
            limits,
        )?;
        let payload_length =
            u32::try_from(payload.len()).map_err(|_| IndexError::ResourceLimit {
                limit: IndexLimit::WorkFramePayloadBytes,
                required: u64::try_from(payload.len()).unwrap_or(u64::MAX),
                allowed: u64::from(u32::MAX),
            })?;
        let frame_bytes = FRAME_PREFIX_BYTES
            .checked_add(u64::from(payload_length))
            .ok_or(IndexError::ResourceLimit {
                limit: IndexLimit::IncompleteIndexBytes,
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
            IndexLimit::IncompleteIndexBytes,
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
        self.observe_temporary_disk_bytes(0, 0)?;

        let sample_offset = frame_offset + FRAME_PREFIX_BYTES + FRAME_FIXED_PAYLOAD_BYTES;
        let sample_bytes = &payload[usize::try_from(FRAME_FIXED_PAYLOAD_BYTES).unwrap_or(72)..];
        self.leaves.push(LeafRecord {
            span,
            bounds,
            sample_offset,
            sample_count: u64::try_from(samples.len()).unwrap_or(u64::MAX),
            sample_checksum: sample_checksum(sample_bytes, self.profile),
        });
        self.durable_points = span.end_ordinal();
        Ok(())
    }
}

pub(crate) fn target_exists(target: &Path) -> Result<bool, IndexError> {
    match fs::symlink_metadata(target) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(IndexError::io("inspect", target, error)),
    }
}

pub(crate) fn open_or_create_work(
    source: &Source,
    target: &Path,
    recipe: IndexRecipe,
    contract: Option<DisplaySampleContract>,
    limits: PrepareLimits,
    resume_existing: bool,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    let profile = PersistenceProfile::requested(recipe, contract)?;
    preflight_work_initialization(source, profile, limits, control)?;
    let path = sibling_path(target, ".work")?;
    reject_symlink(&path, "work path is a symbolic link")?;
    let opened = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) if resume_existing => InitialWorkOpen::Existing(file),
        Ok(_) => {
            return Err(IndexError::IncompatibleWork {
                reason: "fresh preparation work path already exists",
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_or_open_initialized_work(source, target, &path, profile)?
        }
        Err(error) => return Err(IndexError::io("open", path, error)),
    };
    let file = match opened {
        InitialWorkOpen::Existing(_) if !resume_existing => {
            return Err(IndexError::IncompatibleWork {
                reason: "fresh preparation work path appeared concurrently",
            });
        }
        InitialWorkOpen::Existing(file) => {
            acquire_work_ownership(&file, target)?;
            file
        }
        InitialWorkOpen::InitializedLocked(file) => {
            if target_exists(target)? {
                return Err(IndexError::IncompatibleArtifact {
                    reason: "target appeared while its index was being initialized",
                });
            }
            let leaves = reserve_leaf_metadata(source.metadata().point_count(), limits)?;
            return Ok(WorkFile {
                file,
                path,
                leaves,
                durable_points: 0,
                profile,
                peak_temporary_disk_bytes: profile.work_header_bytes(),
            });
        }
    };
    let file_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &path, error))?
        .len();
    if file_bytes == 0 {
        return Err(IndexError::CorruptWork {
            reason: "work path is empty and has no provable index ownership",
        });
    }
    scan_work(source, path, file, profile, limits, control)
}

enum InitialWorkOpen {
    Existing(File),
    InitializedLocked(File),
}

/// Publishes the first work header only after its complete bytes are durable.
///
/// A unique temporary name proves ownership while the header is being written.
/// The final `.work` name is created with a no-replace hard link, so another
/// writer or a caller-owned path is never overwritten. Once the link exists,
/// every retry sees either a complete header or a pre-existing path that is
/// preserved and validated normally.
fn create_or_open_initialized_work(
    source: &Source,
    target: &Path,
    work_path: &Path,
    profile: PersistenceProfile,
) -> Result<InitialWorkOpen, IndexError> {
    create_or_open_initialized_work_with(source, target, work_path, profile, &mut |_, _, _| Ok(()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InitialWorkBoundary {
    WriteHeader,
    SyncHeader,
    PublishLink,
    SyncPublishedParent,
    RemoveTemporary,
    SyncCleanupParent,
}

fn create_or_open_initialized_work_with(
    source: &Source,
    target: &Path,
    work_path: &Path,
    profile: PersistenceProfile,
    reach: &mut impl FnMut(InitialWorkBoundary, &Path, &Path) -> std::io::Result<()>,
) -> Result<InitialWorkOpen, IndexError> {
    let mut temporary = OwnedTemporaryFile::create(target, "work-header", true)?;
    let temporary_path = Arc::clone(&temporary.path);
    let header = encode_work_header(source, profile);
    reach(InitialWorkBoundary::WriteHeader, &temporary_path, work_path).map_err(|error| {
        IndexError::io_shared(
            "write initial work header",
            Arc::clone(&temporary_path),
            error,
        )
    })?;
    temporary.file_mut().write_all(&header).map_err(|error| {
        IndexError::io_shared(
            "write initial work header",
            Arc::clone(&temporary_path),
            error,
        )
    })?;
    reach(InitialWorkBoundary::SyncHeader, &temporary_path, work_path).map_err(|error| {
        IndexError::io_shared(
            "flush initial work header",
            Arc::clone(&temporary_path),
            error,
        )
    })?;
    temporary.file_mut().sync_all().map_err(|error| {
        IndexError::io_shared(
            "flush initial work header",
            Arc::clone(&temporary_path),
            error,
        )
    })?;
    acquire_work_ownership(temporary.file_mut(), target)?;

    reach(InitialWorkBoundary::PublishLink, &temporary_path, work_path).map_err(|error| {
        IndexError::io("atomically publish initial work header", work_path, error)
    })?;
    if !temporary.source_path_matches_open_file()? {
        return Err(IndexError::CorruptWork {
            reason: "owned initial work-header path changed before publication",
        });
    }
    if link_initial_work_header(&mut temporary, work_path)? {
        // First make the complete final name durable. Failure leaves a
        // valid, safely retryable work header at that name.
        reach(
            InitialWorkBoundary::SyncPublishedParent,
            &temporary_path,
            work_path,
        )
        .map_err(|error| IndexError::io("flush parent directory of", work_path, error))?;
        sync_parent(work_path)?;
        reach(
            InitialWorkBoundary::RemoveTemporary,
            &temporary_path,
            work_path,
        )
        .map_err(|error| {
            IndexError::io_shared(
                "remove initial work-header temporary",
                Arc::clone(&temporary_path),
                error,
            )
        })?;
        temporary.remove_owned_path("remove initial work-header temporary")?;
        reach(
            InitialWorkBoundary::SyncCleanupParent,
            &temporary_path,
            work_path,
        )
        .map_err(|error| IndexError::io("flush parent directory of", work_path, error))?;
        sync_parent(work_path)?;
        if !initial_work_header_matches(&mut temporary, &header)?
            || !temporary.target_matches_open_file(work_path)?
        {
            return Err(IndexError::CorruptWork {
                reason: "published initial work header changed before acknowledgement",
            });
        }
        let file = temporary
            .file
            .take()
            .expect("owned work-header temporary is open");
        return Ok(InitialWorkOpen::InitializedLocked(file));
    }
    drop(temporary);
    reject_symlink(work_path, "work path became a symbolic link")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(work_path)
        .map_err(|error| IndexError::io("open raced", work_path, error))?;
    Ok(InitialWorkOpen::Existing(file))
}

fn link_initial_work_header(
    temporary: &mut OwnedTemporaryFile,
    work_path: &Path,
) -> Result<bool, IndexError> {
    match fs::hard_link(&temporary.path, work_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => {
            return Err(IndexError::io(
                "atomically publish initial work header",
                work_path,
                error,
            ));
        }
    }
    let source_matches = temporary.source_path_matches_open_file()?;
    let target_matches = temporary.target_matches_open_file(work_path)?;
    if target_matches {
        // Even a conservative failure must not truncate a correctly linked
        // target through the retained open handle.
        temporary.mark_linked();
    }
    if !source_matches || !target_matches {
        return Err(IndexError::CorruptWork {
            reason: "initial work-header publication did not bind the owned file",
        });
    }
    Ok(true)
}

fn initial_work_header_matches(
    temporary: &mut OwnedTemporaryFile,
    expected: &[u8],
) -> Result<bool, IndexError> {
    let temporary_path = Arc::clone(&temporary.path);
    let file = temporary.file_mut();
    let expected_len = u64::try_from(expected.len()).unwrap_or(u64::MAX);
    if file
        .metadata()
        .map_err(|error| {
            IndexError::io_shared(
                "inspect published initial work header",
                Arc::clone(&temporary_path),
                error,
            )
        })?
        .len()
        != expected_len
    {
        return Ok(false);
    }
    let mut observed = [0_u8; 232];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut observed[..expected.len()]))
        .and_then(|()| file.seek(SeekFrom::End(0)).map(|_| ()))
        .map_err(|error| {
            IndexError::io_shared(
                "verify published initial work header",
                temporary_path,
                error,
            )
        })?;
    Ok(observed[..expected.len()] == *expected)
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

fn preflight_work_initialization(
    source: &Source,
    profile: PersistenceProfile,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<(), IndexError> {
    control.check_cancelled()?;
    require(
        profile.work_header_bytes(),
        limits.max_incomplete_bytes(),
        IndexLimit::IncompleteIndexBytes,
    )?;
    let leaf_count = canonical_leaf_count(source.metadata().point_count());
    let leaf_bytes =
        leaf_count.saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes.saturating_add(profile.work_header_bytes()),
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )
}

fn scan_work(
    source: &Source,
    path: PathBuf,
    mut file: File,
    expected_profile: PersistenceProfile,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    control.check_cancelled()?;
    let metadata = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &path, error))?;
    let file_bytes = metadata.len();
    require(
        file_bytes,
        limits.max_incomplete_bytes(),
        IndexLimit::IncompleteIndexBytes,
    )?;
    if file_bytes < 16 {
        return Err(IndexError::CorruptWork {
            reason: "work header is truncated",
        });
    }
    let actual_profile = read_work_profile(&mut file, &path, file_bytes, source)?;
    if actual_profile != expected_profile {
        return Err(IndexError::IncompatibleWork {
            reason: "index recipe or inspection Attribute profile differs",
        });
    }
    let header_bytes = actual_profile.work_header_bytes();
    let expected_leaf_count = canonical_leaf_count(source.metadata().point_count());
    let leaf_bytes = expected_leaf_count
        .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    let maximum_payload = FRAME_FIXED_PAYLOAD_BYTES
        .saturating_add(MAX_NODE_SAMPLES.saturating_mul(actual_profile.sample_bytes()));
    let scan_buffers = maximum_payload
        .saturating_add(
            MAX_NODE_SAMPLES
                .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX)),
        )
        .saturating_add(
            MAX_NODE_SAMPLES
                .saturating_mul(u64::try_from(mem::size_of::<(u64, u64)>()).unwrap_or(u64::MAX)),
        )
        .saturating_add(
            MAX_NODE_SAMPLES.saturating_mul(u64::try_from(mem::size_of::<u64>()).unwrap_or(8)),
        )
        .saturating_add(header_bytes)
        .saturating_add(FRAME_PREFIX_BYTES);
    require(
        leaf_bytes.saturating_add(scan_buffers),
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut header = vec![0; usize::try_from(header_bytes).unwrap_or(232)];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| IndexError::io("read", &path, error))?;
    let validated_profile = validate_work_header(source, &header)?;
    debug_assert_eq!(validated_profile, actual_profile);

    let mut leaves = reserve_leaf_metadata(source.metadata().point_count(), limits)?;
    let leaf_bytes = u64::try_from(leaves.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes.saturating_add(scan_buffers),
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;

    let mut next_frame = header_bytes;
    let mut durable_points = 0_u64;
    while next_frame < file_bytes && durable_points < source.metadata().point_count() {
        control.check_cancelled()?;
        let context = ScanFrameContext {
            file_bytes,
            source,
            profile: actual_profile,
            retained_bytes: leaf_bytes
                .saturating_add(allocated_bytes(header.capacity(), mem::size_of::<u8>()))
                .saturating_add(FRAME_PREFIX_BYTES),
            max_build_working_bytes: limits.max_build_working_bytes(),
        };
        match scan_frame(&mut file, &path, next_frame, durable_points, context)? {
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
        profile: actual_profile,
        peak_temporary_disk_bytes: file_bytes,
    })
}

#[derive(Clone, Copy)]
struct ScanFrameContext<'a> {
    file_bytes: u64,
    source: &'a Source,
    profile: PersistenceProfile,
    retained_bytes: u64,
    max_build_working_bytes: u64,
}

fn scan_frame(
    file: &mut File,
    path: &Path,
    offset: u64,
    expected_first: u64,
    context: ScanFrameContext<'_>,
) -> Result<Option<(LeafRecord, u64)>, IndexError> {
    let ScanFrameContext {
        file_bytes,
        source,
        profile,
        retained_bytes,
        max_build_working_bytes,
    } = context;
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
    let maximum_payload = FRAME_FIXED_PAYLOAD_BYTES
        .saturating_add(MAX_NODE_SAMPLES.saturating_mul(profile.sample_bytes()));
    if payload_length < FRAME_FIXED_PAYLOAD_BYTES || payload_length > maximum_payload {
        return Ok(None);
    }
    let payload_size = usize::try_from(payload_length).map_err(|_| IndexError::CorruptWork {
        reason: "work frame payload is not addressable",
    })?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(payload_size)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: maximum_payload,
            allowed: max_build_working_bytes,
        })?;
    let actual_payload_bytes = u64::try_from(payload.capacity()).unwrap_or(u64::MAX);
    if retained_bytes.saturating_add(actual_payload_bytes) > max_build_working_bytes {
        return Err(IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: retained_bytes.saturating_add(actual_payload_bytes),
            allowed: max_build_working_bytes,
        });
    }
    payload.resize(payload_size, 0);
    file.read_exact(&mut payload)
        .map_err(|error| IndexError::io("read work frame from", path, error))?;
    if blake3::hash(&payload).as_bytes() != &prefix[8..40] {
        return Ok(None);
    }
    let leaf = match decode_frame(
        &payload,
        offset,
        expected_first,
        source,
        profile,
        retained_bytes.saturating_add(actual_payload_bytes),
        max_build_working_bytes,
    ) {
        Ok(Some(leaf)) => leaf,
        Err(
            error @ (IndexError::ResourceLimit { .. }
            | IndexError::Io { .. }
            | IndexError::SharedPathIo { .. }),
        ) => {
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
    profile: PersistenceProfile,
    retained_bytes: u64,
    max_build_working_bytes: u64,
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
        .checked_add(sample_count.saturating_mul(profile.sample_bytes()))
        .ok_or(IndexError::CorruptWork {
            reason: "work frame sample length overflowed",
        })?;
    if u64::try_from(payload.len()).unwrap_or(u64::MAX) != expected_payload {
        return Ok(None);
    }
    let span = SourceSpan::new(first, count)?;
    let sample_bytes = &payload[usize::try_from(FRAME_FIXED_PAYLOAD_BYTES).unwrap_or(72)..];
    let samples = decode_samples(
        sample_bytes,
        sample_count,
        "work frame",
        profile,
        retained_bytes,
        max_build_working_bytes,
    )?;
    let sample_allocation_bytes =
        allocated_bytes(samples.capacity(), mem::size_of::<StoredSample>());
    match validate_frame_values(
        span,
        bounds,
        &samples,
        retained_bytes.saturating_add(sample_allocation_bytes),
        max_build_working_bytes,
    ) {
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
        sample_checksum: sample_checksum(sample_bytes, profile),
    }))
}

pub(crate) fn merge_samples(
    left: &[StoredSample],
    right: &[StoredSample],
    child_capacities: [usize; 2],
    retained_build_bytes: u64,
    limits: PrepareLimits,
) -> Result<Vec<StoredSample>, IndexError> {
    let retained = left
        .len()
        .saturating_add(right.len())
        .min(usize::try_from(MAX_NODE_SAMPLES).unwrap_or(4_096));
    let allocated_bytes = |capacity: usize, item_bytes: usize| {
        u64::try_from(capacity)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(item_bytes).unwrap_or(u64::MAX))
    };
    let child_bytes =
        allocated_bytes(child_capacities[0], mem::size_of::<StoredSample>()).saturating_add(
            allocated_bytes(child_capacities[1], mem::size_of::<StoredSample>()),
        );
    let requested_heap_bytes =
        allocated_bytes(retained, mem::size_of::<(u64, u64, StoredSample)>());
    let requested_output_bytes = allocated_bytes(retained, mem::size_of::<StoredSample>());
    let requested_peak = retained_build_bytes
        .saturating_add(child_bytes)
        .saturating_add(requested_heap_bytes)
        .saturating_add(requested_output_bytes);
    require(
        requested_peak,
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve_exact(retained)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: requested_peak,
            allowed: limits.max_build_working_bytes(),
        })?;
    let actual_heap_bytes = allocated_bytes(
        selected.capacity(),
        mem::size_of::<(u64, u64, StoredSample)>(),
    );
    let before_output = retained_build_bytes
        .saturating_add(child_bytes)
        .saturating_add(actual_heap_bytes)
        .saturating_add(requested_output_bytes);
    require(
        before_output,
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    for sample in left.iter().chain(right.iter()).copied() {
        retain_bottom_k(
            &mut selected,
            (ordinal_priority(sample.ordinal()), sample.ordinal(), sample),
            retained,
        );
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(retained)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: u64::try_from(retained)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX)),
            allowed: limits.max_build_working_bytes(),
        })?;
    let actual_peak = retained_build_bytes
        .saturating_add(child_bytes)
        .saturating_add(actual_heap_bytes)
        .saturating_add(allocated_bytes(
            samples.capacity(),
            mem::size_of::<StoredSample>(),
        ));
    require(
        actual_peak,
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    samples.extend(selected.into_iter().map(|(_, _, sample)| sample));
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

fn encode_work_header(source: &Source, profile: PersistenceProfile) -> Vec<u8> {
    let mut body = Vec::with_capacity(profile.work_header_body_bytes());
    body.extend_from_slice(WORK_MAGIC);
    push_u32(&mut body, profile.recipe.disk_version());
    push_u32(&mut body, profile.recipe.recipe_version());
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
    push_profile_extension(&mut body, profile);
    debug_assert_eq!(body.len(), profile.work_header_body_bytes());
    let checksum = blake3::hash(&body);
    body.extend_from_slice(checksum.as_bytes());
    body
}

fn validate_work_header(source: &Source, header: &[u8]) -> Result<PersistenceProfile, IndexError> {
    if header.len() < 16 {
        return Err(IndexError::CorruptWork {
            reason: "work header has an invalid length",
        });
    }
    let body_length = header.len().saturating_sub(32);
    let (body, expected_checksum) = header.split_at(body_length);
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
    let profile = decode_profile_extension(&mut decoder, disk, recipe, "work")?;
    if header.len() != usize::try_from(profile.work_header_bytes()).unwrap_or(232) {
        return Err(IndexError::CorruptWork {
            reason: "work header has an invalid length",
        });
    }
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
    Ok(profile)
}

fn read_work_profile(
    file: &mut File,
    path: &Path,
    file_bytes: u64,
    source: &Source,
) -> Result<PersistenceProfile, IndexError> {
    let mut prefix = [0_u8; 16];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut prefix))
        .map_err(|error| IndexError::io("read", path, error))?;
    let mut decoder = Decoder::work(&prefix);
    if decoder.array::<8>("work magic")? != *WORK_MAGIC {
        return Err(IndexError::CorruptWork {
            reason: "work header magic differs",
        });
    }
    let disk = decoder.u32("work disk version")?;
    let _recipe = decoder.u32("work recipe version")?;
    let header_bytes = match disk {
        DISK_VERSION_V1 => WORK_HEADER_V1_BYTES,
        DISK_VERSION_V2 => WORK_HEADER_V2_BYTES,
        version => {
            return Err(IndexError::UnsupportedVersion {
                kind: "incomplete-index disk",
                version,
            });
        }
    };
    if file_bytes < header_bytes {
        return Err(IndexError::CorruptWork {
            reason: "work header is truncated",
        });
    }
    let mut header = vec![0; usize::try_from(header_bytes).unwrap_or(232)];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| IndexError::io("read", path, error))?;
    validate_work_header(source, &header)
}

fn encode_frame_payload(
    span: SourceSpan,
    bounds: WorldBounds,
    samples: &[StoredSample],
    profile: PersistenceProfile,
    retained_bytes: u64,
    limits: PrepareLimits,
) -> Result<Vec<u8>, IndexError> {
    let capacity = FRAME_FIXED_PAYLOAD_BYTES.saturating_add(
        u64::try_from(samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(profile.sample_bytes()),
    );
    let requested = usize::try_from(capacity).map_err(|_| IndexError::ResourceLimit {
        limit: IndexLimit::BuildWorkingBytes,
        required: capacity,
        allowed: limits.max_build_working_bytes(),
    })?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(requested)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: retained_bytes.saturating_add(capacity),
            allowed: limits.max_build_working_bytes(),
        })?;
    let actual_capacity = u64::try_from(payload.capacity()).unwrap_or(u64::MAX);
    require(
        retained_bytes.saturating_add(actual_capacity),
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    push_u64(&mut payload, span.first_ordinal());
    push_u64(&mut payload, span.point_count());
    push_bounds(&mut payload, bounds);
    push_u32(
        &mut payload,
        u32::try_from(samples.len()).expect("bounded sample count fits u32"),
    );
    push_u32(&mut payload, 0);
    push_samples(&mut payload, samples, profile);
    Ok(payload)
}

fn validate_frame_values(
    span: SourceSpan,
    _bounds: WorldBounds,
    samples: &[StoredSample],
    retained_bytes: u64,
    allowed_bytes: u64,
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
    if !has_expected_sample_ordinals(span, samples, retained_bytes, allowed_bytes)? {
        return Err(IndexError::CorruptWork {
            reason: "work frame samples differ from stable bottom-k recipe",
        });
    }
    Ok(())
}

fn has_expected_sample_ordinals(
    span: SourceSpan,
    samples: &[StoredSample],
    retained_bytes: u64,
    allowed_bytes: u64,
) -> Result<bool, IndexError> {
    let capacity = usize::try_from(span.point_count().min(MAX_NODE_SAMPLES)).unwrap_or(4_096);
    let requested_heap_bytes = allocated_bytes(capacity, mem::size_of::<(u64, u64)>());
    require(
        retained_bytes.saturating_add(requested_heap_bytes),
        allowed_bytes,
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: retained_bytes.saturating_add(requested_heap_bytes),
            allowed: allowed_bytes,
        })?;
    let heap_bytes = allocated_bytes(selected.capacity(), mem::size_of::<(u64, u64)>());
    require(
        retained_bytes.saturating_add(heap_bytes),
        allowed_bytes,
        IndexLimit::BuildWorkingBytes,
    )?;
    for row in 0..span.point_count() {
        let ordinal = span.first_ordinal() + row;
        retain_bottom_k(
            &mut selected,
            (ordinal_priority(ordinal), ordinal),
            capacity,
        );
    }
    let requested_ordinal_bytes = allocated_bytes(capacity, mem::size_of::<u64>());
    require(
        retained_bytes
            .saturating_add(heap_bytes)
            .saturating_add(requested_ordinal_bytes),
        allowed_bytes,
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut ordinals = Vec::new();
    ordinals
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: retained_bytes
                .saturating_add(heap_bytes)
                .saturating_add(requested_ordinal_bytes),
            allowed: allowed_bytes,
        })?;
    let ordinal_bytes = allocated_bytes(ordinals.capacity(), mem::size_of::<u64>());
    require(
        retained_bytes
            .saturating_add(heap_bytes)
            .saturating_add(ordinal_bytes),
        allowed_bytes,
        IndexLimit::BuildWorkingBytes,
    )?;
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
            limit: IndexLimit::AddressableWorkFrames,
            required: expected_leaf_count,
            allowed: usize::MAX as u64,
        })?;
    let leaf_bytes = expected_leaf_count
        .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes,
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut leaves = Vec::new();
    leaves
        .try_reserve_exact(leaf_capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required: leaf_bytes,
            allowed: limits.max_build_working_bytes(),
        })?;
    require(
        u64::try_from(leaves.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX)),
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )?;
    Ok(leaves)
}

fn samples_within_bounds(
    samples: &[StoredSample],
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

fn sample_checksum(bytes: &[u8], profile: PersistenceProfile) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(profile.sample_hash_domain());
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn decode_samples(
    bytes: &[u8],
    count: u64,
    kind: &'static str,
    profile: PersistenceProfile,
    retained_bytes: u64,
    allowed_bytes: u64,
) -> Result<Vec<StoredSample>, IndexError> {
    let expected_bytes = count.saturating_mul(profile.sample_bytes());
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != expected_bytes {
        return Err(corrupt(kind, "sample byte length differs"));
    }
    let capacity =
        usize::try_from(count).map_err(|_| corrupt(kind, "sample count is not addressable"))?;
    let requested_sample_bytes = allocated_bytes(capacity, mem::size_of::<StoredSample>());
    require(
        retained_bytes.saturating_add(requested_sample_bytes),
        allowed_bytes,
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::SampleBufferBytes,
            required: retained_bytes.saturating_add(requested_sample_bytes),
            allowed: allowed_bytes,
        })?;
    require(
        retained_bytes.saturating_add(allocated_bytes(
            samples.capacity(),
            mem::size_of::<StoredSample>(),
        )),
        allowed_bytes,
        IndexLimit::BuildWorkingBytes,
    )?;
    let mut decoder = if kind == "artifact" {
        Decoder::artifact(bytes)
    } else {
        Decoder::work(bytes)
    };
    for _ in 0..count {
        samples.push(decode_sample(&mut decoder, profile)?);
    }
    if samples
        .windows(2)
        .any(|pair| pair[0].ordinal() >= pair[1].ordinal())
    {
        return Err(corrupt(kind, "samples are not sorted and unique"));
    }
    Ok(samples)
}

fn decode_sample(
    decoder: &mut Decoder<'_>,
    profile: PersistenceProfile,
) -> Result<StoredSample, IndexError> {
    let ordinal = decoder.u64("sample ordinal")?;
    let ticks = [
        decoder.i64("sample x ticks")?,
        decoder.i64("sample y ticks")?,
        decoder.i64("sample z ticks")?,
    ];
    let IndexRecipe::InspectionV1(_) = profile.recipe else {
        return Ok(StoredSample::position_only(ordinal, ticks));
    };
    let intensity = decoder.u16("sample intensity")?;
    let classification = decoder.array::<1>("sample classification")?[0];
    if decoder.array::<1>("sample reserved byte")?[0] != 0 {
        return Err(decoder.invalid("sample reserved byte is nonzero"));
    }
    let rgb = [
        decoder.u16("sample red")?,
        decoder.u16("sample green")?,
        decoder.u16("sample blue")?,
    ];
    if profile
        .contract
        .is_some_and(|contract| contract.rgb().is_none())
        && rgb != [0; 3]
    {
        return Err(decoder.invalid("unavailable RGB sample values are nonzero"));
    }
    Ok(StoredSample::attributed(
        ordinal,
        ticks,
        DisplayAttributes::new(intensity, classification, rgb),
    ))
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

fn push_profile_extension(bytes: &mut Vec<u8>, profile: PersistenceProfile) {
    let IndexRecipe::InspectionV1(ids) = profile.recipe else {
        return;
    };
    push_u32(bytes, 1);
    let capabilities = 0b11
        | u32::from(
            profile
                .contract
                .is_some_and(|contract| contract.rgb().is_some()),
        ) << 2;
    push_u32(bytes, capabilities);
    for id in ids.all() {
        push_u32(bytes, id.get());
    }
    push_u32(bytes, 42);
}

fn decode_profile_extension(
    decoder: &mut Decoder<'_>,
    disk: u32,
    recipe: u32,
    kind: &'static str,
) -> Result<PersistenceProfile, IndexError> {
    match disk {
        DISK_VERSION_V1 => {
            if recipe != POSITION_RECIPE_VERSION {
                return Err(IndexError::UnsupportedVersion {
                    kind: "index recipe",
                    version: recipe,
                });
            }
            Ok(PersistenceProfile {
                recipe: IndexRecipe::PositionOnlyV1,
                contract: None,
            })
        }
        DISK_VERSION_V2 => {
            if recipe != INSPECTION_RECIPE_VERSION {
                return Err(IndexError::UnsupportedVersion {
                    kind: "index recipe",
                    version: recipe,
                });
            }
            let schema = decoder.u32("display sample schema version")?;
            if schema != 1 {
                return Err(IndexError::UnsupportedVersion {
                    kind: "display sample schema",
                    version: schema,
                });
            }
            let capabilities = decoder.u32("display sample capabilities")?;
            if capabilities & !0b111 != 0 || capabilities & 0b11 != 0b11 {
                return Err(corrupt(kind, "display sample capabilities are invalid"));
            }
            let mut raw_ids = [0_u32; 5];
            for raw in &mut raw_ids {
                *raw = decoder.u32("display sample Attribute identity")?;
            }
            if decoder.u32("display sample record bytes")? != 42 {
                return Err(corrupt(kind, "display sample record width differs"));
            }
            let mut ids = [AttributeId::new(1).expect("one is nonzero"); 5];
            for (id, raw) in ids.iter_mut().zip(raw_ids) {
                *id = AttributeId::new(raw)
                    .map_err(|_| corrupt(kind, "display sample Attribute identity is zero"))?;
            }
            let attribute_ids =
                InspectionAttributeIds::new(ids[0], ids[1], [ids[2], ids[3], ids[4]]).map_err(
                    |_| corrupt(kind, "display sample Attribute identities are not distinct"),
                )?;
            let rgb = (capabilities & 0b100 != 0).then_some(attribute_ids.rgb());
            Ok(PersistenceProfile {
                recipe: IndexRecipe::InspectionV1(attribute_ids),
                contract: Some(DisplaySampleContract::new(
                    attribute_ids.intensity(),
                    attribute_ids.classification(),
                    rgb,
                )),
            })
        }
        version => Err(IndexError::UnsupportedVersion {
            kind: if kind == "artifact" {
                "complete-index disk"
            } else {
                "incomplete-index disk"
            },
            version,
        }),
    }
}

fn push_samples(bytes: &mut Vec<u8>, samples: &[StoredSample], profile: PersistenceProfile) {
    let width = profile.sample_width();
    for sample in samples {
        assert_sample_matches_profile(*sample, profile);
        let wire = sample.wire_bytes();
        bytes.extend_from_slice(&wire[..width]);
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

    fn u16(&mut self, field: &'static str) -> Result<u16, IndexError> {
        Ok(u16::from_le_bytes(self.array(field)?))
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

struct OwnedTemporaryFile {
    file: Option<File>,
    path: Arc<Path>,
    identity: fs::Metadata,
    owned: bool,
    linked: bool,
}

#[cfg(unix)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_state(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

impl OwnedTemporaryFile {
    fn create(target: &Path, role: &str, read: bool) -> Result<Self, IndexError> {
        let mut last_path = None;
        for _ in 0..TEMPORARY_CREATE_ATTEMPTS {
            let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
            let suffix = format!(".{role}.{}.{sequence}", std::process::id());
            let path: Arc<Path> = sibling_path(target, &suffix)?.into();
            match OpenOptions::new()
                .read(read)
                .write(true)
                .create_new(true)
                .open(path.as_ref())
            {
                Ok(file) => {
                    let identity = file.metadata().map_err(|error| {
                        IndexError::io_shared("inspect created", Arc::clone(&path), error)
                    })?;
                    return Ok(Self {
                        file: Some(file),
                        path,
                        identity,
                        owned: true,
                        linked: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_path = Some(path);
                }
                Err(error) => {
                    return Err(IndexError::io_shared("create", Arc::clone(&path), error));
                }
            }
        }
        let path = last_path.expect("temporary creation attempts are nonzero");
        Err(IndexError::io_shared(
            "create",
            path,
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not reserve a unique temporary file name",
            ),
        ))
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("owned temporary file is open")
    }

    fn mark_linked(&mut self) {
        self.linked = true;
    }

    fn source_path_matches_open_file(&self) -> Result<bool, IndexError> {
        let opened = self
            .file
            .as_ref()
            .expect("owned temporary file is open")
            .metadata()
            .map_err(|error| {
                IndexError::io_shared("inspect owned temporary", Arc::clone(&self.path), error)
            })?;
        let current = match fs::symlink_metadata(self.path.as_ref()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(IndexError::io_shared(
                    "inspect owned temporary path",
                    Arc::clone(&self.path),
                    error,
                ));
            }
        };
        Ok(opened.file_type().is_file()
            && current.file_type().is_file()
            && same_file_identity(&self.identity, &opened)
            && same_file_identity(&opened, &current))
    }

    fn target_matches_open_file(&self, target: &Path) -> Result<bool, IndexError> {
        let opened = self
            .file
            .as_ref()
            .expect("owned temporary file is open")
            .metadata()
            .map_err(|error| {
                IndexError::io_shared("inspect owned temporary", Arc::clone(&self.path), error)
            })?;
        let current = match fs::symlink_metadata(target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Ok(false),
        };
        Ok(opened.file_type().is_file()
            && current.file_type().is_file()
            && same_file_identity(&self.identity, &opened)
            && same_file_identity(&opened, &current))
    }

    fn remove_owned_path(&mut self, operation: &'static str) -> Result<(), IndexError> {
        if !self.owned {
            return Ok(());
        }
        if !self.linked {
            let file = self
                .file
                .as_mut()
                .expect("owned temporary contents remain open until retirement");
            file.set_len(0)
                .and_then(|()| file.sync_all())
                .map_err(|error| IndexError::io_shared(operation, Arc::clone(&self.path), error))?;
        }
        // Retain the unique alias. For a linked publication it is another name
        // for the published inode and retains no duplicate blocks. Otherwise
        // the owned handle was truncated before close. The pathname is never
        // touched, so a racing replacement remains at its original name.
        self.owned = false;
        Ok(())
    }

    fn close_and_remove(mut self, operation: &'static str) -> Result<(), IndexError> {
        self.remove_owned_path(operation)?;
        self.file.take();
        Ok(())
    }
}

impl Drop for OwnedTemporaryFile {
    fn drop(&mut self) {
        let _ = self.remove_owned_path("remove owned temporary");
        self.file.take();
    }
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
    let mut spool = OwnedTemporaryFile::create(target, "samples", true)?;
    let spool_path = Arc::clone(&spool.path);
    let mut locations = Vec::new();
    locations
        .try_reserve_exact(plan.nodes.len())
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
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
            let retained_bytes =
                retained_finalization_runtime_bytes(work, plan, locations.capacity());
            let left_samples = read_location(
                work,
                spool.file_mut(),
                &spool_path,
                left,
                retained_bytes,
                limits,
            )?;
            let left_bytes =
                allocated_bytes(left_samples.capacity(), mem::size_of::<StoredSample>());
            let right_samples = read_location(
                work,
                spool.file_mut(),
                &spool_path,
                right,
                retained_bytes.saturating_add(left_bytes),
                limits,
            )?;
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
            append_spool_samples(work, spool.file_mut(), &spool_path, &samples, limits)?
        };
        locations[index] = Some(location);
    }
    spool
        .file_mut()
        .sync_data()
        .map_err(|error| IndexError::io_shared("flush", Arc::clone(&spool_path), error))?;

    let internal_sample_count = plan
        .nodes
        .iter()
        .filter(|node| node.leaf.is_none())
        .try_fold(0_u64, |count, node| {
            count.checked_add(node.display_point_count)
        })
        .ok_or(IndexError::ResourceLimit {
            limit: IndexLimit::ArtifactSamplePoints,
            required: u64::MAX,
            allowed: limits.max_artifact_bytes() / work.profile.sample_bytes(),
        })?;
    let node_count = u64::try_from(plan.nodes.len()).unwrap_or(u64::MAX);
    let node_table_bytes =
        node_count
            .checked_mul(NODE_RECORD_BYTES)
            .ok_or(IndexError::ResourceLimit {
                limit: IndexLimit::ArtifactBytes,
                required: u64::MAX,
                allowed: limits.max_artifact_bytes(),
            })?;
    let sample_bytes = internal_sample_count
        .checked_mul(work.profile.sample_bytes())
        .ok_or(IndexError::ResourceLimit {
            limit: IndexLimit::ArtifactBytes,
            required: u64::MAX,
            allowed: limits.max_artifact_bytes(),
        })?;
    let sample_offset = work
        .profile
        .artifact_header_bytes()
        .checked_add(node_table_bytes)
        .ok_or(IndexError::ResourceLimit {
            limit: IndexLimit::ArtifactBytes,
            required: u64::MAX,
            allowed: limits.max_artifact_bytes(),
        })?;
    let artifact_bytes = sample_offset
        .checked_add(sample_bytes)
        .and_then(|value| value.checked_add(ARTIFACT_CHECKSUM_BYTES))
        .ok_or(IndexError::ResourceLimit {
            limit: IndexLimit::ArtifactBytes,
            required: u64::MAX,
            allowed: limits.max_artifact_bytes(),
        })?;
    require(
        artifact_bytes,
        limits.max_artifact_bytes(),
        IndexLimit::ArtifactBytes,
    )?;

    let header = encode_artifact_header(
        source,
        node_count,
        plan.leaf_count,
        node_table_bytes,
        sample_offset,
        sample_bytes,
        work.profile,
    );
    let mut temporary = OwnedTemporaryFile::create(target, "tmp", false)?;
    let temporary_path = Arc::clone(&temporary.path);
    let mut artifact_hasher = Hasher::new();
    write_hashed(
        temporary.file_mut(),
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
                .checked_add(
                    node.display_point_count
                        .saturating_mul(work.profile.sample_bytes()),
                )
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
            temporary.file_mut(),
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
        let samples = read_location(
            work,
            spool.file_mut(),
            &spool_path,
            location,
            retained_finalization_runtime_bytes(work, plan, locations.capacity()),
            limits,
        )?;
        write_samples_hashed(
            temporary.file_mut(),
            &temporary_path,
            &mut artifact_hasher,
            &samples,
            work.profile,
        )?;
    }
    let checksum = artifact_hasher.finalize();
    temporary
        .file_mut()
        .write_all(checksum.as_bytes())
        .and_then(|()| temporary.file_mut().sync_all())
        .map_err(|error| {
            IndexError::io_shared("finish and flush", Arc::clone(&temporary_path), error)
        })?;
    let actual_bytes = temporary
        .file_mut()
        .metadata()
        .map_err(|error| IndexError::io_shared("inspect", Arc::clone(&temporary_path), error))?
        .len();
    if actual_bytes != artifact_bytes {
        return Err(IndexError::CorruptArtifact {
            reason: "new artifact length differs from its deterministic layout",
        });
    }
    let spool_bytes = spool
        .file_mut()
        .metadata()
        .map_err(|error| IndexError::io_shared("inspect", Arc::clone(&spool_path), error))?
        .len();
    work.observe_temporary_disk_bytes(spool_bytes, actual_bytes)?;
    control.check_cancelled()?;
    if target_exists(target)? {
        return Err(IndexError::IncompatibleArtifact {
            reason: "target appeared before atomic publication",
        });
    }
    publish_no_replace(&mut temporary, target)?;
    sync_parent(target)?;
    temporary.close_and_remove("remove published temporary")?;
    // Keep the predictable work path as a rebuildable recovery cache. There is
    // no portable conditional-unlink primitive that can atomically prove the
    // pathname still names this open file; check-then-remove could delete a
    // caller's concurrently installed replacement.
    spool.close_and_remove("remove sample spool")?;
    sync_parent(target)?;
    Ok(())
}

fn publish_no_replace(temporary: &mut OwnedTemporaryFile, target: &Path) -> Result<(), IndexError> {
    publish_no_replace_with(temporary, target, |source, destination| {
        fs::hard_link(source, destination)
    })
}

fn publish_no_replace_with(
    temporary: &mut OwnedTemporaryFile,
    target: &Path,
    hard_link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), IndexError> {
    if !temporary.source_path_matches_open_file()? {
        return Err(IndexError::CorruptArtifact {
            reason: "owned artifact temporary path changed before publication",
        });
    }
    match hard_link(&temporary.path, target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(IndexError::IncompatibleArtifact {
                reason: "target appeared before atomic publication",
            });
        }
        Err(error) => return Err(IndexError::io("atomically publish", target, error)),
    }
    let source_matches = temporary.source_path_matches_open_file()?;
    let target_matches = temporary.target_matches_open_file(target)?;
    if target_matches {
        // Avoid truncating the published target through the retained handle
        // even when a source-alias race forces a conservative failure.
        temporary.mark_linked();
    }
    if !source_matches || !target_matches {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact publication did not bind the owned temporary file",
        });
    }
    Ok(())
}

fn append_spool_samples(
    work: &mut WorkFile,
    spool: &mut File,
    spool_path: &Path,
    samples: &[StoredSample],
    limits: PrepareLimits,
) -> Result<SampleLocation, IndexError> {
    let offset = spool
        .seek(SeekFrom::End(0))
        .map_err(|error| IndexError::io("seek to end of", spool_path, error))?;
    let bytes = u64::try_from(samples.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(work.profile.sample_bytes());
    let work_bytes = work
        .file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &work.path, error))?
        .len();
    require(
        work_bytes.saturating_add(offset).saturating_add(bytes),
        limits.max_incomplete_bytes(),
        IndexLimit::IncompleteAndSampleSpoolBytes,
    )?;
    let mut hasher = Hasher::new();
    hasher.update(work.profile.sample_hash_domain());
    let width = work.profile.sample_width();
    for sample in samples {
        assert_sample_matches_profile(*sample, work.profile);
        let wire = sample.wire_bytes();
        let encoded = &wire[..width];
        hasher.update(encoded);
        spool
            .write_all(encoded)
            .map_err(|error| IndexError::io("append to", spool_path, error))?;
    }
    work.observe_temporary_disk_bytes(offset.saturating_add(bytes), 0)?;
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
    retained_bytes: u64,
    limits: PrepareLimits,
) -> Result<Vec<StoredSample>, IndexError> {
    let context = SampleReadContext::Work {
        retained_bytes,
        max_build_working_bytes: limits.max_build_working_bytes(),
    };
    match location.storage {
        SampleStorage::Work => read_persisted_samples(
            &mut work.file,
            &work.path,
            location.offset,
            location.count,
            location.checksum,
            context,
            work.profile,
        ),
        SampleStorage::Spool => read_persisted_samples(
            spool,
            spool_path,
            location.offset,
            location.count,
            location.checksum,
            context,
            work.profile,
        ),
    }
}

fn encode_artifact_header(
    source: &Source,
    node_count: u64,
    leaf_count: u64,
    node_table_bytes: u64,
    sample_offset: u64,
    sample_bytes: u64,
    profile: PersistenceProfile,
) -> Vec<u8> {
    let header_bytes = profile.artifact_header_bytes();
    let mut bytes = Vec::with_capacity(usize::try_from(header_bytes).unwrap_or(240));
    bytes.extend_from_slice(ARTIFACT_MAGIC);
    push_u32(&mut bytes, profile.recipe.disk_version());
    push_u32(&mut bytes, profile.recipe.recipe_version());
    bytes.extend_from_slice(source.identity().as_bytes());
    push_u64(&mut bytes, source.metadata().point_count());
    push_transform(&mut bytes, source.metadata().position_transform());
    push_optional_bounds(&mut bytes, source.metadata().world_bounds());
    push_u64(&mut bytes, node_count);
    push_u64(&mut bytes, leaf_count);
    push_u64(&mut bytes, header_bytes);
    push_u64(&mut bytes, node_table_bytes);
    push_u64(&mut bytes, sample_offset);
    push_u64(&mut bytes, sample_bytes);
    push_profile_extension(&mut bytes, profile);
    debug_assert_eq!(bytes.len(), usize::try_from(header_bytes).unwrap_or(240));
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
    samples: &[StoredSample],
    profile: PersistenceProfile,
) -> Result<(), IndexError> {
    let width = profile.sample_width();
    for sample in samples {
        assert_sample_matches_profile(*sample, profile);
        let wire = sample.wire_bytes();
        write_hashed(file, path, hasher, &wire[..width])?;
    }
    Ok(())
}

fn assert_sample_matches_profile(sample: StoredSample, profile: PersistenceProfile) {
    if matches!(profile.recipe, IndexRecipe::InspectionV1(_)) {
        assert!(
            sample.attributes().is_some(),
            "inspection samples carry row-aligned raw Attributes"
        );
    }
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
        IndexLimit::BuildWorkingBytes,
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
    let retained_metadata = retained_finalization_runtime_bytes(work, plan, location_capacity);
    let child_samples = MAX_NODE_SAMPLES
        .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX))
        .saturating_mul(2);
    let selection_heap = MAX_NODE_SAMPLES.saturating_mul(
        u64::try_from(mem::size_of::<(u64, u64, StoredSample)>()).unwrap_or(u64::MAX),
    );
    let merged_samples = MAX_NODE_SAMPLES
        .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX));
    retained_metadata
        .saturating_add(child_samples)
        .saturating_add(selection_heap)
        .saturating_add(merged_samples)
}

fn retained_finalization_runtime_bytes(
    work: &WorkFile,
    plan: &TreePlan,
    location_capacity: usize,
) -> u64 {
    retained_finalization_metadata_bytes(work, plan, location_capacity).saturating_add(1_024)
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
    profile: PersistenceProfile,
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
    requested_recipe: IndexRecipe,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<OpenArtifact, IndexError> {
    control.check_cancelled()?;
    reject_complete_symlink(target)?;
    let mut file = File::open(target).map_err(|error| IndexError::io("open", target, error))?;
    let initial_file_state = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", target, error))?;
    if !artifact_path_matches_open_file(&file, target, &initial_file_state)? {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact path changed while it was being opened",
        });
    }
    let artifact_bytes = initial_file_state.len();
    require(
        artifact_bytes,
        limits.max_artifact_bytes(),
        IndexLimit::ArtifactBytes,
    )?;
    let minimum_bytes = ARTIFACT_HEADER_V1_BYTES.saturating_add(ARTIFACT_CHECKSUM_BYTES);
    if artifact_bytes < minimum_bytes {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact is shorter than its fixed header and checksum",
        });
    }
    require(
        artifact_verification_buffer_bytes(artifact_bytes),
        limits.max_build_working_bytes(),
        IndexLimit::ArtifactVerificationWorkingBytes,
    )?;
    let artifact_checksum =
        verify_artifact_checksum(&mut file, target, artifact_bytes, limits, control)?;

    let mut prefix = [0_u8; 16];
    file.seek(SeekFrom::Start(0))
        .map_err(|error| IndexError::io("seek to header in", target, error))?;
    file.read_exact(&mut prefix)
        .map_err(|error| IndexError::io("read header from", target, error))?;
    let mut prefix_decoder = Decoder::artifact(&prefix);
    if prefix_decoder.array::<8>("artifact magic")? != *ARTIFACT_MAGIC {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact magic differs",
        });
    }
    let disk = prefix_decoder.u32("artifact disk version")?;
    let _recipe = prefix_decoder.u32("artifact recipe version")?;
    let header_length = match disk {
        DISK_VERSION_V1 => ARTIFACT_HEADER_V1_BYTES,
        DISK_VERSION_V2 => ARTIFACT_HEADER_V2_BYTES,
        version => {
            return Err(IndexError::UnsupportedVersion {
                kind: "complete-index disk",
                version,
            });
        }
    };
    if artifact_bytes < header_length.saturating_add(ARTIFACT_CHECKSUM_BYTES) {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact is shorter than its fixed header and checksum",
        });
    }
    let mut header_bytes = vec![0; usize::try_from(header_length).unwrap_or(240)];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header_bytes))
        .map_err(|error| IndexError::io("read header from", target, error))?;
    let header = decode_artifact_header(&header_bytes)?;
    if header.profile.recipe != requested_recipe {
        return Err(IndexError::IncompatibleArtifact {
            reason: "index recipe or inspection Attribute profile differs",
        });
    }
    let source_contract = requested_recipe.resolve_contract(source.metadata())?;
    if header.profile.contract != source_contract {
        return Err(IndexError::IncompatibleArtifact {
            reason: "Source inspection Attribute availability differs",
        });
    }
    validate_artifact_layout(&header, artifact_bytes)?;
    validate_artifact_binding(source, &header)?;
    require(
        header.node_count,
        limits.max_hierarchy_nodes(),
        IndexLimit::HierarchyNodes,
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
        IndexLimit::ResidentIndexMetadataBytes,
    )?;
    let node_capacity =
        usize::try_from(header.node_count).map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::AddressableHierarchyNodes,
            required: header.node_count,
            allowed: usize::MAX as u64,
        })?;
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::ResidentIndexMetadataBytes,
            required: metadata_bytes,
            allowed: limits.max_resident_metadata_bytes(),
        })?;
    let actual_metadata_bytes = u64::try_from(nodes.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<IndexNode>()).unwrap_or(u64::MAX));
    require(
        actual_metadata_bytes,
        limits.max_resident_metadata_bytes(),
        IndexLimit::ResidentIndexMetadataBytes,
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
        let node = decode_node_record(
            index,
            &record,
            &mut expected_sample_offset,
            header.profile.sample_bytes(),
        )?;
        nodes.push(node);
    }
    if expected_sample_offset != header.sample_offset.saturating_add(header.sample_bytes) {
        return Err(IndexError::CorruptArtifact {
            reason: "node sample ranges do not exactly cover the sample section",
        });
    }
    validate_topology(
        &nodes,
        source,
        header.leaf_count,
        header.profile.sample_bytes(),
        limits,
        control,
    )?;
    let hierarchy = IndexHierarchy::new(nodes);
    require(
        hierarchy.estimated_resident_bytes(),
        limits.max_resident_metadata_bytes(),
        IndexLimit::ResidentIndexMetadataBytes,
    )?;
    let reader = ArtifactReader {
        file: Arc::new(Mutex::new(file)),
        path: Arc::new(target.to_path_buf()),
        profile: header.profile,
        identity: Arc::new(initial_file_state.clone()),
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
        || !artifact_path_matches_open_file(
            &lock_recovering(&reader.file),
            target,
            &initial_file_state,
        )?
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
        recipe_version: header.profile.recipe.recipe_version(),
        disk_version: header.profile.recipe.disk_version(),
        recipe: header.profile.recipe,
        display_sample_contract: header.profile.contract,
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

fn artifact_path_matches_open_file(
    file: &File,
    target: &Path,
    initial: &fs::Metadata,
) -> Result<bool, IndexError> {
    let opened = file
        .metadata()
        .map_err(|error| IndexError::io("reinspect open artifact", target, error))?;
    let current = fs::symlink_metadata(target)
        .map_err(|error| IndexError::io("reinspect artifact path", target, error))?;
    Ok(initial.file_type().is_file()
        && opened.file_type().is_file()
        && current.file_type().is_file()
        && same_file_state(initial, &opened)
        && same_file_state(&opened, &current))
}

fn decode_artifact_header(bytes: &[u8]) -> Result<ArtifactHeader, IndexError> {
    let mut decoder = Decoder::artifact(bytes);
    if decoder.array::<8>("artifact magic")? != *ARTIFACT_MAGIC {
        return Err(IndexError::CorruptArtifact {
            reason: "artifact magic differs",
        });
    }
    let disk = decoder.u32("artifact disk version")?;
    let recipe = decoder.u32("artifact recipe version")?;
    let source = SourceId::new(decoder.array("artifact Source identity")?);
    let point_count = decoder.u64("artifact Source Point count")?;
    let transform = decoder.transform("artifact Source transform")?;
    let bounds = decoder.optional_bounds("artifact Source bounds")?;
    let node_count = decoder.u64("artifact node count")?;
    let leaf_count = decoder.u64("artifact leaf count")?;
    let node_table_offset = decoder.u64("artifact node table offset")?;
    let node_table_bytes = decoder.u64("artifact node table bytes")?;
    let sample_offset = decoder.u64("artifact sample offset")?;
    let sample_bytes = decoder.u64("artifact sample bytes")?;
    let profile = decode_profile_extension(&mut decoder, disk, recipe, "artifact")?;
    Ok(ArtifactHeader {
        profile,
        source,
        point_count,
        transform,
        bounds,
        node_count,
        leaf_count,
        node_table_offset,
        node_table_bytes,
        sample_offset,
        sample_bytes,
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
    if header.node_table_offset != header.profile.artifact_header_bytes()
        || header.node_table_bytes != expected_node_bytes
        || header.sample_offset
            != header
                .node_table_offset
                .saturating_add(header.node_table_bytes)
        || header.sample_offset.saturating_add(header.sample_bytes) != checksum_offset
        || !header
            .sample_bytes
            .is_multiple_of(header.profile.sample_bytes())
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
            limit: IndexLimit::ArtifactVerificationWorkingBytes,
            required: u64::try_from(buffer_length).unwrap_or(u64::MAX),
            allowed: limits.max_build_working_bytes(),
        })?;
    require(
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        limits.max_build_working_bytes(),
        IndexLimit::ArtifactVerificationWorkingBytes,
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
    sample_bytes: u64,
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
            .checked_add(display_point_count.saturating_mul(sample_bytes))
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
    sample_bytes: u64,
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
        leaf_bytes.saturating_add(MAX_NODE_SAMPLES.saturating_mul(sample_bytes)),
        limits.max_build_working_bytes(),
        IndexLimit::ArtifactValidationWorkingBytes,
    )?;
    let mut leaf_spans = Vec::new();
    leaf_spans
        .try_reserve_exact(usize::try_from(leaf_count).unwrap_or(usize::MAX))
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::ArtifactValidationWorkingBytes,
            required: leaf_bytes,
            allowed: limits.max_build_working_bytes(),
        })?;
    let actual_leaf_bytes = u64::try_from(leaf_spans.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<SourceSpan>()).unwrap_or(u64::MAX));
    require(
        actual_leaf_bytes.saturating_add(MAX_NODE_SAMPLES.saturating_mul(sample_bytes)),
        limits.max_build_working_bytes(),
        IndexLimit::ArtifactValidationWorkingBytes,
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
    let Some(root) = hierarchy.root() else {
        return Ok(());
    };
    if root.coverage_complete() {
        return Ok(());
    }
    let leaf_count = canonical_leaf_count(source.metadata().point_count());
    let maximum_depth = usize::try_from(
        u64::from(u64::BITS - leaf_count.saturating_sub(1).leading_zeros()).saturating_add(1),
    )
    .unwrap_or(usize::MAX);
    let validator = ArtifactSampleValidator {
        reader,
        nodes: hierarchy.nodes(),
        transform: source.metadata().position_transform(),
        maximum_depth,
        limits,
        control,
    };
    validator.validate_subtree(root, 1, 0)?;
    Ok(())
}

struct ArtifactSampleValidator<'validation> {
    reader: &'validation ArtifactReader,
    nodes: &'validation [IndexNode],
    transform: PositionTransform,
    maximum_depth: usize,
    limits: PrepareLimits,
    control: &'validation OperationControl,
}

impl ArtifactSampleValidator<'_> {
    fn validate_subtree(
        &self,
        node: &IndexNode,
        depth: usize,
        retained_bytes: u64,
    ) -> Result<Vec<u64>, IndexError> {
        self.control.check_cancelled()?;
        if depth > self.maximum_depth {
            return Err(IndexError::CorruptArtifact {
                reason: "hierarchy depth differs from the balanced median-split recipe",
            });
        }
        let Some([left_id, right_id]) = node.children else {
            let span = node.source_span.ok_or(IndexError::CorruptArtifact {
                reason: "leaf node has no Source span",
            })?;
            let point_count = usize::try_from(span.point_count())
                .map_err(|_| self.allocation_limit(usize::MAX, mem::size_of::<(u64, u64)>()))?;
            return self.select_bottom_k(
                span.first_ordinal()..span.end_ordinal(),
                point_count,
                usize::try_from(span.point_count().min(MAX_NODE_SAMPLES)).unwrap_or(4_096),
                retained_bytes,
            );
        };

        let left_node = resolve_child(self.nodes, node, left_id)?;
        let right_node = resolve_child(self.nodes, node, right_id)?;
        let left = self.validate_subtree(left_node, depth.saturating_add(1), retained_bytes)?;
        let left_bytes = allocated_bytes(left.capacity(), mem::size_of::<u64>());
        let right = self.validate_subtree(
            right_node,
            depth.saturating_add(1),
            retained_bytes.saturating_add(left_bytes),
        )?;
        let right_bytes = allocated_bytes(right.capacity(), mem::size_of::<u64>());
        let expected = self.select_bottom_k(
            left.iter().chain(&right).copied(),
            left.len().saturating_add(right.len()),
            usize::try_from(node.display_point_count).unwrap_or(4_096),
            retained_bytes
                .saturating_add(left_bytes)
                .saturating_add(right_bytes),
        )?;
        drop(left);
        drop(right);
        self.validate_node_samples(node, &expected, expected.capacity(), retained_bytes)?;
        Ok(expected)
    }

    fn select_bottom_k(
        &self,
        ordinals: impl Iterator<Item = u64>,
        candidate_count: usize,
        capacity: usize,
        retained_bytes: u64,
    ) -> Result<Vec<u64>, IndexError> {
        let mut selected = self.reserved_vec(candidate_count, retained_bytes)?;
        for (index, ordinal) in ordinals.enumerate() {
            if index.is_multiple_of(4_096) {
                self.control.check_cancelled()?;
            }
            selected.push((ordinal_priority(ordinal), ordinal));
        }
        if selected.len() > capacity {
            selected.select_nth_unstable(capacity);
            selected.truncate(capacity);
        }
        let selected_bytes = allocated_bytes(selected.capacity(), mem::size_of::<(u64, u64)>());
        let mut expected =
            self.reserved_vec(capacity, retained_bytes.saturating_add(selected_bytes))?;
        expected.extend(selected.into_iter().map(|(_, ordinal)| ordinal));
        expected.sort_unstable();
        Ok(expected)
    }

    fn validate_node_samples(
        &self,
        node: &IndexNode,
        expected: &[u64],
        expected_capacity: usize,
        retained_bytes: u64,
    ) -> Result<(), IndexError> {
        let expected_bytes = allocated_bytes(expected_capacity, mem::size_of::<u64>());
        let actual_bytes = node
            .display_point_count
            .saturating_mul(u64::try_from(mem::size_of::<StoredSample>()).unwrap_or(u64::MAX));
        let retained_with_expected = retained_bytes.saturating_add(expected_bytes);
        self.require_memory(retained_with_expected.saturating_add(actual_bytes))?;
        let available_sample_bytes = self
            .limits
            .max_build_working_bytes()
            .saturating_sub(retained_with_expected);
        let samples = self.reader.read_sample_block(
            node.sample_offset,
            node.display_point_count,
            node.sample_checksum,
            available_sample_bytes,
        )?;
        self.require_memory(retained_with_expected.saturating_add(allocated_bytes(
            samples.capacity(),
            mem::size_of::<StoredSample>(),
        )))?;
        if !expected
            .iter()
            .copied()
            .eq(samples.iter().map(|sample| sample.ordinal()))
        {
            return Err(IndexError::CorruptArtifact {
                reason: "internal samples differ from the stable bottom-k descendant recipe",
            });
        }
        if !samples_within_bounds(&samples, self.transform, node.bounds) {
            return Err(IndexError::CorruptArtifact {
                reason: "internal sample position is outside its node",
            });
        }
        Ok(())
    }

    fn reserved_vec<T>(&self, capacity: usize, retained_bytes: u64) -> Result<Vec<T>, IndexError> {
        self.require_memory(
            retained_bytes.saturating_add(allocated_bytes(capacity, mem::size_of::<T>())),
        )?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(capacity)
            .map_err(|_| self.allocation_limit(capacity, mem::size_of::<T>()))?;
        self.require_memory(
            retained_bytes.saturating_add(allocated_bytes(values.capacity(), mem::size_of::<T>())),
        )?;
        Ok(values)
    }

    fn require_memory(&self, required: u64) -> Result<(), IndexError> {
        require(
            required,
            self.limits.max_build_working_bytes(),
            IndexLimit::ArtifactValidationWorkingBytes,
        )
    }

    fn allocation_limit(&self, capacity: usize, item_bytes: usize) -> IndexError {
        IndexError::ResourceLimit {
            limit: IndexLimit::ArtifactValidationWorkingBytes,
            required: allocated_bytes(capacity, item_bytes),
            allowed: self.limits.max_build_working_bytes(),
        }
    }
}

fn allocated_bytes(capacity: usize, item_bytes: usize) -> u64 {
    u64::try_from(capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(item_bytes).unwrap_or(u64::MAX))
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

#[cfg(test)]
mod publication_tests {
    use std::{cell::RefCell, path::PathBuf, time::SystemTime};

    use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
    use source_memory::MemorySource;

    use super::*;

    #[test]
    fn sample_wire_encoding_uses_stack_bytes_without_per_sample_allocation() {
        let position_sample = StoredSample::position_only(7, [-3, 5, 11]);
        let position_profile = PersistenceProfile::requested(IndexRecipe::PositionOnlyV1, None)
            .expect("position-only profile is valid");
        let ids = InspectionAttributeIds::new(
            AttributeId::new(1).expect("nonzero Attribute identity"),
            AttributeId::new(2).expect("nonzero Attribute identity"),
            [
                AttributeId::new(3).expect("nonzero Attribute identity"),
                AttributeId::new(4).expect("nonzero Attribute identity"),
                AttributeId::new(5).expect("nonzero Attribute identity"),
            ],
        )
        .expect("distinct inspection Attribute identities");
        let inspection_profile = PersistenceProfile::requested(
            IndexRecipe::InspectionV1(ids),
            Some(DisplaySampleContract::new(
                ids.intensity(),
                ids.classification(),
                Some(ids.rgb()),
            )),
        )
        .expect("inspection profile is valid");
        let inspection_sample = StoredSample::attributed(
            13,
            [17, 19, 23],
            DisplayAttributes::new(65_535, 18, [1, 32_768, 65_535]),
        );
        let mut position_bytes = Vec::with_capacity(position_profile.sample_width());
        let mut inspection_bytes = Vec::with_capacity(inspection_profile.sample_width());

        let allocations = allocation_counter::measure(|| {
            push_samples(&mut position_bytes, &[position_sample], position_profile);
            push_samples(
                &mut inspection_bytes,
                &[inspection_sample],
                inspection_profile,
            );
        });

        assert_eq!(allocations.bytes_total, 0);
        assert_eq!(
            position_bytes,
            position_sample.wire_bytes()[..position_profile.sample_width()]
        );
        assert_eq!(
            inspection_bytes,
            inspection_sample.wire_bytes()[..inspection_profile.sample_width()]
        );
    }

    #[test]
    fn frame_encoding_charges_actual_capacity_with_retained_allocations() {
        let profile = PersistenceProfile::requested(IndexRecipe::PositionOnlyV1, None)
            .expect("position-only profile is valid");
        let span = SourceSpan::new(0, 1).expect("one-Point span is valid");
        let bounds = WorldBounds::new([0.0; 3], [0.0; 3]).expect("point bounds are valid");
        let samples = [StoredSample::position_only(0, [0; 3])];
        let retained = 1_024;
        let probe = encode_frame_payload(
            span,
            bounds,
            &samples,
            profile,
            retained,
            PrepareLimits::default(),
        )
        .expect("default working limit admits one frame");
        let required = retained.saturating_add(u64::try_from(probe.capacity()).unwrap_or(u64::MAX));

        let exact = encode_frame_payload(
            span,
            bounds,
            &samples,
            profile,
            retained,
            PrepareLimits::default().with_max_build_working_bytes(required),
        )
        .expect("exact actual-capacity peak is inclusive");
        assert_eq!(exact, probe);

        let error = encode_frame_payload(
            span,
            bounds,
            &samples,
            profile,
            retained,
            PrepareLimits::default().with_max_build_working_bytes(required - 1),
        )
        .expect_err("one byte below the actual-capacity peak must fail");
        assert!(matches!(
            error,
            IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required: observed,
                allowed,
            } if observed == required && allowed == required - 1
        ));
    }

    #[test]
    fn finalization_reads_and_merge_charge_all_live_allocations() {
        let retained_bytes = 1_024;
        let sample_capacity = 17;
        let sample_bytes = allocated_bytes(sample_capacity, mem::size_of::<StoredSample>());
        let exact_context = SampleReadContext::Work {
            retained_bytes,
            max_build_working_bytes: retained_bytes.saturating_add(sample_bytes),
        };
        exact_context
            .enforce_buffer_limit(sample_capacity)
            .expect("the exact retained-plus-sample peak is inclusive");
        let error = SampleReadContext::Work {
            retained_bytes,
            max_build_working_bytes: retained_bytes.saturating_add(sample_bytes) - 1,
        }
        .enforce_buffer_limit(sample_capacity)
        .expect_err("one byte below the retained-plus-sample peak must fail");
        assert!(matches!(
            error,
            IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required,
                allowed,
            } if required == retained_bytes + sample_bytes && allowed == required - 1
        ));

        let left = vec![StoredSample::position_only(0, [0; 3])];
        let right = vec![StoredSample::position_only(1, [1; 3])];
        let retained = left.len() + right.len();
        let mut heap_probe = BinaryHeap::<(u64, u64, StoredSample)>::new();
        heap_probe
            .try_reserve_exact(retained)
            .expect("tiny heap probe is addressable");
        let mut output_probe = Vec::<StoredSample>::new();
        output_probe
            .try_reserve_exact(retained)
            .expect("tiny output probe is addressable");
        let exact_peak = retained_bytes
            .saturating_add(allocated_bytes(
                left.capacity(),
                mem::size_of::<StoredSample>(),
            ))
            .saturating_add(allocated_bytes(
                right.capacity(),
                mem::size_of::<StoredSample>(),
            ))
            .saturating_add(allocated_bytes(
                heap_probe.capacity(),
                mem::size_of::<(u64, u64, StoredSample)>(),
            ))
            .saturating_add(allocated_bytes(
                output_probe.capacity(),
                mem::size_of::<StoredSample>(),
            ));
        drop((heap_probe, output_probe));

        let merged = merge_samples(
            &left,
            &right,
            [left.capacity(), right.capacity()],
            retained_bytes,
            PrepareLimits::default().with_max_build_working_bytes(exact_peak),
        )
        .expect("the exact child, heap, and output peak is inclusive");
        assert_eq!(
            merged
                .iter()
                .map(|sample| sample.ordinal())
                .collect::<Vec<_>>(),
            [0, 1]
        );

        let error = merge_samples(
            &left,
            &right,
            [left.capacity(), right.capacity()],
            retained_bytes,
            PrepareLimits::default().with_max_build_working_bytes(exact_peak - 1),
        )
        .expect_err("one byte below the full merge peak must fail before mutation");
        assert!(matches!(
            error,
            IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required,
                allowed,
            } if required == exact_peak && allowed == exact_peak - 1
        ));
    }

    #[test]
    fn publication_error_retains_operation_target_and_os_error() {
        let directory = TestDirectory::new("publication-error");
        let target = directory.path.join("fixture.pidx");
        let mut temporary =
            OwnedTemporaryFile::create(&target, "publication-error", true).expect("create stage");
        let error = publish_no_replace_with(&mut temporary, &target, |_, _| {
            Err(std::io::Error::from_raw_os_error(13))
        })
        .unwrap_err();

        let IndexError::Io {
            operation,
            path,
            source,
        } = error
        else {
            panic!("publication failure lost its filesystem error category");
        };
        assert_eq!(operation, "atomically publish");
        assert_eq!(path, target);
        assert_eq!(source.raw_os_error(), Some(13));
    }

    #[test]
    fn artifact_publication_never_acknowledges_a_replaced_temporary_source() {
        let directory = TestDirectory::new("artifact-publication-source-replacement");
        let target = directory.path.join("fixture.pidx");
        let moved = directory.path.join("owned-artifact-moved-aside");
        let sentinel = b"caller replacement published by an injected race";
        let mut temporary =
            OwnedTemporaryFile::create(&target, "artifact-race", true).expect("create stage");
        temporary
            .file_mut()
            .write_all(b"owned artifact bytes")
            .and_then(|()| temporary.file_mut().sync_all())
            .expect("write owned artifact fixture");

        let error = publish_no_replace_with(&mut temporary, &target, |source, destination| {
            fs::rename(source, &moved)?;
            fs::write(source, sentinel)?;
            fs::hard_link(source, destination)
        })
        .expect_err("a replacement source must never receive a success acknowledgement");

        assert!(matches!(error, IndexError::CorruptArtifact { .. }));
        assert_eq!(
            fs::read(&target).expect("racing target is preserved"),
            sentinel
        );
        assert_eq!(
            fs::read(&temporary.path).expect("racing source alias is preserved"),
            sentinel
        );
        drop(temporary);
        assert_eq!(
            fs::read(&target).expect("drop never mutates the racing target"),
            sentinel
        );
        assert_eq!(
            fs::metadata(&moved)
                .expect("owned inode remains as an empty diagnostic alias")
                .len(),
            0
        );
    }

    #[test]
    fn artifact_open_rejects_a_same_length_path_replacement() {
        let directory = TestDirectory::new("artifact-open-path-replacement");
        let target = directory.path.join("fixture.pidx");
        let moved = directory.path.join("opened-artifact-moved-aside");
        fs::write(&target, b"artifact A").expect("write first artifact fixture");
        let file = File::open(&target).expect("open first artifact fixture");
        let initial = file.metadata().expect("inspect first artifact fixture");
        assert!(
            artifact_path_matches_open_file(&file, &target, &initial)
                .expect("initial binding is inspectable")
        );

        fs::rename(&target, &moved).expect("move the opened artifact aside");
        fs::write(&target, b"artifact B").expect("install same-length replacement");

        assert!(
            !artifact_path_matches_open_file(&file, &target, &initial)
                .expect("replacement binding is inspectable")
        );
        assert_eq!(
            fs::read(&target).expect("replacement remains"),
            b"artifact B"
        );
    }

    #[test]
    fn every_initial_work_header_boundary_is_retryable_and_cleans_only_its_stage() {
        let boundaries = [
            (InitialWorkBoundary::WriteHeader, false),
            (InitialWorkBoundary::SyncHeader, false),
            (InitialWorkBoundary::PublishLink, false),
            (InitialWorkBoundary::SyncPublishedParent, true),
            (InitialWorkBoundary::RemoveTemporary, true),
            (InitialWorkBoundary::SyncCleanupParent, true),
        ];
        for (boundary, published) in boundaries {
            let directory = TestDirectory::new("initial-work-boundary");
            let target = directory.path.join("fixture.pidx");
            let work_path = sibling_path(&target, ".work").expect("fixture work path is valid");
            let source = empty_source();
            let profile = PersistenceProfile::requested(IndexRecipe::PositionOnlyV1, None)
                .expect("position-only profile is valid");

            let result = create_or_open_initialized_work_with(
                &source,
                &target,
                &work_path,
                profile,
                &mut |reached, _, _| {
                    if reached == boundary {
                        Err(std::io::Error::from_raw_os_error(5))
                    } else {
                        Ok(())
                    }
                },
            );
            let Err(error) = result else {
                panic!("injected initial-work boundary must fail")
            };
            let (IndexError::Io { source: error, .. }
            | IndexError::SharedPathIo { source: error, .. }) = error
            else {
                panic!("initial-work boundary failure lost its I/O category")
            };
            assert_eq!(error.raw_os_error(), Some(5));
            assert_eq!(work_path.exists(), published, "boundary: {boundary:?}");
            assert_safe_work_header_residue(&directory.path);

            let work = open_or_create_work(
                &source,
                &target,
                IndexRecipe::PositionOnlyV1,
                None,
                PrepareLimits::default(),
                true,
                &OperationControl::new(),
            )
            .expect("retry opens or publishes one complete work header");
            assert_eq!(work.durable_points(), 0);
            assert_eq!(
                fs::metadata(&work_path).expect("work header exists").len(),
                200
            );
            drop(work);
            assert_safe_work_header_residue(&directory.path);
        }
    }

    #[test]
    fn initial_work_no_replace_race_preserves_the_existing_path() {
        let directory = TestDirectory::new("initial-work-race");
        let target = directory.path.join("fixture.pidx");
        let work_path = sibling_path(&target, ".work").expect("fixture work path is valid");
        let source = empty_source();
        let profile = PersistenceProfile::requested(IndexRecipe::PositionOnlyV1, None)
            .expect("position-only profile is valid");
        let sentinel = b"racing caller-owned work path";

        let opened = create_or_open_initialized_work_with(
            &source,
            &target,
            &work_path,
            profile,
            &mut |boundary, _, work| {
                if boundary == InitialWorkBoundary::PublishLink {
                    fs::write(work, sentinel)?;
                }
                Ok(())
            },
        )
        .expect("a no-replace race opens the existing path for normal validation");
        assert!(matches!(&opened, InitialWorkOpen::Existing(_)));
        drop(opened);
        assert_eq!(fs::read(&work_path).expect("racing path remains"), sentinel);
        assert_safe_work_header_residue(&directory.path);
    }

    #[test]
    fn initial_work_never_publishes_a_replaced_temporary_source() {
        let directory = TestDirectory::new("initial-work-source-replacement");
        let target = directory.path.join("fixture.pidx");
        let work_path = sibling_path(&target, ".work").expect("fixture work path is valid");
        let moved = directory.path.join("owned-work-header-moved-aside");
        let source = empty_source();
        let profile = PersistenceProfile::requested(IndexRecipe::PositionOnlyV1, None)
            .expect("position-only profile is valid");
        let sentinel = b"caller replacement at the temporary source alias";

        let result = create_or_open_initialized_work_with(
            &source,
            &target,
            &work_path,
            profile,
            &mut |boundary, temporary, _| {
                if boundary == InitialWorkBoundary::PublishLink {
                    fs::rename(temporary, &moved)?;
                    fs::write(temporary, sentinel)?;
                }
                Ok(())
            },
        );
        let Err(error) = result else {
            panic!("a replaced temporary source must fail before publication")
        };

        assert!(matches!(error, IndexError::CorruptWork { .. }));
        assert!(!work_path.exists(), "no final work path may be published");
        assert_eq!(
            fs::read(
                directory
                    .path
                    .read_dir()
                    .expect("read fixture directory")
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .is_some_and(|name| name.to_string_lossy().contains(".work-header."))
                    })
                    .expect("replacement temporary alias remains")
            )
            .expect("read replacement temporary alias"),
            sentinel
        );
        assert_eq!(
            fs::metadata(&moved)
                .expect("owned work-header inode remains for diagnosis")
                .len(),
            0
        );
    }

    #[test]
    fn initial_work_cleanup_preserves_a_temporary_path_replacement() {
        let directory = TestDirectory::new("initial-work-replacement");
        let target = directory.path.join("fixture.pidx");
        let work_path = sibling_path(&target, ".work").expect("fixture work path is valid");
        let source = empty_source();
        let profile = PersistenceProfile::requested(IndexRecipe::PositionOnlyV1, None)
            .expect("position-only profile is valid");
        let replacement_path = RefCell::new(None::<PathBuf>);
        let sentinel = b"replacement owned by another actor";

        let opened = create_or_open_initialized_work_with(
            &source,
            &target,
            &work_path,
            profile,
            &mut |boundary, temporary, _| {
                if boundary == InitialWorkBoundary::RemoveTemporary {
                    fs::remove_file(temporary)?;
                    fs::write(temporary, sentinel)?;
                    replacement_path.replace(Some(temporary.to_path_buf()));
                }
                Ok(())
            },
        )
        .expect("replacement does not invalidate the complete final work header");
        assert!(matches!(&opened, InitialWorkOpen::InitializedLocked(_)));
        drop(opened);
        assert_eq!(
            fs::metadata(&work_path).expect("work header exists").len(),
            200
        );
        let replacement_path = replacement_path
            .into_inner()
            .expect("the cleanup hook recorded its replacement");
        assert_eq!(
            fs::read(replacement_path).expect("replacement remains untouched"),
            sentinel
        );
    }

    #[test]
    fn dropping_owned_work_never_removes_a_path_replacement() {
        let directory = TestDirectory::new("fresh-work-completion-replacement");
        let target = directory.path.join("fixture.pidx");
        let work_path = sibling_path(&target, ".work").expect("fixture work path is valid");
        let moved_work = directory.path.join("owned-work-moved-aside");
        let sentinel = b"caller replacement installed while the fresh build was running";
        let source = empty_source();
        let work = open_or_create_work(
            &source,
            &target,
            IndexRecipe::PositionOnlyV1,
            None,
            PrepareLimits::default(),
            false,
            &OperationControl::new(),
        )
        .expect("fresh preparation creates one owned work header");

        fs::rename(&work_path, &moved_work).expect("move the owned open work path aside");
        fs::write(&work_path, sentinel).expect("install a caller-owned replacement");
        drop(work);

        assert_eq!(
            fs::read(&work_path).expect("caller replacement remains"),
            sentinel
        );
        assert_eq!(
            fs::metadata(&moved_work)
                .expect("owned open work path remains available for diagnosis")
                .len(),
            200
        );
    }

    #[test]
    fn v1_append_preflights_live_stored_samples_before_validation_allocations() {
        let directory = TestDirectory::new("v1-append-live-sample-budget");
        let target = directory.path.join("fixture.pidx");
        let source = empty_source();
        let control = OperationControl::new();
        let mut work = open_or_create_work(
            &source,
            &target,
            IndexRecipe::PositionOnlyV1,
            None,
            PrepareLimits::default(),
            false,
            &control,
        )
        .expect("fresh preparation creates one owned work header");
        let sample_count = usize::try_from(MAX_NODE_SAMPLES).unwrap();
        let samples = (0..sample_count)
            .map(|ordinal| StoredSample::position_only(u64::try_from(ordinal).unwrap(), [0; 3]))
            .collect::<Vec<_>>();
        let span = SourceSpan::new(0, MAX_NODE_SAMPLES).unwrap();
        let bounds = WorldBounds::new([0.0; 3], [0.0; 3]).unwrap();
        let retained = work.retained_metadata_bytes();
        let live = allocated_bytes(samples.len(), mem::size_of::<StoredSample>());
        let heap = allocated_bytes(sample_count, mem::size_of::<(u64, u64)>());
        let ordinals = allocated_bytes(sample_count, mem::size_of::<u64>());
        let payload = FRAME_FIXED_PAYLOAD_BYTES.saturating_add(MAX_NODE_SAMPLES.saturating_mul(32));
        let required = retained
            .saturating_add(live)
            .saturating_add(heap)
            .saturating_add(ordinals)
            .saturating_add(payload)
            .saturating_add(FRAME_PREFIX_BYTES);

        let error = work
            .append_block(
                span,
                bounds,
                &samples,
                PrepareLimits::default().with_max_build_working_bytes(required - 1),
            )
            .expect_err("one byte below the live validation peak must fail before allocation");
        assert!(matches!(
            error,
            IndexError::ResourceLimit {
                limit: IndexLimit::BuildWorkingBytes,
                required: observed,
                allowed,
            } if observed == required && allowed == required - 1
        ));

        let exact_preflight = work.append_block(
            span,
            bounds,
            &samples,
            PrepareLimits::default().with_max_build_working_bytes(required),
        );
        assert!(matches!(
            exact_preflight,
            Err(IndexError::CorruptWork {
                reason: "work contains more frames than the canonical Source block count"
            })
        ));
    }

    fn empty_source() -> Source {
        let memory = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [1.0; 3]).expect("identity transform is valid"),
            CoordinateReference::Unknown,
            Vec::new(),
            AttributeColumns::empty(0),
        )
        .expect("empty fixture Source is valid");
        source_memory::open(memory)
            .blocking_wait()
            .expect("empty fixture Source opens")
    }

    fn assert_safe_work_header_residue(directory: &Path) {
        let stages = fs::read_dir(directory)
            .expect("read fixture directory")
            .map(|entry| entry.expect("read fixture entry"))
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".work-header.")
            })
            .collect::<Vec<_>>();
        assert!(
            !stages.is_empty(),
            "one retained work-header alias is expected"
        );
        for stage in stages {
            let metadata = stage.metadata().expect("inspect retained stage alias");
            assert!(metadata.is_file(), "retained stage alias must be regular");
            assert!(
                matches!(metadata.len(), 0 | 200),
                "unlinked stages are empty and linked stages retain one header"
            );
        }
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "punctra-point-index-{label}-{}-{timestamp}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create fixture directory");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).expect("remove fixture directory");
        }
    }
}
