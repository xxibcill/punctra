//! Builds, resumes, reopens, and streams one generated persistent bounded-AOI Surface.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, LinearUnit, PositionTransform, SpatialAxes,
    SpatialReferenceProfile, SpatialReferenceProvenance, WorldBounds,
};
use point_index::{PrepareDisposition, PrepareLimits};
use point_terrain::{
    PreparedTerrainSurface, SurfaceReadLimits, TerrainPrepareDisposition, TerrainPrepareLimits,
    TerrainRecipe,
};
use point_workspace::{OpenLimits, PointRowLimits, Workspace, WorkspaceSchema};
use serde_json::{Value, json};
use source_memory::MemorySource;

const CLASSIFICATION_ATTRIBUTE_ID: u32 = 301;
const GROUND_CLASSIFICATION: u8 = 2;
const DEFAULT_POINT_COUNT: u64 = 10_000;
const MAX_POINT_COUNT: u64 = 1_000_000;
const STREAM_BATCH_RECORDS: u64 = 4_096;
const STREAM_BATCH_PAYLOAD_BYTES: u64 = 1024 * 1024;
const STREAM_VERIFY_BUFFER_BYTES: u64 = 128 * 1024;
const STREAM_WORKING_BYTES: u64 = 2 * 1024 * 1024;
const STREAM_WORK_UNITS: u64 = 100_000_000;
const FORCED_ARTIFACT_LIMIT_BYTES: u64 = 1;
const ARTIFACT_COMPARE_BUFFER_BYTES: usize = 64 * 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let point_count = requested_point_count()?;
    let directory = ExampleDirectory::new()?;
    let fixture = GeneratedFixture::new(point_count, directory.path())?;
    let persistent = PersistentRun::new(&fixture.workspace, point_count, directory.path())?;
    let report = example_report(point_count, &fixture, &persistent)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn example_report(
    point_count: u64,
    fixture: &GeneratedFixture,
    persistent: &PersistentRun,
) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(json!({
        "schema": "punctra.point-terrain.persistent-example.v1",
        "package_version": env!("CARGO_PKG_VERSION"),
        "generated_fixture": true,
        "source": {
            "point_count": point_count,
            "verification_elapsed_ns": elapsed_ns(fixture.source_elapsed),
        },
        "index": index_report(fixture),
        "workspace": {
            "create_elapsed_ns": elapsed_ns(fixture.workspace_elapsed),
        },
        "terrain": terrain_report(persistent)?,
        "bounded_streams": stream_report(persistent),
        "unavailable_observations": unavailable_observations(),
        "claims": {
            "production_data": false,
            "out_of_core_triangulation": false,
            "field_qualified": false,
            "partner_validated": false,
            "support_qualified": false,
            "extrapolated_beyond_measured_scale": false,
        },
    }))
}

fn index_report(fixture: &GeneratedFixture) -> Value {
    let report = fixture.index_report;
    json!({
        "disposition": index_disposition(report.disposition()),
        "elapsed_ns": elapsed_ns(fixture.index_elapsed),
        "source_points_read": report.source_points_read(),
        "durable_points_reused": report.durable_points_reused(),
        "artifact_bytes": report.artifact_bytes(),
        "peak_temporary_disk_bytes": report.peak_temporary_disk_bytes(),
    })
}

