use std::{cmp::Ordering, mem, sync::Arc};

use blake3::Hasher;
use foundation_runtime::{OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::{ContentHash, CoordinateReference, PointId, PositionTransform, WorldBounds};
use point_workspace::{PointQuery, Snapshot, SnapshotPointSummary};
use robust::Coord;

use crate::{
    SurfaceFace, SurfaceFaceId, SurfaceVertex, SurfaceVertexId, TerrainDescriptor, TerrainError,
    TerrainLimits, TerrainRecipe, TerrainSurface,
    model::SurfaceData,
    triangulation::{
        PlanarPoint, TriangulationFailure, TriangulationLimits, TriangulationOutput, triangulate,
    },
};

const CANCEL_STRIDE: u64 = 1_024;
const PROGRESS_STRIDE: u64 = 4_096;
const MAX_EXACT_F64_INTEGER: i128 = 1_i128 << 53;
const GEOMETRY_HASH_DOMAIN: &[u8] = b"punctra-terrain-geometry-v1";
const TOPOLOGY_HASH_DOMAIN: &[u8] = b"punctra-terrain-topology-v1";
const RECIPE_HASH_DOMAIN: &[u8] = b"punctra-terrain-recipe-v1";
const ARTIFACT_HASH_DOMAIN: &[u8] = b"punctra-terrain-artifact-v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct InputVertex {
    ticks: [i64; 3],
    point: PointId,
}

#[derive(Default)]
struct WorkMeter {
    used: u64,
    next_cancel: u64,
    next_progress: u64,
}

impl WorkMeter {
    fn new() -> Self {
        Self {
            used: 0,
            next_cancel: CANCEL_STRIDE,
            next_progress: PROGRESS_STRIDE,
        }
    }

    fn charge(
        &mut self,
        amount: u64,
        limit: u64,
        control: &OperationControl,
    ) -> Result<(), TerrainError> {
        let required = self.used.saturating_add(amount);
        if required > limit {
            return Err(TerrainError::resource(
                "Terrain Derivation work units",
                required,
                limit,
            ));
        }
        self.used = required;
        if self.used >= self.next_cancel {
            control.check_cancelled()?;
            self.next_cancel = self.used.saturating_add(CANCEL_STRIDE);
        }
        if self.used >= self.next_progress {
            control.report_progress(ProgressSnapshot::new(
                ProgressPhase::RUNNING,
                self.used,
                None,
            )?)?;
            self.next_progress = self.used.saturating_add(PROGRESS_STRIDE);
        }
        Ok(())
    }
}

struct MemoryMeter {
    baseline: u64,
    peak: u64,
}

impl MemoryMeter {
    const fn new(baseline: u64) -> Self {
        Self { baseline, peak: 0 }
    }

    fn require(
        &mut self,
        required: u64,
        allowed: u64,
        limit: &'static str,
    ) -> Result<(), TerrainError> {
        let total = self.baseline.saturating_add(required);
        if total > allowed {
            return Err(TerrainError::resource(limit, total, allowed));
        }
        self.peak = self.peak.max(total);
        Ok(())
    }
}

/// Starts one deterministic single-worker Terrain Derivation.
#[must_use]
pub fn derive(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
    limits: TerrainLimits,
) -> crate::TerrainJob {
    crate::TerrainJob::spawn(move |control| run(&snapshot, recipe, limits, &control))
}

