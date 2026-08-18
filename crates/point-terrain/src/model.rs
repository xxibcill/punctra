use std::{
    num::{NonZeroU32, NonZeroU64},
    sync::Arc,
};

use point_contracts::{
    ContentHash, CoordinateReference, PointId, PositionTransform, SpatialReferenceProfile,
    WorldBounds,
};
use point_workspace::SnapshotProvenance;

/// Deterministic terrain algorithm contract implemented by this crate.
pub const ALGORITHM_VERSION: u32 = 1;

/// Normalized Ground Input intent for one Terrain Derivation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainRecipe {
    ground_classification: u8,
    bounds: Option<WorldBounds>,
}

impl TerrainRecipe {
    /// Selects every Point whose effective classification equals `ground_classification`.
    #[must_use]
    pub const fn new(ground_classification: u8) -> Self {
        Self {
            ground_classification,
            bounds: None,
        }
    }

    /// Restricts Ground Input to one inclusive finite world box.
    #[must_use]
    pub const fn within(mut self, bounds: WorldBounds) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Returns the effective classification considered Ground Input.
    #[must_use]
    pub const fn ground_classification(self) -> u8 {
        self.ground_classification
    }

    /// Returns optional inclusive Ground Input bounds.
    #[must_use]
    pub const fn bounds(self) -> Option<WorldBounds> {
        self.bounds
    }
}

/// Stable nonzero identity of one canonical Terrain Surface vertex.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceVertexId(NonZeroU32);

impl SurfaceVertexId {
    pub(crate) fn from_zero_based(index: usize) -> Option<Self> {
        let value = u32::try_from(index).ok()?.checked_add(1)?;
        Some(Self(NonZeroU32::new(value)?))
    }

    /// Returns the one-based public identity.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    pub(crate) fn zero_based(self) -> usize {
        usize::try_from(self.get() - 1).unwrap_or(usize::MAX)
    }
}

/// Stable nonzero identity of one canonical Terrain Surface face.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceFaceId(NonZeroU32);

impl SurfaceFaceId {
    pub(crate) fn from_zero_based(index: usize) -> Option<Self> {
        let value = u32::try_from(index).ok()?.checked_add(1)?;
        Some(Self(NonZeroU32::new(value)?))
    }

    /// Returns the one-based public identity.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// One canonical Surface vertex retaining its authoritative Point and ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceVertex {
    id: SurfaceVertexId,
    point: PointId,
    ticks: [i64; 3],
}

impl SurfaceVertex {
    pub(crate) const fn new(id: SurfaceVertexId, point: PointId, ticks: [i64; 3]) -> Self {
        Self { id, point, ticks }
    }

    /// Returns the canonical one-based vertex identity.
    #[must_use]
    pub const fn id(self) -> SurfaceVertexId {
        self.id
    }

    /// Returns the authoritative Source-aware Point Identity.
    #[must_use]
    pub const fn point(self) -> PointId {
        self.point
    }

    /// Returns exact Source position ticks.
    #[must_use]
    pub const fn ticks(self) -> [i64; 3] {
        self.ticks
    }
}

/// One canonical counter-clockwise Terrain Surface face.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceFace {
    id: SurfaceFaceId,
    vertices: [SurfaceVertexId; 3],
}

impl SurfaceFace {
    pub(crate) const fn new(id: SurfaceFaceId, vertices: [SurfaceVertexId; 3]) -> Self {
        Self { id, vertices }
    }

    /// Returns the canonical one-based face identity.
    #[must_use]
    pub const fn id(self) -> SurfaceFaceId {
        self.id
    }

    /// Returns three canonical vertex identities in counter-clockwise order.
    #[must_use]
    pub const fn vertices(self) -> [SurfaceVertexId; 3] {
        self.vertices
    }
}

/// Immutable provenance, algorithm, topology, and resource facts.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDescriptor {
    snapshot: SnapshotProvenance,
    recipe: TerrainRecipe,
    recipe_hash: ContentHash,
    transform: PositionTransform,
    coordinate_reference: CoordinateReference,
    input_hash: ContentHash,
    geometry_hash: ContentHash,
    topology_hash: ContentHash,
    artifact_hash: ContentHash,
    input_point_count: u64,
    vertex_count: u64,
    face_count: u64,
    hull_vertex_count: u64,
    bounds: WorldBounds,
    accounted_peak_working_bytes: u64,
    retained_surface_bytes: u64,
    topology_steps: u64,
}

