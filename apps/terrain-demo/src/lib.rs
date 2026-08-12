//! Recoverable headless terrain workflow used by the `terrain-demo` binary.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bounded_diagnostic;
mod cli;
mod diagnostic;
mod journal;
mod publication;
mod report;
mod roundtrip;
mod workflow;

pub use cli::run_cli;
pub use diagnostic::WorkflowFailure;
pub use journal::WorkflowRunId;
pub use workflow::{
    WorkflowJob, WorkflowLimits, WorkflowPaths, WorkflowPhase, WorkflowReceipt, WorkflowRunIntent,
    WorkflowStatus, inspect_and_repair_run, resume_run, start_run,
};
