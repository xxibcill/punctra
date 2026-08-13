//! Deterministic terrain derivation and bounded terrain QA.
//!
//! The crate consumes one immutable Workspace Snapshot, derives one complete
//! in-memory 2.5D Terrain Surface, evaluates detached Check Points, and creates
//! the narrow metric-metre `LandXML` deliverable accepted for Punctra v0.6.
//!
//! # Interface classification
//!
//! The documented derivation, QA, and narrow `LandXML` APIs are a
//! **v1-candidate foundation surface**. Private encoders, publication stages,
//! and triangulator internals are not interfaces. The classification records
//! v0.9 review intent and is not a `1.0.0` or production-support claim.
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
//!     .ensure_landxml("existing-ground.xml", options, LandXmlLimits::default())
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
mod sort;
mod triangulation;

pub use derive::derive;
pub use error::{MAX_TERRAIN_DIAGNOSTIC_BYTES, TerrainDiagnostic, TerrainError};
pub use landxml::LandXmlOptions;
pub use limits::{CheckPointLimits, LandXmlLimits, TerrainLimits};
pub use model::{
    ALGORITHM_VERSION, CheckPoint, CheckPointId, CheckPointOutcome, CheckPointReport,
    CheckPointResult, LandXmlDisposition, LandXmlReceipt, ResidualStatistics, SurfaceFace,
    SurfaceFaceId, SurfaceVertex, SurfaceVertexId, TerrainDescriptor, TerrainRecipe,
    TerrainSurface,
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
        I: IntoIterator<Item = CheckPoint> + Send + 'static,
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

    /// Ensures one exact metric-metre `LandXML` 1.2 target without replacement.
    ///
    /// A missing target is created through the same durable create-new
    /// protocol as [`Self::export_landxml`]. A byte-identical existing regular
    /// file is verified and reconciled. Any other existing target fails
    /// without being modified.
    #[must_use]
    pub fn ensure_landxml(
        &self,
        target: impl AsRef<std::path::Path>,
        options: LandXmlOptions,
        limits: LandXmlLimits,
    ) -> LandXmlJob {
        landxml::start_ensure(self, target, options, limits)
    }
}
