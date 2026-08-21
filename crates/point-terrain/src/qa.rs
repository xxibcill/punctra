use std::mem;

use foundation_runtime::{Job, OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::SpatialReferenceProfile;
use robust::{Coord, orient2d};

use crate::{
    CheckPoint, CheckPointJob, CheckPointLimits, CheckPointOutcome, CheckPointReport,
    CheckPointResult, ResidualStatistics, SurfaceFace, TerrainError, TerrainSurface,
    numeric::canonical_zero,
};

const CANCELLATION_STRIDE: usize = 1_024;

pub(crate) fn start<I>(
    surface: &TerrainSurface,
    check_points: I,
    limits: CheckPointLimits,
) -> CheckPointJob
where
    I: IntoIterator<Item = CheckPoint> + Send + 'static,
{
    let surface = surface.clone();
    Job::spawn(move |control| {
        if !surface
            .descriptor()
            .spatial_reference_profile()
            .is_some_and(SpatialReferenceProfile::is_supported_metric_survey)
        {
            return Err(TerrainError::invalid(
                "Check Point spatial reference",
                "the v0.12 QA path requires a complete easting/northing/elevation profile with metre horizontal and vertical units",
            ));
        }
        let (check_points, collection_bytes) =
            collect_check_points(check_points, limits, &control)?;
        evaluate(&surface, check_points, collection_bytes, limits, &control)
    })
}

fn collect_check_points<I>(
    check_points: I,
    limits: CheckPointLimits,
    control: &OperationControl,
) -> Result<(Vec<CheckPoint>, u64), TerrainError>
where
    I: IntoIterator<Item = CheckPoint>,
{
    control.check_cancelled()?;
    let mut input = check_points.into_iter();
    let lower_bound = u64::try_from(input.size_hint().0).unwrap_or(u64::MAX);
    require_count(lower_bound, limits)?;

    let mut collected = Vec::new();
    loop {
        let next = input.next();
        control.check_cancelled()?;
        let Some(check_point) = next else {
            break;
        };
        let next_count = u64::try_from(collected.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        require_count(next_count, limits)?;
        if collected.len() == collected.capacity() {
            grow_check_points(&mut collected, limits)?;
        }
        collected.push(check_point);
    }
    let bytes = allocation_bytes::<CheckPoint>(collected.capacity());
    require_working(bytes, limits, "Check Point input working bytes")?;
    Ok((collected, bytes))
}

fn grow_check_points(
    check_points: &mut Vec<CheckPoint>,
    limits: CheckPointLimits,
) -> Result<(), TerrainError> {
    let old_capacity = check_points.capacity();
    let maximum = usize::try_from(limits.max_check_points()).unwrap_or(usize::MAX);
    let requested = old_capacity
        .max(1)
        .saturating_mul(2)
        .min(maximum)
        .max(check_points.len().saturating_add(1));
    let requested_bytes = allocation_bytes::<CheckPoint>(requested);
    require_working(
        allocation_bytes::<CheckPoint>(old_capacity).saturating_add(requested_bytes),
        limits,
        "Check Point input growth overlap",
    )?;
    check_points
        .try_reserve_exact(requested.saturating_sub(check_points.len()))
        .map_err(|_| {
            TerrainError::resource(
                "Check Point input allocation",
                requested_bytes,
                limits.max_working_bytes(),
            )
        })?;
    let actual_bytes = allocation_bytes::<CheckPoint>(check_points.capacity());
    require_working(
        allocation_bytes::<CheckPoint>(old_capacity).saturating_add(actual_bytes),
        limits,
        "Check Point input growth overlap",
    )
}

fn evaluate(
    surface: &TerrainSurface,
    check_points: Vec<CheckPoint>,
    collection_bytes: u64,
    limits: CheckPointLimits,
    control: &OperationControl,
) -> Result<CheckPointReport, TerrainError> {
    control.check_cancelled()?;
    let count = u64::try_from(check_points.len()).unwrap_or(u64::MAX);
    require_count(count, limits)?;

    let identity_bytes = checked_payload_bytes::<crate::CheckPointId>(check_points.len())?;
    require_working(
        collection_bytes.saturating_add(identity_bytes),
        limits,
        "Check Point identity validation working bytes",
    )?;
    let mut identities = Vec::new();
    identities
        .try_reserve_exact(check_points.len())
        .map_err(|_| {
            TerrainError::resource(
                "Check Point identity allocation",
                identity_bytes,
                limits.max_working_bytes(),
            )
        })?;
    let identity_allocation = allocation_bytes::<crate::CheckPointId>(identities.capacity());
    require_working(
        collection_bytes.saturating_add(identity_allocation),
        limits,
        "Check Point identity validation working bytes",
    )?;
    identities.extend(check_points.iter().map(|check_point| check_point.id()));
    sort_identities(&mut identities, control)?;
    if let Some(duplicate) = identities
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0])
    {
        return Err(TerrainError::invalid(
            "Check Point identities",
            format!("identity {} occurs more than once", duplicate.get()),
        ));
    }
    drop(identities);

    let requested_result_bytes = checked_payload_bytes::<CheckPointResult>(check_points.len())?;
    require_result_bytes(requested_result_bytes, limits)?;
    require_working(
        collection_bytes.saturating_add(requested_result_bytes),
        limits,
        "Check Point result working bytes",
    )?;
    let mut results = Vec::new();
    results.try_reserve_exact(check_points.len()).map_err(|_| {
        TerrainError::resource(
            "Check Point result allocation",
            requested_result_bytes,
            limits.max_working_bytes(),
        )
    })?;
    let result_bytes = allocation_bytes::<CheckPointResult>(results.capacity());
    let peak_working_bytes = collection_bytes
        .saturating_add(identity_allocation)
        .max(collection_bytes.saturating_add(result_bytes));
    require_working(peak_working_bytes, limits, "Check Point peak working bytes")?;

    let mut accumulator = ResidualAccumulator::default();
    let mut face_tests = 0_u64;
    let mut input = check_points.into_iter();
    for (index, check_point) in input.by_ref().enumerate() {
        control.check_cancelled()?;
        let outcome = locate(surface, check_point, limits, &mut face_tests, control)?;
        accumulator.observe(outcome)?;
        results.push(CheckPointResult::new(check_point, outcome));
        let completed = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
        control.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            completed,
            Some(count),
        )?)?;
    }
    drop(input);

    let boxed_result_overlap = result_bytes.saturating_add(requested_result_bytes);
    let peak_working_bytes = peak_working_bytes.max(boxed_result_overlap);
    require_working(
        peak_working_bytes,
        limits,
        "Check Point boxed-result conversion working bytes",
    )?;
    let results = results.into_boxed_slice();
    let report = CheckPointReport::new(
        results,
        accumulator.finish(),
        face_tests,
        peak_working_bytes,
    );
    control.check_cancelled()?;
    control.complete_progress(count)?;
    Ok(report)
}

