// The private orchestrator keeps the complete transition order and normalized
// limit schema visible as linear audit trails. WorkflowFailure intentionally
// carries every known durable identity, and WorkflowLimits is a Copy policy
// snapshot passed intact to child phases.
#![allow(clippy::too_many_arguments, clippy::too_many_lines)]

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use blake3::Hasher;
use foundation_runtime::{Job, OperationControl};
use point_contracts::{ContentHash, PointId, WorldBounds};
use point_index::{IndexError, PrepareLimits};
use point_source::SourceError;
use point_terrain::{
    CheckPoint, CheckPointLimits, CheckPointOutcome, LandXmlLimits, LandXmlOptions, TerrainLimits,
    TerrainRecipe, TerrainSurface,
};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitPhase, CommitRejection, CommitRequest, OpenLimits,
    OperationId, OperationResolution, PointQuery, PointRowLimits, PointSetLimits, RevisionAudit,
    RevisionAuditLimits, RevisionId, RevisionInfo, RevisionKind, Workspace, WorkspaceError,
};

use crate::{
    canonical_output::{CanonicalOutputError, CanonicalOutputLimits},
    diagnostic::{
        Certainty, FailureCode, FailureContext, PublicationPhase, RecoveryAction, WorkflowFailure,
        WorkflowStage,
    },
    journal::{
        self, AuditObserved, Checkpoint, Complete, ExportEnsured, IntentCheckPoint, Journal,
        JournalError, JournalLimits, QaObserved, ReportEnsured, RevisionResolved, SurfaceObserved,
        WorkflowIntent as DurableIntent, WorkflowRunId,
    },
    publication::same_file_identity,
    report::{self, LimitFact, ReportFacts, SurfaceChangeEnvelope},
};

const PATH_BINDING_BYTES: u64 = 4 * 1024;
const ENVELOPE_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-face-change-v1";
const QA_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-qa-result-v1";
const SEMANTIC_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-semantic-results-v1";
const LAS_CLASSIFICATION_ATTRIBUTE: u32 = 6;
const MAX_INTENT_ORDINALS: usize = 1_000;
const MAX_INTENT_CHECK_POINTS: usize = 256;
const LIMIT_FACT_COUNT: usize = 115;

/// Caller-owned paths for one durable terrain Workflow Run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowPaths {
    source: PathBuf,
    index: PathBuf,
    workspace: PathBuf,
    run_root: PathBuf,
}

impl WorkflowPaths {
    /// Creates explicit Source, index, Workspace, and Run-root paths.
    #[must_use]
    pub fn new(
        source: impl Into<PathBuf>,
        index: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        run_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            source: source.into(),
            index: index.into(),
            workspace: workspace.into(),
            run_root: run_root.into(),
        }
    }

    fn journal(&self) -> PathBuf {
        self.run_root.join("run.pwf")
    }

    fn lock(&self) -> PathBuf {
        self.run_root.join("run.lock")
    }

    fn landxml(&self) -> PathBuf {
        self.run_root.join("terrain.xml")
    }

    fn report(&self) -> PathBuf {
        self.run_root.join("audit.json")
    }
}

/// Complete caller-selected immutable intent for one Workflow Run.
#[derive(Clone, Debug)]
pub struct WorkflowRunIntent {
    run: WorkflowRunId,
    operation: OperationId,
    baseline_revision: RevisionId,
    correction_ordinals: Box<[u64]>,
    non_ground_classification: u8,
    recipe: TerrainRecipe,
    check_points: Box<[CheckPoint]>,
    landxml: LandXmlOptions,
}

/// Last semantically validated durable phase of one Workflow Run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowPhase {
    /// The immutable Run intent and bindings are durable.
    IntentRecorded,
    /// The Workspace Operation has one resolved Revision.
    RevisionResolved,
    /// The exact Revision Audit has been observed.
    AuditObserved,
    /// Both Terrain Surfaces and their change envelope have been observed.
    SurfacesObserved,
    /// Detached Check Point QA has been observed.
    QaObserved,
    /// The exact `LandXML` output has been ensured.
    ExportEnsured,
    /// The canonical audit report has been ensured.
    ReportEnsured,
    /// Every final fact has been revalidated and the Run is complete.
    Complete,
}

impl WorkflowPhase {
    /// Returns the stable presentation name for this semantic phase.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IntentRecorded => "intent-recorded",
            Self::RevisionResolved => "revision-resolved",
            Self::AuditObserved => "audit-observed",
            Self::SurfacesObserved => "surfaces-observed",
            Self::QaObserved => "qa-observed",
            Self::ExportEnsured => "export-ensured",
            Self::ReportEnsured => "report-ensured",
            Self::Complete => "complete",
        }
    }
}

impl WorkflowRunIntent {
    /// Creates one bounded explicit Ground-exclusion intent.
    ///
    /// # Errors
    ///
    /// Returns a structured invalid-request or resource-limit failure when an
    /// identity, classification, ordinal set, or Check Point set is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run: WorkflowRunId,
        operation: OperationId,
        baseline_revision: RevisionId,
        correction_ordinals: impl IntoIterator<Item = u64>,
        non_ground_classification: u8,
        recipe: TerrainRecipe,
        check_points: impl IntoIterator<Item = CheckPoint>,
        landxml: LandXmlOptions,
    ) -> Result<Self, WorkflowFailure> {
        if !landxml.coordinates_are_metric_metres_asserted() {
            return Err(WorkflowFailure::invalid(
                WorkflowStage::Validate,
                "LandXML requires an explicit metric-metre coordinate assertion",
            ));
        }
        let mut ordinals = collect_bounded(
            correction_ordinals,
            MAX_INTENT_ORDINALS,
            "correction ordinal count",
        )?;
        ordinals.sort_unstable();
        if ordinals.is_empty() || ordinals.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(WorkflowFailure::invalid(
                WorkflowStage::Validate,
                "correction ordinals must be a nonempty unique set",
            ));
        }
        if recipe.ground_classification() == non_ground_classification {
            return Err(WorkflowFailure::invalid(
                WorkflowStage::Validate,
                "Ground and replacement classifications must differ",
            ));
        }
        let mut check_points = collect_bounded(
            check_points,
            MAX_INTENT_CHECK_POINTS,
            "detached Check Point count",
        )?;
        check_points.sort_unstable_by_key(|point| point.id());
        if check_points
            .windows(2)
            .any(|pair| pair[0].id() == pair[1].id())
        {
            return Err(WorkflowFailure::invalid(
                WorkflowStage::Validate,
                "detached Check Point identities must be unique",
            ));
        }
        Ok(Self {
            run,
            operation,
            baseline_revision,
            correction_ordinals: ordinals.into_boxed_slice(),
            non_ground_classification,
            recipe,
            check_points: check_points.into_boxed_slice(),
            landxml,
        })
    }

    /// Returns the caller-owned Run identity.
    #[must_use]
    pub const fn run(&self) -> WorkflowRunId {
        self.run
    }

    /// Returns the caller-owned durable Workspace Operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the expected baseline Revision identity.
    #[must_use]
    pub const fn baseline_revision(&self) -> RevisionId {
        self.baseline_revision
    }
}

/// Complete semantic ceilings for one Workflow Run.
#[derive(Clone, Copy, Debug)]
pub struct WorkflowLimits {
    prepare: PrepareLimits,
    open: OpenLimits,
    rows: PointRowLimits,
    selection: PointSetLimits,
    commit: CommitLimits,
    audit: RevisionAuditLimits,
    terrain: TerrainLimits,
    qa: CheckPointLimits,
    landxml: LandXmlLimits,
    journal: JournalLimits,
    report: CanonicalOutputLimits,
    max_envelope_faces: u64,
    max_envelope_working_bytes: u64,
    max_aggregate_working_bytes: u64,
}

impl Default for WorkflowLimits {
    fn default() -> Self {
        Self {
            prepare: PrepareLimits::default(),
            open: OpenLimits::default(),
            rows: PointRowLimits::default(),
            selection: PointSetLimits::default(),
            commit: CommitLimits::default(),
            audit: RevisionAuditLimits::default(),
            terrain: TerrainLimits::default(),
            qa: CheckPointLimits::default(),
            landxml: LandXmlLimits::default(),
            journal: JournalLimits::default(),
            report: CanonicalOutputLimits::default(),
            max_envelope_faces: 20_000_000,
            max_envelope_working_bytes: 1024 * 1024 * 1024,
            max_aggregate_working_bytes: 8 * 1024 * 1024 * 1024,
        }
    }
}

impl WorkflowLimits {
    /// Replaces Spatial Index preparation ceilings.
    #[must_use]
    pub const fn with_prepare_limits(mut self, value: PrepareLimits) -> Self {
        self.prepare = value;
        self
    }

    /// Replaces existing-Workspace opening ceilings.
    #[must_use]
    pub const fn with_open_limits(mut self, value: OpenLimits) -> Self {
        self.open = value;
        self
    }

    /// Replaces baseline effective-row validation ceilings.
    #[must_use]
    pub const fn with_point_row_limits(mut self, value: PointRowLimits) -> Self {
        self.rows = value;
        self
    }

    /// Replaces exact Point Set materialization ceilings.
    #[must_use]
    pub const fn with_selection_limits(mut self, value: PointSetLimits) -> Self {
        self.selection = value;
        self
    }

    /// Replaces Workspace commit ceilings.
    #[must_use]
    pub const fn with_commit_limits(mut self, value: CommitLimits) -> Self {
        self.commit = value;
        self
    }

    /// Replaces immutable Revision Audit ceilings.
    #[must_use]
    pub const fn with_audit_limits(mut self, value: RevisionAuditLimits) -> Self {
        self.audit = value;
        self
    }

    /// Replaces Terrain Derivation ceilings.
    #[must_use]
    pub const fn with_terrain_limits(mut self, value: TerrainLimits) -> Self {
        self.terrain = value;
        self
    }

    /// Replaces detached Check Point QA ceilings.
    #[must_use]
    pub const fn with_check_point_limits(mut self, value: CheckPointLimits) -> Self {
        self.qa = value;
        self
    }

    /// Replaces `LandXML` publication ceilings.
    #[must_use]
    pub const fn with_landxml_limits(mut self, value: LandXmlLimits) -> Self {
        self.landxml = value;
        self
    }

    /// Replaces Surface Change Envelope face and incremental-byte ceilings.
    #[must_use]
    pub const fn with_envelope_limits(mut self, faces: u64, working_bytes: u64) -> Self {
        self.max_envelope_faces = faces;
        self.max_envelope_working_bytes = working_bytes;
        self
    }

    /// Replaces durable Intent ordinal and detached Check Point count ceilings.
    #[must_use]
    pub const fn with_intent_count_limits(mut self, ordinals: u64, check_points: u64) -> Self {
        self.journal.max_correction_ordinals = ordinals;
        self.journal.max_check_points = check_points;
        self
    }

    /// Replaces the journal file-byte ceiling for evidence and constrained runs.
    #[must_use]
    pub const fn with_max_journal_bytes(mut self, value: u64) -> Self {
        self.journal.max_journal_bytes = value;
        self
    }

    /// Replaces both report output and staging byte ceilings.
    #[must_use]
    pub const fn with_max_report_bytes(mut self, value: u64) -> Self {
        self.report.max_output_bytes = value;
        self.report.max_staging_bytes = value;
        self
    }

    /// Replaces the combined live orchestrator artifact and working-byte ceiling.
    #[must_use]
    pub const fn with_max_aggregate_working_bytes(mut self, value: u64) -> Self {
        self.max_aggregate_working_bytes = value;
        self
    }
}

/// Successful stable facts for one complete Workflow Run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkflowReceipt {
    run: WorkflowRunId,
    operation: OperationId,
    revision: RevisionId,
    report_hash: ContentHash,
    report_bytes: u64,
}

impl WorkflowReceipt {
    /// Returns the Run identity.
    #[must_use]
    pub const fn run(self) -> WorkflowRunId {
        self.run
    }
    /// Returns the Workspace Operation identity.
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }
    /// Returns the changed Revision identity.
    #[must_use]
    pub const fn revision(self) -> RevisionId {
        self.revision
    }
    /// Returns the canonical report byte hash.
    #[must_use]
    pub const fn report_hash(self) -> ContentHash {
        self.report_hash
    }
    /// Returns the canonical report byte count.
    #[must_use]
    pub const fn report_bytes(self) -> u64 {
        self.report_bytes
    }
}

/// Verified journal status for one Run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkflowStatus {
    run: WorkflowRunId,
    operation: OperationId,
    phase: WorkflowPhase,
}

impl WorkflowStatus {
    /// Returns the Run identity.
    #[must_use]
    pub const fn run(self) -> WorkflowRunId {
        self.run
    }
    /// Returns the durable Operation identity.
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }
    /// Returns the last semantically validated durable phase.
    #[must_use]
    pub const fn phase(self) -> WorkflowPhase {
        self.phase
    }
    /// Reports whether the Complete checkpoint is durable.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self.phase, WorkflowPhase::Complete)
    }
}

/// Background Workflow execution with linked child cancellation.
pub type WorkflowJob = Job<WorkflowReceipt, WorkflowFailure>;

/// Starts a new Run and publishes Intent before Point selection or Workspace commit.
#[must_use]
pub fn start_run(
    paths: WorkflowPaths,
    intent: WorkflowRunIntent,
    limits: WorkflowLimits,
) -> WorkflowJob {
    Job::spawn(move |control| run(&paths, &intent, &limits, true, &control))
}

/// Resumes the same durable Run with identical paths and caller intent.
#[must_use]
pub fn resume_run(
    paths: WorkflowPaths,
    intent: WorkflowRunIntent,
    limits: WorkflowLimits,
) -> WorkflowJob {
    Job::spawn(move |control| run(&paths, &intent, &limits, false, &control))
}