impl TerrainDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        snapshot: SnapshotProvenance,
        recipe: TerrainRecipe,
        recipe_hash: ContentHash,
        transform: PositionTransform,
        coordinate_reference: CoordinateReference,
        input_hash: ContentHash,
        geometry_hash: ContentHash,
        topology_hash: ContentHash,
        artifact_hash: ContentHash,
        input_point_count: u64,
        vertex_count: u64,
        face_count: u64,
        hull_vertex_count: u64,
        bounds: WorldBounds,
        accounted_peak_working_bytes: u64,
        retained_surface_bytes: u64,
        topology_steps: u64,
    ) -> Self {
        Self {
            snapshot,
            recipe,
            recipe_hash,
            transform,
            coordinate_reference,
            input_hash,
            geometry_hash,
            topology_hash,
            artifact_hash,
            input_point_count,
            vertex_count,
            face_count,
            hull_vertex_count,
            bounds,
            accounted_peak_working_bytes,
            retained_surface_bytes,
            topology_steps,
        }
    }

    /// Returns the immutable Snapshot used as Ground Input.
    #[must_use]
    pub const fn snapshot(&self) -> SnapshotProvenance {
        self.snapshot
    }

    /// Returns the normalized terrain Recipe.
    #[must_use]
    pub const fn recipe(&self) -> TerrainRecipe {
        self.recipe
    }

    /// Returns the canonical normalized Recipe digest.
    #[must_use]
    pub const fn recipe_hash(&self) -> ContentHash {
        self.recipe_hash
    }

    /// Returns the fixed terrain algorithm contract version.
    #[must_use]
    pub const fn algorithm_version(&self) -> u32 {
        ALGORITHM_VERSION
    }

    /// Returns the exact Source position transform.
    #[must_use]
    pub const fn position_transform(&self) -> PositionTransform {
        self.transform
    }

    /// Returns the Source-declared Coordinate Reference without inference.
    #[must_use]
    pub const fn coordinate_reference(&self) -> &CoordinateReference {
        &self.coordinate_reference
    }

    /// Returns the complete structured spatial profile when the Source supplied one.
    #[must_use]
    pub const fn spatial_reference_profile(&self) -> Option<SpatialReferenceProfile> {
        self.coordinate_reference.spatial_profile()
    }

    /// Returns the complete Snapshot Point-row content hash.
    #[must_use]
    pub const fn input_hash(&self) -> ContentHash {
        self.input_hash
    }

    /// Returns the canonical vertex-and-face geometry hash.
    #[must_use]
    pub const fn geometry_hash(&self) -> ContentHash {
        self.geometry_hash
    }

    /// Returns the canonical face-connectivity hash.
    #[must_use]
    pub const fn topology_hash(&self) -> ContentHash {
        self.topology_hash
    }

    /// Returns the provenance-sensitive Terrain Artifact hash.
    #[must_use]
    pub const fn artifact_hash(&self) -> ContentHash {
        self.artifact_hash
    }

    /// Returns the exact number of Ground Input rows.
    #[must_use]
    pub const fn input_point_count(&self) -> u64 {
        self.input_point_count
    }

    /// Returns the canonical vertex count.
    #[must_use]
    pub const fn vertex_count(&self) -> u64 {
        self.vertex_count
    }

    /// Returns the canonical face count.
    #[must_use]
    pub const fn face_count(&self) -> u64 {
        self.face_count
    }

    /// Returns the convex-hull vertex count.
    #[must_use]
    pub const fn hull_vertex_count(&self) -> u64 {
        self.hull_vertex_count
    }

    /// Returns inclusive three-dimensional Terrain bounds.
    #[must_use]
    pub const fn bounds(&self) -> WorldBounds {
        self.bounds
    }

    /// Returns the conservatively accounted peak Derivation working bytes.
    #[must_use]
    pub const fn accounted_peak_working_bytes(&self) -> u64 {
        self.accounted_peak_working_bytes
    }

    /// Returns retained in-memory Terrain Surface bytes.
    #[must_use]
    pub const fn retained_surface_bytes(&self) -> u64 {
        self.retained_surface_bytes
    }

    /// Returns charged topology primitive operations.
    #[must_use]
    pub const fn topology_steps(&self) -> u64 {
        self.topology_steps
    }
}

pub(crate) struct SurfaceData {
    pub(crate) descriptor: TerrainDescriptor,
    pub(crate) vertices: Vec<SurfaceVertex>,
    pub(crate) faces: Vec<SurfaceFace>,
}

/// Complete immutable in-memory Terrain Surface.
#[derive(Clone)]
pub struct TerrainSurface {
    pub(crate) inner: Arc<SurfaceData>,
}

impl TerrainSurface {
    /// Returns complete provenance, topology, hashes, and resource facts.
    #[must_use]
    pub fn descriptor(&self) -> &TerrainDescriptor {
        &self.inner.descriptor
    }

