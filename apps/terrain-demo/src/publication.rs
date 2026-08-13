use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
};

use foundation_runtime::OperationControl;
use thiserror::Error;

const HASH_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct CanonicalFileLimits {
    pub(crate) output_bytes: u64,
    pub(crate) staging_bytes: u64,
    pub(crate) write_buffer_bytes: u64,
    pub(crate) working_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalFileDisposition {
    Created,
    ReconciledExisting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalFileReceipt {
    pub(crate) disposition: CanonicalFileDisposition,
    pub(crate) content_hash: [u8; 32],
    pub(crate) byte_length: u64,
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalFileError {
    #[error("invalid canonical target: {0}")]
    Invalid(&'static str),
    #[error("canonical publication exceeded {limit}: required {required}, limit {allowed}")]
    Resource {
        limit: &'static str,
        required: u64,
        allowed: u64,
    },
    #[error("canonical target conflicts with expected bytes: {path}")]
    Conflict {
        path: PathBuf,
        expected_hash: [u8; 32],
        actual_hash: [u8; 32],
    },
    #[error("canonical target is conflicting at {path}: {reason}")]
    TargetConflict { path: PathBuf, reason: &'static str },
    #[error("canonical target changed during verification: {path}")]
    TargetChanged { path: PathBuf },
    #[error("canonical publication is indeterminate for {path}")]
    Indeterminate {
        path: PathBuf,
        expected_hash: [u8; 32],
        #[source]
        source: io::Error,
    },
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl CanonicalFileError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Io { source, .. } if source.kind() == io::ErrorKind::Interrupted
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalBoundary {
    BeforeLink,
    TargetVerification,
    ParentSync,
    StageRemoval,
    CleanupSync,
    TerminalAcknowledgement,
}

trait CanonicalPublicationHook {
    fn reach(&self, boundary: CanonicalBoundary) -> io::Result<()>;
}

#[cfg(test)]
struct ProductionCanonicalPublicationHook;

#[cfg(test)]
impl CanonicalPublicationHook for ProductionCanonicalPublicationHook {
    fn reach(&self, _boundary: CanonicalBoundary) -> io::Result<()> {
        Ok(())
    }
}

struct ControlledCanonicalPublicationHook<'a> {
    control: &'a OperationControl,
}

impl CanonicalPublicationHook for ControlledCanonicalPublicationHook<'_> {
    fn reach(&self, _boundary: CanonicalBoundary) -> io::Result<()> {
        self.control
            .check_cancelled()
            .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))
    }
}

pub(crate) enum StageCreationError {
    RandomnessUnavailable,
    NamespaceExhausted,
    Inspect { path: PathBuf, source: io::Error },
    Create { path: PathBuf, source: io::Error },
}

pub(crate) fn create_stage<E>(
    parent: &Path,
    namespace: &'static str,
    mut before_attempt: impl FnMut() -> Result<(), E>,
    mut map_error: impl FnMut(StageCreationError) -> E,
) -> Result<(StageGuard, File), E> {
    for _ in 0..64 {
        before_attempt()?;
        let mut random = [0; 16];
        getrandom::fill(&mut random)
            .map_err(|_| map_error(StageCreationError::RandomnessUnavailable))?;
        let stage = parent.join(format!(".punctra-{namespace}-{}.tmp", Hex(&random)));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&stage)
        {
            Ok(file) => {
                let metadata = file.metadata().map_err(|source| {
                    map_error(StageCreationError::Inspect {
                        path: stage.clone(),
                        source,
                    })
                })?;
                return Ok((StageGuard::new(stage, parent.to_path_buf(), metadata), file));
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(map_error(StageCreationError::Create {
                    path: stage,
                    source,
                }));
            }
        }
    }
    Err(map_error(StageCreationError::NamespaceExhausted))
}

struct Hex<'a>(&'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub(crate) struct StageGuard {
    path: Option<PathBuf>,
    parent: PathBuf,
    identity: fs::Metadata,
}

impl StageGuard {
    pub(crate) fn new(path: PathBuf, parent: PathBuf, identity: fs::Metadata) -> Self {
        Self {
            path: Some(path),
            parent,
            identity,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("a live publication stage has a path")
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(self.path())?;
        if metadata.file_type().is_file() && same_file_identity(&self.identity, &metadata) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "publication stage identity changed",
            ))
        }
    }

    pub(crate) fn remove(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        self.verify()?;
        fs::remove_file(path)?;
        self.path = None;
        Ok(())
    }

    fn discard(&mut self) {
        if self.path.is_some() && self.verify().is_ok() && self.remove().is_ok() {
            let _ = sync_directory(&self.parent);
        }
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        self.discard();
    }
}

