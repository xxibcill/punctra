//! Headless LAS/LAZ-to-Terrain demonstration with optional detached QA.

use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use point_contracts::{AttributeId, PointId, WorldBounds};
use point_index::{PrepareLimits, PreparedIndex};
use point_terrain::{
    CheckPoint, CheckPointId, CheckPointLimits, LandXmlLimits, LandXmlOptions, TerrainLimits,
    TerrainRecipe, TerrainSurface,
};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRejection, CommitRequest, OpenLimits, OperationId,
    OperationResolution, PointSetLimits, RevisionId, Workspace, WorkspaceId, WorkspaceSchema,
};

const LAS_CLASSIFICATION_ATTRIBUTE_ID: u32 = 6;
const GROUND_CLASSIFICATION: u8 = 2;
const NON_GROUND_CLASSIFICATION: u8 = 1;
const SURFACE_NAME: &str = "Punctra Ground Surface";
const RECOVERY_MAGIC: &[u8; 8] = b"PTRNREC1";
const RECOVERY_PAYLOAD_BYTES: usize = 8 + 16 + 16;
const RECOVERY_RECORD_BYTES: usize = RECOVERY_PAYLOAD_BYTES + 32;

type AppResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecoveryRecord {
    workspace: WorkspaceId,
    operation: OperationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveredOperation {
    Committed(RevisionId),
    Rejected(CommitRejection),
    NotRecorded,
}

struct RecoveryPaths<'a> {
    source: &'a Path,
    index: &'a Path,
    workspace: &'a Path,
    record: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RunCommand {
    source: PathBuf,
    index: PathBuf,
    workspace: PathBuf,
    landxml: PathBuf,
    document_date: String,
    document_time: String,
    qa_sample: bool,
    assert_crs_metric: bool,
    correction_revert_ordinal: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Help,
    Run(RunCommand),
}

impl Command {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> AppResult<Self> {
        let mut arguments = arguments.into_iter();
        let mut paths = Vec::new();
        let mut document_date = None;
        let mut document_time = None;
        let mut qa_sample = false;
        let mut assert_crs_metric = false;
        let mut correction_revert_ordinal = None;
        let mut positional_only = false;

        while let Some(argument) = arguments.next() {
            if !positional_only
                && (argument == OsStr::new("--help") || argument == OsStr::new("-h"))
            {
                return Ok(Self::Help);
            }
            if !positional_only && argument == OsStr::new("--") {
                positional_only = true;
            } else if !positional_only && argument == OsStr::new("--qa-sample") {
                require_new_flag(qa_sample, "--qa-sample")?;
                qa_sample = true;
            } else if !positional_only && argument == OsStr::new("--assert-crs-metric") {
                require_new_flag(assert_crs_metric, "--assert-crs-metric")?;
                assert_crs_metric = true;
            } else if !positional_only && argument == OsStr::new("--exercise-correction-revert") {
                let value = required_option_value(&mut arguments, "--exercise-correction-revert")?;
                let ordinal = value.parse::<u64>().map_err(|_| {
                    invalid_input(
                        "--exercise-correction-revert requires a non-negative Point ordinal",
                    )
                })?;
                if correction_revert_ordinal.replace(ordinal).is_some() {
                    return Err(invalid_input(
                        "--exercise-correction-revert was supplied more than once",
                    )
                    .into());
                }
            } else if !positional_only && argument == OsStr::new("--date") {
                let value = required_option_value(&mut arguments, "--date")?;
                set_once(&mut document_date, value, "--date")?;
            } else if !positional_only && argument == OsStr::new("--time") {
                let value = required_option_value(&mut arguments, "--time")?;
                set_once(&mut document_time, value, "--time")?;
            } else if !positional_only && argument.to_string_lossy().starts_with('-') {
                return Err(invalid_input(format!(
                    "unknown option {}; use --help for usage",
                    Path::new(&argument).display()
                ))
                .into());
            } else {
                paths.push(PathBuf::from(argument));
            }
        }

        let [source, index, workspace, landxml]: [PathBuf; 4] =
            paths.try_into().map_err(|paths: Vec<PathBuf>| {
                invalid_input(format!(
                    "expected SOURCE, INDEX, WORKSPACE, and LANDXML paths; received {}",
                    paths.len()
                ))
            })?;
        Ok(Self::Run(RunCommand {
            source,
            index,
            workspace,
            landxml,
            document_date: document_date
                .ok_or_else(|| invalid_input("missing required --date YYYY-MM-DD"))?,
            document_time: document_time
                .ok_or_else(|| invalid_input("missing required --time HH:MM:SSZ"))?,
            qa_sample,
            assert_crs_metric,
            correction_revert_ordinal,
        }))
    }
}

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
    let recovery = RecoveryPaths {
        source: &command.source,
        index: &command.index,
        workspace: &command.workspace,
        record: recovery_record_path(&command.workspace),
    };
    if recovery.record.exists() && !command.workspace.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "recovery record {} exists without its Workspace",
                recovery.record.display()
            ),
        )
        .into());
    }
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
    if recovery.record.exists() {
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
    let changed = derive_ground(workspace.snapshot(correction_revision)?)?;
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

    let revert_operation = OperationId::generate()?;
    let (workspace, revert_revision) = commit_with_recovery(
        workspace,
        CommitRequest::revert_head(revert_operation, correction_revision),
        "immediate-head Revert",
        revert_operation,
        recovery,
    )?;
    let restored = derive_ground(workspace.snapshot(revert_revision)?)?;
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

    println!(
        "Classification correction and Revert\n  corrected Point ordinal: {ordinal}\n  correction Revision: {correction_revision}\n  changed Ground Input Points: {}\n  changed geometry hash: {}\n  Revert Revision: {revert_revision}\n  restored Ground Input Points: {}\n  restored geometry/topology hashes: yes",
        changed.descriptor().input_point_count(),
        changed.descriptor().geometry_hash(),
        restored.descriptor().input_point_count(),
    );
    Ok(restored)
}

