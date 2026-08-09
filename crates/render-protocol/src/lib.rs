//! Renderer-neutral contracts for progressive point-cloud display.
//!
//! The crate owns no GPU resources. It defines validated, owned values that a
//! producer can send to a renderer without exposing renderer implementation
//! details.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

/// Estimated GPU bytes for one point in the protocol's residency model.
///
/// The model matches the v0.1 fixed GPU vertex: 12 bytes for the three `f32`
/// relative coordinates, four bytes for point size, four bytes for RGBA8
/// color, four bytes for flags, a four-byte renderer pick token, and four bytes
/// of alignment padding. Point size, flags, and pick token are
/// renderer-populated in v0.1. Per-batch metadata, allocation padding outside
/// the vertex, render targets, and renderer bookkeeping are not protocol
/// residency.
pub const ESTIMATED_GPU_BYTES_PER_POINT: u64 = 32;

/// Identifies a caller-owned view.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ViewId(u64);

impl ViewId {
    /// Creates an identifier from a caller-selected value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the caller-selected value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Identifies one caller-owned point for highlighting and picking.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PointId(u64);

impl PointId {
    /// Creates an identifier from a caller-selected value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the caller-selected value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Identifies a replaceable point batch within one view generation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BatchKey(u64);

impl BatchKey {
    /// Creates an identifier from a caller-selected value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the caller-selected value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Orders replacements of one [`BatchKey`] within a view generation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BatchVersion(u64);

impl BatchVersion {
    /// Creates a version from a caller-selected value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the caller-selected value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Names one generation of one view.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameKey {
    view: ViewId,
    generation: u64,
}

impl FrameKey {
    /// Creates a frame key.
    #[must_use]
    pub const fn new(view: ViewId, generation: u64) -> Self {
        Self { view, generation }
    }

    /// Returns the view identity.
    #[must_use]
    pub const fn view(self) -> ViewId {
        self.view
    }

    /// Returns the generation within the view.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// One renderer-neutral, origin-relative display point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderPoint {
    relative_position: [f32; 3],
    color: [u8; 4],
    point_id: PointId,
}

impl RenderPoint {
    /// Creates a point after validating that every position component is finite.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::NonFiniteRelativePosition`] for the first
    /// non-finite component.
    pub fn new(
        relative_position: [f32; 3],
        color: [u8; 4],
        point_id: PointId,
    ) -> Result<Self, ProtocolError> {
        if let Some(axis) = first_non_finite_axis(&relative_position) {
            return Err(ProtocolError::NonFiniteRelativePosition { axis });
        }

        Ok(Self {
            relative_position,
            color,
            point_id,
        })
    }

    /// Returns the position relative to its batch's world origin.
    #[must_use]
    pub const fn relative_position(&self) -> [f32; 3] {
        self.relative_position
    }

    /// Returns the point's linear RGBA8 color.
    ///
    /// An alpha value of zero excludes the point from drawing and picking.
    /// Nonzero alpha is blended while the splat participates in depth testing.
    #[must_use]
    pub const fn color(&self) -> [u8; 4] {
        self.color
    }

    /// Returns the caller's stable point identity.
    #[must_use]
    pub const fn point_id(&self) -> PointId {
        self.point_id
    }
}

/// A non-empty, owned group of points for one frame and batch version.
#[derive(Clone, Debug, PartialEq)]
pub struct PointBatch {
    frame: FrameKey,
    key: BatchKey,
    version: BatchVersion,
    world_origin: [f64; 3],
    points: Vec<RenderPoint>,
    point_count: u64,
    estimated_gpu_bytes: u64,
}

impl PointBatch {
    /// Creates a validated point batch.
    ///
    /// # Errors
    ///
    /// Returns an error when the world origin is non-finite, the batch is
    /// empty, or its size cannot be represented by the residency model.
    pub fn new(
        frame: FrameKey,
        key: BatchKey,
        version: BatchVersion,
        world_origin: [f64; 3],
        points: Vec<RenderPoint>,
    ) -> Result<Self, ProtocolError> {
        if let Some(axis) = first_non_finite_axis(&world_origin) {
            return Err(ProtocolError::NonFiniteWorldOrigin { axis });
        }
        if points.is_empty() {
            return Err(ProtocolError::EmptyPointBatch);
        }

        let point_count = u64::try_from(points.len()).map_err(|_| ProtocolError::SizeOverflow)?;
        let estimated_gpu_bytes = point_count
            .checked_mul(ESTIMATED_GPU_BYTES_PER_POINT)
            .ok_or(ProtocolError::SizeOverflow)?;

        Ok(Self {
            frame,
            key,
            version,
            world_origin,
            points,
            point_count,
            estimated_gpu_bytes,
        })
    }

