use std::collections::HashSet;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::MAX_ATTRIBUTE_NAME_BYTES;
use thiserror::Error;

use crate::{
    error::WorkspaceError,
    util::{allocation_bytes, encode_hex},
};

pub(crate) const WORKSPACE_ID_BYTES: usize = 16;
pub(crate) const OPERATION_ID_BYTES: usize = 16;
pub(crate) const REVISION_ID_BYTES: usize = 32;
pub(crate) const DIGEST_BYTES: usize = 32;
pub(crate) const ATTRIBUTE_DATA_TYPE_U8: u8 = 1;

const DISK_VERSION: u32 = 1;
const SEMANTIC_VERSION: u32 = 1;
const MANIFEST_MAGIC: &[u8; 8] = b"PWSMAN01";
const REVISION_MAGIC: &[u8; 8] = b"PWSREV01";
const BLOCK_MAGIC: &[u8; 8] = b"PWSBLK01";
const FOOTER_MAGIC: &[u8; 8] = b"PWSEND01";
const REJECTION_MAGIC: &[u8; 8] = b"PWSREJ01";
pub(crate) const MANIFEST_BYTES: usize = 228 + MAX_ATTRIBUTE_NAME_BYTES;
const REVISION_HEADER_BYTES: usize = 384;
const BLOCK_HEADER_BYTES: usize = 72;
const ROW_BYTES: usize = 10;
const FOOTER_BYTES: usize = 48;
pub(crate) const REJECTION_BYTES: usize = 184;
const REVISION_HEADER_BYTES_U32: u32 = 384;
const ROW_BYTES_U32: u32 = 10;
const CHECKSUM_READ_BUFFER_BYTES: usize = 64 * 1024;
const REVISION_ID_DOMAIN: &[u8] = b"punctra-workspace-revision-v1";
const DELTA_DOMAIN: &[u8] = b"punctra-workspace-delta-v1";
const BLOCK_DOMAIN: &[u8] = b"punctra-workspace-revision-block-v1";
const FILE_DOMAIN: &[u8] = b"punctra-workspace-revision-file-v1";
const MANIFEST_DOMAIN: &[u8] = b"punctra-workspace-manifest-v1";
const REJECTION_DOMAIN: &[u8] = b"punctra-workspace-rejection-v1";
const CLASSIFICATION_REQUEST_DOMAIN: &[u8] = b"punctra-workspace-classification-request-v1";
const REVERT_REQUEST_DOMAIN: &[u8] = b"punctra-workspace-revert-request-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FaultPoint {
    ManifestStage,
    ManifestLink,
    ManifestDirectorySync,
    ManifestParentDirectorySync,
    ManifestLostAcknowledgement,
    CandidateStage,
    CandidateFileSync,
    CandidateClose,
    CandidateReadOnly,
    CandidateRevalidate,
    ReadyLink,
    OperationsDirectorySync,
    ReadyCleanup,
    OperationLostAcknowledgement,
    RejectionStage,
    RejectionFileSync,
    RejectionReadOnly,
    RejectionRevalidate,
    RejectionLink,
    RejectionDirectorySync,
    RejectionCleanup,
    RejectionLostAcknowledgement,
    RevisionLink,
    RevisionDirectorySync,
    RevisionLostAcknowledgement,
    RecoveryOperationsSync,
    RecoveryRevisionsSync,
    RecoveryRootSync,
    RecoveryParentSync,
}

// The production branch is an intentional no-op; tests replace it with a
// fallible/panicking boundary injector without changing protocol code.
#[allow(clippy::unnecessary_wraps)]
fn inject_fault(path: &Path, point: FaultPoint) -> Result<(), PersistenceError> {
    #[cfg(test)]
    {
        test_fault::inject(path, point)
    }
    #[cfg(not(test))]
    {
        let _ = (path, point);
        Ok(())
    }
}

