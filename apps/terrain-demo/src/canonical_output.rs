//! Exact, bounded publication for caller-owned canonical output files.
#![allow(clippy::struct_field_names, clippy::too_many_lines)]

use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::{fmt, fs::OpenOptions};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use thiserror::Error;

use crate::{
    journal::Digest,
    publication::{
        DirectoryWitness, DirectoryWitnessError, StageCreationError, StageGuard,
        create_stage as create_publication_stage, same_file_identity, same_file_state,
        sync_directory,
    },
};

const HASH_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationBoundary {
    BeforeLink,
    TargetSync,
    TargetVerification,
    ParentSync,
    StageRetention,
    RetentionSync,
    TerminalAcknowledgement,
}

trait PublicationHook {
    fn reach(&self, boundary: PublicationBoundary, control: &OperationControl) -> io::Result<()>;
}

struct ProductionPublicationHook;

impl PublicationHook for ProductionPublicationHook {
    fn reach(&self, _boundary: PublicationBoundary, _control: &OperationControl) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalOutputLimits {
    pub(crate) max_output_bytes: u64,
    pub(crate) max_staging_bytes: u64,
    pub(crate) max_write_buffer_bytes: u64,
    pub(crate) max_working_bytes: u64,
}

impl Default for CanonicalOutputLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 1024 * 1024,
            max_staging_bytes: 1024 * 1024,
            max_write_buffer_bytes: 8 * 1024,
            max_working_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalOutputDisposition {
    Created,
    ReconciledExisting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalOutputReceipt {
    pub(crate) disposition: CanonicalOutputDisposition,
    pub(crate) content_hash: Digest,
    pub(crate) byte_length: u64,
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalOutputError {
    #[error("invalid canonical output request: {0}")]
    Invalid(String),
    #[error("canonical output exceeded {limit}: required {required}, limit {allowed}")]
    Resource {
        limit: String,
        required: u64,
        allowed: u64,
    },
    #[error("canonical output operation was cancelled")]
    Cancelled,
    #[error("canonical output target conflicts with expected bytes: {path}")]
    Conflict {
        path: PathBuf,
        expected_hash: Digest,
        actual_hash: Digest,
    },
    #[error("canonical output target is conflicting at {path}: {reason}")]
    TargetConflict { path: PathBuf, reason: &'static str },
    #[error("canonical output target changed during verification: {path}")]
    TargetChanged { path: PathBuf },
    #[error("canonical output publication is indeterminate for {path}: {source}")]
    Indeterminate {
        path: PathBuf,
        expected_hash: Digest,
        #[source]
        source: io::Error,
    },
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: String,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl CanonicalOutputError {
    fn io(operation: impl Into<String>, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation: operation.into(),
            path: path.to_path_buf(),
            source,
        }
    }
}

#[derive(Clone, Copy)]
struct EncodedOutputRequest<'a> {
    target: &'a Path,
    kind: CanonicalOutputSpec,
    limits: CanonicalOutputLimits,
}

#[derive(Clone, Copy)]
pub(crate) struct CanonicalOutputSpec {
    name: &'static str,
    namespace: &'static str,
    hash_domain: &'static [u8],
}

impl CanonicalOutputSpec {
    pub(crate) const fn new(
        name: &'static str,
        namespace: &'static str,
        hash_domain: &'static [u8],
    ) -> Self {
        Self {
            name,
            namespace,
            hash_domain,
        }
    }

    const fn name(self) -> &'static str {
        self.name
    }

    const fn namespace(self) -> &'static str {
        self.namespace
    }

    const fn hash_domain(self) -> &'static [u8] {
        self.hash_domain
    }

    fn operation(self, action: &str) -> String {
        format!("{action} for {}", self.name())
    }

    fn limit(self, resource: &str) -> String {
        format!("{} {resource}", self.name())
    }

    fn invalid(self, reason: &str) -> CanonicalOutputError {
        CanonicalOutputError::Invalid(format!("{} {reason}", self.name()))
    }

    fn io(self, action: &str, path: &Path, source: io::Error) -> CanonicalOutputError {
        CanonicalOutputError::io(self.operation(action), path, source)
    }
}

pub(crate) fn ensure_output(
    target: &Path,
    spec: CanonicalOutputSpec,
    limits: CanonicalOutputLimits,
    control: &OperationControl,
    encode: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    validate_inputs: impl Fn() -> io::Result<()>,
) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
    ensure_encoded_output(
        EncodedOutputRequest {
            target,
            kind: spec,
            limits,
        },
        control,
        &ProductionPublicationHook,
        |writer| encode(writer),
        &validate_inputs,
    )
}

