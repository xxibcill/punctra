use std::{
    collections::BinaryHeap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    mem,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(test)]
use std::cell::Cell;

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::{PositionTransform, SourceId, WorldBounds};
use point_source::{Source, SourceSpan};

use crate::{
    DisplayCoverage, IndexDescriptor, IndexError, IndexHierarchy, IndexLimit, IndexNode,
    IndexNodeId, PrepareLimits,
    limits::require,
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
const TEMPORARY_CREATE_ATTEMPTS: usize = 128;

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    first: u64,
    second: u64,
}

impl FileIdentity {
    fn read(metadata: &fs::Metadata) -> io::Result<Self> {
        platform_file_identity(metadata).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "stable file identity is unavailable on this platform",
            )
        })
    }
}

struct StablePathFile {
    file: File,
    path: PathBuf,
    identity: FileIdentity,
}

impl StablePathFile {
    fn open(path: &Path, writable: bool) -> io::Result<Self> {
        let initial = regular_path_metadata(path)?;
        let identity = FileIdentity::read(&initial)?;
        let file = platform_open_nofollow(path, writable)?;
        let opened = file.metadata()?;
        if !opened.file_type().is_file() || FileIdentity::read(&opened)? != identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "path changed while its regular file was being opened",
            ));
        }
        let witness = Self {
            file,
            path: path.to_path_buf(),
            identity,
        };
        witness.verify_path()?;
        Ok(witness)
    }

    fn verify_path(&self) -> io::Result<()> {
        let path_metadata = regular_path_metadata(&self.path)?;
        let file_metadata = self.file.metadata()?;
        if FileIdentity::read(&path_metadata)? != self.identity
            || FileIdentity::read(&file_metadata)? != self.identity
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "path no longer names the opened regular file",
            ));
        }
        Ok(())
    }

    fn verify_exact_bytes(&self, expected: &[u8]) -> io::Result<()> {
        self.verify_path()?;
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let mut offset = 0;
        let mut actual = [0_u8; 4_096];
        while offset != expected.len() {
            let count = actual.len().min(expected.len() - offset);
            file.read_exact(&mut actual[..count])?;
            if actual[..count] != expected[offset..offset + count] {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "opened file differs from the expected complete bytes",
                ));
            }
            offset += count;
        }
        let mut trailing = [0_u8; 1];
        if file.read(&mut trailing)? != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "opened file differs from the expected complete bytes",
            ));
        }
        self.verify_path()
    }

    fn sync_all(&self) -> io::Result<()> {
        self.verify_path()?;
        self.file.sync_all()?;
        self.verify_path()
    }
}

fn regular_path_metadata(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "path is not a regular non-symlink file",
        ));
    }
    Ok(metadata)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn platform_open_nofollow(path: &Path, writable: bool) -> io::Result<File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};

    let access = if writable {
        OFlags::RDWR
    } else {
        OFlags::RDONLY
    };
    openat(
        CWD,
        path,
        access | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_open_nofollow(path: &Path, writable: bool) -> io::Result<File> {
    OpenOptions::new().read(true).write(writable).open(path)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn platform_file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(FileIdentity {
        first: metadata.dev(),
        second: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::windows::fs::MetadataExt;

    Some(FileIdentity {
        first: u64::from(metadata.volume_serial_number()?),
        second: metadata.file_index()?,
    })
}

#[cfg(not(any(unix, windows)))]
fn platform_file_identity(_metadata: &fs::Metadata) -> Option<FileIdentity> {
    None
}

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
        let mut file = lock_recovering(&self.file);
        read_persisted_samples(
            &mut file,
            self.path.as_ref(),
            offset,
            count,
            expected_checksum,
            SampleReadContext::ArtifactAfterOpen { max_buffer_bytes },
        )
    }
}

#[derive(Clone, Copy)]
enum SampleReadContext {
    ArtifactAfterOpen { max_buffer_bytes: u64 },
    Work,
}

impl SampleReadContext {
    fn byte_count(self, count: u64) -> Result<u64, IndexError> {
        match self {
            Self::ArtifactAfterOpen { .. } => {
                count
                    .checked_mul(SAMPLE_BYTES)
                    .ok_or(IndexError::CorruptArtifact {
                        reason: "sample block length overflowed",
                    })
            }
            Self::Work => Ok(count.saturating_mul(SAMPLE_BYTES)),
        }
    }

    fn capacity(self, count: u64) -> Result<usize, IndexError> {
        usize::try_from(count).map_err(|_| match self {
            Self::ArtifactAfterOpen { .. } => IndexError::ResourceLimit {
                limit: IndexLimit::AddressableSamplePoints,
                required: count,
                allowed: usize::MAX as u64,
            },
            Self::Work => corrupt("work", "sample count is not addressable"),
        })
    }

    fn enforce_buffer_limit(self, capacity: usize) -> Result<(), IndexError> {
        let Self::ArtifactAfterOpen { max_buffer_bytes } = self else {
            return Ok(());
        };
        let actual_bytes = u64::try_from(capacity)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(mem::size_of::<IndexSample>()).unwrap_or(u64::MAX));
        require(
            actual_bytes,
            max_buffer_bytes,
            IndexLimit::IndexSampleBufferBytes,
        )
    }

    fn decoder(self, bytes: &[u8]) -> Decoder<'_> {
        match self {
            Self::ArtifactAfterOpen { .. } => Decoder::artifact(bytes),
            Self::Work => Decoder::work(bytes),
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
            Self::Work => corrupt("work", "sample block checksum or order differs"),
        }
    }

    fn order_error(self) -> IndexError {
        match self {
            Self::ArtifactAfterOpen { .. } => IndexError::CorruptArtifact {
                reason: "samples are not sorted and unique",
            },
            Self::Work => corrupt("work", "sample block checksum or order differs"),
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
) -> Result<Vec<IndexSample>, IndexError> {
    let byte_count = context.byte_count(count)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(context.capacity(count)?)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::SampleBufferBytes,
            required: byte_count,
            allowed: byte_count,
        })?;
    context.enforce_buffer_limit(samples.capacity())?;

    file.seek(SeekFrom::Start(offset))
        .map_err(|error| IndexError::io("seek in", path, error))?;
    let mut hasher = Hasher::new();
    hasher.update(SAMPLE_HASH_DOMAIN);
    for _ in 0..count {
        let mut encoded = [0_u8; 32];
        file.read_exact(&mut encoded)
            .map_err(|error| context.read_error(path, error))?;
        hasher.update(&encoded);
        let mut decoder = context.decoder(&encoded);
        let ordinal = decoder.u64("sample ordinal")?;
        let ticks = [
            decoder.i64("sample x ticks")?,
            decoder.i64("sample y ticks")?,
            decoder.i64("sample z ticks")?,
        ];
        samples.push(IndexSample::new(ordinal, ticks));
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

pub(crate) struct WorkFile {
    file: File,
    path: PathBuf,
    identity: FileIdentity,
    leaves: Vec<LeafRecord>,
    durable_points: u64,
}

impl WorkFile {
    #[cfg(test)]
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

    pub(crate) fn verified_durable_points(&self) -> Result<u64, IndexError> {
        self.verify_path()?;
        Ok(self.durable_points)
    }

    fn verify_path(&self) -> Result<(), IndexError> {
        verify_work_path_identity(&self.file, &self.path, self.identity)
    }

    pub(crate) fn append_block(
        &mut self,
        span: SourceSpan,
        bounds: WorldBounds,
        samples: &[IndexSample],
        limits: PrepareLimits,
    ) -> Result<(), IndexError> {
        self.verify_path()?;
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
            IndexLimit::BuildWorkingBytes,
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
            IndexLimit::BuildWorkingBytes,
        )?;
        if self.leaves.len() == self.leaves.capacity() {
            return Err(IndexError::CorruptWork {
                reason: "work contains more frames than the canonical Source block count",
            });
        }
        let payload = encode_frame_payload(span, bounds, samples);
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
        #[cfg(test)]
        inject_live_work_replacement(self)?;
        self.verify_path()?;

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
    match open_work_path(&path) {
        Ok(file) => open_existing_work(source, target, path, file, limits, control),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match initialize_new_work(source, target, &path, limits)? {
                InitialWork::Published { file, leaves } => {
                    open_published_work(source, target, path, file, leaves, limits, control)
                }
                InitialWork::Occupied => {
                    reject_symlink(&path, "work path is a symbolic link")?;
                    let file = open_work_path(&path)
                        .map_err(|error| IndexError::io("open raced", &path, error))?;
                    open_existing_work(source, target, path, file, limits, control)
                }
            }
        }
        Err(error) => Err(IndexError::io("open", path, error)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_work_path(path: &Path) -> std::io::Result<File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};

    openat(
        CWD,
        path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_work_path(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "non-symlink work-file opening is unavailable on this platform",
    ))
}

fn open_existing_work(
    source: &Source,
    target: &Path,
    path: PathBuf,
    file: File,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    acquire_work_ownership(&file, target)?;
    let identity = bind_work_path(&file, &path)?;
    scan_work(source, path, file, identity, limits, control)
}

fn open_published_work(
    source: &Source,
    target: &Path,
    path: PathBuf,
    mut file: File,
    leaves: Vec<LeafRecord>,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    if target_exists(target)? {
        return Err(IndexError::IncompatibleArtifact {
            reason: "target appeared while its index was being built",
        });
    }
    control.check_cancelled()?;
    let identity = bind_work_path(&file, &path)?;
    let file_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &path, error))?
        .len();
    if file_bytes != WORK_HEADER_BYTES {
        drop(leaves);
        return scan_work(source, path, file, identity, limits, control);
    }

    let mut header = [0_u8; 200];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| IndexError::io("read", &path, error))?;
    validate_work_header(source, &header)?;
    verify_work_path_identity(&file, &path, identity)?;
    Ok(WorkFile {
        file,
        path,
        identity,
        leaves,
        durable_points: 0,
    })
}