pub(crate) struct DirectoryWitness {
    path: PathBuf,
    identity: fs::Metadata,
}

impl DirectoryWitness {
    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        let identity = fs::symlink_metadata(path)?;
        if !identity.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "publication parent is not a non-symlink directory",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            identity,
        })
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        let current = fs::symlink_metadata(&self.path)?;
        if !current.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "publication parent changed type",
            ));
        }
        #[cfg(any(unix, windows))]
        if !same_file_identity(&self.identity, &current) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "publication parent directory identity changed",
            ));
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
pub(crate) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(test)]
pub(crate) fn publish_canonical_bytes(
    target: &Path,
    bytes: &[u8],
    limits: CanonicalFileLimits,
) -> Result<CanonicalFileReceipt, CanonicalFileError> {
    publish_canonical_bytes_with_hook(target, bytes, limits, &ProductionCanonicalPublicationHook)
}

pub(crate) fn publish_canonical_bytes_controlled(
    target: &Path,
    bytes: &[u8],
    limits: CanonicalFileLimits,
    control: &OperationControl,
) -> Result<CanonicalFileReceipt, CanonicalFileError> {
    publish_canonical_bytes_with_hook(
        target,
        bytes,
        limits,
        &ControlledCanonicalPublicationHook { control },
    )
}

fn publish_canonical_bytes_with_hook(
    target: &Path,
    bytes: &[u8],
    limits: CanonicalFileLimits,
    hook: &impl CanonicalPublicationHook,
) -> Result<CanonicalFileReceipt, CanonicalFileError> {
    validate_canonical_request(target, bytes, limits)?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let parent_witness = DirectoryWitness::capture(parent)
        .map_err(|source| CanonicalFileError::io("witness canonical parent", parent, source))?;
    let (mut stage, mut stage_file) =
        create_stage(parent, "evidence", || Ok(()), map_stage_creation_error)?;
    for chunk in bytes.chunks(canonical_buffer_bytes(limits)?) {
        stage_file.write_all(chunk).map_err(|source| {
            CanonicalFileError::io("write canonical stage", stage.path(), source)
        })?;
    }
    stage_file
        .sync_all()
        .map_err(|source| CanonicalFileError::io("sync canonical stage", stage.path(), source))?;
    drop(stage_file);
    let expected = CanonicalFileFacts {
        content_hash: *blake3::hash(bytes).as_bytes(),
        byte_length: bytes.len() as u64,
    };
    let stage_metadata = fs::symlink_metadata(stage.path()).map_err(|source| {
        CanonicalFileError::io("inspect canonical stage", stage.path(), source)
    })?;
    let readback = verify_canonical_file(stage.path(), &stage_metadata, limits)?;
    if readback != expected {
        return Err(CanonicalFileError::Invalid(
            "canonical stage changed during read-back",
        ));
    }
    parent_witness
        .verify()
        .map_err(|source| CanonicalFileError::io("revalidate canonical parent", parent, source))?;
    let context = CanonicalPublicationContext {
        target,
        parent,
        parent_witness: &parent_witness,
        expected,
        limits,
    };
    match fs::symlink_metadata(target) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            hook.reach(CanonicalBoundary::BeforeLink)
                .map_err(|source| {
                    CanonicalFileError::io("canonical pre-link boundary", target, source)
                })?;
            stage.verify().map_err(|source| {
                CanonicalFileError::io("verify canonical stage", stage.path(), source)
            })?;
            parent_witness.verify().map_err(|source| {
                CanonicalFileError::io("revalidate canonical parent", parent, source)
            })?;
            match fs::hard_link(stage.path(), target) {
                Ok(()) => finish_canonical_publication(context, &mut stage, hook),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(target).map_err(|source| {
                        CanonicalFileError::io("inspect raced canonical target", target, source)
                    })?;
                    reconcile_canonical_target(context, &metadata, &mut stage, hook)
                }
                Err(source) => Err(CanonicalFileError::io(
                    "publish canonical target",
                    target,
                    source,
                )),
            }
        }
        Ok(metadata) => reconcile_canonical_target(context, &metadata, &mut stage, hook),
        Err(source) => Err(CanonicalFileError::io(
            "inspect canonical target",
            target,
            source,
        )),
    }
}

