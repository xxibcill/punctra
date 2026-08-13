// The persisted frame validator and decoder remain linear so the on-disk
// schema can be audited in one place. Intent is deliberately passed by value
// at the publication boundary to make caller ownership explicit.
#![allow(
    clippy::large_types_passed_by_value,
    clippy::needless_pass_by_value,
    clippy::struct_field_names,
    clippy::too_many_lines
)]

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use blake3::Hasher;
use thiserror::Error;

use crate::publication::{
    DirectoryWitness, StageCreationError, StageGuard, create_stage as create_publication_stage,
    same_file_identity, same_file_state, sync_directory,
};

const HEADER_MAGIC: &[u8; 8] = b"PTWFJ001";
const FRAME_MAGIC: &[u8; 4] = b"PWF1";
const DISK_VERSION: u32 = 1;
const SEMANTIC_VERSION: u32 = 1;
const FRAME_VERSION: u16 = 1;
const HEADER_BYTES: usize = 80;
const HEADER_HASH_OFFSET: usize = 48;
const FRAME_HEADER_BYTES: usize = 56;
const FRAME_HASH_BYTES: usize = 32;
const FRAME_OVERHEAD_BYTES: usize = FRAME_HEADER_BYTES + FRAME_HASH_BYTES;
const INTENT_FIXED_BYTES: usize = 452;
const REVISION_BYTES: usize = 96;
const AUDIT_BYTES: usize = 193;
const SURFACE_BYTES: usize = 424;
const QA_BYTES: usize = 136;
const EXPORT_BYTES: usize = 169;
const REPORT_BYTES: usize = 200;
const COMPLETE_BYTES: usize = 224;
const HEADER_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-header-v1";
const FRAME_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-frame-v1";
const REQUEST_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-request-v1";
const ORDINAL_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-ordinals-v1";
const QA_INPUT_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-qa-input-v1";
const RECIPE_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-recipe-v1";
const OPTIONS_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-landxml-options-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationBoundary {
    IntentBeforeLink,
    IntentTargetVerification,
    IntentParentSync,
    IntentStageRemoval,
    IntentCleanupSync,
    IntentTerminalAcknowledgement,
    CheckpointBeforeWrite,
    CheckpointBeforeSync,
    CheckpointAfterSync,
}

trait PublicationHook {
    fn reach(&self, boundary: PublicationBoundary) -> io::Result<()>;
}

struct ProductionPublicationHook;

impl PublicationHook for ProductionPublicationHook {
    fn reach(&self, _boundary: PublicationBoundary) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) type Digest = [u8; 32];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct JournalLimits {
    pub(crate) max_journal_bytes: u64,
    pub(crate) max_frames: u64,
    pub(crate) max_frame_payload_bytes: u64,
    pub(crate) max_working_bytes: u64,
    pub(crate) max_correction_ordinals: u64,
    pub(crate) max_check_points: u64,
    pub(crate) max_surface_name_bytes: u64,
}

impl JournalLimits {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        max_journal_bytes: u64,
        max_frames: u64,
        max_frame_payload_bytes: u64,
        max_working_bytes: u64,
        max_correction_ordinals: u64,
        max_check_points: u64,
        max_surface_name_bytes: u64,
    ) -> Self {
        Self {
            max_journal_bytes,
            max_frames,
            max_frame_payload_bytes,
            max_working_bytes,
            max_correction_ordinals,
            max_check_points,
            max_surface_name_bytes,
        }
    }
}

impl Default for JournalLimits {
    fn default() -> Self {
        Self::new(1024 * 1024, 8, 16 * 1024, 64 * 1024, 1_000, 256, 1_024)
    }
}

/// Caller-owned nonzero identity of one durable terrain Workflow Run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkflowRunId([u8; 16]);

impl WorkflowRunId {
    /// Creates a Workflow Run identity from checked opaque bytes.
    #[must_use]
    pub fn new(bytes: [u8; 16]) -> Option<Self> {
        if bytes == [0; 16] {
            None
        } else {
            Some(Self(bytes))
        }
    }