fn bind_work_path(file: &File, path: &Path) -> Result<FileIdentity, IndexError> {
    let metadata = file
        .metadata()
        .map_err(|error| IndexError::io("inspect live work file at", path, error))?;
    if !metadata.file_type().is_file() {
        return Err(IndexError::io(
            "identify live work file at",
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "work descriptor is not a regular file",
            ),
        ));
    }
    let identity = FileIdentity::read(&metadata)
        .map_err(|error| IndexError::io("identify live work file at", path, error))?;
    verify_work_path_identity(file, path, identity)?;
    Ok(identity)
}

fn verify_work_path_identity(
    file: &File,
    path: &Path,
    identity: FileIdentity,
) -> Result<(), IndexError> {
    let path_metadata = regular_path_metadata(path)
        .map_err(|error| IndexError::io("verify live work path at", path, error))?;
    let file_metadata = file
        .metadata()
        .map_err(|error| IndexError::io("verify live work descriptor at", path, error))?;
    let path_identity = FileIdentity::read(&path_metadata)
        .map_err(|error| IndexError::io("identify live work path at", path, error))?;
    let file_identity = FileIdentity::read(&file_metadata)
        .map_err(|error| IndexError::io("identify live work descriptor at", path, error))?;
    if path_identity != identity || file_identity != identity {
        return Err(IndexError::io(
            "verify live work identity at",
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "work path no longer names the owned durable file",
            ),
        ));
    }
    Ok(())
}

enum InitialWork {
    Published { file: File, leaves: Vec<LeafRecord> },
    Occupied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum PublicationSemantics {
    IndependentCopy,
    AuthoritativeAlias,
}

#[cfg(target_os = "macos")]
const PLATFORM_PUBLICATION_SEMANTICS: PublicationSemantics = PublicationSemantics::IndependentCopy;

#[cfg(target_os = "linux")]
const PLATFORM_PUBLICATION_SEMANTICS: PublicationSemantics =
    PublicationSemantics::AuthoritativeAlias;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const PLATFORM_PUBLICATION_SEMANTICS: PublicationSemantics = PublicationSemantics::IndependentCopy;

struct InitialWorkStage {
    file: File,
    path: PathBuf,
}

impl InitialWorkStage {
    fn create(work_path: &Path) -> Result<Self, IndexError> {
        #[cfg(target_os = "linux")]
        {
            let (file, path) = create_unnamed_publication_file(work_path, "init")?;
            Ok(Self { file, path })
        }

        // Failed named stages are intentionally retained. No portable
        // filesystem primitive can unlink a pathname conditionally on its
        // still naming this open file, so cleanup would reintroduce a
        // replacement race. Each stage is uniquely named, ignored by prepare,
        // and bounded by WORK_HEADER_BYTES.
        #[cfg(not(target_os = "linux"))]
        {
            let mut last_path = None;
            for _ in 0..TEMPORARY_CREATE_ATTEMPTS {
                let mut nonce = [0_u8; 16];
                getrandom::fill(&mut nonce).map_err(|error| {
                    IndexError::io(
                        "choose private initial-work stage for",
                        work_path,
                        std::io::Error::other(error.to_string()),
                    )
                })?;
                let suffix = format!(".init-{:032x}", u128::from_le_bytes(nonce));
                let path = sibling_path(work_path, &suffix)?;
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(file) => return Ok(Self { file, path }),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        last_path = Some(path);
                    }
                    Err(error) => return Err(IndexError::io("create", &path, error)),
                }
            }
            let path = last_path.expect("initial-work stage attempts are nonzero");
            Err(IndexError::io(
                "create",
                &path,
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "could not reserve a private initial-work stage name",
                ),
            ))
        }
    }
}

