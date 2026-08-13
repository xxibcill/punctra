//! Run-bound canonical evidence for one private `LandXML` qualification attempt.
#![allow(clippy::too_many_lines)]

use std::{
    fmt::{self, Write as _},
    io,
    path::Path,
};

use foundation_runtime::OperationControl;

use crate::{
    journal::{Complete, CompleteRunSnapshot, JournalLimits, WorkflowRunId, read_complete_run},
    publication::DirectoryWitness,
    report::{
        CanonicalOutputError, CanonicalOutputLimits, CanonicalOutputReceipt, REPORT_HASH_DOMAIN,
        REPORT_SCHEMA, ensure_evidence,
    },
    roundtrip::{
        RoundTripDeclaration, RoundTripEvaluation, RoundTripFailure, RoundTripLimits,
        RoundTripTolerances,
    },
    roundtrip_file::{CapturedRoundTripFile, capture_round_trip_file},
    roundtrip_stream::evaluate_streaming_round_trip_with_control,
};

const EVIDENCE_SCHEMA: &str = "punctra.terrain-demo.landxml-round-trip-evidence.v1";
const MATCHER_VERSION: &str = "punctra-landxml-semantic-match-v1";
const DEFAULT_MAX_EVIDENCE_BYTES: u64 = 1024 * 1024;
const DEFAULT_MAX_EVIDENCE_WRITE_BUFFER_BYTES: u64 = 8 * 1024;
const DEFAULT_MAX_EVIDENCE_WORKING_BYTES: u64 =
    DEFAULT_MAX_EVIDENCE_BYTES + DEFAULT_MAX_EVIDENCE_WRITE_BUFFER_BYTES;
const PATH_BINDING_BYTES: u64 = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QualificationResult {
    Passed,
    Failed,
}

