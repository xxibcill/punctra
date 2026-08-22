use std::{mem, num::NonZeroU32};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::{ContentHash, PointId, SpatialReferenceProfile};
use point_workspace::{PointQuery, Snapshot, SnapshotPointSummary, SnapshotProvenance};

use crate::{
    ALGORITHM_VERSION, CheckPoint, CheckPointId, CheckPointLimits, CheckPointOutcome,
    PreparedTerrainSurface, ResidualStatistics, TerrainError, TerrainQaLimits, TerrainSurface,
    limits::{require_within, usize_to_u64_saturating},
    persistence::{SurfaceMaterialization, SurfaceMaterializationLimits},
    qa::ResidualAccumulator,
};

const INPUT_HASH_DOMAIN: &[u8] = b"punctra-exact-terrain-qa-input-v1";
const RESULT_HASH_DOMAIN: &[u8] = b"punctra-exact-terrain-qa-result-v1";
const CANCELLATION_STRIDE: usize = 1_024;

/// Asymmetric inclusive vertical residual tolerance in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalTolerance {
    below_metres: f64,
    above_metres: f64,
}

impl VerticalTolerance {
    /// Creates finite nonnegative lower and upper tolerance magnitudes.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::InvalidArgument`] for a negative or non-finite value.
    pub fn new(below_metres: f64, above_metres: f64) -> Result<Self, TerrainError> {
        if !below_metres.is_finite()
            || !above_metres.is_finite()
            || below_metres < 0.0
            || above_metres < 0.0
        {
            return Err(TerrainError::invalid(
                "vertical tolerance",
                "lower and upper metre magnitudes must be finite and nonnegative",
            ));
        }
        Ok(Self {
            below_metres: canonical_zero(below_metres),
            above_metres: canonical_zero(above_metres),
        })
    }

    /// Returns the accepted magnitude below the Surface in metres.
    #[must_use]
    pub const fn below_metres(self) -> f64 {
        self.below_metres
    }

    /// Returns the accepted magnitude above the Surface in metres.
    #[must_use]
    pub const fn above_metres(self) -> f64 {
        self.above_metres
    }

    fn classify(self, residual: f64) -> ToleranceDisposition {
        if residual < -self.below_metres {
            ToleranceDisposition::Below
        } else if residual > self.above_metres {
            ToleranceDisposition::Above
        } else {
            ToleranceDisposition::Within
        }
    }
}

/// One finite, representable-length line sampled at exact, evenly spaced stations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StationProfile {
    start_xy: [f64; 2],
    end_xy: [f64; 2],
    intervals: NonZeroU32,
}

impl StationProfile {
    /// Creates a profile with `intervals + 1` stations including both endpoints.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::InvalidArgument`] when an endpoint is non-finite,
    /// the planar length is zero or not representable as a finite `f64`, or
    /// `intervals` is zero.
    pub fn new(start_xy: [f64; 2], end_xy: [f64; 2], intervals: u32) -> Result<Self, TerrainError> {
        if start_xy
            .into_iter()
            .chain(end_xy)
            .any(|coordinate| !coordinate.is_finite())
        {
            return Err(TerrainError::invalid(
                "station profile",
                "profile endpoints must be finite",
            ));
        }
        let intervals = NonZeroU32::new(intervals).ok_or_else(|| {
            TerrainError::invalid("station profile", "profile interval count must be nonzero")
        })?;
        let length = (end_xy[0] - start_xy[0]).hypot(end_xy[1] - start_xy[1]);
        if !length.is_finite() || length == 0.0 {
            return Err(TerrainError::invalid(
                "station profile",
                "profile length must be finite and nonzero",
            ));
        }
        Ok(Self {
            start_xy,
            end_xy,
            intervals,
        })
    }

    /// Returns the world-XY start point.
    #[must_use]
    pub const fn start_xy(self) -> [f64; 2] {
        self.start_xy
    }

    /// Returns the world-XY end point.
    #[must_use]
    pub const fn end_xy(self) -> [f64; 2] {
        self.end_xy
    }

    /// Returns the nonzero interval count.
    #[must_use]
    pub const fn intervals(self) -> u32 {
        self.intervals.get()
    }

    /// Returns the exact number of generated stations.
    #[must_use]
    pub fn station_count(self) -> u64 {
        u64::from(self.intervals.get()) + 1
    }

    fn length_metres(self) -> f64 {
        (self.end_xy[0] - self.start_xy[0]).hypot(self.end_xy[1] - self.start_xy[1])
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "profile interval indices are bounded u32 values and intentionally mapped to f64 stations"
    )]
    fn station(self, index: u32) -> ([f64; 2], f64) {
        let fraction = f64::from(index) / f64::from(self.intervals.get());
        let xy = if index == 0 {
            self.start_xy
        } else if index == self.intervals.get() {
            self.end_xy
        } else {
            [
                self.start_xy[0] + (self.end_xy[0] - self.start_xy[0]) * fraction,
                self.start_xy[1] + (self.end_xy[1] - self.start_xy[1]) * fraction,
            ]
        };
        (xy, canonical_zero(self.length_metres() * fraction))
    }
}

/// Caller-owned inputs for one exact QA report.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactTerrainQaRequest {
    tolerance: VerticalTolerance,
    source_query: Option<PointQuery>,
    check_points: Box<[CheckPoint]>,
    profile: Option<StationProfile>,
}

impl ExactTerrainQaRequest {
    /// Creates an empty request under one explicit tolerance.
    #[must_use]
    pub fn new(tolerance: VerticalTolerance) -> Self {
        Self {
            tolerance,
            source_query: None,
            check_points: Box::new([]),
            profile: None,
        }
    }

    /// Adds one exact Source-Point Query.
    #[must_use]
    pub const fn source_points(mut self, query: PointQuery) -> Self {
        self.source_query = Some(query);
        self
    }

    /// Adds caller-owned detached Check Points in report order.
    #[must_use]
    pub fn check_points(mut self, check_points: impl Into<Box<[CheckPoint]>>) -> Self {
        self.check_points = check_points.into();
        self
    }

    /// Adds one exact station profile.
    #[must_use]
    pub const fn profile(mut self, profile: StationProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Returns the declared vertical tolerance.
    #[must_use]
    pub const fn tolerance(&self) -> VerticalTolerance {
        self.tolerance
    }

    /// Returns the optional exact Source-Point Query.
    #[must_use]
    pub const fn source_query(&self) -> Option<PointQuery> {
        self.source_query
    }

    /// Returns detached Check Points in caller order.
    #[must_use]
    pub fn detached_check_points(&self) -> &[CheckPoint] {
        &self.check_points
    }

    /// Returns the optional station profile.
    #[must_use]
    pub const fn station_profile(&self) -> Option<StationProfile> {
        self.profile
    }
}

/// Relationship between one residual and the declared inclusive tolerance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToleranceDisposition {
    /// Residual is less than the negative lower tolerance.
    Below,
    /// Residual lies inside both inclusive tolerance limits.
    Within,
    /// Residual is greater than the upper tolerance.
    Above,
}