fn terrain_report(persistent: &PersistentRun) -> Result<Value, Box<dyn std::error::Error>> {
    let descriptor = persistent.cold.descriptor();
    let aoi = descriptor
        .recipe()
        .bounds()
        .ok_or_else(|| io::Error::other("prepared Surface lost its explicit AOI"))?;
    let limits = persistent.limits;
    let derivation_limits = limits.derivation();
    Ok(json!({
        "algorithm_version": descriptor.algorithm_version(),
        "surface_disk_version": point_terrain::SURFACE_DISK_VERSION,
        "ground_classification": descriptor.recipe().ground_classification(),
        "aoi": {
            "min": aoi.min(),
            "max": aoi.max(),
        },
        "input_point_count": descriptor.input_point_count(),
        "vertex_count": descriptor.vertex_count(),
        "face_count": descriptor.face_count(),
        "hull_vertex_count": descriptor.hull_vertex_count(),
        "recipe_hash": descriptor.recipe_hash().to_string(),
        "input_hash": descriptor.input_hash().to_string(),
        "geometry_hash": descriptor.geometry_hash().to_string(),
        "topology_hash": descriptor.topology_hash().to_string(),
        "artifact_hash": descriptor.artifact_hash().to_string(),
        "cold": prepare_report(persistent.cold.report(), persistent.cold_elapsed),
        "resumed": {
            "attempt": prepare_report(
                persistent.resumed.report(),
                persistent.resumed_elapsed,
            ),
            "forced_artifact_limit_bytes": FORCED_ARTIFACT_LIMIT_BYTES,
            "forced_failure": "resource_limit",
            "forced_failure_elapsed_ns": elapsed_ns(persistent.forced_failure_elapsed),
            "retained_work_checkpoint_bytes_before_retry":
                persistent.retained_work_checkpoint_bytes,
            "artifact_bytes_identical_to_cold": true,
            "artifact_comparison_elapsed_ns": elapsed_ns(persistent.comparison_elapsed),
            "artifact_comparison_buffer_bytes_per_file": ARTIFACT_COMPARE_BUFFER_BYTES,
            "artifact_comparison_buffer_bytes_total":
                ARTIFACT_COMPARE_BUFFER_BYTES.saturating_mul(2),
        },
        "warm": prepare_report(persistent.warm.report(), persistent.warm_elapsed),
        "limits": {
            "max_work_bytes": limits.max_work_bytes(),
            "max_artifact_bytes": limits.max_artifact_bytes(),
            "max_temporary_bytes": limits.max_temporary_bytes(),
            "max_verify_buffer_bytes": limits.max_verify_buffer_bytes(),
            "max_retained_handle_bytes": limits.max_retained_handle_bytes(),
            "max_path_bytes": limits.max_path_bytes(),
            "derivation": {
                "point_rows": point_row_limits_report(derivation_limits.point_rows()),
                "max_input_points": derivation_limits.max_input_points(),
                "max_vertices": derivation_limits.max_vertices(),
                "max_faces": derivation_limits.max_faces(),
                "max_working_bytes": derivation_limits.max_working_bytes(),
                "max_surface_bytes": derivation_limits.max_surface_bytes(),
                "max_work_units": derivation_limits.max_work_units(),
            },
        },
    }))
}

fn point_row_limits_report(limits: PointRowLimits) -> Value {
    let candidate = limits.candidate_limits();
    let source_read = limits.source_read_budget();
    json!({
        "candidate": {
            "max_visited_nodes": candidate.max_visited_nodes(),
            "max_output_spans": candidate.max_output_spans(),
            "max_candidate_points": candidate.max_candidate_points(),
            "max_working_bytes": candidate.max_working_bytes(),
        },
        "source_read": {
            "max_spans": source_read.max_spans(),
            "max_points": source_read.max_points(),
            "max_batch_points": source_read.max_batch_points(),
            "max_batch_payload_bytes": source_read.max_batch_payload_bytes(),
            "max_adapter_working_bytes": source_read.max_adapter_working_bytes(),
        },
        "max_overlay_segments": limits.max_overlay_segments(),
        "max_overlay_bytes": limits.max_overlay_bytes(),
        "max_output_points": limits.max_output_points(),
        "max_batch_points": limits.max_batch_points(),
        "max_batch_payload_bytes": limits.max_batch_payload_bytes(),
        "max_working_bytes": limits.max_working_bytes(),
    })
}

