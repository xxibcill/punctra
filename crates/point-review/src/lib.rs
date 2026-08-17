//! Exact CPU review composition over an immutable Workspace Snapshot.
//!
//! A renderer may provide provisional Point identities and a host may provide
//! a screen rectangle, camera, and viewport. This crate confirms either input
//! through [`point_workspace`] and never treats resident display samples,
//! depth, or visibility as authoritative selection.
//!
//! The crate owns no GPU resources or window state. Hosts retain View-generation
//! validation, input gestures, renderer submission, and presentation policy.
//!
//! # Provisional picks and stale state
//!
//! A host validates the producing View generation before passing only the
//! provisional identity to exact confirmation. [`confirm_pick`] deliberately
//! cannot compare renderer generations: it confirms Source and pinned
//! [`Snapshot`] meaning, while the host owns the stale-View policy.
//!
//! ```no_run
//! use point_contracts::PointId;
//! use point_review::{ConfirmedPoint, ReviewError, ScreenReviewLimits, confirm_pick};
//! use point_workspace::Snapshot;
//!
//! fn confirm(
//!     snapshot: &Snapshot,
//!     provisional: PointId,
//! ) -> Result<ConfirmedPoint, ReviewError> {
//!     confirm_pick(snapshot, provisional, ScreenReviewLimits::default()).blocking_wait()
//! }
//! ```
//!
//! A completed [`Inspection`] stays exact for the captured Camera, Viewport,
//! and Snapshot Revision even if the displayed View or Workspace head later
//! advances. A host should label such a result stale rather than silently
//! transplant it to the new View or Revision.
//!
//! # Screen-through review and highlights
//!
//! [`screen_through`] scans every Source Point and tests its exact projected
//! center. GPU residency, occlusion, alpha, depth-buffer winners, and splat
//! radius do not participate. The resulting [`PointSet`] is complete-only and
//! provenance-bound. Renderer highlights must be derived by completely
//! consuming [`PointSet::ids`] under caller-selected
//! [`point_workspace::PointIdReadLimits`]
//! before one atomic highlight replacement:
//!
//! ```no_run
//! use point_contracts::PointId;
//! use point_review::{
//!     ReviewError, ScreenRect, ScreenReviewLimits, ScreenSelection, screen_through,
//! };
//! use point_workspace::{PointIdReadLimits, Snapshot};
//! use render_protocol::{Camera, Viewport};
//!
//! fn exact_highlights(
//!     snapshot: &Snapshot,
//!     camera: Camera,
//!     viewport: Viewport,
//! ) -> Result<Vec<PointId>, ReviewError> {
//!     let rect = ScreenRect::new(
//!         [0.0, 0.0],
//!         [f64::from(viewport.width()), f64::from(viewport.height())],
//!     )?;
//!     let selection = ScreenSelection::new(rect, camera, viewport)?;
//!     let inspection =
//!         screen_through(snapshot, selection, ScreenReviewLimits::default()).blocking_wait()?;
//!     let mut batches = inspection
//!         .points()
//!         .ids(PointIdReadLimits::default())?;
//!     let mut ids = Vec::new();
//!     while let Some(batch) = batches.next()? {
//!         ids.extend_from_slice(batch.ids());
//!     }
//!     Ok(ids)
//! }
//! ```
//!
//! # Limits, edits, and reconciliation
//!
//! [`ScreenReviewLimits`] composes independent [`PointRowLimits`] and
//! [`PointSetLimits`] with review-owned match-count and working-byte ceilings.
//! Highlight accumulation additionally needs
//! [`point_workspace::PointIdReadLimits`] plus a host retained-vector ceiling.
//! Renderer residency, commit, audit, open, and GPU limits remain separate
//! ledgers.
//!
//! A confirmed or selected Point Set may be submitted through
//! `point_workspace::CommitRequest::set_classification` with a caller-owned
//! operation identity. Stale Snapshot provenance is a definitive rejection,
//! not permission to rerun automatically. If publication is indeterminate,
//! retain the same operation identity, reopen the Workspace, and use
//! `point_workspace::Workspace::resolve_operation`; do not guess success or
//! retry under a new identity.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{cell::Cell, mem};

#[cfg(test)]
use std::sync::{Condvar, Mutex};

use foundation_runtime::{Job, OperationControl, ProgressPhase, ProgressSnapshot, RuntimeError};
use point_contracts::{ContentHash, PointId, PositionTransform};
use point_workspace::{
    PointQuery, PointRowLimits, PointSet, PointSetLimits, Snapshot, SnapshotProvenance,
    WorkspaceError,
};
use render_protocol::{Camera, CameraProjection, Viewport};
use thiserror::Error;

const GROWTH_FLOOR: usize = 1_024;
const CANCELLATION_STRIDE: usize = 4_096;
const DEFAULT_MAX_SCREEN_MATCHES: u64 = 1_000_000;
const DEFAULT_RETAINED_MATCH_BYTES: u64 = 128 * 1_024 * 1_024;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestPauseStage {
    Scan,
    Handoff,
    ConfirmationScan,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestPauseState {
    Disabled,
    Armed(TestPauseStage),
    Reached(TestPauseStage),
}

#[cfg(test)]
static TEST_PAUSE: (Mutex<TestPauseState>, Condvar) =
    (Mutex::new(TestPauseState::Disabled), Condvar::new());

#[cfg(test)]
static TEST_PAUSE_SERIAL: Mutex<()> = Mutex::new(());

/// An inclusive rectangle in top-left-origin physical pixel coordinates.
///
/// Coordinates describe continuous pixel boundaries, so the complete viewport
/// spans `[0, width]` by `[0, height]`. Construction normalizes endpoint order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    min: [f64; 2],
    max: [f64; 2],
}

