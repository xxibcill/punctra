//! Runs a traceable correct, re-derive, compare, recheck, and Revert loop.

use std::{
    env,
    fmt::Write as _,
    fs,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, LinearUnit, PositionTransform, SpatialAxes,
    SpatialReferenceProfile, SpatialReferenceProvenance,
};
use point_index::{PrepareLimits, prepare};
use point_terrain::{
    CheckPoint, CheckPointId, ExactTerrainQaReport, ExactTerrainQaRequest, ProfileOutcome,
    ResidualOutcome, StationProfile, SurfaceComparisonLimits, SurfaceComparisonReport,
    TerrainLimits, TerrainQaCurrentState, TerrainQaFreshness, TerrainQaLimits, TerrainRecipe,
    TerrainSurface, VerticalTolerance, compare_surfaces, derive,
};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, PointQuery,
    PointSetLimits, RevisionId, Snapshot, Workspace, WorkspaceSchema, create,
};
use serde_json::{Value, json};
use source_memory::MemorySource;

const OUTPUT_ENVIRONMENT: &str = "PUNCTRA_QA_EXAMPLE_OUTPUT_DIR";
const CLASSIFICATION_ATTRIBUTE_ID: u32 = 301;
const GROUND_CLASSIFICATION: u8 = 2;
const NON_GROUND_CLASSIFICATION: u8 = 1;
const DEFECT_ORDINAL: u64 = 4;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = ExampleOutput::new()?;
    let workspace = workspace(output.path())?;
    let recipe = TerrainRecipe::new(GROUND_CLASSIFICATION);
    let request = qa_request()?;

    let baseline = workspace.head();
    let baseline_surface = derive_surface(baseline.clone(), recipe)?;
    let baseline_qa = run_qa(&baseline_surface, baseline.clone(), request.clone())?;

    let correction_operation = operation(41)?;
    let corrected_revision = correct_defect(&workspace, &baseline, correction_operation)?;
    let corrected = workspace.snapshot(corrected_revision)?;
    let stale = baseline_qa
        .binding()
        .freshness(TerrainQaCurrentState::in_memory(
            &corrected,
            &baseline_surface,
        ));
    let corrected_surface = derive_surface(corrected.clone(), recipe)?;
    let corrected_qa = run_qa(&corrected_surface, corrected.clone(), request)?;
    let change = compare(&baseline_surface, &corrected_surface)?;

    let revert_operation = operation(42)?;
    let reverted_revision = revert(&workspace, corrected_revision, revert_operation)?;
    let restored = workspace.snapshot(reverted_revision)?;
    let restored_surface = derive_surface(restored, recipe)?;
    let restored = compare(&baseline_surface, &restored_surface)?;
    require_restored(&restored)?;

    let evidence = evidence_document(&EvidenceFacts {
        baseline: &baseline_qa,
        corrected: &corrected_qa,
        stale,
        change,
        restored,
        correction_operation,
        revert_operation,
        corrected_revision,
        reverted_revision,
    });
    let evidence_path = output.path().join("exact-terrain-qa-evidence.json");
    write_json(&evidence_path, &evidence)?;
    let svg_path = output.path().join("exact-terrain-qa-profile.svg");
    write_new(
        &svg_path,
        profile_svg(&baseline_qa, &corrected_qa)?.as_bytes(),
    )?;

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "punctra.point-terrain.exact-qa-example-output.v1",
            "evidence": evidence_path,
            "profile_svg": svg_path,
            "preserved": output.is_preserved(),
            "preserve_with": format!("{OUTPUT_ENVIRONMENT}=path/to/new-directory"),
            "claims": {
                "generated_fixture": true,
                "field_qualified": false,
                "observed_workflow_timing": false,
                "independent_adoption": false,
                "partner_validated": false,
                "support_qualified": false,
            }
        }))?
    );
    Ok(())
}

fn workspace(directory: &Path) -> Result<Workspace, Box<dyn std::error::Error>> {
    let source = source_memory::open(memory_fixture()?).blocking_wait()?;
    let index = prepare(
        source,
        directory.join("exact-qa-example.pidx"),
        PrepareLimits::default(),
    )
    .blocking_wait()?;
    Ok(create(
        directory.join("exact-qa-example.pcw"),
        index,
        WorkspaceSchema::new(classification_attribute()?),
        OpenLimits::default(),
    )
    .blocking_wait()?)
}