fn derive_ground(snapshot: point_workspace::Snapshot) -> AppResult<TerrainSurface> {
    Ok(point_terrain::derive(
        snapshot,
        TerrainRecipe::new(GROUND_CLASSIFICATION),
        TerrainLimits::default(),
    )
    .blocking_wait()?)
}

fn commit_with_recovery(
    workspace: Workspace,
    request: CommitRequest,
    action: &'static str,
    operation: OperationId,
    recovery: &RecoveryPaths<'_>,
) -> AppResult<(Workspace, RevisionId)> {
    save_recovery_record(
        &recovery.record,
        RecoveryRecord {
            workspace: workspace.identity(),
            operation,
        },
    )?;
    let outcome = workspace
        .commit(request, CommitLimits::default())
        .blocking_wait()?;
    match outcome {
        CommitOutcome::Committed(receipt) => {
            clear_recovery_record(&recovery.record)?;
            Ok((workspace, receipt.revision()))
        }
        CommitOutcome::Rejected(reason) => {
            clear_recovery_record(&recovery.record)?;
            Err(io::Error::other(format!(
                "{action} Operation {operation} was rejected: {reason:?}"
            ))
            .into())
        }
        CommitOutcome::Indeterminate(uncertainty) => {
            drop(workspace);
            let (workspace, recovered_operation, revision) = reconcile_recovery_record(recovery)?;
            if recovered_operation != operation {
                return Err(io::Error::other(format!(
                    "{action} recovery resolved Operation {recovered_operation}, expected {operation}"
                ))
                .into());
            }
            println!(
                "{action} acknowledgement was uncertain at {:?}; recovery resolved Revision {revision}",
                uncertainty.phase()
            );
            Ok((workspace, revision))
        }
    }
}