fn validate_canonical_request(
    target: &Path,
    bytes: &[u8],
    limits: CanonicalFileLimits,
) -> Result<(), CanonicalFileError> {
    if target.file_name().is_none() {
        return Err(CanonicalFileError::Invalid("target must name a file"));
    }
    let required = bytes.len() as u64;
    require_canonical(required, limits.output_bytes, "canonical output bytes")?;
    require_canonical(required, limits.staging_bytes, "canonical staging bytes")?;
    require_canonical(
        required,
        limits.working_bytes,
        "canonical retained working bytes",
    )?;
    canonical_buffer_bytes(limits).map(|_| ())
}

fn canonical_buffer_bytes(limits: CanonicalFileLimits) -> Result<usize, CanonicalFileError> {
    let bytes = HASH_BUFFER_BYTES.min(usize::try_from(limits.write_buffer_bytes).unwrap_or(0));
    if bytes == 0 {
        return Err(CanonicalFileError::Resource {
            limit: "canonical write buffer bytes",
            required: 1,
            allowed: limits.write_buffer_bytes,
        });
    }
    require_canonical(
        bytes as u64,
        limits.working_bytes,
        "canonical working bytes",
    )?;
    Ok(bytes)
}

fn map_stage_creation_error(error: StageCreationError) -> CanonicalFileError {
    match error {
        StageCreationError::RandomnessUnavailable => {
            CanonicalFileError::Invalid("system randomness is unavailable")
        }
        StageCreationError::NamespaceExhausted => {
            CanonicalFileError::Invalid("canonical staging namespace is exhausted")
        }
        StageCreationError::Inspect { path, source } => {
            CanonicalFileError::io("inspect canonical stage", &path, source)
        }
        StageCreationError::Create { path, source } => {
            CanonicalFileError::io("create canonical stage", &path, source)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CanonicalFileFacts {
    content_hash: [u8; 32],
    byte_length: u64,
}

#[derive(Clone, Copy)]
struct CanonicalPublicationContext<'a> {
    target: &'a Path,
    parent: &'a Path,
    parent_witness: &'a DirectoryWitness,
    expected: CanonicalFileFacts,
    limits: CanonicalFileLimits,
}

fn reconcile_canonical_target(
    context: CanonicalPublicationContext<'_>,
    initial_metadata: &fs::Metadata,
    stage: &mut StageGuard,
    hook: &impl CanonicalPublicationHook,
) -> Result<CanonicalFileReceipt, CanonicalFileError> {
    hook.reach(CanonicalBoundary::TargetVerification)
        .map_err(|source| {
            CanonicalFileError::io("verify canonical boundary", context.target, source)
        })?;
    context
        .parent_witness
        .verify()
        .map_err(|_| changed_canonical_target(context.target))?;
    let actual = verify_canonical_file(context.target, initial_metadata, context.limits)?;
    if actual != context.expected {
        return Err(CanonicalFileError::Conflict {
            path: context.target.to_path_buf(),
            expected_hash: context.expected.content_hash,
            actual_hash: actual.content_hash,
        });
    }
    hook.reach(CanonicalBoundary::ParentSync)
        .map_err(|source| {
            CanonicalFileError::io("sync canonical boundary", context.target, source)
        })?;
    sync_directory(context.parent).map_err(|source| {
        CanonicalFileError::io("sync canonical parent", context.parent, source)
    })?;
    hook.reach(CanonicalBoundary::StageRemoval)
        .map_err(|source| {
            CanonicalFileError::io("remove canonical boundary", context.target, source)
        })?;
    stage
        .remove()
        .map_err(|source| CanonicalFileError::io("remove canonical stage", stage.path(), source))?;
    hook.reach(CanonicalBoundary::CleanupSync)
        .map_err(|source| {
            CanonicalFileError::io("sync canonical cleanup boundary", context.target, source)
        })?;
    sync_directory(context.parent).map_err(|source| {
        CanonicalFileError::io("sync canonical cleanup", context.parent, source)
    })?;
    context
        .parent_witness
        .verify()
        .map_err(|_| changed_canonical_target(context.target))?;
    hook.reach(CanonicalBoundary::TerminalAcknowledgement)
        .map_err(|source| {
            CanonicalFileError::io("acknowledge canonical target", context.target, source)
        })?;
    Ok(canonical_receipt(
        context.expected,
        CanonicalFileDisposition::ReconciledExisting,
    ))
}

fn finish_canonical_publication(
    context: CanonicalPublicationContext<'_>,
    stage: &mut StageGuard,
    hook: &impl CanonicalPublicationHook,
) -> Result<CanonicalFileReceipt, CanonicalFileError> {
    require_canonical_boundary(hook, CanonicalBoundary::TargetVerification, context)?;
    context.parent_witness.verify().map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    let stage_metadata = fs::symlink_metadata(stage.path()).map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    let target_metadata = fs::symlink_metadata(context.target).map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    if !same_file_identity(&stage_metadata, &target_metadata) {
        return Err(canonical_indeterminate(
            context.target,
            context.expected.content_hash,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "published target identity differs",
            ),
        ));
    }
    let actual = verify_canonical_file(context.target, &target_metadata, context.limits).map_err(
        |error| {
            canonical_indeterminate(
                context.target,
                context.expected.content_hash,
                io::Error::other(error),
            )
        },
    )?;
    if actual != context.expected {
        return Err(canonical_indeterminate(
            context.target,
            context.expected.content_hash,
            io::Error::new(io::ErrorKind::InvalidData, "published target bytes differ"),
        ));
    }
    require_canonical_boundary(hook, CanonicalBoundary::ParentSync, context)?;
    sync_directory(context.parent).map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    require_canonical_boundary(hook, CanonicalBoundary::StageRemoval, context)?;
    stage.remove().map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    require_canonical_boundary(hook, CanonicalBoundary::CleanupSync, context)?;
    sync_directory(context.parent).map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    context.parent_witness.verify().map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })?;
    require_canonical_boundary(hook, CanonicalBoundary::TerminalAcknowledgement, context)?;
    Ok(canonical_receipt(
        context.expected,
        CanonicalFileDisposition::Created,
    ))
}