impl ScreenRect {
    /// Creates a normalized rectangle from two finite endpoints.
    ///
    /// Zero-width and zero-height rectangles are valid and retain their
    /// inclusive boundary meaning.
    ///
    /// # Errors
    ///
    /// Returns [`ReviewError::NonFiniteScreenCoordinate`] when either endpoint
    /// contains NaN or infinity.
    pub fn new(first: [f64; 2], second: [f64; 2]) -> Result<Self, ReviewError> {
        validate_screen_endpoint("first", first)?;
        validate_screen_endpoint("second", second)?;
        Ok(Self {
            min: [first[0].min(second[0]), first[1].min(second[1])],
            max: [first[0].max(second[0]), first[1].max(second[1])],
        })
    }

    /// Returns the normalized inclusive minimum `[x, y]` boundary.
    #[must_use]
    pub const fn min(self) -> [f64; 2] {
        self.min
    }

    /// Returns the normalized inclusive maximum `[x, y]` boundary.
    #[must_use]
    pub const fn max(self) -> [f64; 2] {
        self.max
    }

    fn contains(self, pixel: [f64; 2]) -> bool {
        pixel[0] >= self.min[0]
            && pixel[0] <= self.max[0]
            && pixel[1] >= self.min[1]
            && pixel[1] <= self.max[1]
    }
}

/// One exact screen-through selection request at a caller-owned View state.
///
/// The optional classification predicate is evaluated against the effective
/// value at the Snapshot's pinned Revision after canonical CPU projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenSelection {
    rect: ScreenRect,
    camera: Camera,
    viewport: Viewport,
    classification: Option<u8>,
}

impl ScreenSelection {
    /// Binds a rectangle to one validated camera and physical viewport.
    ///
    /// The rectangle must remain inside the inclusive continuous viewport
    /// extent `[0, width]` by `[0, height]`.
    ///
    /// # Errors
    ///
    /// Returns [`ReviewError::ScreenCoordinateOutsideViewport`] when one
    /// normalized rectangle boundary is outside the viewport.
    pub fn new(rect: ScreenRect, camera: Camera, viewport: Viewport) -> Result<Self, ReviewError> {
        validate_rect_in_viewport(rect, viewport)?;
        Ok(Self {
            rect,
            camera,
            viewport,
            classification: None,
        })
    }

    /// Restricts review to one exact effective classification value.
    #[must_use]
    pub const fn classification_is(mut self, value: u8) -> Self {
        self.classification = Some(value);
        self
    }

    /// Returns the inclusive normalized screen rectangle.
    #[must_use]
    pub const fn rect(self) -> ScreenRect {
        self.rect
    }

    /// Returns the exact camera used by CPU projection.
    #[must_use]
    pub const fn camera(self) -> Camera {
        self.camera
    }

    /// Returns the physical viewport used by CPU projection.
    #[must_use]
    pub const fn viewport(self) -> Viewport {
        self.viewport
    }

    /// Returns the optional effective-classification equality predicate.
    #[must_use]
    pub const fn classification(self) -> Option<u8> {
        self.classification
    }
}

/// Hard ceilings for one complete exact screen-through review.
///
/// Point-row and Point-Set limits retain their own cumulative ledgers.
/// `max_working_bytes` is a conservative peak composition ceiling: while rows
/// are streamed it reserves the complete configured Point-row working ceiling
/// plus retained match-vector growth overlap; during Point-Set handoff it
/// reserves the retained match allocation plus the complete configured
/// Point-Set working ceiling. These are caller-selected accounting maxima, not
/// observed allocator or process-memory measurements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenReviewLimits {
    point_rows: PointRowLimits,
    point_set: PointSetLimits,
    max_screen_matches: u64,
    max_working_bytes: u64,
}

impl ScreenReviewLimits {
    /// Creates one composite limit set without hidden fallback or truncation.
    #[must_use]
    pub const fn new(
        point_rows: PointRowLimits,
        point_set: PointSetLimits,
        max_screen_matches: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            point_rows,
            point_set,
            max_screen_matches,
            max_working_bytes,
        }
    }

    /// Returns the complete exact Snapshot row-stream limits.
    #[must_use]
    pub const fn point_row_limits(self) -> PointRowLimits {
        self.point_rows
    }

    /// Returns the terminal exact Point Set limits.
    #[must_use]
    pub const fn point_set_limits(self) -> PointSetLimits {
        self.point_set
    }

    /// Returns the maximum exact Points accepted by the screen predicate.
    #[must_use]
    pub const fn max_screen_matches(self) -> u64 {
        self.max_screen_matches
    }

    /// Returns the conservative composition working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for ScreenReviewLimits {
    fn default() -> Self {
        let point_rows = PointRowLimits::default();
        let point_set = PointSetLimits::default();
        let component_peak = point_rows
            .max_working_bytes()
            .max(point_set.max_working_bytes());
        Self::new(
            point_rows,
            point_set,
            DEFAULT_MAX_SCREEN_MATCHES,
            component_peak.saturating_add(DEFAULT_RETAINED_MATCH_BYTES),
        )
    }
}