fn reconcile_recovery_record(
    paths: &RecoveryPaths<'_>,
) -> AppResult<(Workspace, OperationId, RevisionId)> {
    let record = load_recovery_record(&paths.record)?;
    let workspace = reopen_workspace(paths)?;
    if workspace.identity() != record.workspace {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery record belongs to another Workspace",
        )
        .into());
    }
    let recovered = recover_operation(&workspace, record.operation)?;
    clear_recovery_record(&paths.record)?;
    let revision = match recovered {
        RecoveredOperation::Committed(revision) => revision,
        RecoveredOperation::Rejected(reason) => {
            return Err(io::Error::other(format!(
                "recorded Operation {} was rejected: {reason:?}",
                record.operation
            ))
            .into());
        }
        RecoveredOperation::NotRecorded => {
            return Err(io::Error::other(format!(
                "recorded Operation {} definitely published no durable state",
                record.operation
            ))
            .into());
        }
    };
    Ok((workspace, record.operation, revision))
}

fn recover_operation(
    workspace: &Workspace,
    operation: OperationId,
) -> AppResult<RecoveredOperation> {
    match workspace.resolve_operation(operation)? {
        OperationResolution::Committed(receipt) => {
            Ok(RecoveredOperation::Committed(receipt.revision()))
        }
        OperationResolution::Rejected(rejection) => {
            Ok(RecoveredOperation::Rejected(rejection.reason()))
        }
        OperationResolution::Retryable(intent) => {
            let expected = intent.revision();
            match workspace
                .retry_operation(operation, CommitLimits::default())
                .blocking_wait()?
            {
                CommitOutcome::Committed(receipt) if receipt.revision() == expected => {
                    Ok(RecoveredOperation::Committed(receipt.revision()))
                }
                CommitOutcome::Rejected(reason) => Ok(RecoveredOperation::Rejected(reason)),
                outcome => Err(io::Error::other(format!(
                    "retry of Operation {operation} did not resolve its recorded intent: {outcome:?}"
                ))
                .into()),
            }
        }
        OperationResolution::NotRecorded => Ok(RecoveredOperation::NotRecorded),
        OperationResolution::Indeterminate(uncertainty) => Err(io::Error::other(format!(
            "Operation {operation} remains indeterminate at {:?}: {}",
            uncertainty.phase(),
            uncertainty.reason()
        ))
        .into()),
    }
}

fn reopen_workspace(paths: &RecoveryPaths<'_>) -> AppResult<Workspace> {
    let source = source_las::open(paths.source).blocking_wait()?;
    let index =
        point_index::prepare(source, paths.index, PrepareLimits::default()).blocking_wait()?;
    Ok(point_workspace::open(paths.workspace, index, OpenLimits::default()).blocking_wait()?)
}

fn recovery_record_path(workspace_path: &Path) -> PathBuf {
    let mut path = workspace_path.as_os_str().to_os_string();
    path.push(".recovery");
    PathBuf::from(path)
}

fn save_recovery_record(path: &Path, record: RecoveryRecord) -> Result<(), io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&encode_recovery_record(record))?;
    file.sync_all()?;
    drop(file);
    sync_parent(path)
}

fn load_recovery_record(path: &Path) -> Result<RecoveryRecord, io::Error> {
    let mut file = File::open(path)?;
    let actual_bytes = file.metadata()?.len();
    if actual_bytes != RECOVERY_RECORD_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("recovery record has {actual_bytes} bytes, expected {RECOVERY_RECORD_BYTES}"),
        ));
    }
    let mut bytes = [0_u8; RECOVERY_RECORD_BYTES];
    file.read_exact(&mut bytes)?;
    decode_recovery_record(&bytes)
}

fn clear_recovery_record(path: &Path) -> Result<(), io::Error> {
    fs::remove_file(path)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), io::Error> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