fn require_canonical_boundary(
    hook: &impl CanonicalPublicationHook,
    boundary: CanonicalBoundary,
    context: CanonicalPublicationContext<'_>,
) -> Result<(), CanonicalFileError> {
    hook.reach(boundary).map_err(|source| {
        canonical_indeterminate(context.target, context.expected.content_hash, source)
    })
}

fn canonical_indeterminate(
    target: &Path,
    expected_hash: [u8; 32],
    source: io::Error,
) -> CanonicalFileError {
    CanonicalFileError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash,
        source,
    }
}

fn canonical_receipt(
    facts: CanonicalFileFacts,
    disposition: CanonicalFileDisposition,
) -> CanonicalFileReceipt {
    CanonicalFileReceipt {
        disposition,
        content_hash: facts.content_hash,
        byte_length: facts.byte_length,
    }
}

fn verify_canonical_file(
    path: &Path,
    initial_metadata: &fs::Metadata,
    limits: CanonicalFileLimits,
) -> Result<CanonicalFileFacts, CanonicalFileError> {
    require_canonical_regular_file(path, initial_metadata)?;
    let mut file = File::open(path)
        .map_err(|source| CanonicalFileError::io("open canonical target", path, source))?;
    let opened = file
        .metadata()
        .map_err(|source| CanonicalFileError::io("inspect canonical target", path, source))?;
    let current = fs::symlink_metadata(path)
        .map_err(|source| CanonicalFileError::io("reinspect canonical target", path, source))?;
    require_canonical_stable(path, initial_metadata, &opened, &current)?;
    let buffer_bytes = canonical_buffer_bytes(limits)?;
    let mut buffer = vec![0; buffer_bytes];
    let mut hasher = blake3::Hasher::new();
    let mut byte_length = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| CanonicalFileError::io("hash canonical target", path, source))?;
        if read == 0 {
            break;
        }
        byte_length = byte_length.saturating_add(read as u64);
        require_canonical(byte_length, limits.output_bytes, "canonical output bytes")?;
        require_canonical(byte_length, limits.staging_bytes, "canonical staging bytes")?;
        hasher.update(&buffer[..read]);
    }
    let verified = file
        .metadata()
        .map_err(|source| CanonicalFileError::io("reinspect canonical target", path, source))?;
    let final_metadata = fs::symlink_metadata(path)
        .map_err(|source| CanonicalFileError::io("reinspect canonical target", path, source))?;
    require_canonical_stable(path, &opened, &verified, &final_metadata)?;
    if !same_file_state(&opened, &verified) || byte_length != final_metadata.len() {
        return Err(changed_canonical_target(path));
    }
    Ok(CanonicalFileFacts {
        content_hash: *hasher.finalize().as_bytes(),
        byte_length,
    })
}