    /// Borrows the opaque identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Returns the opaque identity bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Display for WorkflowRunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntentCheckPoint {
    pub(crate) id: u64,
    pub(crate) position_bits: [u64; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkflowIntent {
    pub(crate) run: WorkflowRunId,
    pub(crate) request_hash: Digest,
    pub(crate) source: Digest,
    pub(crate) workspace: [u8; 16],
    pub(crate) baseline_revision: Digest,
    pub(crate) operation: [u8; 16],
    pub(crate) correction_ordinals: Box<[u64]>,
    pub(crate) ordinal_hash: Digest,
    pub(crate) ground_classification: u8,
    pub(crate) non_ground_classification: u8,
    pub(crate) recipe_bounds_bits: Option<[[u64; 2]; 3]>,
    pub(crate) recipe_hash: Digest,
    pub(crate) check_points: Box<[IntentCheckPoint]>,
    pub(crate) qa_input_hash: Digest,
    pub(crate) surface_name: Box<str>,
    pub(crate) document_date: Box<str>,
    pub(crate) document_time: Box<str>,
    pub(crate) coordinates_are_metric_metres_asserted: bool,
    pub(crate) options_hash: Digest,
    pub(crate) path_bindings: [Digest; 4],
}

impl WorkflowIntent {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        run: WorkflowRunId,
        source: Digest,
        workspace: [u8; 16],
        baseline_revision: Digest,
        operation: [u8; 16],
        correction_ordinals: Box<[u64]>,
        ground_classification: u8,
        non_ground_classification: u8,
        recipe_bounds_bits: Option<[[u64; 2]; 3]>,
        check_points: Box<[IntentCheckPoint]>,
        surface_name: Box<str>,
        document_date: Box<str>,
        document_time: Box<str>,
        coordinates_are_metric_metres_asserted: bool,
        path_bindings: [Digest; 4],
        limits: JournalLimits,
    ) -> Result<Self, JournalError> {
        let ordinal_hash = hash_ordinals(&correction_ordinals);
        let qa_input_hash = hash_check_points(&check_points);
        let recipe_hash = hash_recipe(ground_classification, recipe_bounds_bits);
        let options_hash = hash_options(
            &surface_name,
            &document_date,
            &document_time,
            coordinates_are_metric_metres_asserted,
        );
        let mut intent = Self {
            run,
            request_hash: [0; 32],
            source,
            workspace,
            baseline_revision,
            operation,
            correction_ordinals,
            ordinal_hash,
            ground_classification,
            non_ground_classification,
            recipe_bounds_bits,
            recipe_hash,
            check_points,
            qa_input_hash,
            surface_name,
            document_date,
            document_time,
            coordinates_are_metric_metres_asserted,
            options_hash,
            path_bindings,
        };
        intent.validate(limits)?;
        intent.request_hash = request_hash(&intent);
        Ok(intent)
    }

    fn validate(&self, limits: JournalLimits) -> Result<(), JournalError> {
        WorkflowRunId::new(self.run.into_bytes())
            .ok_or(JournalError::Invalid("Run Identity is all zero"))?;
        if self.operation == [0; 16] {
            return Err(JournalError::Invalid(
                "Workspace Operation Identity is all zero",
            ));
        }
        if self.source == [0; 32]
            || self.workspace == [0; 16]
            || self.baseline_revision == [0; 32]
            || self.path_bindings.contains(&[0; 32])
        {
            return Err(JournalError::Invalid(
                "Intent contains a reserved zero identity or binding",
            ));
        }
        require(
            as_u64(self.correction_ordinals.len()),
            limits.max_correction_ordinals,
            "correction ordinals",
        )?;
        if self.correction_ordinals.is_empty() {
            return Err(JournalError::Invalid(
                "at least one correction ordinal is required",
            ));
        }
        if !self
            .correction_ordinals
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(JournalError::Invalid(
                "correction ordinals are not sorted and unique",
            ));
        }
        if self.ground_classification == self.non_ground_classification {
            return Err(JournalError::Invalid(
                "Ground and non-Ground classifications are equal",
            ));
        }
        validate_bounds(self.recipe_bounds_bits)?;
        require(
            as_u64(self.check_points.len()),
            limits.max_check_points,
            "detached Check Points",
        )?;
        validate_check_points(&self.check_points)?;
        require(
            as_u64(self.surface_name.len()),
            limits.max_surface_name_bytes,
            "Surface name bytes",
        )?;
        as_u16(self.surface_name.len())?;
        as_u16(self.document_date.len())?;
        as_u16(self.document_time.len())?;
        let payload_bytes = intent_payload_bytes(self)?;
        require(
            payload_bytes,
            limits.max_frame_payload_bytes,
            "journal frame payload bytes",
        )?;
        let create_overlap = payload_bytes
            .saturating_mul(3)
            .saturating_add(as_u64(FRAME_OVERHEAD_BYTES));
        require(
            create_overlap,
            limits.max_working_bytes,
            "journal working bytes",
        )?;
        if self.ordinal_hash != hash_ordinals(&self.correction_ordinals)
            || self.qa_input_hash != hash_check_points(&self.check_points)
            || self.recipe_hash != hash_recipe(self.ground_classification, self.recipe_bounds_bits)
            || self.options_hash
                != hash_options(
                    &self.surface_name,
                    &self.document_date,
                    &self.document_time,
                    self.coordinates_are_metric_metres_asserted,
                )
        {
            return Err(JournalError::Corrupt("Intent canonical input hash differs"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RevisionResolved {
    pub(crate) operation: [u8; 16],
    pub(crate) revision: Digest,
    pub(crate) parent: Digest,
    pub(crate) sequence: u64,
    pub(crate) kind: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuditObserved {
    pub(crate) revision: Digest,
    pub(crate) content_hash: Digest,
    pub(crate) point_id_hash: Digest,
    pub(crate) changed_points: u64,
    pub(crate) transition_count: u32,
    pub(crate) footprint_bits: Option<[[u64; 2]; 3]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SurfaceObserved {
    pub(crate) revision: Digest,
    pub(crate) recipe_hash: Digest,
    pub(crate) baseline_artifact_hash: Digest,
    pub(crate) changed_artifact_hash: Digest,
    pub(crate) baseline_geometry_hash: Digest,
    pub(crate) changed_geometry_hash: Digest,
    pub(crate) baseline_topology_hash: Digest,
    pub(crate) changed_topology_hash: Digest,
    pub(crate) baseline_vertex_count: u64,
    pub(crate) baseline_face_count: u64,
    pub(crate) changed_vertex_count: u64,
    pub(crate) changed_face_count: u64,
    pub(crate) added_face_count: u64,
    pub(crate) removed_face_count: u64,
    pub(crate) added_face_hash: Digest,
    pub(crate) removed_face_hash: Digest,
    pub(crate) envelope_bits: Option<[[u64; 2]; 3]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QaObserved {
    pub(crate) surface_artifact_hash: Digest,
    pub(crate) result_hash: Digest,
    pub(crate) covered_count: u64,
    pub(crate) gap_count: u64,
    pub(crate) face_tests: u64,
    pub(crate) accounted_peak_working_bytes: u64,
    pub(crate) statistic_bits: [u64; 4],
    pub(crate) statistic_mask: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExportEnsured {
    pub(crate) revision: Digest,
    pub(crate) surface_artifact_hash: Digest,
    pub(crate) options_hash: Digest,
    pub(crate) target_binding: Digest,
    pub(crate) content_hash: Digest,
    pub(crate) byte_length: u64,
    pub(crate) outcome: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReportEnsured {
    pub(crate) report_hash: Digest,
    pub(crate) byte_length: u64,
    pub(crate) revision: Digest,
    pub(crate) audit_hash: Digest,
    pub(crate) surface_hash: Digest,
    pub(crate) qa_hash: Digest,
    pub(crate) landxml_hash: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Complete {
    pub(crate) request_hash: Digest,
    pub(crate) revision: Digest,
    pub(crate) audit_hash: Digest,
    pub(crate) surface_hash: Digest,
    pub(crate) qa_hash: Digest,
    pub(crate) landxml_hash: Digest,
    pub(crate) report_hash: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Checkpoint {
    Intent(Box<WorkflowIntent>),
    RevisionResolved(RevisionResolved),
    AuditObserved(AuditObserved),
    SurfaceObserved(SurfaceObserved),
    QaObserved(QaObserved),
    ExportEnsured(ExportEnsured),
    ReportEnsured(ReportEnsured),
    Complete(Complete),
}

impl Checkpoint {
    fn kind(&self) -> FrameKind {
        match self {
            Self::Intent(_) => FrameKind::Intent,
            Self::RevisionResolved(_) => FrameKind::RevisionResolved,
            Self::AuditObserved(_) => FrameKind::AuditObserved,
            Self::SurfaceObserved(_) => FrameKind::SurfaceObserved,
            Self::QaObserved(_) => FrameKind::QaObserved,
            Self::ExportEnsured(_) => FrameKind::ExportEnsured,
            Self::ReportEnsured(_) => FrameKind::ReportEnsured,
            Self::Complete(_) => FrameKind::Complete,
        }
    }
}

fn validate_semantic_chain(checkpoints: &[Checkpoint]) -> Result<(), JournalError> {
    if !matches!(checkpoints.first(), Some(Checkpoint::Intent(_))) {
        return Err(JournalError::Corrupt("journal does not begin with Intent"));
    }
    for index in 1..checkpoints.len() {
        validate_checkpoint_link(&checkpoints[..index], &checkpoints[index])?;
    }
    Ok(())
}

fn validate_checkpoint_link(
    prior: &[Checkpoint],
    checkpoint: &Checkpoint,
) -> Result<(), JournalError> {
    let Some(Checkpoint::Intent(intent)) = prior.first() else {
        return Err(JournalError::Corrupt("checkpoint has no Intent"));
    };
    match checkpoint {
        Checkpoint::Intent(_) => {
            return Err(JournalError::Corrupt("journal contains a second Intent"));
        }
        Checkpoint::RevisionResolved(revision) => {
            if prior.len() != 1
                || revision.operation != intent.operation
                || revision.parent != intent.baseline_revision
                || revision.revision == [0; 32]
                || revision.sequence == 0
                || revision.kind != 1
            {
                return Err(JournalError::Corrupt(
                    "RevisionResolved does not link to Intent",
                ));
            }
        }
        Checkpoint::AuditObserved(audit) => {
            let Some(Checkpoint::RevisionResolved(revision)) = prior.get(1) else {
                return Err(JournalError::Corrupt(
                    "AuditObserved has no RevisionResolved",
                ));
            };
            if prior.len() != 2
                || audit.revision != revision.revision
                || audit.content_hash == [0; 32]
                || audit.point_id_hash == [0; 32]
                || audit.changed_points != as_u64(intent.correction_ordinals.len())
                || audit.transition_count != 1
            {
                return Err(JournalError::Corrupt(
                    "AuditObserved does not link to RevisionResolved",
                ));
            }
        }
        Checkpoint::SurfaceObserved(surface) => {
            let Some(Checkpoint::RevisionResolved(revision)) = prior.get(1) else {
                return Err(JournalError::Corrupt(
                    "SurfaceObserved has no RevisionResolved",
                ));
            };
            if prior.len() != 3
                || surface.revision != revision.revision
                || surface.recipe_hash != intent.recipe_hash
                || surface.baseline_artifact_hash == [0; 32]
                || surface.changed_artifact_hash == [0; 32]
                || surface.baseline_geometry_hash == [0; 32]
                || surface.changed_geometry_hash == [0; 32]
                || surface.baseline_topology_hash == [0; 32]
                || surface.changed_topology_hash == [0; 32]
                || surface
                    .added_face_count
                    .checked_add(surface.removed_face_count)
                    .is_none()
                || ((surface.added_face_count == 0 && surface.removed_face_count == 0)
                    != surface.envelope_bits.is_none())
            {
                return Err(JournalError::Corrupt(
                    "SurfaceObserved does not link to prior facts",
                ));
            }
        }
        Checkpoint::QaObserved(qa) => {
            let Some(Checkpoint::SurfaceObserved(surface)) = prior.get(3) else {
                return Err(JournalError::Corrupt("QaObserved has no SurfaceObserved"));
            };
            let expected_mask = if qa.covered_count == 0 { 0 } else { 0b1111 };
            let statistics_valid = qa.statistic_bits.iter().enumerate().all(|(index, bits)| {
                let present = qa.statistic_mask & (1 << index) != 0;
                !present || f64::from_bits(*bits).is_finite()
            });
            if prior.len() != 4
                || qa.surface_artifact_hash != surface.changed_artifact_hash
                || qa.result_hash == [0; 32]
                || qa
                    .covered_count
                    .checked_add(qa.gap_count)
                    .is_none_or(|count| count != as_u64(intent.check_points.len()))
                || qa.statistic_mask != expected_mask
                || !statistics_valid
            {
                return Err(JournalError::Corrupt(
                    "QaObserved does not link to SurfaceObserved",
                ));
            }
        }
        Checkpoint::ExportEnsured(export) => {
            let Some(Checkpoint::RevisionResolved(revision)) = prior.get(1) else {
                return Err(JournalError::Corrupt(
                    "ExportEnsured has no RevisionResolved",
                ));
            };
            let Some(Checkpoint::SurfaceObserved(surface)) = prior.get(3) else {
                return Err(JournalError::Corrupt(
                    "ExportEnsured has no SurfaceObserved",
                ));
            };
            if prior.len() != 5
                || export.revision != revision.revision
                || export.surface_artifact_hash != surface.changed_artifact_hash
                || export.options_hash != intent.options_hash
                || export.target_binding != intent.path_bindings[3]
                || export.content_hash == [0; 32]
                || export.byte_length == 0
                || export.outcome != 1
            {
                return Err(JournalError::Corrupt(
                    "ExportEnsured is not the stable ensured_exact fact",
                ));
            }
        }
        Checkpoint::ReportEnsured(report) => {
            let Some(Checkpoint::RevisionResolved(revision)) = prior.get(1) else {
                return Err(JournalError::Corrupt(
                    "ReportEnsured has no RevisionResolved",
                ));
            };
            let Some(Checkpoint::AuditObserved(audit)) = prior.get(2) else {
                return Err(JournalError::Corrupt("ReportEnsured has no AuditObserved"));
            };
            let Some(Checkpoint::SurfaceObserved(surface)) = prior.get(3) else {
                return Err(JournalError::Corrupt(
                    "ReportEnsured has no SurfaceObserved",
                ));
            };
            let Some(Checkpoint::QaObserved(qa)) = prior.get(4) else {
                return Err(JournalError::Corrupt("ReportEnsured has no QaObserved"));
            };
            let Some(Checkpoint::ExportEnsured(export)) = prior.get(5) else {
                return Err(JournalError::Corrupt("ReportEnsured has no ExportEnsured"));
            };
            if prior.len() != 6
                || report.report_hash == [0; 32]
                || report.byte_length == 0
                || report.revision != revision.revision
                || report.audit_hash != audit.content_hash
                || report.surface_hash != surface.changed_artifact_hash
                || report.qa_hash != qa.result_hash
                || report.landxml_hash != export.content_hash
            {
                return Err(JournalError::Corrupt(
                    "ReportEnsured does not link to all prior facts",
                ));
            }
        }
        Checkpoint::Complete(complete) => {
            let Some(Checkpoint::RevisionResolved(revision)) = prior.get(1) else {
                return Err(JournalError::Corrupt("Complete has no RevisionResolved"));
            };
            let Some(Checkpoint::AuditObserved(audit)) = prior.get(2) else {
                return Err(JournalError::Corrupt("Complete has no AuditObserved"));
            };
            let Some(Checkpoint::SurfaceObserved(surface)) = prior.get(3) else {
                return Err(JournalError::Corrupt("Complete has no SurfaceObserved"));
            };
            let Some(Checkpoint::QaObserved(qa)) = prior.get(4) else {
                return Err(JournalError::Corrupt("Complete has no QaObserved"));
            };
            let Some(Checkpoint::ExportEnsured(export)) = prior.get(5) else {
                return Err(JournalError::Corrupt("Complete has no ExportEnsured"));
            };
            let Some(Checkpoint::ReportEnsured(report)) = prior.get(6) else {
                return Err(JournalError::Corrupt("Complete has no ReportEnsured"));
            };
            if prior.len() != 7
                || complete.request_hash != intent.request_hash
                || complete.revision != revision.revision
                || complete.audit_hash != audit.content_hash
                || complete.surface_hash != surface.changed_artifact_hash
                || complete.qa_hash != qa.result_hash
                || complete.landxml_hash != export.content_hash
                || complete.report_hash != report.report_hash
            {
                return Err(JournalError::Corrupt(
                    "Complete does not link to all prior facts",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
enum FrameKind {
    Intent = 1,
    RevisionResolved = 2,
    AuditObserved = 3,
    SurfaceObserved = 4,
    QaObserved = 5,
    ExportEnsured = 6,
    ReportEnsured = 7,
    Complete = 8,
}

impl FrameKind {
    fn decode(value: u16) -> Result<Self, JournalError> {
        match value {
            1 => Ok(Self::Intent),
            2 => Ok(Self::RevisionResolved),
            3 => Ok(Self::AuditObserved),
            4 => Ok(Self::SurfaceObserved),
            5 => Ok(Self::QaObserved),
            6 => Ok(Self::ExportEnsured),
            7 => Ok(Self::ReportEnsured),
            8 => Ok(Self::Complete),
            _ => Err(JournalError::Incompatible("unknown journal frame kind")),
        }
    }

    const fn sequence(self) -> u64 {
        self as u64 - 1
    }
}

#[derive(Debug)]
pub(crate) struct Journal {
    path: PathBuf,
    file: LockedFile,
    identity: fs::Metadata,
    limits: JournalLimits,
    run: WorkflowRunId,
    checkpoints: Vec<Checkpoint>,
    previous_hash: Digest,
    end: u64,
    poisoned: bool,
}

pub(crate) struct SealedJournal {
    path: PathBuf,
    file: LockedFile,
    identity: fs::Metadata,
    run: WorkflowRunId,
    checkpoints: Vec<Checkpoint>,
    content_hash: Digest,
    byte_length: u64,
}

struct DecodedJournal {
    run: WorkflowRunId,
    checkpoints: Vec<Checkpoint>,
    previous_hash: Digest,
    end: u64,
    file_bytes: u64,
}

#[derive(Debug)]
struct LockedFile(File);

impl LockedFile {
    fn acquire(file: File, path: &Path) -> Result<Self, JournalError> {
        file.try_lock()
            .map_err(|error| map_lock_error(path, error))?;
        Ok(Self(file))
    }
}

impl Deref for LockedFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for LockedFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for LockedFile {
    fn drop(&mut self) {
        let _ = File::unlock(&self.0);
    }
}

impl Journal {
    pub(crate) fn create(
        path: &Path,
        intent: WorkflowIntent,
        limits: JournalLimits,
    ) -> Result<Self, JournalError> {
        Self::create_with_hook(path, intent, limits, &ProductionPublicationHook)
    }

    fn create_with_hook(
        path: &Path,
        intent: WorkflowIntent,
        limits: JournalLimits,
        hook: &impl PublicationHook,
    ) -> Result<Self, JournalError> {
        validate_limits(limits)?;
        intent.validate(limits)?;
        if request_hash(&intent) != intent.request_hash {
            return Err(JournalError::Corrupt("Intent request hash differs"));
        }
        require_absent(path)?;
        let parent = target_parent(path);
        let parent_witness = DirectoryWitness::capture(parent)
            .map_err(|source| JournalError::io("witness journal parent", parent, source))?;
        let (mut guard, stage) = create_stage(parent)?;
        let mut stage = LockedFile::acquire(stage, guard.path())?;
        let header = encode_header(intent.run);
        let header_hash = copy_digest(&header[HEADER_HASH_OFFSET..]);
        let checkpoint = Checkpoint::Intent(Box::new(intent.clone()));
        let frame = encode_frame(&checkpoint, 0, header_hash, limits)?;
        let total = as_u64(HEADER_BYTES).saturating_add(as_u64(frame.len()));
        require(total, limits.max_journal_bytes, "journal bytes")?;
        write_all(&mut stage, &header, guard.path())?;
        write_all(&mut stage, &frame, guard.path())?;
        stage
            .sync_all()
            .map_err(|source| JournalError::io("sync journal stage", guard.path(), source))?;
        drop(frame);
        drop(checkpoint);
        validate_stage(&mut stage, &intent, limits, total)?;
        hook.reach(PublicationBoundary::IntentBeforeLink)
            .map_err(|source| JournalError::io("run journal pre-link boundary", path, source))?;
        guard
            .verify()
            .map_err(|source| JournalError::io("verify journal stage", guard.path(), source))?;
        parent_witness
            .verify()
            .map_err(|source| JournalError::io("revalidate journal parent", parent, source))?;
        match fs::hard_link(guard.path(), path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                return Err(JournalError::Exists(path.to_path_buf()));
            }
            Err(source) => return Err(JournalError::io("publish journal", path, source)),
        }
        finish_journal_publication(
            JournalPublication {
                path,
                parent,
                parent_witness: &parent_witness,
                intent: &intent,
                limits,
                bytes: total,
            },
            &mut guard,
            hook,
        )?;
        drop(stage);
        Self::open(path, limits)
    }

    pub(crate) fn open(path: &Path, limits: JournalLimits) -> Result<Self, JournalError> {
        validate_limits(limits)?;
        let target_identity = require_regular_file(path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| JournalError::io("open journal", path, source))?;
        let mut file = LockedFile::acquire(file, path)?;
        let identity = file
            .metadata()
            .map_err(|source| JournalError::io("inspect journal", path, source))?;
        verify_recognized_journal(&file, path, &target_identity)
            .map_err(|source| JournalError::io("verify opened journal target", path, source))?;
        let decoded = decode_journal(&mut file, path, limits, identity.len())?;
        if decoded.end != decoded.file_bytes {
            file.set_len(decoded.end)
                .map_err(|source| JournalError::io("truncate torn journal tail", path, source))?;
            file.sync_data()
                .map_err(|source| JournalError::io("sync repaired journal", path, source))?;
        }
        verify_recognized_journal(&file, path, &identity)
            .map_err(|source| JournalError::io("revalidate opened journal target", path, source))?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
            limits,
            run: decoded.run,
            checkpoints: decoded.checkpoints,
            previous_hash: decoded.previous_hash,
            end: decoded.end,
            poisoned: false,
        })
    }

    pub(crate) const fn run(&self) -> WorkflowRunId {
        self.run
    }

    pub(crate) fn intent(&self) -> &WorkflowIntent {
        match &self.checkpoints[0] {
            Checkpoint::Intent(intent) => intent,
            _ => unreachable!("open validates Intent"),
        }
    }

    pub(crate) fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    pub(crate) fn retained_bytes(&self) -> u64 {
        retained_checkpoint_bytes(&self.checkpoints)
    }

    pub(crate) fn record(&mut self, checkpoint: Checkpoint) -> Result<bool, JournalError> {
        self.record_with_hook(checkpoint, &ProductionPublicationHook)
    }

    fn record_with_hook(
        &mut self,
        checkpoint: Checkpoint,
        hook: &impl PublicationHook,
    ) -> Result<bool, JournalError> {
        if self.poisoned {
            return Err(JournalError::Invalid(
                "journal append certainty is unknown; drop and reopen it",
            ));
        }
        let sequence = as_u64(self.checkpoints.len());
        let checkpoint_sequence = checkpoint.kind().sequence();
        if checkpoint_sequence < sequence {
            return if self.checkpoints.get(as_usize(checkpoint_sequence)?) == Some(&checkpoint) {
                Ok(false)
            } else {
                Err(JournalError::Conflict(
                    "checkpoint differs from durable observation",
                ))
            };
        }
        if checkpoint_sequence != sequence {
            return Err(JournalError::Invalid(
                "checkpoint would create a sequence gap",
            ));
        }
        validate_checkpoint_link(&self.checkpoints, &checkpoint)?;
        require(
            sequence.saturating_add(1),
            self.limits.max_frames,
            "journal frames",
        )?;
        let predicted_frame_bytes =
            checkpoint_payload_bytes(&checkpoint)?.saturating_add(as_u64(FRAME_OVERHEAD_BYTES));
        require(
            retained_checkpoint_bytes(&self.checkpoints)
                .saturating_add(as_u64(std::mem::size_of::<Checkpoint>()))
                .saturating_add(predicted_frame_bytes),
            self.limits.max_working_bytes,
            "journal working bytes",
        )?;
        let frame = encode_frame(&checkpoint, sequence, self.previous_hash, self.limits)?;
        require(
            retained_checkpoint_bytes(&self.checkpoints)
                .saturating_add(as_u64(std::mem::size_of::<Checkpoint>()))
                .saturating_add(as_u64(frame.capacity())),
            self.limits.max_working_bytes,
            "journal working bytes",
        )?;
        let required = self
            .end
            .checked_add(as_u64(frame.len()))
            .ok_or(JournalError::Resource {
                limit: "journal bytes",
                required: u64::MAX,
                allowed: self.limits.max_journal_bytes,
            })?;
        require(required, self.limits.max_journal_bytes, "journal bytes")?;
        self.verify_recognized_path().map_err(|source| {
            JournalError::io("verify journal append target", &self.path, source)
        })?;
        self.file
            .seek(SeekFrom::Start(self.end))
            .map_err(|source| JournalError::io("seek journal append", &self.path, source))?;
        let expected_hash = copy_digest(&frame[frame.len() - FRAME_HASH_BYTES..]);
        hook.reach(PublicationBoundary::CheckpointBeforeWrite)
            .map_err(|source| {
                JournalError::io("run journal pre-append boundary", &self.path, source)
            })?;
        self.verify_recognized_path().map_err(|source| {
            JournalError::io("revalidate journal append target", &self.path, source)
        })?;
        self.poisoned = true;
        self.file
            .write_all(&frame)
            .map_err(|source| JournalError::CheckpointIndeterminate {
                path: self.path.clone(),
                kind: checkpoint.kind() as u16,
                sequence,
                expected_hash,
                source,
            })?;
        hook.reach(PublicationBoundary::CheckpointBeforeSync)
            .map_err(|source| JournalError::CheckpointIndeterminate {
                path: self.path.clone(),
                kind: checkpoint.kind() as u16,
                sequence,
                expected_hash,
                source,
            })?;
        self.file
            .sync_data()
            .map_err(|source| JournalError::CheckpointIndeterminate {
                path: self.path.clone(),
                kind: checkpoint.kind() as u16,
                sequence,
                expected_hash,
                source,
            })?;
        hook.reach(PublicationBoundary::CheckpointAfterSync)
            .map_err(|source| JournalError::CheckpointIndeterminate {
                path: self.path.clone(),
                kind: checkpoint.kind() as u16,
                sequence,
                expected_hash,
                source,
            })?;
        self.verify_recognized_path()
            .map_err(|source| JournalError::CheckpointIndeterminate {
                path: self.path.clone(),
                kind: checkpoint.kind() as u16,
                sequence,
                expected_hash,
                source,
            })?;
        self.poisoned = false;
        self.previous_hash = copy_digest(&frame[frame.len() - FRAME_HASH_BYTES..]);
        self.end = required;
        self.checkpoints.push(checkpoint);
        Ok(true)
    }

    fn verify_recognized_path(&self) -> io::Result<()> {
        verify_recognized_journal(&self.file, &self.path, &self.identity)
    }
}

impl SealedJournal {
    pub(crate) fn open(path: &Path, limits: JournalLimits) -> Result<Self, JournalError> {
        validate_limits(limits)?;
        let target_identity = require_regular_file(path)?;
        let file = File::open(path)
            .map_err(|source| JournalError::io("open sealed journal", path, source))?;
        let mut file = LockedFile::acquire(file, path)?;
        let identity = file
            .metadata()
            .map_err(|source| JournalError::io("inspect sealed journal", path, source))?;
        verify_recognized_journal(&file, path, &target_identity).map_err(|source| {
            JournalError::io("verify opened sealed journal target", path, source)
        })?;
        let decoded = decode_journal(&mut file, path, limits, identity.len())?;
        if decoded.end != decoded.file_bytes {
            return Err(JournalError::Corrupt(
                "sealed journal has a torn trailing frame",
            ));
        }
        if !matches!(decoded.checkpoints.last(), Some(Checkpoint::Complete(_))) {
            return Err(JournalError::Invalid("workflow Run is not Complete"));
        }
        let content_hash = hash_open_journal(&mut file, path, decoded.file_bytes, limits)?;
        verify_recognized_journal(&file, path, &identity)
            .map_err(|source| JournalError::io("revalidate sealed journal target", path, source))?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
            run: decoded.run,
            checkpoints: decoded.checkpoints,
            content_hash,
            byte_length: decoded.file_bytes,
        })
    }

    pub(crate) const fn run(&self) -> WorkflowRunId {
        self.run
    }

    pub(crate) fn intent(&self) -> &WorkflowIntent {
        match &self.checkpoints[0] {
            Checkpoint::Intent(intent) => intent,
            _ => unreachable!("sealed open validates Intent"),
        }
    }

    pub(crate) fn complete(&self) -> Complete {
        match self.checkpoints.last() {
            Some(Checkpoint::Complete(complete)) => *complete,
            _ => unreachable!("sealed open requires Complete"),
        }
    }

    pub(crate) fn export(&self) -> ExportEnsured {
        match self.checkpoints.get(5) {
            Some(Checkpoint::ExportEnsured(export)) => *export,
            _ => unreachable!("sealed open validates the eight-frame chain"),
        }
    }

    pub(crate) fn report(&self) -> ReportEnsured {
        match self.checkpoints.get(6) {
            Some(Checkpoint::ReportEnsured(report)) => *report,
            _ => unreachable!("sealed open validates the eight-frame chain"),
        }
    }

    pub(crate) const fn content_hash(&self) -> Digest {
        self.content_hash
    }

    pub(crate) const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub(crate) fn verify(&self) -> Result<(), JournalError> {
        verify_recognized_journal(&self.file, &self.path, &self.identity).map_err(|source| {
            JournalError::io("revalidate sealed journal target", &self.path, source)
        })?;
        let current = self
            .file
            .metadata()
            .map_err(|source| JournalError::io("reinspect sealed journal", &self.path, source))?;
        if current.len() != self.byte_length || !same_file_state(&self.identity, &current) {
            return Err(JournalError::Corrupt(
                "sealed journal changed during qualification",
            ));
        }
        Ok(())
    }
}

fn decode_journal(
    file: &mut File,
    path: &Path,
    limits: JournalLimits,
    file_bytes: u64,
) -> Result<DecodedJournal, JournalError> {
    require(file_bytes, limits.max_journal_bytes, "journal bytes")?;
    let (run, header_hash) = read_header(file, path)?;
    let mut scan = scan_frames(file, path, limits, file_bytes, header_hash, run)?;
    let Some(Checkpoint::Intent(intent)) = scan.checkpoints.first_mut() else {
        return Err(JournalError::Corrupt("journal does not begin with Intent"));
    };
    intent.run = run;
    intent.validate(limits)?;
    if request_hash(intent) != intent.request_hash {
        return Err(JournalError::Corrupt("Intent request hash differs"));
    }
    validate_semantic_chain(&scan.checkpoints)?;
    Ok(DecodedJournal {
        run,
        checkpoints: scan.checkpoints,
        previous_hash: scan.previous_hash,
        end: scan.end,
        file_bytes,
    })
}

fn hash_open_journal(
    file: &mut File,
    path: &Path,
    expected_bytes: u64,
    limits: JournalLimits,
) -> Result<Digest, JournalError> {
    let buffer_bytes = usize::try_from(limits.max_working_bytes.min(64 * 1024)).unwrap_or(0);
    if buffer_bytes == 0 {
        return Err(JournalError::Resource {
            limit: "journal hashing working bytes",
            required: 1,
            allowed: limits.max_working_bytes,
        });
    }
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(buffer_bytes)
        .map_err(|_| JournalError::Resource {
            limit: "journal hashing working bytes",
            required: buffer_bytes as u64,
            allowed: limits.max_working_bytes,
        })?;
    buffer.resize(buffer_bytes, 0);
    file.seek(SeekFrom::Start(0))
        .map_err(|source| JournalError::io("rewind sealed journal", path, source))?;
    let mut hasher = Hasher::new();
    let mut bytes = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| JournalError::io("hash sealed journal", path, source))?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read as u64);
        require(bytes, limits.max_journal_bytes, "journal bytes")?;
        hasher.update(&buffer[..read]);
    }
    if bytes != expected_bytes {
        return Err(JournalError::Corrupt(
            "sealed journal length changed while hashing",
        ));
    }
    Ok(*hasher.finalize().as_bytes())
}

#[derive(Debug, Error)]
pub(crate) enum JournalError {
    #[error("invalid workflow journal: {0}")]
    Invalid(&'static str),
    #[error("corrupt workflow journal: {0}")]
    Corrupt(&'static str),
    #[error("incompatible workflow journal: {0}")]
    Incompatible(&'static str),
    #[error("workflow journal exceeded {limit}: required {required}, limit {allowed}")]
    Resource {
        limit: &'static str,
        required: u64,
        allowed: u64,
    },
    #[error("workflow journal already exists: {0}")]
    Exists(PathBuf),
    #[error("workflow journal is locked by another process: {0}")]
    Locked(PathBuf),
    #[error("workflow journal checkpoint conflicts: {0}")]
    Conflict(&'static str),
    #[error("system randomness is unavailable")]
    Entropy,
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("journal publication is indeterminate for {path}")]
    Indeterminate {
        path: PathBuf,
        run: WorkflowRunId,
        request_hash: Digest,
        #[source]
        source: io::Error,
    },
    #[error("journal checkpoint publication is indeterminate for {path} sequence {sequence}")]
    CheckpointIndeterminate {
        path: PathBuf,
        kind: u16,
        sequence: u64,
        expected_hash: Digest,
        #[source]
        source: io::Error,
    },
}

impl JournalError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

struct Scan {
    checkpoints: Vec<Checkpoint>,
    previous_hash: Digest,
    end: u64,
}

fn scan_frames(
    file: &mut File,
    path: &Path,
    limits: JournalLimits,
    file_bytes: u64,
    header_hash: Digest,
    run: WorkflowRunId,
) -> Result<Scan, JournalError> {
    let mut checkpoints = Vec::new();
    let retained_slots = as_usize(limits.max_frames.min(8))?;
    checkpoints
        .try_reserve_exact(retained_slots)
        .map_err(|_| JournalError::Resource {
            limit: "journal working bytes",
            required: as_u64(retained_slots)
                .saturating_mul(as_u64(std::mem::size_of::<Checkpoint>())),
            allowed: limits.max_working_bytes,
        })?;
    let retained_capacity_bytes =
        as_u64(checkpoints.capacity()).saturating_mul(as_u64(std::mem::size_of::<Checkpoint>()));
    let mut retained_dynamic_bytes = 0_u64;
    require(
        retained_capacity_bytes,
        limits.max_working_bytes,
        "journal working bytes",
    )?;
    let mut previous_hash = header_hash;
    let mut offset = as_u64(HEADER_BYTES);
    while offset < file_bytes {
        let remaining = file_bytes - offset;
        if remaining < as_u64(FRAME_OVERHEAD_BYTES) {
            break;
        }
        require(
            as_u64(checkpoints.len()).saturating_add(1),
            limits.max_frames,
            "journal frames",
        )?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|source| JournalError::io("seek journal frame", path, source))?;
        let mut header = [0_u8; FRAME_HEADER_BYTES];
        read_exact(file, &mut header, path)?;
        let decoded = decode_frame_header(&header, previous_hash, checkpoints.len())?;
        let payload_bytes = u64::from(decoded.payload_bytes);
        validate_declared_payload(decoded.kind, payload_bytes)?;
        require(
            payload_bytes,
            limits.max_frame_payload_bytes,
            "journal frame payload bytes",
        )?;
        let frame_bytes = payload_bytes.saturating_add(as_u64(FRAME_OVERHEAD_BYTES));
        let decode_overlap = if decoded.kind == FrameKind::Intent {
            payload_bytes.saturating_mul(3)
        } else {
            payload_bytes
        };
        require(
            retained_capacity_bytes
                .saturating_add(retained_dynamic_bytes)
                .saturating_add(decode_overlap)
                .saturating_add(as_u64(FRAME_OVERHEAD_BYTES)),
            limits.max_working_bytes,
            "journal working bytes",
        )?;
        if frame_bytes > remaining {
            break;
        }
        let payload_len = as_usize(payload_bytes)?;
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(payload_len)
            .map_err(|_| JournalError::Resource {
                limit: "journal working bytes",
                required: payload_bytes,
                allowed: limits.max_working_bytes,
            })?;
        require(
            retained_capacity_bytes
                .saturating_add(retained_dynamic_bytes)
                .saturating_add(as_u64(payload.capacity()))
                .saturating_add(if decoded.kind == FrameKind::Intent {
                    payload_bytes.saturating_mul(2)
                } else {
                    0
                })
                .saturating_add(as_u64(FRAME_OVERHEAD_BYTES)),
            limits.max_working_bytes,
            "journal working bytes",
        )?;
        payload.resize(payload_len, 0);
        read_exact(file, &mut payload, path)?;
        let mut recorded_hash = [0; FRAME_HASH_BYTES];
        read_exact(file, &mut recorded_hash, path)?;
        let actual_hash = frame_hash(&header, &payload);
        if recorded_hash != actual_hash {
            return Err(JournalError::Corrupt("journal frame checksum differs"));
        }
        let checkpoint = decode_checkpoint(decoded.kind, &payload, limits, run)?;
        retained_dynamic_bytes =
            retained_dynamic_bytes.saturating_add(checkpoint_dynamic_bytes(&checkpoint));
        require(
            retained_capacity_bytes.saturating_add(retained_dynamic_bytes),
            limits.max_working_bytes,
            "journal working bytes",
        )?;
        checkpoints.push(checkpoint);
        previous_hash = actual_hash;
        offset = offset.saturating_add(frame_bytes);
    }
    Ok(Scan {
        checkpoints,
        previous_hash,
        end: offset,
    })
}

fn validate_declared_payload(kind: FrameKind, payload_bytes: u64) -> Result<(), JournalError> {
    let expected = match kind {
        FrameKind::Intent => {
            if payload_bytes < as_u64(INTENT_FIXED_BYTES) {
                return Err(JournalError::Corrupt("Intent payload width is too small"));
            }
            return Ok(());
        }
        FrameKind::RevisionResolved => REVISION_BYTES,
        FrameKind::AuditObserved => AUDIT_BYTES,
        FrameKind::SurfaceObserved => SURFACE_BYTES,
        FrameKind::QaObserved => QA_BYTES,
        FrameKind::ExportEnsured => EXPORT_BYTES,
        FrameKind::ReportEnsured => REPORT_BYTES,
        FrameKind::Complete => COMPLETE_BYTES,
    };
    if payload_bytes == as_u64(expected) {
        Ok(())
    } else {
        Err(JournalError::Corrupt(
            "fixed checkpoint payload width differs",
        ))
    }
}

struct DecodedFrameHeader {
    kind: FrameKind,
    payload_bytes: u32,
}

fn decode_frame_header(
    bytes: &[u8; FRAME_HEADER_BYTES],
    expected_previous: Digest,
    expected_sequence: usize,
) -> Result<DecodedFrameHeader, JournalError> {
    if &bytes[..4] != FRAME_MAGIC {
        return Err(JournalError::Corrupt("journal frame magic differs"));
    }
    if u16::from_le_bytes(copy_array(&bytes[4..6])) != FRAME_VERSION {
        return Err(JournalError::Incompatible("journal frame version differs"));
    }
    let kind = FrameKind::decode(u16::from_le_bytes(copy_array(&bytes[6..8])))?;
    let sequence = u64::from_le_bytes(copy_array(&bytes[8..16]));
    if sequence != as_u64(expected_sequence) || sequence != kind.sequence() {
        return Err(JournalError::Corrupt(
            "journal frame sequence or kind order differs",
        ));
    }
    if bytes[20..24] != [0; 4] {
        return Err(JournalError::Incompatible(
            "journal frame reserved bytes are nonzero",
        ));
    }
    if bytes[24..56] != expected_previous {
        return Err(JournalError::Corrupt("journal frame hash chain differs"));
    }
    Ok(DecodedFrameHeader {
        kind,
        payload_bytes: u32::from_le_bytes(copy_array(&bytes[16..20])),
    })
}

fn encode_frame(
    checkpoint: &Checkpoint,
    sequence: u64,
    previous_hash: Digest,
    limits: JournalLimits,
) -> Result<Vec<u8>, JournalError> {
    let payload_bytes = checkpoint_payload_bytes(checkpoint)?;
    require(
        payload_bytes,
        limits.max_frame_payload_bytes,
        "journal frame payload bytes",
    )?;
    let frame_bytes = payload_bytes.saturating_add(as_u64(FRAME_OVERHEAD_BYTES));
    require(
        frame_bytes,
        limits.max_working_bytes,
        "journal working bytes",
    )?;
    let mut header = [0; FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(FRAME_MAGIC);
    header[4..6].copy_from_slice(&FRAME_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&(checkpoint.kind() as u16).to_le_bytes());
    header[8..16].copy_from_slice(&sequence.to_le_bytes());
    header[16..20].copy_from_slice(
        &u32::try_from(payload_bytes)
            .map_err(|_| JournalError::Resource {
                limit: "journal frame payload bytes",
                required: payload_bytes,
                allowed: limits.max_frame_payload_bytes,
            })?
            .to_le_bytes(),
    );
    header[24..56].copy_from_slice(&previous_hash);
    let total_bytes = payload_bytes.saturating_add(as_u64(FRAME_OVERHEAD_BYTES));
    let mut frame = Vec::new();
    let total_bytes_usize = as_usize(total_bytes)?;
    frame
        .try_reserve_exact(total_bytes_usize)
        .map_err(|_| JournalError::Resource {
            limit: "journal working bytes",
            required: total_bytes,
            allowed: limits.max_working_bytes,
        })?;
    require(
        as_u64(frame.capacity()),
        limits.max_working_bytes,
        "journal working bytes",
    )?;
    frame.extend_from_slice(&header);
    encode_checkpoint_into(checkpoint, &mut frame)?;
    debug_assert_eq!(
        as_u64(frame.len()),
        as_u64(FRAME_HEADER_BYTES) + payload_bytes
    );
    let checksum = frame_hash(&header, &frame[FRAME_HEADER_BYTES..]);
    frame.extend_from_slice(&checksum);
    Ok(frame)
}

fn checkpoint_payload_bytes(checkpoint: &Checkpoint) -> Result<u64, JournalError> {
    match checkpoint {
        Checkpoint::Intent(intent) => intent_payload_bytes(intent),
        Checkpoint::RevisionResolved(_) => Ok(as_u64(REVISION_BYTES)),
        Checkpoint::AuditObserved(_) => Ok(as_u64(AUDIT_BYTES)),
        Checkpoint::SurfaceObserved(_) => Ok(as_u64(SURFACE_BYTES)),
        Checkpoint::QaObserved(_) => Ok(as_u64(QA_BYTES)),
        Checkpoint::ExportEnsured(_) => Ok(as_u64(EXPORT_BYTES)),
        Checkpoint::ReportEnsured(_) => Ok(as_u64(REPORT_BYTES)),
        Checkpoint::Complete(_) => Ok(as_u64(COMPLETE_BYTES)),
    }
}

fn checkpoint_dynamic_bytes(checkpoint: &Checkpoint) -> u64 {
    match checkpoint {
        Checkpoint::Intent(intent) => as_u64(intent.correction_ordinals.len())
            .saturating_mul(8)
            .saturating_add(
                as_u64(intent.check_points.len())
                    .saturating_mul(as_u64(std::mem::size_of::<IntentCheckPoint>())),
            )
            .saturating_add(as_u64(intent.surface_name.len()))
            .saturating_add(as_u64(intent.document_date.len()))
            .saturating_add(as_u64(intent.document_time.len()))
            .saturating_add(as_u64(std::mem::size_of::<WorkflowIntent>())),
        _ => 0,
    }
}

fn retained_checkpoint_bytes(checkpoints: &Vec<Checkpoint>) -> u64 {
    as_u64(checkpoints.capacity())
        .saturating_mul(as_u64(std::mem::size_of::<Checkpoint>()))
        .saturating_add(
            checkpoints
                .iter()
                .map(checkpoint_dynamic_bytes)
                .fold(0_u64, u64::saturating_add),
        )
}

fn intent_payload_bytes(intent: &WorkflowIntent) -> Result<u64, JournalError> {
    let ordinals = as_u64(intent.correction_ordinals.len())
        .checked_mul(8)
        .ok_or(JournalError::Resource {
            limit: "Intent payload bytes",
            required: u64::MAX,
            allowed: u64::MAX - 1,
        })?;
    let check_points =
        as_u64(intent.check_points.len())
            .checked_mul(32)
            .ok_or(JournalError::Resource {
                limit: "Intent payload bytes",
                required: u64::MAX,
                allowed: u64::MAX - 1,
            })?;
    let text = as_u64(intent.surface_name.len())
        .checked_add(as_u64(intent.document_date.len()))
        .and_then(|value| value.checked_add(as_u64(intent.document_time.len())))
        .ok_or(JournalError::Resource {
            limit: "Intent payload bytes",
            required: u64::MAX,
            allowed: u64::MAX - 1,
        })?;
    as_u64(INTENT_FIXED_BYTES)
        .checked_add(ordinals)
        .and_then(|value| value.checked_add(check_points))
        .and_then(|value| value.checked_add(text))
        .ok_or(JournalError::Resource {
            limit: "Intent payload bytes",
            required: u64::MAX,
            allowed: u64::MAX - 1,
        })
}

fn encode_checkpoint_into(
    checkpoint: &Checkpoint,
    bytes: &mut Vec<u8>,
) -> Result<(), JournalError> {
    let start = bytes.len();
    match checkpoint {
        Checkpoint::Intent(value) => encode_intent(value, bytes)?,
        Checkpoint::RevisionResolved(value) => encode_revision(*value, bytes),
        Checkpoint::AuditObserved(value) => encode_audit(*value, bytes),
        Checkpoint::SurfaceObserved(value) => encode_surface(*value, bytes),
        Checkpoint::QaObserved(value) => encode_qa(*value, bytes),
        Checkpoint::ExportEnsured(value) => encode_export(*value, bytes),
        Checkpoint::ReportEnsured(value) => encode_report(*value, bytes),
        Checkpoint::Complete(value) => encode_complete(*value, bytes),
    }
    debug_assert_eq!(
        as_u64(bytes.len().saturating_sub(start)),
        checkpoint_payload_bytes(checkpoint)?
    );
    Ok(())
}

fn decode_checkpoint(
    kind: FrameKind,
    bytes: &[u8],
    limits: JournalLimits,
    run: WorkflowRunId,
) -> Result<Checkpoint, JournalError> {
    match kind {
        FrameKind::Intent => {
            decode_intent(bytes, limits, run).map(|value| Checkpoint::Intent(Box::new(value)))
        }
        FrameKind::RevisionResolved => decode_revision(bytes).map(Checkpoint::RevisionResolved),
        FrameKind::AuditObserved => decode_audit(bytes).map(Checkpoint::AuditObserved),
        FrameKind::SurfaceObserved => decode_surface(bytes).map(Checkpoint::SurfaceObserved),
        FrameKind::QaObserved => decode_qa(bytes).map(Checkpoint::QaObserved),
        FrameKind::ExportEnsured => decode_export(bytes).map(Checkpoint::ExportEnsured),
        FrameKind::ReportEnsured => decode_report(bytes).map(Checkpoint::ReportEnsured),
        FrameKind::Complete => decode_complete(bytes).map(Checkpoint::Complete),
    }
}

fn encode_intent(value: &WorkflowIntent, bytes: &mut Vec<u8>) -> Result<(), JournalError> {
    bytes.extend_from_slice(&value.request_hash);
    bytes.extend_from_slice(&value.source);
    bytes.extend_from_slice(&value.workspace);
    bytes.extend_from_slice(&value.baseline_revision);
    bytes.extend_from_slice(&value.operation);
    bytes.extend_from_slice(&value.ordinal_hash);
    bytes.extend_from_slice(&value.qa_input_hash);
    bytes.extend_from_slice(&value.recipe_hash);
    bytes.extend_from_slice(&value.options_hash);
    for binding in value.path_bindings {
        bytes.extend_from_slice(&binding);
    }
    bytes.push(value.ground_classification);
    bytes.push(value.non_ground_classification);
    bytes.push(u8::from(value.recipe_bounds_bits.is_some()));
    bytes.push(u8::from(value.coordinates_are_metric_metres_asserted));
    encode_optional_bounds(value.recipe_bounds_bits, bytes);
    push_u32(bytes, as_u32(value.correction_ordinals.len())?);
    push_u32(bytes, as_u32(value.check_points.len())?);
    push_u16(bytes, as_u16(value.surface_name.len())?);
    push_u16(bytes, as_u16(value.document_date.len())?);
    push_u16(bytes, as_u16(value.document_time.len())?);
    push_u16(bytes, 0);
    for ordinal in &value.correction_ordinals {
        push_u64(bytes, *ordinal);
    }
    for check_point in &value.check_points {
        push_u64(bytes, check_point.id);
        for coordinate in check_point.position_bits {
            push_u64(bytes, coordinate);
        }
    }
    bytes.extend_from_slice(value.surface_name.as_bytes());
    bytes.extend_from_slice(value.document_date.as_bytes());
    bytes.extend_from_slice(value.document_time.as_bytes());
    Ok(())
}

fn decode_intent(
    bytes: &[u8],
    limits: JournalLimits,
    run: WorkflowRunId,
) -> Result<WorkflowIntent, JournalError> {
    if bytes.len() < INTENT_FIXED_BYTES {
        return Err(JournalError::Corrupt("Intent payload is truncated"));
    }
    let ordinal_count = usize::try_from(u32::from_le_bytes(copy_array(&bytes[436..440])))
        .map_err(|_| JournalError::Corrupt("Intent ordinal count is not addressable"))?;
    let check_point_count = usize::try_from(u32::from_le_bytes(copy_array(&bytes[440..444])))
        .map_err(|_| JournalError::Corrupt("Intent Check Point count is not addressable"))?;
    require(
        as_u64(ordinal_count),
        limits.max_correction_ordinals,
        "correction ordinals",
    )?;
    require(
        as_u64(check_point_count),
        limits.max_check_points,
        "detached Check Points",
    )?;
    let name_bytes = usize::from(u16::from_le_bytes(copy_array(&bytes[444..446])));
    let date_bytes = usize::from(u16::from_le_bytes(copy_array(&bytes[446..448])));
    let time_bytes = usize::from(u16::from_le_bytes(copy_array(&bytes[448..450])));
    if bytes[450..452] != [0; 2] {
        return Err(JournalError::Incompatible(
            "Intent reserved bytes are nonzero",
        ));
    }
    require(
        as_u64(name_bytes),
        limits.max_surface_name_bytes,
        "Surface name bytes",
    )?;
    let variable = ordinal_count
        .checked_mul(8)
        .and_then(|value| value.checked_add(check_point_count.checked_mul(32)?))
        .and_then(|value| value.checked_add(name_bytes))
        .and_then(|value| value.checked_add(date_bytes))
        .and_then(|value| value.checked_add(time_bytes))
        .ok_or(JournalError::Corrupt("Intent payload length overflowed"))?;
    if bytes.len() != INTENT_FIXED_BYTES.saturating_add(variable) {
        return Err(JournalError::Corrupt("Intent payload length differs"));
    }
    let mut offset = INTENT_FIXED_BYTES;
    let mut ordinals = Vec::new();
    ordinals
        .try_reserve_exact(ordinal_count)
        .map_err(|_| JournalError::Resource {
            limit: "journal working bytes",
            required: as_u64(ordinal_count).saturating_mul(8),
            allowed: limits.max_working_bytes,
        })?;
    for _ in 0..ordinal_count {
        ordinals.push(take_u64(bytes, &mut offset)?);
    }
    let mut check_points = Vec::new();
    check_points
        .try_reserve_exact(check_point_count)
        .map_err(|_| JournalError::Resource {
            limit: "journal working bytes",
            required: as_u64(check_point_count)
                .saturating_mul(as_u64(std::mem::size_of::<IntentCheckPoint>())),
            allowed: limits.max_working_bytes,
        })?;
    for _ in 0..check_point_count {
        check_points.push(IntentCheckPoint {
            id: take_u64(bytes, &mut offset)?,
            position_bits: [
                take_u64(bytes, &mut offset)?,
                take_u64(bytes, &mut offset)?,
                take_u64(bytes, &mut offset)?,
            ],
        });
    }
    let surface_name = take_utf8(bytes, &mut offset, name_bytes)?;
    let document_date = take_utf8(bytes, &mut offset, date_bytes)?;
    let document_time = take_utf8(bytes, &mut offset, time_bytes)?;
    let bounds = decode_optional_bounds(bytes[386], &bytes[388..436])?;
    let intent = WorkflowIntent {
        run,
        request_hash: copy_digest(&bytes[0..32]),
        source: copy_digest(&bytes[32..64]),
        workspace: copy_array(&bytes[64..80]),
        baseline_revision: copy_digest(&bytes[80..112]),
        operation: copy_array(&bytes[112..128]),
        ordinal_hash: copy_digest(&bytes[128..160]),
        qa_input_hash: copy_digest(&bytes[160..192]),
        recipe_hash: copy_digest(&bytes[192..224]),
        options_hash: copy_digest(&bytes[224..256]),
        path_bindings: [
            copy_digest(&bytes[256..288]),
            copy_digest(&bytes[288..320]),
            copy_digest(&bytes[320..352]),
            copy_digest(&bytes[352..384]),
        ],
        ground_classification: bytes[384],
        non_ground_classification: bytes[385],
        recipe_bounds_bits: bounds,
        coordinates_are_metric_metres_asserted: decode_bool(bytes[387])?,
        correction_ordinals: ordinals.into_boxed_slice(),
        check_points: check_points.into_boxed_slice(),
        surface_name,
        document_date,
        document_time,
    };
    Ok(intent)
}

fn encode_revision(value: RevisionResolved, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&value.operation);
    bytes.extend_from_slice(&value.revision);
    bytes.extend_from_slice(&value.parent);
    push_u64(bytes, value.sequence);
    bytes.push(value.kind);
    bytes.extend_from_slice(&[0; 7]);
}

fn decode_revision(bytes: &[u8]) -> Result<RevisionResolved, JournalError> {
    exact(bytes, REVISION_BYTES)?;
    zeroes(&bytes[89..96], "Revision reserved bytes")?;
    Ok(RevisionResolved {
        operation: copy_array(&bytes[0..16]),
        revision: copy_digest(&bytes[16..48]),
        parent: copy_digest(&bytes[48..80]),
        sequence: u64::from_le_bytes(copy_array(&bytes[80..88])),
        kind: bytes[88],
    })
}

fn encode_audit(value: AuditObserved, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&value.revision);
    bytes.extend_from_slice(&value.content_hash);
    bytes.extend_from_slice(&value.point_id_hash);
    push_u64(bytes, value.changed_points);
    push_u32(bytes, value.transition_count);
    bytes.push(u8::from(value.footprint_bits.is_some()));
    encode_optional_bounds(value.footprint_bits, bytes);
    bytes.extend_from_slice(&[0; 36]);
}

fn decode_audit(bytes: &[u8]) -> Result<AuditObserved, JournalError> {
    exact(bytes, AUDIT_BYTES)?;
    zeroes(&bytes[157..193], "Audit reserved bytes")?;
    Ok(AuditObserved {
        revision: copy_digest(&bytes[0..32]),
        content_hash: copy_digest(&bytes[32..64]),
        point_id_hash: copy_digest(&bytes[64..96]),
        changed_points: u64::from_le_bytes(copy_array(&bytes[96..104])),
        transition_count: u32::from_le_bytes(copy_array(&bytes[104..108])),
        footprint_bits: decode_optional_bounds(bytes[108], &bytes[109..157])?,
    })
}

fn encode_surface(value: SurfaceObserved, bytes: &mut Vec<u8>) {
    for digest in [
        value.revision,
        value.recipe_hash,
        value.baseline_artifact_hash,
        value.changed_artifact_hash,
        value.baseline_geometry_hash,
        value.changed_geometry_hash,
        value.baseline_topology_hash,
        value.changed_topology_hash,
    ] {
        bytes.extend_from_slice(&digest);
    }
    for count in [
        value.baseline_vertex_count,
        value.baseline_face_count,
        value.changed_vertex_count,
        value.changed_face_count,
        value.added_face_count,
        value.removed_face_count,
    ] {
        push_u64(bytes, count);
    }
    bytes.extend_from_slice(&value.added_face_hash);
    bytes.extend_from_slice(&value.removed_face_hash);
    bytes.push(u8::from(value.envelope_bits.is_some()));
    encode_optional_bounds(value.envelope_bits, bytes);
    bytes.extend_from_slice(&[0; 7]);
}

fn decode_surface(bytes: &[u8]) -> Result<SurfaceObserved, JournalError> {
    exact(bytes, SURFACE_BYTES)?;
    zeroes(&bytes[417..424], "Surface reserved bytes")?;
    Ok(SurfaceObserved {
        revision: copy_digest(&bytes[0..32]),
        recipe_hash: copy_digest(&bytes[32..64]),
        baseline_artifact_hash: copy_digest(&bytes[64..96]),
        changed_artifact_hash: copy_digest(&bytes[96..128]),
        baseline_geometry_hash: copy_digest(&bytes[128..160]),
        changed_geometry_hash: copy_digest(&bytes[160..192]),
        baseline_topology_hash: copy_digest(&bytes[192..224]),
        changed_topology_hash: copy_digest(&bytes[224..256]),
        baseline_vertex_count: u64::from_le_bytes(copy_array(&bytes[256..264])),
        baseline_face_count: u64::from_le_bytes(copy_array(&bytes[264..272])),
        changed_vertex_count: u64::from_le_bytes(copy_array(&bytes[272..280])),
        changed_face_count: u64::from_le_bytes(copy_array(&bytes[280..288])),
        added_face_count: u64::from_le_bytes(copy_array(&bytes[288..296])),
        removed_face_count: u64::from_le_bytes(copy_array(&bytes[296..304])),
        added_face_hash: copy_digest(&bytes[304..336]),
        removed_face_hash: copy_digest(&bytes[336..368]),
        envelope_bits: decode_optional_bounds(bytes[368], &bytes[369..417])?,
    })
}

fn encode_qa(value: QaObserved, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&value.surface_artifact_hash);
    bytes.extend_from_slice(&value.result_hash);
    for count in [
        value.covered_count,
        value.gap_count,
        value.face_tests,
        value.accounted_peak_working_bytes,
    ] {
        push_u64(bytes, count);
    }
    for bits in value.statistic_bits {
        push_u64(bytes, bits);
    }
    bytes.push(value.statistic_mask);
    bytes.extend_from_slice(&[0; 7]);
}

fn decode_qa(bytes: &[u8]) -> Result<QaObserved, JournalError> {
    exact(bytes, QA_BYTES)?;
    zeroes(&bytes[129..136], "QA reserved bytes")?;
    Ok(QaObserved {
        surface_artifact_hash: copy_digest(&bytes[0..32]),
        result_hash: copy_digest(&bytes[32..64]),
        covered_count: u64::from_le_bytes(copy_array(&bytes[64..72])),
        gap_count: u64::from_le_bytes(copy_array(&bytes[72..80])),
        face_tests: u64::from_le_bytes(copy_array(&bytes[80..88])),
        accounted_peak_working_bytes: u64::from_le_bytes(copy_array(&bytes[88..96])),
        statistic_bits: [
            u64::from_le_bytes(copy_array(&bytes[96..104])),
            u64::from_le_bytes(copy_array(&bytes[104..112])),
            u64::from_le_bytes(copy_array(&bytes[112..120])),
            u64::from_le_bytes(copy_array(&bytes[120..128])),
        ],
        statistic_mask: bytes[128],
    })
}

fn encode_export(value: ExportEnsured, bytes: &mut Vec<u8>) {
    for digest in [
        value.revision,
        value.surface_artifact_hash,
        value.options_hash,
        value.target_binding,
        value.content_hash,
    ] {
        bytes.extend_from_slice(&digest);
    }
    push_u64(bytes, value.byte_length);
    bytes.push(value.outcome);
}

fn decode_export(bytes: &[u8]) -> Result<ExportEnsured, JournalError> {
    exact(bytes, EXPORT_BYTES)?;
    Ok(ExportEnsured {
        revision: copy_digest(&bytes[0..32]),
        surface_artifact_hash: copy_digest(&bytes[32..64]),
        options_hash: copy_digest(&bytes[64..96]),
        target_binding: copy_digest(&bytes[96..128]),
        content_hash: copy_digest(&bytes[128..160]),
        byte_length: u64::from_le_bytes(copy_array(&bytes[160..168])),
        outcome: bytes[168],
    })
}

fn encode_report(value: ReportEnsured, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&value.report_hash);
    push_u64(bytes, value.byte_length);
    for digest in [
        value.revision,
        value.audit_hash,
        value.surface_hash,
        value.qa_hash,
        value.landxml_hash,
    ] {
        bytes.extend_from_slice(&digest);
    }
}

fn decode_report(bytes: &[u8]) -> Result<ReportEnsured, JournalError> {
    exact(bytes, REPORT_BYTES)?;
    Ok(ReportEnsured {
        report_hash: copy_digest(&bytes[0..32]),
        byte_length: u64::from_le_bytes(copy_array(&bytes[32..40])),
        revision: copy_digest(&bytes[40..72]),
        audit_hash: copy_digest(&bytes[72..104]),
        surface_hash: copy_digest(&bytes[104..136]),
        qa_hash: copy_digest(&bytes[136..168]),
        landxml_hash: copy_digest(&bytes[168..200]),
    })
}

fn encode_complete(value: Complete, bytes: &mut Vec<u8>) {
    for digest in [
        value.request_hash,
        value.revision,
        value.audit_hash,
        value.surface_hash,
        value.qa_hash,
        value.landxml_hash,
        value.report_hash,
    ] {
        bytes.extend_from_slice(&digest);
    }
}

fn decode_complete(bytes: &[u8]) -> Result<Complete, JournalError> {
    exact(bytes, COMPLETE_BYTES)?;
    Ok(Complete {
        request_hash: copy_digest(&bytes[0..32]),
        revision: copy_digest(&bytes[32..64]),
        audit_hash: copy_digest(&bytes[64..96]),
        surface_hash: copy_digest(&bytes[96..128]),
        qa_hash: copy_digest(&bytes[128..160]),
        landxml_hash: copy_digest(&bytes[160..192]),
        report_hash: copy_digest(&bytes[192..224]),
    })
}

fn encode_header(run: WorkflowRunId) -> [u8; HEADER_BYTES] {
    let mut bytes = [0; HEADER_BYTES];
    bytes[..8].copy_from_slice(HEADER_MAGIC);
    bytes[8..12].copy_from_slice(&DISK_VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&SEMANTIC_VERSION.to_le_bytes());
    bytes[16..20].copy_from_slice(
        &as_u32(HEADER_BYTES)
            .expect("header width fits")
            .to_le_bytes(),
    );
    bytes[24..40].copy_from_slice(&run.into_bytes());
    let checksum = domain_hash(HEADER_HASH_DOMAIN, &bytes[..HEADER_HASH_OFFSET]);
    bytes[HEADER_HASH_OFFSET..].copy_from_slice(&checksum);
    bytes
}

fn read_header(file: &mut File, path: &Path) -> Result<(WorkflowRunId, Digest), JournalError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| JournalError::io("seek journal header", path, source))?;
    let mut bytes = [0; HEADER_BYTES];
    read_exact(file, &mut bytes, path)?;
    if &bytes[..8] != HEADER_MAGIC {
        return Err(JournalError::Corrupt("journal header magic differs"));
    }
    if u32::from_le_bytes(copy_array(&bytes[8..12])) != DISK_VERSION
        || u32::from_le_bytes(copy_array(&bytes[12..16])) != SEMANTIC_VERSION
    {
        return Err(JournalError::Incompatible(
            "journal disk or semantic version differs",
        ));
    }
    if u32::from_le_bytes(copy_array(&bytes[16..20])) != as_u32(HEADER_BYTES)?
        || bytes[20..24] != [0; 4]
        || bytes[40..48] != [0; 8]
    {
        return Err(JournalError::Incompatible(
            "journal header width or reserved bytes differ",
        ));
    }
    let expected = domain_hash(HEADER_HASH_DOMAIN, &bytes[..HEADER_HASH_OFFSET]);
    let recorded = copy_digest(&bytes[HEADER_HASH_OFFSET..]);
    if expected != recorded {
        return Err(JournalError::Corrupt("journal header checksum differs"));
    }
    let run = WorkflowRunId::new(copy_array(&bytes[24..40]))
        .ok_or(JournalError::Invalid("Run Identity is all zero"))?;
    Ok((run, recorded))
}

fn validate_stage(
    file: &mut File,
    intent: &WorkflowIntent,
    limits: JournalLimits,
    bytes: u64,
) -> Result<(), JournalError> {
    let (run, header_hash) = read_header(file, Path::new("journal stage"))?;
    let mut scan = scan_frames(
        file,
        Path::new("journal stage"),
        limits,
        bytes,
        header_hash,
        run,
    )?;
    if let Some(Checkpoint::Intent(decoded)) = scan.checkpoints.first_mut() {
        decoded.run = run;
        decoded.validate(limits)?;
        if request_hash(decoded) != decoded.request_hash {
            return Err(JournalError::Corrupt("staged Intent request hash differs"));
        }
    }
    if run != intent.run
        || scan.checkpoints != [Checkpoint::Intent(Box::new(intent.clone()))]
        || scan.end != bytes
    {
        return Err(JournalError::Corrupt(
            "staged journal differs after verification",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct JournalPublication<'a> {
    path: &'a Path,
    parent: &'a Path,
    parent_witness: &'a DirectoryWitness,
    intent: &'a WorkflowIntent,
    limits: JournalLimits,
    bytes: u64,
}

fn finish_journal_publication(
    publication: JournalPublication<'_>,
    stage: &mut StageGuard,
    hook: &impl PublicationHook,
) -> Result<(), JournalError> {
    require_intent_boundary(
        publication,
        hook,
        PublicationBoundary::IntentTargetVerification,
    )?;
    publication
        .parent_witness
        .verify()
        .map_err(|source| intent_indeterminate(publication, source))?;
    verify_linked_journal(publication, stage)
        .map_err(|source| intent_indeterminate(publication, source))?;
    require_intent_boundary(publication, hook, PublicationBoundary::IntentParentSync)?;
    sync_directory(publication.parent).map_err(|source| JournalError::Indeterminate {
        path: publication.path.to_path_buf(),
        run: publication.intent.run,
        request_hash: publication.intent.request_hash,
        source,
    })?;
    require_intent_boundary(publication, hook, PublicationBoundary::IntentStageRemoval)?;
    stage
        .remove()
        .map_err(|source| intent_indeterminate(publication, source))?;
    require_intent_boundary(publication, hook, PublicationBoundary::IntentCleanupSync)?;
    sync_directory(publication.parent).map_err(|source| JournalError::Indeterminate {
        path: publication.path.to_path_buf(),
        run: publication.intent.run,
        request_hash: publication.intent.request_hash,
        source,
    })?;
    publication
        .parent_witness
        .verify()
        .map_err(|source| intent_indeterminate(publication, source))?;
    require_intent_boundary(
        publication,
        hook,
        PublicationBoundary::IntentTerminalAcknowledgement,
    )
}

fn require_intent_boundary(
    publication: JournalPublication<'_>,
    hook: &impl PublicationHook,
    boundary: PublicationBoundary,
) -> Result<(), JournalError> {
    hook.reach(boundary)
        .map_err(|source| intent_indeterminate(publication, source))
}

fn intent_indeterminate(publication: JournalPublication<'_>, source: io::Error) -> JournalError {
    JournalError::Indeterminate {
        path: publication.path.to_path_buf(),
        run: publication.intent.run,
        request_hash: publication.intent.request_hash,
        source,
    }
}

fn verify_linked_journal(
    publication: JournalPublication<'_>,
    stage: &StageGuard,
) -> io::Result<()> {
    stage.verify()?;
    let stage_metadata = fs::symlink_metadata(stage.path())?;
    let target_metadata = fs::symlink_metadata(publication.path)?;
    if !target_metadata.file_type().is_file()
        || !same_file_identity(&stage_metadata, &target_metadata)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published journal target identity differs",
        ));
    }
    let mut target = File::open(publication.path)?;
    validate_stage(
        &mut target,
        publication.intent,
        publication.limits,
        publication.bytes,
    )
    .map_err(io::Error::other)?;
    let stage_after = fs::symlink_metadata(stage.path())?;
    let target_after = fs::symlink_metadata(publication.path)?;
    if !same_file_identity(&stage_metadata, &stage_after)
        || !same_file_identity(&stage_after, &target_after)
        || stage_after.len() != publication.bytes
        || target_after.len() != publication.bytes
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published journal target changed during verification",
        ));
    }
    Ok(())
}

fn request_hash(intent: &WorkflowIntent) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(REQUEST_HASH_DOMAIN);
    for digest in [
        intent.source,
        intent.baseline_revision,
        intent.ordinal_hash,
        intent.qa_input_hash,
        intent.recipe_hash,
        intent.options_hash,
    ] {
        hasher.update(&digest);
    }
    hasher.update(&intent.workspace);
    hasher.update(&intent.operation);
    hasher.update(&[
        intent.ground_classification,
        intent.non_ground_classification,
        u8::from(intent.coordinates_are_metric_metres_asserted),
    ]);
    for binding in intent.path_bindings {
        hasher.update(&binding);
    }
    hasher.update(&as_u64(intent.correction_ordinals.len()).to_le_bytes());
    for ordinal in &intent.correction_ordinals {
        hasher.update(&ordinal.to_le_bytes());
    }
    hasher.update(&as_u64(intent.check_points.len()).to_le_bytes());
    for check_point in &intent.check_points {
        hasher.update(&check_point.id.to_le_bytes());
        for bits in check_point.position_bits {
            hasher.update(&bits.to_le_bytes());
        }
    }
    for text in [
        intent.surface_name.as_bytes(),
        intent.document_date.as_bytes(),
        intent.document_time.as_bytes(),
    ] {
        hasher.update(&as_u64(text.len()).to_le_bytes());
        hasher.update(text);
    }
    *hasher.finalize().as_bytes()
}

fn hash_ordinals(ordinals: &[u64]) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(ORDINAL_HASH_DOMAIN);
    hasher.update(&as_u64(ordinals.len()).to_le_bytes());
    for ordinal in ordinals {
        hasher.update(&ordinal.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn hash_check_points(check_points: &[IntentCheckPoint]) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(QA_INPUT_HASH_DOMAIN);
    hasher.update(&as_u64(check_points.len()).to_le_bytes());
    for check_point in check_points {
        hasher.update(&check_point.id.to_le_bytes());
        for bits in check_point.position_bits {
            hasher.update(&bits.to_le_bytes());
        }
    }
    *hasher.finalize().as_bytes()
}

fn hash_recipe(ground: u8, bounds: Option<[[u64; 2]; 3]>) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(RECIPE_HASH_DOMAIN);
    hasher.update(&[ground, u8::from(bounds.is_some())]);
    for axis in bounds.unwrap_or([[0; 2]; 3]) {
        hasher.update(&axis[0].to_le_bytes());
        hasher.update(&axis[1].to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn hash_options(
    name: &str,
    date: &str,
    time: &str,
    coordinates_are_metric_metres_asserted: bool,
) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(OPTIONS_HASH_DOMAIN);
    for value in [name.as_bytes(), date.as_bytes(), time.as_bytes()] {
        hasher.update(&as_u64(value.len()).to_le_bytes());
        hasher.update(value);
    }
    hasher.update(&[u8::from(coordinates_are_metric_metres_asserted)]);
    *hasher.finalize().as_bytes()
}

fn validate_bounds(bounds: Option<[[u64; 2]; 3]>) -> Result<(), JournalError> {
    if let Some(bounds) = bounds {
        for [minimum, maximum] in bounds {
            let minimum = f64::from_bits(minimum);
            let maximum = f64::from_bits(maximum);
            if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
                return Err(JournalError::Invalid("Recipe bounds are invalid"));
            }
        }
    }
    Ok(())
}

fn validate_check_points(check_points: &[IntentCheckPoint]) -> Result<(), JournalError> {
    for (index, check_point) in check_points.iter().enumerate() {
        if check_point.id == 0
            || check_point
                .position_bits
                .iter()
                .any(|bits| !f64::from_bits(*bits).is_finite())
        {
            return Err(JournalError::Invalid("detached Check Point is invalid"));
        }
        if check_points[..index]
            .iter()
            .any(|earlier| earlier.id == check_point.id)
        {
            return Err(JournalError::Invalid(
                "detached Check Point identities are not unique",
            ));
        }
    }
    Ok(())
}

fn decode_optional_bounds(tag: u8, bytes: &[u8]) -> Result<Option<[[u64; 2]; 3]>, JournalError> {
    exact(bytes, 48)?;
    match tag {
        0 => {
            zeroes(bytes, "absent bounds bytes")?;
            Ok(None)
        }
        1 => {
            let bounds = Some([
                [
                    u64::from_le_bytes(copy_array(&bytes[0..8])),
                    u64::from_le_bytes(copy_array(&bytes[8..16])),
                ],
                [
                    u64::from_le_bytes(copy_array(&bytes[16..24])),
                    u64::from_le_bytes(copy_array(&bytes[24..32])),
                ],
                [
                    u64::from_le_bytes(copy_array(&bytes[32..40])),
                    u64::from_le_bytes(copy_array(&bytes[40..48])),
                ],
            ]);
            validate_bounds(bounds)?;
            Ok(bounds)
        }
        _ => Err(JournalError::Corrupt("bounds presence tag is invalid")),
    }
}

fn encode_optional_bounds(bounds: Option<[[u64; 2]; 3]>, bytes: &mut Vec<u8>) {
    for axis in bounds.unwrap_or([[0; 2]; 3]) {
        push_u64(bytes, axis[0]);
        push_u64(bytes, axis[1]);
    }
}

fn decode_bool(value: u8) -> Result<bool, JournalError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(JournalError::Corrupt("boolean Intent field is invalid")),
    }
}

fn frame_hash(header: &[u8; FRAME_HEADER_BYTES], payload: &[u8]) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(FRAME_HASH_DOMAIN);
    hasher.update(header);
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

pub(crate) fn bind_path(path: &Path, max_bytes: u64) -> Result<Digest, JournalError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    require(as_u64(bytes.len()), max_bytes, "path binding bytes")?;
    let mut hasher = Hasher::new();
    hasher.update(b"punctra-terrain-workflow-path-v1");
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update(&[0]);
    hasher.update(bytes);
    Ok(*hasher.finalize().as_bytes())
}

fn validate_limits(limits: JournalLimits) -> Result<(), JournalError> {
    require(8, limits.max_frames, "journal frames")?;
    require(
        as_u64(INTENT_FIXED_BYTES),
        limits.max_frame_payload_bytes,
        "journal frame payload bytes",
    )?;
    require(
        as_u64(INTENT_FIXED_BYTES + FRAME_OVERHEAD_BYTES),
        limits.max_working_bytes,
        "journal working bytes",
    )?;
    require(
        as_u64(HEADER_BYTES + FRAME_OVERHEAD_BYTES + INTENT_FIXED_BYTES),
        limits.max_journal_bytes,
        "journal bytes",
    )
}

fn exact(bytes: &[u8], expected: usize) -> Result<(), JournalError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(JournalError::Corrupt("checkpoint payload width differs"))
    }
}