fn ensure_encoded_output(
    request: EncodedOutputRequest<'_>,
    control: &OperationControl,
    hook: &impl PublicationHook,
    encode: impl FnOnce(&mut HashingWriter<'_>) -> io::Result<()>,
    validate_inputs: &dyn Fn() -> io::Result<()>,
) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
    let EncodedOutputRequest {
        target,
        kind,
        limits,
    } = request;
    check_cancelled(control)?;
    validate_inputs().map_err(|source| kind.io("validate inputs", target, source))?;
    validate_limits(limits, kind)?;
    if target.file_name().is_none() {
        return Err(kind.invalid("target must name a file"));
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let parent_witness = DirectoryWitness::capture(parent)
        .map_err(|source| kind.io("witness parent", parent, source))?;
    let (mut guard, stage_file) = create_stage(parent, kind, control)?;
    let mut writer = HashingWriter::new(
        stage_file,
        limits.max_output_bytes.min(limits.max_staging_bytes),
        control,
        kind.hash_domain(),
    );
    encode(&mut writer)
        .map_err(|source| map_write_error(source, &writer, limits, kind, guard.path()))?;
    let (mut stage, expected_hash, byte_length) = writer.finish(guard.path(), kind)?;
    stage
        .sync_all()
        .map_err(|source| kind.io("sync stage", guard.path(), source))?;
    let expected = FileFacts {
        hash: expected_hash,
        bytes: byte_length,
    };
    stage
        .seek(SeekFrom::Start(0))
        .map_err(|source| kind.io("seek stage for read-back", guard.path(), source))?;
    let readback = verify_open_file(&mut stage, guard.path(), limits, control, kind)?;
    guard
        .verify()
        .map_err(|source| kind.io("revalidate stage", guard.path(), source))?;
    if readback != expected {
        return Err(kind.invalid("stage changed during read-back"));
    }
    parent_witness
        .verify()
        .map_err(|source| kind.io("revalidate parent", parent, source))?;
    validate_inputs().map_err(|source| kind.io("revalidate inputs", target, source))?;
    check_cancelled(control)?;
    publish_or_reconcile(
        PublicationContext {
            target,
            parent,
            parent_witness: &parent_witness,
            expected,
            limits,
            kind,
            terminal_validation: validate_inputs,
        },
        &mut guard,
        control,
        hook,
    )
}

#[derive(Clone, Copy)]
struct PublicationContext<'a> {
    target: &'a Path,
    parent: &'a Path,
    parent_witness: &'a DirectoryWitness,
    expected: FileFacts,
    limits: CanonicalOutputLimits,
    kind: CanonicalOutputSpec,
    terminal_validation: &'a dyn Fn() -> io::Result<()>,
}

fn publish_or_reconcile(
    context: PublicationContext<'_>,
    guard: &mut StageGuard,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
    let target = context.target;
    let parent = context.parent;
    let parent_witness = context.parent_witness;
    let kind = context.kind;
    match fs::symlink_metadata(target) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            hook.reach(PublicationBoundary::BeforeLink, control)
                .map_err(|source| kind.io("run pre-link boundary", target, source))?;
            check_cancelled(control)?;
            guard
                .verify()
                .map_err(|source| kind.io("verify stage", guard.path(), source))?;
            parent_witness
                .verify()
                .map_err(|source| kind.io("revalidate parent", parent, source))?;
            match guard.publish_no_replace(target) {
                Ok(()) => {
                    parent_witness.verify().map_err(|source| {
                        CanonicalOutputError::Indeterminate {
                            path: target.to_path_buf(),
                            expected_hash: context.expected.hash,
                            source,
                        }
                    })?;
                    let mut target_witness =
                        capture_published_target(guard, target, context.limits, control, kind)
                            .and_then(|witness| {
                                if witness.facts == context.expected {
                                    Ok(witness)
                                } else {
                                    Err(kind
                                        .invalid("published bytes differ from the staged output"))
                                }
                            })
                            .map_err(|error| indeterminate(target, context.expected.hash, error))?;
                    finish_publication(context, guard, &mut target_witness, control, hook)
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(target)
                        .map_err(|source| kind.io("inspect raced target", target, source))?;
                    reconcile_existing(context, &metadata, guard, control, hook)
                }
                Err(source) => Err(kind.io("publish target", target, source)),
            }
        }
        Ok(metadata) => reconcile_existing(context, &metadata, guard, control, hook),
        Err(source) => Err(kind.io("inspect target", target, source)),
    }
}

fn reconcile_existing(
    context: PublicationContext<'_>,
    initial_metadata: &fs::Metadata,
    stage: &mut StageGuard,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
    let PublicationContext {
        target,
        parent,
        parent_witness,
        expected,
        limits,
        kind,
        terminal_validation,
    } = context;
    check_cancelled(control)?;
    let mut target_witness =
        OpenTargetWitness::capture(target, initial_metadata, limits, control, kind)?;
    if target_witness.facts != expected {
        return Err(CanonicalOutputError::Conflict {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            actual_hash: target_witness.facts.hash,
        });
    }
    hook.reach(PublicationBoundary::TargetVerification, control)
        .map_err(|source| kind.io("verify reconciled boundary", target, source))?;
    verify_reconciled_parent(parent_witness, parent, target, kind)?;
    target_witness.verify(target, limits, control, kind)?;
    check_cancelled(control)?;
    verify_reconciled_parent(parent_witness, parent, target, kind)?;
    hook.reach(PublicationBoundary::ParentSync, control)
        .map_err(|source| kind.io("sync reconciled boundary", target, source))?;
    sync_directory(parent).map_err(|source| kind.io("sync reconciled parent", parent, source))?;
    target_witness.verify(target, limits, control, kind)?;
    check_cancelled(control)?;
    hook.reach(PublicationBoundary::StageRetention, control)
        .map_err(|source| kind.io("retain reconciled stage", target, source))?;
    stage.retain_private_stage();
    hook.reach(PublicationBoundary::RetentionSync, control)
        .map_err(|source| kind.io("sync retained stage", target, source))?;
    sync_directory(parent)
        .map_err(|source| kind.io("sync reconciled stage retention", parent, source))?;
    verify_reconciled_parent(parent_witness, parent, target, kind)?;
    target_witness.verify(target, limits, control, kind)?;
    hook.reach(PublicationBoundary::TerminalAcknowledgement, control)
        .map_err(|source| kind.io("acknowledge reconciled target", target, source))?;
    check_cancelled(control)?;
    target_witness.verify(target, limits, control, kind)?;
    terminal_validation()
        .map_err(|source| kind.io("perform terminal input validation", target, source))?;
    target_witness.verify(target, limits, control, kind)?;
    Ok(CanonicalOutputReceipt {
        disposition: CanonicalOutputDisposition::ReconciledExisting,
        content_hash: expected.hash,
        byte_length: expected.bytes,
    })
}