fn stream_report(persistent: &PersistentRun) -> Value {
    json!({
        "max_batch_records": persistent.stream_limits.max_batch_records(),
        "max_batch_payload_bytes": persistent.stream_limits.max_batch_payload_bytes(),
        "max_verify_buffer_bytes": persistent.stream_limits.max_verify_buffer_bytes(),
        "max_working_bytes": persistent.stream_limits.max_working_bytes(),
        "max_work_units": persistent.stream_limits.max_work_units(),
        "vertices": {
            "records": persistent.stream_facts.vertices,
            "batches": persistent.stream_facts.vertex_batches,
        },
        "faces": {
            "records": persistent.stream_facts.faces,
            "batches": persistent.stream_facts.face_batches,
        },
        "elapsed_ns": elapsed_ns(persistent.stream_elapsed),
    })
}

fn unavailable_observations() -> Value {
    json!({
        "surface_stage_bytes": Value::Null,
        "worker_heap_bytes": Value::Null,
        "process_peak_resident_bytes": Value::Null,
        "allocated_filesystem_blocks": Value::Null,
        "qa_elapsed_ns": Value::Null,
        "landxml_elapsed_ns": Value::Null,
        "view_elapsed_ns": Value::Null,
        "field_accuracy": Value::Null,
    })
}

fn prepare_report(report: point_terrain::TerrainPrepareReport, elapsed: Duration) -> Value {
    json!({
        "disposition": terrain_disposition(report.disposition()),
        "elapsed_ns": elapsed_ns(elapsed),
        "artifact_bytes": report.artifact_bytes(),
        "reused_input_points": report.reused_input_points(),
        "source_points_read": report.source_points_read(),
        "peak_temporary_disk_bytes": report.peak_temporary_disk_bytes(),
        "accounted_handle_bytes": report.accounted_handle_bytes(),
        "accounted_peak_working_bytes": report.accounted_peak_working_bytes(),
        "topology_steps": report.topology_steps(),
    })
}