impl QualificationResult {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RoundTripEvidenceReceipt {
    pub(crate) run: WorkflowRunId,
    pub(crate) result: QualificationResult,
    pub(crate) evidence_hash: [u8; 32],
    pub(crate) evidence_bytes: u64,
    pub(crate) failure_reason: Option<crate::roundtrip::RoundTripReason>,
}

#[derive(Debug)]
pub(crate) enum RoundTripEvidenceError {
    Invalid(String),
    Journal(crate::journal::JournalError),
    Comparison(RoundTripFailure),
    Publication(CanonicalOutputError),
}

impl fmt::Display for RoundTripEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Journal(error) => write!(formatter, "{error}"),
            Self::Comparison(error) => write!(formatter, "{error}"),
            Self::Publication(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for RoundTripEvidenceError {}

impl From<crate::journal::JournalError> for RoundTripEvidenceError {
    fn from(error: crate::journal::JournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<RoundTripFailure> for RoundTripEvidenceError {
    fn from(error: RoundTripFailure) -> Self {
        Self::Comparison(error)
    }
}

impl From<CanonicalOutputError> for RoundTripEvidenceError {
    fn from(error: CanonicalOutputError) -> Self {
        Self::Publication(error)
    }
}

pub(crate) fn verify_round_trip(
    run_root: &Path,
    returned_landxml: &Path,
    evidence_target: &Path,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
) -> Result<RoundTripEvidenceReceipt, RoundTripEvidenceError> {
    verify_round_trip_with_control(
        run_root,
        returned_landxml,
        evidence_target,
        declaration,
        tolerances,
        &OperationControl::new(),
    )
}

fn verify_round_trip_with_control(
    run_root: &Path,
    returned_landxml: &Path,
    evidence_target: &Path,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    control: &OperationControl,
) -> Result<RoundTripEvidenceReceipt, RoundTripEvidenceError> {
    check_cancelled(control)?;
    let run_witness = DirectoryWitness::capture(run_root).map_err(|error| {
        RoundTripEvidenceError::Invalid(format!("Run root cannot be witnessed: {error}"))
    })?;
    let evidence_parent_witness = require_external_target(run_root, evidence_target)?;
    let journal = read_complete_run(&run_root.join("run.pwf"), JournalLimits::default())?;
    if crate::journal::bind_path(run_root, PATH_BINDING_BYTES)? != journal.intent.path_bindings[3] {
        return Err(RoundTripEvidenceError::Invalid(
            "Run root path differs from the Complete Run binding".to_owned(),
        ));
    }
    check_cancelled(control)?;
    let complete = journal.complete;
    let audit = capture_round_trip_file(&run_root.join("audit.json"), DEFAULT_MAX_EVIDENCE_BYTES)?;
    let audit_hash = domain_hash(REPORT_HASH_DOMAIN, &audit.bytes);
    if audit_hash != complete.report_hash {
        return Err(RoundTripEvidenceError::Invalid(
            "audit.json does not match the Complete checkpoint".to_owned(),
        ));
    }
    validate_audit_bindings(&journal, &audit.bytes)?;
    let limits = RoundTripLimits::full_v07_export();
    let streaming = evaluate_streaming_round_trip_with_control(
        &run_root.join("terrain.xml"),
        returned_landxml,
        declaration,
        tolerances,
        limits,
        control,
    )?;
    let evaluation = &streaming.evaluation;
    if evaluation.reference_content_hash() != complete.landxml_hash {
        return Err(RoundTripEvidenceError::Invalid(
            "terrain.xml does not match the Complete checkpoint".to_owned(),
        ));
    }
    if evaluation.reference_bytes() != journal.export.byte_length {
        return Err(RoundTripEvidenceError::Invalid(
            "terrain.xml byte length does not match the ExportEnsured checkpoint".to_owned(),
        ));
    }
    run_witness.verify().map_err(|error| {
        RoundTripEvidenceError::Invalid(format!("Run root changed during qualification: {error}"))
    })?;
    journal.verify_unchanged()?;
    audit.verify()?;
    streaming.verify_inputs()?;
    evidence_parent_witness.verify().map_err(|error| {
        RoundTripEvidenceError::Invalid(format!(
            "evidence parent changed during qualification: {error}"
        ))
    })?;
    check_cancelled(control)?;
    let evidence = encode_evidence(&journal, complete, &audit, evaluation, limits)?;
    let validate_inputs = || {
        run_witness.verify()?;
        journal.verify_unchanged().map_err(io::Error::other)?;
        audit.verify().map_err(io::Error::other)?;
        streaming.verify_inputs().map_err(io::Error::other)?;
        evidence_parent_witness.verify()
    };
    let publication = ensure_evidence(
        evidence_target,
        CanonicalOutputLimits {
            max_output_bytes: DEFAULT_MAX_EVIDENCE_BYTES,
            max_staging_bytes: DEFAULT_MAX_EVIDENCE_BYTES,
            max_write_buffer_bytes: DEFAULT_MAX_EVIDENCE_WRITE_BUFFER_BYTES,
            max_working_bytes: DEFAULT_MAX_EVIDENCE_WORKING_BYTES,
        },
        control,
        |writer| writer.write_all(evidence.as_bytes()),
        validate_inputs,
    )?;
    Ok(receipt(journal.run, evaluation, publication))
}

fn check_cancelled(control: &OperationControl) -> Result<(), RoundTripEvidenceError> {
    control
        .check_cancelled()
        .map_err(|_| RoundTripEvidenceError::Comparison(RoundTripFailure::cancelled()))
}

fn validate_audit_bindings(
    journal: &CompleteRunSnapshot,
    bytes: &[u8],
) -> Result<(), RoundTripEvidenceError> {
    let report = journal.report;
    if bytes.len() as u64 != report.byte_length {
        return Err(RoundTripEvidenceError::Invalid(
            "audit.json byte length does not match the ReportEnsured checkpoint".to_owned(),
        ));
    }
    let document: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        RoundTripEvidenceError::Invalid(format!("audit.json is not valid JSON: {error}"))
    })?;
    require_audit_string(&document, "/schema", REPORT_SCHEMA, "schema")?;
    require_audit_string(
        &document,
        "/identities/run",
        &hex_string(&journal.run.into_bytes()),
        "Run Identity",
    )?;
    require_audit_string(
        &document,
        "/request/request_hash",
        &hex_string(&journal.intent.request_hash),
        "request hash",
    )?;
    require_audit_string(
        &document,
        "/identities/source",
        &hex_string(&journal.intent.source),
        "Source Identity",
    )?;
    require_audit_string(
        &document,
        "/identities/workspace",
        &hex_string(&journal.intent.workspace),
        "Workspace Identity",
    )?;
    require_audit_string(
        &document,
        "/identities/baseline_revision",
        &hex_string(&journal.intent.baseline_revision),
        "baseline Revision",
    )?;
    require_audit_string(
        &document,
        "/identities/operation",
        &hex_string(&journal.intent.operation),
        "Operation Identity",
    )?;
    require_audit_string(
        &document,
        "/identities/changed_revision",
        &hex_string(&journal.complete.revision),
        "changed Revision",
    )?;
    require_audit_string(
        &document,
        "/request/ordinal_hash",
        &hex_string(&journal.intent.ordinal_hash),
        "ordinal hash",
    )?;
    require_audit_string(
        &document,
        "/request/recipe_hash",
        &hex_string(&journal.intent.recipe_hash),
        "terrain recipe hash",
    )?;
    require_audit_string(
        &document,
        "/request/qa_input_hash",
        &hex_string(&journal.intent.qa_input_hash),
        "QA input hash",
    )?;
    require_audit_string(
        &document,
        "/request/landxml_options_hash",
        &hex_string(&journal.intent.options_hash),
        "LandXML options hash",
    )?;
    let path_bindings = document
        .pointer("/request/path_bindings")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            RoundTripEvidenceError::Invalid(
                "audit.json path bindings are absent or invalid".to_owned(),
            )
        })?;
    if path_bindings.len() != journal.intent.path_bindings.len()
        || path_bindings
            .iter()
            .zip(journal.intent.path_bindings)
            .any(|(actual, expected)| actual.as_str() != Some(&hex_string(&expected)))
    {
        return Err(RoundTripEvidenceError::Invalid(
            "audit.json path bindings do not match the Complete Run".to_owned(),
        ));
    }
    require_audit_string(
        &document,
        "/edit/audit_hash",
        &hex_string(&journal.complete.audit_hash),
        "Revision Audit hash",
    )?;
    require_audit_string(
        &document,
        "/terrain/changed/artifact_hash",
        &hex_string(&journal.complete.surface_hash),
        "changed terrain artifact hash",
    )?;
    require_audit_string(
        &document,
        "/qa/result_hash",
        &hex_string(&journal.complete.qa_hash),
        "QA result hash",
    )?;
    let export = journal.export;
    require_audit_string(
        &document,
        "/landxml/content_hash",
        &hex_string(&export.content_hash),
        "LandXML content hash",
    )?;
    require_audit_u64(
        &document,
        "/landxml/byte_length",
        export.byte_length,
        "LandXML byte length",
    )?;
    require_audit_false(
        &document,
        "/external_evidence/downstream_round_trip_evaluated",
        "downstream round-trip nonclaim",
    )
}

