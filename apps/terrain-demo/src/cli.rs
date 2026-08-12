// CLI failures retain the full structured Workflow context. Parsing stays in
// one bounded grammar routine so duplicate and positional rules are explicit.
#![allow(clippy::result_large_err, clippy::too_many_lines)]

use std::{ffi::OsString, path::PathBuf};

use point_terrain::{CheckPoint, CheckPointId, LandXmlOptions, TerrainRecipe};
use point_workspace::{OperationId, RevisionId};

use crate::{
    WorkflowFailure, WorkflowLimits, WorkflowPaths, WorkflowRunId, WorkflowRunIntent,
    diagnostic::{Certainty, FailureCode, FailureContext, RecoveryAction, WorkflowStage},
    inspect_and_repair_run,
    journal::JournalError,
    qualification::{QualificationError, QualificationRequest, verify_round_trip},
    report::ReportError,
    resume_run,
    roundtrip::{
        RoundTripDeclaration, RoundTripFailure, RoundTripLimits, RoundTripReport,
        RoundTripTolerances, verify_landxml_round_trip,
    },
    start_run,
};

const DEFAULT_SURFACE_NAME: &str = "Punctra Ground Surface";
const GROUND_CLASSIFICATION: u8 = 2;
const DEFAULT_NON_GROUND_CLASSIFICATION: u8 = 1;
const MAX_CLI_ARGUMENTS: u64 = 2_600;
const MAX_CLI_BYTES: u64 = 256 * 1024;
const MAX_CLI_ARGUMENT_BYTES: u64 = 4 * 1024;
const MAX_ORDINALS: usize = 1_000;
const MAX_CHECK_POINTS: usize = 256;
const MAX_DOWNSTREAM_SETTING_KEY_BYTES: usize = 128;
const MAX_DOWNSTREAM_SETTING_VALUE_BYTES: usize = 1024;
const USAGE: &str = "\
terrain-demo start|resume [OPTIONS] SOURCE INDEX WORKSPACE RUN_ROOT
terrain-demo inspect RUN_ROOT
terrain-demo compare-landxml [OPTIONS] REFERENCE RETURNED
terrain-demo verify-round-trip [OPTIONS] RUN_ROOT RETURNED EVIDENCE_TARGET

WORKSPACE must already exist; pass its current head Revision as --baseline.

Required start/resume options:
  --run-id HEX32                 caller-owned nonzero 128-bit Run ID
  --operation-id HEX32           caller-owned nonzero Workspace Operation ID
  --baseline HEX64               expected baseline Revision ID
  --exclude-ground-ordinal N     exact Source ordinal; repeat for a nonempty set
  --date YYYY-MM-DD              deterministic LandXML document date
  --time HH:MM:SSZ               deterministic LandXML UTC document time
  --assert-unknown-crs-metric    assert Source coordinates are metric metres

Optional:
  --check-point ID,X,Y,Z         detached QA point; repeatable
  --non-ground-classification N  replacement class (default 1)
  --surface-name TEXT            LandXML Surface name
  -h, --help                     show this help

Required compare-landxml options:
  --application TEXT             caller-declared downstream application
  --application-version TEXT     caller-declared downstream version
  --settings-profile TEXT        caller-declared export settings profile
  --horizontal-tolerance-metres N
  --vertical-tolerance-metres N

Required verify-round-trip options:
  --downstream-app TEXT
  --downstream-version TEXT
  --downstream-setting KEY=VALUE (repeatable)
  --horizontal-tolerance-metres N
  --vertical-tolerance-metres N
";

