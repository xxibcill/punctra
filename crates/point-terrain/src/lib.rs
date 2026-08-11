//! Deterministic terrain derivation and bounded terrain QA.
//!
//! The crate consumes one immutable Workspace Snapshot, derives one complete
//! in-memory 2.5D Terrain Surface, evaluates detached Check Points, and creates
//! the narrow metric-metre `LandXML` deliverable accepted for Punctra v0.6.
//!
//! # Example
//!
//! ```no_run
//! # fn run(snapshot: point_workspace::Snapshot) -> Result<(), point_terrain::TerrainError> {
//! use point_terrain::{
//!     LandXmlLimits, LandXmlOptions, TerrainLimits, TerrainRecipe, derive,
//! };
//!
//! let surface = derive(
//!     snapshot,
//!     TerrainRecipe::new(2),
//!     TerrainLimits::default(),
//! )
//! .blocking_wait()?;
//! let options = LandXmlOptions::metric_metres(
//!     "Existing Ground",
//!     "2026-08-10",
//!     "00:00:00Z",
//! )?
//! .assert_coordinates_are_metric_metres();
//! let receipt = surface
//!     .export_landxml("existing-ground.xml", options, LandXmlLimits::default())
//!     .blocking_wait()?;
//! assert_eq!(receipt.vertex_count(), surface.descriptor().vertex_count());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod derive;
mod error;
mod landxml;
mod limits;
mod model;
mod numeric;
mod qa;
mod triangulation;

pub use derive::derive;
pub use error::{MAX_TERRAIN_DIAGNOSTIC_BYTES, TerrainDiagnostic, TerrainError};
pub use landxml::LandXmlOptions;
pub use limits::{CheckPointLimits, LandXmlLimits, TerrainLimits};
pub use model::{
    ALGORITHM_VERSION, CheckPoint, CheckPointId, CheckPointOutcome, CheckPointReport,
    CheckPointResult, LandXmlReceipt, ResidualStatistics, SurfaceFace, SurfaceFaceId,
    SurfaceVertex, SurfaceVertexId, TerrainDescriptor, TerrainRecipe, TerrainSurface,
};

/// One-worker deterministic Terrain Derivation job.
pub type TerrainJob = foundation_runtime::Job<TerrainSurface, TerrainError>;

/// One-worker detached Check Point evaluation job.
pub type CheckPointJob = foundation_runtime::Job<CheckPointReport, TerrainError>;

/// One-worker metric-metre `LandXML` publication job.
pub type LandXmlJob = foundation_runtime::Job<LandXmlReceipt, TerrainError>;

impl TerrainSurface {
    /// Starts bounded deterministic evaluation of detached Check Points.
    #[must_use]
    pub fn check_points<I>(&self, check_points: I, limits: CheckPointLimits) -> CheckPointJob
    where
        I: IntoIterator<Item = CheckPoint>,
    {
        qa::start(self, check_points, limits)
    }

    /// Creates one metric-metre `LandXML` 1.2 file without replacing a target.
    #[must_use]
    pub fn export_landxml(
        &self,
        target: impl AsRef<std::path::Path>,
        options: LandXmlOptions,
        limits: LandXmlLimits,
    ) -> LandXmlJob {
        landxml::start(self, target, options, limits)
    }
}