fn require_audit_string(
    document: &serde_json::Value,
    pointer: &str,
    expected: &str,
    label: &str,
) -> Result<(), RoundTripEvidenceError> {
    if document
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        == Some(expected)
    {
        Ok(())
    } else {
        Err(RoundTripEvidenceError::Invalid(format!(
            "audit.json {label} does not match the Complete Run"
        )))
    }
}

fn require_audit_u64(
    document: &serde_json::Value,
    pointer: &str,
    expected: u64,
    label: &str,
) -> Result<(), RoundTripEvidenceError> {
    if document
        .pointer(pointer)
        .and_then(serde_json::Value::as_u64)
        == Some(expected)
    {
        Ok(())
    } else {
        Err(RoundTripEvidenceError::Invalid(format!(
            "audit.json {label} does not match the Complete Run"
        )))
    }
}

fn require_audit_false(
    document: &serde_json::Value,
    pointer: &str,
    label: &str,
) -> Result<(), RoundTripEvidenceError> {
    if document
        .pointer(pointer)
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        Ok(())
    } else {
        Err(RoundTripEvidenceError::Invalid(format!(
            "audit.json {label} is absent or changed"
        )))
    }
}

fn require_external_target(
    run_root: &Path,
    evidence_target: &Path,
) -> Result<DirectoryWitness, RoundTripEvidenceError> {
    if evidence_target.file_name().is_none() {
        return Err(RoundTripEvidenceError::Invalid(
            "evidence target must name a file".to_owned(),
        ));
    }
    let run_root = run_root.canonicalize().map_err(|error| {
        RoundTripEvidenceError::Invalid(format!("Run root cannot be resolved: {error}"))
    })?;
    let parent = evidence_target.parent().unwrap_or_else(|| Path::new("."));
    let parent_witness = DirectoryWitness::capture(parent).map_err(|error| {
        RoundTripEvidenceError::Invalid(format!("evidence parent cannot be witnessed: {error}"))
    })?;
    let resolved_parent = parent.canonicalize().map_err(|error| {
        RoundTripEvidenceError::Invalid(format!("evidence parent cannot be resolved: {error}"))
    })?;
    if resolved_parent.starts_with(&run_root) {
        return Err(RoundTripEvidenceError::Invalid(
            "evidence target must be outside the Run root".to_owned(),
        ));
    }
    Ok(parent_witness)
}