/// Exact Surface sampling outcome carrying tolerance only for residual inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResidualOutcome {
    /// The XY position lies on the closed Terrain domain.
    Sampled {
        /// Deterministically selected canonical face.
        face: crate::SurfaceFaceId,
        /// Interpolated Surface elevation in metres.
        surface_z: f64,
        /// Signed observed-minus-Surface elevation in metres.
        residual: f64,
        /// Inclusive tolerance classification.
        tolerance: ToleranceDisposition,
    },
    /// The XY position lies outside the Surface domain.
    Gap,
}

/// Exact Surface sampling outcome for one profile station.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProfileOutcome {
    /// The station lies on the closed Terrain domain.
    Sampled {
        /// Deterministically selected canonical face.
        face: crate::SurfaceFaceId,
        /// Interpolated Surface elevation in metres.
        surface_z: f64,
    },
    /// The station lies outside the Surface domain.
    Gap,
}

/// One exact Source Point evaluated against a frozen Surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourcePointResidual {
    point: PointId,
    ticks: [i64; 3],
    world_position: [f64; 3],
    effective_classification: u8,
    outcome: ResidualOutcome,
}

impl SourcePointResidual {
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

    /// Returns the exact transformed world position in metres.
    #[must_use]
    pub const fn world_position(self) -> [f64; 3] {
        self.world_position
    }

    /// Returns the effective classification at the frozen Snapshot.
    #[must_use]
    pub const fn effective_classification(self) -> u8 {
        self.effective_classification
    }

    /// Returns a residual sample or explicit gap.
    #[must_use]
    pub const fn outcome(self) -> ResidualOutcome {
        self.outcome
    }
}

/// One detached Check Point evaluated against a frozen Surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckPointResidual {
    check_point: CheckPoint,
    outcome: ResidualOutcome,
}

impl CheckPointResidual {
    /// Returns the unchanged caller input.
    #[must_use]
    pub const fn check_point(self) -> CheckPoint {
        self.check_point
    }

    /// Returns a residual sample or explicit gap.
    #[must_use]
    pub const fn outcome(self) -> ResidualOutcome {
        self.outcome
    }
}

/// One authoritative station in an exact sampled profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileStationResult {
    index: u32,
    station_metres: f64,
    world_xy: [f64; 2],
    outcome: ProfileOutcome,
}

impl ProfileStationResult {
    /// Returns the zero-based station index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Returns horizontal distance from the profile start in metres.
    #[must_use]
    pub const fn station_metres(self) -> f64 {
        self.station_metres
    }

    /// Returns the exact generated world-XY coordinate in metres.
    #[must_use]
    pub const fn world_xy(self) -> [f64; 2] {
        self.world_xy
    }

    /// Returns a Surface sample or explicit gap.
    #[must_use]
    pub const fn outcome(self) -> ProfileOutcome {
        self.outcome
    }
}

/// Completed exact Source-row input facts retained by one QA report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourcePointInputSummary {
    query: PointQuery,
    candidate_point_count: u64,
    exact_count: u64,
    point_id_hash: ContentHash,
    content_hash: ContentHash,
}

impl SourcePointInputSummary {
    fn from_summary(summary: &SnapshotPointSummary) -> Self {
        Self {
            query: summary.query(),
            candidate_point_count: summary.candidate_point_count(),
            exact_count: summary.exact_count(),
            point_id_hash: summary.point_id_hash(),
            content_hash: summary.content_hash(),
        }
    }

    /// Returns the exact Query evaluated at the frozen Snapshot.
    #[must_use]
    pub const fn query(self) -> PointQuery {
        self.query
    }

    /// Returns conservative candidate Points examined by the Query.
    #[must_use]
    pub const fn candidate_point_count(self) -> u64 {
        self.candidate_point_count
    }

    /// Returns exact emitted Source rows.
    #[must_use]
    pub const fn exact_count(self) -> u64 {
        self.exact_count
    }

    /// Returns the canonical ordered Point-identity hash.
    #[must_use]
    pub const fn point_id_hash(self) -> ContentHash {
        self.point_id_hash
    }

    /// Returns the provenance-bound row-content hash.
    #[must_use]
    pub const fn content_hash(self) -> ContentHash {
        self.content_hash
    }
}

/// Aggregate counts across Source and detached residual outcomes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ToleranceSummary {
    below: u64,
    within: u64,
    above: u64,
    gaps: u64,
}

impl ToleranceSummary {
    /// Returns residuals below the lower tolerance.
    #[must_use]
    pub const fn below_count(self) -> u64 {
        self.below
    }

    /// Returns residuals inside both inclusive limits.
    #[must_use]
    pub const fn within_count(self) -> u64 {
        self.within
    }

    /// Returns residuals above the upper tolerance.
    #[must_use]
    pub const fn above_count(self) -> u64 {
        self.above
    }

    /// Returns explicit residual-input gaps.
    #[must_use]
    pub const fn gap_count(self) -> u64 {
        self.gaps
    }

    fn observe(&mut self, outcome: ResidualOutcome) -> Result<(), TerrainError> {
        let count = match outcome {
            ResidualOutcome::Gap => &mut self.gaps,
            ResidualOutcome::Sampled {
                tolerance: ToleranceDisposition::Below,
                ..
            } => &mut self.below,
            ResidualOutcome::Sampled {
                tolerance: ToleranceDisposition::Within,
                ..
            } => &mut self.within,
            ResidualOutcome::Sampled {
                tolerance: ToleranceDisposition::Above,
                ..
            } => &mut self.above,
        };
        *count = count
            .checked_add(1)
            .ok_or_else(|| TerrainError::numeric("QA tolerance count overflowed"))?;
        Ok(())
    }
}

/// Immutable identities and semantic hashes behind one exact QA report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainQaBinding {
    snapshot: SnapshotProvenance,
    recipe_hash: ContentHash,
    input_hash: ContentHash,
    geometry_hash: ContentHash,
    topology_hash: ContentHash,
    artifact_hash: ContentHash,
    spatial_reference: SpatialReferenceProfile,
}

impl TerrainQaBinding {
    fn from_surface(surface: &TerrainSurface) -> Result<Self, TerrainError> {
        let descriptor = surface.descriptor();
        let spatial_reference = descriptor
            .spatial_reference_profile()
            .filter(|profile| profile.is_supported_metric_survey())
            .ok_or_else(|| {
                TerrainError::unsupported_spatial_reference(
                    "exact Terrain QA requires a complete easting/northing/elevation profile with metre horizontal and vertical units",
                )
            })?;
        Ok(Self {
            snapshot: descriptor.snapshot(),
            recipe_hash: descriptor.recipe_hash(),
            input_hash: descriptor.input_hash(),
            geometry_hash: descriptor.geometry_hash(),
            topology_hash: descriptor.topology_hash(),
            artifact_hash: descriptor.artifact_hash(),
            spatial_reference,
        })
    }

    /// Returns the exact Workspace, Source, and Revision identity.
    #[must_use]
    pub const fn snapshot(self) -> SnapshotProvenance {
        self.snapshot
    }

