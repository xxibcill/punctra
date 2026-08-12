use std::fmt;

use foundation_runtime::RuntimeError;
use point_contracts::SourceId;
use point_workspace::{OperationId, RevisionId, WorkspaceId};

#[cfg(test)]
use crate::bounded_diagnostic::MAX_DIAGNOSTIC_BYTES;
use crate::{bounded_diagnostic::BoundedDiagnostic, journal::WorkflowRunId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureCode {
    InvalidRequest,
    ResourceLimit,
    Cancelled,
    SourceMismatch,
    WorkspaceMismatch,
    StaleBaseline,
    OperationRejected,
    OperationIndeterminate,
    JournalConflict,
    JournalCorrupt,
    OutputConflict,
    RoundTripInvalidInput,
    RoundTripResourceLimit,
    RoundTripSemanticMismatch,
    RoundTripXmlInvalid,
    RoundTripSubsetUnsupported,
    RoundTripCoordinateReferenceUnsupported,
    RoundTripUnitDrift,
    RoundTripPointCountDrift,
    RoundTripVertexUnmatched,
    RoundTripVertexAmbiguous,
    RoundTripToleranceDrift,
    RoundTripTopologyDrift,
    PublicationIndeterminate,
    Io,
    Internal,
}

impl FailureCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "PWF_INVALID_REQUEST",
            Self::ResourceLimit => "PWF_RESOURCE_LIMIT",
            Self::Cancelled => "PWF_CANCELLED",
            Self::SourceMismatch => "PWF_SOURCE_MISMATCH",
            Self::WorkspaceMismatch => "PWF_WORKSPACE_MISMATCH",
            Self::StaleBaseline => "PWF_STALE_BASELINE",
            Self::OperationRejected => "PWF_OPERATION_REJECTED",
            Self::OperationIndeterminate => "PWF_OPERATION_INDETERMINATE",
            Self::JournalConflict => "PWF_JOURNAL_CONFLICT",
            Self::JournalCorrupt => "PWF_JOURNAL_CORRUPT",
            Self::OutputConflict => "PWF_OUTPUT_CONFLICT",
            Self::RoundTripInvalidInput => "PRT_INVALID_INPUT",
            Self::RoundTripResourceLimit => "PRT_RESOURCE_LIMIT",
            Self::RoundTripSemanticMismatch => "PRT_SEMANTIC_MISMATCH",
            Self::RoundTripXmlInvalid => "PRT_XML_INVALID",
            Self::RoundTripSubsetUnsupported => "PRT_SUBSET_UNSUPPORTED",
            Self::RoundTripCoordinateReferenceUnsupported => "PRT_COORDINATE_REFERENCE_UNSUPPORTED",
            Self::RoundTripUnitDrift => "PRT_UNIT_DRIFT",
            Self::RoundTripPointCountDrift => "PRT_POINT_COUNT_DRIFT",
            Self::RoundTripVertexUnmatched => "PRT_VERTEX_UNMATCHED",
            Self::RoundTripVertexAmbiguous => "PRT_VERTEX_AMBIGUOUS",
            Self::RoundTripToleranceDrift => "PRT_TOLERANCE_DRIFT",
            Self::RoundTripTopologyDrift => "PRT_TOPOLOGY_DRIFT",
            Self::PublicationIndeterminate => "PWF_PUBLICATION_INDETERMINATE",
            Self::Io => "PWF_IO",
            Self::Internal => "PWF_INTERNAL",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkflowStage {
    Validate,
    Lock,
    Source,
    Index,
    Workspace,
    Intent,
    ResolveOperation,
    Selection,
    Commit,
    RevisionAudit,
    Terrain,
    ChangeEnvelope,
    CheckPointQa,
    LandXml,
    Report,
    Complete,
    Inspect,
    RoundTrip,
}

impl WorkflowStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Lock => "lock",
            Self::Source => "source",
            Self::Index => "index",
            Self::Workspace => "workspace",
            Self::Intent => "intent-publication",
            Self::ResolveOperation => "operation-resolution",
            Self::Selection => "exact-selection",
            Self::Commit => "commit",
            Self::RevisionAudit => "revision-audit",
            Self::Terrain => "terrain-derivation",
            Self::ChangeEnvelope => "surface-change-envelope",
            Self::CheckPointQa => "check-point-qa",
            Self::LandXml => "landxml-ensure",
            Self::Report => "report-ensure",
            Self::Complete => "complete-checkpoint",
            Self::Inspect => "inspect",
            Self::RoundTrip => "landxml-round-trip-comparison",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublicationPhase {
    JournalIntent,
    IndexTarget,
    WorkspaceOperation,
    WorkspaceRevision,
    WorkspaceDirectorySync,
    JournalCheckpoint,
    LandXmlTarget,
    ReportTarget,
    RoundTripEvidenceTarget,
    CompleteCheckpoint,
}

impl PublicationPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::JournalIntent => "journal-intent",
            Self::IndexTarget => "index-target",
            Self::WorkspaceOperation => "workspace-operation",
            Self::WorkspaceRevision => "workspace-revision",
            Self::WorkspaceDirectorySync => "workspace-directory-sync",
            Self::JournalCheckpoint => "journal-checkpoint",
            Self::LandXmlTarget => "landxml-target",
            Self::ReportTarget => "report-target",
            Self::RoundTripEvidenceTarget => "round-trip-evidence-target",
            Self::CompleteCheckpoint => "complete-checkpoint",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Certainty {
    PrePublication,
    DurableFact,
    Indeterminate(PublicationPhase),
}

impl Certainty {
    fn write(self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrePublication => formatter.write_str("pre-publication"),
            Self::DurableFact => formatter.write_str("durable-fact"),
            Self::Indeterminate(phase) => {
                write!(formatter, "indeterminate({})", phase.as_str())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FailureContext {
    pub(crate) run: Option<WorkflowRunId>,
    pub(crate) source: Option<SourceId>,
    pub(crate) workspace: Option<WorkspaceId>,
    pub(crate) operation: Option<OperationId>,
    pub(crate) revision: Option<RevisionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryAction {
    CorrectInvalidRequest,
    RaiseLimitOrNarrow,
    ResumeSameRun,
    RetryAfterRestoringDisk,
    ResolveRecordedOperationByResuming,
    RemoveOrRenameConflictingTarget,
    RestoreExpectedSource,
    CorrectRoundTripInput,
    UseSupportedRoundTripSize,
    ReviewReturnedLandXml,
    StopAndPreserve,
}

impl RecoveryAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CorrectInvalidRequest => "correct the invalid request and start a new Run",
            Self::RaiseLimitOrNarrow => "raise the named limit or narrow the exact request",
            Self::ResumeSameRun => "resume the same Run with the same identities and paths",
            Self::RetryAfterRestoringDisk => {
                "restore disk capacity or permissions, then resume the same Run"
            }
            Self::ResolveRecordedOperationByResuming => {
                "resume to resolve the recorded Operation Identity"
            }
            Self::RemoveOrRenameConflictingTarget => {
                "remove or rename the conflicting caller-owned target, then resume"
            }
            Self::RestoreExpectedSource => "restore the expected immutable Source, then resume",
            Self::CorrectRoundTripInput => {
                "correct the declaration or LandXML input, then retry the comparison"
            }
            Self::UseSupportedRoundTripSize => "use inputs within the named round-trip limits",
            Self::ReviewReturnedLandXml => {
                "review the downstream export settings or reject the returned deliverable"
            }
            Self::StopAndPreserve => "stop and preserve all Run and Workspace files",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Bounded structured Workflow failure with exactly one safe recovery action.
pub struct WorkflowFailure {
    pub(crate) code: FailureCode,
    pub(crate) stage: WorkflowStage,
    pub(crate) certainty: Certainty,
    context: Box<FailureContext>,
    diagnostic: BoundedDiagnostic,
    pub(crate) action: RecoveryAction,
}

impl WorkflowFailure {
    pub(crate) fn new(
        code: FailureCode,
        stage: WorkflowStage,
        certainty: Certainty,
        context: FailureContext,
        error: impl fmt::Display,
        action: RecoveryAction,
    ) -> Self {
        Self {
            code,
            stage,
            certainty,
            context: Box::new(context),
            diagnostic: BoundedDiagnostic::new(error),
            action,
        }
    }

    pub(crate) fn invalid(stage: WorkflowStage, error: impl fmt::Display) -> Self {
        Self::invalid_with_context(stage, FailureContext::default(), error)
    }

    pub(crate) fn invalid_with_context(
        stage: WorkflowStage,
        context: FailureContext,
        error: impl fmt::Display,
    ) -> Self {
        Self::new(
            FailureCode::InvalidRequest,
            stage,
            Certainty::PrePublication,
            context,
            error,
            RecoveryAction::CorrectInvalidRequest,
        )
    }

    pub(crate) fn diagnostic(&self) -> &str {
        self.diagnostic.as_str()
    }

    /// Returns the stable machine-readable failure code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code.as_str()
    }

    /// Returns the stable Workflow stage name.
    #[must_use]
    pub const fn stage(&self) -> &'static str {
        self.stage.as_str()
    }

    /// Returns the stable certainty category.
    #[must_use]
    pub const fn certainty(&self) -> &'static str {
        match self.certainty {
            Certainty::PrePublication => "pre_publication",
            Certainty::DurableFact => "durable_fact",
            Certainty::Indeterminate(_) => "indeterminate",
        }
    }

    /// Returns the publication phase when certainty is indeterminate.
    #[must_use]
    pub const fn publication_phase(&self) -> Option<&'static str> {
        match self.certainty {
            Certainty::Indeterminate(phase) => Some(phase.as_str()),
            Certainty::PrePublication | Certainty::DurableFact => None,
        }
    }

    /// Returns the one stable recovery action.
    #[must_use]
    pub const fn recovery_action(&self) -> &'static str {
        self.action.as_str()
    }

    /// Returns the Run identity when known.
    #[must_use]
    pub fn run(&self) -> Option<WorkflowRunId> {
        self.context.run
    }

    /// Returns the Source identity when known.
    #[must_use]
    pub const fn source(&self) -> Option<SourceId> {
        self.context.source
    }

    /// Returns the Workspace identity when known.
    #[must_use]
    pub const fn workspace(&self) -> Option<WorkspaceId> {
        self.context.workspace
    }

    /// Returns the Operation identity when known.
    #[must_use]
    pub const fn operation(&self) -> Option<OperationId> {
        self.context.operation
    }

    /// Returns the Revision identity when known.
    #[must_use]
    pub const fn revision(&self) -> Option<RevisionId> {
        self.context.revision
    }
}