fn receipt(
    run: WorkflowRunId,
    evaluation: &RoundTripEvaluation,
    publication: CanonicalOutputReceipt,
) -> RoundTripEvidenceReceipt {
    RoundTripEvidenceReceipt {
        run,
        result: if evaluation.is_passed() {
            QualificationResult::Passed
        } else {
            QualificationResult::Failed
        },
        evidence_hash: publication.content_hash,
        evidence_bytes: publication.byte_length,
        failure_reason: evaluation.reason(),
    }
}

fn encode_evidence(
    journal: &CompleteRunSnapshot,
    complete: Complete,
    audit: &CapturedRoundTripFile,
    evaluation: &RoundTripEvaluation,
    limits: RoundTripLimits,
) -> Result<String, RoundTripEvidenceError> {
    let mut json = String::with_capacity(8 * 1024);
    write!(json, "{{\"schema\":")?;
    string(&mut json, EVIDENCE_SCHEMA)?;
    write!(json, ",\"result\":")?;
    string(
        &mut json,
        if evaluation.is_passed() {
            "passed"
        } else {
            "failed"
        },
    )?;
    write!(json, ",\"run\":{{\"run_id\":")?;
    hex(&mut json, &journal.run.into_bytes())?;
    write!(json, ",\"request_hash\":")?;
    hex(&mut json, &journal.intent.request_hash)?;
    write!(json, ",\"complete_journal_hash\":")?;
    hex(&mut json, &journal.journal_hash)?;
    write!(
        json,
        ",\"complete_journal_bytes\":{}",
        journal.journal_bytes
    )?;
    write!(json, ",\"original_landxml_hash\":")?;
    hex(&mut json, &complete.landxml_hash)?;
    write!(
        json,
        ",\"original_landxml_bytes\":{}",
        evaluation.reference_bytes()
    )?;
    write!(json, ",\"audit_json_hash\":")?;
    hex(&mut json, &complete.report_hash)?;
    write!(json, ",\"audit_json_bytes\":{}}}", audit.bytes.len())?;
    write!(json, ",\"downstream_declaration\":{{\"application\":")?;
    let declaration = evaluation.declaration();
    string(&mut json, declaration.declared_application())?;
    write!(json, ",\"version\":")?;
    string(&mut json, declaration.declared_version())?;
    write!(json, ",\"settings_profile\":")?;
    string(&mut json, declaration.declared_settings_profile())?;
    let tolerances = evaluation.tolerances();
    write!(
        json,
        "}},\"comparison_policy\":{{\"horizontal_tolerance_metres\":"
    )?;
    number(&mut json, tolerances.horizontal_metres())?;
    write!(json, ",\"vertical_tolerance_metres\":")?;
    number(&mut json, tolerances.vertical_metres())?;
    write!(json, ",\"matcher_version\":")?;
    string(&mut json, MATCHER_VERSION)?;
    write!(json, "}},\"returned_landxml\":{{\"content_hash\":")?;
    hex(&mut json, &evaluation.returned_content_hash())?;
    write!(
        json,
        ",\"bytes\":{},\"namespace\":",
        evaluation.returned_bytes()
    )?;
    if evaluation.returned_was_parsed() {
        string(&mut json, "http://www.landxml.org/schema/LandXML-1.2")?;
    } else {
        json.push_str("null");
    }
    write!(json, ",\"declared_units\":")?;
    if evaluation.returned_was_parsed() {
        string(&mut json, "meter")?;
    } else {
        json.push_str("null");
    }
    write!(json, ",\"surface_name\":")?;
    match evaluation.returned_surface_name() {
        Some(name) => string(&mut json, name)?,
        None => json.push_str("null"),
    }
    write!(json, ",\"point_count\":")?;
    optional_count(&mut json, evaluation.returned_point_count())?;
    write!(json, ",\"face_count\":")?;
    optional_count(&mut json, evaluation.returned_face_count())?;
    write!(json, ",\"ignored_top_level_section_names\":[")?;
    for (index, section) in evaluation.returned_ignored_sections().iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        string(&mut json, section)?;
    }
    write!(json, "]}}")?;
    write_checks(&mut json, evaluation)?;
    write_comparison(&mut json, evaluation)?;
    write!(
        json,
        ",\"limits\":{{\"xml_input_bytes\":{},\"xml_nodes\":{},\"xml_text_attribute_bytes\":{},\"points\":{},\"faces\":{},\"candidate_vertex_comparisons\":{},\"application_label_bytes\":128,\"version_label_bytes\":128,\"settings_profile_bytes\":1024,\"evidence_output_bytes\":{DEFAULT_MAX_EVIDENCE_BYTES},\"evidence_staging_bytes\":{DEFAULT_MAX_EVIDENCE_BYTES},\"evidence_write_buffer_bytes\":{DEFAULT_MAX_EVIDENCE_WRITE_BUFFER_BYTES},\"evidence_working_bytes\":{DEFAULT_MAX_EVIDENCE_WORKING_BYTES}}}",
        limits.file_bytes(),
        limits.xml_nodes(),
        limits.xml_text_bytes(),
        limits.points(),
        limits.faces(),
        limits.comparisons()
    )?;
    writeln!(
        json,
        ",\"nonclaims\":{{\"punctra_observed_downstream_execution\":false,\"vendor_certification\":false,\"firm_acceptance\":false,\"paid_use\":false,\"conversion\":false,\"measured_labor_savings\":false}}}}"
    )?;
    if json.len() as u64 > DEFAULT_MAX_EVIDENCE_BYTES {
        return Err(RoundTripEvidenceError::Invalid(
            "canonical evidence exceeds its output limit".to_owned(),
        ));
    }
    Ok(json)
}