#[allow(clippy::too_many_lines)]
fn run(
    snapshot: &Snapshot,
    recipe: TerrainRecipe,
    limits: TerrainLimits,
    control: &OperationControl,
) -> Result<TerrainSurface, TerrainError> {
    control.check_cancelled()?;
    let mut work = WorkMeter::new();
    if limits.point_rows().max_working_bytes() > limits.max_working_bytes() {
        return Err(TerrainError::resource(
            "Ground Input allocation bytes",
            limits.point_rows().max_working_bytes(),
            limits.max_working_bytes(),
        ));
    }
    let query = match recipe.bounds() {
        Some(bounds) => PointQuery::within(bounds),
        None => PointQuery::all(),
    }
    .classification_is(recipe.ground_classification());
    let mut rows = snapshot.point_rows(query, limits.point_rows())?;
    let transform = rows.source_metadata().position_transform();
    let coordinate_reference_bytes = rows
        .source_metadata()
        .coordinate_reference()
        .as_wkt()
        .map_or(0, |wkt| u64::try_from(wkt.len()).unwrap_or(u64::MAX));
    if coordinate_reference_bytes > limits.max_working_bytes() {
        return Err(TerrainError::resource(
            "retained Coordinate Reference bytes",
            coordinate_reference_bytes,
            limits.max_working_bytes(),
        ));
    }
    let coordinate_reference = rows.source_metadata().coordinate_reference().clone();
    let mut memory = MemoryMeter::new(coordinate_reference_bytes);
    let mut input = Vec::new();

    while let Some(batch) = pull_rows(&mut rows, control)? {
        for ((&ordinal, &ticks), &classification) in batch
            .ordinals()
            .iter()
            .zip(batch.positions().ticks())
            .zip(batch.effective_classifications())
        {
            work.charge(1, limits.max_work_units(), control)?;
            if classification != recipe.ground_classification() {
                return Err(TerrainError::topology(
                    "Snapshot Point stream returned a row outside the Ground Input predicate",
                ));
            }
            reserve_input(&mut input, limits, &mut memory)?;
            input.push(InputVertex {
                ticks,
                point: PointId::new(batch.source(), ordinal),
            });
        }
    }
    let (snapshot_provenance, input_hash, input_point_count) = {
        let summary = terminal_summary(rows.summary())?;
        (
            *summary.provenance(),
            summary.content_hash(),
            summary.exact_count(),
        )
    };
    if input_point_count != u64::try_from(input.len()).unwrap_or(u64::MAX) {
        return Err(TerrainError::topology(
            "Snapshot Point terminal count differs from retained Ground Input",
        ));
    }
    drop(rows);

    if input.len() < 3 {
        return Err(TerrainError::InsufficientGroundInput {
            actual: u64::try_from(input.len()).unwrap_or(u64::MAX),
        });
    }
    merge_sort(&mut input, 0, limits, &mut work, &mut memory, control)?;
    validate_xy(&input, limits, &mut work, control)?;
    let retained_input = vector_bytes::<InputVertex>(input.capacity());
    let (kernel, bounds) = normalize(
        &input,
        retained_input,
        transform,
        limits,
        &mut work,
        &mut memory,
        control,
    )?;
    let retained_kernel = vector_bytes::<PlanarPoint>(kernel.capacity());
    let triangulation_budget = limits.max_working_bytes().saturating_sub(
        memory
            .baseline
            .saturating_add(retained_input)
            .saturating_add(retained_kernel),
    );
    let remaining_work = limits.max_work_units().saturating_sub(work.used);
    let triangulation = triangulate(
        &kernel,
        TriangulationLimits {
            max_working_bytes: triangulation_budget,
            max_steps: remaining_work,
        },
        control,
    )
    .map_err(|error| map_triangulation_error(&error))?;
    work.charge(triangulation.steps, limits.max_work_units(), control)?;
    memory.require(
        retained_input
            .saturating_add(retained_kernel)
            .saturating_add(triangulation.peak_working_bytes),
        limits.max_working_bytes(),
        "Terrain triangulation working bytes",
    )?;

    let TriangulationOutput {
        mut triangles,
        steps: topology_steps,
        ..
    } = triangulation;
    canonicalize_faces(
        &mut triangles,
        &kernel,
        retained_kernel,
        input.len(),
        limits,
        &mut work,
        &mut memory,
        control,
    )?;
    drop(kernel);

    let mut vertices = allocate_exact::<SurfaceVertex>(
        input.len(),
        retained_input.saturating_add(vector_bytes::<[usize; 3]>(triangles.capacity())),
        limits,
        &mut memory,
        "Terrain vertex publication bytes",
    )?;
    for (index, vertex) in input.iter().enumerate() {
        work.charge(1, limits.max_work_units(), control)?;
        let id = SurfaceVertexId::from_zero_based(index).ok_or_else(|| {
            TerrainError::resource(
                "Surface vertex identity range",
                u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
                u64::from(u32::MAX),
            )
        })?;
        vertices.push(SurfaceVertex::new(id, vertex.point, vertex.ticks));
    }
    drop(input);

    let retained_vertices = vector_bytes::<SurfaceVertex>(vertices.capacity());
    let retained_triangles = vector_bytes::<[usize; 3]>(triangles.capacity());
    let mut faces = allocate_exact::<SurfaceFace>(
        triangles.len(),
        retained_vertices.saturating_add(retained_triangles),
        limits,
        &mut memory,
        "Terrain face publication bytes",
    )?;
    for (index, triangle) in triangles.iter().copied().enumerate() {
        work.charge(1, limits.max_work_units(), control)?;
        let id = SurfaceFaceId::from_zero_based(index).ok_or_else(|| {
            TerrainError::resource(
                "Surface face identity range",
                u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
                u64::from(u32::MAX),
            )
        })?;
        let vertices_for_face = [
            SurfaceVertexId::from_zero_based(triangle[0]),
            SurfaceVertexId::from_zero_based(triangle[1]),
            SurfaceVertexId::from_zero_based(triangle[2]),
        ];
        let [Some(a), Some(b), Some(c)] = vertices_for_face else {
            return Err(TerrainError::topology(
                "canonical face index exceeds the Surface identity range",
            ));
        };
        faces.push(SurfaceFace::new(id, [a, b, c]));
    }
    drop(triangles);

    let hull_vertex_count = hull_vertex_count(vertices.len(), faces.len())?;
    let (geometry_hash, topology_hash) =
        surface_hashes(transform, &vertices, &faces, limits, &mut work, control)?;
    let recipe_hash = recipe_hash(recipe);
    let artifact_hash = artifact_hash(
        snapshot_provenance,
        recipe_hash,
        transform,
        &coordinate_reference,
        input_hash,
        geometry_hash,
        topology_hash,
    );
    let retained_surface_bytes = retained_surface_bytes(&vertices, &faces, &coordinate_reference);
    if retained_surface_bytes > limits.max_surface_bytes() {
        return Err(TerrainError::resource(
            "retained Terrain Surface bytes",
            retained_surface_bytes,
            limits.max_surface_bytes(),
        ));
    }
    memory.require(
        retained_surface_bytes.saturating_sub(coordinate_reference_bytes),
        limits.max_working_bytes(),
        "Terrain publication working bytes",
    )?;
    control.check_cancelled()?;

    let descriptor = TerrainDescriptor::new(
        snapshot_provenance,
        recipe,
        recipe_hash,
        transform,
        coordinate_reference,
        input_hash,
        geometry_hash,
        topology_hash,
        artifact_hash,
        input_point_count,
        u64::try_from(vertices.len()).unwrap_or(u64::MAX),
        u64::try_from(faces.len()).unwrap_or(u64::MAX),
        hull_vertex_count,
        bounds,
        memory.peak,
        retained_surface_bytes,
        topology_steps,
    );
    let surface = TerrainSurface {
        inner: Arc::new(SurfaceData {
            descriptor,
            vertices,
            faces,
        }),
    };
    control.complete_progress(work.used)?;
    Ok(surface)
}