/// Inspects one journal and durably repairs a torn final suffix when needed.
///
/// A repair truncates the journal to its last verified checkpoint and syncs that
/// truncation before returning the semantic durable status.
///
/// # Errors
///
/// Returns a structured failure when the Run lock, journal bytes, hash chain,
/// semantic checkpoint links, or resource limits cannot be verified.
pub fn inspect_and_repair_run(
    run_root: impl AsRef<Path>,
    limits: WorkflowLimits,
) -> Result<WorkflowStatus, WorkflowFailure> {
    let run_root = run_root.as_ref();
    require_workflow_bytes(
        limits.journal.max_working_bytes,
        limits.max_aggregate_working_bytes,
        WorkflowStage::Inspect,
        FailureContext::default(),
    )?;
    let witness = DirectoryWitness::capture(run_root)
        .map_err(|error| io_failure(WorkflowStage::Inspect, error, FailureContext::default()))?;
    let lock = RunLock::acquire(&run_root.join("run.lock"))
        .map_err(|error| lock_failure(error, FailureContext::default()))?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Inspect, error, FailureContext::default()))?;
    let journal = Journal::open(&run_root.join("run.pwf"), limits.journal).map_err(|error| {
        journal_failure(WorkflowStage::Inspect, error, FailureContext::default())
    })?;
    let context = durable_context(journal.run(), journal.intent());
    verify_run_binding(&lock, &witness).map_err(|error| {
        WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            WorkflowStage::Inspect,
            Certainty::Indeterminate(PublicationPhase::JournalCheckpoint),
            context,
            error,
            RecoveryAction::ResumeSameRun,
        )
    })?;
    let intent = journal.intent();
    let operation = OperationId::from_bytes(intent.operation).map_err(|_| {
        journal_failure(
            WorkflowStage::Inspect,
            JournalError::Invalid("Workspace Operation Identity is all zero"),
            context,
        )
    })?;
    let phase = journal
        .checkpoints()
        .last()
        .map(checkpoint_phase)
        .ok_or_else(|| {
            journal_failure(
                WorkflowStage::Inspect,
                JournalError::Corrupt("journal has no validated durable phase"),
                context,
            )
        })?;
    let status = WorkflowStatus {
        run: journal.run(),
        operation,
        phase,
    };
    verify_run_binding(&lock, &witness).map_err(|error| {
        WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            WorkflowStage::Inspect,
            Certainty::Indeterminate(PublicationPhase::JournalCheckpoint),
            context,
            error,
            RecoveryAction::ResumeSameRun,
        )
    })?;
    Ok(status)
}

fn run(
    paths: &WorkflowPaths,
    request: &WorkflowRunIntent,
    limits: &WorkflowLimits,
    start: bool,
    control: &OperationControl,
) -> Result<WorkflowReceipt, WorkflowFailure> {
    validate_run_root(&paths.run_root, base_context(request))?;
    let witness = DirectoryWitness::capture(&paths.run_root)
        .map_err(|error| io_failure(WorkflowStage::Validate, error, base_context(request)))?;
    let lock = RunLock::acquire(&paths.lock())
        .map_err(|error| lock_failure(error, base_context(request)))?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Lock, error, base_context(request)))?;

    require_workflow_bytes(
        request_retained_bytes(request),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Source,
        base_context(request),
    )?;
    let path_bindings = path_bindings(paths)
        .map_err(|error| journal_failure(WorkflowStage::Validate, error, base_context(request)))?;
    let resumed_journal = if start {
        None
    } else {
        require_workflow_bytes(
            request_retained_bytes(request).saturating_add(limits.journal.max_working_bytes),
            limits.max_aggregate_working_bytes,
            WorkflowStage::Intent,
            base_context(request),
        )?;
        let journal = Journal::open(&paths.journal(), limits.journal).map_err(|error| {
            journal_failure(WorkflowStage::Intent, error, base_context(request))
        })?;
        verify_run_binding(&lock, &witness).map_err(|error| {
            WorkflowFailure::new(
                FailureCode::PublicationIndeterminate,
                WorkflowStage::Intent,
                Certainty::Indeterminate(PublicationPhase::JournalCheckpoint),
                base_context(request),
                error,
                RecoveryAction::ResumeSameRun,
            )
        })?;
        validate_supplied_intent(journal.intent(), request, path_bindings)?;
        Some(journal)
    };
    let entry_retained_bytes = run_entry_retained_bytes(request, resumed_journal.as_ref());
    require_workflow_bytes(
        entry_retained_bytes,
        limits.max_aggregate_working_bytes,
        WorkflowStage::Source,
        base_context(request),
    )?;

    let source = source_las::open(&paths.source)
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| {
            source_failure(WorkflowStage::Source, error, control, base_context(request))
        })?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Source, error, base_context(request)))?;
    let source_id = source.identity();
    if let Some(journal) = resumed_journal.as_ref()
        && journal.intent().source != source_id.into_bytes()
    {
        let mut mismatch_context = base_context(request);
        mismatch_context.source = Some(source_id);
        return Err(WorkflowFailure::new(
            FailureCode::SourceMismatch,
            WorkflowStage::Source,
            Certainty::DurableFact,
            mismatch_context,
            "verified Source identity differs from durable Intent",
            RecoveryAction::RestoreExpectedSource,
        ));
    }
    require_workflow_bytes(
        entry_retained_bytes
            .saturating_add(limits.prepare.max_source_batch_payload_bytes())
            .saturating_add(limits.prepare.max_adapter_working_bytes())
            .saturating_add(limits.prepare.max_build_working_bytes())
            .saturating_add(limits.prepare.max_resident_metadata_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Index,
        base_context(request),
    )?;
    let index = point_index::prepare(source, &paths.index, limits.prepare)
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| {
            index_failure(WorkflowStage::Index, error, control, base_context(request))
        })?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Index, error, base_context(request)))?;
    let mut context = base_context(request);
    context.source = Some(source_id);
    require_workflow_bytes(
        entry_retained_bytes
            .saturating_add(limits.prepare.max_resident_metadata_bytes())
            .saturating_add(limits.open.max_working_bytes())
            .saturating_add(limits.open.max_resident_metadata_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Workspace,
        context,
    )?;
    let workspace = open_workspace(index, &paths.workspace, limits.open, control, context)?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Workspace, error, context))?;
    context.workspace = Some(workspace.identity());
    if let Some(journal) = resumed_journal.as_ref()
        && journal.intent().workspace != workspace.identity().into_bytes()
    {
        return Err(WorkflowFailure::new(
            FailureCode::WorkspaceMismatch,
            WorkflowStage::Workspace,
            Certainty::DurableFact,
            context,
            "Workspace lineage differs from durable Intent",
            RecoveryAction::StopAndPreserve,
        ));
    }
    if workspace.source() != source_id {
        return Err(WorkflowFailure::new(
            FailureCode::SourceMismatch,
            WorkflowStage::Workspace,
            Certainty::PrePublication,
            context,
            "Workspace Source differs from verified Source",
            RecoveryAction::RestoreExpectedSource,
        ));
    }
    if workspace.schema().classification().get() != LAS_CLASSIFICATION_ATTRIBUTE {
        return Err(WorkflowFailure::new(
            FailureCode::WorkspaceMismatch,
            WorkflowStage::Workspace,
            Certainty::DurableFact,
            context,
            format_args!(
                "Workspace classification Attribute is {}, expected LAS classification Attribute {LAS_CLASSIFICATION_ATTRIBUTE}",
                workspace.schema().classification().get()
            ),
            RecoveryAction::CorrectInvalidRequest,
        ));
    }
    let baseline_id = request.baseline_revision;
    if start && workspace.head().provenance().revision() != baseline_id {
        return Err(WorkflowFailure::new(
            FailureCode::StaleBaseline,
            WorkflowStage::Workspace,
            Certainty::PrePublication,
            context,
            "Workspace head differs from the expected baseline Revision",
            RecoveryAction::CorrectInvalidRequest,
        ));
    }
    require_workflow_bytes(
        entry_retained_bytes
            .saturating_add(limits.prepare.max_resident_metadata_bytes())
            .saturating_add(limits.open.max_resident_metadata_bytes())
            .saturating_add(limits.rows.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Selection,
        context,
    )?;
    validate_ground_ordinals(
        &workspace,
        baseline_id,
        request,
        limits.rows,
        control,
        context,
    )?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Selection, error, context))?;
    require_workflow_bytes(
        entry_retained_bytes
            .saturating_add(limits.prepare.max_resident_metadata_bytes())
            .saturating_add(limits.open.max_resident_metadata_bytes())
            .saturating_add(limits.journal.max_working_bytes),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Intent,
        context,
    )?;
    let durable = durable_intent(
        request,
        source_id.into_bytes(),
        workspace.identity().into_bytes(),
        path_bindings,
        limits.journal,
    )
    .map_err(|error| journal_failure(WorkflowStage::Validate, error, context))?;
    verify_run_binding(&lock, &witness)
        .map_err(|error| io_failure(WorkflowStage::Intent, error, context))?;
    let mut journal = if start {
        Journal::create(&paths.journal(), durable, limits.journal)
            .map_err(|error| journal_failure(WorkflowStage::Intent, error, context))?
    } else {
        let journal = resumed_journal.expect("resume journal was opened before external paths");
        if journal.intent() != &durable {
            return Err(WorkflowFailure::new(
                FailureCode::JournalConflict,
                WorkflowStage::Intent,
                Certainty::DurableFact,
                context,
                "supplied paths or intent differ from durable Intent",
                RecoveryAction::ResumeSameRun,
            ));
        }
        journal
    };
    verify_run_binding(&lock, &witness).map_err(|error| {
        if start {
            WorkflowFailure::new(
                FailureCode::PublicationIndeterminate,
                WorkflowStage::Intent,
                Certainty::Indeterminate(PublicationPhase::JournalIntent),
                context,
                error,
                RecoveryAction::ResumeSameRun,
            )
        } else {
            io_failure(WorkflowStage::Intent, error, context)
        }
    })?;
    advance(
        paths,
        request,
        limits,
        &workspace,
        &mut journal,
        &witness,
        &lock,
        control,
        context,
    )
}