fn sort_identities(
    identities: &mut [crate::CheckPointId],
    control: &OperationControl,
) -> Result<(), TerrainError> {
    let mut comparisons = 0_usize;
    for root in (0..identities.len() / 2).rev() {
        sift_down(
            identities,
            root,
            identities.len(),
            &mut comparisons,
            control,
        )?;
    }
    for end in (1..identities.len()).rev() {
        identities.swap(0, end);
        sift_down(identities, 0, end, &mut comparisons, control)?;
    }
    control.check_cancelled()?;
    Ok(())
}

fn sift_down(
    identities: &mut [crate::CheckPointId],
    mut root: usize,
    end: usize,
    comparisons: &mut usize,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    loop {
        let Some(left) = root.checked_mul(2).and_then(|value| value.checked_add(1)) else {
            return Ok(());
        };
        if left >= end {
            return Ok(());
        }
        let right = left.saturating_add(1);
        let child = if right < end
            && compare_identity(identities[left], identities[right], comparisons, control)?
        {
            right
        } else {
            left
        };
        if !compare_identity(identities[root], identities[child], comparisons, control)? {
            return Ok(());
        }
        identities.swap(root, child);
        root = child;
    }
}

fn compare_identity(
    left: crate::CheckPointId,
    right: crate::CheckPointId,
    comparisons: &mut usize,
    control: &OperationControl,
) -> Result<bool, TerrainError> {
    *comparisons = comparisons.saturating_add(1);
    if comparisons.is_multiple_of(CANCELLATION_STRIDE) {
        control.check_cancelled()?;
    }
    Ok(left < right)
}