pub(crate) type WorkspaceBytes = [u8; WORKSPACE_ID_BYTES];
pub(crate) type OperationBytes = [u8; OPERATION_ID_BYTES];
pub(crate) type RevisionBytes = [u8; REVISION_ID_BYTES];
pub(crate) type DigestBytes = [u8; DIGEST_BYTES];
type RevisionEntry = (u64, RevisionBytes, PathBuf);
type OperationEntry = (OperationBytes, OperationFileKind, PathBuf);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistedAttributeDefinition {
    pub(crate) id: u32,
    pub(crate) name_len: u32,
    pub(crate) name: [u8; MAX_ATTRIBUTE_NAME_BYTES],
    pub(crate) data_type: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManifestFacts {
    pub(crate) workspace: WorkspaceBytes,
    pub(crate) source: DigestBytes,
    pub(crate) source_point_count: u64,
    pub(crate) position_transform_bits: [u64; 6],
    pub(crate) classification: PersistedAttributeDefinition,
    pub(crate) root_revision: RevisionBytes,
    pub(crate) source_contract: DigestBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RevisionKind {
    SetClassification(u8),
    Revert,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CandidateFacts {
    pub(crate) workspace: WorkspaceBytes,
    pub(crate) source: DigestBytes,
    pub(crate) source_contract: DigestBytes,
    pub(crate) operation: OperationBytes,
    pub(crate) request_digest: DigestBytes,
    pub(crate) parent: RevisionBytes,
    pub(crate) sequence: u64,
    pub(crate) kind: RevisionKind,
    pub(crate) point_set: Option<PersistedPointSetFacts>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistedPointSetFacts {
    pub(crate) exact_count: u64,
    pub(crate) point_id_hash: DigestBytes,
    pub(crate) content_hash: DigestBytes,
}

#[derive(Clone, Copy)]
pub(crate) struct ClassificationRequestFacts {
    pub(crate) workspace: WorkspaceBytes,
    pub(crate) source: DigestBytes,
    pub(crate) classification_attribute: u32,
    pub(crate) point_set_workspace: WorkspaceBytes,
    pub(crate) point_set_source: DigestBytes,
    pub(crate) parent: RevisionBytes,
    pub(crate) point_set: PersistedPointSetFacts,
    pub(crate) value: u8,
}

pub(crate) fn classification_request_digest(facts: ClassificationRequestFacts) -> DigestBytes {
    let mut hasher = Hasher::new();
    hasher.update(CLASSIFICATION_REQUEST_DOMAIN);
    hasher.update(&facts.workspace);
    hasher.update(&facts.source);
    hasher.update(&facts.classification_attribute.to_le_bytes());
    hasher.update(&facts.point_set_workspace);
    hasher.update(&facts.point_set_source);
    hasher.update(&facts.parent);
    hasher.update(&facts.point_set.exact_count.to_le_bytes());
    hasher.update(&facts.point_set.point_id_hash);
    hasher.update(&facts.point_set.content_hash);
    hasher.update(&[facts.value]);
    *hasher.finalize().as_bytes()
}

pub(crate) fn revert_request_digest(
    workspace: WorkspaceBytes,
    source: DigestBytes,
    parent: RevisionBytes,
) -> DigestBytes {
    let mut hasher = Hasher::new();
    hasher.update(REVERT_REQUEST_DOMAIN);
    hasher.update(&workspace);
    hasher.update(&source);
    hasher.update(&parent);
    *hasher.finalize().as_bytes()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RevisionRow {
    pub(crate) ordinal: u64,
    pub(crate) before: u8,
    pub(crate) after: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RevisionFacts {
    pub(crate) candidate: CandidateFacts,
    pub(crate) revision: RevisionBytes,
    pub(crate) row_count: u64,
    pub(crate) block_count: u64,
    pub(crate) delta_digest: DigestBytes,
    pub(crate) body_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RejectionFacts {
    pub(crate) workspace: WorkspaceBytes,
    pub(crate) operation: OperationBytes,
    pub(crate) request_digest: DigestBytes,
    pub(crate) reason_code: u16,
    pub(crate) expected_head: RevisionBytes,
    pub(crate) actual_head: RevisionBytes,
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_field_names)]
pub(crate) struct ReadLimits {
    pub(crate) max_file_bytes: u64,
    pub(crate) max_rows: u64,
    pub(crate) max_blocks: u64,
    pub(crate) max_block_bytes: u64,
    pub(crate) max_working_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WriteLimits {
    pub(crate) max_file_bytes: u64,
    pub(crate) max_rows: u64,
    pub(crate) max_blocks: u64,
    pub(crate) max_block_bytes: u64,
    pub(crate) rows_per_block: u32,
    pub(crate) max_working_bytes: u64,
    pub(crate) retained_input_bytes: u64,
    pub(crate) max_temporary_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CatalogLimits {
    pub(crate) read: ReadLimits,
    pub(crate) max_revisions: u64,
    pub(crate) max_operation_files: u64,
    pub(crate) max_total_bytes: u64,
    pub(crate) max_metadata_bytes: u64,
}

#[derive(Default)]
struct RecoveryLedger {
    durable_bytes: u64,
    scanned_rows: u64,
    scanned_blocks: u64,
    resident_metadata_bytes: u64,
}

impl RecoveryLedger {
    fn preflight_metadata(
        &self,
        bytes: u64,
        limits: CatalogLimits,
    ) -> Result<(), PersistenceError> {
        ensure_at_most(
            self.resident_metadata_bytes.saturating_add(bytes),
            limits.max_metadata_bytes,
            "resident catalog metadata",
        )
    }

    fn charge_durable(
        &mut self,
        bytes: u64,
        limits: CatalogLimits,
    ) -> Result<(), PersistenceError> {
        charge(
            &mut self.durable_bytes,
            bytes,
            limits.max_total_bytes,
            "durable bytes",
        )
    }

    fn charge_metadata(
        &mut self,
        bytes: u64,
        limits: CatalogLimits,
    ) -> Result<(), PersistenceError> {
        charge(
            &mut self.resident_metadata_bytes,
            bytes,
            limits.max_metadata_bytes,
            "resident catalog metadata",
        )
    }

    fn read_limits(
        &self,
        limits: CatalogLimits,
        retained_block_metadata: bool,
        temporary_working_bytes: u64,
    ) -> Result<ReadLimits, PersistenceError> {
        let remaining_rows = limits
            .read
            .max_rows
            .checked_sub(self.scanned_rows)
            .ok_or_else(|| limit("Revision rows scanned", u64::MAX, limits.read.max_rows))?;
        let mut remaining_blocks = limits
            .read
            .max_blocks
            .checked_sub(self.scanned_blocks)
            .ok_or_else(|| limit("Revision blocks scanned", u64::MAX, limits.read.max_blocks))?;
        if retained_block_metadata {
            let remaining_metadata = limits
                .max_metadata_bytes
                .checked_sub(self.resident_metadata_bytes)
                .ok_or_else(|| {
                    limit(
                        "resident catalog metadata",
                        u64::MAX,
                        limits.max_metadata_bytes,
                    )
                })?;
            let block_bytes = u64::try_from(std::mem::size_of::<BlockMetadata>())
                .unwrap_or(u64::MAX)
                .max(1);
            let available_blocks =
                remaining_metadata.saturating_sub(arc_block_vec_fixed_bytes()) / block_bytes;
            remaining_blocks = remaining_blocks.min(available_blocks);
        }
        Ok(ReadLimits {
            max_file_bytes: limits.read.max_file_bytes,
            max_rows: remaining_rows,
            max_blocks: remaining_blocks,
            max_block_bytes: limits.read.max_block_bytes,
            max_working_bytes: limits
                .read
                .max_working_bytes
                .checked_sub(temporary_working_bytes)
                .ok_or_else(|| {
                    limit(
                        "recovery working metadata",
                        temporary_working_bytes,
                        limits.read.max_working_bytes,
                    )
                })?,
        })
    }

    fn charge_revision_scan(
        &mut self,
        revision: &ValidatedRevision,
        retain_blocks: bool,
        limits: CatalogLimits,
    ) -> Result<(), PersistenceError> {
        charge(
            &mut self.scanned_rows,
            revision.facts.row_count,
            limits.read.max_rows,
            "Revision rows scanned",
        )?;
        charge(
            &mut self.scanned_blocks,
            revision.facts.block_count,
            limits.read.max_blocks,
            "Revision blocks scanned",
        )?;
        if retain_blocks {
            self.charge_metadata(revision.block_metadata_retained_bytes(), limits)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_field_names)]
pub(crate) struct OverlayLimits {
    pub(crate) max_blocks: u64,
    pub(crate) max_payload_bytes: u64,
    pub(crate) max_block_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OverlayUsage {
    blocks: u64,
    payload_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_field_names)]
pub(crate) struct RowReadLimits {
    pub(crate) max_frames: u64,
    pub(crate) max_payload_bytes: u64,
    pub(crate) max_working_bytes: u64,
}

#[derive(Debug, Error)]
pub(crate) enum PersistenceError {
    #[error("workspace is already locked by another live session")]
    Locked,
    #[error("{action} failed for {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("corrupt published workspace file {path}: {reason}")]
    Corrupt { path: PathBuf, reason: &'static str },
    #[error("incompatible workspace file {path}: {reason}")]
    Incompatible { path: PathBuf, reason: &'static str },
    #[error("workspace resource limit exceeded for {resource}: {actual} > {limit}")]
    Limit {
        resource: &'static str,
        actual: u64,
        limit: u64,
    },
    #[error("operation publication target already belongs to another payload")]
    PublicationConflict,
    #[error("the operating system could not produce scratch-file entropy")]
    Entropy,
    #[error("Workspace persistence work was cancelled")]
    Cancelled,
    #[error("candidate Revision contains no changed rows")]
    NoRows,
    #[error("candidate input failed: {0}")]
    Input(#[source] Box<WorkspaceError>),
}

#[derive(Debug)]
pub(crate) struct SealedRevision {
    file: ValidatedRevision,
}

pub(crate) struct PreparedRejection {
    facts: RejectionFacts,
    scratch_path: PathBuf,
    target_path: PathBuf,
    file: Option<File>,
    bytes: [u8; REJECTION_BYTES],
    cleanup_scratch: bool,
}

impl PreparedRejection {
    pub(crate) fn working_bytes(&self) -> u64 {
        (REJECTION_BYTES as u64)
            .saturating_add(u64::try_from(self.scratch_path.capacity()).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(self.target_path.capacity()).unwrap_or(u64::MAX))
    }
}

impl Drop for PreparedRejection {
    fn drop(&mut self) {
        if self.cleanup_scratch {
            let _ = fs::remove_file(&self.scratch_path);
        }
    }
}

impl SealedRevision {
    pub(crate) fn path(&self) -> &Path {
        &self.file.path
    }

    pub(crate) const fn file_bytes(&self) -> u64 {
        self.file.facts.body_bytes + FOOTER_BYTES as u64
    }

    pub(crate) const fn operation(&self) -> OperationBytes {
        self.file.facts.candidate.operation
    }

    pub(crate) fn retained_working_bytes(&self) -> u64 {
        self.file.retained_working_bytes()
    }
}

impl Drop for SealedRevision {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file.path);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ValidatedRevision {
    path: PathBuf,
    facts: RevisionFacts,
    blocks: Arc<Vec<BlockMetadata>>,
}

#[derive(Clone, Debug)]
struct BlockMetadata {
    payload_offset: u64,
    payload_bytes: u64,
    row_count: u32,
    first_ordinal: u64,
    last_ordinal: u64,
    checksum: DigestBytes,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OperationRecord {
    pub(crate) ready: Option<Arc<ValidatedRevision>>,
    pub(crate) rejection: Option<RejectionFacts>,
}

#[derive(Clone, Debug)]
pub(crate) struct Catalog {
    root_revision: RevisionBytes,
    pub(crate) revisions: Vec<Arc<ValidatedRevision>>,
    pub(crate) operations: Vec<(OperationBytes, OperationRecord)>,
}

impl Catalog {
    pub(crate) fn empty(root_revision: RevisionBytes) -> Self {
        Self {
            root_revision,
            revisions: Vec::new(),
            operations: Vec::new(),
        }
    }

    pub(crate) fn head(&self) -> RevisionBytes {
        self.revisions
            .last()
            .map_or(self.root_revision, |revision| revision.facts.revision)
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        u64::try_from(self.revisions.len())
            .expect("the validated catalog length fits u64")
            .saturating_add(1)
    }

    pub(crate) fn revision(&self, id: RevisionBytes) -> Option<&ValidatedRevision> {
        self.revisions
            .iter()
            .find(|revision| revision.facts.revision == id)
            .map(Arc::as_ref)
    }

    pub(crate) fn revision_arc(&self, id: RevisionBytes) -> Option<Arc<ValidatedRevision>> {
        self.revisions
            .iter()
            .find(|revision| revision.facts.revision == id)
            .map(Arc::clone)
    }

    pub(crate) fn operation(&self, id: OperationBytes) -> Option<&OperationRecord> {
        self.operations
            .iter()
            .find_map(|(operation, record)| (*operation == id).then_some(record))
    }

    pub(crate) fn operation_file_count(&self) -> u64 {
        self.operations.iter().fold(0_u64, |count, (_, record)| {
            count
                .saturating_add(u64::from(record.ready.is_some()))
                .saturating_add(u64::from(record.rejection.is_some()))
        })
    }

    pub(crate) fn committed_operation(
        &self,
        id: OperationBytes,
    ) -> Option<&Arc<ValidatedRevision>> {
        self.revisions
            .iter()
            .find(|revision| revision.facts.candidate.operation == id)
    }

    pub(crate) fn reserve_operation(
        &mut self,
        operation: OperationBytes,
        max_transition_bytes: u64,
    ) -> Result<u64, PersistenceError> {
        if self.operation(operation).is_some() {
            return Ok(0);
        }
        reserve_vec_transition(
            &mut self.operations,
            1,
            max_transition_bytes,
            "Operation catalog growth",
        )
    }

    pub(crate) fn reserve_revision(
        &mut self,
        max_transition_bytes: u64,
    ) -> Result<u64, PersistenceError> {
        reserve_vec_transition(
            &mut self.revisions,
            1,
            max_transition_bytes,
            "Revision catalog growth",
        )
    }

    pub(crate) fn append_committed(&mut self, revision: Arc<ValidatedRevision>) {
        debug_assert!(self.revisions.len() < self.revisions.capacity());
        debug_assert!(self.operation(revision.facts.candidate.operation).is_some());
        self.revisions.push(revision);
    }

    pub(crate) fn record_ready(&mut self, ready: Arc<ValidatedRevision>) {
        let operation = ready.facts.candidate.operation;
        if let Some((_, record)) = self
            .operations
            .iter_mut()
            .find(|(existing, _)| *existing == operation)
        {
            record.ready = Some(ready);
        } else {
            debug_assert!(self.operations.len() < self.operations.capacity());
            self.operations.push((
                operation,
                OperationRecord {
                    ready: Some(ready),
                    rejection: None,
                },
            ));
        }
    }

    pub(crate) fn record_rejection(&mut self, rejection: RejectionFacts) {
        if let Some((_, record)) = self
            .operations
            .iter_mut()
            .find(|(operation, _)| *operation == rejection.operation)
        {
            record.rejection = Some(rejection);
        } else {
            debug_assert!(self.operations.len() < self.operations.capacity());
            self.operations.push((
                rejection.operation,
                OperationRecord {
                    ready: None,
                    rejection: Some(rejection),
                },
            ));
        }
    }

    pub(crate) fn apply_overlays(
        &self,
        revision: RevisionBytes,
        first_ordinal: u64,
        values: &mut [u8],
        limits: OverlayLimits,
        usage: &mut OverlayUsage,
        control: &OperationControl,
    ) -> Result<(), PersistenceError> {
        if values.is_empty() {
            return Ok(());
        }
        if revision == self.root_revision {
            return Ok(());
        }
        let last_ordinal = first_ordinal
            .checked_add(
                u64::try_from(values.len())
                    .map_err(|_| limit("overlay Point count", u64::MAX, u64::MAX - 1))?,
            )
            .and_then(|exclusive| exclusive.checked_sub(1))
            .ok_or_else(|| limit("overlay ordinal range", u64::MAX, u64::MAX - 1))?;

        let target_index = self
            .revisions
            .iter()
            .position(|item| item.facts.revision == revision);
        let Some(target_index) = target_index else {
            return Err(PersistenceError::Corrupt {
                path: PathBuf::from("<catalog>"),
                reason: "requested overlay Revision is not in the recovered catalog",
            });
        };

        for item in &self.revisions[..=target_index] {
            check_cancelled(control)?;
            item.apply_rows(first_ordinal, last_ordinal, values, limits, usage, control)?;
        }
        Ok(())
    }
}

impl ValidatedRevision {
    pub(crate) const fn operation(&self) -> OperationBytes {
        self.facts.candidate.operation
    }

    pub(crate) const fn request_digest(&self) -> DigestBytes {
        self.facts.candidate.request_digest
    }

    pub(crate) const fn parent(&self) -> RevisionBytes {
        self.facts.candidate.parent
    }

    pub(crate) const fn sequence(&self) -> u64 {
        self.facts.candidate.sequence
    }

    pub(crate) const fn kind(&self) -> RevisionKind {
        self.facts.candidate.kind
    }

    pub(crate) const fn point_set(&self) -> Option<PersistedPointSetFacts> {
        self.facts.candidate.point_set
    }

    pub(crate) const fn revision(&self) -> RevisionBytes {
        self.facts.revision
    }

    pub(crate) const fn row_count(&self) -> u64 {
        self.facts.row_count
    }

    pub(crate) const fn block_count(&self) -> u64 {
        self.facts.block_count
    }

    pub(crate) const fn file_bytes(&self) -> u64 {
        self.facts.body_bytes + FOOTER_BYTES as u64
    }

    pub(crate) fn max_block_rows(&self) -> u64 {
        self.blocks
            .iter()
            .map(|block| u64::from(block.row_count))
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn retained_working_bytes(&self) -> u64 {
        arc_revision_fixed_bytes()
            .saturating_add(u64::try_from(self.path.capacity()).unwrap_or(u64::MAX))
            .saturating_add(arc_block_vec_fixed_bytes())
            .saturating_add(vector_bytes::<BlockMetadata>(self.blocks.capacity()))
    }

    fn block_metadata_retained_bytes(&self) -> u64 {
        arc_block_vec_fixed_bytes()
            .saturating_add(vector_bytes::<BlockMetadata>(self.blocks.capacity()))
    }

    fn with_path(&self, path: PathBuf) -> Self {
        Self {
            path,
            facts: self.facts,
            blocks: Arc::clone(&self.blocks),
        }
    }

    pub(crate) fn max_block_bytes(&self) -> u64 {
        self.blocks
            .iter()
            .map(|block| block.payload_bytes)
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn rows<'a>(
        &'a self,
        limits: RowReadLimits,
        control: &'a OperationControl,
    ) -> Result<RevisionRows<'a>, PersistenceError> {
        let mut file = open_read(&self.path)?;
        let actual_file_bytes = file
            .metadata()
            .map_err(|source| io_error("read metadata", &self.path, source))?
            .len();
        if actual_file_bytes != self.file_bytes() {
            return corrupt(
                &self.path,
                "Revision row source length differs from its validated length",
            );
        }

        let mut header = [0_u8; REVISION_HEADER_BYTES];
        read_exact(&mut file, &mut header, &self.path)?;
        if header != encode_revision_header(&self.facts) {
            return corrupt(
                &self.path,
                "Revision row source header differs from its validated facts",
            );
        }
        let mut file_hasher = Hasher::new();
        file_hasher.update(FILE_DOMAIN);
        file_hasher.update(&header);
        let mut delta_hasher = Hasher::new();
        delta_hasher.update(DELTA_DOMAIN);

        Ok(RevisionRows {
            revision: self,
            file,
            file_hasher: Some(file_hasher),
            delta_hasher: Some(delta_hasher),
            limits,
            control,
            next_block: 0,
            payload: Vec::new(),
            payload_offset: 0,
            charged_frames: 0,
            charged_payload_bytes: 0,
            terminal: false,
        })
    }

    fn validate_source_point_count(&self, source_point_count: u64) -> Result<(), PersistenceError> {
        if self
            .blocks
            .last()
            .is_some_and(|block| block.last_ordinal >= source_point_count)
        {
            return corrupt(
                &self.path,
                "Revision contains an ordinal outside the immutable Source",
            );
        }
        if self
            .facts
            .candidate
            .point_set
            .is_some_and(|point_set| point_set.exact_count > source_point_count)
        {
            return corrupt(
                &self.path,
                "Revision Point Set count exceeds the immutable Source",
            );
        }
        Ok(())
    }

    fn apply_rows(
        &self,
        first_ordinal: u64,
        last_ordinal: u64,
        values: &mut [u8],
        limits: OverlayLimits,
        usage: &mut OverlayUsage,
        control: &OperationControl,
    ) -> Result<(), PersistenceError> {
        let mut file = open_read(&self.path)?;
        for block in self.blocks.iter().filter(|block| {
            block.first_ordinal <= last_ordinal && block.last_ordinal >= first_ordinal
        }) {
            check_cancelled(control)?;
            charge(&mut usage.blocks, 1, limits.max_blocks, "overlay blocks")?;
            charge(
                &mut usage.payload_bytes,
                block.payload_bytes,
                limits.max_payload_bytes,
                "overlay payload bytes",
            )?;
            ensure_at_most(
                block.payload_bytes,
                limits.max_block_bytes,
                "overlay block bytes",
            )?;
            let mut payload = allocate_zeroed(
                block.payload_bytes,
                limits.max_block_bytes,
                "overlay block allocation",
                &self.path,
            )?;
            seek(&mut file, block.payload_offset, &self.path)?;
            read_exact(&mut file, &mut payload, &self.path)?;
            if block_checksum(&payload) != block.checksum {
                return corrupt(&self.path, "overlay block checksum differs");
            }
            apply_payload(
                &payload,
                block.row_count,
                first_ordinal,
                last_ordinal,
                values,
                &self.path,
            )?;
        }
        Ok(())
    }
}

pub(crate) struct RevisionRows<'a> {
    revision: &'a ValidatedRevision,
    file: File,
    file_hasher: Option<Hasher>,
    delta_hasher: Option<Hasher>,
    limits: RowReadLimits,
    control: &'a OperationControl,
    next_block: usize,
    payload: Vec<u8>,
    payload_offset: usize,
    charged_frames: u64,
    charged_payload_bytes: u64,
    terminal: bool,
}

impl Iterator for RevisionRows<'_> {
    type Item = Result<RevisionRow, PersistenceError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.terminal {
            return None;
        }
        loop {
            if self.payload_offset < self.payload.len() {
                let end = self.payload_offset + ROW_BYTES;
                let row = decode_row(&self.payload[self.payload_offset..end]);
                self.payload_offset = end;
                return Some(Ok(row));
            }
            if self.next_block == self.revision.blocks.len() {
                self.terminal = true;
                return self.finish_file().err().map(Err);
            }
            if let Err(error) = self.load_next_block() {
                self.terminal = true;
                return Some(Err(error));
            }
        }
    }
}

impl RevisionRows<'_> {
    fn load_next_block(&mut self) -> Result<(), PersistenceError> {
        check_cancelled(self.control)?;
        let block = &self.revision.blocks[self.next_block];
        charge(
            &mut self.charged_frames,
            1,
            self.limits.max_frames,
            "Revision row input blocks",
        )?;
        charge(
            &mut self.charged_payload_bytes,
            block.payload_bytes,
            self.limits.max_payload_bytes,
            "Revision row input bytes",
        )?;
        let mut header = [0_u8; BLOCK_HEADER_BYTES];
        read_exact(&mut self.file, &mut header, &self.revision.path)?;
        self.file_hasher
            .as_mut()
            .expect("live Revision row reads retain their file hasher")
            .update(&header);
        let payload_offset = stream_position(&mut self.file, &self.revision.path)?;
        if header
            != encode_block_header(
                block.row_count,
                block.first_ordinal,
                block.last_ordinal,
                block.payload_bytes,
                block.checksum,
            )
            || payload_offset != block.payload_offset
        {
            return corrupt(
                &self.revision.path,
                "Revision row source block header differs from validated metadata",
            );
        }
        drop(std::mem::take(&mut self.payload));
        self.payload = allocate_zeroed(
            block.payload_bytes,
            self.limits.max_working_bytes,
            "Revision row read buffer",
            &self.revision.path,
        )?;
        read_exact(&mut self.file, &mut self.payload, &self.revision.path)?;
        self.file_hasher
            .as_mut()
            .expect("live Revision row reads retain their file hasher")
            .update(&self.payload);
        self.delta_hasher
            .as_mut()
            .expect("live Revision row reads retain their delta hasher")
            .update(&self.payload);
        if block_checksum(&self.payload) != block.checksum {
            return corrupt(
                &self.revision.path,
                "Revision row source block checksum differs",
            );
        }
        self.payload_offset = 0;
        self.next_block += 1;
        Ok(())
    }

    fn finish_file(&mut self) -> Result<(), PersistenceError> {
        check_cancelled(self.control)?;
        if stream_position(&mut self.file, &self.revision.path)? != self.revision.facts.body_bytes {
            return corrupt(
                &self.revision.path,
                "Revision row source body length differs from validated facts",
            );
        }
        let mut footer = [0_u8; FOOTER_BYTES];
        read_exact(&mut self.file, &mut footer, &self.revision.path)?;
        let actual_digest = *self
            .file_hasher
            .take()
            .expect("Revision row source file digest is finalized once")
            .finalize()
            .as_bytes();
        if footer != encode_footer(self.revision.facts.body_bytes, actual_digest) {
            return corrupt(
                &self.revision.path,
                "Revision row source footer or whole-file digest differs",
            );
        }
        let actual_delta = *self
            .delta_hasher
            .take()
            .expect("Revision row source delta digest is finalized once")
            .finalize()
            .as_bytes();
        if actual_delta != self.revision.facts.delta_digest {
            return corrupt(
                &self.revision.path,
                "Revision row source delta digest differs from validated facts",
            );
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Store {
    root: PathBuf,
    operations: PathBuf,
    revisions: PathBuf,
    scratch: PathBuf,
    lock: Option<File>,
    remove_on_drop: bool,
}

impl Store {
    pub(crate) fn create(root: &Path) -> Result<Self, PersistenceError> {
        let resumed = create_or_recognize_incomplete_root(root)?;
        let mut lock = Some(acquire_lock(root)?);
        if resumed {
            validate_incomplete_root(root)?;
        }
        let operations = root.join("operations");
        let revisions = root.join("revisions");
        let scratch = root.join("scratch");
        let setup = (|| {
            create_directory_if_missing(&operations)?;
            create_directory_if_missing(&revisions)?;
            create_directory_if_missing(&scratch)?;
            sync_directory(root)?;
            Ok(())
        })();
        if let Err(error) = setup {
            let detached = detach_incomplete_root(root);
            drop(lock.take());
            if let Some(detached) = detached {
                let _ = fs::remove_dir_all(detached);
            }
            return Err(error);
        }
        let store = Self {
            root: root.to_path_buf(),
            operations,
            revisions,
            scratch,
            lock,
            remove_on_drop: true,
        };
        store.clean_recognized_scratch()?;
        Ok(store)
    }

    pub(crate) fn open(root: &Path) -> Result<Self, PersistenceError> {
        require_real_directory(root)?;
        let lock = acquire_lock(root)?;
        let operations = required_directory(root.join("operations"))?;
        let revisions = required_directory(root.join("revisions"))?;
        let scratch = required_directory(root.join("scratch"))?;
        Ok(Self {
            root: root.to_path_buf(),
            operations,
            revisions,
            scratch,
            lock: Some(lock),
            remove_on_drop: false,
        })
    }

    pub(crate) fn scratch(&self) -> &Path {
        &self.scratch
    }

    pub(crate) fn durable_payload_bytes(&self) -> Result<u64, PersistenceError> {
        let mut total = file_length(&self.root.join("manifest.pwm"))?;
        for entry in read_directory(&self.operations)? {
            let entry = entry
                .map_err(|source| io_error("read operation entry", &self.operations, source))?;
            charge(
                &mut total,
                file_length(&entry.path())?,
                u64::MAX,
                "durable bytes",
            )?;
        }
        Ok(total)
    }

    pub(crate) fn publish_manifest(
        &mut self,
        facts: &ManifestFacts,
    ) -> Result<(), PersistenceError> {
        let (scratch_path, mut scratch_file) = create_temporary(&self.scratch, "manifest", "tmp")?;
        let result = (|| {
            inject_fault(&self.root, FaultPoint::ManifestStage)?;
            let bytes = encode_manifest(facts);
            write_all(&mut scratch_file, &bytes, &scratch_path)?;
            sync_file(&scratch_file, &scratch_path)?;
            drop(scratch_file);
            make_read_only(&scratch_path)?;
            let manifest_path = self.root.join("manifest.pwm");
            inject_fault(&self.root, FaultPoint::ManifestLink)?;
            publish_link(&scratch_path, &manifest_path)?;
            inject_fault(&self.root, FaultPoint::ManifestDirectorySync)?;
            sync_directory(&self.root)?;
            inject_fault(&self.root, FaultPoint::ManifestParentDirectorySync)?;
            sync_directory(workspace_parent(&self.root))?;
            self.remove_on_drop = false;
            // Creation has no uncertainty result. Once the manifest directory
            // entry is synced, completion must win over a lost acknowledgement.
            let _ = inject_fault(&self.root, FaultPoint::ManifestLostAcknowledgement);
            // The parent sync is the create commit point. Scratch cleanup cannot
            // revoke a successfully created Workspace.
            let _ = remove_file(&scratch_path);
            let _ = sync_directory(&self.scratch);
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&scratch_path);
        }
        result
    }

    pub(crate) fn read_manifest(&self) -> Result<ManifestFacts, PersistenceError> {
        let path = self.root.join("manifest.pwm");
        let mut file = open_read(&path)?;
        ensure_exact_file_length(&file, MANIFEST_BYTES as u64, &path)?;
        let mut bytes = [0_u8; MANIFEST_BYTES];
        read_exact(&mut file, &mut bytes, &path)?;
        decode_manifest(&bytes, &path)
    }

    pub(crate) fn seal_candidate<I>(
        &self,
        facts: CandidateFacts,
        rows: I,
        limits: WriteLimits,
        control: &OperationControl,
    ) -> Result<SealedRevision, PersistenceError>
    where
        I: IntoIterator<Item = Result<RevisionRow, PersistenceError>>,
    {
        let (path, file) = create_temporary(&self.scratch, "revision", "tmp")?;
        let result = write_candidate_file(&path, file, facts, rows, limits, control);
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        result
    }

    pub(crate) fn prepare_ready(
        &self,
        sealed: &SealedRevision,
        max_path_bytes: u64,
    ) -> Result<Arc<ValidatedRevision>, PersistenceError> {
        let path_budget = max_path_bytes
            .checked_sub(arc_revision_fixed_bytes())
            .ok_or_else(|| {
                limit(
                    "ready target path",
                    arc_revision_fixed_bytes(),
                    max_path_bytes,
                )
            })?;
        preflight_child_path(
            &self.operations,
            OPERATION_ID_BYTES * 2 + ".ready".len(),
            path_budget,
            "ready target path",
        )?;
        let prepared = Arc::new(
            sealed
                .file
                .with_path(self.ready_path(sealed.file.facts.candidate.operation)),
        );
        ensure_at_most(
            u64::try_from(prepared.path.capacity())
                .unwrap_or(u64::MAX)
                .saturating_add(arc_revision_fixed_bytes()),
            max_path_bytes,
            "ready target path",
        )?;
        Ok(prepared)
    }

    #[cfg(test)]
    pub(crate) fn publish_ready<F>(
        &self,
        sealed: &SealedRevision,
        mark_publication_attempted: F,
    ) -> Result<Arc<ValidatedRevision>, PersistenceError>
    where
        F: FnOnce() -> Result<(), PersistenceError>,
    {
        let prepared = self.prepare_ready(sealed, u64::MAX)?;
        self.publish_prepared_ready(sealed, prepared, mark_publication_attempted)
    }

    pub(crate) fn publish_prepared_ready<F>(
        &self,
        sealed: &SealedRevision,
        prepared: Arc<ValidatedRevision>,
        mark_publication_attempted: F,
    ) -> Result<Arc<ValidatedRevision>, PersistenceError>
    where
        F: FnOnce() -> Result<(), PersistenceError>,
    {
        mark_publication_attempted()?;
        inject_fault(&self.root, FaultPoint::ReadyLink)?;
        publish_link(sealed.path(), &prepared.path)?;
        inject_fault(&self.root, FaultPoint::OperationsDirectorySync)?;
        sync_directory(&self.operations)?;
        inject_fault(&self.root, FaultPoint::OperationLostAcknowledgement)?;
        inject_fault(&self.root, FaultPoint::ReadyCleanup)?;
        remove_file(sealed.path())?;
        sync_directory(&self.scratch)?;
        Ok(prepared)
    }

    pub(crate) fn prepare_revision(
        &self,
        ready: &ValidatedRevision,
        max_path_bytes: u64,
    ) -> Result<Arc<ValidatedRevision>, PersistenceError> {
        let path_budget = max_path_bytes
            .checked_sub(arc_revision_fixed_bytes())
            .ok_or_else(|| {
                limit(
                    "Revision target path",
                    arc_revision_fixed_bytes(),
                    max_path_bytes,
                )
            })?;
        preflight_child_path(
            &self.revisions,
            20 + 1 + REVISION_ID_BYTES * 2 + ".pwr".len(),
            path_budget,
            "Revision target path",
        )?;
        let prepared =
            Arc::new(ready.with_path(
                self.revision_path(ready.facts.candidate.sequence, ready.facts.revision),
            ));
        ensure_at_most(
            u64::try_from(prepared.path.capacity())
                .unwrap_or(u64::MAX)
                .saturating_add(arc_revision_fixed_bytes()),
            max_path_bytes,
            "Revision target path",
        )?;
        Ok(prepared)
    }

    #[cfg(test)]
    pub(crate) fn publish_revision<F, G>(
        &self,
        ready: &ValidatedRevision,
        mark_publication_attempted: F,
        mark_directory_sync_attempted: G,
    ) -> Result<Arc<ValidatedRevision>, PersistenceError>
    where
        F: FnOnce() -> Result<(), PersistenceError>,
        G: FnOnce(),
    {
        let prepared = self.prepare_revision(ready, u64::MAX)?;
        self.publish_prepared_revision(
            ready,
            prepared,
            mark_publication_attempted,
            mark_directory_sync_attempted,
        )
    }

    pub(crate) fn publish_prepared_revision<F, G>(
        &self,
        ready: &ValidatedRevision,
        prepared: Arc<ValidatedRevision>,
        mark_publication_attempted: F,
        mark_directory_sync_attempted: G,
    ) -> Result<Arc<ValidatedRevision>, PersistenceError>
    where
        F: FnOnce() -> Result<(), PersistenceError>,
        G: FnOnce(),
    {
        mark_publication_attempted()?;
        inject_fault(&self.root, FaultPoint::RevisionLink)?;
        publish_link(&ready.path, &prepared.path)?;
        mark_directory_sync_attempted();
        inject_fault(&self.root, FaultPoint::RevisionDirectorySync)?;
        sync_directory(&self.revisions)?;
        inject_fault(&self.root, FaultPoint::RevisionLostAcknowledgement)?;
        Ok(prepared)
    }

    pub(crate) fn sync_revisions(&self) -> Result<(), PersistenceError> {
        inject_fault(&self.root, FaultPoint::RecoveryRevisionsSync)?;
        sync_directory(&self.revisions)
    }

    pub(crate) fn sync_operations(&self) -> Result<(), PersistenceError> {
        inject_fault(&self.root, FaultPoint::RecoveryOperationsSync)?;
        sync_directory(&self.operations)
    }

    pub(crate) fn sync_root(&self) -> Result<(), PersistenceError> {
        inject_fault(&self.root, FaultPoint::RecoveryRootSync)?;
        sync_directory(&self.root)
    }

    pub(crate) fn sync_parent(&self) -> Result<(), PersistenceError> {
        inject_fault(&self.root, FaultPoint::RecoveryParentSync)?;
        sync_directory(workspace_parent(&self.root))
    }

    pub(crate) fn prepare_rejection(
        &self,
        facts: RejectionFacts,
        max_working_bytes: u64,
    ) -> Result<PreparedRejection, PersistenceError> {
        let required = (REJECTION_BYTES as u64)
            .saturating_add(child_path_bytes(
                &self.scratch,
                "reject".len() + 1 + 32 + 1 + 3,
            ))
            .saturating_add(child_path_bytes(
                &self.operations,
                OPERATION_ID_BYTES * 2 + ".reject".len(),
            ));
        ensure_at_most(
            required,
            max_working_bytes,
            "rejection publication working bytes",
        )?;
        let (scratch_path, file) = create_temporary(&self.scratch, "reject", "tmp")?;
        let prepared = PreparedRejection {
            facts,
            scratch_path,
            target_path: self.rejection_path(facts.operation),
            file: Some(file),
            bytes: encode_rejection(&facts),
            cleanup_scratch: true,
        };
        ensure_at_most(
            prepared.working_bytes(),
            max_working_bytes,
            "rejection publication working bytes",
        )?;
        Ok(prepared)
    }

    #[cfg(test)]
    pub(crate) fn publish_rejection<F>(
        &self,
        facts: RejectionFacts,
        mark_publication_attempted: F,
    ) -> Result<(), PersistenceError>
    where
        F: FnOnce() -> Result<(), PersistenceError>,
    {
        let prepared = self.prepare_rejection(facts, u64::MAX)?;
        self.publish_prepared_rejection(prepared, mark_publication_attempted)
    }

    pub(crate) fn publish_prepared_rejection<F>(
        &self,
        mut prepared: PreparedRejection,
        mark_publication_attempted: F,
    ) -> Result<(), PersistenceError>
    where
        F: FnOnce() -> Result<(), PersistenceError>,
    {
        let mut scratch_file = prepared
            .file
            .take()
            .expect("prepared rejection retains its writable stage");
        (|| {
            inject_fault(&self.root, FaultPoint::RejectionStage)?;
            write_all(&mut scratch_file, &prepared.bytes, &prepared.scratch_path)?;
            inject_fault(&self.root, FaultPoint::RejectionFileSync)?;
            sync_file(&scratch_file, &prepared.scratch_path)?;
            drop(scratch_file);
            inject_fault(&self.root, FaultPoint::RejectionReadOnly)?;
            make_read_only(&prepared.scratch_path)?;
            inject_fault(&self.root, FaultPoint::RejectionRevalidate)?;
            validate_rejection_file(&prepared.scratch_path, Some(prepared.facts.operation))?;
            mark_publication_attempted()?;
            inject_fault(&self.root, FaultPoint::RejectionLink)?;
            publish_link(&prepared.scratch_path, &prepared.target_path)?;
            inject_fault(&self.root, FaultPoint::RejectionDirectorySync)?;
            sync_directory(&self.operations)?;
            inject_fault(&self.root, FaultPoint::RejectionLostAcknowledgement)?;
            inject_fault(&self.root, FaultPoint::RejectionCleanup)?;
            remove_file(&prepared.scratch_path)?;
            prepared.cleanup_scratch = false;
            sync_directory(&self.scratch)
        })()
    }

    pub(crate) fn recover(
        &self,
        manifest: &ManifestFacts,
        limits: CatalogLimits,
        control: &OperationControl,
    ) -> Result<Catalog, PersistenceError> {
        self.clean_recognized_scratch()?;
        let mut ledger = RecoveryLedger::default();
        let revisions = self.read_revisions(manifest, limits, &mut ledger, control)?;
        let operations =
            self.read_operations(manifest, &revisions, limits, &mut ledger, control)?;
        validate_catalog_semantics(
            manifest,
            &revisions,
            &operations,
            limits,
            &mut ledger,
            control,
        )?;
        Ok(Catalog {
            root_revision: manifest.root_revision,
            revisions,
            operations,
        })
    }

    fn ready_path(&self, operation: OperationBytes) -> PathBuf {
        self.operations
            .join(format!("{}.ready", encode_hex(&operation)))
    }

    fn rejection_path(&self, operation: OperationBytes) -> PathBuf {
        self.operations
            .join(format!("{}.reject", encode_hex(&operation)))
    }

    fn revision_path(&self, sequence: u64, revision: RevisionBytes) -> PathBuf {
        self.revisions
            .join(format!("{sequence:020}-{}.pwr", encode_hex(&revision)))
    }

    fn clean_recognized_scratch(&self) -> Result<(), PersistenceError> {
        let mut removed_any = false;
        for entry in read_directory(&self.scratch)? {
            let entry =
                entry.map_err(|source| io_error("read scratch entry", &self.scratch, source))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if is_recognized_scratch(&name) {
                let file_type = entry
                    .file_type()
                    .map_err(|source| io_error("inspect scratch entry", &self.scratch, source))?;
                if !file_type.is_file() || file_type.is_symlink() {
                    return corrupt(
                        &entry.path(),
                        "recognized scratch entry is not a private real file",
                    );
                }
                remove_file(&entry.path())?;
                removed_any = true;
            }
        }
        if removed_any {
            sync_directory(&self.scratch)?;
        }
        Ok(())
    }

    fn revision_entries(
        &self,
        limits: CatalogLimits,
    ) -> Result<(Vec<RevisionEntry>, u64), PersistenceError> {
        let mut entries = Vec::new();
        let mut path_bytes = 0_u64;
        for entry in read_directory(&self.revisions)? {
            let entry =
                entry.map_err(|source| io_error("read revision entry", &self.revisions, source))?;
            let file_type = entry
                .file_type()
                .map_err(|source| io_error("inspect revision entry", &self.revisions, source))?;
            if !file_type.is_file() || file_type.is_symlink() {
                return corrupt(
                    &self.revisions,
                    "published Revision entry is not a real file",
                );
            }
            let name = entry.file_name();
            let Some(name_text) = name.to_str() else {
                return corrupt(&self.revisions, "published Revision filename is not UTF-8");
            };
            let (sequence, revision) =
                parse_revision_name(name_text).ok_or_else(|| PersistenceError::Corrupt {
                    path: self.revisions.clone(),
                    reason: "unrecognized published Revision filename",
                })?;
            let next_count = u64::try_from(entries.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            ensure_at_most(next_count, limits.max_revisions, "Revision count")?;
            let retained_bytes =
                path_bytes.saturating_add(vector_bytes::<RevisionEntry>(entries.capacity()));
            let path = bounded_directory_child_path(
                &self.revisions,
                &name,
                retained_bytes,
                limits.read.max_working_bytes,
                "Revision path working metadata",
            )?;
            drop(name);
            path_bytes = path_bytes
                .checked_add(u64::try_from(path.capacity()).unwrap_or(u64::MAX))
                .ok_or_else(|| {
                    limit(
                        "Revision path working metadata",
                        u64::MAX,
                        limits.read.max_working_bytes,
                    )
                })?;
            let remaining = limits
                .read
                .max_working_bytes
                .checked_sub(path_bytes)
                .ok_or_else(|| {
                    limit(
                        "Revision path working metadata",
                        path_bytes,
                        limits.read.max_working_bytes,
                    )
                })?;
            reserve_vec_transition(&mut entries, 1, remaining, "Revision path working metadata")?;
            entries.push((sequence, revision, path));
        }
        entries.sort_unstable_by_key(|(sequence, _, _)| *sequence);
        let working_bytes =
            path_bytes.saturating_add(vector_bytes::<RevisionEntry>(entries.capacity()));
        Ok((entries, working_bytes))
    }

    fn operation_entries(
        &self,
        limits: CatalogLimits,
    ) -> Result<(Vec<OperationEntry>, u64), PersistenceError> {
        let mut entries = Vec::new();
        let mut path_bytes = 0_u64;
        for entry in read_directory(&self.operations)? {
            let entry = entry
                .map_err(|source| io_error("read operation entry", &self.operations, source))?;
            let file_type = entry
                .file_type()
                .map_err(|source| io_error("inspect operation entry", &self.operations, source))?;
            if !file_type.is_file() || file_type.is_symlink() {
                return corrupt(
                    &self.operations,
                    "published Operation entry is not a real file",
                );
            }
            let name = entry.file_name();
            let Some(name_text) = name.to_str() else {
                return corrupt(
                    &self.operations,
                    "published Operation filename is not UTF-8",
                );
            };
            let (operation, kind) =
                parse_operation_name(name_text).ok_or_else(|| PersistenceError::Corrupt {
                    path: self.operations.clone(),
                    reason: "unrecognized published Operation filename",
                })?;
            if operation == [0; OPERATION_ID_BYTES] {
                return corrupt(
                    &self.operations,
                    "Operation filename uses the reserved zero identity",
                );
            }
            let next_count = u64::try_from(entries.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            ensure_at_most(
                next_count,
                limits.max_operation_files,
                "Operation file count",
            )?;
            let retained_bytes =
                path_bytes.saturating_add(vector_bytes::<OperationEntry>(entries.capacity()));
            let path = bounded_directory_child_path(
                &self.operations,
                &name,
                retained_bytes,
                limits.read.max_working_bytes,
                "Operation path working metadata",
            )?;
            drop(name);
            path_bytes = path_bytes
                .checked_add(u64::try_from(path.capacity()).unwrap_or(u64::MAX))
                .ok_or_else(|| {
                    limit(
                        "Operation path working metadata",
                        u64::MAX,
                        limits.read.max_working_bytes,
                    )
                })?;
            let remaining = limits
                .read
                .max_working_bytes
                .checked_sub(path_bytes)
                .ok_or_else(|| {
                    limit(
                        "Operation path working metadata",
                        path_bytes,
                        limits.read.max_working_bytes,
                    )
                })?;
            reserve_vec_transition(
                &mut entries,
                1,
                remaining,
                "Operation path working metadata",
            )?;
            entries.push((operation, kind, path));
        }
        let working_bytes =
            path_bytes.saturating_add(vector_bytes::<OperationEntry>(entries.capacity()));
        Ok((entries, working_bytes))
    }

    fn read_revisions(
        &self,
        manifest: &ManifestFacts,
        limits: CatalogLimits,
        ledger: &mut RecoveryLedger,
        control: &OperationControl,
    ) -> Result<Vec<Arc<ValidatedRevision>>, PersistenceError> {
        let (named_paths, directory_working_bytes) = self.revision_entries(limits)?;

        let mut expected_parent = manifest.root_revision;
        let mut seen_operations = HashSet::new();
        ensure_at_most(
            directory_working_bytes
                .saturating_add(hash_table_bytes::<OperationBytes>(named_paths.len())),
            limits.read.max_working_bytes,
            "Revision recovery working metadata",
        )?;
        seen_operations
            .try_reserve(named_paths.len())
            .map_err(|_| {
                limit(
                    "Revision lineage working metadata",
                    u64::try_from(named_paths.len()).unwrap_or(u64::MAX),
                    limits.max_revisions,
                )
            })?;
        let recovery_working_bytes =
            directory_working_bytes.saturating_add(hash_table_bytes::<OperationBytes>(
                seen_operations.capacity(),
            ));
        ensure_at_most(
            recovery_working_bytes,
            limits.read.max_working_bytes,
            "Revision recovery working metadata",
        )?;
        let mut revisions = Vec::new();
        let catalog_growth_bytes = limits
            .read
            .max_working_bytes
            .checked_sub(recovery_working_bytes)
            .ok_or_else(|| {
                limit(
                    "Revision recovery catalog growth",
                    recovery_working_bytes,
                    limits.read.max_working_bytes,
                )
            })?;
        ledger.preflight_metadata(
            vector_bytes::<Arc<ValidatedRevision>>(named_paths.len()),
            limits,
        )?;
        reserve_vec_transition(
            &mut revisions,
            named_paths.len(),
            catalog_growth_bytes,
            "Revision recovery catalog growth",
        )?;
        ledger.charge_metadata(
            vector_bytes::<Arc<ValidatedRevision>>(revisions.capacity()),
            limits,
        )?;
        for (index, (named_sequence, named_revision, path)) in named_paths.into_iter().enumerate() {
            check_cancelled(control)?;
            let expected_sequence = u64::try_from(index)
                .map_err(|_| limit("Revision count", u64::MAX, limits.max_revisions))?
                .saturating_add(1);
            if named_sequence != expected_sequence {
                return corrupt(&path, "Revision sequence has a gap or fork");
            }
            let bytes = file_length(&path)?;
            ledger.charge_durable(bytes, limits)?;
            ledger.charge_metadata(revision_base_metadata_bytes(&path), limits)?;
            let revision = validate_revision_file(
                &path,
                ledger.read_limits(limits, true, recovery_working_bytes)?,
                Some(control),
            )?;
            revision.validate_source_point_count(manifest.source_point_count)?;
            ledger.charge_revision_scan(&revision, true, limits)?;
            if revision.facts.candidate.workspace != manifest.workspace
                || revision.facts.candidate.source != manifest.source
                || revision.facts.candidate.source_contract != manifest.source_contract
            {
                return corrupt(&path, "Revision belongs to another Workspace or Source");
            }
            validate_candidate_request(manifest, &revision.facts.candidate, &path)?;
            if revision.facts.candidate.sequence != expected_sequence
                || revision.facts.candidate.parent != expected_parent
                || revision.facts.revision != named_revision
            {
                return corrupt(
                    &path,
                    "Revision lineage facts differ from its path or predecessor",
                );
            }
            if !seen_operations.insert(revision.facts.candidate.operation) {
                return corrupt(
                    &path,
                    "Operation Identity appears in more than one Revision",
                );
            }
            expected_parent = revision.facts.revision;
            revisions.push(Arc::new(revision));
        }
        Ok(revisions)
    }

    fn read_operations(
        &self,
        manifest: &ManifestFacts,
        revisions: &[Arc<ValidatedRevision>],
        limits: CatalogLimits,
        ledger: &mut RecoveryLedger,
        control: &OperationControl,
    ) -> Result<Vec<(OperationBytes, OperationRecord)>, PersistenceError> {
        let (files, recovery_working_bytes) = self.operation_entries(limits)?;

        let catalog_growth_bytes = limits
            .read
            .max_working_bytes
            .checked_sub(recovery_working_bytes)
            .ok_or_else(|| {
                limit(
                    "Operation recovery working metadata",
                    recovery_working_bytes,
                    limits.read.max_working_bytes,
                )
            })?;
        let mut operations = Vec::<(OperationBytes, OperationRecord)>::new();
        ledger.preflight_metadata(
            vector_bytes::<(OperationBytes, OperationRecord)>(files.len()),
            limits,
        )?;
        reserve_vec_transition(
            &mut operations,
            files.len(),
            catalog_growth_bytes,
            "Operation recovery catalog growth",
        )?;
        ledger.charge_metadata(
            vector_bytes::<(OperationBytes, OperationRecord)>(operations.capacity()),
            limits,
        )?;
        for (operation, kind, path) in files {
            check_cancelled(control)?;
            let committed = revisions
                .iter()
                .find(|revision| revision.facts.candidate.operation == operation);
            if !matches!(kind, OperationFileKind::Ready) || committed.is_none() {
                ledger.charge_durable(file_length(&path)?, limits)?;
            }
            let record_index = operations
                .iter()
                .position(|(existing, _)| *existing == operation);
            let record_index = if let Some(index) = record_index {
                index
            } else {
                operations.push((operation, OperationRecord::default()));
                operations.len() - 1
            };
            let record = &mut operations[record_index].1;
            match kind {
                OperationFileKind::Ready => {
                    if record.ready.is_some() {
                        return corrupt(&path, "duplicate ready Operation file");
                    }
                    let shares_blocks = committed.is_some();
                    ledger.charge_metadata(revision_base_metadata_bytes(&path), limits)?;
                    let mut ready = validate_revision_file(
                        &path,
                        ledger.read_limits(limits, !shares_blocks, recovery_working_bytes)?,
                        Some(control),
                    )?;
                    ready.validate_source_point_count(manifest.source_point_count)?;
                    if ready.facts.candidate.operation != operation
                        || ready.facts.candidate.workspace != manifest.workspace
                        || ready.facts.candidate.source != manifest.source
                        || ready.facts.candidate.source_contract != manifest.source_contract
                    {
                        return corrupt(&path, "ready payload identity facts differ");
                    }
                    validate_candidate_request(manifest, &ready.facts.candidate, &path)?;
                    ledger.charge_revision_scan(&ready, !shares_blocks, limits)?;
                    if let Some(committed) = committed {
                        if ready.facts != committed.facts {
                            return corrupt(
                                &path,
                                "ready payload differs from its committed Revision",
                            );
                        }
                        ready.blocks = Arc::clone(&committed.blocks);
                    }
                    record.ready = Some(Arc::new(ready));
                }
                OperationFileKind::Reject => {
                    if record.rejection.is_some() {
                        return corrupt(&path, "duplicate rejection Operation file");
                    }
                    ensure_at_most(
                        REJECTION_BYTES as u64,
                        limits.read.max_file_bytes,
                        "rejection file bytes",
                    )?;
                    let rejection = validate_rejection_file(&path, Some(operation))?;
                    if rejection.workspace != manifest.workspace {
                        return corrupt(&path, "rejection belongs to another Workspace");
                    }
                    record.rejection = Some(rejection);
                }
            }
        }
        Ok(operations)
    }
}

fn validate_candidate_request(
    manifest: &ManifestFacts,
    candidate: &CandidateFacts,
    path: &Path,
) -> Result<(), PersistenceError> {
    if candidate.operation == [0; OPERATION_ID_BYTES] {
        return corrupt(path, "Revision has the reserved zero Operation Identity");
    }
    let expected = match (candidate.kind, candidate.point_set) {
        (RevisionKind::SetClassification(value), Some(point_set)) => {
            classification_request_digest(ClassificationRequestFacts {
                workspace: candidate.workspace,
                source: candidate.source,
                classification_attribute: manifest.classification.id,
                point_set_workspace: candidate.workspace,
                point_set_source: candidate.source,
                parent: candidate.parent,
                point_set,
                value,
            })
        }
        (RevisionKind::SetClassification(_), None) => {
            return corrupt(path, "classification request has no Point Set provenance");
        }
        (RevisionKind::Revert, None) => {
            revert_request_digest(candidate.workspace, candidate.source, candidate.parent)
        }
        (RevisionKind::Revert, Some(_)) => {
            return corrupt(path, "Revert request unexpectedly has Point Set provenance");
        }
    };
    if candidate.request_digest != expected {
        return corrupt(path, "Revision request digest is not canonical");
    }
    Ok(())
}

impl Drop for Store {
    fn drop(&mut self) {
        if !self.remove_on_drop {
            return;
        }
        // Detach the exact create-new tree while its advisory lock is still
        // held. A concurrent create can then use the original pathname and
        // cannot be deleted by cleanup of this detached inode tree.
        let detached = detach_incomplete_root(&self.root);
        drop(self.lock.take());
        if let Some(detached) = detached {
            let _ = fs::remove_dir_all(detached);
        }
    }
}

fn detach_incomplete_root(root: &Path) -> Option<PathBuf> {
    static NEXT_ABORT: AtomicU64 = AtomicU64::new(0);
    let parent = root.parent()?;
    let name = root.file_name()?.to_string_lossy();
    for _ in 0..64 {
        let sequence = NEXT_ABORT.fetch_add(1, Ordering::Relaxed);
        let target = parent.join(format!(
            ".{name}.punctra-aborted-{}-{sequence:016x}",
            std::process::id()
        ));
        match fs::rename(root, &target) {
            Ok(()) => return Some(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
    }
    None
}

#[derive(Clone, Copy)]
enum OperationFileKind {
    Ready,
    Reject,
}

#[allow(clippy::too_many_lines)]
fn write_candidate_file<I>(
    path: &Path,
    mut file: File,
    facts: CandidateFacts,
    rows: I,
    limits: WriteLimits,
    control: &OperationControl,
) -> Result<SealedRevision, PersistenceError>
where
    I: IntoIterator<Item = Result<RevisionRow, PersistenceError>>,
{
    let mut rows = rows.into_iter();
    let Some(first_row) = rows.next() else {
        return Err(PersistenceError::NoRows);
    };
    let first_row = first_row?;
    validate_write_limits(limits)?;
    ensure_at_most(
        REVISION_HEADER_BYTES as u64 + FOOTER_BYTES as u64,
        limits.max_file_bytes,
        "Revision bytes",
    )?;
    ensure_at_most(
        REVISION_HEADER_BYTES as u64 + FOOTER_BYTES as u64,
        limits.max_temporary_bytes,
        "commit temporary bytes",
    )?;
    write_all(&mut file, &[0_u8; REVISION_HEADER_BYTES], path)?;

    let rows_per_block = usize::try_from(limits.rows_per_block).map_err(|_| {
        limit(
            "rows per block",
            u64::from(limits.rows_per_block),
            usize::MAX as u64,
        )
    })?;
    let block_capacity = rows_per_block.checked_mul(ROW_BYTES).ok_or_else(|| {
        limit(
            "Revision block allocation",
            u64::MAX,
            limits.max_working_bytes,
        )
    })?;
    ensure_at_most(
        u64::try_from(block_capacity)
            .unwrap_or(u64::MAX)
            .saturating_add(limits.retained_input_bytes),
        limits.max_working_bytes,
        "combined commit working bytes",
    )?;
    let mut block_payload = Vec::new();
    block_payload
        .try_reserve_exact(block_capacity)
        .map_err(|_| {
            limit(
                "Revision block allocation",
                block_capacity as u64,
                limits.max_working_bytes,
            )
        })?;
    ensure_at_most(
        u64::try_from(block_payload.capacity())
            .unwrap_or(u64::MAX)
            .saturating_add(limits.retained_input_bytes),
        limits.max_working_bytes,
        "combined commit working bytes",
    )?;
    let mut row_count = 0_u64;
    let mut block_count = 0_u64;
    let mut last_ordinal = None;
    let mut delta_hasher = Hasher::new();
    delta_hasher.update(DELTA_DOMAIN);

    for row in std::iter::once(Ok(first_row)).chain(rows) {
        check_cancelled(control)?;
        let row = row?;
        validate_next_row(row, last_ordinal, path)?;
        last_ordinal = Some(row.ordinal);
        row_count = row_count
            .checked_add(1)
            .ok_or_else(|| limit("Revision rows", u64::MAX, limits.max_rows))?;
        ensure_at_most(row_count, limits.max_rows, "Revision rows")?;
        encode_row(row, &mut block_payload);
        if block_payload.len() == rows_per_block.saturating_mul(ROW_BYTES) {
            flush_block(
                &mut file,
                path,
                &mut block_payload,
                &mut block_count,
                &mut delta_hasher,
                limits,
            )?;
        }
    }
    if !block_payload.is_empty() {
        flush_block(
            &mut file,
            path,
            &mut block_payload,
            &mut block_count,
            &mut delta_hasher,
            limits,
        )?;
    }
    drop(block_payload);
    let body_bytes = stream_position(&mut file, path)?;
    ensure_at_most(
        body_bytes.saturating_add(FOOTER_BYTES as u64),
        limits.max_file_bytes,
        "Revision bytes",
    )?;
    let delta_digest = *delta_hasher.finalize().as_bytes();
    let revision = derive_revision_id(&facts, delta_digest);
    let revision_facts = RevisionFacts {
        candidate: facts,
        revision,
        row_count,
        block_count,
        delta_digest,
        body_bytes,
    };
    let header = encode_revision_header(&revision_facts);
    seek(&mut file, 0, path)?;
    write_all(&mut file, &header, path)?;
    let file_digest = hash_file_prefix(
        &mut file,
        body_bytes,
        limits.max_working_bytes,
        path,
        Some(control),
    )?;
    seek(&mut file, body_bytes, path)?;
    write_all(&mut file, &encode_footer(body_bytes, file_digest), path)?;
    inject_fault(path, FaultPoint::CandidateStage)?;
    inject_fault(path, FaultPoint::CandidateFileSync)?;
    sync_file(&file, path)?;
    inject_fault(path, FaultPoint::CandidateClose)?;
    drop(file);
    inject_fault(path, FaultPoint::CandidateReadOnly)?;
    make_read_only(path)?;

    let read_limits = ReadLimits {
        max_file_bytes: limits.max_file_bytes,
        max_rows: limits.max_rows,
        max_blocks: limits.max_blocks,
        max_block_bytes: limits.max_block_bytes,
        max_working_bytes: limits.max_working_bytes,
    };
    inject_fault(path, FaultPoint::CandidateRevalidate)?;
    let validated = validate_revision_file(path, read_limits, Some(control))?;
    Ok(SealedRevision { file: validated })
}

fn revision_base_metadata_bytes(path: &PathBuf) -> u64 {
    arc_revision_fixed_bytes().saturating_add(u64::try_from(path.capacity()).unwrap_or(u64::MAX))
}

fn arc_revision_fixed_bytes() -> u64 {
    u64::try_from(
        std::mem::size_of::<ValidatedRevision>().saturating_add(2 * std::mem::size_of::<usize>()),
    )
    .unwrap_or(u64::MAX)
}

fn arc_block_vec_fixed_bytes() -> u64 {
    u64::try_from(
        std::mem::size_of::<Vec<BlockMetadata>>().saturating_add(2 * std::mem::size_of::<usize>()),
    )
    .unwrap_or(u64::MAX)
}

fn path_encoded_bytes(path: &Path) -> u64 {
    u64::try_from(path.as_os_str().as_encoded_bytes().len()).unwrap_or(u64::MAX)
}

fn preflight_child_path(
    directory: &Path,
    child_name_bytes: usize,
    allowed: u64,
    resource: &'static str,
) -> Result<(), PersistenceError> {
    ensure_at_most(
        child_path_bytes(directory, child_name_bytes),
        allowed,
        resource,
    )
}

fn child_path_bytes(directory: &Path, child_name_bytes: usize) -> u64 {
    path_encoded_bytes(directory)
        .saturating_add(1)
        .saturating_add(u64::try_from(child_name_bytes).unwrap_or(u64::MAX))
}

fn bounded_directory_child_path(
    directory: &Path,
    child_name: &std::ffi::OsStr,
    retained_bytes: u64,
    max_working_bytes: u64,
    resource: &'static str,
) -> Result<PathBuf, PersistenceError> {
    let name_bytes = u64::try_from(child_name.as_encoded_bytes().len()).unwrap_or(u64::MAX);
    let requested_path_bytes = child_path_bytes(directory, child_name.as_encoded_bytes().len());
    ensure_at_most(
        retained_bytes
            .saturating_add(name_bytes)
            .saturating_add(requested_path_bytes),
        max_working_bytes,
        resource,
    )?;
    let path = directory.join(Path::new(child_name));
    ensure_at_most(
        retained_bytes
            .saturating_add(name_bytes)
            .saturating_add(u64::try_from(path.capacity()).unwrap_or(u64::MAX)),
        max_working_bytes,
        resource,
    )?;
    Ok(path)
}

fn vector_bytes<T>(capacity: usize) -> u64 {
    allocation_bytes::<T>(capacity)
}

fn reserve_vec_transition<T>(
    values: &mut Vec<T>,
    additional: usize,
    max_transition_bytes: u64,
    resource: &'static str,
) -> Result<u64, PersistenceError> {
    let required_len = values
        .len()
        .checked_add(additional)
        .ok_or_else(|| limit(resource, u64::MAX, max_transition_bytes))?;
    if required_len <= values.capacity() {
        return Ok(0);
    }
    let old_bytes = vector_bytes::<T>(values.capacity());
    ensure_at_most(
        old_bytes.saturating_add(vector_bytes::<T>(required_len)),
        max_transition_bytes,
        resource,
    )?;
    values
        .try_reserve_exact(additional)
        .map_err(|_| limit(resource, u64::MAX, max_transition_bytes))?;
    ensure_at_most(
        old_bytes.saturating_add(vector_bytes::<T>(values.capacity())),
        max_transition_bytes,
        resource,
    )?;
    Ok(vector_bytes::<T>(values.capacity()).saturating_sub(old_bytes))
}

fn hash_table_bytes<T>(capacity: usize) -> u64 {
    // Hash-table control bytes and load-factor slack vary by implementation;
    // two entries' worth per slot is a conservative retained estimate.
    vector_bytes::<T>(capacity).saturating_mul(2)
}

fn flush_block(
    file: &mut File,
    path: &Path,
    payload: &mut Vec<u8>,
    block_count: &mut u64,
    delta_hasher: &mut Hasher,
    limits: WriteLimits,
) -> Result<(), PersistenceError> {
    let payload_bytes = u64::try_from(payload.len()).unwrap_or(u64::MAX);
    ensure_at_most(
        payload_bytes,
        limits.max_block_bytes,
        "Revision block bytes",
    )?;
    let row_count = u32::try_from(payload.len() / ROW_BYTES)
        .map_err(|_| limit("rows per block", u64::MAX, u64::from(limits.rows_per_block)))?;
    let first_ordinal = decode_ordinal(&payload[..ROW_BYTES]);
    let last_start = payload.len() - ROW_BYTES;
    let last_ordinal = decode_ordinal(&payload[last_start..]);
    let checksum = block_checksum(payload);
    let header = encode_block_header(
        row_count,
        first_ordinal,
        last_ordinal,
        payload_bytes,
        checksum,
    );
    let current = stream_position(file, path)?;
    let projected = current
        .checked_add(BLOCK_HEADER_BYTES as u64)
        .and_then(|bytes| bytes.checked_add(payload_bytes))
        .and_then(|bytes| bytes.checked_add(FOOTER_BYTES as u64))
        .ok_or_else(|| limit("Revision bytes", u64::MAX, limits.max_file_bytes))?;
    ensure_at_most(projected, limits.max_file_bytes, "Revision bytes")?;
    ensure_at_most(
        projected,
        limits.max_temporary_bytes,
        "commit temporary bytes",
    )?;
    *block_count = block_count
        .checked_add(1)
        .ok_or_else(|| limit("Revision blocks", u64::MAX, limits.max_blocks))?;
    ensure_at_most(*block_count, limits.max_blocks, "Revision blocks")?;
    write_all(file, &header, path)?;
    write_all(file, payload, path)?;
    delta_hasher.update(payload);
    payload.clear();
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_revision_file(
    path: &Path,
    limits: ReadLimits,
    control: Option<&OperationControl>,
) -> Result<ValidatedRevision, PersistenceError> {
    let mut file = open_read(path)?;
    let file_bytes = file
        .metadata()
        .map_err(|source| io_error("read metadata", path, source))?
        .len();
    ensure_at_most(file_bytes, limits.max_file_bytes, "Revision bytes")?;
    if file_bytes < (REVISION_HEADER_BYTES + FOOTER_BYTES) as u64 {
        return corrupt(path, "Revision file is truncated before its footer");
    }

    let footer_offset = file_bytes - FOOTER_BYTES as u64;
    seek(&mut file, footer_offset, path)?;
    let mut footer = [0_u8; FOOTER_BYTES];
    read_exact(&mut file, &mut footer, path)?;
    let (footer_body_bytes, footer_digest) = decode_footer(&footer, path)?;
    if footer_body_bytes != footer_offset {
        return corrupt(path, "Revision footer length differs from the file length");
    }
    let actual_file_digest = hash_file_prefix(
        &mut file,
        footer_body_bytes,
        limits.max_working_bytes,
        path,
        control,
    )?;
    if actual_file_digest != footer_digest {
        return corrupt(path, "Revision whole-file digest differs");
    }
    seek(&mut file, 0, path)?;

    let mut header = [0_u8; REVISION_HEADER_BYTES];
    read_exact(&mut file, &mut header, path)?;
    let facts = decode_revision_header(&header, path)?;
    if facts.row_count == 0 || facts.block_count == 0 {
        return corrupt(path, "Revision has no changed rows");
    }
    ensure_at_most(facts.row_count, limits.max_rows, "Revision rows")?;
    ensure_at_most(facts.block_count, limits.max_blocks, "Revision blocks")?;
    if facts.body_bytes.saturating_add(FOOTER_BYTES as u64) != file_bytes {
        return corrupt(path, "Revision header length differs from the file length");
    }

    let mut delta_hasher = Hasher::new();
    delta_hasher.update(DELTA_DOMAIN);
    let block_count = to_usize(facts.block_count, path)?;
    let requested_metadata_bytes =
        u64::try_from(block_count.saturating_mul(std::mem::size_of::<BlockMetadata>()))
            .unwrap_or(u64::MAX);
    ensure_at_most(
        requested_metadata_bytes,
        limits.max_working_bytes,
        "Revision validation metadata",
    )?;
    let mut blocks = Vec::new();
    blocks.try_reserve_exact(block_count).map_err(|_| {
        limit(
            "Revision validation metadata",
            requested_metadata_bytes,
            limits.max_working_bytes,
        )
    })?;
    let metadata_bytes = vector_bytes::<BlockMetadata>(blocks.capacity());
    ensure_at_most(
        metadata_bytes,
        limits.max_working_bytes,
        "Revision validation metadata",
    )?;
    let payload_working_bytes = limits.max_working_bytes.saturating_sub(metadata_bytes);
    let mut observed_rows = 0_u64;
    let mut previous_ordinal = None;
    for _ in 0..facts.block_count {
        check_optional_cancelled(control)?;
        let mut block_header = [0_u8; BLOCK_HEADER_BYTES];
        read_exact(&mut file, &mut block_header, path)?;
        let decoded = decode_block_header(&block_header, path)?;
        ensure_at_most(
            decoded.payload_bytes,
            limits.max_block_bytes,
            "Revision block bytes",
        )?;
        let expected_payload = u64::from(decoded.row_count)
            .checked_mul(ROW_BYTES as u64)
            .ok_or_else(|| corrupt_value(path, "Revision block payload size overflows"))?;
        if decoded.row_count == 0 || decoded.payload_bytes != expected_payload {
            return corrupt(path, "Revision block row count and payload length differ");
        }
        let payload_offset = stream_position(&mut file, path)?;
        let mut payload = allocate_zeroed(
            decoded.payload_bytes,
            payload_working_bytes,
            "Revision validation buffer",
            path,
        )?;
        read_exact(&mut file, &mut payload, path)?;
        if block_checksum(&payload) != decoded.checksum {
            return corrupt(path, "Revision block checksum differs");
        }
        validate_payload_rows(
            &payload,
            decoded.row_count,
            decoded.first_ordinal,
            decoded.last_ordinal,
            &mut previous_ordinal,
            facts.candidate.kind,
            path,
        )?;
        delta_hasher.update(&payload);
        observed_rows = observed_rows
            .checked_add(u64::from(decoded.row_count))
            .ok_or_else(|| corrupt_value(path, "Revision row count overflows"))?;
        blocks.push(BlockMetadata {
            payload_offset,
            payload_bytes: decoded.payload_bytes,
            row_count: decoded.row_count,
            first_ordinal: decoded.first_ordinal,
            last_ordinal: decoded.last_ordinal,
            checksum: decoded.checksum,
        });
    }
    if stream_position(&mut file, path)? != facts.body_bytes {
        return corrupt(path, "Revision body length differs from its block sequence");
    }
    if observed_rows != facts.row_count {
        return corrupt(path, "Revision header row count differs from its blocks");
    }
    match (facts.candidate.kind, facts.candidate.point_set) {
        (RevisionKind::SetClassification(_), Some(point_set))
            if facts.row_count <= point_set.exact_count => {}
        (RevisionKind::SetClassification(_), Some(_)) => {
            return corrupt(
                path,
                "Revision changes more Points than its Point Set contains",
            );
        }
        (RevisionKind::SetClassification(_), None) => {
            return corrupt(path, "classification Revision has no Point Set provenance");
        }
        (RevisionKind::Revert, None) => {}
        (RevisionKind::Revert, Some(_)) => {
            return corrupt(
                path,
                "Revert Revision unexpectedly has Point Set provenance",
            );
        }
    }
    let delta_digest = *delta_hasher.finalize().as_bytes();
    if delta_digest != facts.delta_digest {
        return corrupt(path, "Revision delta digest differs");
    }
    if derive_revision_id(&facts.candidate, delta_digest) != facts.revision {
        return corrupt(path, "Revision Identity differs from canonical facts");
    }

    if footer_body_bytes != facts.body_bytes {
        return corrupt(path, "Revision footer and header lengths differ");
    }
    Ok(ValidatedRevision {
        path: path.to_path_buf(),
        facts,
        blocks: Arc::new(blocks),
    })
}

#[derive(Clone, Copy)]
struct DecodedBlockHeader {
    row_count: u32,
    first_ordinal: u64,
    last_ordinal: u64,
    payload_bytes: u64,
    checksum: DigestBytes,
}

fn validate_payload_rows(
    payload: &[u8],
    row_count: u32,
    expected_first: u64,
    expected_last: u64,
    previous_ordinal: &mut Option<u64>,
    kind: RevisionKind,
    path: &Path,
) -> Result<(), PersistenceError> {
    let mut first = None;
    let mut last = None;
    for row in payload.chunks_exact(ROW_BYTES).take(row_count as usize) {
        let decoded = decode_row(row);
        validate_next_row(decoded, *previous_ordinal, path)?;
        if decoded.before == decoded.after {
            return corrupt(path, "Revision contains a no-op row");
        }
        if let RevisionKind::SetClassification(value) = kind
            && decoded.after != value
        {
            return corrupt(
                path,
                "classification Revision row differs from its requested value",
            );
        }
        *previous_ordinal = Some(decoded.ordinal);
        first.get_or_insert(decoded.ordinal);
        last = Some(decoded.ordinal);
    }
    if first != Some(expected_first) || last != Some(expected_last) {
        return corrupt(path, "Revision block ordinal range differs from its rows");
    }
    Ok(())
}

fn apply_payload(
    payload: &[u8],
    row_count: u32,
    first_ordinal: u64,
    last_ordinal: u64,
    values: &mut [u8],
    path: &Path,
) -> Result<(), PersistenceError> {
    if payload.len() != row_count as usize * ROW_BYTES {
        return corrupt(path, "overlay block row count differs from payload");
    }
    for bytes in payload.chunks_exact(ROW_BYTES) {
        let row = decode_row(bytes);
        if row.ordinal < first_ordinal || row.ordinal > last_ordinal {
            continue;
        }
        let offset = row.ordinal - first_ordinal;
        let index = usize::try_from(offset)
            .map_err(|_| corrupt_value(path, "overlay row offset does not fit memory"))?;
        let value = values
            .get_mut(index)
            .ok_or_else(|| corrupt_value(path, "overlay row lies outside requested values"))?;
        *value = row.after;
    }
    Ok(())
}

fn validate_catalog_semantics(
    manifest: &ManifestFacts,
    revisions: &[Arc<ValidatedRevision>],
    operations: &[(OperationBytes, OperationRecord)],
    limits: CatalogLimits,
    ledger: &mut RecoveryLedger,
    control: &OperationControl,
) -> Result<(), PersistenceError> {
    for revision in revisions {
        let Some((_, operation)) = operations
            .iter()
            .find(|(id, _)| *id == revision.facts.candidate.operation)
        else {
            return corrupt(
                &revision.path,
                "committed Revision has no durable ready payload",
            );
        };
        let Some(ready) = operation.ready.as_ref() else {
            return corrupt(
                &revision.path,
                "committed Revision has no matching ready payload",
            );
        };
        if ready.facts != revision.facts {
            return corrupt(
                &revision.path,
                "committed Revision and ready payload differ",
            );
        }
        if operation.rejection.is_some() {
            return corrupt(
                &revision.path,
                "Operation has both a committed Revision and rejection",
            );
        }
        validate_revision_lineage_semantics(
            manifest, revisions, revision, limits, ledger, control,
        )?;
    }
    for (_, operation) in operations {
        if let Some(rejection) = operation.rejection {
            validate_catalog_rejection(manifest, revisions, operation, rejection)?;
        }
        let Some(ready) = operation.ready.as_deref() else {
            continue;
        };
        let is_committed = revisions
            .iter()
            .any(|revision| revision.facts.candidate.operation == ready.facts.candidate.operation);
        if !is_committed {
            validate_revision_lineage_semantics(
                manifest, revisions, ready, limits, ledger, control,
            )?;
        }
    }
    Ok(())
}

fn validate_catalog_rejection(
    manifest: &ManifestFacts,
    revisions: &[Arc<ValidatedRevision>],
    operation: &OperationRecord,
    rejection: RejectionFacts,
) -> Result<(), PersistenceError> {
    let known_revision = |identity| {
        identity == manifest.root_revision
            || revisions
                .iter()
                .any(|revision| revision.facts.revision == identity)
    };
    if rejection.reason_code == 2 && !known_revision(rejection.actual_head) {
        return corrupt(
            Path::new("<operation-catalog>"),
            "stale rejection records an unknown observed head Revision",
        );
    }
    if rejection.reason_code == 4
        && rejection.request_digest
            != revert_request_digest(rejection.workspace, manifest.source, manifest.root_revision)
    {
        return corrupt(
            Path::new("<operation-catalog>"),
            "root-Revert rejection request digest is not canonical",
        );
    }
    if let Some(ready) = operation.ready.as_deref()
        && (rejection.reason_code != 2
            || rejection.request_digest != ready.facts.candidate.request_digest
            || rejection.expected_head != ready.facts.candidate.parent
            || !known_revision(rejection.expected_head))
    {
        return corrupt(
            &ready.path,
            "ready payload and terminal rejection record different intent",
        );
    }
    Ok(())
}

fn validate_revision_lineage_semantics(
    manifest: &ManifestFacts,
    revisions: &[Arc<ValidatedRevision>],
    revision: &ValidatedRevision,
    limits: CatalogLimits,
    ledger: &mut RecoveryLedger,
    control: &OperationControl,
) -> Result<(), PersistenceError> {
    let parent = if revision.facts.candidate.parent == manifest.root_revision {
        None
    } else {
        Some(
            revisions
                .iter()
                .find(|candidate| candidate.facts.revision == revision.facts.candidate.parent)
                .ok_or_else(|| corrupt_value(&revision.path, "Revision parent does not exist"))?,
        )
    };
    let expected_sequence =
        parent.map_or(1, |value| value.facts.candidate.sequence.saturating_add(1));
    if revision.facts.candidate.sequence != expected_sequence {
        return corrupt(
            &revision.path,
            "Revision sequence is not exactly one after its parent",
        );
    }
    if revision.facts.candidate.kind != RevisionKind::Revert {
        return Ok(());
    }
    let Some(parent) = parent else {
        return corrupt(&revision.path, "Revert targets the root Revision");
    };
    if revision.facts.row_count != parent.facts.row_count {
        return corrupt(
            &revision.path,
            "Revert row count differs from its immediate parent",
        );
    }
    ledger.charge_revision_scan(parent, false, limits)?;
    if swapped_delta_digest(parent, limits.read, control)? != revision.facts.delta_digest {
        return corrupt(
            &revision.path,
            "Revert rows are not the exact inverse of its immediate parent",
        );
    }
    Ok(())
}

fn swapped_delta_digest(
    parent: &ValidatedRevision,
    limits: ReadLimits,
    control: &OperationControl,
) -> Result<DigestBytes, PersistenceError> {
    let mut hasher = Hasher::new();
    hasher.update(DELTA_DOMAIN);
    let row_limits = RowReadLimits {
        max_frames: parent.facts.block_count,
        max_payload_bytes: parent
            .facts
            .row_count
            .checked_mul(ROW_BYTES as u64)
            .ok_or_else(|| corrupt_value(&parent.path, "Revert parent byte count overflows"))?,
        max_working_bytes: limits.max_working_bytes,
    };
    for row in parent.rows(row_limits, control)? {
        let row = row?;
        let mut encoded = [0_u8; ROW_BYTES];
        encoded[..8].copy_from_slice(&row.ordinal.to_le_bytes());
        encoded[8] = row.after;
        encoded[9] = row.before;
        hasher.update(&encoded);
    }
    Ok(*hasher.finalize().as_bytes())
}

struct StackEncoder<const N: usize> {
    bytes: [u8; N],
    offset: usize,
}

impl<const N: usize> StackEncoder<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            offset: 0,
        }
    }

    fn push(&mut self, value: &[u8]) {
        let end = self.offset + value.len();
        self.bytes[self.offset..end].copy_from_slice(value);
        self.offset = end;
    }

    fn u8(&mut self, value: u8) {
        self.push(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.push(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.push(&value.to_le_bytes());
    }

    fn written(&self) -> &[u8] {
        &self.bytes[..self.offset]
    }

    fn finish(self) -> [u8; N] {
        self.bytes
    }
}

fn encode_manifest(facts: &ManifestFacts) -> [u8; MANIFEST_BYTES] {
    let mut bytes = StackEncoder::new();
    bytes.push(MANIFEST_MAGIC);
    bytes.u32(DISK_VERSION);
    bytes.u32(SEMANTIC_VERSION);
    bytes.push(&facts.workspace);
    bytes.push(&facts.source);
    bytes.u64(facts.source_point_count);
    for value in facts.position_transform_bits {
        bytes.u64(value);
    }
    bytes.u32(facts.classification.id);
    bytes.u32(facts.classification.name_len);
    bytes.push(&facts.classification.name);
    bytes.u8(facts.classification.data_type);
    bytes.push(&[0_u8; 3]);
    bytes.push(&facts.root_revision);
    bytes.push(&facts.source_contract);
    let checksum = domain_hash(MANIFEST_DOMAIN, bytes.written());
    bytes.push(&checksum);
    bytes.finish()
}

fn decode_manifest(
    bytes: &[u8; MANIFEST_BYTES],
    path: &Path,
) -> Result<ManifestFacts, PersistenceError> {
    let payload_bytes = MANIFEST_BYTES - DIGEST_BYTES;
    let (payload, checksum) = bytes.split_at(payload_bytes);
    if domain_hash(MANIFEST_DOMAIN, payload) != checksum {
        return corrupt(path, "manifest checksum differs");
    }
    let mut decoder = Decoder::new(payload, path);
    decoder.expect_magic(*MANIFEST_MAGIC)?;
    decoder.expect_version()?;
    let workspace = decoder.array()?;
    let source = decoder.array()?;
    let source_point_count = decoder.u64()?;
    let mut position_transform_bits = [0_u64; 6];
    for value in &mut position_transform_bits {
        *value = decoder.u64()?;
    }
    let classification_id = decoder.u32()?;
    let classification_name_len = decoder.u32()?;
    let classification_name = decoder.array()?;
    let classification_data_type = decoder.u8()?;
    decoder.zeroes(3)?;
    validate_persisted_attribute_definition(
        classification_id,
        classification_name_len,
        &classification_name,
        classification_data_type,
        path,
    )?;
    let root_revision = decoder.array()?;
    let source_contract = decoder.array()?;
    decoder.finish()?;
    Ok(ManifestFacts {
        workspace,
        source,
        source_point_count,
        position_transform_bits,
        classification: PersistedAttributeDefinition {
            id: classification_id,
            name_len: classification_name_len,
            name: classification_name,
            data_type: classification_data_type,
        },
        root_revision,
        source_contract,
    })
}

fn validate_persisted_attribute_definition(
    id: u32,
    name_len: u32,
    name: &[u8; MAX_ATTRIBUTE_NAME_BYTES],
    data_type: u8,
    path: &Path,
) -> Result<(), PersistenceError> {
    let name_len = usize::try_from(name_len)
        .ok()
        .filter(|length| (1..=MAX_ATTRIBUTE_NAME_BYTES).contains(length))
        .ok_or_else(|| corrupt_value(path, "classification Attribute name length is invalid"))?;
    let definition_name = std::str::from_utf8(&name[..name_len])
        .map_err(|_| corrupt_value(path, "classification Attribute name is not UTF-8"))?;
    if id == 0 || definition_name.trim().is_empty() {
        return corrupt(path, "classification Attribute definition is invalid");
    }
    if name[name_len..].iter().any(|byte| *byte != 0) {
        return corrupt(path, "classification Attribute name padding is not zero");
    }
    if data_type != ATTRIBUTE_DATA_TYPE_U8 {
        return incompatible(path, "classification Attribute type is not U8");
    }
    Ok(())
}

fn encode_revision_header(facts: &RevisionFacts) -> [u8; REVISION_HEADER_BYTES] {
    let mut bytes = StackEncoder::new();
    bytes.push(REVISION_MAGIC);
    bytes.u32(DISK_VERSION);
    bytes.u32(SEMANTIC_VERSION);
    bytes.u32(REVISION_HEADER_BYTES_U32);
    bytes.u32(ROW_BYTES_U32);
    bytes.push(&facts.candidate.workspace);
    bytes.push(&facts.candidate.source);
    bytes.push(&facts.candidate.operation);
    bytes.push(&facts.candidate.request_digest);
    bytes.push(&facts.candidate.parent);
    bytes.push(&facts.revision);
    bytes.u64(facts.candidate.sequence);
    match facts.candidate.kind {
        RevisionKind::SetClassification(value) => {
            bytes.u8(1);
            bytes.u8(value);
        }
        RevisionKind::Revert => {
            bytes.u8(2);
            bytes.u8(0);
        }
    }
    bytes.push(&[0_u8; 6]);
    bytes.u64(facts.row_count);
    bytes.u64(facts.block_count);
    bytes.push(&facts.delta_digest);
    bytes.u64(facts.body_bytes);
    bytes.push(&facts.candidate.source_contract);
    match facts.candidate.point_set {
        Some(point_set) => {
            bytes.u8(1);
            bytes.push(&[0_u8; 7]);
            bytes.u64(point_set.exact_count);
            bytes.push(&point_set.point_id_hash);
            bytes.push(&point_set.content_hash);
        }
        None => bytes.push(&[0_u8; 80]),
    }
    bytes.finish()
}

fn decode_revision_header(
    bytes: &[u8; REVISION_HEADER_BYTES],
    path: &Path,
) -> Result<RevisionFacts, PersistenceError> {
    let mut decoder = Decoder::new(bytes, path);
    decoder.expect_magic(*REVISION_MAGIC)?;
    decoder.expect_version()?;
    if decoder.u32()? != REVISION_HEADER_BYTES_U32 || decoder.u32()? != ROW_BYTES_U32 {
        return incompatible(path, "Revision header or row width differs");
    }
    let workspace = decoder.array()?;
    let source = decoder.array()?;
    let operation = decoder.array()?;
    let request_digest = decoder.array()?;
    let parent = decoder.array()?;
    let revision = decoder.array()?;
    let sequence = decoder.u64()?;
    let kind_tag = decoder.u8()?;
    let kind_value = decoder.u8()?;
    decoder.zeroes(6)?;
    let kind = match kind_tag {
        1 => RevisionKind::SetClassification(kind_value),
        2 if kind_value == 0 => RevisionKind::Revert,
        _ => return corrupt(path, "Revision change-kind tag is invalid"),
    };
    let row_count = decoder.u64()?;
    let block_count = decoder.u64()?;
    let delta_digest = decoder.array()?;
    let body_bytes = decoder.u64()?;
    let source_contract = decoder.array()?;
    let has_point_set = decoder.u8()?;
    decoder.zeroes(7)?;
    let point_set_count = decoder.u64()?;
    let point_id_hash = decoder.array()?;
    let point_set_content_hash = decoder.array()?;
    let point_set = match has_point_set {
        0 if point_set_count == 0
            && point_id_hash == [0; DIGEST_BYTES]
            && point_set_content_hash == [0; DIGEST_BYTES] =>
        {
            None
        }
        1 => Some(PersistedPointSetFacts {
            exact_count: point_set_count,
            point_id_hash,
            content_hash: point_set_content_hash,
        }),
        _ => {
            return corrupt(
                path,
                "Revision Point Set facts have an invalid presence tag",
            );
        }
    };
    decoder.zeroes(REVISION_HEADER_BYTES - decoder.offset())?;
    if matches!(kind, RevisionKind::SetClassification(_)) != point_set.is_some() {
        return corrupt(path, "Revision kind and Point Set facts disagree");
    }
    Ok(RevisionFacts {
        candidate: CandidateFacts {
            workspace,
            source,
            source_contract,
            operation,
            request_digest,
            parent,
            sequence,
            kind,
            point_set,
        },
        revision,
        row_count,
        block_count,
        delta_digest,
        body_bytes,
    })
}

fn encode_block_header(
    row_count: u32,
    first_ordinal: u64,
    last_ordinal: u64,
    payload_bytes: u64,
    checksum: DigestBytes,
) -> [u8; BLOCK_HEADER_BYTES] {
    let mut bytes = StackEncoder::new();
    bytes.push(BLOCK_MAGIC);
    bytes.u32(row_count);
    bytes.u32(0);
    bytes.u64(first_ordinal);
    bytes.u64(last_ordinal);
    bytes.u64(payload_bytes);
    bytes.push(&checksum);
    bytes.finish()
}

fn decode_block_header(
    bytes: &[u8; BLOCK_HEADER_BYTES],
    path: &Path,
) -> Result<DecodedBlockHeader, PersistenceError> {
    let mut decoder = Decoder::new(bytes, path);
    decoder.expect_magic(*BLOCK_MAGIC)?;
    let row_count = decoder.u32()?;
    if decoder.u32()? != 0 {
        return corrupt(path, "Revision block reserved bits are nonzero");
    }
    let first_ordinal = decoder.u64()?;
    let last_ordinal = decoder.u64()?;
    let payload_bytes = decoder.u64()?;
    let checksum = decoder.array()?;
    decoder.finish()?;
    Ok(DecodedBlockHeader {
        row_count,
        first_ordinal,
        last_ordinal,
        payload_bytes,
        checksum,
    })
}

fn encode_footer(body_bytes: u64, checksum: DigestBytes) -> [u8; FOOTER_BYTES] {
    let mut bytes = StackEncoder::new();
    bytes.push(FOOTER_MAGIC);
    bytes.u64(body_bytes);
    bytes.push(&checksum);
    bytes.finish()
}

fn decode_footer(
    bytes: &[u8; FOOTER_BYTES],
    path: &Path,
) -> Result<(u64, DigestBytes), PersistenceError> {
    let mut decoder = Decoder::new(bytes, path);
    decoder.expect_magic(*FOOTER_MAGIC)?;
    let body_bytes = decoder.u64()?;
    let checksum = decoder.array()?;
    decoder.finish()?;
    Ok((body_bytes, checksum))
}

fn encode_rejection(facts: &RejectionFacts) -> [u8; REJECTION_BYTES] {
    let mut bytes = [0_u8; REJECTION_BYTES];
    bytes[..8].copy_from_slice(REJECTION_MAGIC);
    bytes[8..12].copy_from_slice(&DISK_VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&SEMANTIC_VERSION.to_le_bytes());
    bytes[16..32].copy_from_slice(&facts.workspace);
    bytes[32..48].copy_from_slice(&facts.operation);
    bytes[48..80].copy_from_slice(&facts.request_digest);
    bytes[80..82].copy_from_slice(&facts.reason_code.to_le_bytes());
    bytes[88..120].copy_from_slice(&facts.expected_head);
    bytes[120..152].copy_from_slice(&facts.actual_head);
    let checksum = domain_hash(REJECTION_DOMAIN, &bytes[..152]);
    bytes[152..].copy_from_slice(&checksum);
    bytes
}

fn validate_rejection_file(
    path: &Path,
    expected_operation: Option<OperationBytes>,
) -> Result<RejectionFacts, PersistenceError> {
    let mut file = open_read(path)?;
    ensure_exact_file_length(&file, REJECTION_BYTES as u64, path)?;
    let mut bytes = [0_u8; REJECTION_BYTES];
    read_exact(&mut file, &mut bytes, path)?;
    let payload_bytes = REJECTION_BYTES - DIGEST_BYTES;
    let (payload, checksum) = bytes.split_at(payload_bytes);
    if domain_hash(REJECTION_DOMAIN, payload) != checksum {
        return corrupt(path, "rejection checksum differs");
    }
    let mut decoder = Decoder::new(payload, path);
    decoder.expect_magic(*REJECTION_MAGIC)?;
    decoder.expect_version()?;
    let workspace = decoder.array()?;
    let operation = decoder.array()?;
    let request_digest = decoder.array()?;
    let reason_code = decoder.u16()?;
    decoder.zeroes(6)?;
    let expected_head = decoder.array()?;
    let actual_head = decoder.array()?;
    decoder.finish()?;
    if operation == [0; OPERATION_ID_BYTES] {
        return corrupt(path, "rejection has the reserved zero Operation Identity");
    }
    validate_rejection_semantics(path, reason_code, expected_head, actual_head)?;
    if expected_operation.is_some_and(|expected| expected != operation) {
        return corrupt(
            path,
            "rejection Operation Identity differs from its filename",
        );
    }
    Ok(RejectionFacts {
        workspace,
        operation,
        request_digest,
        reason_code,
        expected_head,
        actual_head,
    })
}

fn validate_rejection_semantics(
    path: &Path,
    reason_code: u16,
    expected_head: RevisionBytes,
    actual_head: RevisionBytes,
) -> Result<(), PersistenceError> {
    match reason_code {
        2 if expected_head != [0; REVISION_ID_BYTES]
            && actual_head != [0; REVISION_ID_BYTES]
            && expected_head != actual_head =>
        {
            Ok(())
        }
        2 => corrupt(path, "stale-head rejection has missing Revision identity"),
        1 | 3..=4
            if expected_head == [0; REVISION_ID_BYTES] && actual_head == [0; REVISION_ID_BYTES] =>
        {
            Ok(())
        }
        1 | 3..=4 => corrupt(
            path,
            "non-stale rejection unexpectedly records Revision identities",
        ),
        _ => corrupt(path, "rejection has an unknown reason code"),
    }
}

fn derive_revision_id(facts: &CandidateFacts, delta_digest: DigestBytes) -> RevisionBytes {
    let mut hasher = Hasher::new();
    hasher.update(REVISION_ID_DOMAIN);
    hasher.update(&facts.workspace);
    hasher.update(&facts.source);
    hasher.update(&facts.source_contract);
    hasher.update(&facts.parent);
    hasher.update(&facts.sequence.to_le_bytes());
    hasher.update(&facts.operation);
    hasher.update(&facts.request_digest);
    hasher.update(&delta_digest);
    *hasher.finalize().as_bytes()
}

fn block_checksum(payload: &[u8]) -> DigestBytes {
    domain_hash(BLOCK_DOMAIN, payload)
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> DigestBytes {
    let mut hasher = Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn hash_file_prefix(
    file: &mut File,
    byte_count: u64,
    max_working_bytes: u64,
    path: &Path,
    control: Option<&OperationControl>,
) -> Result<DigestBytes, PersistenceError> {
    seek(file, 0, path)?;
    let mut remaining = byte_count;
    let buffer_bytes = byte_count
        .min(CHECKSUM_READ_BUFFER_BYTES as u64)
        .min(max_working_bytes);
    if byte_count > 0 && buffer_bytes == 0 {
        return Err(limit("checksum working bytes", 1, max_working_bytes));
    }
    let mut buffer = allocate_zeroed(
        buffer_bytes,
        max_working_bytes,
        "checksum working bytes",
        path,
    )?;
    let mut hasher = Hasher::new();
    hasher.update(FILE_DOMAIN);
    while remaining > 0 {
        check_optional_cancelled(control)?;
        let requested = usize::try_from(remaining.min(buffer.len() as u64))
            .expect("bounded checksum read fits usize");
        read_exact(file, &mut buffer[..requested], path)?;
        hasher.update(&buffer[..requested]);
        remaining -= requested as u64;
    }
    Ok(*hasher.finalize().as_bytes())
}

fn encode_row(row: RevisionRow, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&row.ordinal.to_le_bytes());
    bytes.push(row.before);
    bytes.push(row.after);
}

fn decode_row(bytes: &[u8]) -> RevisionRow {
    RevisionRow {
        ordinal: decode_ordinal(bytes),
        before: bytes[8],
        after: bytes[9],
    }
}

fn decode_ordinal(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().expect("row ordinal is eight bytes"))
}

fn validate_next_row(
    row: RevisionRow,
    previous_ordinal: Option<u64>,
    path: &Path,
) -> Result<(), PersistenceError> {
    if row.before == row.after {
        return corrupt(path, "Revision contains a no-op row");
    }
    if previous_ordinal.is_some_and(|previous| row.ordinal <= previous) {
        return corrupt(path, "Revision rows are not strictly increasing and unique");
    }
    Ok(())
}

fn validate_write_limits(limits: WriteLimits) -> Result<(), PersistenceError> {
    if limits.rows_per_block == 0 {
        return Err(limit("rows per block", 0, 0));
    }
    let block_bytes = u64::from(limits.rows_per_block)
        .checked_mul(ROW_BYTES as u64)
        .ok_or_else(|| limit("Revision block bytes", u64::MAX, limits.max_block_bytes))?;
    ensure_at_most(block_bytes, limits.max_block_bytes, "Revision block bytes")?;
    ensure_at_most(
        block_bytes.saturating_add(limits.retained_input_bytes),
        limits.max_working_bytes,
        "combined commit working bytes",
    )?;
    ensure_at_most(
        (REVISION_HEADER_BYTES + FOOTER_BYTES) as u64,
        limits.max_temporary_bytes,
        "commit temporary bytes",
    )
}

fn parse_revision_name(name: &str) -> Option<(u64, RevisionBytes)> {
    let stem = name.strip_suffix(".pwr")?;
    let (sequence, revision) = stem.split_once('-')?;
    if sequence.len() != 20 {
        return None;
    }
    Some((sequence.parse().ok()?, decode_hex(revision)?))
}

fn parse_operation_name(name: &str) -> Option<(OperationBytes, OperationFileKind)> {
    if let Some(stem) = name.strip_suffix(".ready") {
        return Some((decode_hex(stem)?, OperationFileKind::Ready));
    }
    let stem = name.strip_suffix(".reject")?;
    Some((decode_hex(stem)?, OperationFileKind::Reject))
}

fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 {
        return None;
    }
    let mut result = [0_u8; N];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(result)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn create_temporary(
    directory: &Path,
    prefix: &str,
    suffix: &str,
) -> Result<(PathBuf, File), PersistenceError> {
    for _ in 0..64 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| PersistenceError::Entropy)?;
        let path = directory.join(format!("{prefix}-{}.{}", encode_hex(&random), suffix));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io_error("create scratch file", &path, source)),
        }
    }
    Err(PersistenceError::Entropy)
}

// Cleanup deliberately recognizes only canonical lowercase private filenames.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn is_recognized_scratch(name: &str) -> bool {
    const RANDOM_HEX_BYTES: usize = 16 * 2;
    [
        ("revision-", ".tmp"),
        ("reject-", ".tmp"),
        ("manifest-", ".tmp"),
        ("point-set-", ".pset"),
    ]
    .into_iter()
    .any(|(prefix, suffix)| {
        name.strip_prefix(prefix)
            .and_then(|remainder| remainder.strip_suffix(suffix))
            .is_some_and(|nonce| {
                nonce.len() == RANDOM_HEX_BYTES
                    && nonce
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
    })
}

fn acquire_lock(root: &Path) -> Result<File, PersistenceError> {
    let path = root.join("workspace.lock");
    match fs::symlink_metadata(&path) {
        Ok(_) => require_regular_file(&path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("inspect Workspace lock", &path, source)),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| io_error("open Workspace lock", &path, source))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(error) => {
            let source: io::Error = error.into();
            if source.kind() == io::ErrorKind::WouldBlock {
                Err(PersistenceError::Locked)
            } else {
                Err(io_error("acquire Workspace lock", &path, source))
            }
        }
    }
}

fn publish_link(source: &Path, target: &Path) -> Result<(), PersistenceError> {
    fs::hard_link(source, target).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            PersistenceError::PublicationConflict
        } else {
            io_error("publish immutable link", target, error)
        }
    })
}

fn make_read_only(path: &Path) -> Result<(), PersistenceError> {
    let metadata =
        fs::metadata(path).map_err(|source| io_error("read permissions", path, source))?;
    let mut permissions: Permissions = metadata.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .map_err(|source| io_error("seal file read-only", path, source))
}

fn create_directory_if_missing(path: &Path) -> Result<(), PersistenceError> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(io_error("create directory", path, source)),
    }
}

fn create_or_recognize_incomplete_root(root: &Path) -> Result<bool, PersistenceError> {
    match fs::create_dir(root) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(true),
        Err(source) => Err(io_error("create directory", root, source)),
    }
}

fn validate_incomplete_root(root: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|source| io_error("inspect incomplete Workspace", root, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return corrupt(root, "incomplete Workspace root is not a private directory");
    }
    for entry in read_directory(root)? {
        let entry = entry.map_err(|source| io_error("read incomplete Workspace", root, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return corrupt(
                &entry.path(),
                "incomplete Workspace entry name is not UTF-8",
            );
        };
        match name {
            "workspace.lock" => {}
            "operations" | "revisions" => require_empty_private_directory(&entry.path())?,
            "scratch" => require_recognized_scratch_directory(&entry.path())?,
            "manifest.pwm" => {
                return Err(PersistenceError::PublicationConflict);
            }
            _ => {
                return corrupt(
                    &entry.path(),
                    "unrecognized entry in incomplete Workspace root",
                );
            }
        }
    }
    Ok(())
}

fn require_empty_private_directory(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect incomplete Workspace directory", path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return corrupt(
            path,
            "incomplete Workspace entry is not a private directory",
        );
    }
    if read_directory(path)?.next().is_some() {
        return corrupt(path, "incomplete Workspace contains published payloads");
    }
    Ok(())
}

fn require_recognized_scratch_directory(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect incomplete Workspace scratch", path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return corrupt(
            path,
            "incomplete Workspace scratch is not a private directory",
        );
    }
    for entry in read_directory(path)? {
        let entry =
            entry.map_err(|source| io_error("read incomplete Workspace scratch", path, source))?;
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_recognized_scratch) {
            return corrupt(
                &entry.path(),
                "incomplete Workspace scratch contains an unrecognized entry",
            );
        }
    }
    Ok(())
}

fn required_directory(path: PathBuf) -> Result<PathBuf, PersistenceError> {
    require_real_directory(&path)?;
    Ok(path)
}

fn require_real_directory(path: &Path) -> Result<(), PersistenceError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error("open directory", path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return corrupt(
            path,
            "required Workspace directory is not a private real directory",
        );
    }
    Ok(())
}

fn read_directory(path: &Path) -> Result<fs::ReadDir, PersistenceError> {
    fs::read_dir(path).map_err(|source| io_error("read directory", path, source))
}

fn sync_directory(path: &Path) -> Result<(), PersistenceError> {
    let directory =
        File::open(path).map_err(|source| io_error("open directory for sync", path, source))?;
    directory
        .sync_all()
        .map_err(|source| io_error("sync directory", path, source))
}

fn workspace_parent(root: &Path) -> &Path {
    root.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn open_read(path: &Path) -> Result<File, PersistenceError> {
    require_regular_file(path)?;
    File::open(path).map_err(|source| io_error("open file", path, source))
}

fn write_all(file: &mut File, bytes: &[u8], path: &Path) -> Result<(), PersistenceError> {
    file.write_all(bytes)
        .map_err(|source| io_error("write file", path, source))
}

fn read_exact(file: &mut File, bytes: &mut [u8], path: &Path) -> Result<(), PersistenceError> {
    file.read_exact(bytes).map_err(|source| {
        if source.kind() == io::ErrorKind::UnexpectedEof {
            PersistenceError::Corrupt {
                path: path.to_path_buf(),
                reason: "published file is truncated",
            }
        } else {
            io_error("read file", path, source)
        }
    })
}

fn seek(file: &mut File, offset: u64, path: &Path) -> Result<(), PersistenceError> {
    file.seek(SeekFrom::Start(offset))
        .map(|_| ())
        .map_err(|source| io_error("seek file", path, source))
}

fn stream_position(file: &mut File, path: &Path) -> Result<u64, PersistenceError> {
    file.stream_position()
        .map_err(|source| io_error("read file position", path, source))
}

fn sync_file(file: &File, path: &Path) -> Result<(), PersistenceError> {
    file.sync_all()
        .map_err(|source| io_error("sync file", path, source))
}

fn remove_file(path: &Path) -> Result<(), PersistenceError> {
    fs::remove_file(path).map_err(|source| io_error("remove scratch file", path, source))
}

fn file_length(path: &Path) -> Result<u64, PersistenceError> {
    require_regular_file(path)?;
    fs::symlink_metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|source| io_error("read file metadata", path, source))
}

fn require_regular_file(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect published file", path, source))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return corrupt(path, "published Workspace entry is not a private real file");
    }
    Ok(())
}

