use std::{error::Error, fmt, io, path::Path};

use foundation_runtime::RuntimeError;
use point_index::IndexError;
use point_source::SourceError;

const MAX_MESSAGE_BYTES: usize = 1_024;
const ELLIPSIS: &str = "…";
const ALLOCATION_FAILURE_MESSAGE: &str =
    "diagnostic unavailable because its bounded allocation failed";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewFailureCode {
    InvalidRequest,
    Source,
    Index,
    ResourceLimit,
    Cancelled,
    Gpu,
    Io,
    Internal,
}

impl fmt::Display for ViewFailureCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ViewFailureCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "PVIEW_INVALID_REQUEST",
            Self::Source => "PVIEW_SOURCE",
            Self::Index => "PVIEW_INDEX",
            Self::ResourceLimit => "PVIEW_RESOURCE_LIMIT",
            Self::Cancelled => "PVIEW_CANCELLED",
            Self::Gpu => "PVIEW_GPU",
            Self::Io => "PVIEW_IO",
            Self::Internal => "PVIEW_INTERNAL",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewPhase {
    RequestValidation,
    SourceVerification,
    IndexPrepare,
    Hierarchy,
    Planning,
    NodeRead,
    HostStaging,
    GpuSetup,
    GpuUpload,
    Rendering,
    ReportPublication,
}

impl fmt::Display for ViewPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ViewPhase {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::RequestValidation => "request-validation",
            Self::SourceVerification => "source-verification",
            Self::IndexPrepare => "index-prepare",
            Self::Hierarchy => "hierarchy",
            Self::Planning => "planning",
            Self::NodeRead => "node-read",
            Self::HostStaging => "host-staging",
            Self::GpuSetup => "gpu-setup",
            Self::GpuUpload => "gpu-upload",
            Self::Rendering => "rendering",
            Self::ReportPublication => "report-publication",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryAction {
    CorrectRequest,
    CheckSource,
    RebuildIndexExplicitly,
    RetrySameIndex,
    RaiseNamedLimit,
    Retry,
    UseSmokeMode,
    ConfigureCorpusGpu,
    ChooseFreshCorpusTargets,
    RetryCorpusWithFreshTargets,
    RestoreDisk,
    ReportBug,
}

impl fmt::Display for RecoveryAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl RecoveryAction {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CorrectRequest => "correct the request and retry",
            Self::CheckSource => {
                "check that the Source exists, is stable, and is supported, then retry"
            }
            Self::RebuildIndexExplicitly => {
                "move aside the rebuildable index family or choose a new index target, then retry"
            }
            Self::RetrySameIndex => {
                "retry the same index target after the active preparation finishes"
            }
            Self::RaiseNamedLimit => {
                "raise the named hard limit or use a smaller Source, then retry"
            }
            Self::Retry => "retry the same operation",
            Self::UseSmokeMode => {
                "run with --smoke to isolate the GPU path, or select a supported GPU"
            }
            Self::ConfigureCorpusGpu => {
                "select or configure a supported GPU, then rerun the corpus command"
            }
            Self::ChooseFreshCorpusTargets => {
                "preserve the existing report, choose a fresh report path and fresh index target for every corpus entry, then rerun"
            }
            Self::RetryCorpusWithFreshTargets => {
                "correct the reported condition, preserve existing corpus outputs, choose a fresh report path and fresh index target for every corpus entry, then rerun"
            }
            Self::RestoreDisk => "restore disk capacity and permissions, then retry the same path",
            Self::ReportBug => "preserve the diagnostic and report a reproducible bug",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ViewFailure {
    code: ViewFailureCode,
    phase: ViewPhase,
    message: DiagnosticMessage,
    action: RecoveryAction,
}

#[derive(Debug)]
enum DiagnosticMessage {
    Owned(String),
    Static(&'static str),
}

impl PartialEq for DiagnosticMessage {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for DiagnosticMessage {}

impl DiagnosticMessage {
    fn as_str(&self) -> &str {
        match self {
            Self::Owned(message) => message,
            Self::Static(message) => message,
        }
    }
}

impl fmt::Display for DiagnosticMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ViewFailure {
    pub(crate) fn new(
        code: ViewFailureCode,
        phase: ViewPhase,
        message: impl fmt::Display,
        action: RecoveryAction,
    ) -> Self {
        Self::formatted(code, phase, format_args!("{message}"), action)
    }