fn pull_rows(
    rows: &mut point_workspace::SnapshotPointBatches,
    control: &OperationControl,
) -> Result<Option<point_workspace::SnapshotPointBatch>, TerrainError> {
    if let Err(error) = control.check_cancelled() {
        rows.handle().cancel();
        return Err(error.into());
    }
    let next = rows.next().map_err(TerrainError::from)?;
    if let Err(error) = control.check_cancelled() {
        rows.handle().cancel();
        return Err(error.into());
    }
    Ok(next)
}

fn terminal_summary(
    summary: Option<&SnapshotPointSummary>,
) -> Result<&SnapshotPointSummary, TerrainError> {
    summary.ok_or_else(|| {
        TerrainError::topology("Snapshot Point stream ended without a terminal summary")
    })
}

fn reserve_input(
    input: &mut Vec<InputVertex>,
    limits: TerrainLimits,
    memory: &mut MemoryMeter,
) -> Result<(), TerrainError> {
    let count = u64::try_from(input.len()).unwrap_or(u64::MAX);
    if count >= limits.max_input_points() {
        return Err(TerrainError::resource(
            "Ground Input Points",
            count.saturating_add(1),
            limits.max_input_points(),
        ));
    }
    if input.len() < input.capacity() {
        return Ok(());
    }
    let max_capacity = usize::try_from(limits.max_input_points()).unwrap_or(usize::MAX);
    let desired = input
        .capacity()
        .max(4)
        .saturating_mul(2)
        .min(max_capacity)
        .max(input.len().saturating_add(1));
    reserve_to(
        input,
        desired,
        limits.point_rows().max_working_bytes(),
        limits.max_working_bytes(),
        memory,
        "Ground Input allocation bytes",
    )
}