pub(crate) fn locate(
    surface: &TerrainSurface,
    check_point: CheckPoint,
    limits: CheckPointLimits,
    face_tests: &mut u64,
    control: &OperationControl,
) -> Result<CheckPointOutcome, TerrainError> {
    let position = check_point.position();
    let world_query = Coord {
        x: position[0],
        y: position[1],
    };
    for (index, face) in surface.faces().iter().copied().enumerate() {
        if index.is_multiple_of(CANCELLATION_STRIDE) {
            control.check_cancelled()?;
        }
        *face_tests = face_tests.checked_add(1).ok_or_else(|| {
            TerrainError::resource("Check Point face tests", u64::MAX, limits.max_face_tests())
        })?;
        if *face_tests > limits.max_face_tests() {
            return Err(TerrainError::resource(
                "Check Point face tests",
                *face_tests,
                limits.max_face_tests(),
            ));
        }
        let world_triangle = face_world(surface, face)?;
        let (triangle, query, elevation_origin) = match normalize_xy(world_triangle, world_query) {
            NormalizedFace::Outside => continue,
            NormalizedFace::Candidate { triangle, query } => (triangle, query, 0.0),
            NormalizedFace::Degenerate => {
                let frame = face_local(surface, face)?;
                let local_query = Coord {
                    x: position[0] - frame.world_origin[0],
                    y: position[1] - frame.world_origin[1],
                };
                let NormalizedFace::Candidate { triangle, query } =
                    normalize_xy(frame.triangle, local_query)
                else {
                    continue;
                };
                (triangle, query, frame.world_origin[2])
            }
        };
        if contains_closed(triangle, query) {
            let surface_z = canonical_zero(elevation_origin + interpolate_z(triangle, query)?);
            if !surface_z.is_finite() {
                return Err(TerrainError::numeric(
                    "interpolated Surface elevation is not finite",
                ));
            }
            let residual = canonical_zero(position[2] - surface_z);
            if !residual.is_finite() {
                return Err(TerrainError::numeric("Check Point residual is not finite"));
            }
            return Ok(CheckPointOutcome::Sampled {
                face: face.id(),
                surface_z,
                residual,
            });
        }
    }
    Ok(CheckPointOutcome::Gap)
}

struct LocalFaceFrame {
    triangle: [[f64; 3]; 3],
    world_origin: [f64; 3],
}

fn face_world(surface: &TerrainSurface, face: SurfaceFace) -> Result<[[f64; 3]; 3], TerrainError> {
    let transform = surface.descriptor().position_transform();
    let mut world = [[0.0; 3]; 3];
    for (slot, vertex) in face.vertices().into_iter().enumerate() {
        let Some(vertex) = surface.vertices().get(vertex.zero_based()) else {
            return Err(TerrainError::topology(
                "a Surface face references a missing vertex",
            ));
        };
        world[slot] = transform.world_f64(vertex.ticks());
        if world[slot].iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(TerrainError::numeric(
                "a Surface vertex world position is not finite",
            ));
        }
    }
    Ok(world)
}

fn face_local(surface: &TerrainSurface, face: SurfaceFace) -> Result<LocalFaceFrame, TerrainError> {
    let transform = surface.descriptor().position_transform();
    let mut ticks = [[0; 3]; 3];
    for (slot, vertex) in face.vertices().into_iter().enumerate() {
        let Some(vertex) = surface.vertices().get(vertex.zero_based()) else {
            return Err(TerrainError::topology(
                "a Surface face references a missing vertex",
            ));
        };
        ticks[slot] = vertex.ticks();
    }

    let world_origin = transform.world_f64(ticks[0]);
    if world_origin
        .iter()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Err(TerrainError::numeric(
            "a Surface vertex world position is not finite",
        ));
    }

    let scale = transform.scale();
    let mut triangle = [[0.0; 3]; 3];
    for (slot, position) in ticks.into_iter().enumerate() {
        for axis in 0..3 {
            triangle[slot][axis] = scaled_tick_delta(position[axis], ticks[0][axis], scale[axis]);
        }
        if triangle[slot]
            .iter()
            .any(|coordinate| !coordinate.is_finite())
        {
            return Err(TerrainError::numeric(
                "a Surface vertex local position is not finite",
            ));
        }
    }
    Ok(LocalFaceFrame {
        triangle,
        world_origin,
    })
}

#[allow(
    clippy::cast_precision_loss,
    reason = "Surface calculations intentionally use the nearest representable f64 tick delta"
)]
fn scaled_tick_delta(tick: i64, origin: i64, scale: f64) -> f64 {
    let delta = tick
        .checked_sub(origin)
        .map_or_else(|| i128::from(tick) - i128::from(origin), i128::from);
    delta as f64 * scale
}