    /// Returns the frame this batch belongs to.
    #[must_use]
    pub const fn frame(&self) -> FrameKey {
        self.frame
    }

    /// Returns the stable batch key.
    #[must_use]
    pub const fn key(&self) -> BatchKey {
        self.key
    }

    /// Returns the batch version.
    #[must_use]
    pub const fn version(&self) -> BatchVersion {
        self.version
    }

    /// Returns the finite 64-bit world origin.
    #[must_use]
    pub const fn world_origin(&self) -> [f64; 3] {
        self.world_origin
    }

    /// Borrows the points in caller order.
    #[must_use]
    pub fn points(&self) -> &[RenderPoint] {
        &self.points
    }

    /// Returns the number of points.
    #[must_use]
    pub const fn point_count(&self) -> u64 {
        self.point_count
    }

    /// Returns this batch's byte cost under the protocol residency model.
    #[must_use]
    pub const fn estimated_gpu_bytes(&self) -> u64 {
        self.estimated_gpu_bytes
    }
}

/// One complete logical update to renderer state.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderUpdate {
    /// Begins a generation and clears all state from the previous active frame.
    Reset {
        /// The generation to begin.
        frame: FrameKey,
    },
    /// Inserts a new batch or replaces an older version of the same batch key.
    Upsert {
        /// The complete replacement batch.
        batch: PointBatch,
    },
    /// Removes a batch only if its resident version matches.
    Remove {
        /// The frame the removal belongs to.
        frame: FrameKey,
        /// The batch to remove.
        key: BatchKey,
        /// The version the caller expects to be resident.
        expected_version: BatchVersion,
    },
    /// Replaces the complete set of highlighted caller point identities.
    SetHighlights {
        /// The frame the highlight set belongs to.
        frame: FrameKey,
        /// The complete highlight set. An empty vector clears highlighting.
        point_ids: Vec<PointId>,
    },
}

impl RenderUpdate {
    /// Returns the frame this update belongs to.
    #[must_use]
    pub const fn frame(&self) -> FrameKey {
        match self {
            Self::Reset { frame }
            | Self::Remove { frame, .. }
            | Self::SetHighlights { frame, .. } => *frame,
            Self::Upsert { batch } => batch.frame(),
        }
    }
}

/// Hard protocol residency limits selected by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderLimits {
    estimated_gpu_bytes: u64,
    points: u64,
    batches: u64,
}

impl RenderLimits {
    /// Creates hard limits. Zero is valid and permits an empty generation only.
    #[must_use]
    pub const fn new(max_estimated_gpu_bytes: u64, max_points: u64, max_batches: u64) -> Self {
        Self {
            estimated_gpu_bytes: max_estimated_gpu_bytes,
            points: max_points,
            batches: max_batches,
        }
    }

    /// Returns the maximum estimated GPU bytes.
    #[must_use]
    pub const fn max_estimated_gpu_bytes(self) -> u64 {
        self.estimated_gpu_bytes
    }

    /// Returns the maximum resident point count.
    #[must_use]
    pub const fn max_points(self) -> u64 {
        self.points
    }

    /// Returns the maximum resident batch count.
    #[must_use]
    pub const fn max_batches(self) -> u64 {
        self.batches
    }
}

/// Observable aggregate residency for the active generation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResidentStats {
    batch_count: u64,
    point_count: u64,
    estimated_gpu_bytes: u64,
}

impl ResidentStats {
    /// Returns the number of resident batches.
    #[must_use]
    pub const fn batch_count(self) -> u64 {
        self.batch_count
    }

    /// Returns the number of resident points.
    #[must_use]
    pub const fn point_count(self) -> u64 {
        self.point_count
    }