    pub(crate) fn invalid_request(message: impl fmt::Display) -> Self {
        Self::new(
            ViewFailureCode::InvalidRequest,
            ViewPhase::RequestValidation,
            message,
            RecoveryAction::CorrectRequest,
        )
    }

    pub(crate) fn source(path: &Path, error: &SourceError) -> Self {
        let (code, action) = match error {
            SourceError::ResourceLimit { .. } => (
                ViewFailureCode::ResourceLimit,
                RecoveryAction::RaiseNamedLimit,
            ),
            SourceError::Cancelled => (ViewFailureCode::Cancelled, RecoveryAction::Retry),
            _ => (ViewFailureCode::Source, RecoveryAction::CheckSource),
        };
        Self::formatted(
            code,
            ViewPhase::SourceVerification,
            format_args!(
                "Source {} failed Full verification: {error}",
                path.display()
            ),
            action,
        )
    }

    pub(crate) fn index(path: &Path, error: &IndexError) -> Self {
        let (code, action) = index_category(error);
        Self::formatted(
            code,
            ViewPhase::IndexPrepare,
            format_args!("index {} could not be prepared: {error}", path.display()),
            action,
        )
    }

    pub(crate) fn index_read(error: &IndexError) -> Self {
        let (code, action) = index_category(error);
        Self::formatted(
            code,
            ViewPhase::NodeRead,
            format_args!("index node read failed: {error}"),
            action,
        )
    }

    pub(crate) fn gpu(phase: ViewPhase, error: impl fmt::Display) -> Self {
        Self::formatted(
            ViewFailureCode::Gpu,
            phase,
            format_args!("{error}"),
            RecoveryAction::UseSmokeMode,
        )
    }

    pub(crate) fn corpus_gpu(error: impl fmt::Display) -> Self {
        Self::formatted(
            ViewFailureCode::Gpu,
            ViewPhase::GpuSetup,
            format_args!("{error}"),
            RecoveryAction::ConfigureCorpusGpu,
        )
    }

    fn formatted(
        code: ViewFailureCode,
        phase: ViewPhase,
        arguments: fmt::Arguments<'_>,
        action: RecoveryAction,
    ) -> Self {
        Self::formatted_with_storage(code, phase, arguments, action, reserved_message_storage())
    }

    fn formatted_with_storage(
        code: ViewFailureCode,
        phase: ViewPhase,
        arguments: fmt::Arguments<'_>,
        action: RecoveryAction,
        storage: Option<String>,
    ) -> Self {
        Self {
            code,
            phase,
            message: bounded_format(arguments, storage),
            action,
        }
    }

    pub(crate) fn resource(phase: ViewPhase, message: impl fmt::Display) -> Self {
        Self::new(
            ViewFailureCode::ResourceLimit,
            phase,
            message,
            RecoveryAction::RaiseNamedLimit,
        )
    }

    pub(crate) fn internal(phase: ViewPhase, message: impl fmt::Display) -> Self {
        Self::new(
            ViewFailureCode::Internal,
            phase,
            message,
            RecoveryAction::ReportBug,
        )
    }

    pub(crate) fn for_completed_corpus(self) -> Self {
        let action = Self::completed_corpus_action();
        Self { action, ..self }
    }

    pub(crate) fn reowned(&self) -> Self {
        Self::formatted(
            self.code,
            self.phase,
            format_args!("{}", self.message),
            self.action,
        )
    }

    pub(crate) const fn completed_corpus_action() -> RecoveryAction {
        RecoveryAction::RetryCorpusWithFreshTargets
    }