impl fmt::Display for WorkflowFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {} [certainty=",
            self.code.as_str(),
            self.stage.as_str()
        )?;
        self.certainty.write(formatter)?;
        write!(formatter, "]")?;
        if let Some(run) = self.context.run {
            write!(formatter, " run={run}")?;
        }
        if let Some(source) = self.context.source {
            write!(formatter, " source={source}")?;
        }
        if let Some(workspace) = self.context.workspace {
            write!(formatter, " workspace={workspace}")?;
        }
        if let Some(operation) = self.context.operation {
            write!(formatter, " operation={operation}")?;
        }
        if let Some(revision) = self.context.revision {
            write!(formatter, " revision={revision}")?;
        }
        write!(
            formatter,
            ": {}\nrecovery: {}",
            self.diagnostic,
            self.action.as_str()
        )
    }
}

impl std::error::Error for WorkflowFailure {}

impl From<RuntimeError> for WorkflowFailure {
    fn from(error: RuntimeError) -> Self {
        let code = if error == RuntimeError::Cancelled {
            FailureCode::Cancelled
        } else {
            FailureCode::Internal
        };
        Self::new(
            code,
            WorkflowStage::Validate,
            Certainty::PrePublication,
            FailureContext::default(),
            error,
            RecoveryAction::ResumeSameRun,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_is_bounded_structured_and_has_one_action() {
        let failure = WorkflowFailure::new(
            FailureCode::OperationIndeterminate,
            WorkflowStage::Commit,
            Certainty::Indeterminate(PublicationPhase::WorkspaceRevision),
            FailureContext {
                run: Some(WorkflowRunId::new([1; 16]).unwrap()),
                operation: Some(OperationId::from_bytes([2; 16]).unwrap()),
                ..FailureContext::default()
            },
            "ก".repeat(MAX_DIAGNOSTIC_BYTES),
            RecoveryAction::ResolveRecordedOperationByResuming,
        );
        let text = failure.to_string();
        assert!(text.len() < MAX_DIAGNOSTIC_BYTES + 512);
        assert!(text.contains("PWF_OPERATION_INDETERMINATE"));
        assert!(text.contains("workspace-revision"));
        assert!(text.contains("operation=020202"));
        assert!(text.contains("resolve the recorded Operation"));
    }

    #[test]
    fn invalid_failure_can_preserve_known_identities() {
        let context = FailureContext {
            run: Some(WorkflowRunId::new([1; 16]).unwrap()),
            source: Some(SourceId::new([2; 32])),
            workspace: Some(WorkspaceId::from_bytes([3; 16]).unwrap()),
            operation: Some(OperationId::from_bytes([4; 16]).unwrap()),
            revision: Some(RevisionId::from_bytes([5; 32]).unwrap()),
        };

        let failure = WorkflowFailure::invalid_with_context(
            WorkflowStage::Selection,
            context,
            "invalid selection",
        );

        assert_eq!(failure.run(), context.run);
        assert_eq!(failure.source(), context.source);
        assert_eq!(failure.workspace(), context.workspace);
        assert_eq!(failure.operation(), context.operation);
        assert_eq!(failure.revision(), context.revision);
    }
}