/// Complete-only facts for one exact inspection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectionSummary {
    provenance: SnapshotProvenance,
    selection: ScreenSelection,
    candidate_point_count: u64,
    examined_point_count: u64,
    exact_count: u64,
    point_id_hash: ContentHash,
    accounted_peak_working_bytes: u64,
}

impl InspectionSummary {
    /// Returns the exact Workspace, Source, and pinned Revision identity.
    #[must_use]
    pub const fn provenance(self) -> SnapshotProvenance {
        self.provenance
    }

    /// Returns the exact screen selection bound to this terminal result.
    #[must_use]
    pub const fn selection(self) -> ScreenSelection {
        self.selection
    }

    /// Returns conservative Source candidates read before exact predicates.
    #[must_use]
    pub const fn candidate_point_count(self) -> u64 {
        self.candidate_point_count
    }

    /// Returns every exact Source row tested by canonical CPU projection.
    #[must_use]
    pub const fn examined_point_count(self) -> u64 {
        self.examined_point_count
    }

    /// Returns the exact number of Points in the terminal Point Set.
    #[must_use]
    pub const fn exact_count(self) -> u64 {
        self.exact_count
    }

    /// Returns the canonical ordered Point-identity hash of the terminal set.
    #[must_use]
    pub const fn point_id_hash(self) -> ContentHash {
        self.point_id_hash
    }

    /// Returns the conservative algorithm-accounted working-byte high-water.
    ///
    /// This is the greatest successful composition charge evaluated by this
    /// completed review: the configured Point-row worker peak, each retained
    /// match-vector reallocation overlap using its actual post-reserve
    /// capacity, and the terminal retained match allocation plus configured
    /// Point-Set worker peak. It is bounded by
    /// [`ScreenReviewLimits::max_working_bytes`]. It is not an allocator,
    /// resident-set-size, or measured-heap observation.
    #[must_use]
    pub const fn accounted_peak_working_bytes(self) -> u64 {
        self.accounted_peak_working_bytes
    }
}

/// One terminal exact review result.
///
/// No value is published until both the Snapshot row stream (when applicable)
/// and exact explicit-identity Point Set selection complete successfully.
#[derive(Clone, Debug)]
pub struct Inspection {
    points: PointSet,
    summary: InspectionSummary,
}

/// One renderer-provided identity confirmed entirely from an exact Snapshot.
///
/// The Point Set contains exactly `point_id`. Ticks, transform, world position,
/// and effective classification all come from the same pinned Snapshot row;
/// no renderer position, depth, or Attribute value is accepted.
#[derive(Clone, Debug)]
pub struct ConfirmedPoint {
    point_id: PointId,
    ticks: [i64; 3],
    transform: PositionTransform,
    effective_classification: u8,
    provenance: SnapshotProvenance,
    points: PointSet,
}

impl ConfirmedPoint {
    /// Returns the confirmed canonical Source-aware Point identity.
    #[must_use]
    pub const fn point_id(&self) -> PointId {
        self.point_id
    }

    /// Returns the exact signed Source position ticks.
    #[must_use]
    pub const fn ticks(&self) -> [i64; 3] {
        self.ticks
    }

    /// Returns the verified Source position transform.
    #[must_use]
    pub const fn position_transform(&self) -> PositionTransform {
        self.transform
    }

    /// Returns the 64-bit world position derived from exact ticks and transform.
    #[must_use]
    pub fn world_position(&self) -> [f64; 3] {
        self.transform.world_f64(self.ticks)
    }

    /// Returns the exact effective classification at the pinned Revision.
    #[must_use]
    pub const fn effective_classification(&self) -> u8 {
        self.effective_classification
    }

    /// Returns the exact Workspace, Source, and pinned Revision identity.
    #[must_use]
    pub const fn provenance(&self) -> SnapshotProvenance {
        self.provenance
    }

    /// Returns the one-point exact Point Set suitable for a later edit.
    #[must_use]
    pub const fn points(&self) -> &PointSet {
        &self.points
    }
}

impl Inspection {
    /// Returns the immutable exact Point Set.
    #[must_use]
    pub const fn points(&self) -> &PointSet {
        &self.points
    }

    /// Returns complete-only provenance, request, count, and identity facts.
    #[must_use]
    pub const fn summary(&self) -> &InspectionSummary {
        &self.summary
    }

    /// Splits the terminal Point Set from its complete summary.
    #[must_use]
    pub fn into_parts(self) -> (PointSet, InspectionSummary) {
        (self.points, self.summary)
    }
}

/// Background exact screen review.
pub type ScreenReviewJob = Job<Inspection, ReviewError>;

/// Background exact confirmation of one provisional pick identity.
pub type PickConfirmationJob = Job<ConfirmedPoint, ReviewError>;

/// Selects every exact Snapshot Point whose projected center lies inside `selection`.
///
/// This is screen-through selection: depth occlusion and current GPU residency
/// never exclude a Point. Projection uses 64-bit world values and camera math.
#[must_use]
#[allow(clippy::large_types_passed_by_value)]
pub fn screen_through(
    snapshot: &Snapshot,
    selection: ScreenSelection,
    limits: ScreenReviewLimits,
) -> ScreenReviewJob {
    let snapshot = snapshot.clone();
    Job::spawn(move |control| run_screen_through(&snapshot, selection, &limits, &control))
}