    pub(crate) const fn code(&self) -> ViewFailureCode {
        self.code
    }

    pub(crate) const fn phase(&self) -> ViewPhase {
        self.phase
    }

    #[cfg(test)]
    pub(crate) const fn action(&self) -> RecoveryAction {
        self.action
    }
}

fn index_category(error: &IndexError) -> (ViewFailureCode, RecoveryAction) {
    match error {
        IndexError::ResourceLimit { .. }
        | IndexError::Source(SourceError::ResourceLimit { .. }) => (
            ViewFailureCode::ResourceLimit,
            RecoveryAction::RaiseNamedLimit,
        ),
        IndexError::Io { .. } | IndexError::SharedPathIo { .. } => {
            (ViewFailureCode::Io, RecoveryAction::RestoreDisk)
        }
        IndexError::IncompatibleArtifact { .. }
        | IndexError::IncompatibleWork { .. }
        | IndexError::CorruptArtifact { .. }
        | IndexError::CorruptWork { .. }
        | IndexError::UnsupportedVersion { .. } => (
            ViewFailureCode::Index,
            RecoveryAction::RebuildIndexExplicitly,
        ),
        IndexError::PreparationInProgress { .. } => {
            (ViewFailureCode::Index, RecoveryAction::RetrySameIndex)
        }
        IndexError::Source(SourceError::Cancelled)
        | IndexError::Runtime(RuntimeError::Cancelled) => {
            (ViewFailureCode::Cancelled, RecoveryAction::Retry)
        }
        IndexError::Source(_) => (ViewFailureCode::Source, RecoveryAction::CheckSource),
        IndexError::Runtime(_) => (ViewFailureCode::Internal, RecoveryAction::ReportBug),
        _ => (ViewFailureCode::Index, RecoveryAction::ReportBug),
    }
}

impl fmt::Display for ViewFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}\n  detail: {}\n  safe action: {}",
            self.code, self.phase, self.message, self.action
        )
    }
}

impl Error for ViewFailure {}

impl From<ViewFailure> for io::Error {
    fn from(failure: ViewFailure) -> Self {
        Self::other(failure)
    }
}

fn reserved_message_storage() -> Option<String> {
    let mut message = String::new();
    message.try_reserve_exact(MAX_MESSAGE_BYTES).ok()?;
    Some(message)
}

fn bounded_format(arguments: fmt::Arguments<'_>, storage: Option<String>) -> DiagnosticMessage {
    let Some(storage) = storage else {
        return DiagnosticMessage::Static(ALLOCATION_FAILURE_MESSAGE);
    };
    let mut writer = MessageWriter::new(storage);
    let _ = fmt::write(&mut writer, arguments);
    writer.finish()
}

struct MessageWriter {
    message: String,
    truncated: bool,
}

impl MessageWriter {
    fn new(message: String) -> Self {
        debug_assert!(message.is_empty());
        debug_assert!(message.capacity() >= MAX_MESSAGE_BYTES);
        Self {
            message,
            truncated: false,
        }
    }

    fn finish(self) -> DiagnosticMessage {
        DiagnosticMessage::Owned(self.message)
    }

    fn truncate(&mut self) {
        let target = MAX_MESSAGE_BYTES - ELLIPSIS.len();
        while self.message.len() > target {
            self.message.pop();
        }
        self.message.push_str(ELLIPSIS);
        self.truncated = true;
    }
}

