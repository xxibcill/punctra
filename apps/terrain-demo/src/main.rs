//! Headless LAS/LAZ-to-Terrain demonstration with optional detached QA.

mod command;
mod recovery;

use std::{error::Error, fs, io, path::Path};

use point_contracts::{AttributeId, PointId, WorldBounds};
use point_index::{PrepareLimits, PreparedIndex};
use point_terrain::{
    CheckPoint, CheckPointId, CheckPointLimits, LandXmlLimits, LandXmlOptions, TerrainLimits,
    TerrainRecipe, TerrainSurface,
};
use point_workspace::{
    CommitRequest, OpenLimits, OperationId, PointSetLimits, Workspace, WorkspaceSchema,
};

use command::{Command, RunCommand, print_usage};
use recovery::{RecoveryPaths, commit_with_recovery, reconcile_recovery_record};

const LAS_CLASSIFICATION_ATTRIBUTE_ID: u32 = 6;
const GROUND_CLASSIFICATION: u8 = 2;
const NON_GROUND_CLASSIFICATION: u8 = 1;
const SURFACE_NAME: &str = "Punctra Ground Surface";

type AppResult<T> = Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run_main() {
        eprintln!("terrain-demo failed: {error}");
        std::process::exit(1);
    }
}

fn run_main() -> AppResult<()> {
    match Command::parse(std::env::args_os().skip(1))? {
        Command::Help => print_usage(),
        Command::Run(command) => run(command)?,
    }
    Ok(())
}

fn run(command: RunCommand) -> AppResult<()> {
    let recovery = RecoveryPaths::new(&command.source, &command.index, &command.workspace);
    recovery.require_workspace_for_record()?;
    let source = source_las::open(&command.source).blocking_wait()?;
    println!(
        "Verified Source\n  path: {}\n  identity: {}\n  Points: {}\n  Coordinate Reference: {}",
        command.source.display(),
        source.identity(),
        source.metadata().point_count(),
        if source.metadata().coordinate_reference().is_unknown() {
            "unknown"
        } else {
            "declared"
        },
    );

    let index =
        point_index::prepare(source, &command.index, PrepareLimits::default()).blocking_wait()?;
    print_index(&index, &command.index);

    let (mut workspace, workspace_disposition) =
        open_or_create_workspace(index, &command.workspace)?;
    if recovery.has_record() {
        drop(workspace);
        let (reopened, operation, revision) = reconcile_recovery_record(&recovery)?;
        println!("Recovered retained Operation\n  Operation: {operation}\n  Revision: {revision}");
        workspace = reopened;
    }
    let snapshot = workspace.head();
    println!(
        "Workspace {workspace_disposition}\n  path: {}\n  head Revision: {}",
        command.workspace.display(),
        snapshot.provenance().revision(),
    );

    let mut surface = point_terrain::derive(
        snapshot,
        TerrainRecipe::new(GROUND_CLASSIFICATION),
        TerrainLimits::default(),
    )
    .blocking_wait()?;
    print_surface(&surface);

    if let Some(ordinal) = command.correction_revert_ordinal {
        surface =
            exercise_classification_correction_and_revert(workspace, &surface, ordinal, &recovery)?;
    } else {
        drop(workspace);
    }

    if command.qa_sample {
        evaluate_builtin_check_points(&surface)?;
    }

    let mut options =
        LandXmlOptions::metric_metres(SURFACE_NAME, command.document_date, command.document_time)?;
    if command.assert_crs_metric {
        options = options.assert_coordinates_are_metric_metres();
    }
    let receipt = surface
        .export_landxml(&command.landxml, options, LandXmlLimits::default())
        .blocking_wait()?;
    println!(
        "LandXML exported\n  path: {}\n  bytes: {}\n  content hash: {}\n  vertices: {}\n  faces: {}",
        command.landxml.display(),
        receipt.byte_length(),
        receipt.content_hash(),
        receipt.vertex_count(),
        receipt.face_count(),
    );
    Ok(())
}

fn exercise_classification_correction_and_revert(
    workspace: Workspace,
    baseline: &TerrainSurface,
    ordinal: u64,
    recovery: &RecoveryPaths<'_>,
) -> AppResult<TerrainSurface> {
    let point = PointId::new(workspace.source(), ordinal);
    if !baseline
        .vertices()
        .iter()
        .any(|vertex| vertex.point() == point)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Point ordinal {ordinal} is not part of the current Ground Input"),
        )
        .into());
    }

    let baseline_snapshot = workspace.head();
    let target = baseline_snapshot
        .select_point_ids([point], PointSetLimits::default())
        .blocking_wait()?;
    if target.metadata().exact_count() != 1 {
        return Err(io::Error::other(format!(
            "Point ordinal {ordinal} did not materialize as one exact correction target"
        ))
        .into());
    }
    drop(baseline_snapshot);

    let correction_operation = OperationId::generate()?;
    let (workspace, correction_revision) = commit_with_recovery(
        workspace,
        CommitRequest::set_classification(correction_operation, target, NON_GROUND_CLASSIFICATION),
        "classification correction",
        correction_operation,
        recovery,
    )?;
    let changed: AppResult<TerrainSurface> = (|| {
        let changed = derive_ground(workspace.snapshot(correction_revision)?)?;
        validate_changed_surface(baseline, &changed)?;
        Ok(changed)
    })();

    let revert_operation = OperationId::generate()?;
    let (workspace, revert_revision) = commit_with_recovery(
        workspace,
        CommitRequest::revert_head(revert_operation, correction_revision),
        "immediate-head Revert",
        revert_operation,
        recovery,
    )?;
    let restored = derive_ground(workspace.snapshot(revert_revision)?)?;
    validate_restored_surface(baseline, &restored)?;
    let changed = changed?;

    println!(
        "Classification correction and Revert\n  corrected Point ordinal: {ordinal}\n  correction Revision: {correction_revision}\n  changed Ground Input Points: {}\n  changed geometry hash: {}\n  Revert Revision: {revert_revision}\n  restored Ground Input Points: {}\n  restored geometry/topology hashes: yes",
        changed.descriptor().input_point_count(),
        changed.descriptor().geometry_hash(),
        restored.descriptor().input_point_count(),
    );
    Ok(restored)
}