    /// Returns the normalized Terrain Recipe hash.
    #[must_use]
    pub const fn recipe_hash(self) -> ContentHash {
        self.recipe_hash
    }

    /// Returns the complete Ground-Input hash.
    #[must_use]
    pub const fn input_hash(self) -> ContentHash {
        self.input_hash
    }

    /// Returns the canonical geometry hash.
    #[must_use]
    pub const fn geometry_hash(self) -> ContentHash {
        self.geometry_hash
    }

    /// Returns the canonical topology hash.
    #[must_use]
    pub const fn topology_hash(self) -> ContentHash {
        self.topology_hash
    }

    /// Returns the provenance-sensitive Surface Artifact hash.
    #[must_use]
    pub const fn artifact_hash(self) -> ContentHash {
        self.artifact_hash
    }

    /// Returns the complete supported metric spatial profile.
    #[must_use]
    pub const fn spatial_reference(self) -> SpatialReferenceProfile {
        self.spatial_reference
    }

    /// Returns the Terrain algorithm version bound through the Surface hashes.
    #[must_use]
    pub const fn algorithm_version(self) -> u32 {
        ALGORITHM_VERSION
    }

    /// Compares this evidence binding with a caller-declared current state.
    #[must_use]
    pub fn freshness(self, current: TerrainQaCurrentState) -> TerrainQaFreshness {
        let stale_snapshot = self.snapshot.workspace() != current.snapshot.workspace()
            || self.snapshot.source() != current.snapshot.source()
            || self.snapshot.revision() != current.snapshot.revision();
        let snapshot_only = current.surface.is_none();
        let stale_surface = match current.surface {
            None => false,
            Some(surface) => {
                self.artifact_hash != surface.artifact_hash
                    || surface.snapshot.workspace() != current.snapshot.workspace()
                    || surface.snapshot.source() != current.snapshot.source()
                    || surface.snapshot.revision() != current.snapshot.revision()
            }
        };
        match (stale_snapshot, stale_surface, snapshot_only) {
            (false, false, true) => TerrainQaFreshness::SnapshotOnlyCurrent,
            (false, false, false) => TerrainQaFreshness::Current,
            (true, false, _) => TerrainQaFreshness::StaleSnapshot,
            (false, true, _) => TerrainQaFreshness::StaleSurface,
            (true, true, _) => TerrainQaFreshness::StaleSnapshotAndSurface,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CurrentSurfaceState {
    snapshot: SnapshotProvenance,
    artifact_hash: ContentHash,
}

/// Caller-declared current Snapshot and Surface state for a freshness check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainQaCurrentState {
    snapshot: SnapshotProvenance,
    surface: Option<CurrentSurfaceState>,
}

impl TerrainQaCurrentState {
    /// Captures current Snapshot state without declaring a current Surface.
    #[must_use]
    pub fn snapshot(snapshot: &Snapshot) -> Self {
        Self {
            snapshot: *snapshot.provenance(),
            surface: None,
        }
    }

    /// Captures current state from an in-memory Surface.
    #[must_use]
    pub fn in_memory(snapshot: &Snapshot, surface: &TerrainSurface) -> Self {
        Self {
            snapshot: *snapshot.provenance(),
            surface: Some(CurrentSurfaceState {
                snapshot: surface.descriptor().snapshot(),
                artifact_hash: surface.descriptor().artifact_hash(),
            }),
        }
    }

    /// Captures current state from a file-backed Surface.
    #[must_use]
    pub fn prepared(snapshot: &Snapshot, surface: &PreparedTerrainSurface) -> Self {
        Self {
            snapshot: *snapshot.provenance(),
            surface: Some(CurrentSurfaceState {
                snapshot: surface.descriptor().snapshot(),
                artifact_hash: surface.descriptor().artifact_hash(),
            }),
        }
    }
}

/// Whether exact QA evidence is current for a caller-declared Snapshot/Surface state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainQaFreshness {
    /// Snapshot and Surface exactly match the evidence binding.
    Current,
    /// Snapshot matches the evidence binding and no current Surface was declared.
    SnapshotOnlyCurrent,
    /// The current Snapshot differs and no current Surface was declared.
    StaleSnapshot,
    /// The current Surface differs or is not derived from the current Snapshot.
    StaleSurface,
    /// Both the current Snapshot and Surface state differ.
    StaleSnapshotAndSurface,
}

/// Complete CPU-authoritative QA evidence for one frozen Snapshot/Surface pair.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactTerrainQaReport {
    binding: TerrainQaBinding,
    tolerance: VerticalTolerance,
    source_input: Option<SourcePointInputSummary>,
    source_points: Box<[SourcePointResidual]>,
    check_points: Box<[CheckPointResidual]>,
    profile: Option<StationProfile>,
    profile_stations: Box<[ProfileStationResult]>,
    statistics: ResidualStatistics,
    tolerance_summary: ToleranceSummary,
    profile_gap_count: u64,
    input_hash: ContentHash,
    result_hash: ContentHash,
    face_tests: u64,
    accounted_peak_working_bytes: u64,
    retained_result_bytes: u64,
}

impl ExactTerrainQaReport {
    /// Returns the exact immutable Snapshot/Surface binding.
    #[must_use]
    pub const fn binding(&self) -> TerrainQaBinding {
        self.binding
    }

    /// Returns the declared asymmetric vertical tolerance in metres.
    #[must_use]
    pub const fn tolerance(&self) -> VerticalTolerance {
        self.tolerance
    }

    /// Returns completed Source-row input facts when a Query was supplied.
    #[must_use]
    pub const fn source_input(&self) -> Option<SourcePointInputSummary> {
        self.source_input
    }

    /// Returns exact Source residuals in canonical Source-row order.
    #[must_use]
    pub fn source_points(&self) -> &[SourcePointResidual] {
        &self.source_points
    }

    /// Returns detached Check Point residuals in caller order.
    #[must_use]
    pub fn check_points(&self) -> &[CheckPointResidual] {
        &self.check_points
    }

    /// Returns the optional profile definition.
    #[must_use]
    pub const fn profile(&self) -> Option<StationProfile> {
        self.profile
    }

    /// Returns exact profile stations from start through end.
    #[must_use]
    pub fn profile_stations(&self) -> &[ProfileStationResult] {
        &self.profile_stations
    }

    /// Returns aggregate Source and Check Point residual statistics.
    #[must_use]
    pub const fn statistics(&self) -> ResidualStatistics {
        self.statistics
    }

    /// Returns aggregate Source and Check Point tolerance counts.
    #[must_use]
    pub const fn tolerance_summary(&self) -> ToleranceSummary {
        self.tolerance_summary
    }

    /// Returns explicit profile station gaps.
    #[must_use]
    pub const fn profile_gap_count(&self) -> u64 {
        self.profile_gap_count
    }

    /// Returns the canonical request and completed-input hash.
    #[must_use]
    pub const fn input_hash(&self) -> ContentHash {
        self.input_hash
    }