fn reserve_to<T>(
    values: &mut Vec<T>,
    desired_capacity: usize,
    retained_other: u64,
    allowed: u64,
    memory: &mut MemoryMeter,
    limit: &'static str,
) -> Result<(), TerrainError> {
    if desired_capacity <= values.capacity() {
        return Ok(());
    }
    let old_bytes = vector_bytes::<T>(values.capacity());
    let requested_bytes = vector_bytes::<T>(desired_capacity);
    memory.require(
        retained_other
            .saturating_add(old_bytes)
            .saturating_add(requested_bytes),
        allowed,
        limit,
    )?;
    let additional = desired_capacity.saturating_sub(values.len());
    values.try_reserve_exact(additional).map_err(|_| {
        TerrainError::resource(
            limit,
            requested_bytes,
            allowed.saturating_sub(retained_other),
        )
    })?;
    memory.require(
        retained_other
            .saturating_add(old_bytes)
            .saturating_add(vector_bytes::<T>(values.capacity())),
        allowed,
        limit,
    )
}

fn allocate_exact<T>(
    count: usize,
    retained_other: u64,
    limits: TerrainLimits,
    memory: &mut MemoryMeter,
    limit: &'static str,
) -> Result<Vec<T>, TerrainError> {
    let mut result = Vec::new();
    reserve_to(
        &mut result,
        count,
        retained_other,
        limits.max_working_bytes(),
        memory,
        limit,
    )?;
    Ok(result)
}

fn merge_sort<T: Copy + Ord>(
    values: &mut Vec<T>,
    retained_other: u64,
    limits: TerrainLimits,
    work: &mut WorkMeter,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    if values.len() < 2 {
        return Ok(());
    }
    let retained = retained_other.saturating_add(vector_bytes::<T>(values.capacity()));
    let mut scratch = allocate_exact::<T>(
        values.len(),
        retained,
        limits,
        memory,
        "cancellable sort working bytes",
    )?;
    scratch.extend_from_slice(values);
    let output = crate::sort::merge_sort_by(
        values,
        &mut scratch,
        |left, right| left.cmp(&right),
        || work.charge(1, limits.max_work_units(), control),
    )
    .map_err(|error| match error {
        crate::sort::MergeSortError::ScratchLength => {
            TerrainError::topology("cancellable sort scratch length differs from its input")
        }
        crate::sort::MergeSortError::Step(error) => error,
    })?;
    if output == crate::sort::MergeSortOutput::Scratch {
        mem::swap(values, &mut scratch);
    }
    Ok(())
}

fn validate_xy(
    input: &[InputVertex],
    limits: TerrainLimits,
    work: &mut WorkMeter,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    let mut min_y = input[0].ticks[1];
    let mut max_y = input[0].ticks[1];
    for pair in input.windows(2) {
        work.charge(1, limits.max_work_units(), control)?;
        min_y = min_y.min(pair[1].ticks[1]);
        max_y = max_y.max(pair[1].ticks[1]);
        if pair[0].ticks[..2] == pair[1].ticks[..2] {
            return Err(TerrainError::DuplicateHorizontalPosition {
                first: pair[0].point,
                second: pair[1].point,
                conflicting_elevation: pair[0].ticks[2] != pair[1].ticks[2],
            });
        }
    }
    let min_x = i128::from(input.first().map_or(0, |vertex| vertex.ticks[0]));
    let max_x = i128::from(input.last().map_or(0, |vertex| vertex.ticks[0]));
    if max_x - min_x > MAX_EXACT_F64_INTEGER
        || i128::from(max_y) - i128::from(min_y) > MAX_EXACT_F64_INTEGER
    {
        return Err(TerrainError::numeric(
            "XY tick span exceeds the exact normalized f64 integer range",
        ));
    }
    let origin = input[0].ticks;
    let second = input[1].ticks;
    for vertex in &input[2..] {
        work.charge(1, limits.max_work_units(), control)?;
        let ax = i128::from(second[0]) - i128::from(origin[0]);
        let ay = i128::from(second[1]) - i128::from(origin[1]);
        let bx = i128::from(vertex.ticks[0]) - i128::from(origin[0]);
        let by = i128::from(vertex.ticks[1]) - i128::from(origin[1]);
        if ax * by - ay * bx != 0 {
            return Ok(());
        }
    }
    Err(TerrainError::CollinearGroundInput)
}

