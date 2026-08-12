// CLI failures retain the full structured Workflow context. Parsing stays in
// one bounded grammar routine so duplicate and positional rules are explicit.
#![allow(clippy::result_large_err, clippy::too_many_lines)]

use std::{ffi::OsString, path::PathBuf};

use point_terrain::{CheckPoint, CheckPointId, LandXmlOptions, TerrainRecipe};
use point_workspace::{OperationId, RevisionId};

use crate::{
    WorkflowFailure, WorkflowLimits, WorkflowPaths, WorkflowRunId, WorkflowRunIntent, inspect_run,
    resume_run, start_run,
};

const DEFAULT_SURFACE_NAME: &str = "Punctra Ground Surface";
const GROUND_CLASSIFICATION: u8 = 2;
const DEFAULT_NON_GROUND_CLASSIFICATION: u8 = 1;
const MAX_CLI_ARGUMENTS: u64 = 2_600;
const MAX_CLI_BYTES: u64 = 256 * 1024;
const MAX_CLI_ARGUMENT_BYTES: u64 = 4 * 1024;
const MAX_ORDINALS: usize = 1_000;
const MAX_CHECK_POINTS: usize = 256;
const USAGE: &str = "\
terrain-demo start|resume [OPTIONS] SOURCE INDEX WORKSPACE RUN_ROOT
terrain-demo inspect RUN_ROOT

WORKSPACE must already exist; pass its current head Revision as --baseline.

Required start/resume options:
  --run-id HEX32                 caller-owned nonzero 128-bit Run ID
  --operation-id HEX32           caller-owned nonzero Workspace Operation ID
  --baseline HEX64               expected baseline Revision ID
  --exclude-ground-ordinal N     exact Source ordinal; repeat for a nonempty set
  --date YYYY-MM-DD              deterministic LandXML document date
  --time HH:MM:SSZ               deterministic LandXML UTC document time

Optional:
  --check-point ID,X,Y,Z         detached QA point; repeatable
  --non-ground-classification N  replacement class (default 1)
  --surface-name TEXT            LandXML Surface name
  --assert-unknown-crs-metric    assert unknown Source CRS coordinates are metres
  -h, --help                     show this help
";

enum Command {
    Help,
    Inspect(PathBuf),
    Start {
        resume: bool,
        paths: WorkflowPaths,
        intent: Box<WorkflowRunIntent>,
    },
}

/// Parses and executes one `terrain-demo` command, returning presentation text.
///
/// # Errors
///
/// Returns a structured workflow failure for invalid or over-limit arguments,
/// or when the selected workflow command cannot complete.
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> Result<String, WorkflowFailure> {
    match parse(arguments)? {
        Command::Help => Ok(USAGE.to_owned()),
        Command::Inspect(root) => {
            let status = inspect_run(root, WorkflowLimits::default())?;
            Ok(format!(
                "Run {}\nOperation {}\nframes {}\ncomplete {}\n",
                status.run(),
                status.operation(),
                status.frame_count(),
                status.is_complete(),
            ))
        }
        Command::Start {
            resume,
            paths,
            intent,
        } => {
            let receipt = if resume {
                resume_run(paths, *intent, WorkflowLimits::default()).blocking_wait()?
            } else {
                start_run(paths, *intent, WorkflowLimits::default()).blocking_wait()?
            };
            Ok(format!(
                "Run complete\nRun {}\nOperation {}\nRevision {}\nreport hash {}\nreport bytes {}\nframes {}\n",
                receipt.run(),
                receipt.operation(),
                receipt.revision(),
                receipt.report_hash(),
                receipt.report_bytes(),
                receipt.frame_count(),
            ))
        }
    }
}

fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, WorkflowFailure> {
    let mut arguments = BoundedArguments::new(arguments.into_iter());
    let Some(first) = arguments.next()? else {
        return Err(invalid("missing command; use --help for usage"));
    };
    if first == "--help" || first == "-h" || first == "help" {
        return Ok(Command::Help);
    }
    if first == "inspect" {
        let root = arguments
            .next()?
            .ok_or_else(|| invalid("inspect requires exactly one RUN_ROOT path"))?;
        if arguments.next()?.is_some() {
            return Err(invalid("inspect requires exactly one RUN_ROOT path"));
        }
        return Ok(Command::Inspect(PathBuf::from(root)));
    }
    let resume = match first.to_str() {
        Some("start") => false,
        Some("resume") => true,
        _ => {
            return Err(invalid(
                "unknown command; expected start, resume, or inspect",
            ));
        }
    };

    let mut run = None;
    let mut operation = None;
    let mut baseline = None;
    let mut ordinals = Vec::new();
    let mut check_points = Vec::new();
    let mut date = None;
    let mut time = None;
    let mut surface_name = None;
    let mut non_ground = None;
    let mut assert_unknown = None;
    let mut paths = Vec::new();
    reserve(&mut ordinals, MAX_ORDINALS, "Ground ordinal storage")?;
    reserve(&mut check_points, MAX_CHECK_POINTS, "Check Point storage")?;
    reserve(&mut paths, 4, "path storage")?;
    while let Some(argument) = arguments.next()? {
        let option = argument.to_str();
        match option {
            Some("--run-id") => {
                set_once(
                    &mut run,
                    parse_hex::<16>(&arguments.require_value("--run-id")?, "Run ID")?,
                    "--run-id",
                )?;
            }
            Some("--operation-id") => {
                set_once(
                    &mut operation,
                    parse_hex::<16>(&arguments.require_value("--operation-id")?, "Operation ID")?,
                    "--operation-id",
                )?;
            }
            Some("--baseline") => {
                set_once(
                    &mut baseline,
                    parse_hex::<32>(&arguments.require_value("--baseline")?, "baseline Revision")?,
                    "--baseline",
                )?;
            }
            Some("--exclude-ground-ordinal") => {
                push_bounded(
                    &mut ordinals,
                    parse_u64(
                        &arguments.require_value("--exclude-ground-ordinal")?,
                        "Ground ordinal",
                    )?,
                    MAX_ORDINALS,
                    "Ground ordinal count",
                )?;
            }
            Some("--check-point") => {
                push_bounded(
                    &mut check_points,
                    parse_check_point(&arguments.require_value("--check-point")?)?,
                    MAX_CHECK_POINTS,
                    "Check Point count",
                )?;
            }
            Some("--date") => {
                set_once(&mut date, arguments.require_value("--date")?, "--date")?;
            }
            Some("--time") => {
                set_once(&mut time, arguments.require_value("--time")?, "--time")?;
            }
            Some("--surface-name") => {
                set_once(
                    &mut surface_name,
                    arguments.require_value("--surface-name")?,
                    "--surface-name",
                )?;
            }
            Some("--non-ground-classification") => {
                set_once(
                    &mut non_ground,
                    parse_u8(
                        &arguments.require_value("--non-ground-classification")?,
                        "non-Ground classification",
                    )?,
                    "--non-ground-classification",
                )?;
            }
            Some("--assert-unknown-crs-metric") => {
                set_once(&mut assert_unknown, (), "--assert-unknown-crs-metric")?;
            }
            Some(value) if value.starts_with('-') => {
                return Err(invalid("unknown option; use --help for usage"));
            }
            _ => push_bounded(
                &mut paths,
                PathBuf::from(argument),
                4,
                "positional path count",
            )?,
        }
    }
    let [source, spatial_index, workspace, run_root]: [PathBuf; 4] = paths
        .try_into()
        .map_err(|_| invalid("start/resume requires SOURCE INDEX WORKSPACE RUN_ROOT"))?;
    let mut landxml = LandXmlOptions::metric_metres(
        surface_name
            .as_ref()
            .map_or(Ok(DEFAULT_SURFACE_NAME), |value| {
                unicode(value, "--surface-name")
            })?,
        unicode(
            date.as_ref().ok_or_else(|| invalid("missing --date"))?,
            "--date",
        )?,
        unicode(
            time.as_ref().ok_or_else(|| invalid("missing --time"))?,
            "--time",
        )?,
    )
    .map_err(|error| invalid(error.to_string()))?;
    if assert_unknown.is_some() {
        landxml = landxml.assert_coordinates_are_metric_metres();
    }
    let run = WorkflowRunId::new(run.ok_or_else(|| invalid("missing --run-id"))?)
        .ok_or_else(|| invalid("Run ID must be nonzero"))?;
    let operation =
        OperationId::from_bytes(operation.ok_or_else(|| invalid("missing --operation-id"))?)
            .map_err(|error| invalid(error.to_string()))?;
    let baseline = RevisionId::from_bytes(baseline.ok_or_else(|| invalid("missing --baseline"))?)
        .map_err(|error| invalid(error.to_string()))?;
    let intent = WorkflowRunIntent::new(
        run,
        operation,
        baseline,
        ordinals,
        non_ground.unwrap_or(DEFAULT_NON_GROUND_CLASSIFICATION),
        TerrainRecipe::new(GROUND_CLASSIFICATION),
        check_points,
        landxml,
    )?;
    Ok(Command::Start {
        resume,
        paths: WorkflowPaths::new(source, spatial_index, workspace, run_root),
        intent: Box::new(intent),
    })
}

struct BoundedArguments<I> {
    inner: I,
    count: u64,
    bytes: u64,
}

impl<I> BoundedArguments<I>
where
    I: Iterator<Item = OsString>,
{
    const fn new(inner: I) -> Self {
        Self {
            inner,
            count: 0,
            bytes: 0,
        }
    }

    fn next(&mut self) -> Result<Option<OsString>, WorkflowFailure> {
        let Some(value) = self.inner.next() else {
            return Ok(None);
        };
        self.count = self.count.saturating_add(1);
        if self.count > MAX_CLI_ARGUMENTS {
            return Err(resource(
                "CLI argument count",
                self.count,
                MAX_CLI_ARGUMENTS,
            ));
        }
        let value_bytes =
            u64::try_from(value.as_os_str().as_encoded_bytes().len()).unwrap_or(u64::MAX);
        if value_bytes > MAX_CLI_ARGUMENT_BYTES {
            return Err(resource(
                "CLI argument bytes",
                value_bytes,
                MAX_CLI_ARGUMENT_BYTES,
            ));
        }
        self.bytes = self.bytes.saturating_add(value_bytes);
        if self.bytes > MAX_CLI_BYTES {
            return Err(resource("CLI total bytes", self.bytes, MAX_CLI_BYTES));
        }
        Ok(Some(value))
    }

    fn require_value(&mut self, option: &'static str) -> Result<OsString, WorkflowFailure> {
        self.next()?
            .ok_or_else(|| invalid(format_args!("{option} requires a value")))
    }
}

