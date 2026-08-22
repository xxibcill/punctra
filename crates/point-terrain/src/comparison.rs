use std::mem;

use blake3::Hasher;
use foundation_runtime::{OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::{ContentHash, PointId, WorldBounds};
use point_workspace::SnapshotProvenance;

use crate::{
    SurfaceComparisonLimits, SurfaceFace, TerrainError, TerrainSurface,
    limits::{require_within, usize_to_u64_saturating},
};

const CHANGE_HASH_DOMAIN: &[u8] = b"punctra-terrain-surface-change-v1";
const CANCELLATION_STRIDE: u64 = 1_024;

/// Exact semantic topology difference and conservative changed-region bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceComparisonReport {
    before_snapshot: SnapshotProvenance,
    after_snapshot: SnapshotProvenance,
    before_artifact_hash: ContentHash,
    after_artifact_hash: ContentHash,
    added_face_count: u64,
    removed_face_count: u64,
    added_face_hash: ContentHash,
    removed_face_hash: ContentHash,
    changed_bounds: Option<WorldBounds>,
    retained_record_bytes: u64,
    work_units: u64,
    accounted_peak_working_bytes: u64,
}

impl SurfaceComparisonReport {
    /// Returns the exact before-Surface Snapshot identity.
    #[must_use]
    pub const fn before_snapshot(self) -> SnapshotProvenance {
        self.before_snapshot
    }

    /// Returns the exact after-Surface Snapshot identity.
    #[must_use]
    pub const fn after_snapshot(self) -> SnapshotProvenance {
        self.after_snapshot
    }

    /// Returns the provenance-sensitive before-Surface hash.
    #[must_use]
    pub const fn before_artifact_hash(self) -> ContentHash {
        self.before_artifact_hash
    }

    /// Returns the provenance-sensitive after-Surface hash.
    #[must_use]
    pub const fn after_artifact_hash(self) -> ContentHash {
        self.after_artifact_hash
    }

    /// Returns faces present only in the after Surface.
    #[must_use]
    pub const fn added_face_count(self) -> u64 {
        self.added_face_count
    }

    /// Returns faces present only in the before Surface.
    #[must_use]
    pub const fn removed_face_count(self) -> u64 {
        self.removed_face_count
    }

    /// Returns the canonical hash of added face Point identities.
    #[must_use]
    pub const fn added_face_hash(self) -> ContentHash {
        self.added_face_hash
    }

    /// Returns the canonical hash of removed face Point identities.
    #[must_use]
    pub const fn removed_face_hash(self) -> ContentHash {
        self.removed_face_hash
    }

    /// Returns conservative bounds of vertices incident to changed faces.
    ///
    /// These bounds are not an exact change polygon.
    #[must_use]
    pub const fn changed_bounds(self) -> Option<WorldBounds> {
        self.changed_bounds
    }

    /// Returns the retained before/after comparison-record bytes.
    #[must_use]
    pub const fn retained_record_bytes(self) -> u64 {
        self.retained_record_bytes
    }

    /// Returns deterministic record, sort, and merge work units.
    #[must_use]
    pub const fn work_units(self) -> u64 {
        self.work_units
    }

    /// Returns conservatively accounted peak comparison bytes.
    #[must_use]
    pub const fn accounted_peak_working_bytes(self) -> u64 {
        self.accounted_peak_working_bytes
    }
}

#[derive(Clone, Copy)]
struct FaceRecord {
    key: [PointId; 3],
    vertices: [u32; 3],
}

struct ChangedFaces {
    count: u64,
    hasher: Hasher,
    overflow_message: &'static str,
}

impl ChangedFaces {
    fn new(kind: &[u8], overflow_message: &'static str) -> Self {
        Self {
            count: 0,
            hasher: change_hasher(kind),
            overflow_message,
        }
    }

    fn record(
        &mut self,
        surface: &TerrainSurface,
        record: FaceRecord,
        bounds: &mut Option<([f64; 3], [f64; 3])>,
    ) -> Result<(), TerrainError> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| TerrainError::numeric(self.overflow_message))?;
        hash_face(&mut self.hasher, record.key);
        extend_bounds(bounds, surface, record.vertices)
    }
}

struct WorkMeter<'a> {
    used: u64,
    limit: u64,
    control: &'a OperationControl,
}