fn require_canonical_regular_file(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), CanonicalFileError> {
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(CanonicalFileError::TargetConflict {
            path: path.to_path_buf(),
            reason: "target must be a regular non-symlink file",
        })
    }
}

fn require_canonical_stable(
    path: &Path,
    initial: &fs::Metadata,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> Result<(), CanonicalFileError> {
    require_canonical_regular_file(path, opened)?;
    require_canonical_regular_file(path, current)?;
    if same_file_identity(initial, opened)
        && same_file_identity(opened, current)
        && same_file_state(initial, opened)
        && same_file_state(opened, current)
    {
        Ok(())
    } else {
        Err(changed_canonical_target(path))
    }
}

pub(crate) fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && matches!(
            (left.modified(), right.modified()),
            (Ok(left_modified), Ok(right_modified)) if left_modified == right_modified
        )
}

fn changed_canonical_target(path: &Path) -> CanonicalFileError {
    CanonicalFileError::TargetChanged {
        path: path.to_path_buf(),
    }
}

fn require_canonical(
    required: u64,
    allowed: u64,
    limit: &'static str,
) -> Result<(), CanonicalFileError> {
    if required <= allowed {
        Ok(())
    } else {
        Err(CanonicalFileError::Resource {
            limit,
            required,
            allowed,
        })
    }
}

#[cfg(test)]
mod canonical_tests {
    use std::{
        fs, io,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        CanonicalBoundary, CanonicalFileDisposition, CanonicalFileError, CanonicalFileLimits,
        CanonicalPublicationHook, publish_canonical_bytes, publish_canonical_bytes_with_hook,
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn every_post_link_boundary_is_indeterminate_and_reconcilable() {
        for boundary in [
            CanonicalBoundary::TargetVerification,
            CanonicalBoundary::ParentSync,
            CanonicalBoundary::StageRemoval,
            CanonicalBoundary::CleanupSync,
            CanonicalBoundary::TerminalAcknowledgement,
        ] {
            let directory = Directory::new("post-link");
            let target = directory.path.join("evidence.json");
            let bytes = b"{\"schema\":\"generated-test\"}\n";
            let error =
                publish_canonical_bytes_with_hook(&target, bytes, limits(), &FailAt(boundary))
                    .expect_err("post-link fault cannot be acknowledged");
            assert!(
                matches!(error, CanonicalFileError::Indeterminate { .. }),
                "{error}"
            );
            assert_eq!(fs::read(&target).unwrap(), bytes);

            let receipt = publish_canonical_bytes(&target, bytes, limits())
                .expect("retry reconciles complete canonical bytes");
            assert_eq!(
                receipt.disposition,
                CanonicalFileDisposition::ReconciledExisting
            );
            assert_eq!(fs::read(&target).unwrap(), bytes);
        }
    }

    #[test]
    fn pre_link_fault_and_conflict_never_replace_caller_data() {
        let directory = Directory::new("pre-link");
        let target = directory.path.join("evidence.json");
        let bytes = b"canonical evidence\n";
        publish_canonical_bytes_with_hook(
            &target,
            bytes,
            limits(),
            &FailAt(CanonicalBoundary::BeforeLink),
        )
        .expect_err("pre-link fault fails before publication");
        assert!(!target.exists());

        fs::write(&target, b"caller-owned conflict").unwrap();
        let error = publish_canonical_bytes(&target, bytes, limits())
            .expect_err("different existing target conflicts");
        assert!(matches!(error, CanonicalFileError::Conflict { .. }));
        assert_eq!(fs::read(&target).unwrap(), b"caller-owned conflict");
    }

    fn limits() -> CanonicalFileLimits {
        CanonicalFileLimits {
            output_bytes: 1024,
            staging_bytes: 1024,
            write_buffer_bytes: 128,
            working_bytes: 2048,
        }
    }

    struct FailAt(CanonicalBoundary);

    impl CanonicalPublicationHook for FailAt {
        fn reach(&self, boundary: CanonicalBoundary) -> io::Result<()> {
            if boundary == self.0 {
                Err(io::Error::other("injected canonical publication fault"))
            } else {
                Ok(())
            }
        }
    }

    struct Directory {
        path: PathBuf,
    }

    impl Directory {
        fn new(label: &str) -> Self {
            loop {
                let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "punctra-canonical-{label}-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create canonical test directory: {error}"),
                }
            }
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