fn initialize_new_work(
    source: &Source,
    target: &Path,
    work_path: &Path,
    limits: PrepareLimits,
) -> Result<InitialWork, IndexError> {
    let leaves = reserve_leaf_metadata(source.metadata().point_count(), limits)?;
    let mut stage = InitialWorkStage::create(work_path)?;
    acquire_work_ownership(&stage.file, target)?;
    let header = encode_work_header(source);
    write_and_sync_initial_header(&mut stage.file, &header)
        .map_err(|error| IndexError::io("write and flush", &stage.path, error))?;
    #[cfg(all(test, not(target_os = "linux")))]
    inject_initial_stage_replacement(&stage)?;
    if !publish_initial_work_no_replace(&stage, work_path)? {
        return Ok(InitialWork::Occupied);
    }
    let published = StablePathFile::open(work_path, true)
        .map_err(|error| IndexError::io("open published initial work at", work_path, error))?;
    verify_publication_identity(
        &stage.file,
        &published,
        PLATFORM_PUBLICATION_SEMANTICS,
        work_path,
    )?;
    published
        .verify_exact_bytes(&header)
        .map_err(|error| IndexError::io("verify published initial work at", work_path, error))?;
    #[cfg(target_os = "macos")]
    acquire_work_ownership(&published.file, target)?;
    sync_initial_work_target(&published)?;
    sync_initial_work_parent(work_path)?;
    #[cfg(test)]
    inject_initial_target_replacement(work_path)?;
    published.verify_exact_bytes(&header).map_err(|error| {
        IndexError::io("revalidate published initial work at", work_path, error)
    })?;
    verify_publication_identity(
        &stage.file,
        &published,
        PLATFORM_PUBLICATION_SEMANTICS,
        work_path,
    )?;

    #[cfg(target_os = "linux")]
    let file = stage.file;
    #[cfg(not(target_os = "linux"))]
    let file = published.file;
    Ok(InitialWork::Published { file, leaves })
}

fn verify_publication_identity(
    source: &File,
    target: &StablePathFile,
    semantics: PublicationSemantics,
    target_path: &Path,
) -> Result<(), IndexError> {
    target
        .verify_path()
        .map_err(|error| IndexError::io("verify published file identity at", target_path, error))?;
    let source_identity =
        FileIdentity::read(&source.metadata().map_err(|error| {
            IndexError::io("inspect publication source for", target_path, error)
        })?)
        .map_err(|error| IndexError::io("identify publication source for", target_path, error))?;
    let identity_is_valid = match semantics {
        PublicationSemantics::IndependentCopy => target.identity != source_identity,
        PublicationSemantics::AuthoritativeAlias => target.identity == source_identity,
    };
    if !identity_is_valid {
        return Err(IndexError::io(
            "verify publication identity at",
            target_path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                match semantics {
                    PublicationSemantics::IndependentCopy => {
                        "published file unexpectedly aliases its private stage"
                    }
                    PublicationSemantics::AuthoritativeAlias => {
                        "published file does not alias its authoritative stage"
                    }
                },
            ),
        ));
    }
    Ok(())
}

fn sync_initial_work_target(target: &StablePathFile) -> Result<(), IndexError> {
    #[cfg(test)]
    if matches!(initial_work_fault(), Some(InitialWorkFault::TargetSync)) {
        return Err(IndexError::io(
            "flush published initial work at",
            &target.path,
            io::Error::other("injected initial-work target-sync failure"),
        ));
    }
    target
        .sync_all()
        .map_err(|error| IndexError::io("flush published initial work at", &target.path, error))
}

fn publish_initial_work_no_replace(
    stage: &InitialWorkStage,
    work_path: &Path,
) -> Result<bool, IndexError> {
    #[cfg(test)]
    if matches!(initial_work_fault(), Some(InitialWorkFault::PublishRace)) {
        fs::write(work_path, b"racing replacement").map_err(|error| {
            IndexError::io("create injected racing work path", work_path, error)
        })?;
    }
    publish_initial_work_no_replace_with(&stage.file, work_path, platform_publish_initial_work)
}