#[allow(clippy::cast_precision_loss)]
fn normalize(
    input: &[InputVertex],
    retained_input: u64,
    transform: PositionTransform,
    limits: TerrainLimits,
    work: &mut WorkMeter,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<(Vec<PlanarPoint>, WorldBounds), TerrainError> {
    let mut min_x = input[0].ticks[0];
    let mut max_x = input[0].ticks[0];
    let mut min_y = input[0].ticks[1];
    let mut max_y = input[0].ticks[1];
    for vertex in &input[1..] {
        work.charge(1, limits.max_work_units(), control)?;
        min_x = min_x.min(vertex.ticks[0]);
        max_x = max_x.max(vertex.ticks[0]);
        min_y = min_y.min(vertex.ticks[1]);
        max_y = max_y.max(vertex.ticks[1]);
    }
    let range_x = i128::from(max_x) - i128::from(min_x);
    let range_y = i128::from(max_y) - i128::from(min_y);
    if range_x > MAX_EXACT_F64_INTEGER || range_y > MAX_EXACT_F64_INTEGER {
        return Err(TerrainError::numeric(
            "XY tick span exceeds the exact normalized f64 integer range",
        ));
    }
    let scale = transform.scale();
    let extent_x = range_x as f64 * scale[0];
    let extent_y = range_y as f64 * scale[1];
    let common = extent_x.max(extent_y);
    if !common.is_finite() || common <= 0.0 {
        return Err(TerrainError::numeric(
            "XY world extent is not finite and positive",
        ));
    }

    let mut kernel = allocate_exact::<PlanarPoint>(
        input.len(),
        retained_input,
        limits,
        memory,
        "normalized kernel Point bytes",
    )?;
    let mut world_min = [f64::INFINITY; 3];
    let mut world_max = [f64::NEG_INFINITY; 3];
    for vertex in input {
        work.charge(1, limits.max_work_units(), control)?;
        let dx = i128::from(vertex.ticks[0]) - i128::from(min_x);
        let dy = i128::from(vertex.ticks[1]) - i128::from(min_y);
        let point = PlanarPoint {
            x: dx as f64 * scale[0] / common,
            y: dy as f64 * scale[1] / common,
        };
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(TerrainError::numeric(
                "normalized XY coordinate is non-finite",
            ));
        }
        let world = transform.world_f64(vertex.ticks);
        for axis in 0..3 {
            if !world[axis].is_finite() {
                return Err(TerrainError::numeric(
                    "Terrain world coordinate is non-finite",
                ));
            }
            world_min[axis] = world_min[axis].min(world[axis]);
            world_max[axis] = world_max[axis].max(world[axis]);
        }
        kernel.push(point);
    }
    for pair in kernel.windows(2) {
        work.charge(1, limits.max_work_units(), control)?;
        if pair[0] == pair[1] {
            return Err(TerrainError::numeric(
                "distinct XY ticks collapse to one normalized kernel coordinate",
            ));
        }
    }
    Ok((kernel, WorldBounds::new(world_min, world_max)?))
}