fn advance(
    paths: &WorkflowPaths,
    request: &WorkflowRunIntent,
    limits: &WorkflowLimits,
    workspace: &Workspace,
    journal: &mut Journal,
    witness: &DirectoryWitness,
    lock: &RunLock,
    control: &OperationControl,
    mut context: FailureContext,
) -> Result<WorkflowReceipt, WorkflowFailure> {
    let baseline_id = request.baseline_revision;
    let operation = request.operation;
    let resolution = workspace.resolve_operation(operation).map_err(|error| {
        workspace_failure(WorkflowStage::ResolveOperation, error, control, context)
    })?;
    let resolution = match resolution {
        OperationResolution::Indeterminate(uncertainty) => {
            return Err(indeterminate_commit(
                uncertainty.phase(),
                uncertainty.reason(),
                context,
            ));
        }
        resolution => resolution,
    };
    require_workflow_bytes(
        workflow_retained_bytes(journal, request, limits)
            .saturating_add(limits.selection.max_working_bytes())
            .saturating_add(limits.selection.max_resident_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Selection,
        context,
    )?;
    let expected_points = workspace
        .snapshot(baseline_id)
        .map_err(|error| workspace_failure(WorkflowStage::Selection, error, control, context))?
        .select_point_ids(
            request
                .correction_ordinals
                .iter()
                .copied()
                .map(|ordinal| PointId::new(workspace.source(), ordinal)),
            limits.selection,
        )
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| workspace_failure(WorkflowStage::Selection, error, control, context))?;
    if expected_points.metadata().exact_count() != usize_u64(request.correction_ordinals.len()) {
        return Err(WorkflowFailure::invalid_with_context(
            WorkflowStage::Selection,
            context,
            "one or more explicit ordinals do not exist",
        ));
    }
    let expected_metadata = *expected_points.metadata();
    verify_run_binding(lock, witness)
        .map_err(|error| io_failure(WorkflowStage::Selection, error, context))?;
    require_workflow_bytes(
        workflow_retained_bytes(journal, request, limits)
            .saturating_add(limits.selection.max_resident_bytes())
            .saturating_add(limits.commit.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Commit,
        context,
    )?;
    let revision = resolve_revision(
        workspace,
        resolution,
        baseline_id,
        operation,
        request,
        expected_points,
        limits,
        control,
        context,
    )?;
    context.revision = Some(revision.id());
    let revision_fact = RevisionResolved {
        operation: request.operation.into_bytes(),
        revision: revision.id().into_bytes(),
        parent: revision.parent().unwrap_or(baseline_id).into_bytes(),
        sequence: revision.sequence(),
        kind: 1,
    };
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::RevisionResolved(revision_fact),
        WorkflowStage::ResolveOperation,
        context,
    )?;

    require_workflow_bytes(
        workflow_retained_bytes(journal, request, limits)
            .saturating_add(limits.audit.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::RevisionAudit,
        context,
    )?;
    let audit = workspace
        .revision_audit(revision.id(), limits.audit)
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| {
            workspace_failure(WorkflowStage::RevisionAudit, error, control, context)
        })?;
    validate_audit(
        &audit,
        request,
        revision,
        expected_metadata.point_id_hash(),
        context,
    )?;
    let audit_fact = audit_checkpoint(&audit);
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::AuditObserved(audit_fact),
        WorkflowStage::RevisionAudit,
        context,
    )?;

    require_workflow_bytes(
        workflow_retained_bytes(journal, request, limits)
            .saturating_add(audit.retained_result_bytes())
            .saturating_add(limits.terrain.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Terrain,
        context,
    )?;

    let baseline = point_terrain::derive(
        workspace
            .snapshot(baseline_id)
            .map_err(|error| workspace_failure(WorkflowStage::Terrain, error, control, context))?,
        request.recipe,
        limits.terrain,
    )
    .blocking_wait_cancelled_by(&control.token())
    .map_err(|error| terrain_failure(WorkflowStage::Terrain, error, control, context))?;
    verify_run_binding(lock, witness)
        .map_err(|error| io_failure(WorkflowStage::Terrain, error, context))?;
    require_workflow_bytes(
        workflow_retained_bytes(journal, request, limits)
            .saturating_add(audit.retained_result_bytes())
            .saturating_add(baseline.descriptor().retained_surface_bytes())
            .saturating_add(limits.terrain.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Terrain,
        context,
    )?;
    let changed = point_terrain::derive(
        workspace
            .snapshot(revision.id())
            .map_err(|error| workspace_failure(WorkflowStage::Terrain, error, control, context))?,
        request.recipe,
        limits.terrain,
    )
    .blocking_wait_cancelled_by(&control.token())
    .map_err(|error| terrain_failure(WorkflowStage::Terrain, error, control, context))?;
    verify_run_binding(lock, witness)
        .map_err(|error| io_failure(WorkflowStage::Terrain, error, context))?;
    require_workflow_bytes(
        workflow_retained_bytes(journal, request, limits)
            .saturating_add(audit.retained_result_bytes())
            .saturating_add(baseline.descriptor().retained_surface_bytes())
            .saturating_add(changed.descriptor().retained_surface_bytes())
            .saturating_add(limits.max_envelope_working_bytes),
        limits.max_aggregate_working_bytes,
        WorkflowStage::ChangeEnvelope,
        context,
    )?;
    let envelope = change_envelope(
        &baseline,
        &changed,
        limits.max_envelope_faces,
        limits.max_envelope_working_bytes,
        control,
    )
    .map_err(|error| envelope_failure(error, control, context))?;
    let surface_fact = surface_checkpoint(
        &baseline,
        &changed,
        envelope,
        revision.id().into_bytes(),
        journal.intent().recipe_hash,
    );
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::SurfaceObserved(surface_fact),
        WorkflowStage::Terrain,
        context,
    )?;

    let qa_input = copy_check_points_for_job(
        &request.check_points,
        retained_observations_bytes(journal, request, limits, &audit, &baseline, &changed),
        limits,
        context,
    )?;

    let qa = changed
        .check_points(qa_input, limits.qa)
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| terrain_failure(WorkflowStage::CheckPointQa, error, control, context))?;
    let qa_hash = qa_hash(&qa, control)
        .map_err(|error| child_failure(WorkflowStage::CheckPointQa, error, control, context))?;
    let qa_fact = qa_checkpoint(
        &qa,
        changed.descriptor().artifact_hash().into_bytes(),
        qa_hash,
    );
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::QaObserved(qa_fact),
        WorkflowStage::CheckPointQa,
        context,
    )?;

    require_workflow_bytes(
        retained_observations_bytes(journal, request, limits, &audit, &baseline, &changed)
            .saturating_add(qa_retained_bytes(&qa))
            .saturating_add(limits.landxml.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::LandXml,
        context,
    )?;
    verify_run_binding(lock, witness)
        .map_err(|error| io_failure(WorkflowStage::LandXml, error, context))?;
    let landxml = changed
        .ensure_landxml(paths.landxml(), request.landxml.clone(), limits.landxml)
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| terrain_output_failure(WorkflowStage::LandXml, error, control, context))?;
    let export_fact = ExportEnsured {
        revision: revision.id().into_bytes(),
        surface_artifact_hash: changed.descriptor().artifact_hash().into_bytes(),
        options_hash: journal.intent().options_hash,
        target_binding: journal.intent().path_bindings[3],
        content_hash: landxml.content_hash().into_bytes(),
        byte_length: landxml.byte_length(),
        outcome: 1,
    };
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::ExportEnsured(export_fact),
        WorkflowStage::LandXml,
        context,
    )?;

    let semantic_hash = semantic_results_hash(
        &audit, &baseline, &changed, envelope, &qa, &landxml, control,
    )
    .map_err(|error| child_failure(WorkflowStage::Report, error, control, context))?;
    require_workflow_bytes(
        retained_observations_bytes(journal, request, limits, &audit, &baseline, &changed)
            .saturating_add(qa_retained_bytes(&qa))
            .saturating_add(
                usize_u64(LIMIT_FACT_COUNT)
                    .saturating_mul(usize_u64(std::mem::size_of::<LimitFact>())),
            ),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Report,
        context,
    )?;
    let limit_facts = limit_facts(limits, control)
        .map_err(|error| child_failure(WorkflowStage::Report, error, control, context))?;
    require_workflow_bytes(
        retained_observations_bytes(journal, request, limits, &audit, &baseline, &changed)
            .saturating_add(qa_retained_bytes(&qa))
            .saturating_add(
                usize_u64(limit_facts.capacity())
                    .saturating_mul(usize_u64(std::mem::size_of::<LimitFact>())),
            )
            .saturating_add(limits.report.max_working_bytes),
        limits.max_aggregate_working_bytes,
        WorkflowStage::Report,
        context,
    )?;
    let report_facts = ReportFacts {
        run: journal.run(),
        request_hash: journal.intent().request_hash,
        source: journal.intent().source,
        workspace: journal.intent().workspace,
        operation: journal.intent().operation,
        baseline_revision: journal.intent().baseline_revision,
        changed_revision: revision.id().into_bytes(),
        correction_ordinals: &request.correction_ordinals,
        non_ground_classification: journal.intent().non_ground_classification,
        ordinal_hash: journal.intent().ordinal_hash,
        recipe_hash: journal.intent().recipe_hash,
        qa_input_hash: journal.intent().qa_input_hash,
        options_hash: journal.intent().options_hash,
        semantic_results_hash: semantic_hash,
        path_bindings: journal.intent().path_bindings,
        audit: &audit,
        baseline: &baseline,
        changed: &changed,
        envelope,
        qa: &qa,
        qa_hash,
        landxml,
        limits: &limit_facts,
    };
    verify_run_binding(lock, witness)
        .map_err(|error| io_failure(WorkflowStage::Report, error, context))?;
    let report = report::ensure_report(&paths.report(), &report_facts, limits.report, control)
        .map_err(|error| report_failure(WorkflowStage::Report, error, context))?;
    let report_fact = ReportEnsured {
        report_hash: report.content_hash,
        byte_length: report.byte_length,
        revision: revision.id().into_bytes(),
        audit_hash: audit.content_hash().into_bytes(),
        surface_hash: changed.descriptor().artifact_hash().into_bytes(),
        qa_hash,
        landxml_hash: landxml.content_hash().into_bytes(),
    };
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::ReportEnsured(report_fact),
        WorkflowStage::Report,
        context,
    )?;
    control
        .check_cancelled()
        .map_err(|_| cancelled_failure(WorkflowStage::Complete, context))?;
    verify_run_binding(lock, witness)
        .map_err(|error| io_failure(WorkflowStage::Complete, error, context))?;
    let final_landxml = changed
        .ensure_landxml(paths.landxml(), request.landxml.clone(), limits.landxml)
        .blocking_wait_cancelled_by(&control.token())
        .map_err(|error| {
            terrain_output_failure(WorkflowStage::Complete, error, control, context)
        })?;
    if final_landxml.content_hash() != landxml.content_hash()
        || final_landxml.byte_length() != landxml.byte_length()
        || final_landxml.surface_artifact_hash() != landxml.surface_artifact_hash()
    {
        return Err(operation_conflict(
            context,
            "final LandXML revalidation differs from ExportEnsured",
        ));
    }
    let final_report =
        report::ensure_report(&paths.report(), &report_facts, limits.report, control)
            .map_err(|error| report_failure(WorkflowStage::Complete, error, context))?;
    if final_report.content_hash != report.content_hash
        || final_report.byte_length != report.byte_length
    {
        return Err(operation_conflict(
            context,
            "final canonical report revalidation differs from ReportEnsured",
        ));
    }
    match workspace
        .resolve_operation(operation)
        .map_err(|error| workspace_failure(WorkflowStage::Complete, error, control, context))?
    {
        OperationResolution::Committed(receipt) if receipt.revision_info() == revision => {}
        _ => {
            return Err(operation_conflict(
                context,
                "final Workspace Operation revalidation differs",
            ));
        }
    }
    let complete = Complete {
        request_hash: journal.intent().request_hash,
        revision: revision.id().into_bytes(),
        audit_hash: audit.content_hash().into_bytes(),
        surface_hash: changed.descriptor().artifact_hash().into_bytes(),
        qa_hash,
        landxml_hash: landxml.content_hash().into_bytes(),
        report_hash: report.content_hash,
    };
    record(
        journal,
        witness,
        lock,
        control,
        Checkpoint::Complete(complete),
        WorkflowStage::Complete,
        context,
    )?;
    control.complete_progress(8).map_err(|error| {
        WorkflowFailure::new(
            FailureCode::Internal,
            WorkflowStage::Complete,
            Certainty::DurableFact,
            context,
            error,
            RecoveryAction::ResumeSameRun,
        )
    })?;
    verify_run_binding(lock, witness)
        .map_err(|error| checkpoint_binding_failure(WorkflowStage::Complete, error, context))?;
    Ok(WorkflowReceipt {
        run: request.run,
        operation: request.operation,
        revision: revision.id(),
        report_hash: ContentHash::new(report.content_hash),
        report_bytes: report.byte_length,
    })
}

fn checkpoint_phase(checkpoint: &Checkpoint) -> WorkflowPhase {
    match checkpoint {
        Checkpoint::Intent(_) => WorkflowPhase::IntentRecorded,
        Checkpoint::RevisionResolved(_) => WorkflowPhase::RevisionResolved,
        Checkpoint::AuditObserved(_) => WorkflowPhase::AuditObserved,
        Checkpoint::SurfaceObserved(_) => WorkflowPhase::SurfacesObserved,
        Checkpoint::QaObserved(_) => WorkflowPhase::QaObserved,
        Checkpoint::ExportEnsured(_) => WorkflowPhase::ExportEnsured,
        Checkpoint::ReportEnsured(_) => WorkflowPhase::ReportEnsured,
        Checkpoint::Complete(_) => WorkflowPhase::Complete,
    }
}

