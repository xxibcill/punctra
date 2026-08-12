//! Bounded deterministic planar Delaunay triangulation.
//!
//! The advancing-hull implementation in this file is adapted from
//! `delaunator-rs` 1.1.0. Punctra replaces its unbounded allocation and
//! indivisible execution behavior with fixed arenas, cooperative cancellation,
//! deterministic tie-breaking, and robust orientation/in-circle predicates.
//! The upstream ISC terms are retained in
//! `../third_party/delaunator-LICENSE`.

use std::{cmp::Ordering, mem};

use foundation_runtime::OperationControl;
use robust::{Coord, incircle, orient2d};

const EMPTY: usize = usize::MAX;
const CANCEL_INTERVAL: u64 = 1_024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlanarPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TriangulationLimits {
    pub(crate) max_working_bytes: u64,
    pub(crate) max_steps: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TriangulationOutput {
    pub(crate) triangles: Vec<[usize; 3]>,
    pub(crate) steps: u64,
    pub(crate) peak_working_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TriangulationFailure {
    Cancelled,
    Resource {
        resource: &'static str,
        required: u64,
        allowed: u64,
    },
    Collinear,
    Invariant(&'static str),
}

#[allow(clippy::too_many_lines)]
pub(crate) fn triangulate(
    points: &[PlanarPoint],
    limits: TriangulationLimits,
    control: &OperationControl,
) -> Result<TriangulationOutput, TriangulationFailure> {
    check_cancelled(control)?;
    let mut meter = WorkMeter::new(limits.max_steps);
    validate_points(points, &mut meter, control)?;

    let point_count = points.len();
    if point_count < 3 || all_collinear(points, &mut meter, control)? {
        return Err(TriangulationFailure::Collinear);
    }

    let maximum_faces = point_count
        .checked_mul(2)
        .and_then(|value| value.checked_sub(5))
        .ok_or_else(|| resource_overflow("topology cardinality", limits.max_working_bytes))?;
    let maximum_halfedges = maximum_faces
        .checked_mul(3)
        .ok_or_else(|| resource_overflow("topology cardinality", limits.max_working_bytes))?;
    let hash_capacity = integer_ceil_sqrt(point_count, &mut meter, control)?;

    let requested_peak = requested_peak_bytes(
        point_count,
        maximum_faces,
        maximum_halfedges,
        hash_capacity,
        limits.max_working_bytes,
    )?;
    require_bytes(
        requested_peak,
        limits.max_working_bytes,
        "max_working_bytes",
    )?;

    meter.checkpoint(control)?;

    let seed =
        find_seed_triangle(points, &mut meter, control)?.ok_or(TriangulationFailure::Collinear)?;
    let center = points[seed.0].circumcenter(points[seed.1], points[seed.2])?;

    let mut raw =
        RawTriangulation::new(maximum_halfedges, requested_peak, limits.max_working_bytes)?;
    raw.add_triangle(seed.0, seed.1, seed.2, EMPTY, EMPTY, EMPTY)?;

    let (mut distances, mut distance_scratch) = distance_order(
        points,
        center,
        requested_peak,
        limits.max_working_bytes,
        &mut meter,
        control,
    )?;
    cancellable_sort(&mut distances, &mut distance_scratch, &mut meter, control)?;

    let mut hull = Hull::new(
        point_count,
        hash_capacity,
        maximum_halfedges,
        center,
        seed,
        points,
        requested_peak,
        limits.max_working_bytes,
        &mut meter,
        control,
    )?;

    let mut peak_working_bytes =
        phase_one_actual_bytes(&raw, &hull, &distances, &distance_scratch)?;
    require_bytes(
        peak_working_bytes,
        limits.max_working_bytes,
        "max_working_bytes",
    )?;

    for key in &distances {
        meter.charge(control)?;
        let point_index = key.index;
        if point_index == seed.0 || point_index == seed.1 || point_index == seed.2 {
            continue;
        }

        let point = points[point_index];
        let (mut edge, walk_back) = hull.find_visible_edge(point, points, &mut meter, control)?;
        if edge == EMPTY {
            return Err(TriangulationFailure::Invariant(
                "a distinct input vertex was not visible to the advancing hull",
            ));
        }

        let first_triangle = raw.add_triangle(
            edge,
            point_index,
            hull.next[edge],
            EMPTY,
            EMPTY,
            hull.triangle[edge],
        )?;
        hull.triangle[point_index] =
            raw.legalize(first_triangle + 2, points, &mut hull, &mut meter, control)?;
        hull.triangle[edge] = first_triangle;

        let mut next = hull.next[edge];
        loop {
            meter.charge(control)?;
            let after = hull.next[next];
            if orientation(point, points[next], points[after]) <= 0.0 {
                break;
            }
            let triangle = raw.add_triangle(
                next,
                point_index,
                after,
                hull.triangle[point_index],
                EMPTY,
                hull.triangle[next],
            )?;
            hull.triangle[point_index] =
                raw.legalize(triangle + 2, points, &mut hull, &mut meter, control)?;
            hull.next[next] = EMPTY;
            next = after;
        }

        if walk_back {
            loop {
                meter.charge(control)?;
                let before = hull.previous[edge];
                if orientation(point, points[before], points[edge]) <= 0.0 {
                    break;
                }
                let triangle = raw.add_triangle(
                    before,
                    point_index,
                    edge,
                    EMPTY,
                    hull.triangle[edge],
                    hull.triangle[before],
                )?;
                raw.legalize(triangle + 2, points, &mut hull, &mut meter, control)?;
                hull.triangle[before] = triangle;
                hull.next[edge] = EMPTY;
                edge = before;
            }
        }

        hull.previous[point_index] = edge;
        hull.next[point_index] = next;
        hull.previous[next] = point_index;
        hull.next[edge] = point_index;
        hull.start = edge;
        hull.hash_edge(point, point_index);
        hull.hash_edge(points[edge], edge);
    }

    drop(distances);
    drop(distance_scratch);
    drop(hull);

    let mut vertex_seen = fixed_filled_vec_cancellable(
        point_count,
        0_u8,
        requested_peak,
        limits.max_working_bytes,
        &mut meter,
        control,
    )?;
    peak_working_bytes = peak_working_bytes.max(phase_validation_actual_bytes(&raw, &vertex_seen)?);
    require_bytes(
        peak_working_bytes,
        limits.max_working_bytes,
        "max_working_bytes",
    )?;
    validate_topology(points, &raw, &mut vertex_seen, &mut meter, control)?;
    drop(vertex_seen);

    let RawTriangulation {
        triangles: raw_triangles,
        halfedges,
    } = raw;
    drop(halfedges);

    let mut triangles =
        fixed_capacity_vec::<[usize; 3]>(maximum_faces, requested_peak, limits.max_working_bytes)?;
    peak_working_bytes = peak_working_bytes.max(checked_add_bytes(
        capacity_bytes::<usize>(raw_triangles.capacity())?,
        capacity_bytes::<[usize; 3]>(triangles.capacity())?,
        limits.max_working_bytes,
    )?);
    require_bytes(
        peak_working_bytes,
        limits.max_working_bytes,
        "max_working_bytes",
    )?;

    for triangle in raw_triangles.chunks_exact(3) {
        meter.charge(control)?;
        triangles.push([triangle[0], triangle[1], triangle[2]]);
    }

    meter.checkpoint(control)?;
    Ok(TriangulationOutput {
        triangles,
        steps: meter.steps,
        peak_working_bytes,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct DistanceKey {
    index: usize,
    distance: f64,
}

impl DistanceKey {
    fn compare(self, other: Self) -> Ordering {
        self.distance
            .total_cmp(&other.distance)
            .then_with(|| self.index.cmp(&other.index))
    }
}

#[derive(Debug)]
struct WorkMeter {
    steps: u64,
    maximum: u64,
    since_cancel_check: u64,
}

impl WorkMeter {
    const fn new(maximum: u64) -> Self {
        Self {
            steps: 0,
            maximum,
            since_cancel_check: 0,
        }
    }

    fn charge(&mut self, control: &OperationControl) -> Result<(), TriangulationFailure> {
        let required = self
            .steps
            .checked_add(1)
            .ok_or(TriangulationFailure::Resource {
                resource: "max_steps",
                required: u64::MAX,
                allowed: self.maximum,
            })?;
        if required > self.maximum {
            return Err(TriangulationFailure::Resource {
                resource: "max_steps",
                required,
                allowed: self.maximum,
            });
        }
        self.steps = required;
        self.since_cancel_check += 1;
        if self.since_cancel_check >= CANCEL_INTERVAL {
            self.checkpoint(control)?;
        }
        Ok(())
    }

    fn checkpoint(&mut self, control: &OperationControl) -> Result<(), TriangulationFailure> {
        check_cancelled(control)?;
        self.since_cancel_check = 0;
        Ok(())
    }
}

#[derive(Debug)]
struct RawTriangulation {
    triangles: Vec<usize>,
    halfedges: Vec<usize>,
}

impl RawTriangulation {
    fn new(
        maximum_halfedges: usize,
        requested_peak: u64,
        allowed: u64,
    ) -> Result<Self, TriangulationFailure> {
        Ok(Self {
            triangles: fixed_capacity_vec(maximum_halfedges, requested_peak, allowed)?,
            halfedges: fixed_capacity_vec(maximum_halfedges, requested_peak, allowed)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn add_triangle(
        &mut self,
        first: usize,
        second: usize,
        third: usize,
        first_opposite: usize,
        second_opposite: usize,
        third_opposite: usize,
    ) -> Result<usize, TriangulationFailure> {
        if self.triangles.len().saturating_add(3) > self.triangles.capacity()
            || self.halfedges.len().saturating_add(3) > self.halfedges.capacity()
        {
            return Err(TriangulationFailure::Invariant(
                "planar face bound was exceeded",
            ));
        }

        let triangle = self.triangles.len();
        self.triangles.extend_from_slice(&[first, second, third]);
        self.halfedges
            .extend_from_slice(&[first_opposite, second_opposite, third_opposite]);
        self.link_opposite(first_opposite, triangle)?;
        self.link_opposite(second_opposite, triangle + 1)?;
        self.link_opposite(third_opposite, triangle + 2)?;
        Ok(triangle)
    }

    fn link_opposite(&mut self, opposite: usize, edge: usize) -> Result<(), TriangulationFailure> {
        if opposite == EMPTY {
            return Ok(());
        }
        let Some(slot) = self.halfedges.get_mut(opposite) else {
            return Err(TriangulationFailure::Invariant(
                "triangle linked an out-of-range halfedge",
            ));
        };
        *slot = edge;
        Ok(())
    }

    fn legalize(
        &mut self,
        mut edge: usize,
        points: &[PlanarPoint],
        hull: &mut Hull,
        meter: &mut WorkMeter,
        control: &OperationControl,
    ) -> Result<usize, TriangulationFailure> {
        loop {
            meter.charge(control)?;
            let opposite = *self
                .halfedges
                .get(edge)
                .ok_or(TriangulationFailure::Invariant(
                    "legalization edge was out of range",
                ))?;
            let previous = previous_halfedge(edge);

            let illegal = if opposite == EMPTY {
                false
            } else {
                let next = next_halfedge(edge);
                let opposite_previous = previous_halfedge(opposite);
                let p0 = self.triangles[previous];
                let right = self.triangles[edge];
                let left = self.triangles[next];
                let p1 = self.triangles[opposite_previous];
                edge_is_illegal(points, p0, right, left, p1)?
            };

            if !illegal {
                if let Some(next) = hull.edge_stack.pop() {
                    edge = next;
                    continue;
                }
                return Ok(previous);
            }

            let opposite_previous = previous_halfedge(opposite);
            let p1 = self.triangles[opposite_previous];
            let p0 = self.triangles[previous];
            self.triangles[edge] = p1;
            self.triangles[opposite] = p0;

            let opposite_neighbor = self.halfedges[opposite_previous];
            let previous_neighbor = self.halfedges[previous];

            if opposite_neighbor == EMPTY {
                hull.replace_boundary_edge(opposite_previous, edge, meter, control)?;
            }

            self.halfedges[edge] = opposite_neighbor;
            self.halfedges[opposite] = previous_neighbor;
            self.halfedges[previous] = opposite_previous;
            if opposite_neighbor != EMPTY {
                self.halfedges[opposite_neighbor] = edge;
            }
            if previous_neighbor != EMPTY {
                self.halfedges[previous_neighbor] = opposite;
            }
            self.halfedges[opposite_previous] = previous;

            let pending = next_halfedge(opposite);
            if hull.edge_stack.len() == hull.edge_stack.capacity() {
                return Err(TriangulationFailure::Resource {
                    resource: "legalization stack entries",
                    required: u64::try_from(hull.edge_stack.len())
                        .unwrap_or(u64::MAX)
                        .saturating_add(1),
                    allowed: u64::try_from(hull.edge_stack.capacity()).unwrap_or(u64::MAX),
                });
            }
            hull.edge_stack.push(pending);
        }
    }
}

#[derive(Debug)]
struct Hull {
    previous: Vec<usize>,
    next: Vec<usize>,
    triangle: Vec<usize>,
    hash: Vec<usize>,
    edge_stack: Vec<usize>,
    start: usize,
    center: PlanarPoint,
}

impl Hull {
    #[allow(clippy::too_many_arguments)]
    fn new(
        point_count: usize,
        hash_capacity: usize,
        edge_stack_capacity: usize,
        center: PlanarPoint,
        seed: (usize, usize, usize),
        points: &[PlanarPoint],
        requested_peak: u64,
        allowed: u64,
        meter: &mut WorkMeter,
        control: &OperationControl,
    ) -> Result<Self, TriangulationFailure> {
        let mut hull = Self {
            previous: fixed_filled_vec_cancellable(
                point_count,
                EMPTY,
                requested_peak,
                allowed,
                meter,
                control,
            )?,
            next: fixed_filled_vec_cancellable(
                point_count,
                EMPTY,
                requested_peak,
                allowed,
                meter,
                control,
            )?,
            triangle: fixed_filled_vec_cancellable(
                point_count,
                EMPTY,
                requested_peak,
                allowed,
                meter,
                control,
            )?,
            hash: fixed_filled_vec_cancellable(
                hash_capacity,
                EMPTY,
                requested_peak,
                allowed,
                meter,
                control,
            )?,
            edge_stack: fixed_capacity_vec(edge_stack_capacity, requested_peak, allowed)?,
            start: seed.0,
            center,
        };

        let (first, second, third) = seed;
        hull.next[first] = second;
        hull.previous[third] = second;
        hull.next[second] = third;
        hull.previous[first] = third;
        hull.next[third] = first;
        hull.previous[second] = first;
        hull.triangle[first] = 0;
        hull.triangle[second] = 1;
        hull.triangle[third] = 2;
        hull.hash_edge(points[first], first);
        hull.hash_edge(points[second], second);
        hull.hash_edge(points[third], third);
        Ok(hull)
    }

    fn hash_edge(&mut self, point: PlanarPoint, edge: usize) {
        let key = self.hash_key(point);
        self.hash[key] = edge;
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn hash_key(&self, point: PlanarPoint) -> usize {
        let delta_x = point.x - self.center.x;
        let delta_y = point.y - self.center.y;
        let denominator = delta_x.abs() + delta_y.abs();
        if denominator == 0.0 {
            return 0;
        }
        let projection = delta_x / denominator;
        let angle = (if delta_y > 0.0 {
            3.0 - projection
        } else {
            1.0 + projection
        }) / 4.0;
        ((self.hash.len() as f64 * angle).floor() as usize) % self.hash.len()
    }

    fn find_visible_edge(
        &self,
        point: PlanarPoint,
        points: &[PlanarPoint],
        meter: &mut WorkMeter,
        control: &OperationControl,
    ) -> Result<(usize, bool), TriangulationFailure> {
        let key = self.hash_key(point);
        let mut candidate = EMPTY;
        for offset in 0..self.hash.len() {
            meter.charge(control)?;
            let possible = self.hash[(key + offset) % self.hash.len()];
            if possible != EMPTY && self.next[possible] != EMPTY {
                candidate = possible;
                break;
            }
        }
        if candidate == EMPTY {
            return Err(TriangulationFailure::Invariant(
                "advancing hull hash contained no live edge",
            ));
        }

        let start = self.previous[candidate];
        if start == EMPTY {
            return Err(TriangulationFailure::Invariant(
                "advancing hull predecessor was missing",
            ));
        }
        let mut edge = start;
        loop {
            meter.charge(control)?;
            let next = self.next[edge];
            if next == EMPTY {
                return Err(TriangulationFailure::Invariant(
                    "advancing hull walk reached a removed edge",
                ));
            }
            if orientation(point, points[edge], points[next]) > 0.0 {
                return Ok((edge, edge == start));
            }
            edge = next;
            if edge == start {
                return Ok((EMPTY, false));
            }
        }
    }

    fn replace_boundary_edge(
        &mut self,
        old: usize,
        replacement: usize,
        meter: &mut WorkMeter,
        control: &OperationControl,
    ) -> Result<(), TriangulationFailure> {
        let mut edge = self.start;
        loop {
            meter.charge(control)?;
            if self.triangle[edge] == old {
                self.triangle[edge] = replacement;
                return Ok(());
            }
            edge = self.previous[edge];
            if edge == EMPTY {
                return Err(TriangulationFailure::Invariant(
                    "boundary repair reached a missing predecessor",
                ));
            }
            if edge == self.start {
                return Err(TriangulationFailure::Invariant(
                    "boundary repair could not find the replaced edge",
                ));
            }
        }
    }
}

impl PlanarPoint {
    fn squared_distance(self, other: Self) -> f64 {
        let delta_x = self.x - other.x;
        let delta_y = self.y - other.y;
        delta_x * delta_x + delta_y * delta_y
    }

    fn circumcenter(self, second: Self, third: Self) -> Result<Self, TriangulationFailure> {
        let delta_x = second.x - self.x;
        let delta_y = second.y - self.y;
        let other_x = third.x - self.x;
        let other_y = third.y - self.y;
        let second_length = delta_x * delta_x + delta_y * delta_y;
        let third_length = other_x * other_x + other_y * other_y;
        let denominator = 2.0 * (delta_x * other_y - delta_y * other_x);
        if denominator == 0.0 || !denominator.is_finite() {
            return Err(TriangulationFailure::Invariant(
                "seed circumcenter was not finite",
            ));
        }
        let x = self.x + (other_y * second_length - delta_y * third_length) / denominator;
        let y = self.y + (delta_x * third_length - other_x * second_length) / denominator;
        if !x.is_finite() || !y.is_finite() {
            return Err(TriangulationFailure::Invariant(
                "seed circumcenter was not finite",
            ));
        }
        Ok(Self { x, y })
    }

    fn squared_circumradius(self, second: Self, third: Self) -> Result<f64, TriangulationFailure> {
        let center = self.circumcenter(second, third)?;
        let radius = self.squared_distance(center);
        if radius.is_finite() {
            Ok(radius)
        } else {
            Err(TriangulationFailure::Invariant(
                "seed circumradius was not finite",
            ))
        }
    }
}

fn find_seed_triangle(
    points: &[PlanarPoint],
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<Option<(usize, usize, usize)>, TriangulationFailure> {
    let center = bounding_box_center(points, meter, control)?;
    let Some(first) = closest_distinct_point(points, center, None, meter, control)? else {
        return Ok(None);
    };
    let Some(second) = closest_distinct_point(points, points[first], Some(first), meter, control)?
    else {
        return Ok(None);
    };

    let mut third = None;
    let mut minimum_radius = f64::INFINITY;
    for (index, point) in points.iter().copied().enumerate() {
        meter.charge(control)?;
        if index == first || index == second {
            continue;
        }
        if orientation(points[first], points[second], point) == 0.0 {
            continue;
        }
        let radius = points[first].squared_circumradius(points[second], point)?;
        if radius.total_cmp(&minimum_radius) == Ordering::Less
            || (radius.to_bits() == minimum_radius.to_bits()
                && third.is_none_or(|current| index < current))
        {
            third = Some(index);
            minimum_radius = radius;
        }
    }

    let Some(third) = third else {
        return Ok(None);
    };
    if orientation(points[first], points[second], points[third]) > 0.0 {
        Ok(Some((first, third, second)))
    } else {
        Ok(Some((first, second, third)))
    }
}

fn bounding_box_center(
    points: &[PlanarPoint],
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<PlanarPoint, TriangulationFailure> {
    let first = points[0];
    let mut minimum_x = first.x;
    let mut minimum_y = first.y;
    let mut maximum_x = first.x;
    let mut maximum_y = first.y;
    for point in points.iter().copied().skip(1) {
        meter.charge(control)?;
        minimum_x = minimum_x.min(point.x);
        minimum_y = minimum_y.min(point.y);
        maximum_x = maximum_x.max(point.x);
        maximum_y = maximum_y.max(point.y);
    }
    let x = minimum_x + (maximum_x - minimum_x) / 2.0;
    let y = minimum_y + (maximum_y - minimum_y) / 2.0;
    if !x.is_finite() || !y.is_finite() {
        return Err(TriangulationFailure::Invariant(
            "input bounding-box center was not finite",
        ));
    }
    Ok(PlanarPoint { x, y })
}

fn closest_distinct_point(
    points: &[PlanarPoint],
    target: PlanarPoint,
    excluded: Option<usize>,
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<Option<usize>, TriangulationFailure> {
    let mut closest = None;
    let mut minimum_distance = f64::INFINITY;
    for (index, point) in points.iter().copied().enumerate() {
        meter.charge(control)?;
        if excluded == Some(index) {
            continue;
        }
        let distance = target.squared_distance(point);
        if !distance.is_finite() {
            return Err(TriangulationFailure::Invariant(
                "point distance was not finite",
            ));
        }
        if distance > 0.0
            && (distance.total_cmp(&minimum_distance) == Ordering::Less
                || (distance.to_bits() == minimum_distance.to_bits()
                    && closest.is_none_or(|current| index < current)))
        {
            closest = Some(index);
            minimum_distance = distance;
        }
    }
    Ok(closest)
}

fn distance_order(
    points: &[PlanarPoint],
    center: PlanarPoint,
    requested_peak: u64,
    allowed: u64,
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<(Vec<DistanceKey>, Vec<DistanceKey>), TriangulationFailure> {
    let mut keys = fixed_capacity_vec(points.len(), requested_peak, allowed)?;
    for (index, point) in points.iter().copied().enumerate() {
        meter.charge(control)?;
        let distance = center.squared_distance(point);
        if !distance.is_finite() {
            return Err(TriangulationFailure::Invariant(
                "radial sort distance was not finite",
            ));
        }
        keys.push(DistanceKey { index, distance });
    }
    let scratch = fixed_filled_vec_cancellable(
        points.len(),
        DistanceKey::default(),
        requested_peak,
        allowed,
        meter,
        control,
    )?;
    Ok((keys, scratch))
}

fn cancellable_sort(
    values: &mut [DistanceKey],
    scratch: &mut [DistanceKey],
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<(), TriangulationFailure> {
    let output = crate::sort::merge_sort_by(values, scratch, DistanceKey::compare, || {
        meter.charge(control)
    })
    .map_err(|error| match error {
        crate::sort::MergeSortError::ScratchLength => {
            TriangulationFailure::Invariant("distance-sort scratch length differs from its input")
        }
        crate::sort::MergeSortError::Step(error) => error,
    })?;
    if output == crate::sort::MergeSortOutput::Scratch {
        for (output, input) in values.iter_mut().zip(scratch.iter().copied()) {
            meter.charge(control)?;
            *output = input;
        }
    }
    Ok(())
}

fn validate_topology(
    points: &[PlanarPoint],
    triangulation: &RawTriangulation,
    seen: &mut [u8],
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<(), TriangulationFailure> {
    if triangulation.triangles.is_empty()
        || !triangulation.triangles.len().is_multiple_of(3)
        || triangulation.triangles.len() != triangulation.halfedges.len()
    {
        return Err(TriangulationFailure::Invariant(
            "triangulator produced malformed face columns",
        ));
    }

    for triangle in triangulation.triangles.chunks_exact(3) {
        meter.charge(control)?;
        let [first, second, third] = [triangle[0], triangle[1], triangle[2]];
        if first >= points.len() || second >= points.len() || third >= points.len() {
            return Err(TriangulationFailure::Invariant(
                "face referenced an out-of-range vertex",
            ));
        }
        if first == second || second == third || third == first {
            return Err(TriangulationFailure::Invariant("face repeated a vertex"));
        }
        if orientation(points[first], points[second], points[third]) == 0.0 {
            return Err(TriangulationFailure::Invariant(
                "face had zero horizontal area",
            ));
        }
        seen[first] = 1;
        seen[second] = 1;
        seen[third] = 1;
    }

    for was_seen in seen.iter().copied() {
        meter.charge(control)?;
        if was_seen == 0 {
            return Err(TriangulationFailure::Invariant(
                "triangulator omitted a distinct input vertex",
            ));
        }
    }

    for edge in 0..triangulation.halfedges.len() {
        meter.charge(control)?;
        let opposite = triangulation.halfedges[edge];
        if opposite == EMPTY {
            continue;
        }
        if opposite >= triangulation.halfedges.len() || triangulation.halfedges[opposite] != edge {
            return Err(TriangulationFailure::Invariant(
                "halfedge adjacency was not reciprocal",
            ));
        }
        let from = triangulation.triangles[edge];
        let to = triangulation.triangles[next_halfedge(edge)];
        if triangulation.triangles[opposite] != to
            || triangulation.triangles[next_halfedge(opposite)] != from
        {
            return Err(TriangulationFailure::Invariant(
                "adjacent halfedges did not reverse one edge",
            ));
        }
        if edge < opposite {
            let left = triangulation.triangles[previous_halfedge(edge)];
            let right = triangulation.triangles[previous_halfedge(opposite)];
            if edge_is_illegal(points, left, from, to, right)? {
                return Err(TriangulationFailure::Invariant(
                    "an interior edge violated the canonical Delaunay rule",
                ));
            }
        }
    }
    Ok(())
}

fn edge_is_illegal(
    points: &[PlanarPoint],
    first: usize,
    diagonal_first: usize,
    diagonal_second: usize,
    opposite: usize,
) -> Result<bool, TriangulationFailure> {
    let a = points[first];
    let b = points[diagonal_first];
    let c = points[diagonal_second];
    let d = points[opposite];
    let turn = orientation(a, b, c);
    if turn == 0.0 {
        return Err(TriangulationFailure::Invariant(
            "legalization encountered a collinear face",
        ));
    }
    let circle = incircle(coordinate(a), coordinate(b), coordinate(c), coordinate(d));
    let inside = if turn > 0.0 {
        circle > 0.0
    } else {
        circle < 0.0
    };
    if inside {
        return Ok(true);
    }
    if circle == 0.0 {
        let current = canonical_edge(diagonal_first, diagonal_second);
        let alternative = canonical_edge(first, opposite);
        return Ok(alternative < current);
    }
    Ok(false)
}

fn canonical_edge(first: usize, second: usize) -> (usize, usize) {
    (first.min(second), first.max(second))
}

fn validate_points(
    points: &[PlanarPoint],
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<(), TriangulationFailure> {
    for point in points {
        meter.charge(control)?;
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(TriangulationFailure::Invariant(
                "planar kernel input must be finite",
            ));
        }
    }
    Ok(())
}

fn all_collinear(
    points: &[PlanarPoint],
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<bool, TriangulationFailure> {
    let first = points[0];
    let mut second = None;
    for point in points.iter().copied().skip(1) {
        meter.charge(control)?;
        if point != first {
            second = Some(point);
            break;
        }
    }
    let Some(second) = second else {
        return Ok(true);
    };
    for point in points.iter().copied() {
        meter.charge(control)?;
        if orientation(first, second, point) != 0.0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn coordinate(point: PlanarPoint) -> Coord<f64> {
    Coord {
        x: point.x,
        y: point.y,
    }
}

fn orientation(first: PlanarPoint, second: PlanarPoint, third: PlanarPoint) -> f64 {
    orient2d(coordinate(first), coordinate(second), coordinate(third))
}

fn next_halfedge(edge: usize) -> usize {
    if edge % 3 == 2 { edge - 2 } else { edge + 1 }
}

fn previous_halfedge(edge: usize) -> usize {
    if edge.is_multiple_of(3) {
        edge + 2
    } else {
        edge - 1
    }
}

fn check_cancelled(control: &OperationControl) -> Result<(), TriangulationFailure> {
    control
        .check_cancelled()
        .map_err(|_| TriangulationFailure::Cancelled)
}

fn requested_peak_bytes(
    point_count: usize,
    maximum_faces: usize,
    maximum_halfedges: usize,
    hash_capacity: usize,
    allowed: u64,
) -> Result<u64, TriangulationFailure> {
    let triangulation_phase = checked_sum_bytes(
        &[
            capacity_bytes::<DistanceKey>(point_count)?,
            capacity_bytes::<DistanceKey>(point_count)?,
            capacity_bytes::<usize>(point_count)?,
            capacity_bytes::<usize>(point_count)?,
            capacity_bytes::<usize>(point_count)?,
            capacity_bytes::<usize>(hash_capacity)?,
            capacity_bytes::<usize>(maximum_halfedges)?,
            capacity_bytes::<usize>(maximum_halfedges)?,
            capacity_bytes::<usize>(maximum_halfedges)?,
        ],
        allowed,
    )?;
    let validation_phase = checked_sum_bytes(
        &[
            capacity_bytes::<usize>(maximum_halfedges)?,
            capacity_bytes::<usize>(maximum_halfedges)?,
            capacity_bytes::<u8>(point_count)?,
        ],
        allowed,
    )?;
    let output_phase = checked_sum_bytes(
        &[
            capacity_bytes::<usize>(maximum_halfedges)?,
            capacity_bytes::<[usize; 3]>(maximum_faces)?,
        ],
        allowed,
    )?;
    Ok(triangulation_phase.max(validation_phase).max(output_phase))
}

fn phase_one_actual_bytes(
    raw: &RawTriangulation,
    hull: &Hull,
    distances: &Vec<DistanceKey>,
    scratch: &Vec<DistanceKey>,
) -> Result<u64, TriangulationFailure> {
    checked_sum_bytes(
        &[
            capacity_bytes::<DistanceKey>(distances.capacity())?,
            capacity_bytes::<DistanceKey>(scratch.capacity())?,
            capacity_bytes::<usize>(hull.previous.capacity())?,
            capacity_bytes::<usize>(hull.next.capacity())?,
            capacity_bytes::<usize>(hull.triangle.capacity())?,
            capacity_bytes::<usize>(hull.hash.capacity())?,
            capacity_bytes::<usize>(hull.edge_stack.capacity())?,
            capacity_bytes::<usize>(raw.triangles.capacity())?,
            capacity_bytes::<usize>(raw.halfedges.capacity())?,
        ],
        u64::MAX,
    )
}

fn phase_validation_actual_bytes(
    raw: &RawTriangulation,
    seen: &Vec<u8>,
) -> Result<u64, TriangulationFailure> {
    checked_sum_bytes(
        &[
            capacity_bytes::<usize>(raw.triangles.capacity())?,
            capacity_bytes::<usize>(raw.halfedges.capacity())?,
            capacity_bytes::<u8>(seen.capacity())?,
        ],
        u64::MAX,
    )
}

fn checked_sum_bytes(values: &[u64], allowed: u64) -> Result<u64, TriangulationFailure> {
    values.iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or_else(|| resource_overflow("max_working_bytes", allowed))
    })
}

fn checked_add_bytes(first: u64, second: u64, allowed: u64) -> Result<u64, TriangulationFailure> {
    first
        .checked_add(second)
        .ok_or_else(|| resource_overflow("max_working_bytes", allowed))
}

fn capacity_bytes<T>(capacity: usize) -> Result<u64, TriangulationFailure> {
    let capacity =
        u64::try_from(capacity).map_err(|_| resource_overflow("max_working_bytes", u64::MAX))?;
    let element = u64::try_from(mem::size_of::<T>())
        .map_err(|_| resource_overflow("max_working_bytes", u64::MAX))?;
    capacity
        .checked_mul(element)
        .ok_or_else(|| resource_overflow("max_working_bytes", u64::MAX))
}

fn require_bytes(
    required: u64,
    allowed: u64,
    resource: &'static str,
) -> Result<(), TriangulationFailure> {
    if required > allowed {
        Err(TriangulationFailure::Resource {
            resource,
            required,
            allowed,
        })
    } else {
        Ok(())
    }
}

fn resource_overflow(resource: &'static str, allowed: u64) -> TriangulationFailure {
    TriangulationFailure::Resource {
        resource,
        required: u64::MAX,
        allowed,
    }
}

fn fixed_capacity_vec<T>(
    capacity: usize,
    requested_peak: u64,
    allowed: u64,
) -> Result<Vec<T>, TriangulationFailure> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| TriangulationFailure::Resource {
            resource: "allocation",
            required: requested_peak,
            allowed,
        })?;
    if mem::size_of::<T>() != 0 && values.capacity() > capacity {
        return Err(TriangulationFailure::Resource {
            resource: "fixed arena capacity bytes",
            required: capacity_bytes::<T>(values.capacity()).unwrap_or(u64::MAX),
            allowed: capacity_bytes::<T>(capacity).unwrap_or(u64::MAX),
        });
    }
    Ok(values)
}

fn fixed_filled_vec_cancellable<T: Clone>(
    capacity: usize,
    value: T,
    requested_peak: u64,
    allowed: u64,
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<Vec<T>, TriangulationFailure> {
    let mut values = fixed_capacity_vec(capacity, requested_peak, allowed)?;
    for _ in 0..capacity {
        meter.charge(control)?;
        values.push(value.clone());
    }
    Ok(values)
}

fn integer_ceil_sqrt(
    value: usize,
    meter: &mut WorkMeter,
    control: &OperationControl,
) -> Result<usize, TriangulationFailure> {
    if value <= 1 {
        return Ok(value.max(1));
    }
    let mut low = 1_usize;
    let mut high = value;
    while low < high {
        meter.charge(control)?;
        let middle = low + (high - low) / 2;
        if middle >= value / middle && middle.saturating_mul(middle) >= value {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    Ok(low)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENEROUS: TriangulationLimits = TriangulationLimits {
        max_working_bytes: 64 * 1024 * 1024,
        max_steps: 10_000_000,
    };

    fn point(x: f64, y: f64) -> PlanarPoint {
        PlanarPoint { x, y }
    }

    fn undirected_edges(triangles: &[[usize; 3]]) -> Vec<(usize, usize)> {
        let mut edges = Vec::new();
        for triangle in triangles {
            for (first, second) in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                edges.push(canonical_edge(first, second));
            }
        }
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    #[test]
    fn triangulates_one_triangle_with_bounded_accounting() {
        let control = OperationControl::new();
        let output = triangulate(
            &[point(0.0, 0.0), point(0.0, 1.0), point(1.0, 0.0)],
            GENEROUS,
            &control,
        )
        .expect("triangle triangulates");

        assert_eq!(output.triangles.len(), 1);
        let mut vertices = output.triangles[0];
        vertices.sort_unstable();
        assert_eq!(vertices, [0, 1, 2]);
        assert!(output.steps > 0);
        assert!(output.peak_working_bytes > 0);
        assert!(output.peak_working_bytes <= GENEROUS.max_working_bytes);
    }

    #[test]
    fn cocircular_square_uses_lexicographically_smaller_diagonal() {
        let control = OperationControl::new();
        let output = triangulate(
            &[
                point(0.0, 0.0),
                point(0.0, 1.0),
                point(1.0, 0.0),
                point(1.0, 1.0),
            ],
            GENEROUS,
            &control,
        )
        .expect("square triangulates");
        let edges = undirected_edges(&output.triangles);

        assert!(edges.contains(&(0, 3)));
        assert!(!edges.contains(&(1, 2)));
    }

    #[test]
    fn equal_inputs_repeat_exact_topology_and_accounting() {
        let points = [
            point(-2.0, -1.0),
            point(-1.0, 3.0),
            point(0.0, 0.0),
            point(2.0, -2.0),
            point(3.0, 2.0),
            point(4.0, 0.5),
        ];
        let first =
            triangulate(&points, GENEROUS, &OperationControl::new()).expect("first triangulation");
        let second =
            triangulate(&points, GENEROUS, &OperationControl::new()).expect("second triangulation");

        assert_eq!(first, second);
    }

    #[test]
    fn representative_clouds_match_the_pinned_delaunator_oracle() {
        for point_count in [3_usize, 4, 8, 31, 100, 257] {
            let mut state = 0x9e37_79b9_u32;
            let mut points = Vec::with_capacity(point_count);
            for index in 0..point_count {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = f64::from(state) / f64::from(u32::MAX);
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let index = u32::try_from(index).expect("fixture size fits u32");
                let y = f64::from(state) / f64::from(u32::MAX) + f64::from(index) * 1.0e-12;
                points.push(point(x, y));
            }

            let output = triangulate(&points, GENEROUS, &OperationControl::new())
                .expect("representative cloud triangulates");
            let oracle_points: Vec<_> = points
                .iter()
                .map(|point| delaunator::Point {
                    x: point.x,
                    y: point.y,
                })
                .collect();
            let oracle = delaunator::triangulate(&oracle_points);

            let mut actual_faces: Vec<_> = output
                .triangles
                .into_iter()
                .map(|mut face| {
                    face.sort_unstable();
                    face
                })
                .collect();
            let mut oracle_faces: Vec<_> = oracle
                .triangles
                .chunks_exact(3)
                .map(|face| {
                    let mut face = [face[0], face[1], face[2]];
                    face.sort_unstable();
                    face
                })
                .collect();
            actual_faces.sort_unstable();
            oracle_faces.sort_unstable();
            assert_eq!(actual_faces, oracle_faces, "point_count={point_count}");
        }
    }

    #[test]
    fn collinear_input_is_explicit() {
        let error = triangulate(
            &[point(0.0, 0.0), point(1.0, 1.0), point(2.0, 2.0)],
            GENEROUS,
            &OperationControl::new(),
        )
        .expect_err("collinear input fails");
        assert_eq!(error, TriangulationFailure::Collinear);
    }

    #[test]
    fn working_memory_is_preflighted_before_triangulation() {
        let error = triangulate(
            &[point(0.0, 0.0), point(0.0, 1.0), point(1.0, 0.0)],
            TriangulationLimits {
                max_working_bytes: 0,
                max_steps: GENEROUS.max_steps,
            },
            &OperationControl::new(),
        )
        .expect_err("zero memory fails");
        assert!(matches!(
            error,
            TriangulationFailure::Resource {
                resource: "max_working_bytes",
                ..
            }
        ));
    }

    #[test]
    fn topology_work_is_hard_bounded() {
        let error = triangulate(
            &[
                point(0.0, 0.0),
                point(0.0, 1.0),
                point(1.0, 0.0),
                point(1.0, 1.0),
            ],
            TriangulationLimits {
                max_working_bytes: GENEROUS.max_working_bytes,
                max_steps: 1,
            },
            &OperationControl::new(),
        )
        .expect_err("tiny work ceiling fails");
        assert!(matches!(
            error,
            TriangulationFailure::Resource {
                resource: "max_steps",
                ..
            }
        ));
    }

    #[test]
    fn cancellation_is_observed_before_allocation() {
        let control = OperationControl::new();
        control.cancel();
        let error = triangulate(
            &[point(0.0, 0.0), point(0.0, 1.0), point(1.0, 0.0)],
            GENEROUS,
            &control,
        )
        .expect_err("cancelled triangulation fails");
        assert_eq!(error, TriangulationFailure::Cancelled);
    }

    #[test]
    fn rejects_non_finite_kernel_input() {
        let error = triangulate(
            &[point(0.0, 0.0), point(0.0, 1.0), point(f64::NAN, 0.0)],
            GENEROUS,
            &OperationControl::new(),
        )
        .expect_err("non-finite input fails");
        assert!(matches!(error, TriangulationFailure::Invariant(_)));
    }
}