fn zeroes(bytes: &[u8], name: &'static str) -> Result<(), JournalError> {
    if bytes.iter().all(|byte| *byte == 0) {
        Ok(())
    } else {
        Err(JournalError::Incompatible(name))
    }
}

fn require(required: u64, allowed: u64, limit: &'static str) -> Result<(), JournalError> {
    if required > allowed {
        Err(JournalError::Resource {
            limit,
            required,
            allowed,
        })
    } else {
        Ok(())
    }
}

fn require_absent(path: &Path) -> Result<(), JournalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(JournalError::Exists(path.to_path_buf())),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(JournalError::io("inspect journal target", path, source)),
    }
}

fn require_regular_file(path: &Path) -> Result<fs::Metadata, JournalError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| JournalError::io("inspect journal", path, source))?;
    if metadata.file_type().is_file() {
        Ok(metadata)
    } else {
        Err(JournalError::Invalid("journal is not a regular file"))
    }
}

fn verify_recognized_journal(file: &File, path: &Path, identity: &fs::Metadata) -> io::Result<()> {
    let opened = file.metadata()?;
    let target = fs::symlink_metadata(path)?;
    if opened.file_type().is_file()
        && target.file_type().is_file()
        && same_file_identity(identity, &opened)
        && same_file_identity(&opened, &target)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recognized journal target identity changed",
        ))
    }
}