    /// Returns resident bytes under the documented protocol byte model.
    #[must_use]
    pub const fn estimated_gpu_bytes(self) -> u64 {
        self.estimated_gpu_bytes
    }
}

/// Observable metadata for one resident batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResidentBatch {
    key: BatchKey,
    version: BatchVersion,
    point_count: u64,
    estimated_gpu_bytes: u64,
}

impl ResidentBatch {
    /// Returns the batch key.
    #[must_use]
    pub const fn key(self) -> BatchKey {
        self.key
    }

    /// Returns the resident batch version.
    #[must_use]
    pub const fn version(self) -> BatchVersion {
        self.version
    }

    /// Returns the number of points in the batch.
    #[must_use]
    pub const fn point_count(self) -> u64 {
        self.point_count
    }

    /// Returns this batch's bytes under the protocol residency model.
    #[must_use]
    pub const fn estimated_gpu_bytes(self) -> u64 {
        self.estimated_gpu_bytes
    }
}

/// The observable effect category of an accepted update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateKind {
    /// A generation was reset.
    Reset,
    /// A previously unseen batch key became resident.
    BatchInserted,
    /// A resident batch was replaced by a newer version.
    BatchReplaced,
    /// A resident batch was removed.
    BatchRemoved,
    /// The complete highlight set changed.
    HighlightsSet,
}

/// Observable accounting returned after one accepted update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateReport {
    kind: UpdateKind,
    frame: FrameKey,
    resident: ResidentStats,
    uploaded_points: u64,
    uploaded_bytes: u64,
    removed_points: u64,
    removed_bytes: u64,
    highlight_count: u64,
}

impl UpdateReport {
    /// Returns the kind of accepted update.
    #[must_use]
    pub const fn kind(self) -> UpdateKind {
        self.kind
    }

    /// Returns the active frame after the update.
    #[must_use]
    pub const fn frame(self) -> FrameKey {
        self.frame
    }

    /// Returns aggregate residency after the update.
    #[must_use]
    pub const fn resident(self) -> ResidentStats {
        self.resident
    }

    /// Returns the number of points supplied by this update.
    #[must_use]
    pub const fn uploaded_points(self) -> u64 {
        self.uploaded_points
    }

    /// Returns supplied bytes under the protocol residency model.
    #[must_use]
    pub const fn uploaded_bytes(self) -> u64 {
        self.uploaded_bytes
    }

    /// Returns the number of points made non-resident by this update.
    #[must_use]
    pub const fn removed_points(self) -> u64 {
        self.removed_points
    }

    /// Returns removed bytes under the protocol residency model.
    #[must_use]
    pub const fn removed_bytes(self) -> u64 {
        self.removed_bytes
    }

    /// Returns the number of distinct highlighted point identities afterward.
    #[must_use]
    pub const fn highlight_count(self) -> u64 {
        self.highlight_count
    }
}

/// An immutable observable snapshot of protocol state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RenderSnapshot {
    active_frame: Option<FrameKey>,
    resident: ResidentStats,
    batches: Vec<ResidentBatch>,
    highlights: Vec<PointId>,
}

impl RenderSnapshot {
    /// Returns the active frame, or `None` before the first reset.
    #[must_use]
    pub const fn active_frame(&self) -> Option<FrameKey> {
        self.active_frame
    }

    /// Returns aggregate residency.
    #[must_use]
    pub const fn resident(&self) -> ResidentStats {
        self.resident
    }

    /// Returns resident batches in ascending [`BatchKey`] order.
    #[must_use]
    pub fn batches(&self) -> &[ResidentBatch] {
        &self.batches
    }

    /// Returns distinct highlights in ascending [`PointId`] order.
    #[must_use]
    pub fn highlights(&self) -> &[PointId] {
        &self.highlights
    }
}

/// Identifies the hard limit exceeded by an upsert.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResidentResource {
    /// Estimated GPU residency bytes.
    EstimatedGpuBytes,
    /// Resident points.
    Points,
    /// Resident batches.
    Batches,
}