fn write_checks(json: &mut String, evaluation: &RoundTripEvaluation) -> Result<(), fmt::Error> {
    let mapping_completed = match evaluation {
        RoundTripEvaluation::Passed(_) => true,
        RoundTripEvaluation::Failed(mismatch) => mismatch.completed_mapping_point_count().is_some(),
    };
    let [parse, units, unique_mapping, tolerance, topology] =
        check_statuses(evaluation.reason(), mapping_completed);
    write!(
        json,
        ",\"checks\":{{\"provenance\":{{\"status\":\"passed\"}},\"parse\":{{\"status\":"
    )?;
    write_check_status(json, parse)?;
    write!(json, "}},\"units\":{{\"status\":")?;
    write_check_status(json, units)?;
    write!(json, "}},\"unique_mapping\":{{\"status\":")?;
    write_check_status(json, unique_mapping)?;
    write!(json, "}},\"tolerance\":{{\"status\":")?;
    write_check_status(json, tolerance)?;
    write!(json, "}},\"topology\":{{\"status\":")?;
    write_check_status(json, topology)?;
    write!(json, "}}}}")
}

fn check_statuses(
    reason: Option<crate::roundtrip::RoundTripReason>,
    mapping_completed: bool,
) -> [CheckStatus; 5] {
    use crate::roundtrip::RoundTripReason as Reason;

    use CheckStatus::{Failed, NotEvaluated, Passed};

    match reason {
        None => [Passed; 5],
        Some(
            reason @ (Reason::XmlInvalid
            | Reason::SubsetUnsupported
            | Reason::CoordinateReferenceUnsupported),
        ) => [
            Failed(reason),
            NotEvaluated,
            NotEvaluated,
            NotEvaluated,
            NotEvaluated,
        ],
        Some(reason @ Reason::UnitDrift) => [
            Passed,
            Failed(reason),
            NotEvaluated,
            NotEvaluated,
            NotEvaluated,
        ],
        Some(reason @ (Reason::PointCountDrift | Reason::VertexAmbiguous)) => {
            [Passed, Passed, Failed(reason), NotEvaluated, NotEvaluated]
        }
        Some(reason @ (Reason::VertexUnmatched | Reason::ToleranceDrift)) => {
            [Passed, Passed, Failed(reason), Failed(reason), NotEvaluated]
        }
        Some(reason @ Reason::TopologyDrift) if mapping_completed => {
            [Passed, Passed, Passed, Passed, Failed(reason)]
        }
        Some(reason @ Reason::TopologyDrift) => {
            [Passed, Passed, NotEvaluated, NotEvaluated, Failed(reason)]
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckStatus {
    Passed,
    Failed(crate::roundtrip::RoundTripReason),
    NotEvaluated,
}

fn write_check_status(json: &mut String, status: CheckStatus) -> Result<(), fmt::Error> {
    match status {
        CheckStatus::Passed => string(json, "passed"),
        CheckStatus::Failed(reason) => {
            write!(json, "\"failed\",\"reason_code\":")?;
            string(json, reason.as_str())
        }
        CheckStatus::NotEvaluated => string(json, "not_evaluated"),
    }
}

fn write_comparison(json: &mut String, evaluation: &RoundTripEvaluation) -> Result<(), fmt::Error> {
    write!(json, ",\"comparison\":")?;
    match evaluation {
        RoundTripEvaluation::Passed(report) => {
            write!(
                json,
                "{{\"mapped_point_count\":{},\"unmatched_point_count\":0,\"ambiguous_point_count\":0,\"maximum_horizontal_delta_metres\":",
                report.vertex_count()
            )?;
            number(json, report.max_horizontal_drift_metres())?;
            write!(json, ",\"maximum_vertical_delta_metres\":")?;
            number(json, report.max_vertical_drift_metres())?;
            write!(
                json,
                ",\"added_face_count\":0,\"removed_face_count\":0,\"added_face_hash\":null,\"removed_face_hash\":null,\"added_face_sample\":[],\"removed_face_sample\":[]}}"
            )
        }
        RoundTripEvaluation::Failed(mismatch) => {
            let mapping_counts = mismatch.mapping_counts();
            write!(json, "{{\"mapped_point_count\":")?;
            optional_count(json, mapping_counts.map(|(mapped, _, _)| mapped))?;
            write!(json, ",\"unmatched_point_count\":")?;
            optional_count(json, mapping_counts.map(|(_, unmatched, _)| unmatched))?;
            write!(json, ",\"ambiguous_point_count\":")?;
            optional_count(json, mapping_counts.map(|(_, _, ambiguous)| ambiguous))?;
            write!(json, ",\"maximum_horizontal_delta_metres\":")?;
            if let Some((horizontal, _)) = mismatch.mapping_maximum_deltas() {
                number(json, horizontal)?;
            } else {
                json.push_str("null");
            }
            write!(json, ",\"maximum_vertical_delta_metres\":")?;
            if let Some((_, vertical)) = mismatch.mapping_maximum_deltas() {
                number(json, vertical)?;
            } else {
                json.push_str("null");
            }
            write!(json, ",\"added_face_count\":")?;
            if let Some(topology) = mismatch.topology() {
                write!(json, "{}", topology.added_count())?;
            } else {
                json.push_str("null");
            }
            write!(json, ",\"removed_face_count\":")?;
            if let Some(topology) = mismatch.topology() {
                write!(json, "{}", topology.removed_count())?;
            } else {
                json.push_str("null");
            }
            write!(json, ",\"added_face_hash\":")?;
            if let Some(topology) = mismatch.topology() {
                hex(json, &topology.added_hash())?;
            } else {
                json.push_str("null");
            }
            write!(json, ",\"removed_face_hash\":")?;
            if let Some(topology) = mismatch.topology() {
                hex(json, &topology.removed_hash())?;
            } else {
                json.push_str("null");
            }
            write!(json, ",\"added_face_sample\":")?;
            write_face_sample(
                json,
                mismatch
                    .topology()
                    .map(crate::roundtrip::TopologyDrift::added_sample),
            )?;
            write!(json, ",\"removed_face_sample\":")?;
            write_face_sample(
                json,
                mismatch
                    .topology()
                    .map(crate::roundtrip::TopologyDrift::removed_sample),
            )?;
            write!(json, ",\"diagnostic\":")?;
            string(json, mismatch.diagnostic())?;
            write!(json, "}}")
        }
    }
}

fn write_face_sample(json: &mut String, sample: Option<&[[usize; 3]]>) -> Result<(), fmt::Error> {
    json.push('[');
    for (index, face) in sample.unwrap_or_default().iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(json, "[{},{},{}]", face[0], face[1], face[2])?;
    }
    json.push(']');
    Ok(())
}

fn optional_count(json: &mut String, count: Option<u64>) -> Result<(), fmt::Error> {
    match count {
        Some(count) => write!(json, "{count}"),
        None => json.write_str("null"),
    }
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn number(output: &mut String, value: f64) -> Result<(), fmt::Error> {
    let fixed = format!("{value:.17}");
    if fixed
        .parse::<f64>()
        .is_ok_and(|parsed| parsed.to_bits() == value.to_bits())
    {
        output.push_str(&fixed);
        return Ok(());
    }

    let exact = serde_json::Number::from_f64(value).ok_or(fmt::Error)?;
    write!(output, "{exact}")
}

fn hex(output: &mut String, bytes: &[u8]) -> Result<(), fmt::Error> {
    output.push('"');
    for byte in bytes {
        write!(output, "{byte:02x}")?;
    }
    output.push('"');
    Ok(())
}

fn hex_string(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn string(output: &mut String, value: &str) -> Result<(), fmt::Error> {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value < '\u{20}' => write!(output, "\\u{:04x}", value as u32)?,
            value => output.push(value),
        }
    }
    output.push('"');
    Ok(())
}

impl From<fmt::Error> for RoundTripEvidenceError {
    fn from(_error: fmt::Error) -> Self {
        Self::Invalid("canonical evidence encoding failed".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    use foundation_runtime::OperationControl;

    use crate::roundtrip::{
        RoundTripDeclaration, RoundTripFailureKind, RoundTripReason, RoundTripTolerances,
    };

    use super::{
        CheckStatus, RoundTripEvidenceError, check_statuses, number, verify_round_trip_with_control,
    };

    static NEXT_TARGET: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn canonical_numbers_preserve_exact_f64_values() {
        for value in [1.0e-18, f64::MIN_POSITIVE, f64::from_bits(1)] {
            let mut encoded = String::new();
            number(&mut encoded, value).expect("writing to a String must succeed");
            let decoded: f64 =
                serde_json::from_str(&encoded).expect("canonical number must be valid JSON");

            assert_eq!(decoded.to_bits(), value.to_bits(), "encoded as {encoded}");
        }
    }

    #[test]
    fn canonical_numbers_retain_existing_exact_fixed_point_bytes() {
        let mut encoded = String::new();
        number(&mut encoded, 0.001).expect("writing to a String must succeed");

        assert_eq!(encoded, "0.00100000000000000");
    }

    #[test]
    fn ambiguous_mapping_leaves_dependent_checks_not_evaluated() {
        let [parse, units, mapping, tolerance, topology] =
            check_statuses(Some(RoundTripReason::VertexAmbiguous), false);

        assert_eq!(parse, CheckStatus::Passed);
        assert_eq!(units, CheckStatus::Passed);
        assert_eq!(
            mapping,
            CheckStatus::Failed(RoundTripReason::VertexAmbiguous)
        );
        assert_eq!(tolerance, CheckStatus::NotEvaluated);
        assert_eq!(topology, CheckStatus::NotEvaluated);
    }

    #[test]
    fn topology_rejection_before_mapping_keeps_unrun_checks_not_evaluated() {
        let [parse, units, mapping, tolerance, topology] =
            check_statuses(Some(RoundTripReason::TopologyDrift), false);

        assert_eq!(parse, CheckStatus::Passed);
        assert_eq!(units, CheckStatus::Passed);
        assert_eq!(mapping, CheckStatus::NotEvaluated);
        assert_eq!(tolerance, CheckStatus::NotEvaluated);
        assert_eq!(
            topology,
            CheckStatus::Failed(RoundTripReason::TopologyDrift)
        );
    }

    #[test]
    fn topology_comparison_after_mapping_preserves_completed_checks() {
        let [parse, units, mapping, tolerance, topology] =
            check_statuses(Some(RoundTripReason::TopologyDrift), true);

        assert_eq!(parse, CheckStatus::Passed);
        assert_eq!(units, CheckStatus::Passed);
        assert_eq!(mapping, CheckStatus::Passed);
        assert_eq!(tolerance, CheckStatus::Passed);
        assert_eq!(
            topology,
            CheckStatus::Failed(RoundTripReason::TopologyDrift)
        );
    }

    #[test]
    fn cancelled_preflight_publishes_no_evidence() {
        let control = OperationControl::new();
        control.cancel();
        let target = std::env::temp_dir().join(format!(
            "punctra-cancelled-round-trip-{}-{}.json",
            std::process::id(),
            NEXT_TARGET.fetch_add(1, Ordering::Relaxed)
        ));
        let error = verify_round_trip_with_control(
            Path::new("cancelled-round-trip-run-must-not-exist"),
            Path::new("cancelled-returned-must-not-exist.xml"),
            &target,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            &control,
        )
        .expect_err("pre-cancelled qualification must stop before input access");

        let RoundTripEvidenceError::Comparison(error) = error else {
            panic!("cancellation must retain its operational failure class");
        };
        assert_eq!(error.kind(), RoundTripFailureKind::Cancelled);
        assert!(!target.exists());
    }
}