fn open_workspace(
    index: point_index::PreparedIndex,
    path: &Path,
    limits: OpenLimits,
    control: &OperationControl,
    context: FailureContext,
) -> Result<Workspace, WorkflowFailure> {
    let job = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => point_workspace::open(path, index, limits),
        Ok(_) => {
            return Err(WorkflowFailure::new(
                FailureCode::InvalidRequest,
                WorkflowStage::Workspace,
                Certainty::PrePublication,
                context,
                "Workspace path is not a directory",
                RecoveryAction::CorrectInvalidRequest,
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(WorkflowFailure::new(
                FailureCode::InvalidRequest,
                WorkflowStage::Workspace,
                Certainty::PrePublication,
                context,
                "Workspace must already exist; use its current head as --baseline",
                RecoveryAction::CorrectInvalidRequest,
            ));
        }
        Err(error) => {
            return Err(io_failure(WorkflowStage::Workspace, error, context));
        }
    };
    job.blocking_wait_cancelled_by(&control.token())
        .map_err(|error| workspace_failure(WorkflowStage::Workspace, error, control, context))
}

fn resolve_revision(
    workspace: &Workspace,
    resolution: OperationResolution,
    baseline: RevisionId,
    operation: OperationId,
    request: &WorkflowRunIntent,
    expected_points: point_workspace::PointSet,
    limits: &WorkflowLimits,
    control: &OperationControl,
    context: FailureContext,
) -> Result<RevisionInfo, WorkflowFailure> {
    let outcome = match resolution {
        OperationResolution::Committed(_) => workspace
            .commit(
                CommitRequest::set_classification(
                    operation,
                    expected_points,
                    request.non_ground_classification,
                ),
                limits.commit,
            )
            .blocking_wait_cancelled_by(&control.token())
            .map_err(|error| workspace_failure(WorkflowStage::Commit, error, control, context))?,
        OperationResolution::Retryable(intent) => {
            if intent.parent().revision() != baseline
                || intent.parent().workspace() != workspace.identity()
                || intent.parent().source() != workspace.source()
                || intent.kind()
                    != (RevisionKind::SetClassification {
                        value: request.non_ground_classification,
                        changed_points: usize_u64(request.correction_ordinals.len()),
                    })
                || intent.point_set() != Some(*expected_points.metadata())
            {
                return Err(operation_conflict(
                    context,
                    "recorded Workspace intent differs from durable Run Intent",
                ));
            }
            workspace
                .retry_operation(operation, limits.commit)
                .blocking_wait_cancelled_by(&control.token())
                .map_err(|error| {
                    workspace_failure(WorkflowStage::Commit, error, control, context)
                })?
        }
        OperationResolution::NotRecorded => {
            if workspace.head().provenance().revision() != baseline {
                return Err(WorkflowFailure::new(
                    FailureCode::StaleBaseline,
                    WorkflowStage::Commit,
                    Certainty::DurableFact,
                    context,
                    "Workspace head advanced before Operation publication",
                    RecoveryAction::StopAndPreserve,
                ));
            }
            workspace
                .commit(
                    CommitRequest::set_classification(
                        operation,
                        expected_points,
                        request.non_ground_classification,
                    ),
                    limits.commit,
                )
                .blocking_wait_cancelled_by(&control.token())
                .map_err(|error| {
                    workspace_failure(WorkflowStage::Commit, error, control, context)
                })?
        }
        OperationResolution::Rejected(_) => workspace
            .commit(
                CommitRequest::set_classification(
                    operation,
                    expected_points,
                    request.non_ground_classification,
                ),
                limits.commit,
            )
            .blocking_wait_cancelled_by(&control.token())
            .map_err(|error| workspace_failure(WorkflowStage::Commit, error, control, context))?,
        OperationResolution::Indeterminate(uncertainty) => {
            return Err(indeterminate_commit(
                uncertainty.phase(),
                uncertainty.reason(),
                context,
            ));
        }
    };
    match outcome {
        CommitOutcome::Committed(receipt) => validate_revision(
            receipt.revision_info(),
            baseline,
            operation,
            request,
            context,
        ),
        CommitOutcome::Rejected(CommitRejection::OperationConflict) => Err(operation_conflict(
            context,
            "Operation Identity is durably bound to a different canonical request",
        )),
        CommitOutcome::Rejected(reason) => Err(rejected_failure(reason, context)),
        CommitOutcome::Indeterminate(uncertainty) => Err(indeterminate_commit(
            uncertainty.phase(),
            uncertainty.reason(),
            context,
        )),
    }
}

fn validate_revision(
    info: RevisionInfo,
    baseline: RevisionId,
    operation: OperationId,
    request: &WorkflowRunIntent,
    context: FailureContext,
) -> Result<RevisionInfo, WorkflowFailure> {
    let expected_count = usize_u64(request.correction_ordinals.len());
    if info.parent() != Some(baseline)
        || info.operation() != Some(operation)
        || info.kind()
            != (RevisionKind::SetClassification {
                value: request.non_ground_classification,
                changed_points: expected_count,
            })
    {
        return Err(operation_conflict(
            context,
            "committed Revision facts differ from durable Run Intent",
        ));
    }
    Ok(info)
}

fn validate_ground_ordinals(
    workspace: &Workspace,
    baseline: RevisionId,
    request: &WorkflowRunIntent,
    limits: PointRowLimits,
    control: &OperationControl,
    context: FailureContext,
) -> Result<(), WorkflowFailure> {
    let snapshot = workspace
        .snapshot(baseline)
        .map_err(|error| workspace_failure(WorkflowStage::Selection, error, control, context))?;
    let query = match request.recipe.bounds() {
        Some(bounds) => PointQuery::within(bounds),
        None => PointQuery::all(),
    }
    .classification_is(request.recipe.ground_classification());
    let mut rows = snapshot
        .point_rows(query, limits)
        .map_err(|error| workspace_failure(WorkflowStage::Selection, error, control, context))?;
    let mut next_requested = 0_usize;
    loop {
        if control.check_cancelled().is_err() {
            rows.handle().cancel();
            return Err(cancelled_failure(WorkflowStage::Selection, context));
        }
        let Some(batch) = rows.next().map_err(|error| {
            workspace_failure(WorkflowStage::Selection, error, control, context)
        })?
        else {
            break;
        };
        for ordinal in batch.ordinals() {
            if let Some(expected) = request.correction_ordinals.get(next_requested) {
                if expected < ordinal {
                    return Err(WorkflowFailure::invalid_with_context(
                        WorkflowStage::Selection,
                        context,
                        "a correction ordinal is not Ground in the expected baseline",
                    ));
                }
                if expected == ordinal {
                    next_requested += 1;
                }
            }
        }
    }
    if next_requested != request.correction_ordinals.len() {
        return Err(WorkflowFailure::invalid_with_context(
            WorkflowStage::Selection,
            context,
            "every correction ordinal must be Ground in the expected baseline and Recipe bounds",
        ));
    }
    Ok(())
}

fn durable_intent(
    request: &WorkflowRunIntent,
    source: [u8; 32],
    workspace: [u8; 16],
    bindings: [[u8; 32]; 4],
    limits: JournalLimits,
) -> Result<DurableIntent, JournalError> {
    DurableIntent::new(
        request.run,
        source,
        workspace,
        request.baseline_revision.into_bytes(),
        request.operation.into_bytes(),
        request.correction_ordinals.clone(),
        request.recipe.ground_classification(),
        request.non_ground_classification,
        bounds_bits(request.recipe.bounds()),
        request
            .check_points
            .iter()
            .map(|point| IntentCheckPoint {
                id: point.id().get(),
                position_bits: point.position().map(f64::to_bits),
            })
            .collect(),
        request.landxml.surface_name().into(),
        request.landxml.document_date().into(),
        request.landxml.document_time().into(),
        request.landxml.coordinates_are_metric_metres_asserted(),
        bindings,
        limits,
    )
}

fn validate_supplied_intent(
    durable: &DurableIntent,
    supplied: &WorkflowRunIntent,
    path_bindings: [[u8; 32]; 4],
) -> Result<(), WorkflowFailure> {
    let check_points_match = durable.check_points.len() == supplied.check_points.len()
        && durable
            .check_points
            .iter()
            .zip(supplied.check_points.iter())
            .all(|(left, right)| {
                left.id == right.id().get()
                    && left.position_bits == right.position().map(f64::to_bits)
            });
    if durable.run != supplied.run
        || durable.operation != supplied.operation.into_bytes()
        || durable.baseline_revision != supplied.baseline_revision.into_bytes()
        || durable.correction_ordinals.as_ref() != supplied.correction_ordinals.as_ref()
        || durable.ground_classification != supplied.recipe.ground_classification()
        || durable.non_ground_classification != supplied.non_ground_classification
        || durable.recipe_bounds_bits != bounds_bits(supplied.recipe.bounds())
        || !check_points_match
        || durable.surface_name.as_ref() != supplied.landxml.surface_name()
        || durable.document_date.as_ref() != supplied.landxml.document_date()
        || durable.document_time.as_ref() != supplied.landxml.document_time()
        || durable.coordinates_are_metric_metres_asserted
            != supplied.landxml.coordinates_are_metric_metres_asserted()
        || durable.path_bindings != path_bindings
    {
        return Err(WorkflowFailure::new(
            FailureCode::JournalConflict,
            WorkflowStage::Intent,
            Certainty::DurableFact,
            base_context(supplied),
            "supplied paths or intent differ from durable Intent",
            RecoveryAction::ResumeSameRun,
        ));
    }
    Ok(())
}

fn path_bindings(paths: &WorkflowPaths) -> Result<[[u8; 32]; 4], JournalError> {
    Ok([
        journal::bind_path(&paths.source, PATH_BINDING_BYTES)?,
        journal::bind_path(&paths.index, PATH_BINDING_BYTES)?,
        journal::bind_path(&paths.workspace, PATH_BINDING_BYTES)?,
        journal::bind_path(&paths.run_root, PATH_BINDING_BYTES)?,
    ])
}

fn validate_audit(
    audit: &RevisionAudit,
    request: &WorkflowRunIntent,
    revision: RevisionInfo,
    expected_point_id_hash: point_contracts::ContentHash,
    context: FailureContext,
) -> Result<(), WorkflowFailure> {
    let expected_count = usize_u64(request.correction_ordinals.len());
    if audit.revision() != revision
        || audit.changed_point_count() != expected_count
        || audit.transitions().len() != 1
        || audit.transitions()[0].before() != request.recipe.ground_classification()
        || audit.transitions()[0].after() != request.non_ground_classification
        || audit.transitions()[0].count() != expected_count
        || audit.point_id_hash() != expected_point_id_hash
    {
        return Err(operation_conflict(
            context,
            "Revision Audit differs from exact requested edit",
        ));
    }
    Ok(())
}

fn audit_checkpoint(audit: &RevisionAudit) -> AuditObserved {
    AuditObserved {
        revision: audit.revision().id().into_bytes(),
        content_hash: audit.content_hash().into_bytes(),
        point_id_hash: audit.point_id_hash().into_bytes(),
        changed_points: audit.changed_point_count(),
        transition_count: u32::try_from(audit.transitions().len()).unwrap_or(u32::MAX),
        footprint_bits: bounds_bits(audit.edit_footprint()),
    }
}

fn surface_checkpoint(
    baseline: &TerrainSurface,
    changed: &TerrainSurface,
    envelope: SurfaceChangeEnvelope,
    revision: [u8; 32],
    recipe_hash: [u8; 32],
) -> SurfaceObserved {
    let before = baseline.descriptor();
    let after = changed.descriptor();
    SurfaceObserved {
        revision,
        recipe_hash,
        baseline_artifact_hash: before.artifact_hash().into_bytes(),
        changed_artifact_hash: after.artifact_hash().into_bytes(),
        baseline_geometry_hash: before.geometry_hash().into_bytes(),
        changed_geometry_hash: after.geometry_hash().into_bytes(),
        baseline_topology_hash: before.topology_hash().into_bytes(),
        changed_topology_hash: after.topology_hash().into_bytes(),
        baseline_vertex_count: before.vertex_count(),
        baseline_face_count: before.face_count(),
        changed_vertex_count: after.vertex_count(),
        changed_face_count: after.face_count(),
        added_face_count: envelope.added_face_count,
        removed_face_count: envelope.removed_face_count,
        added_face_hash: envelope.added_face_hash,
        removed_face_hash: envelope.removed_face_hash,
        envelope_bits: envelope.bounds_bits,
    }
}

fn qa_checkpoint(
    qa: &point_terrain::CheckPointReport,
    surface: [u8; 32],
    result_hash: [u8; 32],
) -> QaObserved {
    let statistics = qa.statistics();
    let values = [
        statistics.minimum(),
        statistics.maximum(),
        statistics.mean(),
        statistics.root_mean_square(),
    ];
    let mut mask = 0_u8;
    let mut bits = [0; 4];
    for (index, value) in values.into_iter().enumerate() {
        if let Some(value) = value {
            mask |= 1 << index;
            bits[index] = value.to_bits();
        }
    }
    QaObserved {
        surface_artifact_hash: surface,
        result_hash,
        covered_count: statistics.covered_count(),
        gap_count: statistics.gap_count(),
        face_tests: qa.face_tests(),
        accounted_peak_working_bytes: qa.accounted_peak_working_bytes(),
        statistic_bits: bits,
        statistic_mask: mask,
    }
}

#[derive(Clone, Copy)]
struct FaceRecord {
    key: [PointId; 3],
    vertices: [u32; 3],
}

fn change_envelope(
    baseline: &TerrainSurface,
    changed: &TerrainSurface,
    max_faces: u64,
    max_working: u64,
    control: &OperationControl,
) -> Result<SurfaceChangeEnvelope, &'static str> {
    poll_control(control)?;
    let total = usize_u64(baseline.faces().len()).saturating_add(usize_u64(changed.faces().len()));
    if total > max_faces {
        return Err("Surface Change Envelope face limit exceeded");
    }
    let required = total.saturating_mul(usize_u64(std::mem::size_of::<FaceRecord>()));
    if required > max_working {
        return Err("Surface Change Envelope working-byte limit exceeded");
    }
    let mut before = face_records(baseline, control)?;
    let mut after = face_records(changed, control)?;
    let allocated = usize_u64(before.capacity())
        .saturating_add(usize_u64(after.capacity()))
        .saturating_mul(usize_u64(std::mem::size_of::<FaceRecord>()));
    if allocated > max_working {
        return Err("Surface Change Envelope actual allocation exceeds working-byte limit");
    }
    poll_control(control)?;
    before.sort_unstable_by_key(|record| record.key);
    poll_control(control)?;
    after.sort_unstable_by_key(|record| record.key);
    poll_control(control)?;
    let mut removed_hasher = Hasher::new();
    removed_hasher.update(ENVELOPE_HASH_DOMAIN);
    removed_hasher.update(b"removed");
    let mut added_hasher = Hasher::new();
    added_hasher.update(ENVELOPE_HASH_DOMAIN);
    added_hasher.update(b"added");
    let mut removed = 0_u64;
    let mut added = 0_u64;
    let mut bounds: Option<([f64; 3], [f64; 3])> = None;
    let (mut left, mut right) = (0, 0);
    let mut comparisons = 0_u64;
    while left < before.len() || right < after.len() {
        if comparisons.is_multiple_of(1_024) {
            poll_control(control)?;
        }
        comparisons = comparisons.saturating_add(1);
        match (before.get(left), after.get(right)) {
            (Some(a), Some(b)) if a.key == b.key => {
                left += 1;
                right += 1;
            }
            (Some(a), Some(b)) if a.key < b.key => {
                removed += 1;
                hash_face(&mut removed_hasher, a.key);
                extend_face_bounds(&mut bounds, baseline, a.vertices);
                left += 1;
            }
            (Some(_) | None, Some(b)) => {
                added += 1;
                hash_face(&mut added_hasher, b.key);
                extend_face_bounds(&mut bounds, changed, b.vertices);
                right += 1;
            }
            (Some(a), None) => {
                removed += 1;
                hash_face(&mut removed_hasher, a.key);
                extend_face_bounds(&mut bounds, baseline, a.vertices);
                left += 1;
            }
            (None, None) => break,
        }
    }
    Ok(SurfaceChangeEnvelope {
        added_face_count: added,
        removed_face_count: removed,
        added_face_hash: *added_hasher.finalize().as_bytes(),
        removed_face_hash: *removed_hasher.finalize().as_bytes(),
        bounds_bits: bounds.map(|(min, max)| {
            [
                [min[0].to_bits(), max[0].to_bits()],
                [min[1].to_bits(), max[1].to_bits()],
                [min[2].to_bits(), max[2].to_bits()],
            ]
        }),
    })
}

fn face_records(
    surface: &TerrainSurface,
    control: &OperationControl,
) -> Result<Vec<FaceRecord>, &'static str> {
    poll_control(control)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(surface.faces().len())
        .map_err(|_| "Surface Change Envelope allocation failed")?;
    for (index, face) in surface.faces().iter().enumerate() {
        if index.is_multiple_of(1_024) {
            poll_control(control)?;
        }
        let vertices = face.vertices().map(point_terrain::SurfaceVertexId::get);
        let key = vertices.map(|id| {
            surface
                .vertices()
                .get(usize::try_from(id - 1).unwrap_or(usize::MAX))
                .map(|v| v.point())
                .ok_or("Surface face references an invalid vertex")
        });
        let [a, b, c] = [key[0]?, key[1]?, key[2]?];
        let mut key = [a, b, c];
        key.sort_unstable();
        records.push(FaceRecord { key, vertices });
    }
    Ok(records)
}

fn extend_face_bounds(
    bounds: &mut Option<([f64; 3], [f64; 3])>,
    surface: &TerrainSurface,
    vertices: [u32; 3],
) {
    for id in vertices {
        if let Some(vertex) = surface
            .vertices()
            .get(usize::try_from(id - 1).unwrap_or(usize::MAX))
        {
            let point = surface
                .descriptor()
                .position_transform()
                .world_f64(vertex.ticks());
            match bounds {
                None => *bounds = Some((point, point)),
                Some((min, max)) => {
                    for axis in 0..3 {
                        min[axis] = min[axis].min(point[axis]);
                        max[axis] = max[axis].max(point[axis]);
                    }
                }
            }
        }
    }
}

fn hash_face(hasher: &mut Hasher, key: [PointId; 3]) {
    for point in key {
        hasher.update(point.source().as_bytes());
        hasher.update(&point.ordinal().to_le_bytes());
    }
}