/// CPU reference state for validating protocol transitions and accounting.
///
/// Every rejected update leaves the model byte-for-byte observably unchanged.
#[derive(Clone, Debug)]
pub struct RenderStateModel {
    limits: RenderLimits,
    active_frame: Option<FrameKey>,
    last_generations: BTreeMap<ViewId, u64>,
    batches: BTreeMap<BatchKey, ResidentBatch>,
    latest_versions: BTreeMap<BatchKey, BatchVersion>,
    highlights: BTreeSet<PointId>,
    resident: ResidentStats,
}

impl RenderStateModel {
    /// Creates an empty model with caller-selected hard residency limits.
    #[must_use]
    pub fn new(limits: RenderLimits) -> Self {
        Self {
            limits,
            active_frame: None,
            last_generations: BTreeMap::new(),
            batches: BTreeMap::new(),
            latest_versions: BTreeMap::new(),
            highlights: BTreeSet::new(),
            resident: ResidentStats::default(),
        }
    }

    /// Returns the configured hard residency limits.
    #[must_use]
    pub const fn limits(&self) -> RenderLimits {
        self.limits
    }

    /// Applies one complete update atomically.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] when the update does not belong to a valid
    /// active generation, violates batch-version ordering, fails a conditional
    /// removal, overflows accounting, or exceeds a hard residency limit.
    pub fn apply(&mut self, update: &RenderUpdate) -> Result<UpdateReport, ProtocolError> {
        match update {
            RenderUpdate::Reset { frame } => self.apply_reset(*frame),
            RenderUpdate::Upsert { batch } => self.apply_upsert(batch),
            RenderUpdate::Remove {
                frame,
                key,
                expected_version,
            } => self.apply_remove(*frame, *key, *expected_version),
            RenderUpdate::SetHighlights { frame, point_ids } => {
                self.apply_highlights(*frame, point_ids)
            }
        }
    }

    /// Captures all caller-observable state in deterministic key order.
    #[must_use]
    pub fn snapshot(&self) -> RenderSnapshot {
        RenderSnapshot {
            active_frame: self.active_frame,
            resident: self.resident,
            batches: self.batches.values().copied().collect(),
            highlights: self.highlights.iter().copied().collect(),
        }
    }

    fn apply_reset(&mut self, frame: FrameKey) -> Result<UpdateReport, ProtocolError> {
        self.validate_reset(frame)?;
        let removed = self.resident;

        self.active_frame = Some(frame);
        self.last_generations
            .insert(frame.view(), frame.generation());
        self.batches.clear();
        self.latest_versions.clear();
        self.highlights.clear();
        self.resident = ResidentStats::default();

        Ok(self.report(
            UpdateKind::Reset,
            0,
            0,
            removed.point_count,
            removed.estimated_gpu_bytes,
        ))
    }

    fn validate_reset(&self, frame: FrameKey) -> Result<(), ProtocolError> {
        let Some(last_generation) = self.last_generations.get(&frame.view()).copied() else {
            return Ok(());
        };
        if frame.generation() < last_generation {
            return Err(ProtocolError::StaleGeneration {
                view: frame.view(),
                last_generation,
                received_generation: frame.generation(),
            });
        }
        if frame.generation() == last_generation {
            return Err(ProtocolError::GenerationAlreadyStarted { frame });
        }
        Ok(())
    }

    fn apply_upsert(&mut self, batch: &PointBatch) -> Result<UpdateReport, ProtocolError> {
        self.require_active_frame(batch.frame())?;
        self.require_increasing_version(batch)?;

        let replaced = self.batches.get(&batch.key()).copied();
        let next_resident = self.residency_after_upsert(batch, replaced)?;
        self.enforce_limits(next_resident)?;

        let kind = if replaced.is_some() {
            UpdateKind::BatchReplaced
        } else {
            UpdateKind::BatchInserted
        };
        let removed_points = replaced.map_or(0, ResidentBatch::point_count);
        let removed_bytes = replaced.map_or(0, ResidentBatch::estimated_gpu_bytes);
        let resident_batch = ResidentBatch {
            key: batch.key(),
            version: batch.version(),
            point_count: batch.point_count(),
            estimated_gpu_bytes: batch.estimated_gpu_bytes(),
        };

        self.batches.insert(batch.key(), resident_batch);
        self.latest_versions.insert(batch.key(), batch.version());
        self.resident = next_resident;

        Ok(self.report(
            kind,
            batch.point_count(),
            batch.estimated_gpu_bytes(),
            removed_points,
            removed_bytes,
        ))
    }