enum NormalizedFace {
    Outside,
    Degenerate,
    Candidate {
        triangle: [[f64; 3]; 3],
        query: Coord<f64>,
    },
}

fn normalize_xy(mut triangle: [[f64; 3]; 3], point: Coord<f64>) -> NormalizedFace {
    let minimum_x = triangle
        .iter()
        .map(|position| position[0])
        .reduce(f64::min)
        .expect("a face always has three positions");
    let maximum_x = triangle
        .iter()
        .map(|position| position[0])
        .reduce(f64::max)
        .expect("a face always has three positions");
    let minimum_y = triangle
        .iter()
        .map(|position| position[1])
        .reduce(f64::min)
        .expect("a face always has three positions");
    let maximum_y = triangle
        .iter()
        .map(|position| position[1])
        .reduce(f64::max)
        .expect("a face always has three positions");
    if point.x < minimum_x || point.x > maximum_x || point.y < minimum_y || point.y > maximum_y {
        return NormalizedFace::Outside;
    }

    let origin = Coord {
        x: triangle[0][0],
        y: triangle[0][1],
    };
    for position in &mut triangle {
        position[0] -= origin.x;
        position[1] -= origin.y;
    }
    let mut point = Coord {
        x: point.x - origin.x,
        y: point.y - origin.y,
    };
    let scale = triangle
        .iter()
        .flat_map(|position| [position[0].abs(), position[1].abs()])
        .reduce(f64::max)
        .expect("a face always has planar coordinates");
    if scale == 0.0 {
        return NormalizedFace::Degenerate;
    }
    for position in &mut triangle {
        position[0] /= scale;
        position[1] /= scale;
    }
    point.x /= scale;
    point.y /= scale;
    if orient2d(xy(triangle[0]), xy(triangle[1]), xy(triangle[2])) == 0.0 {
        return NormalizedFace::Degenerate;
    }
    NormalizedFace::Candidate {
        triangle,
        query: point,
    }
}

fn contains_closed(triangle: [[f64; 3]; 3], point: Coord<f64>) -> bool {
    let a = xy(triangle[0]);
    let b = xy(triangle[1]);
    let c = xy(triangle[2]);
    orient2d(a, b, point) >= 0.0 && orient2d(b, c, point) >= 0.0 && orient2d(c, a, point) >= 0.0
}

fn interpolate_z(triangle: [[f64; 3]; 3], point: Coord<f64>) -> Result<f64, TerrainError> {
    let [a, b, c] = triangle;
    let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if !denominator.is_finite() || denominator == 0.0 {
        return Err(TerrainError::numeric(
            "a Surface face cannot be interpolated in normalized coordinates",
        ));
    }
    let a_weight =
        ((b[1] - c[1]) * (point.x - c[0]) + (c[0] - b[0]) * (point.y - c[1])) / denominator;
    let b_weight =
        ((c[1] - a[1]) * (point.x - c[0]) + (a[0] - c[0]) * (point.y - c[1])) / denominator;
    let c_weight = 1.0 - a_weight - b_weight;
    let elevation = canonical_zero(a_weight * a[2] + b_weight * b[2] + c_weight * c[2]);
    if !elevation.is_finite() {
        return Err(TerrainError::numeric(
            "interpolated Surface elevation is not finite",
        ));
    }
    Ok(elevation)
}

fn xy(position: [f64; 3]) -> Coord<f64> {
    Coord {
        x: position[0],
        y: position[1],
    }
}

#[derive(Default)]
pub(crate) struct ResidualAccumulator {
    covered_count: u64,
    gap_count: u64,
    minimum: Option<f64>,
    maximum: Option<f64>,
    sum: ScaledSum,
    squared_sum: ScaledSquareSum,
}