fn qa_hash(
    qa: &point_terrain::CheckPointReport,
    control: &OperationControl,
) -> Result<[u8; 32], &'static str> {
    let mut hasher = Hasher::new();
    hasher.update(QA_HASH_DOMAIN);
    for (index, result) in qa.results().iter().enumerate() {
        if index.is_multiple_of(1_024) {
            poll_control(control)?;
        }
        let point = result.check_point();
        hasher.update(&point.id().get().to_le_bytes());
        for value in point.position() {
            hasher.update(&value.to_bits().to_le_bytes());
        }
        match result.outcome() {
            CheckPointOutcome::Gap => {
                hasher.update(&[0]);
            }
            CheckPointOutcome::Sampled {
                face,
                surface_z,
                residual,
            } => {
                hasher.update(&[1]);
                hasher.update(&face.get().to_le_bytes());
                hasher.update(&surface_z.to_bits().to_le_bytes());
                hasher.update(&residual.to_bits().to_le_bytes());
            }
        }
    }
    Ok(*hasher.finalize().as_bytes())
}

fn semantic_results_hash(
    audit: &RevisionAudit,
    baseline: &TerrainSurface,
    changed: &TerrainSurface,
    envelope: SurfaceChangeEnvelope,
    qa: &point_terrain::CheckPointReport,
    landxml: &point_terrain::LandXmlReceipt,
    control: &OperationControl,
) -> Result<[u8; 32], &'static str> {
    poll_control(control)?;
    let mut hasher = Hasher::new();
    hasher.update(SEMANTIC_HASH_DOMAIN);
    hasher.update(&audit.changed_point_count().to_le_bytes());
    for (index, transition) in audit.transitions().iter().enumerate() {
        if index.is_multiple_of(1_024) {
            poll_control(control)?;
        }
        hasher.update(&[transition.before(), transition.after()]);
        hasher.update(&transition.count().to_le_bytes());
    }
    for surface in [baseline, changed] {
        let transform = surface.descriptor().position_transform();
        for (index, vertex) in surface.vertices().iter().enumerate() {
            if index.is_multiple_of(1_024) {
                poll_control(control)?;
            }
            for coordinate in transform.world_f64(vertex.ticks()) {
                hasher.update(&coordinate.to_bits().to_le_bytes());
            }
        }
        for (index, face) in surface.faces().iter().enumerate() {
            if index.is_multiple_of(1_024) {
                poll_control(control)?;
            }
            for vertex in face.vertices() {
                hasher.update(&vertex.get().to_le_bytes());
            }
        }
    }
    hasher.update(&envelope.added_face_count.to_le_bytes());
    hasher.update(&envelope.removed_face_count.to_le_bytes());
    hasher.update(&qa_hash(qa, control)?);
    hasher.update(&landxml.byte_length().to_le_bytes());
    hasher.update(landxml.content_hash().as_bytes());
    Ok(*hasher.finalize().as_bytes())
}

fn poll_control(control: &OperationControl) -> Result<(), &'static str> {
    control
        .check_cancelled()
        .map_err(|_| "workflow cancellation was requested")
}

fn limit_facts(
    limits: &WorkflowLimits,
    control: &OperationControl,
) -> Result<Vec<LimitFact>, &'static str> {
    poll_control(control)?;
    let mut facts = Vec::new();
    facts
        .try_reserve_exact(LIMIT_FACT_COUNT)
        .map_err(|_| "Workflow Limit Fact allocation failed")?;

    macro_rules! fact {
        ($name:expr, $value:expr) => {{
            if facts.len() == LIMIT_FACT_COUNT {
                return Err("Workflow Limit Fact schema exceeded its fixed bound");
            }
            facts.push(LimitFact {
                name: $name,
                value: $value,
            });
        }};
    }
    macro_rules! candidate_facts {
        ($prefix:literal, $limits:expr) => {{
            let candidate = $limits;
            fact!(
                concat!($prefix, ".max_visited_nodes"),
                candidate.max_visited_nodes()
            );
            fact!(
                concat!($prefix, ".max_output_spans"),
                candidate.max_output_spans()
            );
            fact!(
                concat!($prefix, ".max_candidate_points"),
                candidate.max_candidate_points()
            );
            fact!(
                concat!($prefix, ".max_working_bytes"),
                candidate.max_working_bytes()
            );
        }};
    }
    macro_rules! source_read_facts {
        ($prefix:literal, $limits:expr) => {{
            let source_read = $limits;
            fact!(concat!($prefix, ".max_spans"), source_read.max_spans());
            fact!(concat!($prefix, ".max_points"), source_read.max_points());
            fact!(
                concat!($prefix, ".max_batch_points"),
                source_read.max_batch_points()
            );
            fact!(
                concat!($prefix, ".max_batch_payload_bytes"),
                source_read.max_batch_payload_bytes()
            );
            fact!(
                concat!($prefix, ".max_adapter_working_bytes"),
                source_read.max_adapter_working_bytes()
            );
        }};
    }
    macro_rules! row_facts {
        ($prefix:literal, $candidate_prefix:literal, $read_prefix:literal, $limits:expr) => {{
            let rows = $limits;
            candidate_facts!($candidate_prefix, rows.candidate_limits());
            source_read_facts!($read_prefix, rows.source_read_budget());
            fact!(
                concat!($prefix, ".max_overlay_segments"),
                rows.max_overlay_segments()
            );
            fact!(
                concat!($prefix, ".max_overlay_bytes"),
                rows.max_overlay_bytes()
            );
            fact!(
                concat!($prefix, ".max_output_points"),
                rows.max_output_points()
            );
            fact!(
                concat!($prefix, ".max_batch_points"),
                rows.max_batch_points()
            );
            fact!(
                concat!($prefix, ".max_batch_payload_bytes"),
                rows.max_batch_payload_bytes()
            );
            fact!(
                concat!($prefix, ".max_working_bytes"),
                rows.max_working_bytes()
            );
        }};
    }

    let prepare = limits.prepare;
    fact!(
        "prepare.max_source_batch_points",
        prepare.max_source_batch_points()
    );
    fact!(
        "prepare.max_source_batch_payload_bytes",
        prepare.max_source_batch_payload_bytes()
    );
    fact!(
        "prepare.max_adapter_working_bytes",
        prepare.max_adapter_working_bytes()
    );
    fact!(
        "prepare.max_build_working_bytes",
        prepare.max_build_working_bytes()
    );
    fact!(
        "prepare.max_incomplete_bytes",
        prepare.max_incomplete_bytes()
    );
    fact!("prepare.max_artifact_bytes", prepare.max_artifact_bytes());
    fact!("prepare.max_hierarchy_nodes", prepare.max_hierarchy_nodes());
    fact!(
        "prepare.max_resident_metadata_bytes",
        prepare.max_resident_metadata_bytes()
    );

    let open = limits.open;
    fact!("open.max_manifest_bytes", open.max_manifest_bytes());
    fact!("open.max_operation_records", open.max_operation_records());
    fact!("open.max_revision_files", open.max_revision_files());
    fact!("open.max_revision_blocks", open.max_revision_blocks());
    fact!("open.max_revision_rows", open.max_revision_rows());
    fact!(
        "open.max_revision_block_bytes",
        open.max_revision_block_bytes()
    );
    fact!("open.max_single_file_bytes", open.max_single_file_bytes());
    fact!(
        "open.max_total_persisted_bytes",
        open.max_total_persisted_bytes()
    );
    fact!("open.max_working_bytes", open.max_working_bytes());
    fact!(
        "open.max_resident_metadata_bytes",
        open.max_resident_metadata_bytes()
    );

    row_facts!("rows", "rows.candidate", "rows.source_read", limits.rows);

    let selection = limits.selection;
    candidate_facts!("selection.candidate", selection.candidate_limits());
    source_read_facts!("selection.source_read", selection.source_read_budget());
    fact!(
        "selection.max_input_point_ids",
        selection.max_input_point_ids()
    );
    fact!("selection.max_output_points", selection.max_output_points());
    fact!(
        "selection.max_overlay_segments",
        selection.max_overlay_segments()
    );
    fact!("selection.max_overlay_bytes", selection.max_overlay_bytes());
    fact!("selection.max_working_bytes", selection.max_working_bytes());
    fact!(
        "selection.max_resident_bytes",
        selection.max_resident_bytes()
    );
    fact!(
        "selection.max_temporary_bytes",
        selection.max_temporary_bytes()
    );

    let commit = limits.commit;
    fact!("commit.max_selected_points", commit.max_selected_points());
    fact!("commit.max_changed_points", commit.max_changed_points());
    fact!("commit.max_input_frames", commit.max_input_frames());
    fact!("commit.max_block_points", commit.max_block_points());
    fact!("commit.max_block_bytes", commit.max_block_bytes());
    fact!("commit.max_working_bytes", commit.max_working_bytes());
    fact!("commit.max_temporary_bytes", commit.max_temporary_bytes());
    fact!("commit.max_revision_bytes", commit.max_revision_bytes());
    fact!(
        "commit.max_total_durable_bytes",
        commit.max_total_durable_bytes()
    );

    let audit = limits.audit;
    source_read_facts!("audit.source_read", audit.source_read_budget());
    fact!("audit.max_revision_blocks", audit.max_revision_blocks());
    fact!("audit.max_revision_bytes", audit.max_revision_bytes());
    fact!("audit.max_changed_points", audit.max_changed_points());
    fact!(
        "audit.max_transition_entries",
        audit.max_transition_entries()
    );
    fact!("audit.max_result_bytes", audit.max_result_bytes());
    fact!("audit.max_working_bytes", audit.max_working_bytes());

    let terrain = limits.terrain;
    row_facts!(
        "terrain.point_rows",
        "terrain.point_rows.candidate",
        "terrain.point_rows.source_read",
        terrain.point_rows()
    );
    fact!("terrain.max_input_points", terrain.max_input_points());
    fact!("terrain.max_faces", terrain.max_faces());
    fact!("terrain.max_working_bytes", terrain.max_working_bytes());
    fact!("terrain.max_surface_bytes", terrain.max_surface_bytes());
    fact!("terrain.max_work_units", terrain.max_work_units());

    let qa = limits.qa;
    fact!("qa.max_check_points", qa.max_check_points());
    fact!("qa.max_result_bytes", qa.max_result_bytes());
    fact!("qa.max_face_tests", qa.max_face_tests());
    fact!("qa.max_working_bytes", qa.max_working_bytes());

    let landxml = limits.landxml;
    fact!("landxml.max_vertices", landxml.max_vertices());
    fact!("landxml.max_faces", landxml.max_faces());
    fact!("landxml.max_output_bytes", landxml.max_output_bytes());
    fact!("landxml.max_staging_bytes", landxml.max_staging_bytes());
    fact!(
        "landxml.max_write_buffer_bytes",
        landxml.max_write_buffer_bytes()
    );
    fact!("landxml.max_xml_token_bytes", landxml.max_xml_token_bytes());
    fact!("landxml.max_working_bytes", landxml.max_working_bytes());

    fact!(
        "journal.max_journal_bytes",
        limits.journal.max_journal_bytes
    );
    fact!("journal.max_frames", limits.journal.max_frames);
    fact!(
        "journal.max_frame_payload_bytes",
        limits.journal.max_frame_payload_bytes
    );
    fact!(
        "journal.max_working_bytes",
        limits.journal.max_working_bytes
    );
    fact!(
        "journal.max_correction_ordinals",
        limits.journal.max_correction_ordinals
    );
    fact!("journal.max_check_points", limits.journal.max_check_points);
    fact!(
        "journal.max_surface_name_bytes",
        limits.journal.max_surface_name_bytes
    );
    fact!("journal.max_path_binding_bytes", PATH_BINDING_BYTES);

    fact!("report.max_output_bytes", limits.report.max_output_bytes);
    fact!("report.max_staging_bytes", limits.report.max_staging_bytes);
    fact!(
        "report.max_write_buffer_bytes",
        limits.report.max_write_buffer_bytes
    );
    fact!("report.max_working_bytes", limits.report.max_working_bytes);
    fact!("envelope.max_faces", limits.max_envelope_faces);
    fact!(
        "envelope.max_working_bytes",
        limits.max_envelope_working_bytes
    );
    fact!(
        "workflow.max_aggregate_working_bytes",
        limits.max_aggregate_working_bytes
    );
    poll_control(control)?;
    if facts.len() != LIMIT_FACT_COUNT {
        return Err("Workflow Limit Fact schema is incomplete");
    }
    Ok(facts)
}

fn record(
    journal: &mut Journal,
    witness: &DirectoryWitness,
    lock: &RunLock,
    control: &OperationControl,
    checkpoint: Checkpoint,
    stage: WorkflowStage,
    context: FailureContext,
) -> Result<(), WorkflowFailure> {
    control
        .check_cancelled()
        .map_err(|_| cancelled_failure(stage, context))?;
    verify_run_binding(lock, witness).map_err(|error| io_failure(stage, error, context))?;
    journal
        .record(checkpoint)
        .map(|_| ())
        .map_err(|error| journal_failure(stage, error, context))?;
    verify_run_binding(lock, witness)
        .map_err(|error| checkpoint_binding_failure(stage, error, context))
}

fn checkpoint_binding_failure(
    stage: WorkflowStage,
    error: io::Error,
    context: FailureContext,
) -> WorkflowFailure {
    let publication_phase = if stage == WorkflowStage::Complete {
        PublicationPhase::CompleteCheckpoint
    } else {
        PublicationPhase::JournalCheckpoint
    };
    WorkflowFailure::new(
        FailureCode::PublicationIndeterminate,
        stage,
        Certainty::Indeterminate(publication_phase),
        context,
        error,
        RecoveryAction::ResumeSameRun,
    )
}

fn bounds_bits(bounds: Option<WorldBounds>) -> Option<[[u64; 2]; 3]> {
    bounds.map(|bounds| {
        let min = bounds.min();
        let max = bounds.max();
        [
            [min[0].to_bits(), max[0].to_bits()],
            [min[1].to_bits(), max[1].to_bits()],
            [min[2].to_bits(), max[2].to_bits()],
        ]
    })
}