    fn require_increasing_version(&self, batch: &PointBatch) -> Result<(), ProtocolError> {
        if let Some(previous) = self.latest_versions.get(&batch.key()).copied()
            && batch.version() <= previous
        {
            return Err(ProtocolError::BatchVersionNotIncreasing {
                key: batch.key(),
                previous,
                received: batch.version(),
            });
        }
        Ok(())
    }

    fn residency_after_upsert(
        &self,
        batch: &PointBatch,
        replaced: Option<ResidentBatch>,
    ) -> Result<ResidentStats, ProtocolError> {
        let old_points = replaced.map_or(0, ResidentBatch::point_count);
        let old_bytes = replaced.map_or(0, ResidentBatch::estimated_gpu_bytes);
        let added_batches = u64::from(replaced.is_none());

        Ok(ResidentStats {
            batch_count: self
                .resident
                .batch_count
                .checked_add(added_batches)
                .ok_or(ProtocolError::SizeOverflow)?,
            point_count: replace_count(self.resident.point_count, old_points, batch.point_count())?,
            estimated_gpu_bytes: replace_count(
                self.resident.estimated_gpu_bytes,
                old_bytes,
                batch.estimated_gpu_bytes(),
            )?,
        })
    }

    fn enforce_limits(&self, attempted: ResidentStats) -> Result<(), ProtocolError> {
        enforce_limit(
            ResidentResource::EstimatedGpuBytes,
            attempted.estimated_gpu_bytes,
            self.limits.estimated_gpu_bytes,
        )?;
        enforce_limit(
            ResidentResource::Points,
            attempted.point_count,
            self.limits.points,
        )?;
        enforce_limit(
            ResidentResource::Batches,
            attempted.batch_count,
            self.limits.batches,
        )
    }

    fn apply_remove(
        &mut self,
        frame: FrameKey,
        key: BatchKey,
        expected_version: BatchVersion,
    ) -> Result<UpdateReport, ProtocolError> {
        self.require_active_frame(frame)?;
        let resident = self
            .batches
            .get(&key)
            .copied()
            .ok_or(ProtocolError::BatchNotResident { key })?;
        if resident.version != expected_version {
            return Err(ProtocolError::BatchVersionMismatch {
                key,
                resident: resident.version,
                expected: expected_version,
            });
        }

        self.batches.remove(&key);
        self.resident = ResidentStats {
            batch_count: self.resident.batch_count - 1,
            point_count: self.resident.point_count - resident.point_count,
            estimated_gpu_bytes: self.resident.estimated_gpu_bytes - resident.estimated_gpu_bytes,
        };

        Ok(self.report(
            UpdateKind::BatchRemoved,
            0,
            0,
            resident.point_count,
            resident.estimated_gpu_bytes,
        ))
    }

    fn apply_highlights(
        &mut self,
        frame: FrameKey,
        point_ids: &[PointId],
    ) -> Result<UpdateReport, ProtocolError> {
        self.require_active_frame(frame)?;
        let next_highlights: BTreeSet<_> = point_ids.iter().copied().collect();
        u64::try_from(next_highlights.len()).map_err(|_| ProtocolError::SizeOverflow)?;

        self.highlights = next_highlights;
        Ok(self.report(UpdateKind::HighlightsSet, 0, 0, 0, 0))
    }

    fn require_active_frame(&self, received: FrameKey) -> Result<(), ProtocolError> {
        let active = self
            .active_frame
            .ok_or(ProtocolError::GenerationNotStarted { received })?;
        if active != received {
            return Err(ProtocolError::FrameMismatch { active, received });
        }
        Ok(())
    }

    fn report(
        &self,
        kind: UpdateKind,
        uploaded_points: u64,
        uploaded_bytes: u64,
        removed_points: u64,
        removed_bytes: u64,
    ) -> UpdateReport {
        UpdateReport {
            kind,
            frame: self
                .active_frame
                .expect("an accepted update always has an active frame"),
            resident: self.resident,
            uploaded_points,
            uploaded_bytes,
            removed_points,
            removed_bytes,
            highlight_count: u64::try_from(self.highlights.len())
                .expect("accepted highlight counts fit the protocol range"),
        }
    }
}