/// Confirms one provisional Point identity through exact explicit-ID selection.
///
/// The host must validate that a renderer pick belongs to the intended View
/// generation before calling this helper. This function validates Source and
/// Snapshot meaning only.
#[must_use]
#[allow(clippy::large_types_passed_by_value)]
pub fn confirm_pick(
    snapshot: &Snapshot,
    point: PointId,
    limits: ScreenReviewLimits,
) -> PickConfirmationJob {
    let snapshot = snapshot.clone();
    Job::spawn(move |control| run_confirm_pick(&snapshot, point, &limits, &control))
}

/// Stable arithmetic stage for an unsupported exact CPU projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionStage {
    /// Exact world position minus the Camera eye.
    WorldMinusEye,
    /// Ordered dot products with the canonical Camera basis.
    BasisProjection,
    /// Perspective tangent and horizontal or vertical projection scale.
    PerspectiveScale,
    /// Orthographic horizontal or vertical projection scale.
    OrthographicScale,
    /// Division into normalized device coordinates.
    NormalizedCoordinates,
    /// Mapping normalized coordinates into physical pixel-edge space.
    PixelCoordinates,
}

/// Resource family owned by exact screen-review composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewResource {
    /// Exact Points accepted by the screen predicate.
    ScreenMatches,
    /// Accounted row, Point-Set, and retained-identity working memory.
    WorkingBytes,
}

/// A validation, projection, resource, runtime, or Workspace review failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReviewError {
    /// A screen endpoint contains NaN or infinity.
    #[error("{endpoint} screen endpoint axis {axis} must be finite")]
    NonFiniteScreenCoordinate {
        /// Stable endpoint name.
        endpoint: &'static str,
        /// Zero-based x/y axis.
        axis: usize,
    },
    /// A normalized rectangle boundary is outside its physical viewport.
    #[error(
        "screen rectangle {boundary} axis {axis} coordinate {coordinate} is outside 0..={maximum}"
    )]
    ScreenCoordinateOutsideViewport {
        /// Stable normalized rectangle boundary name.
        boundary: &'static str,
        /// Zero-based x/y axis.
        axis: usize,
        /// Rejected finite coordinate.
        coordinate: f64,
        /// Inclusive viewport maximum on this axis.
        maximum: f64,
    },
    /// Exact world-to-screen arithmetic produced a non-finite value.
    #[error("exact projection of Point {point:?} produced a non-finite value at {stage:?}")]
    NonFiniteProjection {
        /// Point whose exact world position could not be projected safely.
        point: PointId,
        /// Stable arithmetic stage that rejected the Point.
        stage: ProjectionStage,
    },
    /// Review-owned retained output exceeded a hard ceiling.
    #[error("screen review exceeded {resource:?}: required {required}, limit {allowed}")]
    ResourceLimit {
        /// Stable resource family.
        resource: ReviewResource,
        /// Minimum amount required.
        required: u64,
        /// Caller-selected hard ceiling.
        allowed: u64,
    },
    /// A Snapshot row stream ended without its complete terminal summary.
    #[error("exact Snapshot Point rows ended without a terminal summary")]
    MissingPointRowSummary,
    /// Independently completed exact stages disagreed on their result facts.
    #[error("exact review stages produced inconsistent terminal facts")]
    InconsistentResult,
    /// Exact Workspace selection, Source access, or persistence failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Runtime-neutral cancellation, progress, or worker handling failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

fn run_screen_through(
    snapshot: &Snapshot,
    selection: ScreenSelection,
    limits: &ScreenReviewLimits,
    control: &OperationControl,
) -> Result<Inspection, ReviewError> {
    control.check_cancelled()?;
    let mut working = WorkingAccounting::default();
    require_row_stream_capacity(limits, &mut working)?;
    let projector = Projector::new(selection);
    let mut rows = snapshot.point_rows(PointQuery::all(), limits.point_row_limits())?;
    let mut matches = Vec::new();
    let mut examined = 0_u64;

    #[cfg(test)]
    test_pause(TestPauseStage::Scan, control)?;

    while let Some(batch) = pull_rows(&mut rows, control)? {
        for row in 0..batch.len() {
            if row.is_multiple_of(CANCELLATION_STRIDE) {
                check_row_cancellation(&rows, control)?;
            }
            let point = batch.point_id(row).ok_or(ReviewError::InconsistentResult)?;
            let world = batch
                .positions()
                .world_f64(row)
                .ok_or(ReviewError::InconsistentResult)?;
            let is_inside = projector.includes(point, world)?;
            let effective_classification = batch
                .effective_classifications()
                .get(row)
                .copied()
                .ok_or(ReviewError::InconsistentResult)?;
            let classification_matches = selection
                .classification()
                .is_none_or(|expected| effective_classification == expected);
            if is_inside && classification_matches {
                push_match(&mut matches, point, limits, &mut working)?;
            }
        }
        examined = examined
            .checked_add(u64::try_from(batch.len()).unwrap_or(u64::MAX))
            .ok_or(ReviewError::ResourceLimit {
                resource: ReviewResource::ScreenMatches,
                required: u64::MAX,
                allowed: limits.max_screen_matches(),
            })?;
        control.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            examined,
            None,
        )?)?;
    }

    let row_summary = rows.summary().ok_or(ReviewError::MissingPointRowSummary)?;
    let provenance = *row_summary.provenance();
    let candidate_point_count = row_summary.candidate_point_count();
    let examined_point_count = row_summary.exact_count();
    if provenance != *snapshot.provenance() || examined != examined_point_count {
        return Err(ReviewError::InconsistentResult);
    }
    drop(rows);

    require_handoff_capacity(matches.capacity(), limits, &mut working)?;
    control.check_cancelled()?;

    #[cfg(test)]
    test_pause(TestPauseStage::Handoff, control)?;

    let expected_count = u64::try_from(matches.len()).unwrap_or(u64::MAX);
    let point_set = select_matches(snapshot, matches, limits, control)?;
    let inspection = finish_inspection(
        point_set,
        selection,
        provenance,
        candidate_point_count,
        examined_point_count,
        expected_count,
        working.peak_bytes(),
    )?;
    control.check_cancelled()?;
    control.complete_progress(examined_point_count)?;
    Ok(inspection)
}

