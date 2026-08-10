//! Recoverable headless terrain workflow used by the `terrain-demo` binary.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cli;
mod diagnostic;
mod journal;
mod report;
mod workflow;

pub use cli::run_cli;
pub use diagnostic::WorkflowFailure;
pub use workflow::{
    WorkflowJob, WorkflowLimits, WorkflowPaths, WorkflowReceipt, WorkflowRunIntent, WorkflowStatus,
    inspect_run, resume_run, start_run,
};
