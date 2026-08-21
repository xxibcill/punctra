//! Deterministic terrain derivation, persistent bounded streaming, and terrain QA.
//!
//! The crate consumes one immutable Workspace Snapshot and either derives one
//! complete in-memory 2.5D Terrain Surface or prepares one rebuildable,
//! file-backed Surface for an explicit inclusive area of interest. It also
//! evaluates detached Check Points and creates the narrow metric-metre
//! `LandXML` deliverable accepted for Punctra v0.6. A complete v0.12 structured
//! Source profile is propagated and must declare easting/northing/elevation
//! metre coordinates. Unsupported or opaque references fail Terrain derivation
//! and preparation with [`TerrainError::UnsupportedSpatialReference`].
//!
//! # In-memory derivation
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
//! )?;
//! let receipt = surface
//!     .ensure_landxml("existing-ground.xml", options, LandXmlLimits::default())
//!     .blocking_wait()?;
//! assert_eq!(receipt.vertex_count(), surface.descriptor().vertex_count());
//! # Ok(())
//! # }
//! ```
//!
//! # Persistent bounded-AOI preparation
//!
//! [`prepare`] requires an explicit inclusive [`point_contracts::WorldBounds`]
//! and publishes without replacing an existing target. The prepared handle
//! retains bounded metadata and an open file, while its vertices and faces are
//! consumed through separately limited batches. After publication, the
//! verified stage and any work sibling remain because a portable unlink cannot
//! be conditioned on the open owned file identity. Work is trusted for resume
//! only when that attempt verifies it.
//!
//! ```no_run
//! # fn run(snapshot: point_workspace::Snapshot) -> Result<(), Box<dyn std::error::Error>> {
//! use point_contracts::WorldBounds;
//! use point_terrain::{
//!     SurfaceReadLimits, TerrainPrepareLimits, TerrainRecipe, prepare,
//! };
//!
//! let aoi = WorldBounds::new(
//!     [500_000.0, 1_500_000.0, 0.0],
//!     [500_500.0, 1_500_500.0, 200.0],
//! )?;
//! let prepared = prepare(
//!     snapshot,
//!     "existing-ground.pterr",
//!     TerrainRecipe::new(2).within(aoi),
//!     TerrainPrepareLimits::default(),
//! )
//! .blocking_wait()?;
//!
//! let mut vertex_count = 0_u64;
//! for batch in prepared.vertex_batches(SurfaceReadLimits::default())? {
//!     vertex_count += u64::try_from(batch?.len())?;
//! }
//! assert_eq!(vertex_count, prepared.descriptor().vertex_count());
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
mod persistence;
mod qa;
mod sort;
mod triangulation;

pub use derive::derive;
pub use error::{MAX_TERRAIN_DIAGNOSTIC_BYTES, TerrainDiagnostic, TerrainError};
pub use landxml::LandXmlOptions;
pub use limits::{
    CheckPointLimits, LandXmlLimits, SurfaceReadLimits, TerrainLimits, TerrainPrepareLimits,
};
pub use model::{
    ALGORITHM_VERSION, CheckPoint, CheckPointId, CheckPointOutcome, CheckPointReport,
    CheckPointResult, LandXmlDisposition, LandXmlReceipt, ResidualStatistics, SurfaceFace,
    SurfaceFaceId, SurfaceVertex, SurfaceVertexId, TerrainDescriptor, TerrainRecipe,
    TerrainSurface,
};
pub use persistence::{
    PreparedTerrainSurface, SURFACE_DISK_VERSION, SurfaceArtifactDescriptor, SurfaceFaceBatches,
    SurfaceVertexBatches, TerrainPrepareDisposition, TerrainPrepareReport, prepare,
};

/// One-worker deterministic Terrain Derivation job.
pub type TerrainJob = foundation_runtime::Job<TerrainSurface, TerrainError>;

/// One-worker durable explicit-AOI Surface preparation job.
pub type TerrainPrepareJob = foundation_runtime::Job<PreparedTerrainSurface, TerrainError>;

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