fn run_confirm_pick(
    snapshot: &Snapshot,
    point: PointId,
    limits: &ScreenReviewLimits,
    control: &OperationControl,
) -> Result<ConfirmedPoint, ReviewError> {
    control.check_cancelled()?;
    require_confirmation_capacity(limits)?;
    let point_set = snapshot
        .select_point_ids([point], limits.point_set_limits())
        .blocking_wait_cancelled_by(&control.token())?;
    let metadata = *point_set.metadata();
    if metadata.provenance() != *snapshot.provenance() || metadata.exact_count() != 1 {
        return Err(ReviewError::InconsistentResult);
    }

    let (row, examined) = scan_confirmed_row(snapshot, point, limits.point_row_limits(), control)?;
    control.check_cancelled()?;
    control.complete_progress(examined)?;
    Ok(ConfirmedPoint {
        point_id: point,
        ticks: row.ticks,
        transform: row.transform,
        effective_classification: row.effective_classification,
        provenance: metadata.provenance(),
        points: point_set,
    })
}

#[derive(Clone, Copy)]
struct ConfirmedRow {
    ticks: [i64; 3],
    transform: PositionTransform,
    effective_classification: u8,
}

fn scan_confirmed_row(
    snapshot: &Snapshot,
    point: PointId,
    limits: PointRowLimits,
    control: &OperationControl,
) -> Result<(ConfirmedRow, u64), ReviewError> {
    let mut rows = snapshot.point_rows(PointQuery::all(), limits)?;
    let mut confirmed = None;
    let mut examined = 0_u64;

    #[cfg(test)]
    test_pause(TestPauseStage::ConfirmationScan, control)?;

    while let Some(batch) = pull_rows(&mut rows, control)? {
        for row in 0..batch.len() {
            if row.is_multiple_of(CANCELLATION_STRIDE) {
                check_row_cancellation(&rows, control)?;
            }
            if batch.point_id(row) == Some(point) {
                if confirmed.is_some() {
                    return Err(ReviewError::InconsistentResult);
                }
                let ticks = batch
                    .positions()
                    .ticks()
                    .get(row)
                    .copied()
                    .ok_or(ReviewError::InconsistentResult)?;
                let effective_classification = batch
                    .effective_classifications()
                    .get(row)
                    .copied()
                    .ok_or(ReviewError::InconsistentResult)?;
                confirmed = Some(ConfirmedRow {
                    ticks,
                    transform: batch.positions().transform(),
                    effective_classification,
                });
            }
        }
        examined = examined
            .checked_add(u64::try_from(batch.len()).unwrap_or(u64::MAX))
            .ok_or(ReviewError::InconsistentResult)?;
        control.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            examined,
            None,
        )?)?;
    }

    let summary = rows.summary().ok_or(ReviewError::MissingPointRowSummary)?;
    if summary.provenance() != snapshot.provenance() || summary.exact_count() != examined {
        return Err(ReviewError::InconsistentResult);
    }
    confirmed
        .map(|row| (row, examined))
        .ok_or(ReviewError::InconsistentResult)
}

fn pull_rows(
    rows: &mut point_workspace::SnapshotPointBatches,
    control: &OperationControl,
) -> Result<Option<point_workspace::SnapshotPointBatch>, ReviewError> {
    check_row_cancellation(rows, control)?;
    Ok(rows.next()?)
}

fn check_row_cancellation(
    rows: &point_workspace::SnapshotPointBatches,
    control: &OperationControl,
) -> Result<(), ReviewError> {
    if let Err(error) = control.check_cancelled() {
        rows.handle().cancel();
        return Err(error.into());
    }
    Ok(())
}

fn push_match(
    matches: &mut Vec<PointId>,
    point: PointId,
    limits: &ScreenReviewLimits,
    working: &mut WorkingAccounting,
) -> Result<(), ReviewError> {
    let next_count = u64::try_from(matches.len())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    if next_count > limits.max_screen_matches() {
        return Err(ReviewError::ResourceLimit {
            resource: ReviewResource::ScreenMatches,
            required: next_count,
            allowed: limits.max_screen_matches(),
        });
    }
    if matches.len() == matches.capacity() {
        grow_matches(matches, limits, working)?;
    }
    matches.push(point);
    Ok(())
}