enum Command {
    Help,
    Inspect(PathBuf),
    CompareLandXml {
        reference: PathBuf,
        returned: PathBuf,
        declaration: RoundTripDeclaration,
        tolerances: RoundTripTolerances,
    },
    VerifyRoundTrip {
        request: QualificationRequest,
    },
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
            let status = inspect_and_repair_run(root, WorkflowLimits::default())?;
            Ok(format!(
                "Run {}\nOperation {}\nphase {}\ncomplete {}\n",
                status.run(),
                status.operation(),
                status.phase().as_str(),
                status.is_complete(),
            ))
        }
        Command::CompareLandXml {
            reference,
            returned,
            declaration,
            tolerances,
        } => {
            let report = verify_landxml_round_trip(
                &reference,
                &returned,
                declaration,
                tolerances,
                RoundTripLimits::default(),
            )
            .map_err(|error| round_trip_failure(&error))?;
            Ok(round_trip_summary(&report))
        }
        Command::VerifyRoundTrip { request } => {
            let receipt = verify_round_trip(&request).map_err(qualification_failure)?;
            let disposition = match receipt.disposition {
                crate::report::ReportDisposition::Created => "created",
                crate::report::ReportDisposition::ReconciledExisting => "reconciled_existing",
            };
            let result = if receipt.passed { "passed" } else { "failed" };
            let output = format!(
                "Round-Trip Evidence {result}\ndisposition {disposition}\nevidence hash {}\nevidence bytes {}\n",
                Hex(&receipt.content_hash),
                receipt.byte_length
            );
            if receipt.passed {
                Ok(output)
            } else {
                Err(WorkflowFailure::new(
                    FailureCode::RoundTripSemanticMismatch,
                    WorkflowStage::RoundTrip,
                    Certainty::DurableFact,
                    FailureContext::default(),
                    format_args!(
                        "semantic qualification failed; canonical failed evidence {disposition}; hash {}; bytes {}",
                        Hex(&receipt.content_hash),
                        receipt.byte_length
                    ),
                    RecoveryAction::ReviewReturnedLandXml,
                ))
            }
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
                "Run complete\nRun {}\nOperation {}\nRevision {}\nreport hash {}\nreport bytes {}\n",
                receipt.run(),
                receipt.operation(),
                receipt.revision(),
                receipt.report_hash(),
                receipt.report_bytes(),
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
    if first == "compare-landxml" {
        return parse_compare_landxml(&mut arguments).map_err(round_trip_cli_failure);
    }
    if first == "verify-round-trip" {
        return parse_verify_round_trip(&mut arguments).map_err(round_trip_cli_failure);
    }
    let resume = match first.to_str() {
        Some("start") => false,
        Some("resume") => true,
        _ => {
            return Err(invalid(
                "unknown command; expected start, resume, inspect, compare-landxml, or verify-round-trip",
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

fn parse_verify_round_trip<I>(
    arguments: &mut BoundedArguments<I>,
) -> Result<Command, WorkflowFailure>
where
    I: Iterator<Item = OsString>,
{
    let mut application = None;
    let mut version = None;
    let mut settings = Vec::new();
    let mut horizontal_tolerance = None;
    let mut vertical_tolerance = None;
    let mut paths = Vec::new();
    reserve(&mut settings, 64, "downstream setting storage")?;
    reserve(&mut paths, 3, "qualification path storage")?;
    while let Some(argument) = arguments.next()? {
        match argument.to_str() {
            Some("--downstream-app") => set_once(
                &mut application,
                arguments.require_value("--downstream-app")?,
                "--downstream-app",
            )?,
            Some("--downstream-version") => set_once(
                &mut version,
                arguments.require_value("--downstream-version")?,
                "--downstream-version",
            )?,
            Some("--downstream-setting") => {
                let value = arguments.require_value("--downstream-setting")?;
                let value = unicode(&value, "--downstream-setting")?;
                let Some((key, setting)) = value.split_once('=') else {
                    return Err(invalid("--downstream-setting requires KEY=VALUE"));
                };
                if key.is_empty() || setting.is_empty() {
                    return Err(invalid(
                        "--downstream-setting requires nonempty KEY and VALUE",
                    ));
                }
                validate_setting_component(
                    key,
                    "--downstream-setting key",
                    MAX_DOWNSTREAM_SETTING_KEY_BYTES,
                )?;
                validate_setting_component(
                    setting,
                    "--downstream-setting value",
                    MAX_DOWNSTREAM_SETTING_VALUE_BYTES,
                )?;
                push_bounded(
                    &mut settings,
                    (key.to_owned(), setting.to_owned()),
                    64,
                    "downstream setting count",
                )?;
            }
            Some("--horizontal-tolerance-metres") => set_once(
                &mut horizontal_tolerance,
                parse_f64(
                    &arguments.require_value("--horizontal-tolerance-metres")?,
                    "horizontal tolerance",
                )?,
                "--horizontal-tolerance-metres",
            )?,
            Some("--vertical-tolerance-metres") => set_once(
                &mut vertical_tolerance,
                parse_f64(
                    &arguments.require_value("--vertical-tolerance-metres")?,
                    "vertical tolerance",
                )?,
                "--vertical-tolerance-metres",
            )?,
            Some(value) if value.starts_with('-') => {
                return Err(invalid(
                    "unknown verify-round-trip option; use --help for usage",
                ));
            }
            _ => push_bounded(
                &mut paths,
                PathBuf::from(argument),
                3,
                "qualification positional path count",
            )?,
        }
    }
    if settings.is_empty() {
        return Err(invalid("at least one --downstream-setting is required"));
    }
    settings.sort_by(|left, right| left.0.cmp(&right.0));
    if settings.windows(2).any(|values| values[0].0 == values[1].0) {
        return Err(invalid("duplicate --downstream-setting key is not allowed"));
    }
    let [run_root, returned_landxml, evidence_target]: [PathBuf; 3] = paths
        .try_into()
        .map_err(|_| invalid("verify-round-trip requires RUN_ROOT RETURNED EVIDENCE_TARGET"))?;
    let declaration = RoundTripDeclaration::new(
        unicode(
            application
                .as_ref()
                .ok_or_else(|| invalid("missing --downstream-app"))?,
            "--downstream-app",
        )?,
        unicode(
            version
                .as_ref()
                .ok_or_else(|| invalid("missing --downstream-version"))?,
            "--downstream-version",
        )?,
        "qualification-uses-structured-settings-v1",
    )
    .map_err(|error| round_trip_failure(&error))?;
    let tolerances = RoundTripTolerances::new(
        horizontal_tolerance.ok_or_else(|| invalid("missing --horizontal-tolerance-metres"))?,
        vertical_tolerance.ok_or_else(|| invalid("missing --vertical-tolerance-metres"))?,
    )
    .map_err(|error| round_trip_failure(&error))?;
    Ok(Command::VerifyRoundTrip {
        request: QualificationRequest {
            run_root,
            returned_landxml,
            evidence_target,
            declaration,
            downstream_settings: settings,
            tolerances,
        },
    })
}

fn validate_setting_component(
    value: &str,
    name: &'static str,
    max_bytes: usize,
) -> Result<(), WorkflowFailure> {
    if value.trim() != value {
        return Err(invalid(format_args!(
            "{name} must not contain surrounding whitespace"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid(format_args!(
            "{name} must not contain control characters"
        )));
    }
    if value.len() > max_bytes {
        return Err(resource(name, value.len() as u64, max_bytes as u64));
    }
    Ok(())
}

fn parse_compare_landxml<I>(arguments: &mut BoundedArguments<I>) -> Result<Command, WorkflowFailure>
where
    I: Iterator<Item = OsString>,
{
    let mut application = None;
    let mut version = None;
    let mut settings_profile = None;
    let mut horizontal_tolerance = None;
    let mut vertical_tolerance = None;
    let mut paths = Vec::new();
    reserve(&mut paths, 2, "round-trip path storage")?;
    while let Some(argument) = arguments.next()? {
        match argument.to_str() {
            Some("--application") => set_once(
                &mut application,
                arguments.require_value("--application")?,
                "--application",
            )?,
            Some("--application-version") => set_once(
                &mut version,
                arguments.require_value("--application-version")?,
                "--application-version",
            )?,
            Some("--settings-profile") => set_once(
                &mut settings_profile,
                arguments.require_value("--settings-profile")?,
                "--settings-profile",
            )?,
            Some("--horizontal-tolerance-metres") => set_once(
                &mut horizontal_tolerance,
                parse_f64(
                    &arguments.require_value("--horizontal-tolerance-metres")?,
                    "horizontal tolerance",
                )?,
                "--horizontal-tolerance-metres",
            )?,
            Some("--vertical-tolerance-metres") => set_once(
                &mut vertical_tolerance,
                parse_f64(
                    &arguments.require_value("--vertical-tolerance-metres")?,
                    "vertical tolerance",
                )?,
                "--vertical-tolerance-metres",
            )?,
            Some(value) if value.starts_with('-') => {
                return Err(invalid(
                    "unknown compare-landxml option; use --help for usage",
                ));
            }
            _ => push_bounded(
                &mut paths,
                PathBuf::from(argument),
                2,
                "round-trip positional path count",
            )?,
        }
    }
    let [reference, returned]: [PathBuf; 2] = paths
        .try_into()
        .map_err(|_| invalid("compare-landxml requires REFERENCE and RETURNED paths"))?;
    let declaration = RoundTripDeclaration::new(
        unicode(
            application
                .as_ref()
                .ok_or_else(|| invalid("missing --application"))?,
            "--application",
        )?,
        unicode(
            version
                .as_ref()
                .ok_or_else(|| invalid("missing --application-version"))?,
            "--application-version",
        )?,
        unicode(
            settings_profile
                .as_ref()
                .ok_or_else(|| invalid("missing --settings-profile"))?,
            "--settings-profile",
        )?,
    )
    .map_err(|error| round_trip_failure(&error))?;
    let tolerances = RoundTripTolerances::new(
        horizontal_tolerance.ok_or_else(|| invalid("missing --horizontal-tolerance-metres"))?,
        vertical_tolerance.ok_or_else(|| invalid("missing --vertical-tolerance-metres"))?,
    )
    .map_err(|error| round_trip_failure(&error))?;
    Ok(Command::CompareLandXml {
        reference,
        returned,
        declaration,
        tolerances,
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

fn parse_f64(value: &OsString, name: &'static str) -> Result<f64, WorkflowFailure> {
    unicode(value, name)?
        .parse()
        .map_err(|_| invalid(format!("{name} must be a decimal number")))
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

fn round_trip_failure(error: &RoundTripFailure) -> WorkflowFailure {
    let (code, action) = error.kind().workflow_mapping();
    WorkflowFailure::new(
        code,
        WorkflowStage::RoundTrip,
        Certainty::PrePublication,
        FailureContext::default(),
        error.diagnostic(),
        action,
    )
}

fn qualification_failure(error: QualificationError) -> WorkflowFailure {
    if error.is_publication_indeterminate() {
        return WorkflowFailure::new(
            FailureCode::PublicationIndeterminate,
            WorkflowStage::RoundTrip,
            Certainty::Indeterminate(crate::diagnostic::PublicationPhase::RoundTripEvidence),
            FailureContext::default(),
            error,
            RecoveryAction::StopAndPreserve,
        );
    }
    let (code, certainty, action) = qualification_mapping(&error);
    WorkflowFailure::new(
        code,
        WorkflowStage::RoundTrip,
        certainty,
        FailureContext::default(),
        error,
        action,
    )
}

fn qualification_mapping(error: &QualificationError) -> (FailureCode, Certainty, RecoveryAction) {
    match error {
        QualificationError::Resource { .. }
        | QualificationError::Publication(ReportError::Resource { .. }) => (
            FailureCode::RoundTripResourceLimit,
            Certainty::PrePublication,
            RecoveryAction::UseSupportedRoundTripSize,
        ),
        QualificationError::Cancelled | QualificationError::Publication(ReportError::Cancelled) => {
            (
                FailureCode::Cancelled,
                Certainty::PrePublication,
                RecoveryAction::ResumeSameRun,
            )
        }
        QualificationError::Io { .. }
        | QualificationError::Journal(JournalError::Io { .. })
        | QualificationError::Publication(ReportError::Io { .. }) => (
            FailureCode::Io,
            Certainty::PrePublication,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
        QualificationError::Journal(
            JournalError::Corrupt(_) | JournalError::Incompatible(_) | JournalError::Invalid(_),
        ) => (
            FailureCode::JournalCorrupt,
            Certainty::DurableFact,
            RecoveryAction::StopAndPreserve,
        ),
        QualificationError::Journal(JournalError::Resource { .. }) => (
            FailureCode::ResourceLimit,
            Certainty::PrePublication,
            RecoveryAction::RaiseLimitOrNarrow,
        ),
        QualificationError::Journal(_) => (
            FailureCode::JournalConflict,
            Certainty::DurableFact,
            RecoveryAction::StopAndPreserve,
        ),
        QualificationError::Publication(
            ReportError::Conflict { .. } | ReportError::TargetConflict { .. },
        ) => (
            FailureCode::OutputConflict,
            Certainty::DurableFact,
            RecoveryAction::RemoveOrRenameConflictingTarget,
        ),
        QualificationError::Publication(
            ReportError::Indeterminate { .. } | ReportError::TargetChanged { .. },
        ) => (
            FailureCode::PublicationIndeterminate,
            Certainty::Indeterminate(crate::diagnostic::PublicationPhase::RoundTripEvidence),
            RecoveryAction::StopAndPreserve,
        ),
        QualificationError::Invalid(_)
        | QualificationError::InputChanged(_)
        | QualificationError::Xml(_)
        | QualificationError::Subset(_)
        | QualificationError::Report(_)
        | QualificationError::Provenance(_)
        | QualificationError::Publication(ReportError::Invalid(_)) => (
            FailureCode::RoundTripInvalidInput,
            Certainty::PrePublication,
            RecoveryAction::CorrectRoundTripInput,
        ),
    }
}

fn round_trip_cli_failure(error: WorkflowFailure) -> WorkflowFailure {
    match error.code {
        FailureCode::RoundTripInvalidInput
        | FailureCode::RoundTripResourceLimit
        | FailureCode::RoundTripSemanticMismatch => error,
        FailureCode::ResourceLimit => WorkflowFailure::new(
            FailureCode::RoundTripResourceLimit,
            WorkflowStage::RoundTrip,
            Certainty::PrePublication,
            FailureContext::default(),
            error.diagnostic(),
            RecoveryAction::UseSupportedRoundTripSize,
        ),
        _ => WorkflowFailure::new(
            FailureCode::RoundTripInvalidInput,
            WorkflowStage::RoundTrip,
            Certainty::PrePublication,
            FailureContext::default(),
            error.diagnostic(),
            RecoveryAction::CorrectRoundTripInput,
        ),
    }
}

fn round_trip_summary(report: &RoundTripReport) -> String {
    let tolerances = report.tolerances();
    let reference_hash = report.reference_content_hash();
    let returned_hash = report.returned_content_hash();
    format!(
        "LandXML semantic comparison passed\n\
caller-declared application {}\n\
caller-declared application version {}\n\
caller-declared settings profile {}\n\
horizontal tolerance metres {:e}\n\
vertical tolerance metres {:e}\n\
reference hash {}\n\
reference bytes {}\n\
returned hash {}\n\
returned bytes {}\n\
vertices {}\n\
faces {}\n\
vertex comparisons {}\n\
maximum easting drift metres {:e}\n\
maximum northing drift metres {:e}\n\
maximum horizontal drift metres {:e}\n\
maximum vertical drift metres {:e}\n\
exact bytes {}\n\
topology matches {}\n\
run bound false\n\
canonical evidence published false\n\
external application execution verified false\n",
        report.declared_application(),
        report.declared_version(),
        report.declared_settings_profile(),
        tolerances.horizontal_metres(),
        tolerances.vertical_metres(),
        Hex(&reference_hash),
        report.reference_bytes(),
        Hex(&returned_hash),
        report.returned_bytes(),
        report.vertex_count(),
        report.face_count(),
        report.comparison_count(),
        report.max_easting_drift_metres(),
        report.max_northing_drift_metres(),
        report.max_horizontal_drift_metres(),
        report.max_vertical_drift_metres(),
        report.exact_bytes(),
        report.topology_matches(),
    )
}

struct Hex<'a>(&'a [u8]);

impl std::fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, io, path::PathBuf};

    use super::*;

    #[test]
    fn qualification_settings_are_sorted_structurally_without_delimiter_collisions() {
        let Command::VerifyRoundTrip { request } = parse(arguments(&[
            "--downstream-setting",
            "z=a;b=c",
            "--downstream-setting",
            "a=b;c=d",
        ]))
        .unwrap() else {
            panic!("expected qualification command")
        };
        assert_eq!(
            request.downstream_settings,
            vec![
                ("a".to_owned(), "b;c=d".to_owned()),
                ("z".to_owned(), "a;b=c".to_owned()),
            ]
        );
    }

    #[test]
    fn qualification_rejects_duplicate_keys_and_invalid_components() {
        for settings in [
            [
                "--downstream-setting",
                "key=one",
                "--downstream-setting",
                "key=two",
            ],
            [
                "--downstream-setting",
                " key=value",
                "--downstream-setting",
                "ok=yes",
            ],
            [
                "--downstream-setting",
                "key=value\n",
                "--downstream-setting",
                "ok=yes",
            ],
        ] {
            assert!(parse(arguments(&settings)).is_err());
        }
    }

    #[test]
    fn qualification_operational_failures_keep_owner_specific_taxonomy() {
        let cases = [
            (
                QualificationError::Cancelled,
                FailureCode::Cancelled,
                RecoveryAction::ResumeSameRun,
            ),
            (
                QualificationError::Io {
                    operation: "read",
                    path: PathBuf::from("input.xml"),
                    source: io::Error::other("injected I/O"),
                },
                FailureCode::Io,
                RecoveryAction::RetryAfterRestoringDisk,
            ),
            (
                QualificationError::Journal(JournalError::Corrupt("injected corruption")),
                FailureCode::JournalCorrupt,
                RecoveryAction::StopAndPreserve,
            ),
            (
                QualificationError::Publication(ReportError::Conflict {
                    path: PathBuf::from("evidence.json"),
                    expected_hash: [1; 32],
                    actual_hash: [2; 32],
                }),
                FailureCode::OutputConflict,
                RecoveryAction::RemoveOrRenameConflictingTarget,
            ),
            (
                QualificationError::Publication(ReportError::Resource {
                    limit: "evidence bytes",
                    required: 2,
                    allowed: 1,
                }),
                FailureCode::RoundTripResourceLimit,
                RecoveryAction::UseSupportedRoundTripSize,
            ),
        ];
        for (error, expected_code, expected_action) in cases {
            let failure = qualification_failure(error);
            assert_eq!(failure.code, expected_code);
            assert_eq!(failure.action, expected_action);
        }
    }

    fn arguments(settings: &[&str]) -> Vec<OsString> {
        let mut arguments = vec![
            "verify-round-trip",
            "--downstream-app",
            "app",
            "--downstream-version",
            "1",
        ];
        arguments.extend_from_slice(settings);
        arguments.extend_from_slice(&[
            "--horizontal-tolerance-metres",
            "0",
            "--vertical-tolerance-metres",
            "0",
            "run",
            "returned.xml",
            "evidence.json",
        ]);
        arguments.into_iter().map(OsString::from).collect()
    }
}