fn consume_vertices(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<(u64, u64), point_terrain::TerrainError> {
    let mut records = 0_u64;
    let mut batches = 0_u64;
    for batch in surface.vertex_batches(limits)? {
        let batch = batch?;
        records = records.saturating_add(u64::try_from(batch.len()).unwrap_or(u64::MAX));
        batches = batches.saturating_add(1);
    }
    Ok((records, batches))
}

fn consume_faces(
    surface: &PreparedTerrainSurface,
    limits: SurfaceReadLimits,
) -> Result<(u64, u64), point_terrain::TerrainError> {
    let mut records = 0_u64;
    let mut batches = 0_u64;
    for batch in surface.face_batches(limits)? {
        let batch = batch?;
        records = records.saturating_add(u64::try_from(batch.len()).unwrap_or(u64::MAX));
        batches = batches.saturating_add(1);
    }
    Ok((records, batches))
}

struct GeneratedFixture {
    workspace: Workspace,
    source_elapsed: Duration,
    index_elapsed: Duration,
    index_report: point_index::PrepareReport,
    workspace_elapsed: Duration,
}

impl GeneratedFixture {
    fn new(point_count: u64, directory: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let input = memory_fixture(point_count)?;
        let source_started = Instant::now();
        let source = source_memory::open(input).blocking_wait()?;
        let source_elapsed = source_started.elapsed();

        let index_started = Instant::now();
        let index = point_index::prepare(
            source,
            directory.join("example.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()?;
        let index_elapsed = index_started.elapsed();
        let index_report = *index.prepare_report();

        let workspace_started = Instant::now();
        let workspace = point_workspace::create(
            directory.join("example.pcw"),
            index,
            WorkspaceSchema::new(classification_attribute()?),
            OpenLimits::default(),
        )
        .blocking_wait()?;
        let workspace_elapsed = workspace_started.elapsed();
        Ok(Self {
            workspace,
            source_elapsed,
            index_elapsed,
            index_report,
            workspace_elapsed,
        })
    }
}

struct PersistentRun {
    cold: PreparedTerrainSurface,
    resumed: PreparedTerrainSurface,
    warm: PreparedTerrainSurface,
    cold_elapsed: Duration,
    resumed_elapsed: Duration,
    warm_elapsed: Duration,
    forced_failure_elapsed: Duration,
    comparison_elapsed: Duration,
    retained_work_checkpoint_bytes: u64,
    stream_elapsed: Duration,
    stream_facts: StreamFacts,
    limits: TerrainPrepareLimits,
    stream_limits: SurfaceReadLimits,
}

impl PersistentRun {
    fn new(
        workspace: &Workspace,
        point_count: u64,
        directory: &Path,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let side_world = f64::from(u32::try_from(grid_side(point_count))?);
        let recipe = TerrainRecipe::new(GROUND_CLASSIFICATION).within(WorldBounds::new(
            [-1.0, -1.0, -1_000.0],
            [side_world, side_world, 1_000.0],
        )?);
        let target = directory.join("example.pterr");
        let limits = TerrainPrepareLimits::default();

        let (cold, cold_elapsed) = timed_prepare(workspace, &target, recipe, limits)?;
        if cold.report().disposition() != TerrainPrepareDisposition::Built {
            return Err(io::Error::other("fresh example Surface was not built cold").into());
        }

        let resumed_target = directory.join("example-resumed.pterr");
        let resumed_run = resume_from_input(workspace, &resumed_target, recipe, limits)?;
        if resumed_run.surface.descriptor() != cold.descriptor()
            || resumed_run.surface.report().artifact_bytes() != cold.report().artifact_bytes()
        {
            return Err(io::Error::other("resumed input changed the canonical Surface").into());
        }
        let comparison_started = Instant::now();
        if !files_are_identical(&target, &resumed_target)? {
            return Err(io::Error::other("resumed input changed exact Surface bytes").into());
        }
        let comparison_elapsed = comparison_started.elapsed();

        let stream_limits = SurfaceReadLimits::new(
            STREAM_BATCH_RECORDS,
            STREAM_BATCH_PAYLOAD_BYTES,
            STREAM_VERIFY_BUFFER_BYTES,
            STREAM_WORKING_BYTES,
            STREAM_WORK_UNITS,
        );
        let stream_started = Instant::now();
        let (vertices, vertex_batches) = consume_vertices(&cold, stream_limits)?;
        let (faces, face_batches) = consume_faces(&cold, stream_limits)?;
        let stream_elapsed = stream_started.elapsed();
        if vertices != cold.descriptor().vertex_count() || faces != cold.descriptor().face_count() {
            return Err(
                io::Error::other("bounded streams did not reach declared completion").into(),
            );
        }

        let (warm, warm_elapsed) = timed_prepare(workspace, &target, recipe, limits)?;
        if warm.report().disposition() != TerrainPrepareDisposition::Opened
            || warm.descriptor() != cold.descriptor()
        {
            return Err(io::Error::other("warm reopen changed the canonical Surface").into());
        }

        Ok(Self {
            cold,
            resumed: resumed_run.surface,
            warm,
            cold_elapsed,
            resumed_elapsed: resumed_run.elapsed,
            warm_elapsed,
            forced_failure_elapsed: resumed_run.forced_failure_elapsed,
            comparison_elapsed,
            retained_work_checkpoint_bytes: resumed_run.retained_work_checkpoint_bytes,
            stream_elapsed,
            stream_facts: StreamFacts {
                vertices,
                vertex_batches,
                faces,
                face_batches,
            },
            limits,
            stream_limits,
        })
    }
}

struct ResumedRun {
    surface: PreparedTerrainSurface,
    elapsed: Duration,
    forced_failure_elapsed: Duration,
    retained_work_checkpoint_bytes: u64,
}

fn resume_from_input(
    workspace: &Workspace,
    target: &Path,
    recipe: TerrainRecipe,
    limits: TerrainPrepareLimits,
) -> Result<ResumedRun, Box<dyn std::error::Error>> {
    let constrained = TerrainPrepareLimits::new(
        limits.derivation(),
        limits.max_work_bytes(),
        FORCED_ARTIFACT_LIMIT_BYTES,
        limits.max_temporary_bytes(),
        limits.max_verify_buffer_bytes(),
        limits.max_retained_handle_bytes(),
        limits.max_path_bytes(),
    );
    let failure_started = Instant::now();
    let Err(failure) =
        point_terrain::prepare(workspace.head(), target, recipe, constrained).blocking_wait()
    else {
        return Err(
            io::Error::other("forced artifact limit unexpectedly published a Surface").into(),
        );
    };
    let forced_failure_elapsed = failure_started.elapsed();
    match failure {
        point_terrain::TerrainError::ResourceLimit { limit, allowed, .. }
            if limit == "Surface artifact bytes" && allowed == FORCED_ARTIFACT_LIMIT_BYTES => {}
        error => return Err(error.into()),
    }

    let work_path = sibling_path(target, ".surface-work-v1")?;
    let retained_work_checkpoint_bytes = fs::metadata(&work_path)?.len();
    let (surface, elapsed) = timed_prepare(workspace, target, recipe, limits)?;
    if surface.report().disposition() != TerrainPrepareDisposition::ResumedInput {
        return Err(io::Error::other("retry did not resume its verified input checkpoint").into());
    }
    Ok(ResumedRun {
        surface,
        elapsed,
        forced_failure_elapsed,
        retained_work_checkpoint_bytes,
    })
}

fn timed_prepare(
    workspace: &Workspace,
    target: &Path,
    recipe: TerrainRecipe,
    limits: TerrainPrepareLimits,
) -> Result<(PreparedTerrainSurface, Duration), point_terrain::TerrainError> {
    let started = Instant::now();
    let surface =
        point_terrain::prepare(workspace.head(), target, recipe, limits).blocking_wait()?;
    Ok((surface, started.elapsed()))
}

fn sibling_path(target: &Path, suffix: &str) -> io::Result<PathBuf> {
    let mut name = target
        .file_name()
        .ok_or_else(|| io::Error::other("example Surface target has no file name"))?
        .to_os_string();
    name.push(suffix);
    Ok(target.with_file_name(name))
}

fn files_are_identical(left: &Path, right: &Path) -> io::Result<bool> {
    let left_bytes = fs::metadata(left)?.len();
    if fs::metadata(right)?.len() != left_bytes {
        return Ok(false);
    }
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    let mut left_buffer = vec![0_u8; ARTIFACT_COMPARE_BUFFER_BYTES].into_boxed_slice();
    let mut right_buffer = vec![0_u8; ARTIFACT_COMPARE_BUFFER_BYTES].into_boxed_slice();
    let mut remaining = left_bytes;
    while remaining > 0 {
        let count = usize::try_from(
            remaining.min(u64::try_from(ARTIFACT_COMPARE_BUFFER_BYTES).unwrap_or(u64::MAX)),
        )
        .unwrap_or(ARTIFACT_COMPARE_BUFFER_BYTES);
        left.read_exact(&mut left_buffer[..count])?;
        right.read_exact(&mut right_buffer[..count])?;
        if left_buffer[..count] != right_buffer[..count] {
            return Ok(false);
        }
        remaining = remaining.saturating_sub(u64::try_from(count).unwrap_or(u64::MAX));
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct StreamFacts {
    vertices: u64,
    vertex_batches: u64,
    faces: u64,
    face_batches: u64,
}

fn memory_fixture(point_count: u64) -> Result<MemorySource, Box<dyn std::error::Error>> {
    let side = grid_side(point_count);
    let mut ticks = Vec::new();
    ticks.try_reserve_exact(usize::try_from(point_count)?)?;
    for ordinal in 0..point_count {
        let x = i64::try_from(ordinal % side)?;
        let y = i64::try_from(ordinal / side)?;
        ticks.push([x, y, (x * x + 3 * y * y + x * y) % 10_000]);
    }
    let definition = AttributeDefinition::new(
        classification_attribute()?,
        "classification",
        AttributeDataType::U8,
    )?;
    let column = AttributeColumn::new(
        definition,
        AttributeValues::u8(vec![GROUND_CLASSIFICATION; ticks.len()]),
    )?;
    let attributes = AttributeColumns::new(vec![column], ticks.len())?;
    Ok(MemorySource::from_columns(
        PositionTransform::new([0.0; 3], [1.0, 1.0, 0.01])?,
        CoordinateReference::profile(SpatialReferenceProfile::new(
            32_647,
            5_703,
            SpatialAxes::EastingNorthingElevation,
            LinearUnit::Metre,
            LinearUnit::Metre,
            SpatialReferenceProvenance::CallerDeclaration,
        )?),
        ticks,
        attributes,
    )?)
}

fn requested_point_count() -> Result<u64, Box<dyn std::error::Error>> {
    let Some(value) = std::env::var_os("PUNCTRA_PERSISTENT_TERRAIN_EXAMPLE_POINTS") else {
        return Ok(DEFAULT_POINT_COUNT);
    };
    let text = value
        .to_str()
        .ok_or_else(|| io::Error::other("example Point count must be valid UTF-8"))?;
    let count = text.parse::<u64>()?;
    if !(3..=MAX_POINT_COUNT).contains(&count) {
        return Err(io::Error::other(format!(
            "example Point count must be between 3 and {MAX_POINT_COUNT}"
        ))
        .into());
    }
    Ok(count)
}

fn grid_side(point_count: u64) -> u64 {
    let mut side = 1_u64;
    while side.saturating_mul(side) < point_count {
        side = side.saturating_add(1);
    }
    side
}

fn elapsed_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

const fn index_disposition(disposition: PrepareDisposition) -> &'static str {
    match disposition {
        PrepareDisposition::Opened => "opened",
        PrepareDisposition::Built => "built",
        PrepareDisposition::Resumed => "resumed",
    }
}

const fn terrain_disposition(disposition: TerrainPrepareDisposition) -> &'static str {
    match disposition {
        TerrainPrepareDisposition::Opened => "opened",
        TerrainPrepareDisposition::Built => "built",
        TerrainPrepareDisposition::ResumedInput => "resumed_input",
        TerrainPrepareDisposition::ResumedPublication => "resumed_publication",
    }
}

fn classification_attribute() -> Result<AttributeId, point_contracts::ContractError> {
    AttributeId::new(CLASSIFICATION_ATTRIBUTE_ID)
}

struct ExampleDirectory {
    path: PathBuf,
}

impl ExampleDirectory {
    fn new() -> io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..100_u32 {
            let path = std::env::temp_dir().join(format!(
                "punctra-persistent-surface-example-{}-{timestamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create isolated persistent-Surface example directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExampleDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_row_report_names_every_nested_ceiling() {
        let limits = PointRowLimits::default();
        let candidate = limits.candidate_limits();
        let source_read = limits.source_read_budget();

        assert_eq!(
            point_row_limits_report(limits),
            json!({
                "candidate": {
                    "max_visited_nodes": candidate.max_visited_nodes(),
                    "max_output_spans": candidate.max_output_spans(),
                    "max_candidate_points": candidate.max_candidate_points(),
                    "max_working_bytes": candidate.max_working_bytes(),
                },
                "source_read": {
                    "max_spans": source_read.max_spans(),
                    "max_points": source_read.max_points(),
                    "max_batch_points": source_read.max_batch_points(),
                    "max_batch_payload_bytes": source_read.max_batch_payload_bytes(),
                    "max_adapter_working_bytes": source_read.max_adapter_working_bytes(),
                },
                "max_overlay_segments": limits.max_overlay_segments(),
                "max_overlay_bytes": limits.max_overlay_bytes(),
                "max_output_points": limits.max_output_points(),
                "max_batch_points": limits.max_batch_points(),
                "max_batch_payload_bytes": limits.max_batch_payload_bytes(),
                "max_working_bytes": limits.max_working_bytes(),
            })
        );
    }
}