fn set_once<T>(
    slot: &mut Option<T>,
    value: T,
    option: &'static str,
) -> Result<(), WorkflowFailure> {
    if slot.replace(value).is_some() {
        Err(invalid(format_args!("duplicate {option}")))
    } else {
        Ok(())
    }
}

fn reserve<T>(
    values: &mut Vec<T>,
    count: usize,
    limit: &'static str,
) -> Result<(), WorkflowFailure> {
    values
        .try_reserve_exact(count)
        .map_err(|_| resource(limit, u64::try_from(count).unwrap_or(u64::MAX), u64::MAX))
}

fn push_bounded<T>(
    values: &mut Vec<T>,
    value: T,
    maximum: usize,
    limit: &'static str,
) -> Result<(), WorkflowFailure> {
    if values.len() == maximum {
        return Err(resource(
            limit,
            u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1),
            u64::try_from(maximum).unwrap_or(u64::MAX),
        ));
    }
    values.push(value);
    Ok(())
}

fn parse_check_point(value: &OsString) -> Result<CheckPoint, WorkflowFailure> {
    let value = unicode(value, "--check-point")?;
    let mut fields = value.split(',');
    let id = fields
        .next()
        .ok_or_else(|| invalid("--check-point requires ID,X,Y,Z"))?;
    let x = fields
        .next()
        .ok_or_else(|| invalid("--check-point requires ID,X,Y,Z"))?;
    let y = fields
        .next()
        .ok_or_else(|| invalid("--check-point requires ID,X,Y,Z"))?;
    let z = fields
        .next()
        .ok_or_else(|| invalid("--check-point requires ID,X,Y,Z"))?;
    if fields.next().is_some() {
        return Err(invalid("--check-point requires ID,X,Y,Z"));
    }
    let id = id
        .parse::<u64>()
        .map_err(|_| invalid("invalid Check Point ID"))?;
    let x = x
        .parse::<f64>()
        .map_err(|_| invalid("invalid Check Point coordinate"))?;
    let y = y
        .parse::<f64>()
        .map_err(|_| invalid("invalid Check Point coordinate"))?;
    let z = z
        .parse::<f64>()
        .map_err(|_| invalid("invalid Check Point coordinate"))?;
    CheckPoint::new(
        CheckPointId::new(id).map_err(|error| invalid(error.to_string()))?,
        [x, y, z],
    )
    .map_err(|error| invalid(error.to_string()))
}

fn parse_hex<const N: usize>(
    value: &OsString,
    name: &'static str,
) -> Result<[u8; N], WorkflowFailure> {
    let value = unicode(value, name)?;
    if value.len() != N * 2 || !value.is_ascii() {
        return Err(invalid(format!(
            "{name} must contain exactly {} hexadecimal characters",
            N * 2
        )));
    }
    let mut bytes = [0; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0])
            .ok_or_else(|| invalid(format!("{name} is not hexadecimal")))?
            << 4)
            | hex_nibble(pair[1]).ok_or_else(|| invalid(format!("{name} is not hexadecimal")))?;
    }
    Ok(bytes)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_u64(value: &OsString, name: &'static str) -> Result<u64, WorkflowFailure> {
    unicode(value, name)?
        .parse()
        .map_err(|_| invalid(format!("{name} must be an unsigned integer")))
}

fn parse_u8(value: &OsString, name: &'static str) -> Result<u8, WorkflowFailure> {
    unicode(value, name)?
        .parse()
        .map_err(|_| invalid(format!("{name} must be an integer from 0 through 255")))
}

fn unicode<'a>(value: &'a OsString, name: &'static str) -> Result<&'a str, WorkflowFailure> {
    value
        .to_str()
        .ok_or_else(|| invalid(format!("{name} requires Unicode text")))
}

fn invalid(message: impl std::fmt::Display) -> WorkflowFailure {
    WorkflowFailure::invalid(crate::diagnostic::WorkflowStage::Validate, message)
}

fn resource(limit: &'static str, required: u64, allowed: u64) -> WorkflowFailure {
    WorkflowFailure::new(
        crate::diagnostic::FailureCode::ResourceLimit,
        crate::diagnostic::WorkflowStage::Validate,
        crate::diagnostic::Certainty::PrePublication,
        crate::diagnostic::FailureContext::default(),
        format_args!("{limit} requires {required}, limit {allowed}"),
        crate::diagnostic::RecoveryAction::RaiseLimitOrNarrow,
    )
}
