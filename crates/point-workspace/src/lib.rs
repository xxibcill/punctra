//! Durable edits over one verified Point Source and one complete spatial index.
//!
//! The crate owns exact Point selection, process-scoped spillable Point Sets,
//! immutable classification Revisions, and crash-reconcilable commit intents.
//! Source geometry and attributes remain authoritative and unchanged.
//!
//! Temporary scratch paths are never removed automatically: a pathname can be
//! replaced after its owner checks it, and portable filesystems provide no
//! conditional unlink tied to an open file identity. Each selection or commit
//! bounds the bytes it creates, while retained debris can accumulate across
//! attempts and is ignored by recovery. An operator may remove `scratch/`
//! contents only while no Workspace, Snapshot, Point Set, or job is live.
//! Publishing a retained named stage requires an independent descriptor-bound
//! copy; the reference macOS implementation uses clone-on-write publication,
//! while platforms without that primitive fail closed.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod hashes;
mod limits;
mod model;
mod persistence;
mod point_id_hash;
mod point_rows;
mod point_set;
mod query;
mod revision_audit;
mod selection;
mod util;
mod workspace;

pub use error::{MAX_WORKSPACE_DIAGNOSTIC_BYTES, WorkspaceDiagnostic, WorkspaceError};
pub use limits::{
    CommitLimits, OpenLimits, PointIdReadLimits, PointRowLimits, PointSetLimits,
    RevisionAuditLimits,
};
pub use model::{
    ClassificationTransition, CommitOutcome, CommitPhase, CommitReceipt, CommitRejection,
    CommitRequest, CommitUncertainty, OperationId, OperationResolution, PointQuery,
    PointSetMetadata, RecordedIntent, RecordedRejection, RevisionAudit, RevisionId, RevisionInfo,
    RevisionKind, SnapshotProvenance, WorkspaceId, WorkspaceSchema,
};
pub use point_rows::{SnapshotPointBatch, SnapshotPointBatches, SnapshotPointSummary};
pub use point_set::{PointIdBatch, PointIdBatches, PointSet};
pub use revision_audit::RevisionAuditJob;
pub use selection::PointSetJob;
pub use workspace::{CommitJob, Snapshot, Workspace, WorkspaceJob, create, open};