    /// Returns canonical vertices in ascending [`SurfaceVertexId`] order.
    #[must_use]
    pub fn vertices(&self) -> &[SurfaceVertex] {
        &self.inner.vertices
    }

    /// Returns canonical faces in ascending [`SurfaceFaceId`] order.
    #[must_use]
    pub fn faces(&self) -> &[SurfaceFace] {
        &self.inner.faces
    }

    #[cfg(test)]
    pub(crate) fn with_coordinate_reference_for_test(
        &self,
        coordinate_reference: CoordinateReference,
    ) -> Self {
        let mut descriptor = self.inner.descriptor.clone();
        descriptor.coordinate_reference = coordinate_reference;
        Self {
            inner: Arc::new(SurfaceData {
                descriptor,
                vertices: self.inner.vertices.clone(),
                faces: self.inner.faces.clone(),
            }),
        }
    }
}

impl std::fmt::Debug for TerrainSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerrainSurface")
            .field("descriptor", &self.inner.descriptor)
            .finish_non_exhaustive()
    }
}

/// Stable nonzero caller identity for one detached Check Point.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CheckPointId(NonZeroU64);

impl CheckPointId {
    /// Creates a checked nonzero Check Point identity.
    ///
    /// # Errors
    ///
    /// Returns [`crate::TerrainError::InvalidArgument`] when `value` is zero.
    pub fn new(value: u64) -> Result<Self, crate::TerrainError> {
        NonZeroU64::new(value).map(Self).ok_or_else(|| {
            crate::TerrainError::invalid("Check Point identity", "identity must be nonzero")
        })
    }

    /// Returns the caller-provided nonzero value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// One detached observed position in Terrain world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckPoint {
    id: CheckPointId,
    position: [f64; 3],
}

impl CheckPoint {
    /// Creates one finite Check Point.
    ///
    /// # Errors
    ///
    /// Returns [`crate::TerrainError::InvalidArgument`] when any coordinate is
    /// non-finite.
    pub fn new(id: CheckPointId, position: [f64; 3]) -> Result<Self, crate::TerrainError> {
        if let Some(axis) = position.iter().position(|value| !value.is_finite()) {
            return Err(crate::TerrainError::invalid(
                "Check Point position",
                format!("coordinate axis {axis} must be finite"),
            ));
        }
        Ok(Self { id, position })
    }

    /// Returns the caller identity.
    #[must_use]
    pub const fn id(self) -> CheckPointId {
        self.id
    }

    /// Returns finite world `[x, y, z]` coordinates.
    #[must_use]
    pub const fn position(self) -> [f64; 3] {
        self.position
    }
}

/// Terrain evaluation outcome for one detached Check Point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckPointOutcome {
    /// The Check Point lies on the closed Terrain Surface domain.
    Sampled {
        /// Deterministically selected containing face.
        face: SurfaceFaceId,
        /// Interpolated world elevation.
        surface_z: f64,
        /// Signed `observed_z - surface_z` residual.
        residual: f64,
    },
    /// The Check Point lies outside the Terrain Surface convex hull.
    Gap,
}

/// One ordered detached Check Point result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckPointResult {
    check_point: CheckPoint,
    outcome: CheckPointOutcome,
}

impl CheckPointResult {
    pub(crate) const fn new(check_point: CheckPoint, outcome: CheckPointOutcome) -> Self {
        Self {
            check_point,
            outcome,
        }
    }

    /// Returns the unchanged caller Check Point.
    #[must_use]
    pub const fn check_point(self) -> CheckPoint {
        self.check_point
    }

    /// Returns a sampled residual or explicit Terrain Gap.
    #[must_use]
    pub const fn outcome(self) -> CheckPointOutcome {
        self.outcome
    }
}

/// Aggregate signed-residual facts for covered Check Points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResidualStatistics {
    covered_count: u64,
    gap_count: u64,
    minimum: Option<f64>,
    maximum: Option<f64>,
    mean: Option<f64>,
    root_mean_square: Option<f64>,
}

impl ResidualStatistics {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        covered_count: u64,
        gap_count: u64,
        minimum: Option<f64>,
        maximum: Option<f64>,
        mean: Option<f64>,
        root_mean_square: Option<f64>,
    ) -> Self {
        Self {
            covered_count,
            gap_count,
            minimum,
            maximum,
            mean,
            root_mean_square,
        }
    }

    /// Returns the number of numeric residuals.
    #[must_use]
    pub const fn covered_count(self) -> u64 {
        self.covered_count
    }

    /// Returns the number of explicit Terrain Gaps.
    #[must_use]
    pub const fn gap_count(self) -> u64 {
        self.gap_count
    }

    /// Returns the minimum signed residual, absent when no Check Point is covered.
    #[must_use]
    pub const fn minimum(self) -> Option<f64> {
        self.minimum
    }

    /// Returns the maximum signed residual, absent when no Check Point is covered.
    #[must_use]
    pub const fn maximum(self) -> Option<f64> {
        self.maximum
    }

    /// Returns the arithmetic mean signed residual, absent for no coverage.
    #[must_use]
    pub const fn mean(self) -> Option<f64> {
        self.mean
    }

    /// Returns root-mean-square residual, absent for no coverage.
    #[must_use]
    pub const fn root_mean_square(self) -> Option<f64> {
        self.root_mean_square
    }
}