fn encode_recovery_record(record: RecoveryRecord) -> [u8; RECOVERY_RECORD_BYTES] {
    let mut bytes = [0_u8; RECOVERY_RECORD_BYTES];
    bytes[..8].copy_from_slice(RECOVERY_MAGIC);
    bytes[8..24].copy_from_slice(record.workspace.as_bytes());
    bytes[24..RECOVERY_PAYLOAD_BYTES].copy_from_slice(record.operation.as_bytes());
    let checksum = blake3::hash(&bytes[..RECOVERY_PAYLOAD_BYTES]);
    bytes[RECOVERY_PAYLOAD_BYTES..].copy_from_slice(checksum.as_bytes());
    bytes
}

fn decode_recovery_record(
    bytes: &[u8; RECOVERY_RECORD_BYTES],
) -> Result<RecoveryRecord, io::Error> {
    let expected_checksum = blake3::hash(&bytes[..RECOVERY_PAYLOAD_BYTES]);
    if &bytes[..8] != RECOVERY_MAGIC
        || bytes[RECOVERY_PAYLOAD_BYTES..] != *expected_checksum.as_bytes()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery record is incompatible or corrupt",
        ));
    }
    let workspace = WorkspaceId::try_from_slice(&bytes[8..24])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let operation = OperationId::try_from_slice(&bytes[24..RECOVERY_PAYLOAD_BYTES])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(RecoveryRecord {
        workspace,
        operation,
    })
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

fn required_option_value(
    arguments: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<String, io::Error> {
    let value = arguments
        .next()
        .ok_or_else(|| invalid_input(format!("{option} requires a value")))?;
    value.into_string().map_err(|_| {
        invalid_input(format!(
            "{option} requires Unicode text; paths may remain non-Unicode"
        ))
    })
}

fn set_once(slot: &mut Option<String>, value: String, option: &'static str) -> AppResult<()> {
    if slot.replace(value).is_some() {
        return Err(invalid_input(format!("{option} was supplied more than once")).into());
    }
    Ok(())
}

fn require_new_flag(already_set: bool, option: &'static str) -> Result<(), io::Error> {
    if already_set {
        Err(invalid_input(format!(
            "{option} was supplied more than once"
        )))
    } else {
        Ok(())
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn print_usage() {
    println!(
        "{}",
        concat!(
            "Usage: terrain-demo [OPTIONS] SOURCE INDEX WORKSPACE LANDXML\n",
            "\n",
            "Required deterministic document options:\n",
            "  --date YYYY-MM-DD       LandXML document date; no clock is read\n",
            "  --time HH:MM:SSZ        LandXML UTC document time; no clock is read\n",
            "\n",
            "Optional:\n",
            "  --qa-sample             Evaluate one Surface vertex and one deliberate gap\n",
            "  --assert-crs-metric     Assert Source X/Y/Z are metric metres\n",
            "  --exercise-correction-revert ORDINAL\n",
            "                           Set one exact Ground Point to class 1, derive, Revert, and verify restoration\n",
            "  -h, --help              Show this help\n",
            "\n",
            "The LAS classification Attribute (ID 6) and Ground class (2) are fixed.\n",
            "INDEX is built or opened; WORKSPACE is created or opened; LANDXML is never replaced.",
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_record_round_trips_and_detects_corruption() {
        let record = RecoveryRecord {
            workspace: WorkspaceId::from_bytes([1; 16]).expect("fixture Workspace ID is valid"),
            operation: OperationId::from_bytes([2; 16]).expect("fixture Operation ID is valid"),
        };
        let bytes = encode_recovery_record(record);
        assert_eq!(decode_recovery_record(&bytes).unwrap(), record);

        let mut corrupt = bytes;
        corrupt[12] ^= 0x80;
        assert!(decode_recovery_record(&corrupt).is_err());
    }

    #[test]
    fn recovery_record_sits_next_to_the_workspace() {
        assert_eq!(
            recovery_record_path(Path::new("survey.pcw")),
            Path::new("survey.pcw.recovery")
        );
    }
}