impl<'a> WorkMeter<'a> {
    const fn new(limit: u64, control: &'a OperationControl) -> Self {
        Self {
            used: 0,
            limit,
            control,
        }
    }

    fn charge(&mut self) -> Result<(), TerrainError> {
        self.used = self.used.checked_add(1).ok_or_else(|| {
            TerrainError::resource("Surface comparison work units", u64::MAX, self.limit)
        })?;
        if self.used > self.limit {
            return Err(TerrainError::resource(
                "Surface comparison work units",
                self.used,
                self.limit,
            ));
        }
        if self.used.is_multiple_of(CANCELLATION_STRIDE) {
            self.control.check_cancelled()?;
            self.control.report_progress(ProgressSnapshot::new(
                ProgressPhase::RUNNING,
                self.used,
                None,
            )?)?;
        }
        Ok(())
    }
}

/// Starts exact semantic comparison of two immutable in-memory Surfaces.
#[must_use]
pub fn compare_surfaces(
    before: &TerrainSurface,
    after: &TerrainSurface,
    limits: SurfaceComparisonLimits,
) -> crate::SurfaceComparisonJob {
    let before = before.clone();
    let after = after.clone();
    crate::SurfaceComparisonJob::spawn(move |control| run(&before, &after, limits, &control))
}

fn run(
    before: &TerrainSurface,
    after: &TerrainSurface,
    limits: SurfaceComparisonLimits,
    control: &OperationControl,
) -> Result<SurfaceComparisonReport, TerrainError> {
    control.check_cancelled()?;
    validate_compatible(before, after)?;
    preflight_records(before, after, limits)?;

    let mut meter = WorkMeter::new(limits.max_work_units(), control);
    let mut before_records = face_records(before, &mut meter)?;
    let mut after_records = face_records(after, &mut meter)?;
    let retained_record_bytes =
        allocation_bytes(&before_records).saturating_add(allocation_bytes(&after_records));
    require_within(
        "Surface comparison record bytes",
        retained_record_bytes,
        limits.max_record_bytes(),
    )?;
    let accounted_peak_working_bytes = retained_record_bytes;
    require_within(
        "Surface comparison working bytes",
        accounted_peak_working_bytes,
        limits.max_working_bytes(),
    )?;
    crate::sort::heap_sort_by(&mut before_records, |left, right| {
        less(left, right, &mut meter)
    })?;
    crate::sort::heap_sort_by(&mut after_records, |left, right| {
        less(left, right, &mut meter)
    })?;

    let mut added = ChangedFaces::new(b"added", "added Surface face count overflowed");
    let mut removed = ChangedFaces::new(b"removed", "removed Surface face count overflowed");
    let mut bounds: Option<([f64; 3], [f64; 3])> = None;
    let (mut left, mut right) = (0, 0);
    while left < before_records.len() || right < after_records.len() {
        meter.charge()?;
        match (before_records.get(left), after_records.get(right)) {
            (Some(before_record), Some(after_record)) if before_record.key == after_record.key => {
                left += 1;
                right += 1;
            }
            (Some(before_record), Some(after_record)) if before_record.key < after_record.key => {
                removed.record(before, *before_record, &mut bounds)?;
                left += 1;
            }
            (Some(_) | None, Some(after_record)) => {
                added.record(after, *after_record, &mut bounds)?;
                right += 1;
            }
            (Some(before_record), None) => {
                removed.record(before, *before_record, &mut bounds)?;
                left += 1;
            }
            (None, None) => break,
        }
    }
    let changed_bounds = bounds
        .map(|(min, max)| WorldBounds::new(min, max))
        .transpose()?;
    control.check_cancelled()?;
    control.complete_progress(meter.used)?;
    Ok(SurfaceComparisonReport {
        before_snapshot: before.descriptor().snapshot(),
        after_snapshot: after.descriptor().snapshot(),
        before_artifact_hash: before.descriptor().artifact_hash(),
        after_artifact_hash: after.descriptor().artifact_hash(),
        added_face_count: added.count,
        removed_face_count: removed.count,
        added_face_hash: ContentHash::new(*added.hasher.finalize().as_bytes()),
        removed_face_hash: ContentHash::new(*removed.hasher.finalize().as_bytes()),
        changed_bounds,
        retained_record_bytes,
        work_units: meter.used,
        accounted_peak_working_bytes,
    })
}