fn ensure_exact_file_length(
    file: &File,
    expected: u64,
    path: &Path,
) -> Result<(), PersistenceError> {
    let actual = file
        .metadata()
        .map_err(|source| io_error("read metadata", path, source))?
        .len();
    if actual != expected {
        return corrupt(path, "published file length differs from its fixed format");
    }
    Ok(())
}

fn charge(
    current: &mut u64,
    additional: u64,
    maximum: u64,
    resource: &'static str,
) -> Result<(), PersistenceError> {
    let next = current
        .checked_add(additional)
        .ok_or_else(|| limit(resource, u64::MAX, maximum))?;
    ensure_at_most(next, maximum, resource)?;
    *current = next;
    Ok(())
}

fn ensure_at_most(
    actual: u64,
    limit_value: u64,
    resource: &'static str,
) -> Result<(), PersistenceError> {
    if actual > limit_value {
        return Err(limit(resource, actual, limit_value));
    }
    Ok(())
}

fn check_cancelled(control: &OperationControl) -> Result<(), PersistenceError> {
    control
        .check_cancelled()
        .map_err(|_| PersistenceError::Cancelled)
}

fn check_optional_cancelled(control: Option<&OperationControl>) -> Result<(), PersistenceError> {
    control.map_or(Ok(()), check_cancelled)
}