fn base_context(request: &WorkflowRunIntent) -> FailureContext {
    FailureContext {
        run: Some(request.run),
        operation: Some(request.operation),
        revision: Some(request.baseline_revision),
        ..FailureContext::default()
    }
}

fn durable_context(run: WorkflowRunId, intent: &DurableIntent) -> FailureContext {
    FailureContext {
        run: Some(run),
        source: Some(point_contracts::SourceId::new(intent.source)),
        workspace: point_workspace::WorkspaceId::from_bytes(intent.workspace).ok(),
        operation: OperationId::from_bytes(intent.operation).ok(),
        revision: RevisionId::from_bytes(intent.baseline_revision).ok(),
    }
}

fn rejected_failure(reason: CommitRejection, context: FailureContext) -> WorkflowFailure {
    WorkflowFailure::new(
        FailureCode::OperationRejected,
        WorkflowStage::Commit,
        Certainty::DurableFact,
        context,
        format_args!("Workspace definitively rejected Operation: {reason:?}"),
        RecoveryAction::StopAndPreserve,
    )
}

fn indeterminate_commit(
    phase: CommitPhase,
    error: impl std::fmt::Display,
    context: FailureContext,
) -> WorkflowFailure {
    let publication = match phase {
        CommitPhase::OperationPublication => PublicationPhase::WorkspaceOperation,
        CommitPhase::RevisionPublication => PublicationPhase::WorkspaceRevision,
        CommitPhase::RevisionDirectorySync => PublicationPhase::WorkspaceDirectorySync,
    };
    WorkflowFailure::new(
        FailureCode::OperationIndeterminate,
        WorkflowStage::Commit,
        Certainty::Indeterminate(publication),
        context,
        error,
        RecoveryAction::ResolveRecordedOperationByResuming,
    )
}

fn operation_conflict(context: FailureContext, message: &'static str) -> WorkflowFailure {
    WorkflowFailure::new(
        FailureCode::JournalConflict,
        WorkflowStage::ResolveOperation,
        Certainty::DurableFact,
        context,
        message,
        RecoveryAction::StopAndPreserve,
    )
}

fn phase_failure(
    stage: WorkflowStage,
    error: impl std::fmt::Display,
    context: FailureContext,
) -> WorkflowFailure {
    WorkflowFailure::new(
        FailureCode::Internal,
        stage,
        Certainty::PrePublication,
        context,
        error,
        RecoveryAction::ResumeSameRun,
    )
}

fn child_failure(
    stage: WorkflowStage,
    error: impl std::fmt::Display,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    if control.check_cancelled().is_err() {
        cancelled_failure(stage, context)
    } else {
        phase_failure(stage, error, context)
    }
}

fn envelope_failure(
    error: &'static str,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    if control.check_cancelled().is_err() {
        return cancelled_failure(WorkflowStage::ChangeEnvelope, context);
    }
    if matches!(
        error,
        "Surface Change Envelope face limit exceeded"
            | "Surface Change Envelope working-byte limit exceeded"
            | "Surface Change Envelope allocation failed"
            | "Surface Change Envelope actual allocation exceeds working-byte limit"
    ) {
        WorkflowFailure::new(
            FailureCode::ResourceLimit,
            WorkflowStage::ChangeEnvelope,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RaiseLimitOrNarrow,
        )
    } else {
        phase_failure(WorkflowStage::ChangeEnvelope, error, context)
    }
}

fn source_failure(
    stage: WorkflowStage,
    error: SourceError,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    if control.check_cancelled().is_err() || matches!(&error, SourceError::Cancelled) {
        return cancelled_failure(stage, context);
    }
    let (code, action) = match &error {
        SourceError::ResourceLimit { .. } => (
            FailureCode::ResourceLimit,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        SourceError::InvalidBudget { .. } | SourceError::InvalidSourceSpan { .. } => (
            FailureCode::InvalidRequest,
            RecoveryAction::CorrectInvalidRequest,
        ),
        SourceError::SourceMissing { .. }
        | SourceError::CorruptSource { .. }
        | SourceError::UnsupportedFormat { .. }
        | SourceError::UnsupportedSchema { .. }
        | SourceError::VerificationRequired
        | SourceError::SourceChanged { .. }
        | SourceError::SourceContractMismatch { .. }
        | SourceError::UnsupportedRecordVersion { .. }
        | SourceError::AdapterSourceMismatch { .. }
        | SourceError::AdapterTransformMismatch => (
            FailureCode::SourceMismatch,
            RecoveryAction::RestoreExpectedSource,
        ),
        _ => (FailureCode::Internal, RecoveryAction::StopAndPreserve),
    };
    WorkflowFailure::new(
        code,
        stage,
        Certainty::PrePublication,
        context,
        error,
        action,
    )
}

fn index_failure(
    stage: WorkflowStage,
    error: IndexError,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    if matches!(
        &error,
        IndexError::Runtime(foundation_runtime::RuntimeError::Cancelled)
    ) || (control.check_cancelled().is_err() && !matches!(&error, IndexError::Io { .. }))
    {
        return cancelled_failure(stage, context);
    }
    match error {
        IndexError::Source(source) => source_failure(stage, source, control, context),
        error @ IndexError::ResourceLimit { .. } => WorkflowFailure::new(
            FailureCode::ResourceLimit,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        error @ (IndexError::ZeroNodeId | IndexError::InvalidLimit { .. }) => WorkflowFailure::new(
            FailureCode::InvalidRequest,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::CorrectInvalidRequest,
        ),
        error @ (IndexError::IncompatibleArtifact { .. } | IndexError::IncompatibleWork { .. }) => {
            WorkflowFailure::new(
                FailureCode::SourceMismatch,
                stage,
                Certainty::DurableFact,
                context,
                error,
                RecoveryAction::RestoreExpectedSource,
            )
        }
        error @ (IndexError::CorruptArtifact { .. }
        | IndexError::CorruptWork { .. }
        | IndexError::UnsupportedVersion { .. }) => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::DurableFact,
            context,
            error,
            RecoveryAction::StopAndPreserve,
        ),
        error @ IndexError::Io { .. } => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::Indeterminate(PublicationPhase::IndexTarget),
            context,
            error,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
        error => phase_failure(stage, error, context),
    }
}

fn terrain_failure(
    stage: WorkflowStage,
    error: point_terrain::TerrainError,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    if matches!(&error, point_terrain::TerrainError::Cancelled)
        || (control.check_cancelled().is_err() && !terrain_error_is_io(&error))
    {
        return cancelled_failure(stage, context);
    }
    match error {
        error @ point_terrain::TerrainError::ResourceLimit { .. } => WorkflowFailure::new(
            FailureCode::ResourceLimit,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        point_terrain::TerrainError::Workspace { source, .. } => {
            workspace_failure(stage, *source, control, context)
        }
        error @ (point_terrain::TerrainError::InvalidArgument { .. }
        | point_terrain::TerrainError::InsufficientGroundInput { .. }
        | point_terrain::TerrainError::DuplicateHorizontalPosition { .. }
        | point_terrain::TerrainError::CollinearGroundInput
        | point_terrain::TerrainError::UnsupportedNumericRange { .. }) => WorkflowFailure::new(
            FailureCode::InvalidRequest,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::CorrectInvalidRequest,
        ),
        error @ point_terrain::TerrainError::Io { .. } => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
        error => phase_failure(stage, error, context),
    }
}

fn terrain_error_is_io(error: &point_terrain::TerrainError) -> bool {
    match error {
        point_terrain::TerrainError::Io { .. } => true,
        point_terrain::TerrainError::Workspace { source, .. } => {
            matches!(source.as_ref(), WorkspaceError::Io { .. })
        }
        _ => false,
    }
}

fn workspace_failure(
    stage: WorkflowStage,
    error: WorkspaceError,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    if matches!(&error, WorkspaceError::Cancelled)
        || (control.check_cancelled().is_err() && !matches!(&error, WorkspaceError::Io { .. }))
    {
        return cancelled_failure(stage, context);
    }
    let (code, certainty, action) = match &error {
        WorkspaceError::ResourceLimit { .. } => (
            FailureCode::ResourceLimit,
            Certainty::PrePublication,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        WorkspaceError::Poisoned | WorkspaceError::RecoveryIndeterminate { .. } => (
            FailureCode::OperationIndeterminate,
            Certainty::Indeterminate(PublicationPhase::WorkspaceDirectorySync),
            RecoveryAction::ResolveRecordedOperationByResuming,
        ),
        WorkspaceError::OperationConflict { .. } | WorkspaceError::OperationNotRetryable { .. } => {
            (
                FailureCode::JournalConflict,
                Certainty::DurableFact,
                RecoveryAction::StopAndPreserve,
            )
        }
        WorkspaceError::Incompatible { .. }
        | WorkspaceError::Corrupt { .. }
        | WorkspaceError::UnknownRevision { .. }
        | WorkspaceError::InvalidPointSet { .. } => (
            FailureCode::WorkspaceMismatch,
            Certainty::DurableFact,
            RecoveryAction::StopAndPreserve,
        ),
        WorkspaceError::InvalidArgument { .. } => (
            FailureCode::InvalidRequest,
            Certainty::PrePublication,
            RecoveryAction::CorrectInvalidRequest,
        ),
        WorkspaceError::Locked => (
            FailureCode::Io,
            Certainty::PrePublication,
            RecoveryAction::ResumeSameRun,
        ),
        WorkspaceError::Io { .. } => (
            FailureCode::Io,
            Certainty::PrePublication,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
        _ => (
            FailureCode::Internal,
            Certainty::PrePublication,
            RecoveryAction::ResumeSameRun,
        ),
    };
    WorkflowFailure::new(code, stage, certainty, context, error, action)
}

fn cancelled_failure(stage: WorkflowStage, context: FailureContext) -> WorkflowFailure {
    WorkflowFailure::new(
        FailureCode::Cancelled,
        stage,
        Certainty::PrePublication,
        context,
        "workflow cancellation was requested",
        RecoveryAction::ResumeSameRun,
    )
}

fn terrain_output_failure(
    stage: WorkflowStage,
    error: point_terrain::TerrainError,
    control: &OperationControl,
    context: FailureContext,
) -> WorkflowFailure {
    match error {
        point_terrain::TerrainError::ExportConflict { .. }
        | point_terrain::TerrainError::TargetExists { .. } => WorkflowFailure::new(
            FailureCode::OutputConflict,
            stage,
            Certainty::DurableFact,
            context,
            error,
            RecoveryAction::RemoveOrRenameConflictingTarget,
        ),
        point_terrain::TerrainError::ExportIndeterminate { .. } => WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            stage,
            Certainty::Indeterminate(PublicationPhase::LandXmlTarget),
            context,
            error,
            RecoveryAction::ResumeSameRun,
        ),
        error @ point_terrain::TerrainError::TargetChanged { .. } => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::ResumeSameRun,
        ),
        error @ point_terrain::TerrainError::Io { .. } => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
        error => terrain_failure(stage, error, control, context),
    }
}

fn report_failure(
    stage: WorkflowStage,
    error: CanonicalOutputError,
    context: FailureContext,
) -> WorkflowFailure {
    match error {
        CanonicalOutputError::Conflict { .. } | CanonicalOutputError::TargetConflict { .. } => {
            WorkflowFailure::new(
                FailureCode::OutputConflict,
                stage,
                Certainty::DurableFact,
                context,
                error,
                RecoveryAction::RemoveOrRenameConflictingTarget,
            )
        }
        CanonicalOutputError::TargetChanged { .. } => WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            stage,
            Certainty::Indeterminate(PublicationPhase::ReportTarget),
            context,
            error,
            RecoveryAction::StopAndPreserve,
        ),
        CanonicalOutputError::Indeterminate { .. } => WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            stage,
            Certainty::Indeterminate(PublicationPhase::ReportTarget),
            context,
            error,
            RecoveryAction::ResumeSameRun,
        ),
        CanonicalOutputError::Resource { .. } => WorkflowFailure::new(
            FailureCode::ResourceLimit,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        CanonicalOutputError::Cancelled => cancelled_failure(stage, context),
        error @ CanonicalOutputError::Invalid(_) => WorkflowFailure::new(
            FailureCode::InvalidRequest,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::CorrectInvalidRequest,
        ),
        error @ CanonicalOutputError::Io { .. } => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
    }
}

fn journal_failure(
    stage: WorkflowStage,
    error: JournalError,
    context: FailureContext,
) -> WorkflowFailure {
    match error {
        JournalError::Resource { .. } => WorkflowFailure::new(
            FailureCode::ResourceLimit,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        JournalError::Indeterminate { .. } => WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            stage,
            Certainty::Indeterminate(PublicationPhase::JournalIntent),
            context,
            error,
            RecoveryAction::ResumeSameRun,
        ),
        JournalError::CheckpointIndeterminate { .. } => WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            stage,
            Certainty::Indeterminate(if stage == WorkflowStage::Complete {
                PublicationPhase::CompleteCheckpoint
            } else {
                PublicationPhase::JournalCheckpoint
            }),
            context,
            error,
            RecoveryAction::ResumeSameRun,
        ),
        JournalError::Corrupt(_) | JournalError::Incompatible(_) => WorkflowFailure::new(
            FailureCode::JournalCorrupt,
            stage,
            Certainty::DurableFact,
            context,
            error,
            RecoveryAction::StopAndPreserve,
        ),
        error @ (JournalError::Exists(_) | JournalError::Conflict(_)) => WorkflowFailure::new(
            FailureCode::JournalConflict,
            stage,
            Certainty::DurableFact,
            context,
            error,
            RecoveryAction::StopAndPreserve,
        ),
        error @ JournalError::Invalid(_) => WorkflowFailure::new(
            FailureCode::InvalidRequest,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::CorrectInvalidRequest,
        ),
        error @ JournalError::Locked => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::ResumeSameRun,
        ),
        error @ (JournalError::Entropy | JournalError::Io { .. }) => WorkflowFailure::new(
            FailureCode::Io,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
    }
}