fn grow_matches(
    matches: &mut Vec<PointId>,
    limits: &ScreenReviewLimits,
    working: &mut WorkingAccounting,
) -> Result<(), ReviewError> {
    let maximum_items = usize::try_from(limits.max_screen_matches()).unwrap_or(usize::MAX);
    let target = matches
        .capacity()
        .saturating_mul(2)
        .max(GROWTH_FLOOR)
        .min(maximum_items)
        .max(matches.len().saturating_add(1));
    let old_bytes = point_id_bytes(matches.capacity());
    let row_peak = limits.point_row_limits().max_working_bytes();
    require_review_working(
        row_peak
            .saturating_add(old_bytes)
            .saturating_add(point_id_bytes(target)),
        limits.max_working_bytes(),
    )?;
    matches
        .try_reserve_exact(target.saturating_sub(matches.len()))
        .map_err(|_| ReviewError::ResourceLimit {
            resource: ReviewResource::WorkingBytes,
            required: row_peak
                .saturating_add(old_bytes)
                .saturating_add(point_id_bytes(target)),
            allowed: limits.max_working_bytes(),
        })?;
    working.observe(
        row_peak
            .saturating_add(old_bytes)
            .saturating_add(point_id_bytes(matches.capacity())),
        limits.max_working_bytes(),
    )
}

fn require_row_stream_capacity(
    limits: &ScreenReviewLimits,
    working: &mut WorkingAccounting,
) -> Result<(), ReviewError> {
    working.observe(
        limits.point_row_limits().max_working_bytes(),
        limits.max_working_bytes(),
    )
}

fn require_confirmation_capacity(limits: &ScreenReviewLimits) -> Result<(), ReviewError> {
    require_review_working(
        limits
            .point_row_limits()
            .max_working_bytes()
            .max(limits.point_set_limits().max_working_bytes()),
        limits.max_working_bytes(),
    )
}

fn require_handoff_capacity(
    match_capacity: usize,
    limits: &ScreenReviewLimits,
    working: &mut WorkingAccounting,
) -> Result<(), ReviewError> {
    working.observe(
        point_id_bytes(match_capacity)
            .saturating_add(limits.point_set_limits().max_working_bytes()),
        limits.max_working_bytes(),
    )
}

fn select_matches(
    snapshot: &Snapshot,
    matches: Vec<PointId>,
    limits: &ScreenReviewLimits,
    control: &OperationControl,
) -> Result<PointSet, ReviewError> {
    let cancellation_seen = Cell::new(false);
    let ids = CancellablePointIds::new(matches, control, &cancellation_seen);
    let point_set = snapshot.select_point_ids(ids, limits.point_set_limits());
    if cancellation_seen.get() {
        point_set.handle().cancel();
        return Err(RuntimeError::Cancelled.into());
    }
    if let Err(error) = control.check_cancelled() {
        point_set.handle().cancel();
        return Err(error.into());
    }
    Ok(point_set.blocking_wait_cancelled_by(&control.token())?)
}

struct CancellablePointIds<'control> {
    ids: std::vec::IntoIter<PointId>,
    control: &'control OperationControl,
    cancellation_seen: &'control Cell<bool>,
}

impl<'control> CancellablePointIds<'control> {
    fn new(
        ids: Vec<PointId>,
        control: &'control OperationControl,
        cancellation_seen: &'control Cell<bool>,
    ) -> Self {
        Self {
            ids: ids.into_iter(),
            control,
            cancellation_seen,
        }
    }
}

impl Iterator for CancellablePointIds<'_> {
    type Item = PointId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cancellation_seen.get() {
            return None;
        }
        if self.control.check_cancelled().is_err() {
            self.cancellation_seen.set(true);
            return None;
        }
        self.ids.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.ids.len()))
    }
}

fn require_review_working(required: u64, allowed: u64) -> Result<(), ReviewError> {
    if required > allowed {
        return Err(ReviewError::ResourceLimit {
            resource: ReviewResource::WorkingBytes,
            required,
            allowed,
        });
    }
    Ok(())
}

#[derive(Default)]
struct WorkingAccounting {
    peak_bytes: u64,
}

impl WorkingAccounting {
    fn observe(&mut self, required: u64, allowed: u64) -> Result<(), ReviewError> {
        require_review_working(required, allowed)?;
        self.peak_bytes = self.peak_bytes.max(required);
        Ok(())
    }

    const fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }
}

fn point_id_bytes(items: usize) -> u64 {
    u64::try_from(items)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<PointId>()).unwrap_or(u64::MAX))
}

fn finish_inspection(
    points: PointSet,
    selection: ScreenSelection,
    provenance: SnapshotProvenance,
    candidate_point_count: u64,
    examined_point_count: u64,
    expected_count: u64,
    accounted_peak_working_bytes: u64,
) -> Result<Inspection, ReviewError> {
    let metadata = *points.metadata();
    if metadata.provenance() != provenance || metadata.exact_count() != expected_count {
        return Err(ReviewError::InconsistentResult);
    }
    Ok(Inspection {
        points,
        summary: InspectionSummary {
            provenance,
            selection,
            candidate_point_count,
            examined_point_count,
            exact_count: metadata.exact_count(),
            point_id_hash: metadata.point_id_hash(),
            accounted_peak_working_bytes,
        },
    })
}