/// Complete ordered detached Check Point evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckPointReport {
    results: Box<[CheckPointResult]>,
    statistics: ResidualStatistics,
    face_tests: u64,
    accounted_peak_working_bytes: u64,
}

impl CheckPointReport {
    pub(crate) const fn new(
        results: Box<[CheckPointResult]>,
        statistics: ResidualStatistics,
        face_tests: u64,
        accounted_peak_working_bytes: u64,
    ) -> Self {
        Self {
            results,
            statistics,
            face_tests,
            accounted_peak_working_bytes,
        }
    }

    /// Returns results in caller input order.
    #[must_use]
    pub fn results(&self) -> &[CheckPointResult] {
        &self.results
    }

    /// Returns complete residual statistics.
    #[must_use]
    pub const fn statistics(&self) -> ResidualStatistics {
        self.statistics
    }

    /// Returns the exact number of charged face containment tests.
    #[must_use]
    pub const fn face_tests(&self) -> u64 {
        self.face_tests
    }

    /// Returns conservatively accounted peak incremental bytes.
    #[must_use]
    pub const fn accounted_peak_working_bytes(&self) -> u64 {
        self.accounted_peak_working_bytes
    }
}

/// How one successful `LandXML` ensure operation satisfied its target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LandXmlDisposition {
    /// This operation durably created the target without replacement.
    Created,
    /// A complete byte-identical target already existed and was verified.
    ReconciledExisting,
}

/// Receipt for one durably published or reconciled `LandXML` export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LandXmlReceipt {
    disposition: LandXmlDisposition,
    surface_artifact_hash: ContentHash,
    recipe_hash: ContentHash,
    input_hash: ContentHash,
    geometry_hash: ContentHash,
    topology_hash: ContentHash,
    content_hash: ContentHash,
    byte_length: u64,
    vertex_count: u64,
    face_count: u64,
}

impl LandXmlReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        disposition: LandXmlDisposition,
        surface_artifact_hash: ContentHash,
        recipe_hash: ContentHash,
        input_hash: ContentHash,
        geometry_hash: ContentHash,
        topology_hash: ContentHash,
        content_hash: ContentHash,
        byte_length: u64,
        vertex_count: u64,
        face_count: u64,
    ) -> Self {
        Self {
            disposition,
            surface_artifact_hash,
            recipe_hash,
            input_hash,
            geometry_hash,
            topology_hash,
            content_hash,
            byte_length,
            vertex_count,
            face_count,
        }
    }

    /// Returns whether this operation created or reconciled the target.
    #[must_use]
    pub const fn disposition(self) -> LandXmlDisposition {
        self.disposition
    }

    /// Returns the exported Terrain Artifact hash.
    #[must_use]
    pub const fn surface_artifact_hash(self) -> ContentHash {
        self.surface_artifact_hash
    }

    /// Returns the exported surface's normalized Recipe hash.
    #[must_use]
    pub const fn recipe_hash(self) -> ContentHash {
        self.recipe_hash
    }

    /// Returns the exported surface's canonical input Point-row hash.
    #[must_use]
    pub const fn input_hash(self) -> ContentHash {
        self.input_hash
    }

    /// Returns the exported geometry hash.
    #[must_use]
    pub const fn geometry_hash(self) -> ContentHash {
        self.geometry_hash
    }

    /// Returns the exported topology hash.
    #[must_use]
    pub const fn topology_hash(self) -> ContentHash {
        self.topology_hash
    }

    /// Returns the exact published XML content hash.
    #[must_use]
    pub const fn content_hash(self) -> ContentHash {
        self.content_hash
    }

    /// Returns the exact published byte length.
    #[must_use]
    pub const fn byte_length(self) -> u64 {
        self.byte_length
    }

    /// Returns the exact exported point count.
    #[must_use]
    pub const fn vertex_count(self) -> u64 {
        self.vertex_count
    }

    /// Returns the exact exported face count.
    #[must_use]
    pub const fn face_count(self) -> u64 {
        self.face_count
    }
}
