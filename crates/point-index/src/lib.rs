//! Deterministic, rebuildable spatial access for one verified Point Source.
//!
//! `point-index` builds or opens one checksummed fixed-block bounds hierarchy.
//! It returns conservative Source spans for spatial candidates and bounded,
//! display-only samples for hierarchical rendering. The retained
//! [`point_source::Source`] remains authoritative for complete Point values.
//!
//! ```no_run
//! use point_index::{CandidateLimits, PrepareLimits, prepare};
//! use point_source::Source;
//! # fn example(source: Source) -> Result<(), point_index::IndexError> {
//! let index = prepare(source, "cloud.pidx", PrepareLimits::default()).blocking_wait()?;
//! if let Some(bounds) = index.descriptor().world_bounds() {
//!     let candidates = index.candidates(bounds, CandidateLimits::default())?;
//!     println!("{} candidate points", candidates.candidate_point_count());
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod limits;
mod model;
mod persistence;
mod prepare;
mod read;
mod tree;

use std::path::Path;

use foundation_runtime::Job;
use point_source::Source;

pub use error::IndexError;
pub use limits::{CandidateLimits, NodeReadBudget, PrepareLimits};
pub use model::{
    CandidatePlan, DisplayCoverage, IndexDescriptor, IndexHierarchy, IndexNode, IndexNodeId,
    PrepareDisposition, PrepareReport, PreparedIndex,
};
pub use read::{IndexPointBatch, IndexPointBatches, IndexReadSummary, IndexSample};

/// Background preparation job for one complete index.
pub type IndexJob = Job<PreparedIndex, IndexError>;

/// Builds, resumes, or opens one complete index bound to `source`.
///
/// A compatible complete artifact is opened without rebuilding. Otherwise a
/// compatible append-only work file is resumed, or a new one is created. An
/// incompatible or corrupt target fails without being replaced.
#[must_use]
pub fn prepare(source: Source, target: impl AsRef<Path>, limits: PrepareLimits) -> IndexJob {
    prepare::start(source, target.as_ref().to_path_buf(), limits)
}