fn create_stage(parent: &Path) -> Result<(StageGuard, File), JournalError> {
    create_publication_stage(
        parent,
        "workflow",
        || Ok(()),
        |error| match error {
            StageCreationError::RandomnessUnavailable | StageCreationError::NamespaceExhausted => {
                JournalError::Entropy
            }
            StageCreationError::Inspect { path, source } => {
                JournalError::io("inspect journal stage", &path, source)
            }
            StageCreationError::Create { path, source } => {
                JournalError::io("create journal stage", &path, source)
            }
        },
    )
}

fn map_lock_error(path: &Path, error: std::fs::TryLockError) -> JournalError {
    let source: io::Error = error.into();
    if source.kind() == io::ErrorKind::WouldBlock {
        JournalError::Locked(path.to_path_buf())
    } else {
        JournalError::io("lock journal", path, source)
    }
}

fn target_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn write_all(file: &mut File, bytes: &[u8], path: &Path) -> Result<(), JournalError> {
    file.write_all(bytes)
        .map_err(|source| JournalError::io("write journal", path, source))
}

fn read_exact(file: &mut File, bytes: &mut [u8], path: &Path) -> Result<(), JournalError> {
    file.read_exact(bytes)
        .map_err(|source| JournalError::io("read journal", path, source))
}