fn memory_fixture() -> Result<MemorySource, Box<dyn std::error::Error>> {
    let mut ticks = Vec::new();
    for y in 0..3_i64 {
        for x in 0..3_i64 {
            ticks.push([x, y, if x == 1 && y == 1 { 10 } else { 0 }]);
        }
    }
    let point_count = ticks.len();
    let definition = AttributeDefinition::new(
        classification_attribute()?,
        "classification",
        AttributeDataType::U8,
    )?;
    let attributes = AttributeColumns::new(
        vec![AttributeColumn::new(
            definition,
            AttributeValues::u8(vec![GROUND_CLASSIFICATION; point_count]),
        )?],
        point_count,
    )?;
    Ok(MemorySource::from_columns(
        PositionTransform::new([500_000.0, 4_600_000.0, 100.0], [1.0, 1.0, 1.0])?,
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

fn qa_request() -> Result<ExactTerrainQaRequest, point_terrain::TerrainError> {
    let tolerance = VerticalTolerance::new(0.05, 0.05)?;
    let check_point = CheckPoint::new(CheckPointId::new(1)?, [500_001.0, 4_600_001.0, 100.0])?;
    let profile = StationProfile::new([500_000.0, 4_600_001.0], [500_002.0, 4_600_001.0], 2)?;
    Ok(ExactTerrainQaRequest::new(tolerance)
        .source_points(PointQuery::all())
        .check_points(vec![check_point].into_boxed_slice())
        .profile(profile))
}

fn derive_surface(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
) -> Result<TerrainSurface, point_terrain::TerrainError> {
    derive(snapshot, recipe, TerrainLimits::default()).blocking_wait()
}

fn run_qa(
    surface: &TerrainSurface,
    snapshot: Snapshot,
    request: ExactTerrainQaRequest,
) -> Result<ExactTerrainQaReport, point_terrain::TerrainError> {
    surface
        .exact_qa(snapshot, request, TerrainQaLimits::default())
        .blocking_wait()
}

fn correct_defect(
    workspace: &Workspace,
    baseline: &Snapshot,
    operation: OperationId,
) -> Result<RevisionId, Box<dyn std::error::Error>> {
    let point = point_contracts::PointId::new(workspace.source(), DEFECT_ORDINAL);
    let selected = baseline
        .select_point_ids([point], PointSetLimits::default())
        .blocking_wait()?;
    committed_revision(
        workspace
            .commit(
                CommitRequest::set_classification(operation, selected, NON_GROUND_CLASSIFICATION),
                CommitLimits::default(),
            )
            .blocking_wait()?,
    )
}

fn revert(
    workspace: &Workspace,
    corrected: RevisionId,
    operation: OperationId,
) -> Result<RevisionId, Box<dyn std::error::Error>> {
    committed_revision(
        workspace
            .commit(
                CommitRequest::revert_head(operation, corrected),
                CommitLimits::default(),
            )
            .blocking_wait()?,
    )
}

fn committed_revision(outcome: CommitOutcome) -> Result<RevisionId, Box<dyn std::error::Error>> {
    match outcome {
        CommitOutcome::Committed(receipt) => Ok(receipt.revision()),
        CommitOutcome::Rejected(reason) => {
            Err(io::Error::other(format!("example commit was rejected: {reason:?}")).into())
        }
        CommitOutcome::Indeterminate(uncertainty) => Err(io::Error::other(format!(
            "example commit must be reconciled by operation {}: {uncertainty:?}",
            uncertainty.operation()
        ))
        .into()),
    }
}

fn compare(
    before: &TerrainSurface,
    after: &TerrainSurface,
) -> Result<SurfaceComparisonReport, point_terrain::TerrainError> {
    compare_surfaces(before, after, SurfaceComparisonLimits::default()).blocking_wait()
}

fn require_restored(report: &SurfaceComparisonReport) -> io::Result<()> {
    if report.added_face_count() == 0
        && report.removed_face_count() == 0
        && report.changed_bounds().is_none()
    {
        return Ok(());
    }
    Err(io::Error::other(
        "Revert did not reproduce baseline semantic topology",
    ))
}

struct EvidenceFacts<'a> {
    baseline: &'a ExactTerrainQaReport,
    corrected: &'a ExactTerrainQaReport,
    stale: TerrainQaFreshness,
    change: SurfaceComparisonReport,
    restored: SurfaceComparisonReport,
    correction_operation: OperationId,
    revert_operation: OperationId,
    corrected_revision: RevisionId,
    reverted_revision: RevisionId,
}

fn evidence_document(facts: &EvidenceFacts<'_>) -> Value {
    json!({
        "schema": "punctra.point-terrain.exact-qa-evidence.v1",
        "package_version": env!("CARGO_PKG_VERSION"),
        "units": {
            "horizontal": "metre",
            "vertical": "metre",
            "profile_station": "metre",
            "residual": "metre",
        },
        "operations": {
            "correction": facts.correction_operation.to_string(),
            "corrected_revision": facts.corrected_revision.to_string(),
            "revert": facts.revert_operation.to_string(),
            "reverted_revision": facts.reverted_revision.to_string(),
        },
        "qa": {
            "baseline": qa_json(facts.baseline),
            "freshness_after_correction": freshness_name(facts.stale),
            "corrected": qa_json(facts.corrected),
        },
        "surface_change": comparison_json(&facts.change),
        "post_revert_comparison": comparison_json(&facts.restored),
        "trace": {
            "profile_svg_station_pointer": "/qa/{baseline|corrected}/profile/stations/{index}",
            "source_residual_pointer": "/qa/{baseline|corrected}/source_points/{index}",
            "check_point_pointer": "/qa/{baseline|corrected}/check_points/{index}",
        },
        "claims": {
            "cpu_authoritative": true,
            "generated_fixture": true,
            "field_qualified": false,
            "observed_workflow_timing": false,
            "independent_adoption": false,
            "partner_validated": false,
            "support_qualified": false,
        },
    })
}

fn qa_json(report: &ExactTerrainQaReport) -> Value {
    let binding = report.binding();
    let tolerance = report.tolerance();
    json!({
        "binding": {
            "workspace": binding.snapshot().workspace().to_string(),
            "source": binding.snapshot().source().to_string(),
            "revision": binding.snapshot().revision().to_string(),
            "terrain_algorithm_version": binding.algorithm_version(),
            "recipe_hash": binding.recipe_hash().to_string(),
            "surface_input_hash": binding.input_hash().to_string(),
            "geometry_hash": binding.geometry_hash().to_string(),
            "topology_hash": binding.topology_hash().to_string(),
            "artifact_hash": binding.artifact_hash().to_string(),
            "horizontal_epsg": binding.spatial_reference().horizontal_epsg(),
            "vertical_epsg": binding.spatial_reference().vertical_epsg(),
        },
        "tolerance": {
            "below_metres": tolerance.below_metres(),
            "above_metres": tolerance.above_metres(),
            "boundary": "inclusive",
        },
        "input_hash": report.input_hash().to_string(),
        "result_hash": report.result_hash().to_string(),
        "source_input": report.source_input().map(|summary| json!({
            "query": {
                "bounds": summary.query().bounds().map(|bounds| json!({
                    "min": bounds.min(),
                    "max": bounds.max(),
                })),
                "classification_eq": summary.query().classification_eq(),
            },
            "candidate_point_count": summary.candidate_point_count(),
            "exact_count": summary.exact_count(),
            "point_id_hash": summary.point_id_hash().to_string(),
            "content_hash": summary.content_hash().to_string(),
        })),
        "source_points": report.source_points().iter().map(source_result_json).collect::<Vec<_>>(),
        "check_points": report.check_points().iter().map(|result| json!({
            "id": result.check_point().id().get(),
            "position": result.check_point().position(),
            "outcome": residual_outcome_json(result.outcome()),
        })).collect::<Vec<_>>(),
        "profile": {
            "definition": report.profile().map(|profile| json!({
                "start_xy": profile.start_xy(),
                "end_xy": profile.end_xy(),
                "intervals": profile.intervals(),
            })),
            "stations": report.profile_stations().iter().map(|station| json!({
                "id": format!("station-{}", station.index()),
                "index": station.index(),
                "station_metres": station.station_metres(),
                "world_xy": station.world_xy(),
                "outcome": profile_outcome_json(station.outcome()),
            })).collect::<Vec<_>>(),
            "gap_count": report.profile_gap_count(),
        },
        "statistics": {
            "covered_count": report.statistics().covered_count(),
            "gap_count": report.statistics().gap_count(),
            "minimum_residual_metres": report.statistics().minimum(),
            "maximum_residual_metres": report.statistics().maximum(),
            "mean_residual_metres": report.statistics().mean(),
            "root_mean_square_metres": report.statistics().root_mean_square(),
        },
        "tolerance_summary": {
            "below": report.tolerance_summary().below_count(),
            "within": report.tolerance_summary().within_count(),
            "above": report.tolerance_summary().above_count(),
            "gaps": report.tolerance_summary().gap_count(),
        },
        "resources": {
            "face_tests": report.face_tests(),
            "accounted_peak_working_bytes": report.accounted_peak_working_bytes(),
            "retained_result_bytes": report.retained_result_bytes(),
        },
    })
}

fn source_result_json(result: &point_terrain::SourcePointResidual) -> Value {
    json!({
        "source": result.point().source().to_string(),
        "ordinal": result.point().ordinal(),
        "ticks": result.ticks(),
        "world_position": result.world_position(),
        "effective_classification": result.effective_classification(),
        "outcome": residual_outcome_json(result.outcome()),
    })
}

fn residual_outcome_json(outcome: ResidualOutcome) -> Value {
    let Some(sample) = outcome.sampled() else {
        return json!({ "kind": "gap" });
    };
    json!({
        "kind": "sampled",
        "face": sample.face().get(),
        "surface_z_metres": sample.surface_z(),
        "residual_metres": sample.residual(),
        "tolerance": sample.tolerance().as_str(),
    })
}

fn profile_outcome_json(outcome: ProfileOutcome) -> Value {
    match outcome {
        ProfileOutcome::Gap => json!({ "kind": "gap" }),
        ProfileOutcome::Sampled { face, surface_z } => json!({
            "kind": "sampled",
            "face": face.get(),
            "surface_z_metres": surface_z,
        }),
    }
}

fn comparison_json(report: &SurfaceComparisonReport) -> Value {
    json!({
        "before_revision": report.before_snapshot().revision().to_string(),
        "after_revision": report.after_snapshot().revision().to_string(),
        "before_artifact_hash": report.before_artifact_hash().to_string(),
        "after_artifact_hash": report.after_artifact_hash().to_string(),
        "added_face_count": report.added_face_count(),
        "removed_face_count": report.removed_face_count(),
        "added_face_hash": report.added_face_hash().to_string(),
        "removed_face_hash": report.removed_face_hash().to_string(),
        "changed_bounds": report.changed_bounds().map(|bounds| json!({
            "min": bounds.min(),
            "max": bounds.max(),
            "meaning": "conservative incident-vertex envelope; not an exact change polygon",
        })),
        "retained_record_bytes": report.retained_record_bytes(),
        "work_units": report.work_units(),
        "accounted_peak_working_bytes": report.accounted_peak_working_bytes(),
    })
}

fn profile_svg(
    baseline: &ExactTerrainQaReport,
    corrected: &ExactTerrainQaReport,
) -> Result<String, io::Error> {
    let mut svg = String::new();
    writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"760\" height=\"360\" viewBox=\"0 0 760 360\" role=\"img\" aria-labelledby=\"title description\">"
    )
    .map_err(io::Error::other)?;
    writeln!(svg, "<title id=\"title\">Exact terrain QA profile</title>")
        .map_err(io::Error::other)?;
    writeln!(svg, "<desc id=\"description\">Generated baseline and corrected CPU-authoritative profile stations. Every circle carries a JSON evidence pointer.</desc>").map_err(io::Error::other)?;
    writeln!(svg, "<rect width=\"760\" height=\"360\" fill=\"#f8fafc\"/>")
        .map_err(io::Error::other)?;
    writeln!(
        svg,
        "<path d=\"M80 40V300H720\" fill=\"none\" stroke=\"#475569\" stroke-width=\"2\"/>"
    )
    .map_err(io::Error::other)?;
    writeln!(svg, "<text x=\"400\" y=\"338\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"14\">profile station (metre)</text>").map_err(io::Error::other)?;
    writeln!(svg, "<text x=\"20\" y=\"170\" transform=\"rotate(-90 20 170)\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"14\">surface elevation (metre)</text>").map_err(io::Error::other)?;
    write_profile_series(&mut svg, "baseline", baseline, "#dc2626")?;
    write_profile_series(&mut svg, "corrected", corrected, "#059669")?;
    writeln!(svg, "<text x=\"100\" y=\"28\" fill=\"#dc2626\" font-family=\"sans-serif\">baseline defect</text><text x=\"260\" y=\"28\" fill=\"#059669\" font-family=\"sans-serif\">corrected</text>").map_err(io::Error::other)?;
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn write_profile_series(
    svg: &mut String,
    name: &str,
    report: &ExactTerrainQaReport,
    color: &str,
) -> Result<(), io::Error> {
    let sampled = report
        .profile_stations()
        .iter()
        .map(|station| match station.outcome() {
            ProfileOutcome::Sampled { surface_z, .. } => Ok((station, surface_z)),
            ProfileOutcome::Gap => Err(io::Error::other(
                "generated profile unexpectedly contains a gap",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let points = sampled
        .iter()
        .map(|(station, z)| {
            format!(
                "{:.3},{:.3}",
                100.0 + station.station_metres() * 280.0,
                280.0 - (z - 100.0) * 22.0
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(
        svg,
        "<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"3\"/>"
    )
    .map_err(io::Error::other)?;
    for (station, z) in sampled {
        let x = 100.0 + station.station_metres() * 280.0;
        let y = 280.0 - (z - 100.0) * 22.0;
        writeln!(svg, "<circle id=\"{name}-station-{}\" data-evidence-pointer=\"/qa/{name}/profile/stations/{}\" cx=\"{x:.3}\" cy=\"{y:.3}\" r=\"6\" fill=\"{color}\"><title>{name} station {}: {:.6} m, {:.6} m elevation</title></circle>", station.index(), station.index(), station.index(), station.station_metres(), z).map_err(io::Error::other)?;
    }
    Ok(())
}

fn freshness_name(value: TerrainQaFreshness) -> &'static str {
    match value {
        TerrainQaFreshness::Current => "current",
        TerrainQaFreshness::SnapshotOnlyCurrent => "snapshot_only_current",
        TerrainQaFreshness::StaleSnapshot => "stale_snapshot",
        TerrainQaFreshness::StaleSurface => "stale_surface",
        TerrainQaFreshness::StaleSnapshotAndSurface => "stale_snapshot_and_surface",
    }
}

fn operation(byte: u8) -> Result<OperationId, point_workspace::WorkspaceError> {
    OperationId::from_bytes([byte; 16])
}

fn classification_attribute() -> Result<AttributeId, point_contracts::ContractError> {
    AttributeId::new(CLASSIFICATION_ATTRIBUTE_ID)
}

fn write_json(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let file = create_new(path)?;
    serde_json::to_writer_pretty(&file, value)?;
    file.sync_all()?;
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn create_new(path: &Path) -> io::Result<File> {
    File::options().write(true).create_new(true).open(path)
}

struct ExampleOutput {
    path: PathBuf,
    preserve: bool,
}

impl ExampleOutput {
    fn new() -> io::Result<Self> {
        if let Some(path) = env::var_os(OUTPUT_ENVIRONMENT) {
            let path = PathBuf::from(path);
            fs::create_dir(&path)?;
            return Ok(Self {
                path,
                preserve: true,
            });
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..100_u32 {
            let path = env::temp_dir().join(format!(
                "punctra-exact-qa-example-{}-{timestamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        preserve: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create isolated exact-QA example directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    const fn is_preserved(&self) -> bool {
        self.preserve
    }
}

impl Drop for ExampleOutput {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