fn io_failure(stage: WorkflowStage, error: io::Error, context: FailureContext) -> WorkflowFailure {
    let action = if error.kind() == io::ErrorKind::InvalidInput {
        RecoveryAction::CorrectInvalidRequest
    } else {
        RecoveryAction::RetryAfterRestoringDisk
    };
    WorkflowFailure::new(
        FailureCode::Io,
        stage,
        Certainty::PrePublication,
        context,
        error,
        action,
    )
}

fn lock_failure(error: io::Error, context: FailureContext) -> WorkflowFailure {
    let action = match error.kind() {
        io::ErrorKind::WouldBlock => RecoveryAction::ResumeSameRun,
        io::ErrorKind::InvalidInput => RecoveryAction::CorrectInvalidRequest,
        _ => RecoveryAction::RetryAfterRestoringDisk,
    };
    WorkflowFailure::new(
        FailureCode::Io,
        WorkflowStage::Lock,
        Certainty::PrePublication,
        context,
        error,
        action,
    )
}

fn validate_run_root(path: &Path, context: FailureContext) -> Result<(), WorkflowFailure> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| io_failure(WorkflowStage::Validate, error, context))?;
    if !metadata.file_type().is_dir() {
        return Err(WorkflowFailure::new(
            FailureCode::InvalidRequest,
            WorkflowStage::Validate,
            Certainty::PrePublication,
            context,
            "Run root must be an existing non-symlink directory",
            RecoveryAction::CorrectInvalidRequest,
        ));
    }
    Ok(())
}

struct RunLock {
    file: File,
    path: PathBuf,
    identity: fs::Metadata,
}

impl RunLock {
    fn acquire(path: &Path) -> io::Result<Self> {
        let initial = match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !is_empty_regular_lock(&metadata) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "run.lock must be an empty regular non-symlink file",
                    ));
                }
                Some(metadata)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let opened = file.metadata()?;
        let current = fs::symlink_metadata(path)?;
        if !is_empty_regular_lock(&opened)
            || !is_empty_regular_lock(&current)
            || !same_file_identity(&opened, &current)
            || initial
                .as_ref()
                .is_some_and(|value| !same_file_identity(value, &opened))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "run.lock changed identity while it was opened",
            ));
        }
        file.try_lock().map_err(io::Error::from)?;
        let locked_path = fs::symlink_metadata(path)?;
        if !is_empty_regular_lock(&locked_path) || !same_file_identity(&opened, &locked_path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "run.lock changed identity or contents while it was locked",
            ));
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            identity: opened,
        })
    }

    fn verify(&self) -> io::Result<()> {
        let opened = self.file.metadata()?;
        let current = fs::symlink_metadata(&self.path)?;
        if is_empty_regular_lock(&opened)
            && is_empty_regular_lock(&current)
            && same_file_identity(&self.identity, &opened)
            && same_file_identity(&opened, &current)
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "run.lock path no longer names the locked file",
            ))
        }
    }
}

fn is_empty_regular_lock(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file() && metadata.len() == 0
}

fn verify_run_binding(lock: &RunLock, witness: &DirectoryWitness) -> io::Result<()> {
    witness.verify()?;
    lock.verify()
}

struct DirectoryWitness {
    path: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
}

impl DirectoryWitness {
    fn capture(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Run root changed type",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                path: path.to_path_buf(),
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;

            let volume_serial_number = metadata.volume_serial_number().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Run root volume identity is unavailable",
                )
            })?;
            let file_index = metadata.file_index().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Run root file identity is unavailable",
                )
            })?;
            Ok(Self {
                path: path.to_path_buf(),
                volume_serial_number,
                file_index,
            })
        }
        #[cfg(not(any(unix, windows)))]
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn verify(&self) -> io::Result<()> {
        let current = Self::capture(&self.path)?;
        #[cfg(unix)]
        if current.device != self.device || current.inode != self.inode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Run root directory identity changed",
            ));
        }
        #[cfg(windows)]
        if current.volume_serial_number != self.volume_serial_number
            || current.file_index != self.file_index
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Run root directory identity changed",
            ));
        }
        Ok(())
    }
}

fn usize_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn workflow_retained_bytes(
    journal: &Journal,
    request: &WorkflowRunIntent,
    limits: &WorkflowLimits,
) -> u64 {
    journal
        .retained_bytes()
        .saturating_add(limits.prepare.max_resident_metadata_bytes())
        .saturating_add(limits.open.max_resident_metadata_bytes())
        .saturating_add(request_retained_bytes(request))
}

fn run_entry_retained_bytes(request: &WorkflowRunIntent, journal: Option<&Journal>) -> u64 {
    request_retained_bytes(request).saturating_add(journal.map_or(0, Journal::retained_bytes))
}

fn request_retained_bytes(request: &WorkflowRunIntent) -> u64 {
    usize_u64(std::mem::size_of::<WorkflowRunIntent>())
        .saturating_add(
            usize_u64(request.correction_ordinals.len())
                .saturating_mul(usize_u64(std::mem::size_of::<u64>())),
        )
        .saturating_add(
            usize_u64(request.check_points.len())
                .saturating_mul(usize_u64(std::mem::size_of::<CheckPoint>())),
        )
        .saturating_add(usize_u64(request.landxml.surface_name().len()))
        .saturating_add(usize_u64(request.landxml.document_date().len()))
        .saturating_add(usize_u64(request.landxml.document_time().len()))
}

fn retained_observations_bytes(
    journal: &Journal,
    request: &WorkflowRunIntent,
    limits: &WorkflowLimits,
    audit: &RevisionAudit,
    baseline: &TerrainSurface,
    changed: &TerrainSurface,
) -> u64 {
    workflow_retained_bytes(journal, request, limits)
        .saturating_add(audit.retained_result_bytes())
        .saturating_add(baseline.descriptor().retained_surface_bytes())
        .saturating_add(changed.descriptor().retained_surface_bytes())
}

fn qa_retained_bytes(qa: &point_terrain::CheckPointReport) -> u64 {
    usize_u64(std::mem::size_of::<point_terrain::CheckPointReport>())
        .saturating_add(usize_u64(std::mem::size_of_val(qa.results())))
}

