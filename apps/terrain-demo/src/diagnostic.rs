use std::fmt::{self, Write as _};

use foundation_runtime::RuntimeError;

use crate::journal::RunId;

const MAX_DIAGNOSTIC_BYTES: usize = 1_024;
const ELLIPSIS: &str = "...";

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
    WorkspaceOperation,
    WorkspaceRevision,
    WorkspaceDirectorySync,
    JournalCheckpoint,
    LandXmlTarget,
    ReportTarget,
    CompleteCheckpoint,
}

impl PublicationPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::JournalIntent => "journal-intent",
            Self::WorkspaceOperation => "workspace-operation",
            Self::WorkspaceRevision => "workspace-revision",
            Self::WorkspaceDirectorySync => "workspace-directory-sync",
            Self::JournalCheckpoint => "journal-checkpoint",
            Self::LandXmlTarget => "landxml-target",
            Self::ReportTarget => "report-target",
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
    pub(crate) run: Option<RunId>,
    pub(crate) source: Option<[u8; 32]>,
    pub(crate) workspace: Option<[u8; 16]>,
    pub(crate) operation: Option<[u8; 16]>,
    pub(crate) revision: Option<[u8; 32]>,
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
            Self::UseSupportedRoundTripSize => {
                "use inputs within the named comparison limit or preserve them for a later slice"
            }
            Self::ReviewReturnedLandXml => {
                "review the downstream export settings or reject the returned deliverable"
            }
            Self::StopAndPreserve => "stop and preserve all Run and Workspace files",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundedDiagnostic(Box<str>);

impl BoundedDiagnostic {
    fn new(message: impl fmt::Display) -> Self {
        let mut output = CappedFormatter::new();
        let _ = write!(&mut output, "{message}");
        Self(output.text.into_boxed_str())
    }
}

struct CappedFormatter {
    text: String,
    truncated: bool,
}

impl CappedFormatter {
    fn new() -> Self {
        let mut text = String::new();
        let _ = text.try_reserve_exact(MAX_DIAGNOSTIC_BYTES);
        Self {
            text,
            truncated: false,
        }
    }
}

impl fmt::Write for CappedFormatter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let remaining = MAX_DIAGNOSTIC_BYTES.saturating_sub(self.text.len());
        if value.len() <= remaining {
            self.text.push_str(value);
            return Ok(());
        }
        let mut end = remaining;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&value[..end]);
        let target = MAX_DIAGNOSTIC_BYTES - ELLIPSIS.len();
        while self.text.len() > target {
            self.text.pop();
        }
        self.text.push_str(ELLIPSIS);
        self.truncated = true;
        Ok(())
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
        Self::new(
            FailureCode::InvalidRequest,
            stage,
            Certainty::PrePublication,
            FailureContext::default(),
            error,
            RecoveryAction::CorrectInvalidRequest,
        )
    }

    pub(crate) fn diagnostic(&self) -> &str {
        &self.diagnostic.0
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
    pub fn run(&self) -> Option<[u8; 16]> {
        self.context.run.map(RunId::into_bytes)
    }

    /// Returns the Source identity when known.
    #[must_use]
    pub const fn source(&self) -> Option<[u8; 32]> {
        self.context.source
    }

    /// Returns the Workspace identity when known.
    #[must_use]
    pub const fn workspace(&self) -> Option<[u8; 16]> {
        self.context.workspace
    }

    /// Returns the Operation identity when known.
    #[must_use]
    pub const fn operation(&self) -> Option<[u8; 16]> {
        self.context.operation
    }

    /// Returns the Revision identity when known.
    #[must_use]
    pub const fn revision(&self) -> Option<[u8; 32]> {
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
            write!(formatter, " run={}", Hex(&run.into_bytes()))?;
        }
        if let Some(source) = self.context.source {
            write!(formatter, " source={}", Hex(&source))?;
        }
        if let Some(workspace) = self.context.workspace {
            write!(formatter, " workspace={}", Hex(&workspace))?;
        }
        if let Some(operation) = self.context.operation {
            write!(formatter, " operation={}", Hex(&operation))?;
        }
        if let Some(revision) = self.context.revision {
            write!(formatter, " revision={}", Hex(&revision))?;
        }
        write!(
            formatter,
            ": {}\nrecovery: {}",
            self.diagnostic,
            self.action.as_str()
        )
    }
}

impl fmt::Display for BoundedDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
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

struct Hex<'a>(&'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
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
                run: Some(RunId::new([1; 16]).unwrap()),
                operation: Some([2; 16]),
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
}
