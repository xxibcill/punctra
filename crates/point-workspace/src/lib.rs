//! Durable edits over one verified Point Source and one complete spatial index.
//!
//! The crate owns exact Point selection, process-scoped spillable Point Sets,
//! immutable classification Revisions, and crash-reconcilable commit intents.
//! Source geometry and attributes remain authoritative and unchanged.
//!
//! # Interface classification
//!
//! The documented Workspace, Snapshot, Query, Edit, audit, limits, and error
//! APIs are a **v1-candidate foundation surface**. Persisted stages and private
//! recovery controls are not interfaces. This records v0.9 review intent, not
//! a `1.0.0` or production-support claim.

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