fn to_usize(value: u64, path: &Path) -> Result<usize, PersistenceError> {
    usize::try_from(value)
        .map_err(|_| corrupt_value(path, "published length does not fit this platform"))
}

fn allocate_zeroed(
    bytes: u64,
    allowed: u64,
    resource: &'static str,
    path: &Path,
) -> Result<Vec<u8>, PersistenceError> {
    ensure_at_most(bytes, allowed, resource)?;
    let length = to_usize(bytes, path)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| limit(resource, bytes, allowed))?;
    ensure_at_most(
        u64::try_from(result.capacity()).unwrap_or(u64::MAX),
        allowed,
        resource,
    )?;
    result.resize(length, 0);
    Ok(result)
}

fn io_error(action: &'static str, path: &Path, source: io::Error) -> PersistenceError {
    PersistenceError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

fn corrupt<T>(path: &Path, reason: &'static str) -> Result<T, PersistenceError> {
    Err(PersistenceError::Corrupt {
        path: path.to_path_buf(),
        reason,
    })
}

fn corrupt_value(path: &Path, reason: &'static str) -> PersistenceError {
    PersistenceError::Corrupt {
        path: path.to_path_buf(),
        reason,
    }
}

fn incompatible<T>(path: &Path, reason: &'static str) -> Result<T, PersistenceError> {
    Err(PersistenceError::Incompatible {
        path: path.to_path_buf(),
        reason,
    })
}

fn limit(resource: &'static str, actual: u64, limit: u64) -> PersistenceError {
    PersistenceError::Limit {
        resource,
        actual,
        limit,
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    path: &'a Path,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8], path: &'a Path) -> Self {
        Self {
            bytes,
            offset: 0,
            path,
        }
    }

    const fn offset(&self) -> usize {
        self.offset
    }

    fn expect_magic(&mut self, expected: [u8; 8]) -> Result<(), PersistenceError> {
        let actual: [u8; 8] = self.array()?;
        if actual != expected {
            return incompatible(self.path, "file magic differs");
        }
        Ok(())
    }

    fn expect_version(&mut self) -> Result<(), PersistenceError> {
        if self.u32()? != DISK_VERSION || self.u32()? != SEMANTIC_VERSION {
            return incompatible(self.path, "disk or semantic contract version differs");
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, PersistenceError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PersistenceError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, PersistenceError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, PersistenceError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PersistenceError> {
        self.take(N)?
            .try_into()
            .map_err(|_| corrupt_value(self.path, "fixed-width field is truncated"))
    }

    fn zeroes(&mut self, count: usize) -> Result<(), PersistenceError> {
        if self.take(count)?.iter().any(|byte| *byte != 0) {
            return corrupt(self.path, "reserved bytes are nonzero");
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), PersistenceError> {
        if self.offset != self.bytes.len() {
            return corrupt(self.path, "fixed-format payload has trailing bytes");
        }
        Ok(())
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], PersistenceError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| corrupt_value(self.path, "fixed-format offset overflows"))?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| corrupt_value(self.path, "fixed-format field is truncated"))?;
        self.offset = end;
        Ok(result)
    }
}

#[cfg(test)]
pub(crate) mod test_fault {
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

    use super::{FaultPoint, Path, PathBuf, PersistenceError, io, io_error};

    #[derive(Clone, Copy)]
    pub(crate) enum Action {
        Error,
        Cancel,
        Panic,
        PauseThenContinue,
        PauseThenPanic,
    }

    #[derive(Default)]
    struct PauseState {
        flags: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    struct Plan {
        root: PathBuf,
        point: FaultPoint,
        action: Action,
        pause: Arc<PauseState>,
    }

    fn plan() -> &'static Mutex<Option<Plan>> {
        static PLAN: OnceLock<Mutex<Option<Plan>>> = OnceLock::new();
        PLAN.get_or_init(|| Mutex::new(None))
    }

    fn serial() -> &'static Mutex<()> {
        static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
        SERIAL.get_or_init(|| Mutex::new(()))
    }

    pub(crate) struct Guard {
        _serial: MutexGuard<'static, ()>,
        pause: Arc<PauseState>,
    }

    impl Guard {
        pub(crate) fn install(root: &Path, point: FaultPoint, action: Action) -> Self {
            let serial = serial()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let pause = Arc::new(PauseState::default());
            *plan()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Plan {
                root: root.to_path_buf(),
                point,
                action,
                pause: Arc::clone(&pause),
            });
            Self {
                _serial: serial,
                pause,
            }
        }

        pub(crate) fn wait_until_hit(&self) {
            let mut flags = self
                .pause
                .flags
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while !flags.0 {
                flags = self
                    .pause
                    .changed
                    .wait(flags)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }

        pub(crate) fn release(&self) {
            let mut flags = self
                .pause
                .flags
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            flags.1 = true;
            self.pause.changed.notify_all();
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.release();
            *plan()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
    }

    pub(super) fn inject(path: &Path, point: FaultPoint) -> Result<(), PersistenceError> {
        let action = {
            let mut plan = plan()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let matches = plan
                .as_ref()
                .is_some_and(|item| item.point == point && path.starts_with(&item.root));
            matches.then(|| {
                let plan = plan.take().expect("matching fault plan");
                (plan.action, plan.pause)
            })
        };
        match action {
            None => Ok(()),
            Some((Action::Error, _)) => Err(io_error(
                "run injected persistence fault",
                path,
                io::Error::other(format!("injected fault at {point:?}")),
            )),
            Some((Action::Cancel, _)) => Err(PersistenceError::Cancelled),
            Some((Action::Panic, _)) => panic!("injected panic at {point:?}"),
            Some((Action::PauseThenContinue, pause)) => {
                pause_until_released(&pause);
                Ok(())
            }
            Some((Action::PauseThenPanic, pause)) => {
                pause_until_released(&pause);
                panic!("injected paused panic at {point:?}");
            }
        }
    }

    fn pause_until_released(pause: &PauseState) {
        let mut flags = pause
            .flags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        flags.0 = true;
        pause.changed.notify_all();
        while !flags.1 {
            flags = pause
                .changed
                .wait(flags)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let base = std::env::temp_dir();
            for _ in 0..64 {
                let mut random = [0_u8; 16];
                getrandom::fill(&mut random).expect("test entropy");
                let path = base.join(format!("punctra-persistence-{}", encode_hex(&random)));
                if !path.exists() {
                    return Self(path);
                }
            }
            panic!("could not choose a unique test directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest() -> ManifestFacts {
        let mut classification_name = [0_u8; MAX_ATTRIBUTE_NAME_BYTES];
        classification_name[..14].copy_from_slice(b"classification");
        ManifestFacts {
            workspace: [1; WORKSPACE_ID_BYTES],
            source: [2; DIGEST_BYTES],
            source_point_count: 42,
            position_transform_bits: [1, 2, 3, 4, 5, 6],
            classification: PersistedAttributeDefinition {
                id: 7,
                name_len: 14,
                name: classification_name,
                data_type: ATTRIBUTE_DATA_TYPE_U8,
            },
            root_revision: [3; REVISION_ID_BYTES],
            source_contract: [4; DIGEST_BYTES],
        }
    }

    fn candidate() -> CandidateFacts {
        let mut facts = CandidateFacts {
            workspace: [1; WORKSPACE_ID_BYTES],
            source: [2; DIGEST_BYTES],
            source_contract: [4; DIGEST_BYTES],
            operation: [5; OPERATION_ID_BYTES],
            request_digest: [0; DIGEST_BYTES],
            parent: [3; REVISION_ID_BYTES],
            sequence: 1,
            kind: RevisionKind::SetClassification(9),
            point_set: Some(PersistedPointSetFacts {
                exact_count: 2,
                point_id_hash: [7; DIGEST_BYTES],
                content_hash: [8; DIGEST_BYTES],
            }),
        };
        facts.request_digest = classification_request_digest(ClassificationRequestFacts {
            workspace: facts.workspace,
            source: facts.source,
            classification_attribute: manifest().classification.id,
            point_set_workspace: facts.workspace,
            point_set_source: facts.source,
            parent: facts.parent,
            point_set: facts.point_set.expect("classification Point Set facts"),
            value: 9,
        });
        facts
    }

    fn write_limits() -> WriteLimits {
        WriteLimits {
            max_file_bytes: 1 << 20,
            max_rows: 16,
            max_blocks: 16,
            max_block_bytes: 1 << 10,
            rows_per_block: 2,
            max_working_bytes: 1 << 20,
            retained_input_bytes: 0,
            max_temporary_bytes: 1 << 20,
        }
    }

    fn catalog_limits() -> CatalogLimits {
        CatalogLimits {
            read: ReadLimits {
                max_file_bytes: 1 << 20,
                max_rows: 64,
                max_blocks: 64,
                max_block_bytes: 1 << 10,
                max_working_bytes: 1 << 20,
            },
            max_revisions: 16,
            max_operation_files: 32,
            max_total_bytes: 1 << 24,
            max_metadata_bytes: 1 << 20,
        }
    }

    fn rows() -> [Result<RevisionRow, PersistenceError>; 2] {
        [
            Ok(RevisionRow {
                ordinal: 3,
                before: 1,
                after: 9,
            }),
            Ok(RevisionRow {
                ordinal: 8,
                before: 2,
                after: 9,
            }),
        ]
    }

    fn initialized_store(directory: &TestDirectory) -> Store {
        let mut store = Store::create(directory.path()).expect("create test Store");
        store
            .publish_manifest(&manifest())
            .expect("publish test manifest");
        store
    }

    fn sealed(store: &Store) -> SealedRevision {
        store
            .seal_candidate(
                candidate(),
                rows(),
                write_limits(),
                &OperationControl::new(),
            )
            .expect("seal test candidate")
    }

    fn reopen(directory: &TestDirectory) -> Catalog {
        let store = Store::open(directory.path()).expect("reopen test Store");
        let facts = store.read_manifest().expect("read test manifest");
        store
            .recover(&facts, catalog_limits(), &OperationControl::new())
            .expect("recover test Store")
    }

    #[test]
    fn manifest_round_trips_and_detects_corruption() {
        let facts = manifest();
        let bytes = encode_manifest(&facts);
        assert_eq!(
            decode_manifest(&bytes, Path::new("manifest")).expect("valid manifest"),
            facts
        );

        let mut corrupt_bytes = bytes;
        corrupt_bytes[40] ^= 1;
        assert!(matches!(
            decode_manifest(&corrupt_bytes, Path::new("manifest")),
            Err(PersistenceError::Corrupt { .. })
        ));
    }

    #[test]
    fn revision_identity_is_stable_and_payload_sensitive() {
        let facts = candidate();
        let first = derive_revision_id(&facts, [8; DIGEST_BYTES]);
        assert_eq!(first, derive_revision_id(&facts, [8; DIGEST_BYTES]));
        assert_ne!(first, derive_revision_id(&facts, [9; DIGEST_BYTES]));
    }

    #[test]
    fn catalog_growth_is_preflighted_at_old_plus_new_capacity() {
        let directory = TestDirectory::new();
        let store = initialized_store(&directory);
        let sealed_candidate = sealed(&store);
        let ready = Arc::new(sealed_candidate.file.clone());
        let mut catalog = Catalog::empty(manifest().root_revision);

        let ready_minimum = arc_revision_fixed_bytes().saturating_add(child_path_bytes(
            &store.operations,
            OPERATION_ID_BYTES * 2 + ".ready".len(),
        ));
        assert!(matches!(
            store.prepare_ready(&sealed_candidate, ready_minimum - 1),
            Err(PersistenceError::Limit { .. })
        ));
        let prepared_ready = store
            .prepare_ready(&sealed_candidate, u64::MAX)
            .expect("prepare bounded ready target");
        let ready_exact = arc_revision_fixed_bytes()
            .saturating_add(u64::try_from(prepared_ready.path.capacity()).unwrap_or(u64::MAX));
        store
            .prepare_ready(&sealed_candidate, ready_exact)
            .expect("exact ready target allocation");
        let prepared_revision = store
            .prepare_revision(&prepared_ready, u64::MAX)
            .expect("prepare bounded Revision target");
        let revision_exact = arc_revision_fixed_bytes()
            .saturating_add(u64::try_from(prepared_revision.path.capacity()).unwrap_or(u64::MAX));
        assert!(matches!(
            store.prepare_revision(&prepared_ready, revision_exact - 1),
            Err(PersistenceError::Limit { .. })
        ));
        store
            .prepare_revision(&prepared_ready, revision_exact)
            .expect("exact Revision target allocation");

        let operation_entry_bytes = vector_bytes::<(OperationBytes, OperationRecord)>(1);
        assert!(matches!(
            catalog.reserve_operation(
                ready.facts.candidate.operation,
                operation_entry_bytes.saturating_sub(1),
            ),
            Err(PersistenceError::Limit { .. })
        ));
        assert_eq!(catalog.operations.capacity(), 0);
        catalog
            .reserve_operation(ready.facts.candidate.operation, operation_entry_bytes)
            .expect("exact first Operation transition");
        catalog.record_ready(Arc::clone(&ready));

        let second_operation = [9; OPERATION_ID_BYTES];
        let second_transition = operation_entry_bytes.saturating_mul(3);
        assert!(matches!(
            catalog.reserve_operation(second_operation, second_transition - 1),
            Err(PersistenceError::Limit { .. })
        ));
        catalog
            .reserve_operation(second_operation, second_transition)
            .expect("exact growing Operation transition");

        let revision_entry_bytes = vector_bytes::<Arc<ValidatedRevision>>(1);
        assert!(matches!(
            catalog.reserve_revision(revision_entry_bytes.saturating_sub(1)),
            Err(PersistenceError::Limit { .. })
        ));
        catalog
            .reserve_revision(revision_entry_bytes)
            .expect("exact first Revision transition");
        catalog.append_committed(Arc::clone(&ready));
        let second_revision_transition = revision_entry_bytes.saturating_mul(3);
        assert!(matches!(
            catalog.reserve_revision(second_revision_transition - 1),
            Err(PersistenceError::Limit { .. })
        ));
        catalog
            .reserve_revision(second_revision_transition)
            .expect("exact growing Revision transition");
    }

    #[test]
    fn rejection_semantics_fail_closed_before_catalog_publication() {
        let path = Path::new("operation.reject");
        let none = [0; REVISION_ID_BYTES];
        let revision = [1; REVISION_ID_BYTES];
        let other_revision = [2; REVISION_ID_BYTES];

        assert!(validate_rejection_semantics(path, 1, none, none).is_ok());
        assert!(validate_rejection_semantics(path, 2, revision, other_revision).is_ok());
        for invalid in [
            validate_rejection_semantics(path, 0, none, none),
            validate_rejection_semantics(path, 6, none, none),
            validate_rejection_semantics(path, 2, none, revision),
            validate_rejection_semantics(path, 2, revision, none),
            validate_rejection_semantics(path, 2, revision, revision),
            validate_rejection_semantics(path, 3, revision, revision),
            validate_rejection_semantics(path, 5, none, none),
        ] {
            assert!(matches!(invalid, Err(PersistenceError::Corrupt { .. })));
        }
    }

    #[test]
    fn revision_name_requires_canonical_width_and_lower_hex() {
        let revision = [0xab; REVISION_ID_BYTES];
        let name = format!("{:020}-{}.pwr", 3, encode_hex(&revision));
        assert_eq!(parse_revision_name(&name), Some((3, revision)));
        assert_eq!(parse_revision_name("3-ab.pwr"), None);
        assert_eq!(parse_revision_name(&name.to_uppercase()), None);
    }

    #[test]
    fn scratch_name_requires_canonical_width_and_lower_hex() {
        for name in [
            "revision-deadbeefdeadbeefdeadbeefdeadbeef.tmp",
            "reject-deadbeefdeadbeefdeadbeefdeadbeef.tmp",
            "manifest-deadbeefdeadbeefdeadbeefdeadbeef.tmp",
            "point-set-deadbeefdeadbeefdeadbeefdeadbeef.pset",
        ] {
            assert!(is_recognized_scratch(name), "canonical name {name}");
        }
        for name in [
            "revision-user-backup.tmp",
            "revision-deadbeef.tmp",
            "revision-DEADBEEFDEADBEEFDEADBEEFDEADBEEF.tmp",
            "revision-deadbeefdeadbeefdeadbeefdeadbeef.pset",
            "point-set-deadbeefdeadbeefdeadbeefdeadbeef.tmp",
        ] {
            assert!(!is_recognized_scratch(name), "noncanonical name {name}");
        }
    }

    #[test]
    fn empty_candidate_needs_no_block_capacity_and_sealed_drop_cleans_stage() {
        let directory = TestDirectory::new();
        let store = initialized_store(&directory);
        let no_block_limits = WriteLimits {
            max_file_bytes: 0,
            max_rows: 0,
            max_blocks: 0,
            max_block_bytes: 0,
            rows_per_block: 0,
            max_working_bytes: 0,
            retained_input_bytes: 0,
            max_temporary_bytes: 0,
        };
        assert!(matches!(
            store.seal_candidate(
                candidate(),
                std::iter::empty(),
                no_block_limits,
                &OperationControl::new(),
            ),
            Err(PersistenceError::NoRows)
        ));

        let stage = sealed(&store);
        let path = stage.path().to_path_buf();
        assert!(path.exists());
        drop(stage);
        assert!(!path.exists());
    }

    #[test]
    fn create_resumes_only_a_recognized_premanifest_tree() {
        let directory = TestDirectory::new();
        fs::create_dir(directory.path()).expect("partial root");
        File::create(directory.path().join("workspace.lock")).expect("partial lock");
        fs::create_dir(directory.path().join("operations")).expect("partial operations");
        fs::create_dir(directory.path().join("scratch")).expect("partial scratch");
        File::create(
            directory
                .path()
                .join("scratch/revision-deadbeefdeadbeefdeadbeefdeadbeef.tmp"),
        )
        .expect("recognized partial stage");

        let mut store = Store::create(directory.path()).expect("resume recognized partial create");
        assert!(directory.path().join("revisions").is_dir());
        assert_eq!(
            fs::read_dir(directory.path().join("scratch"))
                .expect("cleaned scratch")
                .count(),
            0
        );
        store
            .publish_manifest(&manifest())
            .expect("complete resumed create");
        drop(store);
        assert!(directory.path().join("manifest.pwm").is_file());
    }

    #[test]
    fn create_preserves_noncanonical_scratch_files() {
        let directory = TestDirectory::new();
        fs::create_dir(directory.path()).expect("partial root");
        File::create(directory.path().join("workspace.lock")).expect("partial lock");
        fs::create_dir(directory.path().join("operations")).expect("partial operations");
        fs::create_dir(directory.path().join("scratch")).expect("partial scratch");
        let sentinel = directory.path().join("scratch/revision-user-backup.tmp");
        File::create(&sentinel).expect("noncanonical scratch sentinel");

        assert!(Store::create(directory.path()).is_err());
        assert!(sentinel.is_file());
    }

    #[test]
    fn create_parent_sync_failure_remains_precommit_and_cleans_owned_root() {
        let directory = TestDirectory::new();
        let mut store = Store::create(directory.path()).expect("create premanifest Store");
        let fault = test_fault::Guard::install(
            directory.path(),
            FaultPoint::ManifestParentDirectorySync,
            test_fault::Action::Error,
        );
        assert!(store.publish_manifest(&manifest()).is_err());
        drop(fault);
        drop(store);
        assert!(!directory.path().exists());
    }

    #[test]
    fn create_preserves_unrecognized_existing_directories() {
        let directory = TestDirectory::new();
        fs::create_dir(directory.path()).expect("existing root");
        let sentinel = directory.path().join("user-data");
        File::create(&sentinel).expect("sentinel");
        assert!(Store::create(directory.path()).is_err());
        assert!(sentinel.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn open_rejects_symlinked_private_directories_without_touching_targets() {
        use std::os::unix::fs::symlink;

        for child in ["operations", "revisions", "scratch"] {
            let directory = TestDirectory::new();
            let store = initialized_store(&directory);
            drop(store);
            let external = TestDirectory::new();
            fs::create_dir(external.path()).expect("external directory");
            let sentinel = external.path().join("revision-deadbeef.tmp");
            File::create(&sentinel).expect("external sentinel");
            fs::remove_dir_all(directory.path().join(child)).expect("remove private child");
            symlink(external.path(), directory.path().join(child)).expect("symlink private child");

            assert!(matches!(
                Store::open(directory.path()),
                Err(PersistenceError::Corrupt { .. })
            ));
            assert!(sentinel.is_file());
        }
    }

    #[cfg(unix)]
    #[test]
    fn open_rejects_symlinked_published_leaf_and_lock_files() {
        use std::os::unix::fs::symlink;

        for relative in ["manifest.pwm", "workspace.lock"] {
            let directory = TestDirectory::new();
            let store = initialized_store(&directory);
            drop(store);
            let external = TestDirectory::new();
            fs::create_dir(external.path()).expect("external directory");
            let target = external.path().join("target");
            File::create(&target).expect("external target");
            fs::remove_file(directory.path().join(relative)).expect("remove private leaf");
            symlink(&target, directory.path().join(relative)).expect("symlink private leaf");

            assert!(matches!(
                Store::open(directory.path()).and_then(|store| store.read_manifest()),
                Err(PersistenceError::Corrupt { .. })
            ));
        }
    }

    #[test]
    fn recovery_charges_hardlinked_ready_and_revision_payload_once() {
        let directory = TestDirectory::new();
        let store = initialized_store(&directory);
        let stage = sealed(&store);
        let ready = store
            .publish_ready(&stage, || Ok(()))
            .expect("publish ready");
        let logical_bytes = ready.file_bytes();
        store
            .publish_revision(&ready, || Ok(()), || {})
            .expect("publish Revision");
        drop(store);

        let store = Store::open(directory.path()).expect("reopen Store");
        let mut exact = catalog_limits();
        exact.max_total_bytes = logical_bytes;
        let recovered = store
            .recover(&manifest(), exact, &OperationControl::new())
            .expect("exact logical byte boundary");
        assert_eq!(recovered.revisions.len(), 1);
        drop(store);

        let store = Store::open(directory.path()).expect("reopen Store below boundary");
        exact.max_total_bytes = logical_bytes - 1;
        assert!(matches!(
            store.recover(&manifest(), exact, &OperationControl::new()),
            Err(PersistenceError::Limit {
                resource: "durable bytes",
                ..
            })
        ));
    }

    #[test]
    fn candidate_stage_faults_publish_nothing_and_recovery_cleans_scratch() {
        let points = [
            FaultPoint::CandidateStage,
            FaultPoint::CandidateFileSync,
            FaultPoint::CandidateClose,
            FaultPoint::CandidateReadOnly,
            FaultPoint::CandidateRevalidate,
        ];
        for point in points {
            let directory = TestDirectory::new();
            let store = initialized_store(&directory);
            let fault =
                test_fault::Guard::install(directory.path(), point, test_fault::Action::Error);
            assert!(
                store
                    .seal_candidate(
                        candidate(),
                        rows(),
                        write_limits(),
                        &OperationControl::new(),
                    )
                    .is_err()
            );
            drop(fault);
            drop(store);
            let catalog = reopen(&directory);
            assert!(catalog.revisions.is_empty());
            assert!(catalog.operations.is_empty());
            assert_eq!(
                fs::read_dir(directory.path().join("scratch"))
                    .expect("read scratch")
                    .count(),
                0
            );
        }
    }

    #[test]
    fn ready_faults_recover_only_absent_or_complete_intent() {
        let points = [
            (FaultPoint::ReadyLink, false),
            (FaultPoint::OperationsDirectorySync, true),
            (FaultPoint::OperationLostAcknowledgement, true),
            (FaultPoint::ReadyCleanup, true),
        ];
        for (point, expected_ready) in points {
            let directory = TestDirectory::new();
            let store = initialized_store(&directory);
            let sealed_candidate = sealed(&store);
            let fault =
                test_fault::Guard::install(directory.path(), point, test_fault::Action::Error);
            assert!(store.publish_ready(&sealed_candidate, || Ok(())).is_err());
            drop(fault);
            drop(store);
            let catalog = reopen(&directory);
            assert_eq!(
                catalog
                    .operation(candidate().operation)
                    .and_then(|record| record.ready.as_ref())
                    .is_some(),
                expected_ready
            );
            assert!(catalog.revisions.is_empty());
        }
    }

    #[test]
    fn revision_faults_recover_only_old_or_complete_new_head() {
        let points = [
            (FaultPoint::RevisionLink, false),
            (FaultPoint::RevisionDirectorySync, true),
            (FaultPoint::RevisionLostAcknowledgement, true),
        ];
        for (point, expected_commit) in points {
            let directory = TestDirectory::new();
            let store = initialized_store(&directory);
            let sealed_candidate = sealed(&store);
            let ready = store
                .publish_ready(&sealed_candidate, || Ok(()))
                .expect("publish ready payload");
            let fault =
                test_fault::Guard::install(directory.path(), point, test_fault::Action::Error);
            assert!(store.publish_revision(&ready, || Ok(()), || {}).is_err());
            drop(fault);
            drop(store);
            let catalog = reopen(&directory);
            assert_eq!(catalog.revisions.len(), usize::from(expected_commit));
            assert_eq!(catalog.head() != manifest().root_revision, expected_commit);
        }
    }

    #[test]
    fn rejection_faults_recover_only_absent_or_complete_rejection() {
        let points = [
            (FaultPoint::RejectionStage, false),
            (FaultPoint::RejectionFileSync, false),
            (FaultPoint::RejectionReadOnly, false),
            (FaultPoint::RejectionRevalidate, false),
            (FaultPoint::RejectionLink, false),
            (FaultPoint::RejectionDirectorySync, true),
            (FaultPoint::RejectionLostAcknowledgement, true),
            (FaultPoint::RejectionCleanup, true),
        ];
        let rejection = RejectionFacts {
            workspace: manifest().workspace,
            operation: candidate().operation,
            request_digest: candidate().request_digest,
            reason_code: 3,
            expected_head: [0; REVISION_ID_BYTES],
            actual_head: [0; REVISION_ID_BYTES],
        };
        for (point, expected_rejection) in points {
            let directory = TestDirectory::new();
            let store = initialized_store(&directory);
            let fault =
                test_fault::Guard::install(directory.path(), point, test_fault::Action::Error);
            assert!(store.publish_rejection(rejection, || Ok(())).is_err());
            drop(fault);
            drop(store);
            let catalog = reopen(&directory);
            assert_eq!(
                catalog
                    .operation(rejection.operation)
                    .and_then(|record| record.rejection)
                    .is_some(),
                expected_rejection
            );
            assert!(catalog.revisions.is_empty());
        }
    }

    #[test]
    fn recovery_preflights_rejection_against_single_file_limit() {
        let directory = TestDirectory::new();
        let store = initialized_store(&directory);
        let rejection = RejectionFacts {
            workspace: manifest().workspace,
            operation: candidate().operation,
            request_digest: candidate().request_digest,
            reason_code: 3,
            expected_head: [0; REVISION_ID_BYTES],
            actual_head: [0; REVISION_ID_BYTES],
        };
        store
            .publish_rejection(rejection, || Ok(()))
            .expect("publish rejection");
        drop(store);
        let store = Store::open(directory.path()).expect("reopen Store");
        let mut limits = catalog_limits();
        limits.read.max_file_bytes = REJECTION_BYTES as u64 - 1;
        assert!(matches!(
            store.recover(&manifest(), limits, &OperationControl::new()),
            Err(PersistenceError::Limit { .. })
        ));
    }

    #[test]
    fn injected_panic_after_ready_link_recovers_complete_intent() {
        let directory = TestDirectory::new();
        let store = initialized_store(&directory);
        let sealed_candidate = sealed(&store);
        let fault = test_fault::Guard::install(
            directory.path(),
            FaultPoint::OperationLostAcknowledgement,
            test_fault::Action::Panic,
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.publish_ready(&sealed_candidate, || Ok(()));
        }));
        assert!(result.is_err());
        drop(fault);
        drop(store);
        let catalog = reopen(&directory);
        assert!(
            catalog
                .operation(candidate().operation)
                .and_then(|record| record.ready.as_ref())
                .is_some()
        );
    }

    #[test]
    fn injected_cancellation_at_publication_boundaries_keeps_complete_states() {
        let directory = TestDirectory::new();
        let store = initialized_store(&directory);
        let sealed_candidate = sealed(&store);
        let fault = test_fault::Guard::install(
            directory.path(),
            FaultPoint::OperationsDirectorySync,
            test_fault::Action::Cancel,
        );
        assert!(matches!(
            store.publish_ready(&sealed_candidate, || Ok(())),
            Err(PersistenceError::Cancelled)
        ));
        drop(fault);
        drop(store);
        let catalog = reopen(&directory);
        assert!(
            catalog
                .operation(candidate().operation)
                .and_then(|record| record.ready.as_ref())
                .is_some()
        );
        assert!(catalog.revisions.is_empty());

        let store = Store::open(directory.path()).expect("reopen for Revision cancellation");
        let ready = store
            .recover(&manifest(), catalog_limits(), &OperationControl::new())
            .expect("recover ready")
            .operation(candidate().operation)
            .and_then(|record| record.ready.clone())
            .expect("ready payload");
        let fault = test_fault::Guard::install(
            directory.path(),
            FaultPoint::RevisionDirectorySync,
            test_fault::Action::Cancel,
        );
        assert!(matches!(
            store.publish_revision(&ready, || Ok(()), || {}),
            Err(PersistenceError::Cancelled)
        ));
        drop(fault);
        drop(store);
        assert_eq!(reopen(&directory).revisions.len(), 1);
    }
}
