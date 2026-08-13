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
//! # Interface classification
//!
//! The documented preparation, lookup, and display-read APIs are a
//! **v1-candidate foundation surface**. Disk formats are separately versioned,
//! rebuildable cache contracts; that classification is not authority, a
//! `1.0.0` promise, or permission to reinterpret an unsupported artifact.
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
    CandidatePlan, DisplayAttributes, DisplayCoverage, DisplaySampleContract, IndexDescriptor,
    IndexHierarchy, IndexNode, IndexNodeId, IndexRecipe, InspectionAttributeIds,
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
    prepare_with_recipe(source, target, IndexRecipe::PositionOnlyV1, limits)
}

/// Builds, resumes, or opens one complete index using an explicit persisted recipe.
///
/// A target created by another recipe is preserved and rejected. Inspection
/// recipes validate their narrow Attribute profile before filesystem mutation.
#[must_use]
pub fn prepare_with_recipe(
    source: Source,
    target: impl AsRef<Path>,
    recipe: IndexRecipe,
    limits: PrepareLimits,
) -> IndexJob {
    prepare::start(source, target.as_ref().to_path_buf(), recipe, limits)
}

/// Builds a new complete index only when both target and work paths are absent.
///
/// Unlike [`prepare_with_recipe`], this operation never opens a complete target
/// or resumes an existing work file. A path that appears concurrently is
/// preserved and rejected. This is intended for measurements that must prove a
/// cold build rather than silently mixing build, resume, and open timings. A
/// successful build retains its rebuildable work cache because portable
/// filesystems do not provide an identity-conditional unlink for a predictable
/// path that another process may replace concurrently.
#[must_use]
pub fn prepare_fresh_with_recipe(
    source: Source,
    target: impl AsRef<Path>,
    recipe: IndexRecipe,
    limits: PrepareLimits,
) -> IndexJob {
    prepare::start_fresh(source, target.as_ref().to_path_buf(), recipe, limits)
}