impl ResidualAccumulator {
    pub(crate) fn observe(&mut self, outcome: CheckPointOutcome) -> Result<(), TerrainError> {
        match outcome {
            CheckPointOutcome::Gap => {
                self.gap_count = self
                    .gap_count
                    .checked_add(1)
                    .ok_or_else(|| TerrainError::numeric("Check Point gap count overflowed"))?;
            }
            CheckPointOutcome::Sampled { residual, .. } => {
                self.covered_count = self
                    .covered_count
                    .checked_add(1)
                    .ok_or_else(|| TerrainError::numeric("covered Check Point count overflowed"))?;
                self.minimum = Some(self.minimum.map_or(residual, |value| value.min(residual)));
                self.maximum = Some(self.maximum.map_or(residual, |value| value.max(residual)));
                self.sum.add(residual)?;
                self.squared_sum.add(residual)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn finish(self) -> ResidualStatistics {
        if self.covered_count == 0 {
            return ResidualStatistics::new(0, self.gap_count, None, None, None, None);
        }
        let count = self.covered_count as f64;
        ResidualStatistics::new(
            self.covered_count,
            self.gap_count,
            self.minimum,
            self.maximum,
            Some(self.sum.mean(count)),
            Some(self.squared_sum.root_mean_square(count)),
        )
    }
}

#[derive(Default)]
struct ScaledSum {
    scale: f64,
    normalized_sum: CompensatedSum,
}

impl ScaledSum {
    fn add(&mut self, value: f64) -> Result<(), TerrainError> {
        let magnitude = value.abs();
        if magnitude == 0.0 {
            return Ok(());
        }
        if magnitude > self.scale {
            let prior_sum = self.normalized_sum.total() * (self.scale / magnitude);
            self.scale = magnitude;
            self.normalized_sum = CompensatedSum::default();
            self.normalized_sum.add(value.signum())?;
            self.normalized_sum.add(prior_sum)?;
        } else {
            self.normalized_sum.add(value / self.scale)?;
        }
        Ok(())
    }

    fn mean(&self, count: f64) -> f64 {
        let normalized = self.normalized_sum.total();
        let sum = self.scale * normalized;
        let mean = if sum.is_finite() {
            sum / count
        } else {
            self.scale * (normalized / count)
        };
        canonical_zero(mean)
    }
}

#[derive(Default)]
struct ScaledSquareSum {
    scale: f64,
    normalized_sum: CompensatedSum,
}

impl ScaledSquareSum {
    fn add(&mut self, value: f64) -> Result<(), TerrainError> {
        let magnitude = value.abs();
        if magnitude == 0.0 {
            return Ok(());
        }
        if magnitude > self.scale {
            let ratio = self.scale / magnitude;
            let prior_sum = self.normalized_sum.total() * ratio * ratio;
            self.scale = magnitude;
            self.normalized_sum = CompensatedSum::default();
            self.normalized_sum.add(1.0)?;
            self.normalized_sum.add(prior_sum)?;
        } else {
            let ratio = magnitude / self.scale;
            self.normalized_sum.add(ratio * ratio)?;
        }
        Ok(())
    }

    fn root_mean_square(&self, count: f64) -> f64 {
        canonical_zero(self.scale * (self.normalized_sum.total() / count).sqrt())
    }
}

#[derive(Default)]
struct CompensatedSum {
    sum: f64,
    correction: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) -> Result<(), TerrainError> {
        let next = self.sum + value;
        let correction = if self.sum.abs() >= value.abs() {
            (self.sum - next) + value
        } else {
            (value - next) + self.sum
        };
        self.sum = next;
        self.correction += correction;
        if !self.sum.is_finite() || !self.correction.is_finite() || !self.total().is_finite() {
            return Err(TerrainError::numeric(
                "Check Point residual statistics are not finite",
            ));
        }
        Ok(())
    }

    fn total(&self) -> f64 {
        self.sum + self.correction
    }
}

fn require_count(count: u64, limits: CheckPointLimits) -> Result<(), TerrainError> {
    if count > limits.max_check_points() {
        return Err(TerrainError::resource(
            "detached Check Points",
            count,
            limits.max_check_points(),
        ));
    }
    Ok(())
}

fn require_result_bytes(bytes: u64, limits: CheckPointLimits) -> Result<(), TerrainError> {
    if bytes > limits.max_result_bytes() {
        return Err(TerrainError::resource(
            "Check Point result bytes",
            bytes,
            limits.max_result_bytes(),
        ));
    }
    Ok(())
}

fn require_working(
    bytes: u64,
    limits: CheckPointLimits,
    name: &'static str,
) -> Result<(), TerrainError> {
    if bytes > limits.max_working_bytes() {
        return Err(TerrainError::resource(
            name,
            bytes,
            limits.max_working_bytes(),
        ));
    }
    Ok(())
}

fn checked_payload_bytes<T>(len: usize) -> Result<u64, TerrainError> {
    u64::try_from(len)
        .unwrap_or(u64::MAX)
        .checked_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
        .ok_or_else(|| TerrainError::numeric("Check Point payload byte count overflowed"))
}

fn allocation_bytes<T>(capacity: usize) -> u64 {
    u64::try_from(capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
}