fn finish_publication(
    context: PublicationContext<'_>,
    stage: &mut StageGuard,
    target_witness: &mut OpenTargetWitness,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
    let PublicationContext {
        target,
        parent,
        parent_witness,
        expected,
        limits,
        kind,
        terminal_validation,
    } = context;
    // A complete target may be observable once the hard link succeeds. Every
    // subsequent failure is therefore indeterminate, including cancellation.
    require_post_link_boundary(
        hook,
        PublicationBoundary::TargetSync,
        control,
        target,
        expected.hash,
    )?;
    target_witness
        .sync(kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    target_witness
        .verify(target, limits, control, kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::TargetVerification,
        control,
        target,
        expected.hash,
    )?;
    parent_witness
        .verify()
        .map_err(|source| CanonicalOutputError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            source,
        })?;
    target_witness
        .verify(target, limits, control, kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::ParentSync,
        control,
        target,
        expected.hash,
    )?;
    sync_directory(parent).map_err(|source| CanonicalOutputError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash: expected.hash,
        source,
    })?;
    target_witness
        .verify(target, limits, control, kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::StageRetention,
        control,
        target,
        expected.hash,
    )?;
    stage.retain_private_stage();
    require_post_link_boundary(
        hook,
        PublicationBoundary::RetentionSync,
        control,
        target,
        expected.hash,
    )?;
    sync_directory(parent).map_err(|source| CanonicalOutputError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash: expected.hash,
        source,
    })?;
    parent_witness
        .verify()
        .map_err(|source| CanonicalOutputError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            source,
        })?;
    target_witness
        .verify(target, limits, control, kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::TerminalAcknowledgement,
        control,
        target,
        expected.hash,
    )?;
    target_witness
        .verify(target, limits, control, kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    terminal_validation().map_err(|source| CanonicalOutputError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash: expected.hash,
        source,
    })?;
    target_witness
        .verify(target, limits, control, kind)
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    Ok(CanonicalOutputReceipt {
        disposition: CanonicalOutputDisposition::Created,
        content_hash: expected.hash,
        byte_length: expected.bytes,
    })
}

fn require_post_link_boundary(
    hook: &impl PublicationHook,
    boundary: PublicationBoundary,
    control: &OperationControl,
    target: &Path,
    expected_hash: Digest,
) -> Result<(), CanonicalOutputError> {
    hook.reach(boundary, control)
        .map_err(|source| CanonicalOutputError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash,
            source,
        })?;
    check_cancelled(control).map_err(|error| indeterminate(target, expected_hash, error))
}

fn indeterminate(
    target: &Path,
    expected_hash: Digest,
    error: CanonicalOutputError,
) -> CanonicalOutputError {
    CanonicalOutputError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash,
        source: io::Error::other(error),
    }
}

struct HashingWriter<'a> {
    file: File,
    hasher: Hasher,
    bytes: u64,
    max_bytes: u64,
    required: u64,
    control: &'a OperationControl,
}

impl<'a> HashingWriter<'a> {
    fn new(file: File, max_bytes: u64, control: &'a OperationControl, hash_domain: &[u8]) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(hash_domain);
        Self {
            file,
            hasher,
            bytes: 0,
            max_bytes,
            required: 0,
            control,
        }
    }