fn validate_compatible(
    before: &TerrainSurface,
    after: &TerrainSurface,
) -> Result<(), TerrainError> {
    let before_descriptor = before.descriptor();
    let after_descriptor = after.descriptor();
    let before_snapshot = before_descriptor.snapshot();
    let after_snapshot = after_descriptor.snapshot();
    if before_snapshot.workspace() != after_snapshot.workspace()
        || before_snapshot.source() != after_snapshot.source()
    {
        return Err(TerrainError::invalid(
            "Surface comparison lineage",
            "both Surfaces must belong to the same Workspace and Source",
        ));
    }
    if before_descriptor.recipe() != after_descriptor.recipe() {
        return Err(TerrainError::invalid(
            "Surface comparison Recipe",
            "both Surfaces must use the same normalized Terrain Recipe",
        ));
    }
    if before_descriptor.position_transform() != after_descriptor.position_transform() {
        return Err(TerrainError::invalid(
            "Surface comparison transform",
            "both Surfaces must use the same exact position transform",
        ));
    }
    if before_descriptor.coordinate_reference() != after_descriptor.coordinate_reference() {
        return Err(TerrainError::invalid(
            "Surface comparison spatial reference",
            "both Surfaces must use the same coordinate reference",
        ));
    }
    Ok(())
}

fn preflight_records(
    before: &TerrainSurface,
    after: &TerrainSurface,
    limits: SurfaceComparisonLimits,
) -> Result<(), TerrainError> {
    let total_faces = usize_to_u64_saturating(before.faces().len())
        .saturating_add(usize_to_u64_saturating(after.faces().len()));
    require_within("Surface comparison faces", total_faces, limits.max_faces())?;
    let required_record_bytes =
        total_faces.saturating_mul(usize_to_u64_saturating(mem::size_of::<FaceRecord>()));
    require_within(
        "Surface comparison record bytes",
        required_record_bytes,
        limits.max_record_bytes(),
    )?;
    require_within(
        "Surface comparison working bytes",
        required_record_bytes,
        limits.max_working_bytes(),
    )
}

fn face_records(
    surface: &TerrainSurface,
    meter: &mut WorkMeter<'_>,
) -> Result<Vec<FaceRecord>, TerrainError> {
    let mut records = Vec::new();
    records
        .try_reserve_exact(surface.faces().len())
        .map_err(|_| {
            TerrainError::resource(
                "Surface comparison allocation",
                usize_to_u64_saturating(surface.faces().len()),
                usize_to_u64_saturating(usize::MAX),
            )
        })?;
    for face in surface.faces().iter().copied() {
        meter.charge()?;
        records.push(face_record(surface, face)?);
    }
    Ok(records)
}

fn face_record(surface: &TerrainSurface, face: SurfaceFace) -> Result<FaceRecord, TerrainError> {
    let vertices = face.vertices().map(crate::SurfaceVertexId::get);
    let mut key = [
        point_for_vertex(surface, vertices[0])?,
        point_for_vertex(surface, vertices[1])?,
        point_for_vertex(surface, vertices[2])?,
    ];
    key.sort_unstable();
    Ok(FaceRecord { key, vertices })
}

fn point_for_vertex(surface: &TerrainSurface, id: u32) -> Result<PointId, TerrainError> {
    surface
        .vertices()
        .get(usize::try_from(id - 1).unwrap_or(usize::MAX))
        .map(|vertex| vertex.point())
        .ok_or_else(|| TerrainError::topology("a Surface face references a missing vertex"))
}

fn less(
    left: FaceRecord,
    right: FaceRecord,
    meter: &mut WorkMeter<'_>,
) -> Result<bool, TerrainError> {
    meter.charge()?;
    Ok(left.key < right.key)
}