    /// Returns the canonical binding-and-outcome hash.
    #[must_use]
    pub const fn result_hash(&self) -> ContentHash {
        self.result_hash
    }

    /// Returns charged deterministic face-containment tests.
    #[must_use]
    pub const fn face_tests(&self) -> u64 {
        self.face_tests
    }

    /// Returns conservative peak incremental QA bytes.
    #[must_use]
    pub const fn accounted_peak_working_bytes(&self) -> u64 {
        self.accounted_peak_working_bytes
    }

    /// Returns exact retained report payload bytes.
    #[must_use]
    pub const fn retained_result_bytes(&self) -> u64 {
        self.retained_result_bytes
    }
}

#[derive(Clone, Copy)]
struct SourceInput {
    point: PointId,
    ticks: [i64; 3],
    world_position: [f64; 3],
    effective_classification: u8,
}

struct QaEvaluation {
    source_points: Box<[SourcePointResidual]>,
    check_points: Box<[CheckPointResidual]>,
    profile_stations: Box<[ProfileStationResult]>,
    statistics: ResidualStatistics,
    tolerance_summary: ToleranceSummary,
    profile_gap_count: u64,
    face_tests: u64,
    peak_working_bytes: u64,
}

struct BoxedResults<T> {
    values: Box<[T]>,
    peak_working_bytes: u64,
}

struct EvaluationState<'a> {
    surface: &'a TerrainSurface,
    tolerance: VerticalTolerance,
    locator_limits: CheckPointLimits,
    face_tests: u64,
    residuals: ResidualAccumulator,
    tolerance_summary: ToleranceSummary,
    control: &'a OperationControl,
}

impl EvaluationState<'_> {
    fn residual(&mut self, position: [f64; 3]) -> Result<ResidualOutcome, TerrainError> {
        let outcome = locate_residual(
            self.surface,
            position,
            self.tolerance,
            self.locator_limits,
            &mut self.face_tests,
            self.control,
        )?;
        self.residuals.observe(as_check_point_outcome(outcome))?;
        self.tolerance_summary.observe(outcome)?;
        Ok(outcome)
    }

    fn profile(&mut self, world_xy: [f64; 2]) -> Result<ProfileOutcome, TerrainError> {
        Ok(
            match crate::qa::sample_surface(
                self.surface,
                world_xy,
                self.locator_limits,
                &mut self.face_tests,
                self.control,
            )? {
                None => ProfileOutcome::Gap,
                Some(sample) => ProfileOutcome::Sampled {
                    face: sample.face,
                    surface_z: sample.surface_z,
                },
            },
        )
    }
}

pub(crate) fn start_in_memory(
    surface: &TerrainSurface,
    snapshot: Snapshot,
    request: ExactTerrainQaRequest,
    limits: TerrainQaLimits,
) -> crate::ExactTerrainQaJob {
    let surface = surface.clone();
    crate::ExactTerrainQaJob::spawn(move |control| {
        run(
            &snapshot,
            &surface,
            &request,
            limits,
            QaWorkingBaseline::default(),
            &control,
        )
    })
}

pub(crate) fn start_prepared(
    surface: &PreparedTerrainSurface,
    snapshot: Snapshot,
    request: ExactTerrainQaRequest,
    limits: TerrainQaLimits,
) -> crate::ExactTerrainQaJob {
    let surface = surface.clone();
    crate::ExactTerrainQaJob::spawn(move |control| {
        control.check_cancelled()?;
        validate_snapshot_binding(&snapshot, surface.descriptor().snapshot())?;
        let SurfaceMaterialization {
            surface,
            retained_bytes,
            peak_working_bytes,
        } = surface.materialize_in_memory(
            SurfaceMaterializationLimits::new(
                limits.surface_read(),
                limits.max_materialized_surface_bytes(),
                limits.max_working_bytes(),
            ),
            &control,
        )?;
        run(
            &snapshot,
            &surface,
            &request,
            limits,
            QaWorkingBaseline {
                retained_surface_bytes: retained_bytes,
                prior_peak_working_bytes: peak_working_bytes,
            },
            &control,
        )
    })
}

#[derive(Clone, Copy, Default)]
struct QaWorkingBaseline {
    retained_surface_bytes: u64,
    prior_peak_working_bytes: u64,
}

fn run(
    snapshot: &Snapshot,
    surface: &TerrainSurface,
    request: &ExactTerrainQaRequest,
    limits: TerrainQaLimits,
    working: QaWorkingBaseline,
    control: &OperationControl,
) -> Result<ExactTerrainQaReport, TerrainError> {
    control.check_cancelled()?;
    let binding = TerrainQaBinding::from_surface(surface)?;
    validate_snapshot_binding(snapshot, binding.snapshot)?;
    let validation_peak_working_bytes =
        validate_request(request, limits, working.retained_surface_bytes, control)?;
    let (source_inputs, source_input, source_collection_peak_working_bytes) =
        collect_source_inputs(
            snapshot,
            request,
            limits,
            working.retained_surface_bytes,
            control,
        )?;
    let check_count = u64::try_from(request.check_points.len()).unwrap_or(u64::MAX);
    let profile_count = request.profile.map_or(0, StationProfile::station_count);
    let observation_count = u64::try_from(source_inputs.len())
        .unwrap_or(u64::MAX)
        .saturating_add(check_count)
        .saturating_add(profile_count);
    require_within(
        "exact QA observations",
        observation_count,
        limits.max_observations(),
    )?;
    let retained_result_bytes = result_bytes(source_inputs.len(), request)?;
    require_within(
        "exact QA retained result bytes",
        retained_result_bytes,
        limits.max_result_bytes(),
    )?;
    let source_input_bytes = allocation_bytes::<SourceInput>(source_inputs.capacity());
    let pre_evaluation_peak_working_bytes = validation_peak_working_bytes
        .max(source_collection_peak_working_bytes)
        .max(
            working
                .retained_surface_bytes
                .saturating_add(source_input_bytes)
                .saturating_add(retained_result_bytes),
        );
    require_within(
        "exact QA working bytes",
        pre_evaluation_peak_working_bytes,
        limits.max_working_bytes(),
    )?;

    let evaluation = evaluate(
        surface,
        request,
        source_inputs,
        observation_count,
        working.retained_surface_bytes,
        limits,
        control,
    )?;
    let peak_working_bytes = working
        .prior_peak_working_bytes
        .max(pre_evaluation_peak_working_bytes)
        .max(evaluation.peak_working_bytes);
    control.check_cancelled()?;
    let input_hash = hash_input(request, source_input, control)?;
    let result_hash = hash_results(
        binding,
        request.tolerance,
        input_hash,
        &evaluation.source_points,
        &evaluation.check_points,
        &evaluation.profile_stations,
        control,
    )?;
    control.complete_progress(observation_count)?;
    Ok(ExactTerrainQaReport {
        binding,
        tolerance: request.tolerance,
        source_input,
        source_points: evaluation.source_points,
        check_points: evaluation.check_points,
        profile: request.profile,
        profile_stations: evaluation.profile_stations,
        statistics: evaluation.statistics,
        tolerance_summary: evaluation.tolerance_summary,
        profile_gap_count: evaluation.profile_gap_count,
        input_hash,
        result_hash,
        face_tests: evaluation.face_tests,
        accounted_peak_working_bytes: peak_working_bytes,
        retained_result_bytes,
    })
}