    fn finish(
        mut self,
        path: &Path,
        kind: CanonicalOutputSpec,
    ) -> Result<(File, Digest, u64), CanonicalOutputError> {
        self.file
            .flush()
            .map_err(|source| kind.io("flush stage", path, source))?;
        Ok((self.file, *self.hasher.finalize().as_bytes(), self.bytes))
    }
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.control
            .check_cancelled()
            .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
        let requested = self
            .bytes
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        self.required = self.required.max(requested);
        if requested > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical output byte limit",
            ));
        }
        let written = self.file.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn map_write_error(
    source: io::Error,
    writer: &HashingWriter<'_>,
    limits: CanonicalOutputLimits,
    kind: CanonicalOutputSpec,
    stage_path: &Path,
) -> CanonicalOutputError {
    if source.kind() == io::ErrorKind::FileTooLarge {
        let (limit, allowed) = if writer.required > limits.max_output_bytes {
            (kind.limit("output bytes"), limits.max_output_bytes)
        } else {
            (kind.limit("staging bytes"), limits.max_staging_bytes)
        };
        CanonicalOutputError::Resource {
            limit,
            required: writer.required,
            allowed,
        }
    } else if source.kind() == io::ErrorKind::Interrupted {
        CanonicalOutputError::Cancelled
    } else {
        kind.io("encode stage", stage_path, source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileFacts {
    hash: Digest,
    bytes: u64,
}

struct OpenTargetWitness {
    file: File,
    identity: fs::Metadata,
    facts: FileFacts,
}

impl OpenTargetWitness {
    fn capture(
        path: &Path,
        initial_metadata: &fs::Metadata,
        limits: CanonicalOutputLimits,
        control: &OperationControl,
        kind: CanonicalOutputSpec,
    ) -> Result<Self, CanonicalOutputError> {
        require_regular_target(path, initial_metadata)?;
        let file =
            File::open(path).map_err(|source| kind.io("open target witness", path, source))?;
        let identity = file
            .metadata()
            .map_err(|source| kind.io("inspect target witness", path, source))?;
        let current = fs::symlink_metadata(path)
            .map_err(|source| kind.io("reinspect target", path, source))?;
        require_stable_target(path, initial_metadata, &identity, &current)?;
        let mut witness = Self {
            file,
            identity,
            facts: FileFacts {
                hash: [0; 32],
                bytes: 0,
            },
        };
        witness
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|source| kind.io("seek target witness", path, source))?;
        witness.facts = verify_open_file(&mut witness.file, path, limits, control, kind)?;
        witness.verify(path, limits, control, kind)?;
        Ok(witness)
    }

    fn verify(
        &mut self,
        path: &Path,
        limits: CanonicalOutputLimits,
        control: &OperationControl,
        kind: CanonicalOutputSpec,
    ) -> Result<(), CanonicalOutputError> {
        let opened_before = self
            .file
            .metadata()
            .map_err(|source| kind.io("inspect open target", path, source))?;
        let path_before = fs::symlink_metadata(path)
            .map_err(|source| kind.io("inspect target path", path, source))?;
        require_stable_target(path, &self.identity, &opened_before, &path_before)?;
        if !same_file_state(&self.identity, &opened_before) {
            return Err(changed_target_error(path));
        }
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|source| kind.io("seek open target", path, source))?;
        let facts = verify_open_file(&mut self.file, path, limits, control, kind)?;
        let opened_after = self
            .file
            .metadata()
            .map_err(|source| kind.io("reinspect open target", path, source))?;
        let path_after = fs::symlink_metadata(path)
            .map_err(|source| kind.io("reinspect target path", path, source))?;
        require_stable_target(path, &self.identity, &opened_after, &path_after)?;
        if !same_file_state(&self.identity, &opened_after) || facts != self.facts {
            return Err(changed_target_error(path));
        }
        Ok(())
    }

    fn sync(&self, kind: CanonicalOutputSpec) -> Result<(), CanonicalOutputError> {
        self.file.sync_all().map_err(|source| {
            kind.io(
                "sync created target",
                Path::new("publication target"),
                source,
            )
        })
    }
}

#[cfg(test)]
fn verify_existing_regular_file(
    path: &Path,
    initial_metadata: &fs::Metadata,
    limits: CanonicalOutputLimits,
    control: &OperationControl,
    kind: CanonicalOutputSpec,
) -> Result<FileFacts, CanonicalOutputError> {
    Ok(OpenTargetWitness::capture(path, initial_metadata, limits, control, kind)?.facts)
}

fn capture_published_target(
    stage: &StageGuard,
    target: &Path,
    limits: CanonicalOutputLimits,
    control: &OperationControl,
    kind: CanonicalOutputSpec,
) -> Result<OpenTargetWitness, CanonicalOutputError> {
    stage
        .verify()
        .map_err(|source| kind.io("inspect publication stage", stage.path(), source))?;
    let stage_before = stage
        .source_metadata()
        .map_err(|source| kind.io("inspect publication stage", stage.path(), source))?;
    let target_before = fs::symlink_metadata(target)
        .map_err(|source| kind.io("inspect linked target", target, source))?;
    require_regular_target(stage.path(), &stage_before)?;
    require_regular_target(target, &target_before)?;
    if stage.has_named_stage() && same_file_identity(&stage_before, &target_before) {
        return Err(changed_target_error(target));
    }
    let witness = OpenTargetWitness::capture(target, &target_before, limits, control, kind)?;
    stage
        .verify()
        .map_err(|source| kind.io("reinspect publication stage", stage.path(), source))?;
    let stage_after = stage
        .source_metadata()
        .map_err(|source| kind.io("reinspect publication stage", stage.path(), source))?;
    let target_after = fs::symlink_metadata(target)
        .map_err(|source| kind.io("reinspect linked target", target, source))?;
    require_stable_target(target, &target_before, &target_before, &target_after)?;
    if !same_file_identity(&stage_before, &stage_after)
        || !same_file_state(&stage_before, &stage_after)
        || stage_after.len() != witness.facts.bytes
    {
        return Err(changed_target_error(target));
    }
    Ok(witness)
}

fn verify_open_file(
    file: &mut File,
    path: &Path,
    limits: CanonicalOutputLimits,
    control: &OperationControl,
    kind: CanonicalOutputSpec,
) -> Result<FileFacts, CanonicalOutputError> {
    let buffer_bytes =
        HASH_BUFFER_BYTES.min(usize::try_from(limits.max_write_buffer_bytes).unwrap_or(0));
    if buffer_bytes == 0 {
        return Err(CanonicalOutputError::Resource {
            limit: kind.limit("write buffer bytes"),
            required: 1,
            allowed: limits.max_write_buffer_bytes,
        });
    }
    require(
        u64::try_from(buffer_bytes).unwrap_or(u64::MAX),
        limits.max_working_bytes,
        kind.limit("working bytes"),
    )?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(buffer_bytes)
        .map_err(|_| CanonicalOutputError::Resource {
            limit: kind.limit("verification buffer allocation"),
            required: u64::try_from(buffer_bytes).unwrap_or(u64::MAX),
            allowed: limits.max_working_bytes,
        })?;
    require(
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        limits.max_working_bytes,
        kind.limit("working bytes"),
    )?;
    buffer.resize(buffer_bytes, 0);
    let mut hasher = Hasher::new();
    hasher.update(kind.hash_domain());
    let mut bytes = 0_u64;
    loop {
        check_cancelled(control)?;
        let read = file
            .read(&mut buffer)
            .map_err(|source| kind.io("hash target", path, source))?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        require(bytes, limits.max_output_bytes, kind.limit("output bytes"))?;
        require(bytes, limits.max_staging_bytes, kind.limit("staging bytes"))?;
        hasher.update(&buffer[..read]);
    }
    Ok(FileFacts {
        hash: *hasher.finalize().as_bytes(),
        bytes,
    })
}

fn require_regular_target(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), CanonicalOutputError> {
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(CanonicalOutputError::TargetConflict {
            path: path.to_path_buf(),
            reason: "an existing target must be a regular non-symlink file",
        })
    }
}