fn validate_screen_endpoint(endpoint: &'static str, value: [f64; 2]) -> Result<(), ReviewError> {
    for (axis, coordinate) in value.into_iter().enumerate() {
        if !coordinate.is_finite() {
            return Err(ReviewError::NonFiniteScreenCoordinate { endpoint, axis });
        }
    }
    Ok(())
}

fn validate_rect_in_viewport(rect: ScreenRect, viewport: Viewport) -> Result<(), ReviewError> {
    let maxima = [f64::from(viewport.width()), f64::from(viewport.height())];
    for (boundary, coordinates) in [("minimum", rect.min()), ("maximum", rect.max())] {
        for axis in 0..2 {
            if coordinates[axis] < 0.0 || coordinates[axis] > maxima[axis] {
                return Err(ReviewError::ScreenCoordinateOutsideViewport {
                    boundary,
                    axis,
                    coordinate: coordinates[axis],
                    maximum: maxima[axis],
                });
            }
        }
    }
    Ok(())
}

struct Projector {
    selection: ScreenSelection,
    eye: [f64; 3],
    forward: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    aspect: f64,
}

impl Projector {
    fn new(selection: ScreenSelection) -> Self {
        let camera = selection.camera();
        let basis = camera.world_basis();
        let viewport = selection.viewport();
        Self {
            selection,
            eye: camera.eye(),
            forward: basis.forward(),
            right: basis.right(),
            up: basis.up(),
            aspect: f64::from(viewport.width()) / f64::from(viewport.height()),
        }
    }

