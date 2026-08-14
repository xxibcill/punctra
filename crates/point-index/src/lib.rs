//! Deterministic, rebuildable spatial access for one verified Point Source.
//!
//! `point-index` builds or opens one checksummed fixed-block bounds hierarchy.
//! It returns conservative Source spans for spatial candidates and bounded,
//! display-only samples for hierarchical rendering. The retained
//! [`point_source::Source`] remains authoritative for complete Point values.
//! Complete artifacts are trusted local caches: their unkeyed checksums detect
//! corruption, not deliberate rewriting. Discard and rebuild an artifact that
//! came from untrusted or adversarial storage.
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

pub use error::{IndexError, IndexLimit};
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
/// incompatible or corrupt target fails without being replaced. Successful
/// builds retain their valid work prefix because deleting it by pathname could
/// delete a racing replacement; when both paths exist, the complete artifact
/// wins and the work file is left untouched. Private named build stages and
/// sample spools may also remain as per-attempt bounded debris on platforms
/// that cannot publish from an unnamed file; Linux publication stages are
/// unnamed and leave no alias after their descriptor closes. `prepare` never
/// scans, adopts, or removes private names.
#[must_use]
pub fn prepare(source: Source, target: impl AsRef<Path>, limits: PrepareLimits) -> IndexJob {
    prepare::start(source, target.as_ref().to_path_buf(), limits)
}