fn publish_initial_work_no_replace_with<T: ?Sized>(
    source: &T,
    work_path: &Path,
    publish: impl FnOnce(&T, &Path) -> std::io::Result<()>,
) -> Result<bool, IndexError> {
    match publish(source, work_path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(IndexError::io(
            "atomically publish initial work header at",
            work_path,
            error,
        )),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn initial_work_parent(work_path: &Path) -> (&Path, &std::ffi::OsStr) {
    let parent = work_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = work_path
        .file_name()
        .expect("validated index targets have a file name");
    (parent, name)
}

#[cfg(target_os = "macos")]
fn platform_publish_initial_work(stage: &File, work_path: &Path) -> std::io::Result<()> {
    use rustix::fs::{CloneFlags, fclonefileat};

    let (parent, name) = initial_work_parent(work_path);
    let directory = File::open(parent)?;
    fclonefileat(stage, &directory, name, CloneFlags::empty()).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn platform_publish_initial_work(stage: &File, work_path: &Path) -> std::io::Result<()> {
    use rustix::fs::{AtFlags, linkat};
    use std::os::fd::AsRawFd;

    let (parent, name) = initial_work_parent(work_path);
    let directory = File::open(parent)?;
    let descriptor_path = format!("/proc/self/fd/{}", stage.as_raw_fd());
    linkat(
        rustix::fs::CWD,
        descriptor_path,
        &directory,
        name,
        AtFlags::SYMLINK_FOLLOW,
    )
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_publish_initial_work(_stage: &File, _work_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace initial-work publication is unavailable on this platform",
    ))
}

fn sync_initial_work_parent(work_path: &Path) -> Result<(), IndexError> {
    #[cfg(test)]
    if matches!(initial_work_fault(), Some(InitialWorkFault::ParentSync)) {
        return Err(IndexError::io(
            "flush parent directory of",
            work_path,
            std::io::Error::other("injected initial-work parent-sync failure"),
        ));
    }
    sync_parent(work_path)
}

#[cfg(target_os = "linux")]
fn create_unnamed_publication_file(
    target: &Path,
    role: &str,
) -> Result<(File, PathBuf), IndexError> {
    use rustix::fs::{Mode, OFlags, openat};

    let (parent, _) = initial_work_parent(target);
    let diagnostic_path = sibling_path(target, &format!(".{role}.unnamed"))?;
    let directory = File::open(parent)
        .map_err(|error| IndexError::io("open parent directory of", target, error))?;
    let mode = Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH;
    let file = openat(
        &directory,
        ".",
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::TMPFILE,
        mode,
    )
    .map(File::from)
    .map_err(|error| {
        IndexError::io("create unnamed publication stage for", target, error.into())
    })?;
    Ok((file, diagnostic_path))
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

#[cfg(test)]
thread_local! {
    static INITIAL_WORK_FAULT: Cell<Option<InitialWorkFault>> = const { Cell::new(None) };
}

fn write_and_sync_initial_header(file: &mut File, header: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    match initial_work_fault() {
        Some(InitialWorkFault::WriteAfter(limit)) => {
            file.write_all(&header[..limit.min(header.len())])?;
            return Err(std::io::Error::other(
                "injected initial-header write failure",
            ));
        }
        Some(InitialWorkFault::HeaderSync) => {
            file.write_all(header)?;
            return Err(std::io::Error::other(
                "injected initial-header sync failure",
            ));
        }
        _ => {}
    }
    file.write_all(header).and_then(|()| file.sync_data())
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum InitialWorkFault {
    WriteAfter(usize),
    HeaderSync,
    #[cfg(not(target_os = "linux"))]
    StageReplacement,
    PublishRace,
    TargetSync,
    ParentSync,
    InitialTargetReplacement,
    LiveWorkReplacement,
    CompletedWorkReplacement,
    #[cfg(not(target_os = "linux"))]
    ArtifactStageReplacement,
    ArtifactTargetSync,
    ArtifactParentSync,
    ArtifactTargetReplacement,
    OpenCompleteTargetReplacement,
    SampleSpoolReplacement,
}

#[cfg(test)]
fn initial_work_fault() -> Option<InitialWorkFault> {
    INITIAL_WORK_FAULT.get()
}

#[cfg(all(test, not(target_os = "linux")))]
fn inject_initial_stage_replacement(stage: &InitialWorkStage) -> Result<(), IndexError> {
    if !matches!(
        initial_work_fault(),
        Some(InitialWorkFault::StageReplacement)
    ) {
        return Ok(());
    }
    let displaced = sibling_path(&stage.path, ".displaced")?;
    fs::rename(&stage.path, &displaced)
        .map_err(|error| IndexError::io("inject replacement for", &stage.path, error))?;
    fs::write(&stage.path, b"racing private-stage replacement")
        .map_err(|error| IndexError::io("inject replacement for", &stage.path, error))
}

#[cfg(test)]
fn inject_completed_work_replacement(work: &WorkFile) -> Result<(), IndexError> {
    if !matches!(
        initial_work_fault(),
        Some(InitialWorkFault::CompletedWorkReplacement)
    ) {
        return Ok(());
    }
    let displaced = sibling_path(&work.path, ".completed-displaced")?;
    fs::rename(&work.path, &displaced)
        .map_err(|error| IndexError::io("inject replacement for", &work.path, error))?;
    fs::write(&work.path, b"racing completed-work replacement")
        .map_err(|error| IndexError::io("inject replacement for", &work.path, error))
}

#[cfg(test)]
fn inject_live_work_replacement(work: &WorkFile) -> Result<(), IndexError> {
    if !matches!(
        initial_work_fault(),
        Some(InitialWorkFault::LiveWorkReplacement)
    ) {
        return Ok(());
    }
    let displaced = sibling_path(&work.path, ".live-displaced")?;
    fs::rename(&work.path, &displaced)
        .map_err(|error| IndexError::io("inject replacement for", &work.path, error))?;
    fs::write(&work.path, b"racing live-work replacement")
        .map_err(|error| IndexError::io("inject replacement for", &work.path, error))
}

#[cfg(test)]
fn inject_initial_target_replacement(work_path: &Path) -> Result<(), IndexError> {
    if !matches!(
        initial_work_fault(),
        Some(InitialWorkFault::InitialTargetReplacement)
    ) {
        return Ok(());
    }
    inject_target_replacement(work_path, "initial-target-displaced")
}

#[cfg(test)]
fn inject_artifact_target_replacement(target: &Path) -> Result<(), IndexError> {
    if !matches!(
        initial_work_fault(),
        Some(InitialWorkFault::ArtifactTargetReplacement)
    ) {
        return Ok(());
    }
    inject_target_replacement(target, "artifact-target-displaced")
}

#[cfg(test)]
fn inject_open_complete_target_replacement(target: &Path) -> Result<(), IndexError> {
    if !matches!(
        initial_work_fault(),
        Some(InitialWorkFault::OpenCompleteTargetReplacement)
    ) {
        return Ok(());
    }
    inject_target_replacement(target, "open-target-displaced")
}

#[cfg(test)]
fn inject_target_replacement(target: &Path, role: &str) -> Result<(), IndexError> {
    let target_bytes = fs::metadata(target)
        .map_err(|error| IndexError::io("inspect injected replacement target", target, error))?
        .len();
    let displaced = sibling_path(target, &format!(".{role}"))?;
    fs::rename(target, &displaced)
        .map_err(|error| IndexError::io("inject replacement for", target, error))?;
    let mut replacement = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| IndexError::io("inject replacement for", target, error))?;
    replacement
        .write_all(b"racing published-target replacement")
        .and_then(|()| replacement.set_len(target_bytes))
        .map_err(|error| IndexError::io("inject replacement for", target, error))
}

#[cfg(test)]
fn inject_temporary_replacement(
    temporary: &OwnedTemporaryFile,
    fault: InitialWorkFault,
) -> Result<(), IndexError> {
    if initial_work_fault() != Some(fault) {
        return Ok(());
    }
    let displaced = sibling_path(&temporary.path, ".displaced")?;
    fs::rename(&temporary.path, &displaced)
        .map_err(|error| IndexError::io("inject replacement for", &temporary.path, error))?;
    fs::write(&temporary.path, b"racing temporary replacement")
        .map_err(|error| IndexError::io("inject replacement for", &temporary.path, error))
}

#[cfg(all(test, not(target_os = "linux")))]
fn inject_publication_replacement(
    temporary: &OwnedPublicationFile,
    fault: InitialWorkFault,
) -> Result<(), IndexError> {
    if initial_work_fault() != Some(fault) {
        return Ok(());
    }
    let displaced = sibling_path(&temporary.path, ".displaced")?;
    fs::rename(&temporary.path, &displaced)
        .map_err(|error| IndexError::io("inject replacement for", &temporary.path, error))?;
    fs::write(&temporary.path, b"racing temporary replacement")
        .map_err(|error| IndexError::io("inject replacement for", &temporary.path, error))
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
        IndexLimit::IncompleteIndexBytes,
    )?;
    let leaf_count = canonical_leaf_count(source.metadata().point_count());
    let leaf_bytes =
        leaf_count.saturating_mul(u64::try_from(mem::size_of::<LeafRecord>()).unwrap_or(u64::MAX));
    require(
        leaf_bytes.saturating_add(WORK_HEADER_BYTES),
        limits.max_build_working_bytes(),
        IndexLimit::BuildWorkingBytes,
    )
}

fn scan_work(
    source: &Source,
    path: PathBuf,
    mut file: File,
    identity: FileIdentity,
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<WorkFile, IndexError> {
    control.check_cancelled()?;
    verify_work_path_identity(&file, &path, identity)?;
    let file_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", &path, error))?
        .len();
    require(
        file_bytes,
        limits.max_incomplete_bytes(),
        IndexLimit::IncompleteIndexBytes,
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
        IndexLimit::BuildWorkingBytes,
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
        IndexLimit::BuildWorkingBytes,
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
    verify_work_path_identity(&file, &path, identity)?;
    Ok(WorkFile {
        file,
        path,
        identity,
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
            limit: IndexLimit::BuildWorkingBytes,
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
        IndexLimit::BuildWorkingBytes,
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
            limit: IndexLimit::BuildWorkingBytes,
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
        IndexLimit::BuildWorkingBytes,
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
            limit: IndexLimit::BuildWorkingBytes,
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
            limit: IndexLimit::BuildWorkingBytes,
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
            limit: IndexLimit::SampleBufferBytes,
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
    file: File,
    path: PathBuf,
}

impl OwnedTemporaryFile {
    fn create(target: &Path, role: &str, read: bool) -> Result<Self, IndexError> {
        let mut last_path = None;
        for _ in 0..TEMPORARY_CREATE_ATTEMPTS {
            let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
            let suffix = format!(".{role}.{}.{sequence}", std::process::id());
            let path = sibling_path(target, &suffix)?;
            match OpenOptions::new()
                .read(read)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok(Self { file, path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_path = Some(path);
                }
                Err(error) => return Err(IndexError::io("create", &path, error)),
            }
        }
        let path = last_path.expect("temporary creation attempts are nonzero");
        Err(IndexError::io(
            "create",
            &path,
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not reserve a unique temporary file name",
            ),
        ))
    }

    fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

struct OwnedPublicationFile {
    file: File,
    path: PathBuf,
}

impl OwnedPublicationFile {
    fn create(target: &Path, role: &str) -> Result<Self, IndexError> {
        #[cfg(target_os = "linux")]
        {
            let (file, path) = create_unnamed_publication_file(target, role)?;
            Ok(Self { file, path })
        }

        #[cfg(not(target_os = "linux"))]
        {
            let temporary = OwnedTemporaryFile::create(target, role, true)?;
            Ok(Self {
                file: temporary.file,
                path: temporary.path,
            })
        }
    }

    fn file(&self) -> &File {
        &self.file
    }

    fn file_mut(&mut self) -> &mut File {
        &mut self.file
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
    work.verify_path()?;
    preflight_finalization(work, plan, limits)?;
    if target_exists(target)? {
        return Err(IndexError::IncompatibleArtifact {
            reason: "target appeared while its index was being built",
        });
    }
    let mut spool = OwnedTemporaryFile::create(target, "samples", true)?;
    let spool_path = spool.path.clone();
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
            let left_samples = read_location(work, spool.file_mut(), &spool_path, left)?;
            let right_samples = read_location(work, spool.file_mut(), &spool_path, right)?;
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
            append_spool_samples(work, spool.file_mut(), &spool_path, &samples, limits)?
        };
        locations[index] = Some(location);
    }
    spool
        .file_mut()
        .sync_data()
        .map_err(|error| IndexError::io("flush", &spool_path, error))?;
    #[cfg(test)]
    inject_temporary_replacement(&spool, InitialWorkFault::SampleSpoolReplacement)?;

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
            allowed: limits.max_artifact_bytes() / SAMPLE_BYTES,
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
    let sample_bytes =
        internal_sample_count
            .checked_mul(SAMPLE_BYTES)
            .ok_or(IndexError::ResourceLimit {
                limit: IndexLimit::ArtifactBytes,
                required: u64::MAX,
                allowed: limits.max_artifact_bytes(),
            })?;
    let sample_offset =
        ARTIFACT_HEADER_BYTES
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
    );
    let mut temporary = OwnedPublicationFile::create(target, "tmp")?;
    let temporary_path = temporary.path.clone();
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
        let samples = read_location(work, spool.file_mut(), &spool_path, location)?;
        write_samples_hashed(
            temporary.file_mut(),
            &temporary_path,
            &mut artifact_hasher,
            &samples,
        )?;
    }
    let checksum = artifact_hasher.finalize();
    let expected_checksum = *checksum.as_bytes();
    temporary
        .file_mut()
        .write_all(checksum.as_bytes())
        .and_then(|()| temporary.file_mut().sync_all())
        .map_err(|error| IndexError::io("finish and flush", &temporary_path, error))?;
    let actual_bytes = temporary
        .file_mut()
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
    #[cfg(all(test, not(target_os = "linux")))]
    inject_publication_replacement(&temporary, InitialWorkFault::ArtifactStageReplacement)?;
    publish_no_replace(temporary.file(), target)?;
    let published = StablePathFile::open(target, false)
        .map_err(|error| IndexError::io("open published artifact at", target, error))?;
    verify_publication_identity(
        temporary.file(),
        &published,
        PLATFORM_PUBLICATION_SEMANTICS,
        target,
    )?;
    sync_artifact_target(&published)?;
    sync_artifact_parent(target)?;
    #[cfg(test)]
    inject_artifact_target_replacement(target)?;
    verify_publication_identity(
        temporary.file(),
        &published,
        PLATFORM_PUBLICATION_SEMANTICS,
        target,
    )?;
    verify_published_artifact(&published, artifact_bytes, expected_checksum, limits)?;
    #[cfg(test)]
    inject_completed_work_replacement(work)?;
    // The complete artifact wins on future opens, but the valid work prefix is
    // retained. There is no portable unlink operation conditional on this path
    // still naming `work.file`; removing by pathname could delete a racing
    // replacement. Explicit caller-owned index-family cleanup may remove both.
    Ok(())
}

fn sync_artifact_target(target: &StablePathFile) -> Result<(), IndexError> {
    #[cfg(test)]
    if matches!(
        initial_work_fault(),
        Some(InitialWorkFault::ArtifactTargetSync)
    ) {
        return Err(IndexError::io(
            "flush published artifact at",
            &target.path,
            io::Error::other("injected artifact target-sync failure"),
        ));
    }
    target
        .sync_all()
        .map_err(|error| IndexError::io("flush published artifact at", &target.path, error))
}

fn sync_artifact_parent(target: &Path) -> Result<(), IndexError> {
    #[cfg(test)]
    if matches!(
        initial_work_fault(),
        Some(InitialWorkFault::ArtifactParentSync)
    ) {
        return Err(IndexError::io(
            "flush parent directory of",
            target,
            io::Error::other("injected artifact parent-sync failure"),
        ));
    }
    sync_parent(target)
}

fn verify_published_artifact(
    target: &StablePathFile,
    expected_bytes: u64,
    expected_checksum: [u8; 32],
    limits: PrepareLimits,
) -> Result<(), IndexError> {
    target
        .verify_path()
        .map_err(|error| IndexError::io("revalidate published artifact at", &target.path, error))?;
    let actual_bytes = target
        .file
        .metadata()
        .map_err(|error| IndexError::io("reinspect published artifact at", &target.path, error))?
        .len();
    if actual_bytes != expected_bytes {
        return Err(IndexError::io(
            "revalidate published artifact at",
            &target.path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "published artifact length differs from its completed stage",
            ),
        ));
    }
    let mut file = target.file.try_clone().map_err(|error| {
        IndexError::io(
            "duplicate published artifact descriptor at",
            &target.path,
            error,
        )
    })?;
    let actual_checksum = verify_artifact_checksum(
        &mut file,
        &target.path,
        expected_bytes,
        limits,
        &OperationControl::new(),
    )
    .map_err(|error| published_artifact_uncertainty(&target.path, error))?;
    if actual_checksum != expected_checksum {
        return Err(IndexError::io(
            "revalidate published artifact at",
            &target.path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "published artifact checksum differs from its completed stage",
            ),
        ));
    }
    target
        .verify_path()
        .map_err(|error| IndexError::io("revalidate published artifact at", &target.path, error))
}

fn published_artifact_uncertainty(path: &Path, error: IndexError) -> IndexError {
    match error {
        IndexError::Io { source, .. } => {
            IndexError::io("revalidate published artifact at", path, source)
        }
        error => IndexError::io(
            "revalidate published artifact at",
            path,
            io::Error::other(error.to_string()),
        ),
    }
}

fn publish_no_replace(temporary: &File, target: &Path) -> Result<(), IndexError> {
    publish_no_replace_with(temporary, target, platform_publish_initial_work)
}

fn publish_no_replace_with<T: ?Sized>(
    temporary: &T,
    target: &Path,
    publish: impl FnOnce(&T, &Path) -> std::io::Result<()>,
) -> Result<(), IndexError> {
    match publish(temporary, target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(IndexError::IncompatibleArtifact {
                reason: "target appeared before atomic publication",
            });
        }
        Err(error) => return Err(IndexError::io("atomically publish", target, error)),
    }
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
        IndexLimit::IncompleteAndSampleSpoolBytes,
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
        SampleStorage::Work => read_persisted_samples(
            &mut work.file,
            &work.path,
            location.offset,
            location.count,
            location.checksum,
            SampleReadContext::Work,
        ),
        SampleStorage::Spool => read_persisted_samples(
            spool,
            spool_path,
            location.offset,
            location.count,
            location.checksum,
            SampleReadContext::Work,
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
    let witness = StablePathFile::open(target, false)
        .map_err(|error| IndexError::io("open stable complete artifact at", target, error))?;
    let mut file = witness.file.try_clone().map_err(|error| {
        IndexError::io("duplicate complete artifact descriptor at", target, error)
    })?;
    let artifact_bytes = file
        .metadata()
        .map_err(|error| IndexError::io("inspect", target, error))?
        .len();
    require(
        artifact_bytes,
        limits.max_artifact_bytes(),
        IndexLimit::ArtifactBytes,
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
        IndexLimit::ArtifactVerificationWorkingBytes,
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
        IndexLimit::ResidentIndexMetadataBytes,
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
    if final_checksum != artifact_checksum {
        return Err(IndexError::io(
            "revalidate complete artifact at",
            target,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact checksum changed while it was being opened",
            ),
        ));
    }
    #[cfg(test)]
    inject_open_complete_target_replacement(target)?;
    witness
        .verify_path()
        .map_err(|error| IndexError::io("revalidate complete artifact path at", target, error))?;
    if witness
        .file
        .metadata()
        .map_err(|error| IndexError::io("reinspect complete artifact at", target, error))?
        .len()
        != artifact_bytes
    {
        return Err(IndexError::io(
            "revalidate complete artifact at",
            target,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact length changed while it was being opened",
            ),
        ));
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
        actual_leaf_bytes.saturating_add(MAX_NODE_SAMPLES.saturating_mul(SAMPLE_BYTES)),
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
        self.validate_node_samples(node, &expected, retained_bytes)?;
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
        retained_bytes: u64,
    ) -> Result<(), IndexError> {
        let expected_bytes = allocated_bytes(expected.len(), mem::size_of::<u64>());
        let actual_bytes = node
            .display_point_count
            .saturating_mul(u64::try_from(mem::size_of::<IndexSample>()).unwrap_or(u64::MAX));
        self.require_memory(
            retained_bytes
                .saturating_add(expected_bytes)
                .saturating_add(actual_bytes),
        )?;
        let samples = self.reader.read_sample_block(
            node.sample_offset,
            node.display_point_count,
            node.sample_checksum,
            self.limits.max_build_working_bytes(),
        )?;
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
    use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
    use source_memory::MemorySource;

    use super::*;

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn partial_initial_header_stays_private_and_retry_ignores_it() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("partial-write");
        let result = with_initial_work_fault(InitialWorkFault::WriteAfter(17), || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        assert!(matches!(result, Err(IndexError::Io { .. })));
        assert!(
            !fixture.work.exists(),
            "a partial initial header must remain private, never canonical"
        );
        let stages = fixture.stage_paths();
        assert_eq!(stages.len(), 1);
        assert_eq!(fs::metadata(&stages[0]).unwrap().len(), 17);
        let retained = fs::read(&stages[0]).unwrap();

        let retry = open_or_create_work(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(retry.durable_points(), 0);
        assert_eq!(
            fs::metadata(&fixture.work).unwrap().len(),
            WORK_HEADER_BYTES
        );
        assert_eq!(fs::read(&stages[0]).unwrap(), retained);
        assert!(fixture.stage_paths().contains(&stages[0]));
        assert_eq!(fixture.stage_lengths(), vec![17, WORK_HEADER_BYTES]);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn initial_header_sync_failure_keeps_the_complete_header_private() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("header-sync");
        let result = with_initial_work_fault(InitialWorkFault::HeaderSync, || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        assert!(matches!(result, Err(IndexError::Io { .. })));
        assert!(!fixture.work.exists());
        let stages = fixture.stage_paths();
        assert_eq!(stages.len(), 1);
        assert_eq!(fs::metadata(&stages[0]).unwrap().len(), WORK_HEADER_BYTES);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn racing_private_stage_replacement_cannot_change_published_header() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("private-stage-race");
        let result = with_initial_work_fault(InitialWorkFault::StageReplacement, || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        let work = result.unwrap();
        assert_eq!(work.durable_points(), 0);
        assert_eq!(
            fs::metadata(&fixture.work).unwrap().len(),
            WORK_HEADER_BYTES
        );
        let stages = fixture.stage_paths();
        assert_eq!(stages.len(), 2);
        let replacement = stages
            .into_iter()
            .find(|path| !path.to_string_lossy().ends_with(".displaced"))
            .expect("the racing stage replacement is preserved");
        assert_eq!(
            fs::read(replacement).unwrap(),
            b"racing private-stage replacement"
        );
    }

    #[test]
    fn racing_work_replacement_is_preserved_and_never_replaced() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("publish-race");
        let result = with_initial_work_fault(InitialWorkFault::PublishRace, || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        assert!(matches!(result, Err(IndexError::CorruptWork { .. })));
        assert_eq!(fs::read(&fixture.work).unwrap(), b"racing replacement");
        #[cfg(not(target_os = "linux"))]
        {
            let stages = fixture.stage_paths();
            assert_eq!(stages.len(), 1);
            assert_eq!(fs::metadata(&stages[0]).unwrap().len(), WORK_HEADER_BYTES);
        }
        #[cfg(target_os = "linux")]
        assert!(fixture.stage_paths().is_empty());
    }

    #[test]
    fn parent_sync_uncertainty_retains_a_valid_resumable_header() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("parent-sync");
        let result = with_initial_work_fault(InitialWorkFault::ParentSync, || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        let Err(IndexError::Io {
            operation, path, ..
        }) = result
        else {
            panic!("parent-sync uncertainty must retain its filesystem category");
        };
        assert_eq!(operation, "flush parent directory of");
        assert_eq!(path, fixture.work);
        assert_eq!(
            fs::metadata(&fixture.work).unwrap().len(),
            WORK_HEADER_BYTES
        );
        #[cfg(not(target_os = "linux"))]
        assert_eq!(fixture.stage_lengths(), vec![WORK_HEADER_BYTES]);
        #[cfg(target_os = "linux")]
        assert!(fixture.stage_paths().is_empty());

        let reopened = open_or_create_work(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(reopened.durable_points(), 0);
    }

    #[test]
    fn target_sync_uncertainty_retains_a_valid_resumable_header() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("target-sync");
        let result = with_initial_work_fault(InitialWorkFault::TargetSync, || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        let Err(IndexError::Io {
            operation, path, ..
        }) = result
        else {
            panic!("target-sync uncertainty must retain its filesystem category");
        };
        assert_eq!(operation, "flush published initial work at");
        assert_eq!(path, fixture.work);
        assert_eq!(
            fs::metadata(&fixture.work).unwrap().len(),
            WORK_HEADER_BYTES
        );

        let reopened = open_or_create_work(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(reopened.durable_points(), 0);
    }

    #[test]
    fn final_initial_work_replacement_is_preserved_and_not_acknowledged() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("initial-final-window");
        let result = with_initial_work_fault(InitialWorkFault::InitialTargetReplacement, || {
            open_or_create_work(
                &source,
                &fixture.target,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });

        assert!(matches!(result, Err(IndexError::Io { .. })));
        let replacement = fs::read(&fixture.work).unwrap();
        assert_eq!(u64::try_from(replacement.len()).unwrap(), WORK_HEADER_BYTES);
        assert!(replacement.starts_with(b"racing published-target replacement"));
        let displaced = sibling_path(&fixture.work, ".initial-target-displaced").unwrap();
        assert_eq!(fs::metadata(displaced).unwrap().len(), WORK_HEADER_BYTES);
    }

    #[test]
    fn live_work_replacement_after_sync_is_not_acknowledged_as_durable() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("live-work-final-window");
        let mut work = open_or_create_work(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        let span = SourceSpan::new(0, 1).unwrap();
        let bounds = WorldBounds::new([0.001, 0.002, 0.003], [0.001, 0.002, 0.003]).unwrap();
        let samples = [IndexSample::new(0, [1, 2, 3])];

        let result = with_initial_work_fault(InitialWorkFault::LiveWorkReplacement, || {
            work.append_block(span, bounds, &samples, PrepareLimits::default())
        });

        let Err(IndexError::Io {
            operation, path, ..
        }) = result
        else {
            panic!("a replaced live work path must fail with its filesystem context");
        };
        assert_eq!(operation, "verify live work identity at");
        assert_eq!(path, fixture.work);
        assert_eq!(work.durable_points(), 0);
        assert_eq!(
            fs::read(&fixture.work).unwrap(),
            b"racing live-work replacement"
        );
        let displaced = sibling_path(&fixture.work, ".live-displaced").unwrap();
        assert!(fs::metadata(displaced).unwrap().len() > WORK_HEADER_BYTES);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_failed_initial_headers_leave_no_namespace_entry() {
        let source = fixture_source();
        let fixture = InitialWorkFixture::new("linux-failed-header");

        for fault in [
            InitialWorkFault::WriteAfter(17),
            InitialWorkFault::HeaderSync,
        ] {
            let result = with_initial_work_fault(fault, || {
                open_or_create_work(
                    &source,
                    &fixture.target,
                    PrepareLimits::default(),
                    &OperationControl::new(),
                )
            });
            assert!(matches!(result, Err(IndexError::Io { .. })));
            assert!(!fixture.work.exists());
            assert!(fixture.stage_paths().is_empty());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_publication_retains_no_named_aliases() {
        use std::os::unix::fs::MetadataExt;

        let fixture = InitialWorkFixture::new("linux-unnamed-publication");
        crate::prepare(
            fixture_source_with_points(Vec::new()),
            &fixture.target,
            PrepareLimits::default(),
        )
        .blocking_wait()
        .unwrap();

        assert_eq!(fs::metadata(&fixture.work).unwrap().nlink(), 1);
        assert_eq!(fs::metadata(&fixture.target).unwrap().nlink(), 1);
        assert!(fixture.stage_paths().is_empty());
        assert!(fixture.temporary_paths("tmp").is_empty());
    }

    #[test]
    fn completed_work_path_replacement_survives_finalization() {
        let source = fixture_source_with_points(Vec::new());
        let fixture = InitialWorkFixture::new("completed-work-race");
        let work = open_or_create_work(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        let mut work = work;
        let plan = crate::tree::plan(
            work.leaves(),
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        with_initial_work_fault(InitialWorkFault::CompletedWorkReplacement, || {
            finalize(
                &source,
                &fixture.target,
                &mut work,
                &plan,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        })
        .unwrap();

        assert!(fixture.target.exists());
        assert_eq!(
            fs::read(&fixture.work).unwrap(),
            b"racing completed-work replacement"
        );
        assert_eq!(fixture.completed_displaced_paths().len(), 1);
        let opened = open_complete(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(opened.descriptor.source_point_count(), 0);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn racing_artifact_stage_replacement_cannot_change_complete_target() {
        let (fixture, source) = finalize_empty_with_fault(
            "artifact-stage-race",
            InitialWorkFault::ArtifactStageReplacement,
        );

        assert_complete_empty(&fixture, &source);
        assert_racing_temporary_preserved(&fixture, "tmp");
    }

    #[test]
    fn racing_sample_spool_replacement_is_preserved() {
        let (fixture, source) = finalize_empty_with_fault(
            "sample-spool-race",
            InitialWorkFault::SampleSpoolReplacement,
        );

        assert_complete_empty(&fixture, &source);
        assert_racing_temporary_preserved(&fixture, "samples");
    }

    #[test]
    fn artifact_target_sync_uncertainty_retains_the_published_artifact() {
        let (fixture, source, result) = finalize_empty_result_with_fault(
            "artifact-target-sync",
            InitialWorkFault::ArtifactTargetSync,
        );

        let Err(IndexError::Io {
            operation, path, ..
        }) = result
        else {
            panic!("artifact target-sync uncertainty must remain an I/O failure");
        };
        assert_eq!(operation, "flush published artifact at");
        assert_eq!(path, fixture.target);
        assert_complete_empty(&fixture, &source);
    }

    #[test]
    fn artifact_parent_sync_uncertainty_retains_the_published_artifact() {
        let (fixture, source, result) = finalize_empty_result_with_fault(
            "artifact-parent-sync",
            InitialWorkFault::ArtifactParentSync,
        );

        let Err(IndexError::Io {
            operation, path, ..
        }) = result
        else {
            panic!("artifact parent-sync uncertainty must remain an I/O failure");
        };
        assert_eq!(operation, "flush parent directory of");
        assert_eq!(path, fixture.target);
        assert_complete_empty(&fixture, &source);
    }

    #[test]
    fn final_artifact_replacement_is_preserved_and_not_acknowledged() {
        let (fixture, source, result) = finalize_empty_result_with_fault(
            "artifact-final-window",
            InitialWorkFault::ArtifactTargetReplacement,
        );

        assert!(matches!(result, Err(IndexError::Io { .. })));
        let replacement = fs::read(&fixture.target).unwrap();
        assert!(replacement.starts_with(b"racing published-target replacement"));
        let displaced = sibling_path(&fixture.target, ".artifact-target-displaced").unwrap();
        assert_eq!(
            replacement.len() as u64,
            fs::metadata(&displaced).unwrap().len()
        );
        let opened = open_complete(
            &source,
            &displaced,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(opened.descriptor.source_point_count(), 0);
    }

    #[test]
    fn final_open_replacement_is_preserved_and_not_returned() {
        let (fixture, source) = finalize_empty_with_fault(
            "open-final-window",
            InitialWorkFault::SampleSpoolReplacement,
        );
        let result =
            with_initial_work_fault(InitialWorkFault::OpenCompleteTargetReplacement, || {
                open_complete(
                    &source,
                    &fixture.target,
                    PrepareLimits::default(),
                    &OperationControl::new(),
                )
            });

        assert!(matches!(result, Err(IndexError::Io { .. })));
        let replacement = fs::read(&fixture.target).unwrap();
        assert!(replacement.starts_with(b"racing published-target replacement"));
        let displaced = sibling_path(&fixture.target, ".open-target-displaced").unwrap();
        assert_eq!(
            replacement.len() as u64,
            fs::metadata(&displaced).unwrap().len()
        );
        let opened = open_complete(
            &source,
            &displaced,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(opened.descriptor.source_point_count(), 0);
    }

    #[test]
    fn initial_work_publication_error_retains_target_and_os_error() {
        let stage = Path::new("fixture.pidx.work.init-fixture");
        let work = Path::new("fixture.pidx.work");
        let error = publish_initial_work_no_replace_with(stage, work, |_, _| {
            Err(std::io::Error::from_raw_os_error(13))
        })
        .unwrap_err();

        let IndexError::Io {
            operation,
            path,
            source,
        } = error
        else {
            panic!("initial-work publication lost its filesystem category");
        };
        assert_eq!(operation, "atomically publish initial work header at");
        assert_eq!(path, work);
        assert_eq!(source.raw_os_error(), Some(13));
    }

    #[test]
    fn publication_error_retains_operation_target_and_os_error() {
        let temporary = Path::new("fixture.pidx.tmp.123.1");
        let target = Path::new("fixture.pidx");
        let error = publish_no_replace_with(temporary, target, |_, _| {
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

    fn fixture_source() -> Source {
        fixture_source_with_points(vec![[1, 2, 3]])
    }

    fn fixture_source_with_points(ticks: Vec<[i64; 3]>) -> Source {
        let point_count = ticks.len();
        let input = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [0.001; 3]).unwrap(),
            CoordinateReference::Unknown,
            ticks,
            AttributeColumns::empty(point_count),
        )
        .unwrap();
        source_memory::open(input).blocking_wait().unwrap()
    }

    fn with_initial_work_fault<T>(fault: InitialWorkFault, run: impl FnOnce() -> T) -> T {
        struct ResetFault(Option<InitialWorkFault>);

        impl Drop for ResetFault {
            fn drop(&mut self) {
                INITIAL_WORK_FAULT.with(|current| current.set(self.0));
            }
        }

        let previous = INITIAL_WORK_FAULT.with(|current| current.replace(Some(fault)));
        let _reset = ResetFault(previous);
        run()
    }

    fn finalize_empty_with_fault(
        label: &str,
        fault: InitialWorkFault,
    ) -> (InitialWorkFixture, Source) {
        let (fixture, source, result) = finalize_empty_result_with_fault(label, fault);
        result.unwrap();
        (fixture, source)
    }

    fn finalize_empty_result_with_fault(
        label: &str,
        fault: InitialWorkFault,
    ) -> (InitialWorkFixture, Source, Result<(), IndexError>) {
        let source = fixture_source_with_points(Vec::new());
        let fixture = InitialWorkFixture::new(label);
        let mut work = open_or_create_work(
            &source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        let plan = crate::tree::plan(
            work.leaves(),
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        let result = with_initial_work_fault(fault, || {
            finalize(
                &source,
                &fixture.target,
                &mut work,
                &plan,
                PrepareLimits::default(),
                &OperationControl::new(),
            )
        });
        (fixture, source, result)
    }

    fn assert_complete_empty(fixture: &InitialWorkFixture, source: &Source) {
        let opened = open_complete(
            source,
            &fixture.target,
            PrepareLimits::default(),
            &OperationControl::new(),
        )
        .unwrap();
        assert_eq!(opened.descriptor.source_point_count(), 0);
    }

    fn assert_racing_temporary_preserved(fixture: &InitialWorkFixture, role: &str) {
        let paths = fixture.temporary_paths(role);
        assert_eq!(paths.len(), 2);
        let replacement = paths
            .into_iter()
            .find(|path| !path.to_string_lossy().ends_with(".displaced"))
            .unwrap();
        assert_eq!(
            fs::read(replacement).unwrap(),
            b"racing temporary replacement"
        );
    }

    struct InitialWorkFixture {
        directory: PathBuf,
        target: PathBuf,
        work: PathBuf,
    }

    impl InitialWorkFixture {
        fn new(label: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "punctra-point-index-initial-work-{label}-{}-{}",
                std::process::id(),
                NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            Self {
                target: directory.join("fixture.pidx"),
                work: directory.join("fixture.pidx.work"),
                directory,
            }
        }

        fn stage_paths(&self) -> Vec<PathBuf> {
            let mut paths = fs::read_dir(&self.directory)
                .unwrap()
                .map(|entry| entry.unwrap())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("fixture.pidx.work.init-")
                })
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            paths.sort_unstable();
            paths
        }

        #[cfg(not(target_os = "linux"))]
        fn stage_lengths(&self) -> Vec<u64> {
            let mut lengths = self
                .stage_paths()
                .iter()
                .map(|path| fs::metadata(path).unwrap().len())
                .collect::<Vec<_>>();
            lengths.sort_unstable();
            lengths
        }

        fn temporary_paths(&self, role: &str) -> Vec<PathBuf> {
            let marker = format!(".{role}.");
            fs::read_dir(&self.directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .contains(&marker)
                })
                .collect()
        }

        fn completed_displaced_paths(&self) -> Vec<PathBuf> {
            fs::read_dir(&self.directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .ends_with(".completed-displaced")
                })
                .collect()
        }
    }

    impl Drop for InitialWorkFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