fn validate_snapshot_binding(
    snapshot: &Snapshot,
    surface_snapshot: SnapshotProvenance,
) -> Result<(), TerrainError> {
    if *snapshot.provenance() != surface_snapshot {
        return Err(TerrainError::invalid(
            "exact QA binding",
            "Snapshot provenance must exactly match Surface provenance",
        ));
    }
    Ok(())
}

fn evaluate(
    surface: &TerrainSurface,
    request: &ExactTerrainQaRequest,
    source_inputs: Vec<SourceInput>,
    observation_count: u64,
    base_working_bytes: u64,
    limits: TerrainQaLimits,
    control: &OperationControl,
) -> Result<QaEvaluation, TerrainError> {
    let mut state = EvaluationState {
        surface,
        tolerance: request.tolerance,
        locator_limits: CheckPointLimits::new(
            observation_count,
            limits.max_result_bytes(),
            limits.max_face_tests(),
            limits.max_working_bytes(),
        ),
        face_tests: 0,
        residuals: ResidualAccumulator::default(),
        tolerance_summary: ToleranceSummary::default(),
        control,
    };
    let source_points =
        evaluate_source_points(source_inputs, base_working_bytes, limits, &mut state)?;
    let retained_source_bytes = payload_bytes::<SourcePointResidual>(source_points.values.len());
    let check_points = evaluate_check_points(
        &request.check_points,
        base_working_bytes,
        retained_source_bytes,
        limits,
        &mut state,
    )?;
    let retained_check_bytes = payload_bytes::<CheckPointResidual>(check_points.values.len());
    let (profile_stations, profile_gap_count) = evaluate_profile(
        request.profile,
        base_working_bytes,
        retained_source_bytes.saturating_add(retained_check_bytes),
        limits,
        &mut state,
    )?;
    let peak_working_bytes = source_points
        .peak_working_bytes
        .max(check_points.peak_working_bytes)
        .max(profile_stations.peak_working_bytes);
    Ok(QaEvaluation {
        source_points: source_points.values,
        check_points: check_points.values,
        profile_stations: profile_stations.values,
        statistics: state.residuals.finish(),
        tolerance_summary: state.tolerance_summary,
        profile_gap_count,
        face_tests: state.face_tests,
        peak_working_bytes,
    })
}

fn evaluate_source_points(
    inputs: Vec<SourceInput>,
    base_working_bytes: u64,
    limits: TerrainQaLimits,
    state: &mut EvaluationState<'_>,
) -> Result<BoxedResults<SourcePointResidual>, TerrainError> {
    let source_input_bytes = allocation_bytes::<SourceInput>(inputs.capacity());
    let mut results = allocate_exact::<SourcePointResidual>(inputs.len())?;
    for (index, input) in inputs.into_iter().enumerate() {
        poll(index, state.control)?;
        results.push(SourcePointResidual {
            point: input.point,
            ticks: input.ticks,
            world_position: input.world_position,
            effective_classification: input.effective_classification,
            outcome: state.residual(input.world_position)?,
        });
    }
    box_results(results, base_working_bytes, 0, source_input_bytes, limits)
}

fn evaluate_check_points(
    inputs: &[CheckPoint],
    base_working_bytes: u64,
    retained_result_bytes: u64,
    limits: TerrainQaLimits,
    state: &mut EvaluationState<'_>,
) -> Result<BoxedResults<CheckPointResidual>, TerrainError> {
    let mut results = allocate_exact::<CheckPointResidual>(inputs.len())?;
    for (index, check_point) in inputs.iter().copied().enumerate() {
        poll(index, state.control)?;
        results.push(CheckPointResidual {
            check_point,
            outcome: state.residual(check_point.position())?,
        });
    }
    box_results(
        results,
        base_working_bytes,
        retained_result_bytes,
        0,
        limits,
    )
}

fn evaluate_profile(
    profile: Option<StationProfile>,
    base_working_bytes: u64,
    retained_result_bytes: u64,
    limits: TerrainQaLimits,
    state: &mut EvaluationState<'_>,
) -> Result<(BoxedResults<ProfileStationResult>, u64), TerrainError> {
    let Some(profile) = profile else {
        return Ok((
            BoxedResults {
                values: Box::new([]),
                peak_working_bytes: base_working_bytes.saturating_add(retained_result_bytes),
            },
            0,
        ));
    };
    let count = profile.station_count();
    let count = usize::try_from(count)
        .map_err(|_| TerrainError::resource("profile stations", count, usize_limit()))?;
    let mut results = allocate_exact::<ProfileStationResult>(count)?;
    let mut gaps = 0_u64;
    for index in 0..=profile.intervals() {
        poll(usize::try_from(index).unwrap_or(usize::MAX), state.control)?;
        let (world_xy, station_metres) = profile.station(index);
        let outcome = state.profile(world_xy)?;
        if outcome == ProfileOutcome::Gap {
            gaps = gaps
                .checked_add(1)
                .ok_or_else(|| TerrainError::numeric("profile gap count overflowed"))?;
        }
        results.push(ProfileStationResult {
            index,
            station_metres,
            world_xy,
            outcome,
        });
    }
    let bytes = allocation_bytes::<ProfileStationResult>(results.capacity());
    require_within("profile result bytes", bytes, limits.max_result_bytes())?;
    Ok((
        box_results(
            results,
            base_working_bytes,
            retained_result_bytes,
            0,
            limits,
        )?,
        gaps,
    ))
}

fn box_results<T>(
    results: Vec<T>,
    base_working_bytes: u64,
    retained_result_bytes: u64,
    concurrent_working_bytes: u64,
    limits: TerrainQaLimits,
) -> Result<BoxedResults<T>, TerrainError> {
    let allocation_bytes = allocation_bytes::<T>(results.capacity());
    let boxed_bytes = payload_bytes::<T>(results.len());
    let peak_working_bytes = base_working_bytes
        .saturating_add(retained_result_bytes)
        .saturating_add(concurrent_working_bytes)
        .saturating_add(allocation_bytes)
        .saturating_add(boxed_bytes);
    require_within(
        "exact QA boxed result conversion working bytes",
        peak_working_bytes,
        limits.max_working_bytes(),
    )?;
    Ok(BoxedResults {
        values: results.into_boxed_slice(),
        peak_working_bytes,
    })
}