fn copy_check_points_for_job(
    check_points: &[CheckPoint],
    retained_bytes: u64,
    limits: &WorkflowLimits,
    context: FailureContext,
) -> Result<Vec<CheckPoint>, WorkflowFailure> {
    let count = usize_u64(check_points.len());
    if count > limits.qa.max_check_points() {
        return Err(WorkflowFailure::new(
            FailureCode::ResourceLimit,
            WorkflowStage::CheckPointQa,
            Certainty::PrePublication,
            context,
            format_args!(
                "detached Check Point count requires {count}, limit {}",
                limits.qa.max_check_points()
            ),
            RecoveryAction::RaiseLimitOrNarrow,
        ));
    }
    let requested_bytes = usize_u64(std::mem::size_of_val(check_points));
    require_workflow_bytes(
        retained_bytes
            .saturating_add(requested_bytes)
            .saturating_add(limits.qa.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::CheckPointQa,
        context,
    )?;
    let mut owned = Vec::new();
    owned.try_reserve_exact(check_points.len()).map_err(|_| {
        WorkflowFailure::new(
            FailureCode::ResourceLimit,
            WorkflowStage::CheckPointQa,
            Certainty::PrePublication,
            context,
            "failed to allocate bounded detached Check Point job input",
            RecoveryAction::RaiseLimitOrNarrow,
        )
    })?;
    let allocated_bytes =
        usize_u64(owned.capacity()).saturating_mul(usize_u64(std::mem::size_of::<CheckPoint>()));
    require_workflow_bytes(
        retained_bytes
            .saturating_add(allocated_bytes)
            .saturating_add(limits.qa.max_working_bytes()),
        limits.max_aggregate_working_bytes,
        WorkflowStage::CheckPointQa,
        context,
    )?;
    owned.extend_from_slice(check_points);
    Ok(owned)
}

fn require_workflow_bytes(
    required: u64,
    allowed: u64,
    stage: WorkflowStage,
    context: FailureContext,
) -> Result<(), WorkflowFailure> {
    if required <= allowed {
        Ok(())
    } else {
        Err(WorkflowFailure::new(
            FailureCode::ResourceLimit,
            stage,
            Certainty::PrePublication,
            context,
            format_args!("workflow aggregate working bytes require {required}, limit {allowed}"),
            RecoveryAction::RaiseLimitOrNarrow,
        ))
    }
}

fn collect_bounded<T>(
    values: impl IntoIterator<Item = T>,
    maximum: usize,
    limit: &'static str,
) -> Result<Vec<T>, WorkflowFailure> {
    let mut values = values.into_iter();
    let lower = values.size_hint().0;
    if lower > maximum {
        return Err(WorkflowFailure::new(
            FailureCode::ResourceLimit,
            WorkflowStage::Validate,
            Certainty::PrePublication,
            FailureContext::default(),
            format_args!("{limit} requires at least {lower}, limit {maximum}"),
            RecoveryAction::RaiseLimitOrNarrow,
        ));
    }
    let mut collected = Vec::new();
    collected.try_reserve_exact(maximum).map_err(|_| {
        WorkflowFailure::new(
            FailureCode::ResourceLimit,
            WorkflowStage::Validate,
            Certainty::PrePublication,
            FailureContext::default(),
            format_args!("failed to reserve bounded {limit}"),
            RecoveryAction::RaiseLimitOrNarrow,
        )
    })?;
    for value in values.by_ref().take(maximum) {
        collected.push(value);
    }
    if values.next().is_some() {
        return Err(WorkflowFailure::new(
            FailureCode::ResourceLimit,
            WorkflowStage::Validate,
            Certainty::PrePublication,
            FailureContext::default(),
            format_args!("{limit} exceeds {maximum}"),
            RecoveryAction::RaiseLimitOrNarrow,
        ));
    }
    Ok(collected)
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod workflow_test_support;

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use point_contracts::AttributeId;
    use point_workspace::{WorkspaceSchema, create};

    use super::workflow_test_support::{TestDirectory, write_las_family_fixture};

    #[test]
    fn index_io_failure_preserves_the_recoverable_workflow_taxonomy() {
        let error = IndexError::Io {
            operation: "write and flush",
            path: PathBuf::from("fixture.pidx.work"),
            source: io::Error::new(io::ErrorKind::StorageFull, "disk is full"),
        };
        assert_eq!(
            error.to_string(),
            "failed to write and flush fixture.pidx.work: disk is full"
        );
        let failure = index_failure(
            WorkflowStage::Index,
            error,
            &OperationControl::new(),
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_IO");
        assert_eq!(failure.stage(), "index");
        assert_eq!(failure.certainty(), "indeterminate");
        assert_eq!(failure.publication_phase(), Some("index-target"));
        assert_eq!(
            failure.recovery_action(),
            "restore disk capacity or permissions, then resume the same Run"
        );
        assert_eq!(
            failure.diagnostic(),
            "failed to write and flush fixture.pidx.work: disk is full"
        );
    }

    #[test]
    fn index_io_uncertainty_wins_over_concurrent_ambient_cancellation() {
        let control = OperationControl::new();
        control.cancel();
        let failure = index_failure(
            WorkflowStage::Index,
            IndexError::Io {
                operation: "sync published target",
                path: PathBuf::from("fixture.pidx"),
                source: io::Error::other("injected terminal I/O"),
            },
            &control,
            FailureContext::default(),
        );
        assert_eq!(failure.code(), "PWF_IO");
        assert_eq!(failure.certainty(), "indeterminate");
        assert_eq!(failure.publication_phase(), Some("index-target"));
    }

    #[test]
    fn terrain_io_wins_over_concurrent_ambient_cancellation() {
        let control = OperationControl::new();
        control.cancel();
        let failure = terrain_failure(
            WorkflowStage::Terrain,
            point_terrain::TerrainError::Io {
                operation: "read terrain input",
                path: point_terrain::TerrainDiagnostic::new("terrain-input.bin"),
                source: io::Error::other("injected terrain I/O"),
            },
            &control,
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_IO");
        assert_eq!(failure.stage(), "terrain-derivation");
        assert_eq!(failure.certainty(), "pre_publication");
        assert!(failure.diagnostic().contains("terrain-input.bin"));
        assert!(failure.diagnostic().contains("injected terrain I/O"));
    }

    #[test]
    fn workspace_io_wins_over_concurrent_ambient_cancellation() {
        let control = OperationControl::new();
        control.cancel();
        let workspace_io = WorkspaceError::Io {
            operation: "read Workspace rows",
            path: point_workspace::WorkspaceDiagnostic::new("rows.bin"),
            source: io::Error::other("injected Workspace I/O"),
        };
        let failure = workspace_failure(
            WorkflowStage::Selection,
            workspace_io,
            &control,
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_IO");
        assert_eq!(failure.stage(), "exact-selection");
        assert_eq!(failure.certainty(), "pre_publication");
        assert!(failure.diagnostic().contains("rows.bin"));
        assert!(failure.diagnostic().contains("injected Workspace I/O"));

        let nested = terrain_failure(
            WorkflowStage::Terrain,
            point_terrain::TerrainError::from(WorkspaceError::Io {
                operation: "read terrain Workspace rows",
                path: point_workspace::WorkspaceDiagnostic::new("terrain-rows.bin"),
                source: io::Error::other("injected nested Workspace I/O"),
            }),
            &control,
            FailureContext::default(),
        );
        assert_eq!(nested.code(), "PWF_IO");
        assert!(nested.diagnostic().contains("terrain-rows.bin"));
        assert!(
            nested
                .diagnostic()
                .contains("injected nested Workspace I/O")
        );
    }

    #[test]
    fn run_root_witness_detects_same_path_replacement() {
        let directory = TestDirectory::new("run-root-witness").expect("create test directory");
        let run_root = directory.path().join("run-root");
        let moved_root = directory.path().join("moved-run-root");
        fs::create_dir(&run_root).expect("create witnessed Run root");
        let witness = DirectoryWitness::capture(&run_root).expect("capture Run root identity");

        fs::rename(&run_root, &moved_root).expect("move witnessed Run root");
        fs::create_dir(&run_root).expect("install same-path replacement");

        assert!(witness.verify().is_err());

        fs::remove_dir(&run_root).expect("remove replacement Run root");
        fs::rename(&moved_root, &run_root).expect("restore witnessed Run root");
    }

    #[cfg(unix)]
    #[test]
    fn run_lock_detects_same_path_replacement_while_held() {
        let directory = TestDirectory::new("run-lock-witness").expect("create test directory");
        let path = directory.path().join("run.lock");
        let moved = directory.path().join("moved.lock");
        let lock = RunLock::acquire(&path).expect("acquire original Run lock");

        fs::rename(&path, &moved).expect("move locked path");
        File::create(&path).expect("install replacement lock file");
        let replacement = RunLock::acquire(&path).expect("replacement path has a distinct lock");

        assert!(lock.verify().is_err());

        drop(replacement);
        fs::remove_file(&path).expect("remove replacement lock");
        fs::rename(&moved, &path).expect("restore original lock path");
        assert!(lock.verify().is_ok());
    }

    #[test]
    fn durable_context_preserves_valid_identities_when_operation_is_invalid() {
        let run = WorkflowRunId::new([1; 16]).expect("nonzero Run identity");
        let mut intent = DurableIntent::new(
            run,
            [2; 32],
            [3; 16],
            [4; 32],
            [5; 16],
            vec![7].into_boxed_slice(),
            2,
            1,
            None,
            Vec::new().into_boxed_slice(),
            "Ground".into(),
            "2026-08-12".into(),
            "00:00:00Z".into(),
            true,
            [[6; 32], [7; 32], [8; 32], [9; 32]],
            JournalLimits::default(),
        )
        .expect("create valid durable Intent");
        intent.operation = [0; 16];

        let context = durable_context(run, &intent);

        assert_eq!(context.run, Some(run));
        assert_eq!(
            context.source,
            Some(point_contracts::SourceId::new([2; 32]))
        );
        assert_eq!(
            context.workspace,
            Some(point_workspace::WorkspaceId::from_bytes([3; 16]).unwrap())
        );
        assert_eq!(context.operation, None);
        assert_eq!(
            context.revision,
            Some(RevisionId::from_bytes([4; 32]).unwrap())
        );
    }

    #[test]
    fn canonical_limit_facts_name_the_path_binding_ceiling() {
        let facts = limit_facts(&WorkflowLimits::default(), &OperationControl::new())
            .expect("construct canonical Workflow Limit Facts");

        assert_eq!(facts.len(), LIMIT_FACT_COUNT);
        assert!(facts.iter().any(|fact| {
            fact.name == "journal.max_path_binding_bytes" && fact.value == PATH_BINDING_BYTES
        }));
    }

    #[test]
    fn resumed_run_entry_bytes_include_the_open_journal() {
        let directory = TestDirectory::new("resume-entry-bytes").expect("create test directory");
        let run = WorkflowRunId::new([1; 16]).expect("nonzero Run identity");
        let operation = test_operation(2);
        let baseline = RevisionId::from_bytes([3; 32]).expect("nonzero Revision identity");
        let request = WorkflowRunIntent::new(
            run,
            operation,
            baseline,
            [7],
            1,
            TerrainRecipe::new(2),
            [],
            LandXmlOptions::metric_metres("Ground", "2026-08-12", "00:00:00Z")
                .expect("valid deterministic LandXML options")
                .assert_coordinates_are_metric_metres(),
        )
        .expect("create Workflow intent");
        let durable = DurableIntent::new(
            run,
            [4; 32],
            [5; 16],
            baseline.into_bytes(),
            operation.into_bytes(),
            vec![7].into_boxed_slice(),
            2,
            1,
            None,
            Vec::new().into_boxed_slice(),
            "Ground".into(),
            "2026-08-12".into(),
            "00:00:00Z".into(),
            true,
            [[6; 32], [7; 32], [8; 32], [9; 32]],
            JournalLimits::default(),
        )
        .expect("create durable Workflow intent");
        let journal = Journal::create(
            &directory.path().join("run.pwf"),
            durable,
            JournalLimits::default(),
        )
        .expect("create Run journal");

        assert_eq!(
            run_entry_retained_bytes(&request, Some(&journal)),
            request_retained_bytes(&request).saturating_add(journal.retained_bytes())
        );
        assert!(journal.retained_bytes() > 0);
    }

    #[test]
    fn workflow_intent_requires_metric_coordinates_before_run_creation() {
        let failure = WorkflowRunIntent::new(
            WorkflowRunId::new([1; 16]).expect("nonzero Run identity"),
            test_operation(2),
            RevisionId::from_bytes([3; 32]).expect("nonzero Revision identity"),
            [4],
            1,
            point_terrain::TerrainRecipe::new(2),
            [],
            point_terrain::LandXmlOptions::metric_metres("Ground", "2026-08-12", "00:00:00Z")
                .expect("valid deterministic LandXML options"),
        )
        .expect_err("an unasserted metric request must fail before Run creation");

        assert_eq!(failure.code(), "PWF_INVALID_REQUEST");
        assert_eq!(failure.stage(), "validate");
        assert_eq!(failure.certainty(), "pre_publication");
        assert!(failure.to_string().contains("metric-metre"));
    }

    #[test]
    fn post_complete_binding_failure_preserves_indeterminate_certainty() {
        let failure = checkpoint_binding_failure(
            WorkflowStage::Complete,
            io::Error::new(io::ErrorKind::InvalidData, "Run binding changed"),
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_PUBLICATION_INDETERMINATE");
        assert_eq!(failure.stage(), "complete-checkpoint");
        assert_eq!(failure.certainty(), "indeterminate");
        assert_eq!(failure.publication_phase(), Some("complete-checkpoint"));
        assert_eq!(
            failure.recovery_action(),
            "resume the same Run with the same identities and paths"
        );
    }

    #[test]
    fn post_link_landxml_failure_remains_indeterminate_after_parent_cancellation() {
        let control = OperationControl::new();
        control.cancel();

        let failure = terrain_output_failure(
            WorkflowStage::LandXml,
            point_terrain::TerrainError::ExportIndeterminate {
                expected_hash: point_contracts::ContentHash::new([1; 32]),
                source: Box::new(point_terrain::TerrainError::Cancelled),
            },
            &control,
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_PUBLICATION_INDETERMINATE");
        assert_eq!(failure.certainty(), "indeterminate");
        assert_eq!(failure.publication_phase(), Some("landxml-target"));
    }

    #[test]
    fn landxml_io_before_publication_preserves_prepublication_taxonomy() {
        let failure = terrain_output_failure(
            WorkflowStage::LandXml,
            point_terrain::TerrainError::Io {
                operation: "create LandXML stage",
                path: point_terrain::TerrainDiagnostic::new(".punctra-landxml.stage"),
                source: io::Error::other("disk is full"),
            },
            &OperationControl::new(),
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_IO");
        assert_eq!(failure.certainty(), "pre_publication");
        assert_eq!(failure.publication_phase(), None);
        assert_eq!(
            failure.recovery_action(),
            "restore disk capacity or permissions, then resume the same Run"
        );
    }

    #[test]
    fn landxml_target_replacement_is_retryable_io_not_output_conflict() {
        let failure = terrain_output_failure(
            WorkflowStage::LandXml,
            point_terrain::TerrainError::TargetChanged {
                path: point_terrain::TerrainDiagnostic::new("terrain.xml"),
            },
            &OperationControl::new(),
            FailureContext::default(),
        );

        assert_eq!(failure.code(), "PWF_IO");
        assert_eq!(failure.certainty(), "pre_publication");
        assert_eq!(
            failure.recovery_action(),
            "resume the same Run with the same identities and paths"
        );
    }

    #[test]
    fn report_operational_failures_keep_their_stable_taxonomy() {
        let context = FailureContext::default();
        let invalid = report_failure(
            WorkflowStage::Report,
            CanonicalOutputError::Invalid("invalid request".to_owned()),
            context,
        );
        assert_eq!(invalid.code(), "PWF_INVALID_REQUEST");
        assert_eq!(invalid.certainty(), "pre_publication");

        let io = report_failure(
            WorkflowStage::Report,
            CanonicalOutputError::Io {
                operation: "sync report".to_owned(),
                path: PathBuf::from("audit.json"),
                source: io::Error::other("disk unavailable"),
            },
            context,
        );
        assert_eq!(io.code(), "PWF_IO");
        assert_eq!(io.certainty(), "pre_publication");
        assert_eq!(
            io.recovery_action(),
            "restore disk capacity or permissions, then resume the same Run"
        );

        let indeterminate = report_failure(
            WorkflowStage::Report,
            CanonicalOutputError::Indeterminate {
                path: PathBuf::from("audit.json"),
                expected_hash: [1; 32],
                source: io::Error::other(CanonicalOutputError::Io {
                    operation: "sync created report target".to_owned(),
                    path: PathBuf::from("audit.json"),
                    source: io::Error::other("disk unavailable after publication"),
                }),
            },
            context,
        );
        assert_eq!(indeterminate.code(), "PWF_PUBLICATION_INDETERMINATE");
        assert_eq!(indeterminate.certainty(), "indeterminate");
        assert!(
            indeterminate
                .diagnostic()
                .contains("sync created report target")
        );
        assert!(
            indeterminate
                .diagnostic()
                .contains("disk unavailable after publication")
        );
    }

    #[test]
    fn journal_operational_failures_keep_their_stable_taxonomy() {
        let context = FailureContext::default();
        for (error, code, action) in [
            (
                JournalError::Invalid("bad request"),
                "PWF_INVALID_REQUEST",
                "correct the invalid request and start a new Run",
            ),
            (
                JournalError::Locked,
                "PWF_IO",
                "resume the same Run with the same identities and paths",
            ),
            (
                JournalError::Entropy,
                "PWF_IO",
                "restore disk capacity or permissions, then resume the same Run",
            ),
            (
                JournalError::Exists(PathBuf::from("run.pwf")),
                "PWF_JOURNAL_CONFLICT",
                "stop and preserve all Run and Workspace files",
            ),
            (
                JournalError::Corrupt("bad frame"),
                "PWF_JOURNAL_CORRUPT",
                "stop and preserve all Run and Workspace files",
            ),
        ] {
            let failure = journal_failure(WorkflowStage::Intent, error, context);
            assert_eq!(failure.code(), code);
            assert_eq!(failure.recovery_action(), action);
        }
        let io = journal_failure(
            WorkflowStage::Intent,
            JournalError::Io {
                operation: "sync intent",
                path: PathBuf::from("run.pwf"),
                source: io::Error::other("disk unavailable"),
            },
            context,
        );
        assert_eq!(io.code(), "PWF_IO");
        assert_eq!(io.certainty(), "pre_publication");
    }

    #[test]
    fn revert_restores_an_empty_baseline_surface_change_envelope() {
        let directory = TestDirectory::new("revert-envelope").expect("create test directory");
        let source_path = directory.path().join("fixture.las");
        let index_path = directory.path().join("fixture.pidx");
        let workspace_path = directory.path().join("fixture.pcw");
        write_las_family_fixture(&source_path, 64).expect("write generated LAS Source");

        let source = source_las::open(&source_path)
            .blocking_wait()
            .expect("open generated Source");
        let index = point_index::prepare(source, &index_path, PrepareLimits::default())
            .blocking_wait()
            .expect("prepare generated Source index");
        let workspace = create(
            &workspace_path,
            index,
            WorkspaceSchema::new(
                AttributeId::new(LAS_CLASSIFICATION_ATTRIBUTE)
                    .expect("LAS classification Attribute ID is nonzero"),
            ),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create baseline Workspace");
        let baseline = workspace.head();
        let baseline_surface = point_terrain::derive(
            baseline.clone(),
            TerrainRecipe::new(2),
            TerrainLimits::default(),
        )
        .blocking_wait()
        .expect("derive baseline Surface");

        let points = baseline
            .select_point_ids(
                [PointId::new(workspace.source(), 9)],
                PointSetLimits::default(),
            )
            .blocking_wait()
            .expect("materialize one correction Point");
        let edit = committed_revision(
            workspace
                .commit(
                    CommitRequest::set_classification(test_operation(201), points, 1),
                    CommitLimits::default(),
                )
                .blocking_wait()
                .expect("commit test classification Edit"),
        );
        let changed_surface = point_terrain::derive(
            workspace.snapshot(edit).expect("open changed Snapshot"),
            TerrainRecipe::new(2),
            TerrainLimits::default(),
        )
        .blocking_wait()
        .expect("derive changed Surface");
        let limits = WorkflowLimits::default();
        let changed_envelope = change_envelope(
            &baseline_surface,
            &changed_surface,
            limits.max_envelope_faces,
            limits.max_envelope_working_bytes,
            &OperationControl::new(),
        )
        .expect("compare baseline and changed Surfaces");
        assert!(
            changed_envelope.added_face_count > 0 || changed_envelope.removed_face_count > 0,
            "the fixture Edit must change Terrain topology"
        );

        let reverted = committed_revision(
            workspace
                .commit(
                    CommitRequest::revert_head(test_operation(202), edit),
                    CommitLimits::default(),
                )
                .blocking_wait()
                .expect("commit test Revert"),
        );
        let restored_surface = point_terrain::derive(
            workspace.snapshot(reverted).expect("open Revert Snapshot"),
            TerrainRecipe::new(2),
            TerrainLimits::default(),
        )
        .blocking_wait()
        .expect("derive restored Surface");
        let restored_envelope = change_envelope(
            &baseline_surface,
            &restored_surface,
            limits.max_envelope_faces,
            limits.max_envelope_working_bytes,
            &OperationControl::new(),
        )
        .expect("compare baseline and restored Surfaces");

        assert_eq!(restored_envelope.added_face_count, 0);
        assert_eq!(restored_envelope.removed_face_count, 0);
        assert_eq!(restored_envelope.bounds_bits, None);
    }

    fn committed_revision(outcome: CommitOutcome) -> RevisionId {
        match outcome {
            CommitOutcome::Committed(receipt) => receipt.revision(),
            other => panic!("test commit was not definitive: {other:?}"),
        }
    }

    fn test_operation(value: u8) -> OperationId {
        OperationId::from_bytes([value; 16]).expect("test Operation ID is nonzero")
    }
}