impl fmt::Write for MessageWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let remaining = MAX_MESSAGE_BYTES.saturating_sub(self.message.len());
        if value.len() <= remaining {
            self.message.push_str(value);
            return Ok(());
        }
        let mut end = remaining;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.message.push_str(&value[..end]);
        self.truncate();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn diagnostic_is_bounded_and_has_exactly_one_action() {
        let failure = ViewFailure::resource(ViewPhase::HostStaging, "é".repeat(2_000));

        assert!(failure.message.as_str().len() <= MAX_MESSAGE_BYTES);
        assert!(failure.message.as_str().ends_with('…'));
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::HostStaging);
        assert_eq!(failure.action(), RecoveryAction::RaiseNamedLimit);
        assert_eq!(failure.to_string().matches("safe action:").count(), 1);
    }

    #[test]
    fn external_path_and_error_formatting_is_capped_during_formatting() {
        let path = PathBuf::from("x".repeat(2 * MAX_MESSAGE_BYTES));
        let error = IndexError::ResourceLimit {
            limit: point_index::IndexLimit::BuildWorkingBytes,
            required: 2,
            allowed: 1,
        };

        let failure = ViewFailure::index(&path, &error);

        assert!(failure.message.as_str().len() <= MAX_MESSAGE_BYTES);
        assert!(failure.message.as_str().ends_with('…'));
        assert_eq!(failure.action(), RecoveryAction::RaiseNamedLimit);
    }

    #[test]
    fn allocation_failure_uses_a_nonempty_static_diagnostic() {
        let failure = ViewFailure::formatted_with_storage(
            ViewFailureCode::ResourceLimit,
            ViewPhase::HostStaging,
            format_args!("external detail that must not be formatted"),
            RecoveryAction::RaiseNamedLimit,
            None,
        );

        assert_eq!(failure.message.as_str(), ALLOCATION_FAILURE_MESSAGE);
        assert!(!failure.message.as_str().is_empty());
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::HostStaging);
        assert_eq!(failure.action(), RecoveryAction::RaiseNamedLimit);
        assert_eq!(failure.reowned(), failure);
    }

    #[test]
    fn invalid_request_has_stable_public_mapping() {
        let failure = ViewFailure::invalid_request("unsupported option");

        assert_eq!(failure.code(), ViewFailureCode::InvalidRequest);
        assert_eq!(failure.phase(), ViewPhase::RequestValidation);
        assert_eq!(failure.action(), RecoveryAction::CorrectRequest);
        assert!(failure.to_string().starts_with(
            "PVIEW_INVALID_REQUEST at request-validation\n  detail: unsupported option"
        ));
    }

    #[test]
    fn index_diagnostics_preserve_nested_source_and_cancellation_categories() {
        let resource = IndexError::from(SourceError::ResourceLimit {
            limit: point_source::ReadLimit::BatchPoints,
            required: 2,
            allowed: 1,
        });
        let resource = ViewFailure::index(Path::new("cache.pidx"), &resource);
        assert_eq!(resource.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(resource.action(), RecoveryAction::RaiseNamedLimit);

        let read_resource = IndexError::ResourceLimit {
            limit: point_index::IndexLimit::DisplayBatchBytes,
            required: 2,
            allowed: 1,
        };
        let read_resource = ViewFailure::index_read(&read_resource);
        assert_eq!(read_resource.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(read_resource.phase(), ViewPhase::NodeRead);
        assert_eq!(read_resource.action(), RecoveryAction::RaiseNamedLimit);

        let source_cancelled = IndexError::from(SourceError::Cancelled);
        let source_cancelled = ViewFailure::index(Path::new("cache.pidx"), &source_cancelled);
        assert_eq!(source_cancelled.code(), ViewFailureCode::Cancelled);

        let runtime_cancelled = IndexError::from(RuntimeError::Cancelled);
        let runtime_cancelled = ViewFailure::index(Path::new("cache.pidx"), &runtime_cancelled);
        assert_eq!(runtime_cancelled.code(), ViewFailureCode::Cancelled);
        assert_eq!(runtime_cancelled.action(), RecoveryAction::Retry);

        let read_cancelled = ViewFailure::index_read(&IndexError::from(RuntimeError::Cancelled));
        assert_eq!(read_cancelled.code(), ViewFailureCode::Cancelled);
        assert_eq!(read_cancelled.phase(), ViewPhase::NodeRead);
        assert_eq!(read_cancelled.action(), RecoveryAction::Retry);
    }
}