    fn includes(&self, point: PointId, world: [f64; 3]) -> Result<bool, ReviewError> {
        let relative = subtract(world, self.eye);
        if !finite3(relative) {
            return Err(ReviewError::NonFiniteProjection {
                point,
                stage: ProjectionStage::WorldMinusEye,
            });
        }
        let depth = dot(self.forward, relative);
        let horizontal = dot(self.right, relative);
        let vertical = dot(self.up, relative);
        if !depth.is_finite() || !horizontal.is_finite() || !vertical.is_finite() {
            return Err(ReviewError::NonFiniteProjection {
                point,
                stage: ProjectionStage::BasisProjection,
            });
        }

        let camera = self.selection.camera();
        if matches!(camera.projection(), CameraProjection::Perspective { .. }) && depth <= 0.0 {
            return Ok(false);
        }
        if depth < f64::from(camera.near_distance()) || depth > f64::from(camera.far_distance()) {
            return Ok(false);
        }
        let ndc = match camera.projection() {
            CameraProjection::Perspective {
                vertical_field_of_view_radians,
            } => {
                let half_vertical_tangent = (f64::from(vertical_field_of_view_radians) * 0.5).tan();
                let vertical_scale = depth * half_vertical_tangent;
                let horizontal_scale = vertical_scale * self.aspect;
                if !half_vertical_tangent.is_finite()
                    || !vertical_scale.is_finite()
                    || !horizontal_scale.is_finite()
                    || half_vertical_tangent <= 0.0
                    || vertical_scale <= 0.0
                    || horizontal_scale <= 0.0
                {
                    return Err(ReviewError::NonFiniteProjection {
                        point,
                        stage: ProjectionStage::PerspectiveScale,
                    });
                }
                [horizontal / horizontal_scale, vertical / vertical_scale]
            }
            CameraProjection::Orthographic {
                vertical_world_height,
            } => {
                let half_vertical = vertical_world_height * 0.5;
                let half_horizontal = half_vertical * self.aspect;
                if !half_vertical.is_finite()
                    || !half_horizontal.is_finite()
                    || half_vertical <= 0.0
                    || half_horizontal <= 0.0
                {
                    return Err(ReviewError::NonFiniteProjection {
                        point,
                        stage: ProjectionStage::OrthographicScale,
                    });
                }
                [horizontal / half_horizontal, vertical / half_vertical]
            }
        };
        if !ndc[0].is_finite() || !ndc[1].is_finite() {
            return Err(ReviewError::NonFiniteProjection {
                point,
                stage: ProjectionStage::NormalizedCoordinates,
            });
        }
        if ndc[0] < -1.0 || ndc[0] > 1.0 || ndc[1] < -1.0 || ndc[1] > 1.0 {
            return Ok(false);
        }

        let viewport = self.selection.viewport();
        let pixel = [
            (ndc[0] + 1.0) * f64::from(viewport.width()) / 2.0,
            (1.0 - ndc[1]) * f64::from(viewport.height()) / 2.0,
        ];
        if !pixel[0].is_finite() || !pixel[1].is_finite() {
            return Err(ReviewError::NonFiniteProjection {
                point,
                stage: ProjectionStage::PixelCoordinates,
            });
        }
        Ok(self.selection.rect().contains(pixel))
    }
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn finite3(value: [f64; 3]) -> bool {
    value.into_iter().all(f64::is_finite)
}

#[cfg(test)]
fn test_pause(stage: TestPauseStage, control: &OperationControl) -> Result<(), ReviewError> {
    let (lock, wake) = &TEST_PAUSE;
    let mut pause_state = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *pause_state != TestPauseState::Armed(stage) {
        return Ok(());
    }
    *pause_state = TestPauseState::Reached(stage);
    wake.notify_all();
    while !control.token().is_cancelled() {
        pause_state = wake
            .wait(pause_state)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    *pause_state = TestPauseState::Disabled;
    wake.notify_all();
    Err(RuntimeError::Cancelled.into())
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use point_contracts::{
        AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
        AttributeValues, CoordinateReference, PositionTransform, SourceId,
    };
    use point_index::{PrepareLimits, prepare};
    use point_workspace::{OpenLimits, WorkspaceSchema, create};
    use render_protocol::{Camera, Viewport};
    use source_memory::MemorySource;

    use super::*;

    const CLASSIFICATION: u32 = 101;

    #[test]
    fn point_set_handoff_iterator_stops_at_parent_cancellation() {
        let source = SourceId::new([0x77; 32]);
        let control = OperationControl::new();
        let cancellation_seen = Cell::new(false);
        let mut ids = CancellablePointIds::new(
            vec![PointId::new(source, 0), PointId::new(source, 1)],
            &control,
            &cancellation_seen,
        );

        assert_eq!(ids.next(), Some(PointId::new(source, 0)));
        control.cancel();
        assert_eq!(ids.next(), None);
        assert_eq!(ids.next(), None);
        assert!(cancellation_seen.get());
    }

    #[test]
    fn cancellation_is_observed_at_mid_scan_and_mid_handoff_checkpoints() {
        let _serial = TEST_PAUSE_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = fixture();
        let snapshot = fixture.workspace.head();
        let selection = full_selection();
        for stage in [TestPauseStage::Scan, TestPauseStage::Handoff] {
            let _guard = TestPauseGuard::arm(stage);
            let job = screen_through(&snapshot, selection, ScreenReviewLimits::default());
            wait_for_pause(stage);
            job.handle().cancel();
            TEST_PAUSE.1.notify_all();
            assert!(matches!(
                job.blocking_wait(),
                Err(ReviewError::Runtime(RuntimeError::Cancelled))
            ));
        }
    }

    #[test]
    fn confirmation_cancellation_is_observed_after_exact_point_set_selection() {
        let _serial = TEST_PAUSE_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = fixture();
        let snapshot = fixture.workspace.head();
        let _guard = TestPauseGuard::arm(TestPauseStage::ConfirmationScan);
        let job = confirm_pick(
            &snapshot,
            PointId::new(fixture.source, 0),
            ScreenReviewLimits::default(),
        );
        wait_for_pause(TestPauseStage::ConfirmationScan);
        job.handle().cancel();
        TEST_PAUSE.1.notify_all();
        assert!(matches!(
            job.blocking_wait(),
            Err(ReviewError::Runtime(RuntimeError::Cancelled))
        ));
    }

    struct TestPauseGuard;

    impl TestPauseGuard {
        fn arm(stage: TestPauseStage) -> Self {
            let mut pause_state = TEST_PAUSE
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(*pause_state, TestPauseState::Disabled);
            *pause_state = TestPauseState::Armed(stage);
            Self
        }
    }

    impl Drop for TestPauseGuard {
        fn drop(&mut self) {
            let mut pause_state = TEST_PAUSE
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *pause_state = TestPauseState::Disabled;
            TEST_PAUSE.1.notify_all();
        }
    }

    fn wait_for_pause(stage: TestPauseStage) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut pause_state = TEST_PAUSE
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *pause_state != TestPauseState::Reached(stage) {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .expect("review reached the deterministic cancellation checkpoint");
            let (next, timeout) = TEST_PAUSE
                .1
                .wait_timeout(pause_state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(!timeout.timed_out(), "review checkpoint timed out");
            pause_state = next;
        }
        drop(pause_state);
        thread::yield_now();
    }

    struct TestFixture {
        workspace: point_workspace::Workspace,
        source: SourceId,
        _directory: tempfile::TempDir,
    }

    fn fixture() -> TestFixture {
        let directory = tempfile::tempdir().unwrap();
        let definition = AttributeDefinition::new(
            AttributeId::new(CLASSIFICATION).unwrap(),
            "classification",
            AttributeDataType::U8,
        )
        .unwrap();
        let columns = AttributeColumns::new(
            vec![AttributeColumn::new(definition, AttributeValues::u8(vec![2; 64])).unwrap()],
            64,
        )
        .unwrap();
        let ticks = (0_i64..64).map(|x| [x, 0, -5]).collect::<Vec<_>>();
        let memory = MemorySource::from_columns(
            PositionTransform::new([0.0; 3], [1.0; 3]).unwrap(),
            CoordinateReference::Unknown,
            ticks,
            columns,
        )
        .unwrap();
        let source = source_memory::open(memory).blocking_wait().unwrap();
        let source_id = source.identity();
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        let workspace = create(
            directory.path().join("fixture.pcw"),
            index,
            WorkspaceSchema::new(AttributeId::new(CLASSIFICATION).unwrap()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        TestFixture {
            workspace,
            source: source_id,
            _directory: directory,
        }
    }

    fn full_selection() -> ScreenSelection {
        let camera = Camera::perspective(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            std::f32::consts::FRAC_PI_2,
            1.0,
            10.0,
        )
        .unwrap();
        ScreenSelection::new(
            ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
            camera,
            Viewport::new(100, 100).unwrap(),
        )
        .unwrap()
    }
}