fn require_stable_target(
    path: &Path,
    initial: &fs::Metadata,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> Result<(), CanonicalOutputError> {
    require_regular_target(path, current)?;
    if same_file_identity(initial, opened) && same_file_identity(opened, current) {
        Ok(())
    } else {
        Err(changed_target_error(path))
    }
}

fn changed_target_error(path: &Path) -> CanonicalOutputError {
    CanonicalOutputError::TargetChanged {
        path: path.to_path_buf(),
    }
}

fn verify_reconciled_parent(
    witness: &DirectoryWitness,
    parent: &Path,
    target: &Path,
    kind: CanonicalOutputSpec,
) -> Result<(), CanonicalOutputError> {
    match witness.verify_detailed() {
        Ok(()) => Ok(()),
        Err(DirectoryWitnessError::Changed(_)) => Err(changed_target_error(target)),
        Err(DirectoryWitnessError::Io(source)) => {
            Err(kind.io("verify reconciled parent", parent, source))
        }
    }
}

fn validate_limits(
    limits: CanonicalOutputLimits,
    kind: CanonicalOutputSpec,
) -> Result<(), CanonicalOutputError> {
    require(1, limits.max_output_bytes, kind.limit("output bytes"))?;
    require(1, limits.max_staging_bytes, kind.limit("staging bytes"))?;
    require(
        1,
        limits.max_write_buffer_bytes,
        kind.limit("write buffer bytes"),
    )?;
    require(
        limits.max_write_buffer_bytes.min(HASH_BUFFER_BYTES as u64),
        limits.max_working_bytes,
        kind.limit("working bytes"),
    )
}

fn require(required: u64, allowed: u64, limit: String) -> Result<(), CanonicalOutputError> {
    if required > allowed {
        Err(CanonicalOutputError::Resource {
            limit,
            required,
            allowed,
        })
    } else {
        Ok(())
    }
}

fn check_cancelled(control: &OperationControl) -> Result<(), CanonicalOutputError> {
    control
        .check_cancelled()
        .map_err(|_| CanonicalOutputError::Cancelled)
}

fn create_stage(
    parent: &Path,
    kind: CanonicalOutputSpec,
    control: &OperationControl,
) -> Result<(StageGuard, File), CanonicalOutputError> {
    create_publication_stage(
        parent,
        kind.namespace(),
        || check_cancelled(control),
        |error| match error {
            StageCreationError::NamespaceExhausted => {
                kind.invalid("staging name space is exhausted")
            }
            StageCreationError::Inspect { path, source } => kind.io("inspect stage", &path, source),
            StageCreationError::Create { path, source } => kind.io("create stage", &path, source),
        },
    )
}

#[cfg(test)]
struct Hex<'a>(&'a [u8]);

#[cfg(test)]
impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
const REPORT_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-report-bytes-v1";
#[cfg(test)]
const REPORT_OUTPUT: CanonicalOutputSpec =
    CanonicalOutputSpec::new("report", "report", REPORT_HASH_DOMAIN);