fn validate_request(
    request: &ExactTerrainQaRequest,
    limits: TerrainQaLimits,
    base_working_bytes: u64,
    control: &OperationControl,
) -> Result<u64, TerrainError> {
    if request.source_query.is_none()
        && request.check_points.is_empty()
        && request.profile.is_none()
    {
        return Err(TerrainError::invalid(
            "exact QA request",
            "at least one Source Query, Check Point, or profile is required",
        ));
    }
    let check_count = u64::try_from(request.check_points.len()).unwrap_or(u64::MAX);
    require_within(
        "exact QA detached Check Points",
        check_count,
        limits.max_check_points(),
    )?;
    if let Some(profile) = request.profile {
        require_within(
            "profile stations",
            profile.station_count(),
            limits.max_profile_stations(),
        )?;
    }
    let identity_payload_bytes = payload_bytes::<CheckPointId>(request.check_points.len());
    require_within(
        "exact QA identity validation working bytes",
        base_working_bytes.saturating_add(identity_payload_bytes),
        limits.max_working_bytes(),
    )?;
    let mut identities = allocate_exact::<CheckPointId>(request.check_points.len())?;
    let identity_bytes =
        base_working_bytes.saturating_add(allocation_bytes::<CheckPointId>(identities.capacity()));
    require_within(
        "exact QA identity validation working bytes",
        identity_bytes,
        limits.max_working_bytes(),
    )?;
    extend_with_cancellation(
        &mut identities,
        request.check_points.iter().map(|point| point.id()),
        control,
    )?;
    crate::qa::sort_identities(&mut identities, control)?;
    for (index, pair) in identities.windows(2).enumerate() {
        poll(index, control)?;
        if pair[0] == pair[1] {
            return Err(TerrainError::invalid(
                "detached Check Point identities",
                format!("identity {} occurs more than once", pair[0].get()),
            ));
        }
    }
    Ok(identity_bytes)
}