#[allow(clippy::too_many_arguments)]
fn canonicalize_faces(
    triangles: &mut Vec<[usize; 3]>,
    kernel: &[PlanarPoint],
    retained_kernel: u64,
    vertex_count: usize,
    limits: TerrainLimits,
    work: &mut WorkMeter,
    memory: &mut MemoryMeter,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    let face_count = u64::try_from(triangles.len()).unwrap_or(u64::MAX);
    if face_count > limits.max_faces() {
        return Err(TerrainError::resource(
            "Terrain faces",
            face_count,
            limits.max_faces(),
        ));
    }
    for triangle in triangles.iter_mut() {
        work.charge(1, limits.max_work_units(), control)?;
        if triangle.iter().any(|&index| index >= vertex_count)
            || triangle[0] == triangle[1]
            || triangle[1] == triangle[2]
            || triangle[0] == triangle[2]
        {
            return Err(TerrainError::topology(
                "triangulator returned an invalid face index",
            ));
        }
        let orientation = robust::orient2d(
            coord(kernel[triangle[0]]),
            coord(kernel[triangle[1]]),
            coord(kernel[triangle[2]]),
        );
        match orientation.partial_cmp(&0.0) {
            Some(Ordering::Less) => triangle.swap(1, 2),
            Some(Ordering::Greater) => {}
            _ => {
                return Err(TerrainError::topology(
                    "triangulator returned a zero-area face",
                ));
            }
        }
        let minimum = triangle
            .iter()
            .enumerate()
            .min_by_key(|(_, index)| *index)
            .map_or(0, |(position, _)| position);
        triangle.rotate_left(minimum);
    }
    merge_sort(triangles, retained_kernel, limits, work, memory, control)?;
    for pair in triangles.windows(2) {
        work.charge(1, limits.max_work_units(), control)?;
        if pair[0] == pair[1] {
            return Err(TerrainError::topology(
                "triangulator returned a duplicate canonical face",
            ));
        }
    }

    let retained = vector_bytes::<[usize; 3]>(triangles.capacity()).saturating_add(retained_kernel);
    let mut used = allocate_exact::<bool>(
        vertex_count,
        retained,
        limits,
        memory,
        "Terrain vertex-coverage validation bytes",
    )?;
    used.resize(vertex_count, false);
    for triangle in triangles.iter() {
        for &index in triangle {
            work.charge(1, limits.max_work_units(), control)?;
            used[index] = true;
        }
    }
    for &is_used in &used {
        work.charge(1, limits.max_work_units(), control)?;
        if !is_used {
            return Err(TerrainError::topology(
                "triangulator omitted a Ground Input vertex",
            ));
        }
    }
    Ok(())
}

const fn coord(point: PlanarPoint) -> Coord<f64> {
    Coord {
        x: point.x,
        y: point.y,
    }
}

fn map_triangulation_error(error: &TriangulationFailure) -> TerrainError {
    match error {
        TriangulationFailure::Cancelled => TerrainError::Cancelled,
        TriangulationFailure::Resource {
            resource,
            required,
            allowed,
        } => TerrainError::resource(resource, *required, *allowed),
        TriangulationFailure::Collinear => TerrainError::CollinearGroundInput,
        TriangulationFailure::Invariant(reason) => TerrainError::topology(*reason),
    }
}

fn hull_vertex_count(vertices: usize, faces: usize) -> Result<u64, TerrainError> {
    let vertices = u64::try_from(vertices).unwrap_or(u64::MAX);
    let faces = u64::try_from(faces).unwrap_or(u64::MAX);
    vertices
        .checked_mul(2)
        .and_then(|twice| twice.checked_sub(2))
        .and_then(|twice_minus_two| twice_minus_two.checked_sub(faces))
        .filter(|&hull| hull >= 3 && hull <= vertices)
        .ok_or_else(|| {
            TerrainError::topology("face count violates planar triangulation Euler facts")
        })
}