fn replace_count(total: u64, old: u64, new: u64) -> Result<u64, ProtocolError> {
    total
        .checked_sub(old)
        .and_then(|remaining| remaining.checked_add(new))
        .ok_or(ProtocolError::SizeOverflow)
}

fn enforce_limit(
    resource: ResidentResource,
    attempted: u64,
    limit: u64,
) -> Result<(), ProtocolError> {
    if attempted > limit {
        return Err(ProtocolError::ResidentLimitExceeded {
            resource,
            limit,
            attempted,
        });
    }
    Ok(())
}

/// Errors returned while constructing or applying protocol values.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProtocolError {
    /// One origin-relative position component was NaN or infinite.
    #[error("relative position axis {axis} is not finite")]
    NonFiniteRelativePosition {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// One world-origin component was NaN or infinite.
    #[error("world origin axis {axis} is not finite")]
    NonFiniteWorldOrigin {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// A point batch contained no points.
    #[error("point batches must not be empty")]
    EmptyPointBatch,
    /// A point or byte count exceeded the protocol's integer representation.
    #[error("point batch size exceeds the protocol accounting range")]
    SizeOverflow,
    /// This view generation was already begun by a reset.
    #[error("frame {frame:?} was already started")]
    GenerationAlreadyStarted {
        /// The duplicate frame.
        frame: FrameKey,
    },
    /// A reset attempted to return to an older generation of one view.
    #[error(
        "view {view:?} generation {received_generation} is older than generation {last_generation}"
    )]
    StaleGeneration {
        /// The affected view.
        view: ViewId,
        /// The last generation begun for that view.
        last_generation: u64,
        /// The generation in the rejected reset.
        received_generation: u64,
    },
    /// A non-reset update arrived before any generation began.
    #[error("frame {received:?} has not been started by a reset")]
    GenerationNotStarted {
        /// The rejected update's frame.
        received: FrameKey,
    },
    /// An update did not belong to the active frame.
    #[error("update frame {received:?} does not match active frame {active:?}")]
    FrameMismatch {
        /// The active frame.
        active: FrameKey,
        /// The rejected update's frame.
        received: FrameKey,
    },
    /// An upsert did not advance the version last seen for its batch key.
    #[error("batch {key:?} version {received:?} does not advance {previous:?}")]
    BatchVersionNotIncreasing {
        /// The affected batch key.
        key: BatchKey,
        /// The latest accepted version, including removed versions.
        previous: BatchVersion,
        /// The rejected version.
        received: BatchVersion,
    },
    /// A conditional removal named a batch that was not resident.
    #[error("batch {key:?} is not resident")]
    BatchNotResident {
        /// The missing batch key.
        key: BatchKey,
    },
    /// A conditional removal did not match the resident batch version.
    #[error("batch {key:?} has version {resident:?}, not expected version {expected:?}")]
    BatchVersionMismatch {
        /// The affected batch key.
        key: BatchKey,
        /// The currently resident version.
        resident: BatchVersion,
        /// The caller's expected version.
        expected: BatchVersion,
    },
    /// An upsert would exceed a caller-selected hard residency limit.
    #[error("{resource:?} residency {attempted} exceeds hard limit {limit}")]
    ResidentLimitExceeded {
        /// The resource whose limit would be exceeded.
        resource: ResidentResource,
        /// The configured hard limit.
        limit: u64,
        /// The residency that the rejected update would produce.
        attempted: u64,
    },
}

fn first_non_finite_axis<T, const N: usize>(values: &[T; N]) -> Option<usize>
where
    T: Finite,
{
    values.iter().position(|value| !value.is_finite())
}

trait Finite {
    fn is_finite(&self) -> bool;
}

impl Finite for f32 {
    fn is_finite(&self) -> bool {
        f32::is_finite(*self)
    }
}

impl Finite for f64 {
    fn is_finite(&self) -> bool {
        f64::is_finite(*self)
    }
}