#[cfg(test)]
const EVIDENCE_OUTPUT: CanonicalOutputSpec =
    CanonicalOutputSpec::new("Round-Trip Evidence", "round-trip-evidence", b"");

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[derive(Clone, Copy)]
    enum TestAction<'a> {
        Failure(PublicationBoundary),
        Cancellation(PublicationBoundary),
        Install {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
            replace: bool,
        },
        ModifyInPlace {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
        },
    }

    struct TestHook<'a>(TestAction<'a>);

    impl PublicationHook for TestHook<'_> {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            control: &OperationControl,
        ) -> io::Result<()> {
            match self.0 {
                TestAction::Failure(expected) if boundary == expected => Err(io::Error::other(
                    format!("injected report failure at {boundary:?}"),
                )),
                TestAction::Cancellation(expected) if boundary == expected => {
                    control.cancel();
                    Ok(())
                }
                TestAction::Install {
                    boundary: expected,
                    target,
                    bytes,
                    replace,
                } if boundary == expected => {
                    if replace {
                        fs::remove_file(target)?;
                    }
                    write_synced(target, bytes)
                }
                TestAction::ModifyInPlace {
                    boundary: expected,
                    target,
                    bytes,
                } if boundary == expected => overwrite_synced(target, bytes),
                _ => Ok(()),
            }
        }
    }

    #[test]
    fn pre_link_failure_and_cancellation_leave_no_target_and_only_safe_stage() {
        let directory = Directory::new("pre-link");
        for action in [
            TestAction::Failure(PublicationBoundary::BeforeLink),
            TestAction::Cancellation(PublicationBoundary::BeforeLink),
        ] {
            let target = directory
                .path
                .join(format!("target-{}.json", action_name(action)));
            let (mut stage, expected) = directory.prepared(b"canonical report\n");
            let control = OperationControl::new();
            let failure =
                publish_prepared(&target, &mut stage, expected, &control, &TestHook(action))
                    .expect_err("a pre-link stop cannot return a receipt");
            assert!(matches!(
                failure,
                CanonicalOutputError::Io { .. } | CanonicalOutputError::Cancelled
            ));
            assert!(!target.exists());
            drop(stage);
            directory.assert_safe_stages();
        }
    }

    #[test]
    fn evidence_publication_failures_name_the_evidence_artifact() {
        let directory = Directory::new("evidence-diagnostic");
        let target = directory.path.join("evidence.json");
        let validate_inputs = || Ok(());
        let failure = ensure_encoded_output(
            EncodedOutputRequest {
                target: &target,
                kind: EVIDENCE_OUTPUT,
                limits: CanonicalOutputLimits::default(),
            },
            &OperationControl::new(),
            &TestHook(TestAction::Failure(PublicationBoundary::BeforeLink)),
            |writer| writer.write_all(b"canonical evidence\n"),
            &validate_inputs,
        )
        .expect_err("an injected evidence failure cannot return a receipt");

        assert!(matches!(
            failure,
            CanonicalOutputError::Io { operation, .. }
                if operation == "run pre-link boundary for Round-Trip Evidence"
        ));
        assert!(!target.exists());
    }

    #[test]
    fn every_post_link_boundary_is_indeterminate_with_complete_bytes() {
        let directory = Directory::new("post-link");
        let canonical = b"canonical report\n";
        for boundary in [
            PublicationBoundary::TargetSync,
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRetention,
            PublicationBoundary::RetentionSync,
            PublicationBoundary::TerminalAcknowledgement,
        ] {
            let target = directory.path.join(format!("{boundary:?}.json"));
            let (mut stage, expected) = directory.prepared(canonical);
            let failure = publish_prepared(
                &target,
                &mut stage,
                expected,
                &OperationControl::new(),
                &TestHook(TestAction::Failure(boundary)),
            )
            .expect_err("post-link failure cannot acknowledge publication");
            assert!(matches!(
                failure,
                CanonicalOutputError::Indeterminate {
                    expected_hash,
                    ..
                } if expected_hash == expected.hash
            ));
            assert_eq!(fs::read(&target).unwrap(), canonical);
            drop(stage);
            directory.assert_safe_stages();
        }
    }

    #[test]
    fn lost_acknowledgement_cancellation_is_indeterminate_and_reconcilable() {
        let directory = Directory::new("lost-ack");
        let target = directory.path.join("audit.json");
        let canonical = b"canonical report\n";
        let (mut stage, expected) = directory.prepared(canonical);
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Cancellation(
                PublicationBoundary::TerminalAcknowledgement,
            )),
        )
        .expect_err("lost acknowledgement has no receipt");
        assert!(matches!(
            failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        drop(stage);

        let (mut retry_stage, retry_expected) = directory.prepared(canonical);
        let receipt = publish_prepared(
            &target,
            &mut retry_stage,
            retry_expected,
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("retry reconciles exact durable bytes");
        assert_eq!(
            receipt.disposition,
            CanonicalOutputDisposition::ReconciledExisting
        );
        assert_eq!(receipt.content_hash, expected.hash);
        directory.assert_safe_stages();
    }

    #[test]
    fn already_exists_race_reconciles_exact_and_preserves_conflict() {
        let directory = Directory::new("already-exists");
        let canonical = b"canonical report\n";
        let exact_target = directory.path.join("exact.json");
        let (mut exact_stage, expected) = directory.prepared(canonical);
        let exact_receipt = publish_prepared(
            &exact_target,
            &mut exact_stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::BeforeLink,
                target: &exact_target,
                bytes: canonical,
                replace: false,
            }),
        )
        .expect("an exact create race reconciles");
        assert_eq!(
            exact_receipt.disposition,
            CanonicalOutputDisposition::ReconciledExisting
        );

        let conflict_target = directory.path.join("conflict.json");
        let caller_bytes = b"caller-owned conflict\n";
        let (mut conflict_stage, conflict_expected) = directory.prepared(canonical);
        let failure = publish_prepared(
            &conflict_target,
            &mut conflict_stage,
            conflict_expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::BeforeLink,
                target: &conflict_target,
                bytes: caller_bytes,
                replace: false,
            }),
        )
        .expect_err("a conflicting create race fails closed");
        assert!(matches!(
            failure,
            CanonicalOutputError::Conflict {
                expected_hash,
                actual_hash,
                ..
            } if expected_hash == conflict_expected.hash
                && actual_hash == facts_for(caller_bytes).hash
        ));
        assert_eq!(fs::read(&conflict_target).unwrap(), caller_bytes);
        drop(exact_stage);
        drop(conflict_stage);
        directory.assert_safe_stages();
    }

    #[test]
    fn post_link_replacement_is_preserved_and_never_acknowledged() {
        let directory = Directory::new("replacement");
        let target = directory.path.join("audit.json");
        let canonical = b"canonical report\n";
        let replacement = b"caller replacement\n";
        let (mut stage, expected) = directory.prepared(canonical);
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::TargetVerification,
                target: &target,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("a replaced post-link target has no receipt");
        assert!(matches!(
            failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        drop(stage);
        directory.assert_safe_stages();
    }

    #[test]
    fn final_window_replacement_is_preserved_and_never_acknowledged() {
        let directory = Directory::new("final-window-replacement");
        let target = directory.path.join("audit.json");
        let replacement = b"caller final-window replacement\n";
        let (mut stage, expected) = directory.prepared(b"canonical report\n");
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::TerminalAcknowledgement,
                target: &target,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("a final-window replacement has no receipt");
        assert!(matches!(
            failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        drop(stage);
        directory.assert_safe_stages();
    }

    #[test]
    fn replacement_during_terminal_input_validation_has_no_receipt() {
        let directory = Directory::new("terminal-validation-replacement");
        let target = directory.path.join("evidence.json");
        let replacement = b"caller replacement during input validation\n";
        let (mut stage, expected) = directory.prepared(b"canonical report\n");
        let validation = || {
            fs::remove_file(&target)?;
            write_synced(&target, replacement)
        };
        let failure = publish_prepared_validating(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &ProductionPublicationHook,
            &validation,
        )
        .expect_err("target replacement during input validation has no receipt");
        assert!(matches!(
            failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), replacement);
    }

    #[test]
    fn parent_sync_replacements_are_preserved_for_create_and_reconcile() {
        let directory = Directory::new("parent-sync-replacement");
        let canonical = b"canonical report\n";
        let replacement = b"caller parent-sync replacement\n";

        let created_target = directory.path.join("created.json");
        let (mut created_stage, created_expected) = directory.prepared(canonical);
        let created_failure = publish_prepared(
            &created_target,
            &mut created_stage,
            created_expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::ParentSync,
                target: &created_target,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("create replacement during parent sync has no receipt");
        assert!(matches!(
            created_failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        assert_eq!(fs::read(&created_target).unwrap(), replacement);

        let reconciled_target = directory.path.join("reconciled.json");
        write_synced(&reconciled_target, canonical).unwrap();
        let (mut reconciled_stage, reconciled_expected) = directory.prepared(canonical);
        let reconciled_failure = publish_prepared(
            &reconciled_target,
            &mut reconciled_stage,
            reconciled_expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::ParentSync,
                target: &reconciled_target,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("reconciliation replacement during parent sync has no receipt");
        assert!(!matches!(
            reconciled_failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        assert_eq!(fs::read(&reconciled_target).unwrap(), replacement);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn retained_stage_and_published_target_never_alias() {
        let directory = Directory::new("anti-alias");
        let target = directory.path.join("audit.json");
        let canonical = b"canonical report\n";
        let (mut stage, expected) = directory.prepared(canonical);
        let stage_path = stage.path().to_path_buf();
        publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("publish independent clone");
        #[cfg(target_os = "macos")]
        {
            overwrite_synced(&stage_path, b"mutated retained private stage\n").unwrap();
            assert_eq!(fs::read(&target).unwrap(), canonical);
            assert_ne!(fs::read(&stage_path).unwrap(), canonical);
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt as _;
            assert!(!stage_path.exists(), "Linux stage must remain unnamed");
            assert_eq!(fs::metadata(&target).unwrap().nlink(), 1);
            assert_eq!(fs::read(&target).unwrap(), canonical);
        }
    }

    #[cfg(unix)]
    #[test]
    fn post_link_in_place_modification_is_preserved() {
        let directory = Directory::new("in-place-modification");
        let target = directory.path.join("audit.json");
        let concurrent_bytes = b"concurrent writer bytes\n";
        let (mut stage, expected) = directory.prepared(b"canonical report\n");
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::ModifyInPlace {
                boundary: PublicationBoundary::TargetVerification,
                target: &target,
                bytes: concurrent_bytes,
            }),
        )
        .expect_err("a modified post-link target has no receipt");
        assert!(matches!(
            failure,
            CanonicalOutputError::Indeterminate { .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), concurrent_bytes);
        drop(stage);
        assert_eq!(fs::read(&target).unwrap(), concurrent_bytes);
        fs::remove_file(target).unwrap();
        directory.assert_safe_stages();
    }

    #[test]
    fn stage_guard_never_removes_a_replacement_path() {
        let directory = Directory::new("stage-replacement");
        let (stage, _) = directory.prepared(b"canonical report\n");
        let stage_path = stage.path().to_path_buf();
        fs::remove_file(&stage_path).unwrap();
        write_synced(&stage_path, b"unowned replacement\n").unwrap();
        drop(stage);
        assert_eq!(fs::read(&stage_path).unwrap(), b"unowned replacement\n");
        fs::remove_file(stage_path).unwrap();
    }

    #[test]
    fn non_regular_target_has_target_conflict_taxonomy() {
        let directory = Directory::new("target-kind");
        let target = directory.path.join("audit.json");
        fs::create_dir(&target).unwrap();
        let (mut stage, expected) = directory.prepared(b"canonical report\n");
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect_err("a directory target fails closed");
        assert!(matches!(
            failure,
            CanonicalOutputError::TargetConflict { .. }
        ));
        assert!(target.is_dir());
        drop(stage);
        directory.assert_safe_stages();
    }

    #[test]
    fn staging_and_actual_verification_capacity_are_bounded() {
        let directory = Directory::new("limits");
        let stage_path = directory.path.join("stage.json");
        let file = File::create(&stage_path).unwrap();
        let control = OperationControl::new();
        let mut writer = HashingWriter::new(file, 3, &control, REPORT_HASH_DOMAIN);
        let write_error = writer.write_all(b"four").unwrap_err();
        let limits = CanonicalOutputLimits {
            max_output_bytes: 10,
            max_staging_bytes: 3,
            max_write_buffer_bytes: 8,
            max_working_bytes: 8,
        };
        assert!(matches!(
            map_write_error(
                write_error,
                &writer,
                limits,
                REPORT_OUTPUT,
                &stage_path
            ),
            CanonicalOutputError::Resource {
                limit,
                required: 4,
                allowed: 3
            } if limit == "report staging bytes"
        ));
        drop(writer);

        fs::remove_file(&stage_path).unwrap();
        write_synced(&stage_path, b"bounded").unwrap();
        let metadata = fs::symlink_metadata(&stage_path).unwrap();
        let verification_limits = CanonicalOutputLimits {
            max_output_bytes: 10,
            max_staging_bytes: 10,
            max_write_buffer_bytes: 8,
            max_working_bytes: 7,
        };
        assert!(matches!(
            verify_existing_regular_file(
                &stage_path,
                &metadata,
                verification_limits,
                &OperationControl::new(),
                REPORT_OUTPUT
            ),
            Err(CanonicalOutputError::Resource {
                limit,
                required: 8,
                allowed: 7
            }) if limit == "report working bytes"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn directory_witness_detects_same_path_replacement() {
        let directory = Directory::new("parent-witness");
        let witness = DirectoryWitness::capture(&directory.path).unwrap();
        fs::remove_dir(&directory.path).unwrap();
        fs::create_dir(&directory.path).unwrap();
        assert!(witness.verify().is_err());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn reconciled_parent_verification_distinguishes_identity_change_from_io() {
        let replaced = Directory::new("reconciled-parent-replaced");
        let replaced_witness = DirectoryWitness::capture(&replaced.path).unwrap();
        let replaced_target = replaced.path.join("audit.json");
        fs::remove_dir(&replaced.path).unwrap();
        fs::create_dir(&replaced.path).unwrap();

        assert!(matches!(
            verify_reconciled_parent(
                &replaced_witness,
                &replaced.path,
                &replaced_target,
                REPORT_OUTPUT,
            ),
            Err(CanonicalOutputError::TargetChanged { path }) if path == replaced_target
        ));

        let missing = Directory::new("reconciled-parent-missing");
        let missing_witness = DirectoryWitness::capture(&missing.path).unwrap();
        let missing_target = missing.path.join("audit.json");
        fs::remove_dir(&missing.path).unwrap();

        assert!(matches!(
            verify_reconciled_parent(
                &missing_witness,
                &missing.path,
                &missing_target,
                REPORT_OUTPUT,
            ),
            Err(CanonicalOutputError::Io {
                operation,
                path,
                source,
            }) if operation == "verify reconciled parent for report"
                && path == missing.path
                && source.kind() == io::ErrorKind::NotFound
        ));
    }

    fn publish_prepared(
        target: &Path,
        stage: &mut StageGuard,
        expected: FileFacts,
        control: &OperationControl,
        hook: &impl PublicationHook,
    ) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
        let terminal_validation = || Ok(());
        publish_prepared_validating(target, stage, expected, control, hook, &terminal_validation)
    }

    fn publish_prepared_validating(
        target: &Path,
        stage: &mut StageGuard,
        expected: FileFacts,
        control: &OperationControl,
        hook: &impl PublicationHook,
        terminal_validation: &dyn Fn() -> io::Result<()>,
    ) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
        let parent = target.parent().unwrap();
        let parent_witness = DirectoryWitness::capture(parent).unwrap();
        publish_or_reconcile(
            PublicationContext {
                target,
                parent,
                parent_witness: &parent_witness,
                expected,
                limits: CanonicalOutputLimits::default(),
                kind: REPORT_OUTPUT,
                terminal_validation,
            },
            stage,
            control,
            hook,
        )
    }

    fn facts_for(bytes: &[u8]) -> FileFacts {
        let mut hasher = Hasher::new();
        hasher.update(REPORT_HASH_DOMAIN);
        hasher.update(bytes);
        FileFacts {
            hash: *hasher.finalize().as_bytes(),
            bytes: u64::try_from(bytes.len()).unwrap(),
        }
    }

    fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        sync_directory(path.parent().unwrap())
    }

    fn overwrite_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    const fn action_name(action: TestAction<'_>) -> &'static str {
        match action {
            TestAction::Failure(_) => "failure",
            TestAction::Cancellation(_) => "cancellation",
            TestAction::Install { .. } => "install",
            TestAction::ModifyInPlace { .. } => "modify-in-place",
        }
    }

    struct Directory {
        path: PathBuf,
    }

    impl Directory {
        fn new(label: &str) -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "punctra-terrain-report-{label}-{}-{}",
                std::process::id(),
                Hex(&random)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn prepared(&self, bytes: &[u8]) -> (StageGuard, FileFacts) {
            let (stage, mut file) =
                create_stage(&self.path, REPORT_OUTPUT, &OperationControl::new()).unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            drop(file);
            stage.verify().unwrap();
            (stage, facts_for(bytes))
        }

        fn assert_safe_stages(&self) {
            let stages = fs::read_dir(&self.path)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".punctra-report-")
                })
                .collect::<Vec<_>>();
            assert!(stages.len() <= 16, "test operation leaked excess stages");
            for stage in stages {
                let metadata = stage.metadata().unwrap();
                assert!(metadata.is_file(), "private stage must remain regular");
                assert!(
                    metadata.len() <= CanonicalOutputLimits::default().max_staging_bytes,
                    "private stage must remain bounded"
                );
            }
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