fn take_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, JournalError> {
    let end = offset
        .checked_add(8)
        .ok_or(JournalError::Corrupt("Intent offset overflowed"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or(JournalError::Corrupt("Intent value is truncated"))?;
    *offset = end;
    Ok(u64::from_le_bytes(copy_array(value)))
}

fn take_utf8(bytes: &[u8], offset: &mut usize, length: usize) -> Result<Box<str>, JournalError> {
    let end = offset
        .checked_add(length)
        .ok_or(JournalError::Corrupt("Intent text offset overflowed"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or(JournalError::Corrupt("Intent text is truncated"))?;
    *offset = end;
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map(String::into_boxed_str)
        .map_err(|_| JournalError::Corrupt("Intent text is not UTF-8"))
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn as_usize(value: u64) -> Result<usize, JournalError> {
    usize::try_from(value).map_err(|_| JournalError::Resource {
        limit: "addressable journal bytes",
        required: value,
        allowed: usize::MAX as u64,
    })
}

fn as_u32(value: usize) -> Result<u32, JournalError> {
    u32::try_from(value).map_err(|_| JournalError::Resource {
        limit: "journal field bytes",
        required: as_u64(value),
        allowed: u64::from(u32::MAX),
    })
}

fn as_u16(value: usize) -> Result<u16, JournalError> {
    u16::try_from(value).map_err(|_| JournalError::Resource {
        limit: "journal text bytes",
        required: as_u64(value),
        allowed: u64::from(u16::MAX),
    })
}

fn copy_digest(bytes: &[u8]) -> Digest {
    copy_array(bytes)
}

fn copy_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes.try_into().expect("validated fixed-width slice")
}

#[cfg(test)]
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("String writes cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write as _, path::PathBuf};

    use super::*;

    #[derive(Clone, Copy)]
    enum TestAction<'a> {
        FailAt(PublicationBoundary),
        InstallAt {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
            replace: bool,
        },
    }

    struct TestHook<'a>(TestAction<'a>);

    impl PublicationHook for TestHook<'_> {
        fn reach(&self, boundary: PublicationBoundary) -> io::Result<()> {
            match self.0 {
                TestAction::FailAt(expected) if boundary == expected => Err(io::Error::other(
                    format!("injected journal failure at {boundary:?}"),
                )),
                TestAction::InstallAt {
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
                _ => Ok(()),
            }
        }
    }

    #[test]
    fn every_intent_publication_boundary_is_old_or_complete() {
        let directory = Directory::new("intent-boundaries");
        let pre_link = directory.path.join("pre-link.pwf");
        let failure = Journal::create_with_hook(
            &pre_link,
            intent(),
            JournalLimits::default(),
            &TestHook(TestAction::FailAt(PublicationBoundary::IntentBeforeLink)),
        )
        .expect_err("a pre-link failure cannot create a Run");
        assert!(matches!(failure, JournalError::Io { .. }));
        assert!(!pre_link.exists());
        directory.assert_no_stages();

        for boundary in [
            PublicationBoundary::IntentTargetVerification,
            PublicationBoundary::IntentParentSync,
            PublicationBoundary::IntentStageRemoval,
            PublicationBoundary::IntentCleanupSync,
            PublicationBoundary::IntentTerminalAcknowledgement,
        ] {
            let path = directory.path.join(format!("{boundary:?}.pwf"));
            let expected_intent = intent();
            let failure = Journal::create_with_hook(
                &path,
                expected_intent.clone(),
                JournalLimits::default(),
                &TestHook(TestAction::FailAt(boundary)),
            )
            .expect_err("post-link failure cannot acknowledge the Intent");
            assert!(matches!(
                failure,
                JournalError::Indeterminate {
                    run,
                    request_hash,
                    ..
                } if run == expected_intent.run
                    && request_hash == expected_intent.request_hash
            ));
            let reopened = Journal::open(&path, JournalLimits::default())
                .expect("a post-link journal is complete and recoverable");
            assert_eq!(reopened.intent(), &expected_intent);
            drop(reopened);
            directory.assert_no_stages();
        }
    }

    #[test]
    fn checkpoint_write_sync_and_lost_ack_are_old_or_complete() {
        let directory = Directory::new("checkpoint-boundaries");
        for boundary in [
            PublicationBoundary::CheckpointBeforeWrite,
            PublicationBoundary::CheckpointBeforeSync,
            PublicationBoundary::CheckpointAfterSync,
        ] {
            let path = directory.path.join(format!("{boundary:?}.pwf"));
            let expected_intent = intent();
            let all_checkpoints = checkpoints(&expected_intent);
            let mut journal =
                Journal::create(&path, expected_intent, JournalLimits::default()).unwrap();
            for checkpoint in &all_checkpoints[..6] {
                journal.record(checkpoint.clone()).unwrap();
            }
            let before_bytes = fs::metadata(&path).unwrap().len();
            let failure = journal
                .record_with_hook(
                    all_checkpoints[6].clone(),
                    &TestHook(TestAction::FailAt(boundary)),
                )
                .expect_err("injected Complete boundary cannot acknowledge the checkpoint");
            assert_eq!(
                journal.checkpoints().len(),
                7,
                "no false in-memory Complete"
            );
            match boundary {
                PublicationBoundary::CheckpointBeforeWrite => {
                    assert!(matches!(failure, JournalError::Io { .. }));
                    assert!(!journal.poisoned);
                    assert_eq!(fs::metadata(&path).unwrap().len(), before_bytes);
                }
                PublicationBoundary::CheckpointBeforeSync
                | PublicationBoundary::CheckpointAfterSync => {
                    assert!(matches!(
                        failure,
                        JournalError::CheckpointIndeterminate { sequence: 7, .. }
                    ));
                    assert!(journal.poisoned);
                }
                _ => unreachable!("test enumerates checkpoint boundaries"),
            }
            drop(journal);
            let reopened = Journal::open(&path, JournalLimits::default()).unwrap();
            let expected_frames = if boundary == PublicationBoundary::CheckpointBeforeWrite {
                7
            } else {
                8
            };
            assert_eq!(reopened.checkpoints().len(), expected_frames);
            assert_eq!(
                matches!(reopened.checkpoints().last(), Some(Checkpoint::Complete(_))),
                expected_frames == 8
            );
        }
    }

    #[test]
    fn checkpoint_append_rejects_a_replaced_recognized_path() {
        let directory = Directory::new("checkpoint-path-replacement");
        let path = directory.path.join("run.pwf");
        let moved = directory.path.join("moved.pwf");
        let expected_intent = intent();
        let checkpoint = checkpoints(&expected_intent)[0].clone();
        let mut journal =
            Journal::create(&path, expected_intent, JournalLimits::default()).unwrap();
        let recognized_bytes = fs::read(&path).unwrap();

        fs::rename(&path, &moved).unwrap();
        fs::copy(&moved, &path).unwrap();

        let failure = journal
            .record(checkpoint)
            .expect_err("a byte-identical replacement is not the recognized journal");
        assert!(matches!(failure, JournalError::Io { .. }));
        assert!(!journal.poisoned);
        assert_eq!(fs::read(&path).unwrap(), recognized_bytes);
        assert_eq!(fs::read(&moved).unwrap(), recognized_bytes);
    }

    #[test]
    fn post_write_path_replacement_is_indeterminate_and_preserved() {
        let directory = Directory::new("checkpoint-post-write-replacement");
        let path = directory.path.join("run.pwf");
        let expected_intent = intent();
        let checkpoint = checkpoints(&expected_intent)[0].clone();
        let replacement = b"caller replacement after checkpoint sync";
        let mut journal =
            Journal::create(&path, expected_intent, JournalLimits::default()).unwrap();

        let failure = journal
            .record_with_hook(
                checkpoint,
                &TestHook(TestAction::InstallAt {
                    boundary: PublicationBoundary::CheckpointAfterSync,
                    target: &path,
                    bytes: replacement,
                    replace: true,
                }),
            )
            .expect_err("a replaced target cannot acknowledge the checkpoint");

        assert!(matches!(
            failure,
            JournalError::CheckpointIndeterminate { sequence: 1, .. }
        ));
        assert!(journal.poisoned);
        assert_eq!(fs::read(path).unwrap(), replacement);
    }

    #[test]
    fn intent_create_race_and_post_link_replacement_preserve_caller_bytes() {
        let directory = Directory::new("intent-races");
        let raced = directory.path.join("raced.pwf");
        let caller_bytes = b"caller-owned journal path";
        let failure = Journal::create_with_hook(
            &raced,
            intent(),
            JournalLimits::default(),
            &TestHook(TestAction::InstallAt {
                boundary: PublicationBoundary::IntentBeforeLink,
                target: &raced,
                bytes: caller_bytes,
                replace: false,
            }),
        )
        .expect_err("a create-new race does not overwrite its winner");
        assert!(matches!(failure, JournalError::Exists(path) if path == raced));
        assert_eq!(fs::read(&raced).unwrap(), caller_bytes);

        let replaced = directory.path.join("replaced.pwf");
        let replacement = b"caller replacement after link";
        let failure = Journal::create_with_hook(
            &replaced,
            intent(),
            JournalLimits::default(),
            &TestHook(TestAction::InstallAt {
                boundary: PublicationBoundary::IntentTargetVerification,
                target: &replaced,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("a post-link replacement has no receipt");
        assert!(matches!(failure, JournalError::Indeterminate { .. }));
        assert_eq!(fs::read(&replaced).unwrap(), replacement);
        directory.assert_no_stages();
    }

    #[test]
    fn recognized_stage_cleanup_never_removes_a_replacement() {
        let directory = Directory::new("stage-replacement");
        let (stage, file) = create_stage(&directory.path).unwrap();
        let stage_path = stage.path().to_path_buf();
        drop(file);
        fs::remove_file(&stage_path).unwrap();
        write_synced(&stage_path, b"unowned replacement").unwrap();
        drop(stage);
        assert_eq!(fs::read(&stage_path).unwrap(), b"unowned replacement");
        fs::remove_file(stage_path).unwrap();
    }

    #[test]
    fn eight_frame_journal_round_trips_without_retry_growth() {
        let directory = Directory::new("round-trip");
        let path = directory.path.join("run.pwf");
        let intent = intent();
        let mut journal = Journal::create(&path, intent.clone(), JournalLimits::default()).unwrap();
        for checkpoint in checkpoints(&intent) {
            assert!(journal.record(checkpoint.clone()).unwrap());
            assert!(!journal.record(checkpoint).unwrap());
        }
        let bytes = fs::metadata(&path).unwrap().len();
        drop(journal);
        let reopened = Journal::open(&path, JournalLimits::default()).unwrap();
        assert_eq!(reopened.intent(), &intent);
        assert_eq!(reopened.checkpoints().len(), 8);
        assert_eq!(fs::metadata(path).unwrap().len(), bytes);
    }

    #[test]
    fn torn_tail_repairs_and_complete_corruption_fails_closed() {
        let directory = Directory::new("tail");
        let path = directory.path.join("run.pwf");
        let journal = Journal::create(&path, intent(), JournalLimits::default()).unwrap();
        let valid = fs::metadata(&path).unwrap().len();
        drop(journal);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"torn")
            .unwrap();
        drop(Journal::open(&path, JournalLimits::default()).unwrap());
        assert_eq!(fs::metadata(&path).unwrap().len(), valid);
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            Journal::open(&path, JournalLimits::default()),
            Err(JournalError::Corrupt(_))
        ));
    }

    #[test]
    fn variable_intent_is_canonical_and_bounded() {
        let limits = JournalLimits::new(1024 * 1024, 8, 512, 1024, 1, 0, 1024);
        let error = Journal::create(
            &Directory::new("limit").path.join("run.pwf"),
            intent(),
            limits,
        )
        .unwrap_err();
        assert!(matches!(error, JournalError::Resource { .. }));
    }

    fn intent() -> WorkflowIntent {
        WorkflowIntent::new(
            WorkflowRunId::new([1; 16]).unwrap(),
            [2; 32],
            [3; 16],
            [4; 32],
            [5; 16],
            vec![2, 7].into_boxed_slice(),
            2,
            1,
            None,
            vec![IntentCheckPoint {
                id: 1,
                position_bits: [1.0_f64.to_bits(), 2.0_f64.to_bits(), 3.0_f64.to_bits()],
            }]
            .into_boxed_slice(),
            "Ground".into(),
            "2026-08-10".into(),
            "00:00:00Z".into(),
            true,
            [[6; 32], [7; 32], [8; 32], [9; 32]],
            JournalLimits::default(),
        )
        .unwrap()
    }

    fn checkpoints(intent: &WorkflowIntent) -> [Checkpoint; 7] {
        let revision = [10; 32];
        let audit = [11; 32];
        let surface = [12; 32];
        let qa = [13; 32];
        let xml = [14; 32];
        let report = [15; 32];
        [
            Checkpoint::RevisionResolved(RevisionResolved {
                operation: intent.operation,
                revision,
                parent: intent.baseline_revision,
                sequence: 1,
                kind: 1,
            }),
            Checkpoint::AuditObserved(AuditObserved {
                revision,
                content_hash: audit,
                point_id_hash: [16; 32],
                changed_points: 2,
                transition_count: 1,
                footprint_bits: Some([[1, 2], [3, 4], [5, 6]]),
            }),
            Checkpoint::SurfaceObserved(SurfaceObserved {
                revision,
                recipe_hash: intent.recipe_hash,
                baseline_artifact_hash: [17; 32],
                changed_artifact_hash: surface,
                baseline_geometry_hash: [18; 32],
                changed_geometry_hash: [19; 32],
                baseline_topology_hash: [20; 32],
                changed_topology_hash: [21; 32],
                baseline_vertex_count: 5,
                baseline_face_count: 4,
                changed_vertex_count: 3,
                changed_face_count: 1,
                added_face_count: 1,
                removed_face_count: 4,
                added_face_hash: [22; 32],
                removed_face_hash: [23; 32],
                envelope_bits: Some([[1, 2], [3, 4], [5, 6]]),
            }),
            Checkpoint::QaObserved(QaObserved {
                surface_artifact_hash: surface,
                result_hash: qa,
                covered_count: 1,
                gap_count: 0,
                face_tests: 1,
                accounted_peak_working_bytes: 64,
                statistic_bits: [0; 4],
                statistic_mask: 15,
            }),
            Checkpoint::ExportEnsured(ExportEnsured {
                revision,
                surface_artifact_hash: surface,
                options_hash: intent.options_hash,
                target_binding: intent.path_bindings[3],
                content_hash: xml,
                byte_length: 100,
                outcome: 1,
            }),
            Checkpoint::ReportEnsured(ReportEnsured {
                report_hash: report,
                byte_length: 200,
                revision,
                audit_hash: audit,
                surface_hash: surface,
                qa_hash: qa,
                landxml_hash: xml,
            }),
            Checkpoint::Complete(Complete {
                request_hash: intent.request_hash,
                revision,
                audit_hash: audit,
                surface_hash: surface,
                qa_hash: qa,
                landxml_hash: xml,
                report_hash: report,
            }),
        ]
    }

    fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        sync_directory(path.parent().unwrap())
    }

    struct Directory {
        path: PathBuf,
    }

    impl Directory {
        fn new(label: &str) -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "punctra-terrain-journal-{label}-{}-{}",
                std::process::id(),
                hex(&random)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn assert_no_stages(&self) {
            let stages = fs::read_dir(&self.path)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".punctra-workflow-")
                })
                .count();
            assert_eq!(stages, 0, "recognized journal stages must be cleaned");
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