fn extend_bounds(
    bounds: &mut Option<([f64; 3], [f64; 3])>,
    surface: &TerrainSurface,
    vertices: [u32; 3],
) -> Result<(), TerrainError> {
    for id in vertices {
        let vertex = surface
            .vertices()
            .get(usize::try_from(id - 1).unwrap_or(usize::MAX))
            .ok_or_else(|| TerrainError::topology("a Surface face references a missing vertex"))?;
        let world = surface
            .descriptor()
            .position_transform()
            .world_f64(vertex.ticks());
        if world.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(TerrainError::numeric(
                "a changed Surface vertex world position is not finite",
            ));
        }
        match bounds {
            None => *bounds = Some((world, world)),
            Some((min, max)) => {
                for axis in 0..3 {
                    min[axis] = min[axis].min(world[axis]);
                    max[axis] = max[axis].max(world[axis]);
                }
            }
        }
    }
    Ok(())
}

fn change_hasher(kind: &[u8]) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(CHANGE_HASH_DOMAIN);
    hasher.update(kind);
    hasher
}

fn hash_face(hasher: &mut Hasher, key: [PointId; 3]) {
    for point in key {
        hasher.update(point.source().as_bytes());
        hasher.update(&point.ordinal().to_le_bytes());
    }
}

fn allocation_bytes(records: &Vec<FaceRecord>) -> u64 {
    usize_to_u64_saturating(records.capacity())
        .saturating_mul(usize_to_u64_saturating(mem::size_of::<FaceRecord>()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use foundation_runtime::OperationControl;
    use point_contracts::{
        ContentHash, CoordinateReference, LinearUnit, PositionTransform, SpatialAxes,
        SpatialReferenceProfile, SpatialReferenceProvenance, WorldBounds,
    };
    use point_workspace::SnapshotProvenance;

    use super::{CANCELLATION_STRIDE, WorkMeter, compare_surfaces};
    use crate::{
        TerrainDescriptor, TerrainError, TerrainRecipe, TerrainSurface,
        model::SurfaceData as ModelSurfaceData,
    };

    #[test]
    fn comparison_rejects_a_spatial_reference_mismatch_without_a_report() {
        let before = empty_surface(spatial_reference(32_647));
        let after = before.with_coordinate_reference_for_test(spatial_reference(32_648));

        let error = compare_surfaces(&before, &after, crate::SurfaceComparisonLimits::default())
            .blocking_wait()
            .expect_err("a reference mismatch must not publish a comparison report");

        assert!(matches!(
            error,
            TerrainError::InvalidArgument {
                argument: "Surface comparison spatial reference",
                ..
            }
        ));
    }

    #[test]
    fn comparison_observes_cancellation_at_the_bounded_work_stride() {
        let control = OperationControl::new();
        let mut meter = WorkMeter::new(u64::MAX, &control);
        for _ in 1..CANCELLATION_STRIDE {
            meter.charge().unwrap();
        }
        control.cancel();

        let error = meter
            .charge()
            .expect_err("the next comparison boundary must observe cancellation");

        assert!(matches!(error, TerrainError::Cancelled));
    }

    fn empty_surface(coordinate_reference: CoordinateReference) -> TerrainSurface {
        let snapshot: SnapshotProvenance = serde_json::from_value(serde_json::json!({
            "workspace": vec![1_u8; 16],
            "source": vec![2_u8; 32],
            "revision": vec![3_u8; 32],
        }))
        .unwrap();
        let descriptor = TerrainDescriptor::new(
            snapshot,
            TerrainRecipe::new(2),
            ContentHash::new([4; 32]),
            PositionTransform::new([0.0; 3], [1.0; 3]).unwrap(),
            coordinate_reference,
            ContentHash::new([5; 32]),
            ContentHash::new([6; 32]),
            ContentHash::new([7; 32]),
            ContentHash::new([8; 32]),
            0,
            0,
            0,
            0,
            WorldBounds::new([0.0; 3], [1.0; 3]).unwrap(),
            0,
            0,
            0,
        );
        TerrainSurface {
            inner: Arc::new(ModelSurfaceData {
                descriptor,
                vertices: Vec::new(),
                faces: Vec::new(),
            }),
        }
    }

    fn spatial_reference(horizontal_epsg: u32) -> CoordinateReference {
        CoordinateReference::profile(
            SpatialReferenceProfile::new(
                horizontal_epsg,
                5_703,
                SpatialAxes::EastingNorthingElevation,
                LinearUnit::Metre,
                LinearUnit::Metre,
                SpatialReferenceProvenance::CallerDeclaration,
            )
            .unwrap(),
        )
    }
}