fn collect_source_inputs(
    snapshot: &Snapshot,
    request: &ExactTerrainQaRequest,
    limits: TerrainQaLimits,
    base_working_bytes: u64,
    control: &OperationControl,
) -> Result<(Vec<SourceInput>, Option<SourcePointInputSummary>, u64), TerrainError> {
    let Some(query) = request.source_query else {
        return Ok((Vec::new(), None, base_working_bytes));
    };
    let stream_working_bytes = limits.point_rows().max_working_bytes();
    let mut collection_peak_working_bytes = base_working_bytes.saturating_add(stream_working_bytes);
    require_within(
        "exact QA Source collection working bytes",
        collection_peak_working_bytes,
        limits.max_working_bytes(),
    )?;
    let mut rows = snapshot.point_rows(query, limits.point_rows())?;
    let _rows_parent_link = rows.handle().token().link_to_parent(&control.token())?;
    let mut inputs = Vec::new();
    while let Some(batch) = rows.next()? {
        control.check_cancelled()?;
        for row in 0..batch.len() {
            poll(row, control)?;
            let required = u64::try_from(inputs.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            require_within(
                "exact QA Source Points",
                required,
                limits.max_source_points(),
            )?;
            if inputs.len() == inputs.capacity() {
                let growth_peak = grow_source_inputs(
                    &mut inputs,
                    limits,
                    base_working_bytes,
                    stream_working_bytes,
                )?;
                collection_peak_working_bytes = collection_peak_working_bytes.max(growth_peak);
            }
            let point = batch.point_id(row).ok_or_else(|| {
                TerrainError::numeric("Snapshot Point row has no valid Point Identity")
            })?;
            let ticks = *batch.positions().ticks().get(row).ok_or_else(|| {
                TerrainError::numeric("Snapshot Point row has no exact position ticks")
            })?;
            let world_position = batch.positions().world_f64(row).ok_or_else(|| {
                TerrainError::numeric("Snapshot Point row has no finite world position")
            })?;
            let effective_classification =
                *batch.effective_classifications().get(row).ok_or_else(|| {
                    TerrainError::numeric("Snapshot Point row has no effective classification")
                })?;
            inputs.push(SourceInput {
                point,
                ticks,
                world_position,
                effective_classification,
            });
        }
    }
    let summary = rows.summary().ok_or_else(|| {
        TerrainError::numeric("Snapshot Point rows ended without a terminal summary")
    })?;
    if summary.exact_count() != u64::try_from(inputs.len()).unwrap_or(u64::MAX) {
        return Err(TerrainError::numeric(
            "Snapshot Point summary count differs from collected QA input",
        ));
    }
    Ok((
        inputs,
        Some(SourcePointInputSummary::from_summary(summary)),
        collection_peak_working_bytes,
    ))
}

fn grow_source_inputs(
    inputs: &mut Vec<SourceInput>,
    limits: TerrainQaLimits,
    base_working_bytes: u64,
    stream_working_bytes: u64,
) -> Result<u64, TerrainError> {
    let maximum = usize::try_from(limits.max_source_points()).unwrap_or(usize::MAX);
    let requested = inputs
        .capacity()
        .max(1)
        .saturating_mul(2)
        .min(maximum)
        .max(inputs.len().saturating_add(1));
    let bytes = allocation_bytes::<SourceInput>(requested);
    let overlap = base_working_bytes
        .saturating_add(stream_working_bytes)
        .saturating_add(allocation_bytes::<SourceInput>(inputs.capacity()))
        .saturating_add(bytes);
    require_within(
        "exact QA Source input growth overlap",
        overlap,
        limits.max_working_bytes(),
    )?;
    inputs
        .try_reserve_exact(requested.saturating_sub(inputs.len()))
        .map_err(|_| {
            TerrainError::resource(
                "exact QA Source input allocation",
                bytes,
                limits.max_working_bytes(),
            )
        })?;
    let retained = base_working_bytes
        .saturating_add(stream_working_bytes)
        .saturating_add(allocation_bytes::<SourceInput>(inputs.capacity()));
    require_within(
        "exact QA Source collection working bytes",
        retained,
        limits.max_working_bytes(),
    )?;
    Ok(overlap.max(retained))
}

fn locate_residual(
    surface: &TerrainSurface,
    position: [f64; 3],
    tolerance: VerticalTolerance,
    limits: CheckPointLimits,
    face_tests: &mut u64,
    control: &OperationControl,
) -> Result<ResidualOutcome, TerrainError> {
    let Some(sample) = crate::qa::sample_surface(
        surface,
        [position[0], position[1]],
        limits,
        face_tests,
        control,
    )?
    else {
        return Ok(ResidualOutcome::Gap);
    };
    let residual = canonical_zero(position[2] - sample.surface_z);
    if !residual.is_finite() {
        return Err(TerrainError::numeric(
            "exact Terrain QA residual is not finite",
        ));
    }
    Ok(ResidualOutcome::Sampled {
        face: sample.face,
        surface_z: sample.surface_z,
        residual,
        tolerance: tolerance.classify(residual),
    })
}

fn as_check_point_outcome(outcome: ResidualOutcome) -> CheckPointOutcome {
    match outcome {
        ResidualOutcome::Gap => CheckPointOutcome::Gap,
        ResidualOutcome::Sampled {
            face,
            surface_z,
            residual,
            ..
        } => CheckPointOutcome::Sampled {
            face,
            surface_z,
            residual,
        },
    }
}

fn result_bytes(source_count: usize, request: &ExactTerrainQaRequest) -> Result<u64, TerrainError> {
    let profile_count = request.profile.map_or(0, StationProfile::station_count);
    let profile_count = usize::try_from(profile_count)
        .map_err(|_| TerrainError::numeric("profile station count is not addressable"))?;
    Ok(payload_bytes::<SourcePointResidual>(source_count)
        .saturating_add(payload_bytes::<CheckPointResidual>(
            request.check_points.len(),
        ))
        .saturating_add(payload_bytes::<ProfileStationResult>(profile_count)))
}

fn hash_input(
    request: &ExactTerrainQaRequest,
    source: Option<SourcePointInputSummary>,
    control: &OperationControl,
) -> Result<ContentHash, TerrainError> {
    control.check_cancelled()?;
    let mut hasher = Hasher::new();
    hasher.update(INPUT_HASH_DOMAIN);
    hasher.update(&request.tolerance.below_metres.to_bits().to_le_bytes());
    hasher.update(&request.tolerance.above_metres.to_bits().to_le_bytes());
    match source {
        None => {
            hasher.update(&[0]);
        }
        Some(summary) => {
            hasher.update(&[1]);
            hash_query(&mut hasher, summary.query);
            hasher.update(&summary.candidate_point_count.to_le_bytes());
            hasher.update(&summary.exact_count.to_le_bytes());
            hasher.update(summary.point_id_hash.as_bytes());
            hasher.update(summary.content_hash.as_bytes());
        }
    }
    hasher.update(&usize_to_u64_saturating(request.check_points.len()).to_le_bytes());
    for (index, point) in request.check_points.iter().enumerate() {
        poll(index, control)?;
        hasher.update(&point.id().get().to_le_bytes());
        for coordinate in point.position() {
            hasher.update(&coordinate.to_bits().to_le_bytes());
        }
    }
    match request.profile {
        None => {
            hasher.update(&[0]);
        }
        Some(profile) => {
            hasher.update(&[1]);
            for coordinate in profile.start_xy.into_iter().chain(profile.end_xy) {
                hasher.update(&coordinate.to_bits().to_le_bytes());
            }
            hasher.update(&profile.intervals().to_le_bytes());
        }
    }
    Ok(ContentHash::new(*hasher.finalize().as_bytes()))
}

fn hash_query(hasher: &mut Hasher, query: PointQuery) {
    match query.bounds() {
        None => {
            hasher.update(&[0]);
        }
        Some(bounds) => {
            hasher.update(&[1]);
            for coordinate in bounds.min().into_iter().chain(bounds.max()) {
                hasher.update(&coordinate.to_bits().to_le_bytes());
            }
        }
    }
    match query.classification_eq() {
        None => {
            hasher.update(&[0]);
        }
        Some(classification) => {
            hasher.update(&[1, classification]);
        }
    }
}

fn hash_results(
    binding: TerrainQaBinding,
    tolerance: VerticalTolerance,
    input_hash: ContentHash,
    source_points: &[SourcePointResidual],
    check_points: &[CheckPointResidual],
    profile_stations: &[ProfileStationResult],
    control: &OperationControl,
) -> Result<ContentHash, TerrainError> {
    control.check_cancelled()?;
    let mut hasher = Hasher::new();
    hasher.update(RESULT_HASH_DOMAIN);
    hasher.update(binding.snapshot.workspace().as_bytes());
    hasher.update(binding.snapshot.source().as_bytes());
    hasher.update(binding.snapshot.revision().as_bytes());
    hasher.update(&ALGORITHM_VERSION.to_le_bytes());
    for hash in [
        binding.recipe_hash,
        binding.input_hash,
        binding.geometry_hash,
        binding.topology_hash,
        binding.artifact_hash,
        input_hash,
    ] {
        hasher.update(hash.as_bytes());
    }
    hasher.update(&binding.spatial_reference.canonical_bytes());
    hasher.update(&tolerance.below_metres.to_bits().to_le_bytes());
    hasher.update(&tolerance.above_metres.to_bits().to_le_bytes());
    hasher.update(&usize_to_u64_saturating(source_points.len()).to_le_bytes());
    for (index, result) in source_points.iter().enumerate() {
        poll(index, control)?;
        hasher.update(result.point.source().as_bytes());
        hasher.update(&result.point.ordinal().to_le_bytes());
        for tick in result.ticks {
            hasher.update(&tick.to_le_bytes());
        }
        hasher.update(&[result.effective_classification]);
        hash_residual_outcome(&mut hasher, result.outcome);
    }
    hasher.update(&usize_to_u64_saturating(check_points.len()).to_le_bytes());
    for (index, result) in check_points.iter().enumerate() {
        poll(index, control)?;
        hasher.update(&result.check_point.id().get().to_le_bytes());
        for coordinate in result.check_point.position() {
            hasher.update(&coordinate.to_bits().to_le_bytes());
        }
        hash_residual_outcome(&mut hasher, result.outcome);
    }
    hasher.update(&usize_to_u64_saturating(profile_stations.len()).to_le_bytes());
    for (index, result) in profile_stations.iter().enumerate() {
        poll(index, control)?;
        hasher.update(&result.index.to_le_bytes());
        hasher.update(&result.station_metres.to_bits().to_le_bytes());
        for coordinate in result.world_xy {
            hasher.update(&coordinate.to_bits().to_le_bytes());
        }
        match result.outcome {
            ProfileOutcome::Gap => {
                hasher.update(&[0]);
            }
            ProfileOutcome::Sampled { face, surface_z } => {
                hasher.update(&[1]);
                hasher.update(&face.get().to_le_bytes());
                hasher.update(&surface_z.to_bits().to_le_bytes());
            }
        }
    }
    Ok(ContentHash::new(*hasher.finalize().as_bytes()))
}

fn hash_residual_outcome(hasher: &mut Hasher, outcome: ResidualOutcome) {
    match outcome {
        ResidualOutcome::Gap => {
            hasher.update(&[0]);
        }
        ResidualOutcome::Sampled {
            face,
            surface_z,
            residual,
            tolerance,
        } => {
            hasher.update(&[1]);
            hasher.update(&face.get().to_le_bytes());
            hasher.update(&surface_z.to_bits().to_le_bytes());
            hasher.update(&residual.to_bits().to_le_bytes());
            hasher.update(&[match tolerance {
                ToleranceDisposition::Below => 0,
                ToleranceDisposition::Within => 1,
                ToleranceDisposition::Above => 2,
            }]);
        }
    }
}

fn allocate_exact<T>(count: usize) -> Result<Vec<T>, TerrainError> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| {
        TerrainError::resource(
            "exact QA allocation",
            payload_bytes::<T>(count),
            usize_limit(),
        )
    })?;
    Ok(values)
}

fn extend_with_cancellation<T>(
    target: &mut Vec<T>,
    values: impl IntoIterator<Item = T>,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    for (index, value) in values.into_iter().enumerate() {
        poll(index, control)?;
        target.push(value);
    }
    Ok(())
}

fn payload_bytes<T>(count: usize) -> u64 {
    usize_to_u64_saturating(count).saturating_mul(usize_to_u64_saturating(mem::size_of::<T>()))
}

