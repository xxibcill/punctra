// CLI failures retain the full structured Workflow context. Parsing stays in
// one bounded grammar routine so duplicate and positional rules are explicit.
#![allow(clippy::result_large_err, clippy::too_many_lines)]

use std::{ffi::OsString, path::PathBuf};

use point_terrain::{CheckPoint, CheckPointId, LandXmlOptions, TerrainRecipe};
use point_workspace::{OperationId, RevisionId};

use crate::{
    WorkflowFailure, WorkflowLimits, WorkflowPaths, WorkflowRunId, WorkflowRunIntent,
    diagnostic::{Certainty, FailureCode, FailureContext, RecoveryAction, WorkflowStage},
    inspect_and_repair_run, resume_run,
    roundtrip::{
        RoundTripDeclaration, RoundTripFailure, RoundTripLimits, RoundTripReport,
        RoundTripTolerances, verify_landxml_round_trip,
    },
    roundtrip_evidence::{
        QualificationResult, RoundTripEvidenceError, RoundTripEvidenceReceipt, verify_round_trip,
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
  --downstream-app TEXT          caller-declared downstream application
  --downstream-version TEXT      caller-declared downstream version
  --downstream-setting TEXT      one opaque caller settings-profile label
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
        run_root: PathBuf,
        returned: PathBuf,
        evidence_target: PathBuf,
        declaration: RoundTripDeclaration,
        tolerances: RoundTripTolerances,
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
        Command::VerifyRoundTrip {
            run_root,
            returned,
            evidence_target,
            declaration,
            tolerances,
        } => {
            let receipt = verify_round_trip(
                &run_root,
                &returned,
                &evidence_target,
                declaration,
                tolerances,
            )
            .map_err(round_trip_evidence_failure)?;
            if let Some(reason) = receipt.failure_reason {
                return Err(WorkflowFailure::new(
                    reason.failure_code(),
                    WorkflowStage::RoundTrip,
                    Certainty::DurableFact,
                    FailureContext {
                        run: Some(receipt.run),
                        ..FailureContext::default()
                    },
                    format_args!(
                        "canonical failed evidence published with hash {} and {} bytes",
                        Hex(&receipt.evidence_hash),
                        receipt.evidence_bytes
                    ),
                    RecoveryAction::ReviewReturnedLandXml,
                ));
            }
            Ok(round_trip_evidence_summary(receipt))
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
    let parsed = parse_round_trip_arguments(
        arguments,
        RoundTripCliGrammar {
            application: "--downstream-app",
            version: "--downstream-version",
            settings_profile: "--downstream-setting",
            path_storage_limit: "round-trip evidence path storage",
            positional_limit: "round-trip evidence positional path count",
            positional_error: "verify-round-trip requires RUN_ROOT, RETURNED, and EVIDENCE_TARGET paths",
            unknown_option_error: "unknown verify-round-trip option; use --help for usage",
        },
    )?;
    let [run_root, returned, evidence_target] = parsed.paths;
    Ok(Command::VerifyRoundTrip {
        run_root,
        returned,
        evidence_target,
        declaration: parsed.declaration,
        tolerances: parsed.tolerances,
    })
}

fn parse_compare_landxml<I>(arguments: &mut BoundedArguments<I>) -> Result<Command, WorkflowFailure>
where
    I: Iterator<Item = OsString>,
{
    let parsed = parse_round_trip_arguments(
        arguments,
        RoundTripCliGrammar {
            application: "--application",
            version: "--application-version",
            settings_profile: "--settings-profile",
            path_storage_limit: "round-trip path storage",
            positional_limit: "round-trip positional path count",
            positional_error: "compare-landxml requires REFERENCE and RETURNED paths",
            unknown_option_error: "unknown compare-landxml option; use --help for usage",
        },
    )?;
    let [reference, returned] = parsed.paths;
    Ok(Command::CompareLandXml {
        reference,
        returned,
        declaration: parsed.declaration,
        tolerances: parsed.tolerances,
    })
}

#[derive(Clone, Copy)]
struct RoundTripCliGrammar {
    application: &'static str,
    version: &'static str,
    settings_profile: &'static str,
    path_storage_limit: &'static str,
    positional_limit: &'static str,
    positional_error: &'static str,
    unknown_option_error: &'static str,
}

struct ParsedRoundTripArguments<const PATHS: usize> {
    paths: [PathBuf; PATHS],
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
}

fn parse_round_trip_arguments<I, const PATHS: usize>(
    arguments: &mut BoundedArguments<I>,
    grammar: RoundTripCliGrammar,
) -> Result<ParsedRoundTripArguments<PATHS>, WorkflowFailure>
where
    I: Iterator<Item = OsString>,
{
    let mut application = None;
    let mut version = None;
    let mut settings_profile = None;
    let mut horizontal_tolerance = None;
    let mut vertical_tolerance = None;
    let mut paths = Vec::new();
    reserve(&mut paths, PATHS, grammar.path_storage_limit)?;
    while let Some(argument) = arguments.next()? {
        match argument.to_str() {
            Some(option) if option == grammar.application => set_once(
                &mut application,
                arguments.require_value(grammar.application)?,
                grammar.application,
            )?,
            Some(option) if option == grammar.version => set_once(
                &mut version,
                arguments.require_value(grammar.version)?,
                grammar.version,
            )?,
            Some(option) if option == grammar.settings_profile => set_once(
                &mut settings_profile,
                arguments.require_value(grammar.settings_profile)?,
                grammar.settings_profile,
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
                return Err(invalid(grammar.unknown_option_error));
            }
            _ => push_bounded(
                &mut paths,
                PathBuf::from(argument),
                PATHS,
                grammar.positional_limit,
            )?,
        }
    }
    let paths = paths
        .try_into()
        .map_err(|_| invalid(grammar.positional_error))?;
    let declaration = RoundTripDeclaration::new(
        unicode(
            application
                .as_ref()
                .ok_or_else(|| invalid(format_args!("missing {}", grammar.application)))?,
            grammar.application,
        )?,
        unicode(
            version
                .as_ref()
                .ok_or_else(|| invalid(format_args!("missing {}", grammar.version)))?,
            grammar.version,
        )?,
        unicode(
            settings_profile
                .as_ref()
                .ok_or_else(|| invalid(format_args!("missing {}", grammar.settings_profile)))?,
            grammar.settings_profile,
        )?,
    )
    .map_err(|error| round_trip_failure(&error))?;
    let tolerances = RoundTripTolerances::new(
        horizontal_tolerance.ok_or_else(|| invalid("missing --horizontal-tolerance-metres"))?,
        vertical_tolerance.ok_or_else(|| invalid("missing --vertical-tolerance-metres"))?,
    )
    .map_err(|error| round_trip_failure(&error))?;
    Ok(ParsedRoundTripArguments {
        paths,
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

fn round_trip_evidence_failure(error: RoundTripEvidenceError) -> WorkflowFailure {
    match error {
        RoundTripEvidenceError::Comparison(error) => round_trip_failure(&error),
        RoundTripEvidenceError::Publication(error) => {
            use crate::canonical_output::CanonicalOutputError;
            match error {
                error @ (CanonicalOutputError::Conflict { .. }
                | CanonicalOutputError::TargetConflict { .. }) => WorkflowFailure::new(
                    FailureCode::OutputConflict,
                    WorkflowStage::RoundTrip,
                    Certainty::DurableFact,
                    FailureContext::default(),
                    error,
                    RecoveryAction::RemoveOrRenameConflictingTarget,
                ),
                error @ (CanonicalOutputError::Indeterminate { .. }
                | CanonicalOutputError::TargetChanged { .. }) => WorkflowFailure::new(
                    FailureCode::PublicationIndeterminate,
                    WorkflowStage::RoundTrip,
                    Certainty::Indeterminate(
                        crate::diagnostic::PublicationPhase::RoundTripEvidenceTarget,
                    ),
                    FailureContext::default(),
                    error,
                    RecoveryAction::StopAndPreserve,
                ),
                error @ CanonicalOutputError::Resource { .. } => WorkflowFailure::new(
                    FailureCode::RoundTripResourceLimit,
                    WorkflowStage::RoundTrip,
                    Certainty::PrePublication,
                    FailureContext::default(),
                    error,
                    RecoveryAction::UseSupportedRoundTripSize,
                ),
                error @ CanonicalOutputError::Cancelled => WorkflowFailure::new(
                    FailureCode::Cancelled,
                    WorkflowStage::RoundTrip,
                    Certainty::PrePublication,
                    FailureContext::default(),
                    error,
                    RecoveryAction::ResumeSameRun,
                ),
                error @ CanonicalOutputError::Io { .. } => WorkflowFailure::new(
                    FailureCode::Io,
                    WorkflowStage::RoundTrip,
                    Certainty::PrePublication,
                    FailureContext::default(),
                    error,
                    RecoveryAction::RetryAfterRestoringDisk,
                ),
                error @ CanonicalOutputError::Invalid(_) => WorkflowFailure::new(
                    FailureCode::RoundTripInvalidInput,
                    WorkflowStage::RoundTrip,
                    Certainty::PrePublication,
                    FailureContext::default(),
                    error,
                    RecoveryAction::CorrectRoundTripInput,
                ),
            }
        }
        RoundTripEvidenceError::Invalid(error) => WorkflowFailure::new(
            FailureCode::RoundTripInvalidInput,
            WorkflowStage::RoundTrip,
            Certainty::PrePublication,
            FailureContext::default(),
            error,
            RecoveryAction::CorrectRoundTripInput,
        ),
        RoundTripEvidenceError::Journal(error) => round_trip_journal_failure(error),
    }
}

fn round_trip_journal_failure(error: crate::journal::JournalError) -> WorkflowFailure {
    use crate::journal::JournalError;

    let (code, certainty, action) = match &error {
        JournalError::Resource { .. } => (
            FailureCode::RoundTripResourceLimit,
            Certainty::PrePublication,
            RecoveryAction::UseSupportedRoundTripSize,
        ),
        JournalError::Corrupt(_) | JournalError::Incompatible(_) => (
            FailureCode::JournalCorrupt,
            Certainty::DurableFact,
            RecoveryAction::StopAndPreserve,
        ),
        JournalError::Exists(_) | JournalError::Conflict(_) => (
            FailureCode::JournalConflict,
            Certainty::DurableFact,
            RecoveryAction::StopAndPreserve,
        ),
        JournalError::Locked => (
            FailureCode::Io,
            Certainty::PrePublication,
            RecoveryAction::ResumeSameRun,
        ),
        JournalError::Entropy | JournalError::Io { .. } => (
            FailureCode::Io,
            Certainty::PrePublication,
            RecoveryAction::RetryAfterRestoringDisk,
        ),
        JournalError::Invalid(_) => (
            FailureCode::RoundTripInvalidInput,
            Certainty::PrePublication,
            RecoveryAction::CorrectRoundTripInput,
        ),
        JournalError::Indeterminate { .. } | JournalError::CheckpointIndeterminate { .. } => (
            FailureCode::PublicationIndeterminate,
            Certainty::Indeterminate(crate::diagnostic::PublicationPhase::RoundTripEvidenceTarget),
            RecoveryAction::StopAndPreserve,
        ),
    };
    WorkflowFailure::new(
        code,
        WorkflowStage::RoundTrip,
        certainty,
        FailureContext::default(),
        error,
        action,
    )
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

fn round_trip_evidence_summary(receipt: RoundTripEvidenceReceipt) -> String {
    debug_assert_eq!(receipt.result, QualificationResult::Passed);
    format!(
        "LandXML round-trip {}\nRun {}\ncanonical evidence published true\nevidence hash {}\nevidence bytes {}\nexternal application execution verified false\n",
        receipt.result.as_str(),
        receipt.run,
        Hex(&receipt.evidence_hash),
        receipt.evidence_bytes,
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