fn surface_hashes(
    transform: PositionTransform,
    vertices: &[SurfaceVertex],
    faces: &[SurfaceFace],
    limits: TerrainLimits,
    work: &mut WorkMeter,
    control: &OperationControl,
) -> Result<(ContentHash, ContentHash), TerrainError> {
    let mut geometry = domain_hasher(GEOMETRY_HASH_DOMAIN);
    let mut topology = domain_hasher(TOPOLOGY_HASH_DOMAIN);
    hash_transform(&mut geometry, transform);
    geometry.update(
        &u64::try_from(vertices.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    topology.update(
        &u64::try_from(vertices.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for vertex in vertices {
        work.charge(1, limits.max_work_units(), control)?;
        geometry.update(&vertex.id().get().to_le_bytes());
        geometry.update(vertex.point().source().as_bytes());
        geometry.update(&vertex.point().ordinal().to_le_bytes());
        for tick in vertex.ticks() {
            geometry.update(&tick.to_le_bytes());
        }
    }
    geometry.update(&u64::try_from(faces.len()).unwrap_or(u64::MAX).to_le_bytes());
    topology.update(&u64::try_from(faces.len()).unwrap_or(u64::MAX).to_le_bytes());
    for face in faces {
        work.charge(1, limits.max_work_units(), control)?;
        geometry.update(&face.id().get().to_le_bytes());
        topology.update(&face.id().get().to_le_bytes());
        for vertex in face.vertices() {
            geometry.update(&vertex.get().to_le_bytes());
            topology.update(&vertex.get().to_le_bytes());
        }
    }
    Ok((
        ContentHash::new(*geometry.finalize().as_bytes()),
        ContentHash::new(*topology.finalize().as_bytes()),
    ))
}

#[allow(clippy::too_many_arguments)]
fn artifact_hash(
    provenance: point_workspace::SnapshotProvenance,
    recipe_hash: ContentHash,
    transform: PositionTransform,
    coordinate_reference: &CoordinateReference,
    input_hash: ContentHash,
    geometry_hash: ContentHash,
    topology_hash: ContentHash,
) -> ContentHash {
    let mut hasher = domain_hasher(ARTIFACT_HASH_DOMAIN);
    hasher.update(&crate::ALGORITHM_VERSION.to_le_bytes());
    hasher.update(provenance.workspace().as_bytes());
    hasher.update(provenance.source().as_bytes());
    hasher.update(provenance.revision().as_bytes());
    hasher.update(recipe_hash.as_bytes());
    hash_transform(&mut hasher, transform);
    if let Some(wkt) = coordinate_reference.as_wkt() {
        hasher.update(&u64::try_from(wkt.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(wkt.as_bytes());
    } else {
        hasher.update(&0_u64.to_le_bytes());
    }
    hasher.update(input_hash.as_bytes());
    hasher.update(geometry_hash.as_bytes());
    hasher.update(topology_hash.as_bytes());
    ContentHash::new(*hasher.finalize().as_bytes())
}

fn recipe_hash(recipe: TerrainRecipe) -> ContentHash {
    let mut hasher = domain_hasher(RECIPE_HASH_DOMAIN);
    hasher.update(&crate::ALGORITHM_VERSION.to_le_bytes());
    hasher.update(&[recipe.ground_classification()]);
    match recipe.bounds() {
        Some(bounds) => {
            hasher.update(&[1]);
            for value in bounds.min().into_iter().chain(bounds.max()) {
                hasher.update(&canonical_f64_bits(value).to_le_bytes());
            }
        }
        None => {
            hasher.update(&[0]);
        }
    }
    ContentHash::new(*hasher.finalize().as_bytes())
}

fn hash_transform(hasher: &mut Hasher, transform: PositionTransform) {
    for value in transform.offset().into_iter().chain(transform.scale()) {
        hasher.update(&canonical_f64_bits(value).to_le_bytes());
    }
}

fn canonical_f64_bits(value: f64) -> u64 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

fn domain_hasher(domain: &[u8]) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(
        &u64::try_from(domain.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    hasher.update(domain);
    hasher
}

fn retained_surface_bytes(
    vertices: &Vec<SurfaceVertex>,
    faces: &Vec<SurfaceFace>,
    coordinate_reference: &CoordinateReference,
) -> u64 {
    vector_bytes::<SurfaceVertex>(vertices.capacity())
        .saturating_add(vector_bytes::<SurfaceFace>(faces.capacity()))
        .saturating_add(u64::try_from(mem::size_of::<SurfaceData>()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(2 * mem::size_of::<usize>()).unwrap_or(u64::MAX))
        .saturating_add(
            coordinate_reference
                .as_wkt()
                .map_or(0, |wkt| u64::try_from(wkt.len()).unwrap_or(u64::MAX)),
        )
}

fn vector_bytes<T>(capacity: usize) -> u64 {
    u64::try_from(capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::{InputVertex, hull_vertex_count};
    use point_contracts::{PointId, SourceId};

    #[test]
    fn canonical_input_key_uses_ticks_before_point_identity() {
        let source = SourceId::new([1; 32]);
        let mut values = [
            InputVertex {
                ticks: [2, 0, 0],
                point: PointId::new(source, 0),
            },
            InputVertex {
                ticks: [1, 9, 0],
                point: PointId::new(source, 1),
            },
        ];
        values.sort_unstable();
        assert_eq!(values[0].ticks, [1, 9, 0]);
    }

    #[test]
    fn euler_facts_recover_triangle_and_square_hulls() {
        assert_eq!(hull_vertex_count(3, 1).expect("triangle"), 3);
        assert_eq!(hull_vertex_count(4, 2).expect("square"), 4);
    }
}