fn validate_changed_surface(baseline: &TerrainSurface, changed: &TerrainSurface) -> AppResult<()> {
    let expected_changed_count = baseline
        .descriptor()
        .input_point_count()
        .checked_sub(1)
        .ok_or_else(|| io::Error::other("baseline Ground Input count underflowed"))?;
    if changed.descriptor().input_point_count() != expected_changed_count
        || changed.descriptor().geometry_hash() == baseline.descriptor().geometry_hash()
    {
        return Err(io::Error::other(
            "classification correction did not change exact Ground Input geometry",
        )
        .into());
    }
    Ok(())
}

fn validate_restored_surface(
    baseline: &TerrainSurface,
    restored: &TerrainSurface,
) -> AppResult<()> {
    let restored_hashes = restored.descriptor().geometry_hash()
        == baseline.descriptor().geometry_hash()
        && restored.descriptor().topology_hash() == baseline.descriptor().topology_hash();
    if !restored_hashes
        || restored.vertices() != baseline.vertices()
        || restored.faces() != baseline.faces()
    {
        return Err(io::Error::other(
            "immediate-head Revert did not restore exact Terrain geometry and topology",
        )
        .into());
    }
    Ok(())
}

fn derive_ground(snapshot: point_workspace::Snapshot) -> AppResult<TerrainSurface> {
    Ok(point_terrain::derive(
        snapshot,
        TerrainRecipe::new(GROUND_CLASSIFICATION),
        TerrainLimits::default(),
    )
    .blocking_wait()?)
}

fn print_index(index: &PreparedIndex, path: &Path) {
    let report = *index.prepare_report();
    println!(
        "Point index prepared\n  path: {}\n  disposition: {:?}\n  Source Points read: {}\n  durable Points reused: {}\n  artifact bytes: {}",
        path.display(),
        report.disposition(),
        report.source_points_read(),
        report.durable_points_reused(),
        report.artifact_bytes(),
    );
}

fn open_or_create_workspace(
    index: PreparedIndex,
    path: &Path,
) -> AppResult<(Workspace, &'static str)> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok((
            point_workspace::open(path, index, OpenLimits::default()).blocking_wait()?,
            "opened",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let classification = AttributeId::new(LAS_CLASSIFICATION_ATTRIBUTE_ID)?;
            Ok((
                point_workspace::create(
                    path,
                    index,
                    WorkspaceSchema::new(classification),
                    OpenLimits::default(),
                )
                .blocking_wait()?,
                "created",
            ))
        }
        Err(error) => Err(error.into()),
    }
}

fn print_surface(surface: &TerrainSurface) {
    let descriptor = surface.descriptor();
    println!(
        "Terrain derived\n  Ground Input Points: {}\n  vertices: {}\n  faces: {}\n  hull vertices: {}\n  geometry hash: {}\n  topology hash: {}\n  peak working bytes: {}\n  topology steps: {}",
        descriptor.input_point_count(),
        descriptor.vertex_count(),
        descriptor.face_count(),
        descriptor.hull_vertex_count(),
        descriptor.geometry_hash(),
        descriptor.topology_hash(),
        descriptor.accounted_peak_working_bytes(),
        descriptor.topology_steps(),
    );
}

fn evaluate_builtin_check_points(surface: &TerrainSurface) -> AppResult<()> {
    let first_vertex = surface
        .vertices()
        .first()
        .ok_or_else(|| io::Error::other("derived Terrain contains no vertices"))?;
    let sampled_position = surface
        .descriptor()
        .position_transform()
        .world_f64(first_vertex.ticks());
    let bounds = surface.descriptor().bounds();
    let gap_position = [outside_x(bounds)?, bounds.min()[1], bounds.min()[2]];
    let check_points = [
        CheckPoint::new(CheckPointId::new(1)?, sampled_position)?,
        CheckPoint::new(CheckPointId::new(2)?, gap_position)?,
    ];
    let report = surface
        .check_points(check_points, CheckPointLimits::default())
        .blocking_wait()?;
    let statistics = report.statistics();
    if statistics.covered_count() != 1 || statistics.gap_count() != 1 {
        return Err(io::Error::other(
            "built-in detached QA sample did not produce one sample and one gap",
        )
        .into());
    }
    println!(
        "Detached QA sample\n  covered: {}\n  gaps: {}\n  minimum residual: {:?}\n  maximum residual: {:?}\n  RMS residual: {:?}\n  face tests: {}",
        statistics.covered_count(),
        statistics.gap_count(),
        statistics.minimum(),
        statistics.maximum(),
        statistics.root_mean_square(),
        report.face_tests(),
    );
    Ok(())
}

fn outside_x(bounds: WorldBounds) -> Result<f64, io::Error> {
    let minimum = bounds.min()[0];
    let maximum = bounds.max()[0];
    let margin = (maximum - minimum).abs().max(1.0);
    let above = maximum + margin;
    if above.is_finite() && above > maximum {
        return Ok(above);
    }
    let below = minimum - margin;
    if below.is_finite() && below < minimum {
        return Ok(below);
    }
    Err(io::Error::other(
        "Terrain bounds leave no finite coordinate for the QA gap sample",
    ))
}