fn allocation_bytes<T>(capacity: usize) -> u64 {
    payload_bytes::<T>(capacity)
}

fn usize_limit() -> u64 {
    u64::try_from(usize::MAX).unwrap_or(u64::MAX)
}

fn poll(index: usize, control: &OperationControl) -> Result<(), TerrainError> {
    if index.is_multiple_of(CANCELLATION_STRIDE) {
        control.check_cancelled()?;
    }
    Ok(())
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use foundation_runtime::OperationControl;
    use point_contracts::{
        ContentHash, LinearUnit, SpatialAxes, SpatialReferenceProfile, SpatialReferenceProvenance,
    };
    use point_workspace::SnapshotProvenance;

    use super::{
        CANCELLATION_STRIDE, CheckPointResidual, ExactTerrainQaRequest, ResidualOutcome,
        StationProfile, TerrainQaBinding, VerticalTolerance, extend_with_cancellation, hash_input,
        hash_results, poll, validate_request,
    };
    use crate::{CheckPoint, CheckPointId, TerrainError, TerrainQaLimits};

    #[test]
    fn profile_stations_preserve_the_declared_endpoints() {
        let profile = StationProfile::new([1.0e16, 2.0], [1.0, 3.0], 1).unwrap();

        assert_eq!(
            profile.station(0).0.map(f64::to_bits),
            profile.start_xy().map(f64::to_bits)
        );
        assert_eq!(
            profile.station(1).0.map(f64::to_bits),
            profile.end_xy().map(f64::to_bits)
        );
    }

    #[test]
    fn batch_extension_observes_cancellation_at_the_bounded_stride() {
        let control = OperationControl::new();
        let values = (0..=CANCELLATION_STRIDE).inspect(|&index| {
            if index == CANCELLATION_STRIDE {
                control.cancel();
            }
        });
        let mut collected = Vec::new();

        let error = extend_with_cancellation(&mut collected, values, &control)
            .expect_err("the next batch-copy boundary must observe cancellation");

        assert!(matches!(error, TerrainError::Cancelled));
        assert_eq!(collected.len(), CANCELLATION_STRIDE);
    }

    #[test]
    fn identity_validation_observes_cancellation_while_sorting() {
        let check_points = (1..=2_048_u64)
            .rev()
            .map(|identity| {
                CheckPoint::new(CheckPointId::new(identity).unwrap(), [0.0; 3]).unwrap()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let request = ExactTerrainQaRequest::new(VerticalTolerance::new(0.0, 0.0).unwrap())
            .check_points(check_points);
        let control = OperationControl::new();
        control.cancel();

        let error = validate_request(&request, TerrainQaLimits::default(), 0, &control)
            .expect_err("identity sorting must observe cancellation at a bounded interval");

        assert!(matches!(error, TerrainError::Cancelled));
    }

    #[test]
    fn evaluation_poll_observes_cancellation_at_the_bounded_stride() {
        let control = OperationControl::new();
        control.cancel();

        poll(CANCELLATION_STRIDE - 1, &control)
            .expect("work before the next cancellation boundary may complete");
        let error = poll(CANCELLATION_STRIDE, &control)
            .expect_err("the next evaluation boundary must observe cancellation");

        assert!(matches!(error, TerrainError::Cancelled));
    }

    #[test]
    fn input_hashing_observes_cancellation() {
        let request = ExactTerrainQaRequest::new(VerticalTolerance::new(0.0, 0.0).unwrap())
            .check_points(
                vec![CheckPoint::new(CheckPointId::new(1).unwrap(), [0.0; 3]).unwrap()]
                    .into_boxed_slice(),
            );
        let control = OperationControl::new();
        control.cancel();

        let error = hash_input(&request, None, &control)
            .expect_err("input hashing must observe cancellation before publishing evidence");

        assert!(matches!(error, TerrainError::Cancelled));
    }

    #[test]
    fn empty_result_hashing_observes_cancellation() {
        let control = OperationControl::new();
        control.cancel();

        let error = hash_results(
            deterministic_binding(),
            VerticalTolerance::new(0.0, 0.0).unwrap(),
            ContentHash::new([9; 32]),
            &[],
            &[],
            &[],
            &control,
        )
        .expect_err("empty result hashing must observe cancellation before publishing evidence");

        assert!(matches!(error, TerrainError::Cancelled));
    }

    #[test]
    fn exact_qa_input_and_result_hashes_match_golden_digests() {
        let check_point =
            CheckPoint::new(CheckPointId::new(42).unwrap(), [1.25, -2.5, 3.75]).unwrap();
        let tolerance = VerticalTolerance::new(0.25, 0.75).unwrap();
        let request = ExactTerrainQaRequest::new(tolerance)
            .check_points(vec![check_point].into_boxed_slice());
        let control = OperationControl::new();
        let input_hash = hash_input(&request, None, &control).unwrap();
        let result_hash = hash_results(
            deterministic_binding(),
            tolerance,
            input_hash,
            &[],
            &[CheckPointResidual {
                check_point,
                outcome: ResidualOutcome::Gap,
            }],
            &[],
            &control,
        )
        .unwrap();

        assert_eq!(
            input_hash,
            ContentHash::new([
                0x43, 0xf6, 0x6c, 0x32, 0x4e, 0x20, 0x40, 0xa9, 0x31, 0xed, 0x4c, 0xd2, 0x45, 0x15,
                0xd7, 0x1e, 0x5a, 0x7f, 0xd9, 0xdf, 0x85, 0xd8, 0xc9, 0xbb, 0x41, 0x55, 0xb4, 0x5e,
                0xdc, 0x2c, 0xea, 0x83,
            ])
        );
        assert_eq!(
            result_hash,
            ContentHash::new([
                0xa7, 0x8e, 0x6a, 0xe2, 0xd0, 0xba, 0x6a, 0x9c, 0xb3, 0x80, 0xe0, 0x43, 0xc2, 0xce,
                0x0c, 0x02, 0x7e, 0x29, 0xa4, 0xda, 0x9e, 0x92, 0x04, 0xa1, 0x58, 0x6c, 0xe9, 0xe6,
                0x64, 0xed, 0x20, 0x73,
            ])
        );
    }

    fn deterministic_binding() -> TerrainQaBinding {
        let snapshot: SnapshotProvenance = serde_json::from_value(serde_json::json!({
            "workspace": vec![1_u8; 16],
            "source": vec![2_u8; 32],
            "revision": vec![3_u8; 32],
        }))
        .unwrap();
        TerrainQaBinding {
            snapshot,
            recipe_hash: ContentHash::new([4; 32]),
            input_hash: ContentHash::new([5; 32]),
            geometry_hash: ContentHash::new([6; 32]),
            topology_hash: ContentHash::new([7; 32]),
            artifact_hash: ContentHash::new([8; 32]),
            spatial_reference: SpatialReferenceProfile::new(
                32_647,
                5_703,
                SpatialAxes::EastingNorthingElevation,
                LinearUnit::Metre,
                LinearUnit::Metre,
                SpatialReferenceProvenance::CallerDeclaration,
            )
            .unwrap(),
        }
    }
}
