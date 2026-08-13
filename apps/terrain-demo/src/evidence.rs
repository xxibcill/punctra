//! Canonical Run-bound `LandXML` round-trip evidence and exact publication.

use std::{
    fs::{self, File},
    io::{self, Read as _, Write as _},
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

use crate::{
    publication::{
        DirectoryWitness, StageCreationError, StageGuard, create_stage, same_file_identity,
        sync_directory,
    },
    qualification::CompleteRunQualificationSnapshot,
    roundtrip::{
        ADDED_FACE_HASH_DOMAIN, MATCHER_VERSION, REMOVED_FACE_HASH_DOMAIN, RoundTripEvaluation,
        RoundTripFailedReport, RoundTripLimits, RoundTripReport, hash_face_difference,
    },
};

const EVIDENCE_SCHEMA: &str = "punctra.terrain-demo.landxml-round-trip-evidence.v1";
const MAX_EVIDENCE_BYTES: u64 = 1024 * 1024;
const MAX_PUBLICATION_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationBoundary {
    BeforeLink,
    TargetVerification,
    ParentSync,
    StageRemoval,
    CleanupSync,
    TerminalAcknowledgement,
}

trait PublicationHook {
    fn reach(&self, boundary: PublicationBoundary) -> io::Result<()>;
}

struct ProductionPublicationHook;

impl PublicationHook for ProductionPublicationHook {
    fn reach(&self, _boundary: PublicationBoundary) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceReceipt {
    pub(crate) content_hash: [u8; 32],
    pub(crate) byte_length: u64,
    pub(crate) result: EvidenceResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceResult {
    Passed,
    Failed,
}

impl EvidenceResult {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum EvidenceError {
    #[error("evidence target must name a file outside the qualified Run root")]
    InvalidTarget,
    #[error("evidence bytes exceed the {0} byte publication limit")]
    Resource(u64),
    #[error("evidence target conflicts with canonical bytes: {0}")]
    Conflict(PathBuf),
    #[error("round-trip reference differs from the qualified Run LandXML fact")]
    RunBindingConflict,
    #[error("evidence publication is indeterminate for {path}: {source}")]
    Indeterminate {
        path: PathBuf,
        expected_hash: [u8; 32],
        #[source]
        source: io::Error,
    },
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to encode canonical evidence: {0}")]
    Encode(#[from] serde_json::Error),
}

impl EvidenceError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

pub(crate) fn publish_evidence(
    target: &Path,
    run_root: &Path,
    run: &CompleteRunQualificationSnapshot,
    evaluation: &RoundTripEvaluation,
    limits: RoundTripLimits,
) -> Result<EvidenceReceipt, EvidenceError> {
    let target = BoundEvidenceTarget::bind(target, run_root)?;
    let (bytes, result) = encode_evidence(run, evaluation, limits)?;
    let byte_length = bytes.len() as u64;
    if byte_length > MAX_EVIDENCE_BYTES {
        return Err(EvidenceError::Resource(byte_length));
    }
    let content_hash = evidence_hash(&bytes);
    publish_bound_or_reconcile(&target, &bytes, content_hash, &ProductionPublicationHook)?;
    Ok(EvidenceReceipt {
        content_hash,
        byte_length,
        result,
    })
}

struct BoundEvidenceTarget {
    requested_parent: PathBuf,
    requested_run_root: PathBuf,
    canonical_parent: PathBuf,
    canonical_run_root: PathBuf,
    target: PathBuf,
    parent_witness: DirectoryWitness,
    run_witness: DirectoryWitness,
}

impl BoundEvidenceTarget {
    fn bind(target: &Path, run_root: &Path) -> Result<Self, EvidenceError> {
        let Some(Component::Normal(file_name)) = target.components().next_back() else {
            return Err(EvidenceError::InvalidTarget);
        };
        let requested_parent = target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let canonical_parent = fs::canonicalize(&requested_parent).map_err(|source| {
            EvidenceError::io("resolve evidence parent", &requested_parent, source)
        })?;
        let canonical_run_root = fs::canonicalize(run_root)
            .map_err(|source| EvidenceError::io("resolve qualified Run root", run_root, source))?;
        if canonical_parent == canonical_run_root
            || canonical_parent.starts_with(&canonical_run_root)
        {
            return Err(EvidenceError::InvalidTarget);
        }
        let parent_witness = DirectoryWitness::capture(&canonical_parent).map_err(|source| {
            EvidenceError::io(
                "witness resolved evidence parent",
                &canonical_parent,
                source,
            )
        })?;
        let run_witness = DirectoryWitness::capture(&canonical_run_root).map_err(|source| {
            EvidenceError::io(
                "witness resolved qualified Run root",
                &canonical_run_root,
                source,
            )
        })?;
        let bound = Self {
            requested_parent,
            requested_run_root: run_root.to_path_buf(),
            target: canonical_parent.join(file_name),
            canonical_parent,
            canonical_run_root,
            parent_witness,
            run_witness,
        };
        bound
            .verify()
            .map_err(|source| EvidenceError::io("bind evidence target", target, source))?;
        Ok(bound)
    }

    fn verify(&self) -> io::Result<()> {
        self.parent_witness.verify()?;
        self.run_witness.verify()?;
        let requested_parent = fs::canonicalize(&self.requested_parent)?;
        let run_root = fs::canonicalize(&self.requested_run_root)?;
        if requested_parent == self.canonical_parent
            && run_root == self.canonical_run_root
            && self.canonical_parent != self.canonical_run_root
            && !self.canonical_parent.starts_with(&self.canonical_run_root)
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "evidence parent binding or outside-Run containment changed",
            ))
        }
    }

    fn target(&self) -> &Path {
        &self.target
    }

    fn parent(&self) -> &Path {
        &self.canonical_parent
    }
}

fn encode_evidence(
    run: &CompleteRunQualificationSnapshot,
    evaluation: &RoundTripEvaluation,
    limits: RoundTripLimits,
) -> Result<(Vec<u8>, EvidenceResult), EvidenceError> {
    encode_evidence_with_limit(run, evaluation, limits, MAX_EVIDENCE_BYTES)
}

fn encode_evidence_with_limit(
    run: &CompleteRunQualificationSnapshot,
    evaluation: &RoundTripEvaluation,
    limits: RoundTripLimits,
    max_bytes: u64,
) -> Result<(Vec<u8>, EvidenceResult), EvidenceError> {
    let (
        result,
        tolerances,
        reference_hash,
        reference_bytes,
        reference_parser_peak_bytes,
        returned_parser_peak_bytes,
        retained_peak_bytes,
    ) = match evaluation {
        RoundTripEvaluation::Passed(report) => (
            EvidenceResult::Passed,
            report.tolerances(),
            report.reference_content_hash(),
            report.reference_bytes(),
            report.reference_parser_peak_bytes(),
            report.returned_parser_peak_bytes(),
            report.retained_peak_bytes(),
        ),
        RoundTripEvaluation::Failed(report) => (
            EvidenceResult::Failed,
            report.tolerances(),
            report.reference_content_hash(),
            report.reference_bytes(),
            report.reference_parser_peak_bytes(),
            report.returned_parser_peak_bytes(),
            report.retained_peak_bytes(),
        ),
    };
    if reference_hash != run.landxml_hash || reference_bytes != run.landxml_bytes {
        return Err(EvidenceError::RunBindingConflict);
    }
    let mut writer = CanonicalEvidenceWriter::new(max_bytes)?;
    writer.raw(b"{\"schema\":")?;
    writer.string(EVIDENCE_SCHEMA)?;
    writer.raw(b",\"result\":")?;
    writer.string(result.as_str())?;
    writer.raw(b",\"run\":")?;
    write_run(&mut writer, run)?;
    writer.raw(b",\"downstream_declaration\":")?;
    write_declaration(&mut writer, evaluation)?;
    writer.raw(b",\"comparison_policy\":{")?;
    writer.raw(b"\"horizontal_tolerance_metres\":")?;
    writer.float_string(tolerances.horizontal_metres())?;
    writer.raw(b",\"matcher_version\":")?;
    writer.string(MATCHER_VERSION)?;
    writer.raw(b",\"vertical_tolerance_metres\":")?;
    writer.float_string(tolerances.vertical_metres())?;
    writer.raw(b"},\"returned_landxml\":")?;
    write_returned(&mut writer, evaluation)?;
    writer.raw(b",\"checks\":")?;
    write_checks(&mut writer, evaluation)?;
    writer.raw(b",\"comparison\":")?;
    write_comparison(&mut writer, evaluation)?;
    writer.raw(b",\"limits\":")?;
    write_limits(
        &mut writer,
        limits,
        reference_parser_peak_bytes,
        returned_parser_peak_bytes,
        retained_peak_bytes,
    )?;
    writer.raw(b",\"nonclaims\":{\"conversion\":false,\"firm_acceptance\":false,\"measured_labor_savings\":false,\"paid_use\":false,\"punctra_observed_downstream_execution\":false,\"vendor_certification\":false}}\n")?;
    Ok((writer.finish(), result))
}

struct CanonicalEvidenceWriter {
    bytes: Vec<u8>,
    max_bytes: u64,
    resource_required: Option<u64>,
}

impl CanonicalEvidenceWriter {
    fn new(max_bytes: u64) -> Result<Self, EvidenceError> {
        let capacity =
            usize::try_from(max_bytes).map_err(|_| EvidenceError::Resource(max_bytes))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| EvidenceError::Resource(max_bytes))?;
        let actual_capacity = bytes.capacity() as u64;
        if actual_capacity > max_bytes {
            return Err(EvidenceError::Resource(actual_capacity));
        }
        Ok(Self {
            bytes,
            max_bytes,
            resource_required: None,
        })
    }

    fn raw(&mut self, bytes: &[u8]) -> Result<(), EvidenceError> {
        self.write_all(bytes).map_err(|source| self.map_io(source))
    }

    fn string(&mut self, value: &str) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, value);
        self.map_json(result)
    }

    fn optional_string(&mut self, value: Option<&str>) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, &value);
        self.map_json(result)
    }

    fn strings(&mut self, value: &[String]) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, value);
        self.map_json(result)
    }

    fn optional_strings(&mut self, value: Option<&[String]>) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, &value);
        self.map_json(result)
    }

    fn optional_faces(&mut self, value: Option<&[[u64; 3]]>) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, &value);
        self.map_json(result)
    }

    fn u64(&mut self, value: u64) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, &value);
        self.map_json(result)
    }

    fn optional_u64(&mut self, value: Option<u64>) -> Result<(), EvidenceError> {
        let result = serde_json::to_writer(&mut *self, &value);
        self.map_json(result)
    }

    fn float_string(&mut self, value: f64) -> Result<(), EvidenceError> {
        self.raw(b"\"")?;
        write!(self, "{value:.17}").map_err(|source| self.map_io(source))?;
        self.raw(b"\"")
    }

    fn optional_float_string(&mut self, value: Option<f64>) -> Result<(), EvidenceError> {
        if let Some(value) = value {
            self.float_string(value)
        } else {
            self.raw(b"null")
        }
    }

    fn hex(&mut self, bytes: &[u8]) -> Result<(), EvidenceError> {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        self.raw(b"\"")?;
        for byte in bytes {
            self.raw(&[
                DIGITS[usize::from(byte >> 4)],
                DIGITS[usize::from(byte & 0x0f)],
            ])?;
        }
        self.raw(b"\"")
    }

    fn optional_hex(&mut self, value: Option<&[u8]>) -> Result<(), EvidenceError> {
        if let Some(value) = value {
            self.hex(value)
        } else {
            self.raw(b"null")
        }
    }

    fn map_json(&mut self, result: Result<(), serde_json::Error>) -> Result<(), EvidenceError> {
        result.map_err(|source| {
            if let Some(required) = self.resource_required {
                EvidenceError::Resource(required)
            } else {
                EvidenceError::Encode(source)
            }
        })
    }

    fn map_io(&self, source: io::Error) -> EvidenceError {
        self.resource_required.map_or_else(
            || EvidenceError::Encode(serde_json::Error::io(source)),
            EvidenceError::Resource,
        )
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

impl io::Write for CanonicalEvidenceWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let required = self
            .bytes
            .len()
            .checked_add(input.len())
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX);
        if required > self.max_bytes {
            self.resource_required = Some(required);
            return Err(io::Error::other("canonical evidence byte limit exceeded"));
        }
        debug_assert!(required <= self.bytes.capacity() as u64);
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn write_run(
    writer: &mut CanonicalEvidenceWriter,
    run: &CompleteRunQualificationSnapshot,
) -> Result<(), EvidenceError> {
    writer.raw(b"{\"audit_json\":{\"bytes\":")?;
    writer.u64(run.report_bytes)?;
    writer.raw(b",\"content_hash\":")?;
    writer.hex(&run.report_hash)?;
    writer.raw(b"},\"baseline_revision\":")?;
    writer.hex(&run.baseline_revision)?;
    writer.raw(b",\"complete_journal_hash\":")?;
    writer.hex(&run.terminal_journal_hash)?;
    writer.raw(b",\"journal_bytes\":")?;
    writer.u64(run.journal_bytes)?;
    writer.raw(b",\"operation\":")?;
    writer.hex(&run.operation)?;
    writer.raw(b",\"request_hash\":")?;
    writer.hex(&run.request_hash)?;
    writer.raw(b",\"revision\":")?;
    writer.hex(&run.revision)?;
    writer.raw(b",\"run_identity\":")?;
    writer.hex(run.run.as_bytes())?;
    writer.raw(b",\"source\":")?;
    writer.hex(&run.source)?;
    writer.raw(b",\"terrain_landxml\":{\"bytes\":")?;
    writer.u64(run.landxml_bytes)?;
    writer.raw(b",\"content_hash\":")?;
    writer.hex(&run.landxml_hash)?;
    writer.raw(b"},\"workspace\":")?;
    writer.hex(&run.workspace)?;
    writer.raw(b"}")
}

fn write_declaration(
    writer: &mut CanonicalEvidenceWriter,
    evaluation: &RoundTripEvaluation,
) -> Result<(), EvidenceError> {
    let (application, settings_profile, version) = match evaluation {
        RoundTripEvaluation::Passed(report) => (
            report.declared_application(),
            report.declared_settings_profile(),
            report.declared_version(),
        ),
        RoundTripEvaluation::Failed(report) => {
            let declaration = report.declaration();
            (
                declaration.declared_application(),
                declaration.declared_settings_profile(),
                declaration.declared_version(),
            )
        }
    };
    writer.raw(b"{\"application\":")?;
    writer.string(application)?;
    writer.raw(b",\"settings_profile\":")?;
    writer.string(settings_profile)?;
    writer.raw(b",\"version\":")?;
    writer.string(version)?;
    writer.raw(b"}")
}

fn write_returned(
    writer: &mut CanonicalEvidenceWriter,
    evaluation: &RoundTripEvaluation,
) -> Result<(), EvidenceError> {
    writer.raw(b"{\"bytes\":")?;
    match evaluation {
        RoundTripEvaluation::Passed(report) => {
            writer.u64(report.returned_bytes())?;
            writer.raw(b",\"content_hash\":")?;
            writer.hex(&report.returned_content_hash())?;
            writer.raw(b",\"declared_units\":\"metric_metres\",\"face_count\":")?;
            writer.u64(report.face_count())?;
            writer.raw(b",\"ignored_top_level_sections\":")?;
            writer.strings(report.returned_ignored_top_level_sections())?;
            writer.raw(
                b",\"namespace\":\"http://www.landxml.org/schema/LandXML-1.2\",\"point_count\":",
            )?;
            writer.u64(report.vertex_count())?;
            writer.raw(b",\"surface_name\":")?;
            writer.string(report.returned_surface_name())?;
        }
        RoundTripEvaluation::Failed(report) => {
            writer.u64(report.returned_bytes())?;
            writer.raw(b",\"content_hash\":")?;
            writer.hex(&report.returned_content_hash())?;
            writer.raw(b",\"declared_units\":")?;
            writer.optional_string(report.returned_surface_name().map(|_| "metric_metres"))?;
            writer.raw(b",\"face_count\":")?;
            writer.optional_u64(report.returned_face_count())?;
            writer.raw(b",\"ignored_top_level_sections\":")?;
            writer.optional_strings(report.returned_ignored_top_level_sections())?;
            writer.raw(b",\"namespace\":")?;
            writer.optional_string(
                report
                    .returned_surface_name()
                    .map(|_| "http://www.landxml.org/schema/LandXML-1.2"),
            )?;
            writer.raw(b",\"point_count\":")?;
            writer.optional_u64(report.returned_point_count())?;
            writer.raw(b",\"surface_name\":")?;
            writer.optional_string(report.returned_surface_name())?;
        }
    }
    writer.raw(b"}")
}

fn write_checks(
    writer: &mut CanonicalEvidenceWriter,
    evaluation: &RoundTripEvaluation,
) -> Result<(), EvidenceError> {
    let RoundTripEvaluation::Failed(report) = evaluation else {
        return writer.raw(b"{\"parse\":{\"status\":\"passed\"},\"provenance\":{\"status\":\"passed\"},\"tolerance\":{\"status\":\"passed\"},\"topology\":{\"status\":\"passed\"},\"unique_mapping\":{\"status\":\"passed\"},\"units\":{\"status\":\"passed\"}}");
    };
    let reason = report
        .failure()
        .reason()
        .expect("failed evidence requires a semantic reason")
        .as_str();
    let failed = match reason {
        "PRT_UNIT_DRIFT" => "units",
        "PRT_POINT_COUNT_DRIFT" | "PRT_VERTEX_UNMATCHED" | "PRT_VERTEX_AMBIGUOUS" => {
            "unique_mapping"
        }
        "PRT_TOLERANCE_DRIFT" => "tolerance",
        "PRT_TOPOLOGY_DRIFT" => "topology",
        _ => "parse",
    };
    writer.raw(b"{")?;
    for (index, check) in [
        "parse",
        "provenance",
        "tolerance",
        "topology",
        "unique_mapping",
        "units",
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            writer.raw(b",")?;
        }
        writer.string(check)?;
        writer.raw(b":")?;
        write_check_status(writer, check, failed, reason)?;
    }
    writer.raw(b"}")
}

fn write_check_status(
    writer: &mut CanonicalEvidenceWriter,
    check: &str,
    failed: &str,
    reason: &str,
) -> Result<(), EvidenceError> {
    let order = [
        "provenance",
        "parse",
        "units",
        "unique_mapping",
        "tolerance",
        "topology",
    ];
    let check_index = order
        .iter()
        .position(|value| *value == check)
        .expect("known check");
    let failed_index = order
        .iter()
        .position(|value| *value == failed)
        .expect("known failed check");
    match check_index.cmp(&failed_index) {
        std::cmp::Ordering::Less => writer.raw(b"{\"status\":\"passed\"}"),
        std::cmp::Ordering::Equal => {
            writer.raw(b"{\"reason\":")?;
            writer.string(reason)?;
            writer.raw(b",\"status\":\"failed\"}")
        }
        std::cmp::Ordering::Greater => writer.raw(b"{\"status\":\"not_evaluated\"}"),
    }
}

fn write_comparison(
    writer: &mut CanonicalEvidenceWriter,
    evaluation: &RoundTripEvaluation,
) -> Result<(), EvidenceError> {
    match evaluation {
        RoundTripEvaluation::Passed(report) => write_passed_comparison(writer, report),
        RoundTripEvaluation::Failed(report) => write_failed_comparison(writer, report),
    }
}

fn write_passed_comparison(
    writer: &mut CanonicalEvidenceWriter,
    report: &RoundTripReport,
) -> Result<(), EvidenceError> {
    let empty_added = hash_face_difference(ADDED_FACE_HASH_DOMAIN, &[]);
    let empty_removed = hash_face_difference(REMOVED_FACE_HASH_DOMAIN, &[]);
    writer.raw(b"{\"added_face_count\":0,\"added_face_hash\":")?;
    writer.hex(&empty_added)?;
    writer
        .raw(b",\"added_face_sample\":[],\"ambiguous_point_count\":0,\"candidate_comparisons\":")?;
    writer.u64(report.comparison_count())?;
    writer.raw(b",\"mapped_point_count\":")?;
    writer.u64(report.vertex_count())?;
    writer.raw(b",\"maximum_easting_delta_metres\":")?;
    writer.float_string(report.max_easting_drift_metres())?;
    writer.raw(b",\"maximum_horizontal_delta_metres\":")?;
    writer.float_string(report.max_horizontal_drift_metres())?;
    writer.raw(b",\"maximum_northing_delta_metres\":")?;
    writer.float_string(report.max_northing_drift_metres())?;
    writer.raw(b",\"maximum_vertical_delta_metres\":")?;
    writer.float_string(report.max_vertical_drift_metres())?;
    writer.raw(b",\"removed_face_count\":0,\"removed_face_hash\":")?;
    writer.hex(&empty_removed)?;
    writer.raw(b",\"removed_face_sample\":[],\"unmatched_point_count\":0}")
}

fn write_failed_comparison(
    writer: &mut CanonicalEvidenceWriter,
    report: &RoundTripFailedReport,
) -> Result<(), EvidenceError> {
    let difference = report.failure().topology_difference();
    let comparison_count = report.comparison_count();
    let completed_mapping_count = comparison_count.map(|_| 0_u64);
    writer.raw(b"{\"added_face_count\":")?;
    writer.optional_u64(difference.map(|value| value.added_count))?;
    writer.raw(b",\"added_face_hash\":")?;
    writer.optional_hex(difference.map(|value| value.added_hash.as_slice()))?;
    writer.raw(b",\"added_face_sample\":")?;
    writer.optional_faces(difference.map(|value| value.added_sample.as_ref()))?;
    writer.raw(b",\"ambiguous_point_count\":")?;
    writer.optional_u64(completed_mapping_count)?;
    writer.raw(b",\"candidate_comparisons\":")?;
    writer.optional_u64(comparison_count)?;
    writer.raw(b",\"mapped_point_count\":")?;
    writer.optional_u64(comparison_count.and(report.returned_point_count()))?;
    writer.raw(b",\"maximum_easting_delta_metres\":")?;
    writer.optional_float_string(report.max_easting_drift_metres())?;
    writer.raw(b",\"maximum_horizontal_delta_metres\":")?;
    writer.optional_float_string(report.max_horizontal_drift_metres())?;
    writer.raw(b",\"maximum_northing_delta_metres\":")?;
    writer.optional_float_string(report.max_northing_drift_metres())?;
    writer.raw(b",\"maximum_vertical_delta_metres\":")?;
    writer.optional_float_string(report.max_vertical_drift_metres())?;
    writer.raw(b",\"reason\":")?;
    writer.optional_string(
        report
            .failure()
            .reason()
            .map(super::roundtrip::RoundTripReasonCode::as_str),
    )?;
    writer.raw(b",\"reference_face_count\":")?;
    writer.u64(report.reference_face_count())?;
    writer.raw(b",\"reference_point_count\":")?;
    writer.u64(report.reference_point_count())?;
    writer.raw(b",\"removed_face_count\":")?;
    writer.optional_u64(difference.map(|value| value.removed_count))?;
    writer.raw(b",\"removed_face_hash\":")?;
    writer.optional_hex(difference.map(|value| value.removed_hash.as_slice()))?;
    writer.raw(b",\"removed_face_sample\":")?;
    writer.optional_faces(difference.map(|value| value.removed_sample.as_ref()))?;
    writer.raw(b",\"unmatched_point_count\":")?;
    writer.optional_u64(completed_mapping_count)?;
    writer.raw(b"}")
}

fn write_limits(
    writer: &mut CanonicalEvidenceWriter,
    limits: RoundTripLimits,
    reference_parser_peak_bytes: u64,
    returned_parser_peak_bytes: u64,
    retained_peak_bytes: u64,
) -> Result<(), EvidenceError> {
    writer.raw(b"{\"accounted_reference_parser_peak_bytes\":")?;
    writer.u64(reference_parser_peak_bytes)?;
    writer.raw(b",\"accounted_retained_peak_bytes\":")?;
    writer.u64(retained_peak_bytes)?;
    writer.raw(b",\"accounted_returned_parser_peak_bytes\":")?;
    writer.u64(returned_parser_peak_bytes)?;
    writer.raw(b",\"candidate_vertex_comparisons\":")?;
    writer.u64(limits.comparisons())?;
    writer.raw(b",\"evidence_output_bytes\":")?;
    writer.u64(MAX_EVIDENCE_BYTES)?;
    writer.raw(b",\"faces_per_surface\":")?;
    writer.u64(limits.faces())?;
    writer.raw(b",\"file_bytes_per_input\":")?;
    writer.u64(limits.file_bytes())?;
    writer.raw(b",\"parser_working_bytes_per_input\":")?;
    writer.u64(limits.parser_working_bytes())?;
    writer.raw(b",\"points_per_surface\":")?;
    writer.u64(limits.points())?;
    writer.raw(b",\"publication_buffer_bytes\":")?;
    writer.u64(MAX_PUBLICATION_BUFFER_BYTES as u64)?;
    writer.raw(b",\"retained_working_bytes_total\":")?;
    writer.u64(limits.retained_working_bytes())?;
    writer.raw(b",\"xml_nodes_per_input\":")?;
    writer.u64(limits.xml_nodes())?;
    writer.raw(b",\"xml_text_attribute_bytes_per_input\":")?;
    writer.u64(limits.xml_text_bytes())?;
    writer.raw(b",\"xml_token_bytes_per_input\":")?;
    writer.u64(limits.xml_token_bytes())?;
    writer.raw(b"}")
}

#[cfg(test)]
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("String writes cannot fail");
    }
    output
}

fn evidence_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn publish_bound_or_reconcile(
    binding: &BoundEvidenceTarget,
    expected: &[u8],
    expected_hash: [u8; 32],
    hook: &impl PublicationHook,
) -> Result<(), EvidenceError> {
    let target = binding.target();
    let parent = binding.parent();
    let parent_witness = DirectoryWitness::capture(parent)
        .map_err(|source| EvidenceError::io("witness evidence parent", parent, source))?;
    let (mut stage, mut file) = create_stage(
        parent,
        "round-trip-evidence",
        || Ok::<(), EvidenceError>(()),
        map_stage_error,
    )?;
    file.write_all(expected)
        .and_then(|()| file.flush())
        .map_err(|source| EvidenceError::io("write evidence stage", stage.path(), source))?;
    file.sync_all()
        .map_err(|source| EvidenceError::io("sync evidence stage", stage.path(), source))?;
    drop(file);
    stage
        .verify()
        .map_err(|source| EvidenceError::io("verify evidence stage", stage.path(), source))?;
    parent_witness
        .verify()
        .map_err(|source| EvidenceError::io("verify evidence parent", parent, source))?;
    binding
        .verify()
        .map_err(|source| EvidenceError::io("verify bound evidence target", target, source))?;
    hook.reach(PublicationBoundary::BeforeLink)
        .map_err(|source| EvidenceError::io("run evidence pre-link boundary", target, source))?;
    binding
        .verify()
        .map_err(|source| EvidenceError::io("revalidate bound evidence target", target, source))?;

    match fs::hard_link(stage.path(), target) {
        Ok(()) => {
            stage.mark_linked();
            binding
                .verify()
                .map_err(|source| EvidenceError::Indeterminate {
                    path: target.to_path_buf(),
                    expected_hash,
                    source,
                })?;
            finish_publication(
                binding,
                &parent_witness,
                &mut stage,
                expected,
                expected_hash,
                hook,
            )
        }
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => reconcile_existing(
            binding,
            &parent_witness,
            &mut stage,
            expected,
            expected_hash,
            hook,
        ),
        Err(source) => Err(EvidenceError::io("publish evidence", target, source)),
    }
}

fn reconcile_existing(
    binding: &BoundEvidenceTarget,
    parent_witness: &DirectoryWitness,
    stage: &mut StageGuard,
    expected: &[u8],
    expected_hash: [u8; 32],
    hook: &impl PublicationHook,
) -> Result<(), EvidenceError> {
    let target = binding.target();
    let parent = binding.parent();
    let indeterminate = |source| EvidenceError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash,
        source,
    };
    hook.reach(PublicationBoundary::TargetVerification)
        .map_err(indeterminate)?;
    binding.verify().map_err(indeterminate)?;
    let actual = read_stable_target(target, expected)?;
    if !actual.matches_expected {
        return Err(EvidenceError::Conflict(target.to_path_buf()));
    }
    parent_witness.verify().map_err(indeterminate)?;
    hook.reach(PublicationBoundary::ParentSync)
        .map_err(indeterminate)?;
    sync_directory(parent).map_err(indeterminate)?;
    hook.reach(PublicationBoundary::StageRemoval)
        .map_err(indeterminate)?;
    stage.remove().map_err(indeterminate)?;
    hook.reach(PublicationBoundary::CleanupSync)
        .map_err(indeterminate)?;
    sync_directory(parent).map_err(indeterminate)?;
    parent_witness.verify().map_err(indeterminate)?;
    hook.reach(PublicationBoundary::TerminalAcknowledgement)
        .map_err(indeterminate)?;
    binding.verify().map_err(indeterminate)?;
    verify_acknowledged_target(target, expected, &actual.identity)
        .map_err(io::Error::other)
        .map_err(indeterminate)
}

fn finish_publication(
    binding: &BoundEvidenceTarget,
    parent_witness: &DirectoryWitness,
    stage: &mut StageGuard,
    expected: &[u8],
    expected_hash: [u8; 32],
    hook: &impl PublicationHook,
) -> Result<(), EvidenceError> {
    let target = binding.target();
    let parent = binding.parent();
    let post_link = |source| EvidenceError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash,
        source,
    };
    hook.reach(PublicationBoundary::TargetVerification)
        .map_err(post_link)?;
    binding.verify().map_err(post_link)?;
    parent_witness.verify().map_err(post_link)?;
    let actual = read_linked_target(stage.path(), target, expected)
        .map_err(io::Error::other)
        .map_err(post_link)?;
    if !actual.matches_expected {
        return Err(post_link(io::Error::new(
            io::ErrorKind::InvalidData,
            "published evidence differs from the canonical stage",
        )));
    }
    hook.reach(PublicationBoundary::ParentSync)
        .map_err(post_link)?;
    sync_directory(parent).map_err(post_link)?;
    hook.reach(PublicationBoundary::StageRemoval)
        .map_err(post_link)?;
    stage.remove().map_err(post_link)?;
    hook.reach(PublicationBoundary::CleanupSync)
        .map_err(post_link)?;
    sync_directory(parent).map_err(post_link)?;
    parent_witness.verify().map_err(post_link)?;
    hook.reach(PublicationBoundary::TerminalAcknowledgement)
        .map_err(post_link)?;
    binding.verify().map_err(post_link)?;
    verify_acknowledged_target(target, expected, &actual.identity)
        .map_err(io::Error::other)
        .map_err(post_link)?;
    Ok(())
}

fn read_linked_target(
    stage: &Path,
    target: &Path,
    expected: &[u8],
) -> Result<StableTarget, EvidenceError> {
    let stage_metadata = fs::symlink_metadata(stage)
        .map_err(|source| EvidenceError::io("inspect evidence stage", stage, source))?;
    let target_metadata = fs::symlink_metadata(target)
        .map_err(|source| EvidenceError::io("inspect evidence target", target, source))?;
    if !same_file_identity(&stage_metadata, &target_metadata) {
        return Err(EvidenceError::Conflict(target.to_path_buf()));
    }
    read_stable_target(target, expected)
}

struct StableTarget {
    matches_expected: bool,
    identity: fs::Metadata,
}

fn read_stable_target(path: &Path, expected: &[u8]) -> Result<StableTarget, EvidenceError> {
    let expected_bytes = expected.len() as u64;
    let initial = fs::symlink_metadata(path)
        .map_err(|source| EvidenceError::io("inspect evidence target", path, source))?;
    if !initial.file_type().is_file() || initial.len() != expected_bytes {
        return Err(EvidenceError::Conflict(path.to_path_buf()));
    }
    let mut file = File::open(path)
        .map_err(|source| EvidenceError::io("open evidence target", path, source))?;
    let opened = file
        .metadata()
        .map_err(|source| EvidenceError::io("inspect open evidence target", path, source))?;
    if !same_file_state(&initial, &opened) || opened.len() != expected_bytes {
        return Err(EvidenceError::Conflict(path.to_path_buf()));
    }
    let mut buffer = [0; MAX_PUBLICATION_BUFFER_BYTES];
    let mut offset = 0;
    let mut matches_expected = true;
    while offset < expected.len() {
        let requested = buffer.len().min(expected.len() - offset);
        let count = file
            .read(&mut buffer[..requested])
            .map_err(|source| EvidenceError::io("read evidence target", path, source))?;
        if count == 0 {
            return Err(EvidenceError::Conflict(path.to_path_buf()));
        }
        matches_expected &= buffer[..count] == expected[offset..offset + count];
        offset += count;
    }
    let final_opened = file
        .metadata()
        .map_err(|source| EvidenceError::io("reinspect open evidence target", path, source))?;
    let final_metadata = fs::symlink_metadata(path)
        .map_err(|source| EvidenceError::io("reinspect evidence target", path, source))?;
    if !same_file_state(&opened, &final_opened) || !same_file_state(&final_opened, &final_metadata)
    {
        return Err(EvidenceError::Conflict(path.to_path_buf()));
    }
    Ok(StableTarget {
        matches_expected,
        identity: final_metadata,
    })
}

fn verify_acknowledged_target(
    path: &Path,
    expected: &[u8],
    linked_identity: &fs::Metadata,
) -> Result<(), EvidenceError> {
    let current = read_stable_target(path, expected)?;
    if same_file_identity(linked_identity, &current.identity) && current.matches_expected {
        Ok(())
    } else {
        Err(EvidenceError::Conflict(path.to_path_buf()))
    }
}

#[cfg(unix)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_state(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

fn map_stage_error(error: StageCreationError) -> EvidenceError {
    match error {
        StageCreationError::RandomnessUnavailable => EvidenceError::Io {
            operation: "create evidence stage",
            path: PathBuf::from("evidence stage"),
            source: io::Error::other("system randomness is unavailable"),
        },
        StageCreationError::NamespaceExhausted => EvidenceError::Io {
            operation: "create evidence stage",
            path: PathBuf::from("evidence stage"),
            source: io::Error::other("evidence staging namespace is exhausted"),
        },
        StageCreationError::Inspect { path, source } => {
            EvidenceError::io("inspect evidence stage", &path, source)
        }
        StageCreationError::Create { path, source } => {
            EvidenceError::io("create evidence stage", &path, source)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::Write as _,
        path::Path,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use serde_json::Value;

    use super::*;
    use crate::{
        journal::WorkflowRunId,
        roundtrip::{RoundTripDeclaration, RoundTripTolerances, evaluate_landxml_round_trip},
    };

    const REFERENCE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" version=\"1.2\"><Units><Metric linearUnit=\"meter\"/></Units><Surfaces><Surface name=\"Reference\"><Definition surfType=\"TIN\"><Pnts><P id=\"1\">0 0 0</P><P id=\"2\">0 10 0</P><P id=\"3\">10 10 0</P><P id=\"4\">10 0 0</P></Pnts><Faces><F>1 2 3</F><F>1 3 4</F></Faces></Definition></Surface></Surfaces></LandXML>";
    const TOPOLOGY_FAILURE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" version=\"1.2\"><Units><Metric linearUnit=\"meter\"/></Units><Surfaces><Surface name=\"Reference\"><Definition surfType=\"TIN\"><Pnts><P id=\"1\">0 0 0</P><P id=\"2\">0 10 0</P><P id=\"3\">10 10 0</P><P id=\"4\">10 0 0</P></Pnts><Faces><F>1 2 3</F></Faces></Definition></Surface></Surfaces></LandXML>";
    const CANONICAL_PUBLICATION: &[u8] = b"canonical round-trip evidence\n";

    #[test]
    fn canonical_v1_bytes_match_checked_in_pass_and_topology_failure_fixtures() {
        let directory = FixtureDirectory::new();
        let reference = directory.path.join("reference.xml");
        fs::write(&reference, REFERENCE).unwrap();
        let run = fixed_run_snapshot();
        let cases: [(&str, &str, &[u8], EvidenceResult, &str); 2] = [
            (
                "pass",
                REFERENCE,
                include_bytes!("../tests/fixtures/round-trip-evidence-v1/passed.json"),
                EvidenceResult::Passed,
                "45d22a67f6a72168a65f821480cd9a02e8ef4a6114cf8ef3f28754b334b9a7d8",
            ),
            (
                "topology-failed",
                TOPOLOGY_FAILURE,
                include_bytes!("../tests/fixtures/round-trip-evidence-v1/topology-failed.json"),
                EvidenceResult::Failed,
                "8b524bc678f32e8f1d85ea0d594b32b7ebeb9c279adae5c4165abce09ed16ab2",
            ),
        ];
        for (label, returned_xml, expected, expected_result, expected_hash) in cases {
            let returned = directory.path.join(format!("{label}.xml"));
            fs::write(&returned, returned_xml).unwrap();
            let evaluation = evaluate_landxml_round_trip(
                &reference,
                &returned,
                RoundTripDeclaration::new(
                    "generated-fixture",
                    "v1-test-only",
                    "layer=ground;profile=metric-tin",
                )
                .unwrap(),
                RoundTripTolerances::new(0.001, 0.002).unwrap(),
                RoundTripLimits::default(),
            )
            .unwrap();
            let (bytes, result) =
                encode_evidence(&run, &evaluation, RoundTripLimits::default()).unwrap();
            assert_eq!(result, expected_result);
            assert_eq!(bytes, expected, "canonical {label} evidence changed");
            assert_eq!(hex(blake3::hash(&bytes).as_bytes()), expected_hash);
            if label == "topology-failed" {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["comparison"]["mapped_point_count"], 4);
                assert_eq!(value["comparison"]["unmatched_point_count"], 0);
                assert_eq!(value["comparison"]["ambiguous_point_count"], 0);
                assert_eq!(value["comparison"]["candidate_comparisons"], 8);
                assert_eq!(
                    value["comparison"]["maximum_horizontal_delta_metres"],
                    "0.00000000000000000"
                );
            }
        }
    }

    #[test]
    fn canonical_encoder_accepts_exact_output_limit_and_rejects_one_under() {
        let directory = FixtureDirectory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        fs::write(&reference, REFERENCE).unwrap();
        fs::write(&returned, REFERENCE).unwrap();
        let run = fixed_run_snapshot();
        let evaluation = evaluate_landxml_round_trip(
            &reference,
            &returned,
            RoundTripDeclaration::new(
                "generated-fixture",
                "v1-test-only",
                "layer=ground;profile=metric-tin",
            )
            .unwrap(),
            RoundTripTolerances::new(0.001, 0.002).unwrap(),
            RoundTripLimits::default(),
        )
        .unwrap();
        let expected = include_bytes!("../tests/fixtures/round-trip-evidence-v1/passed.json");

        let (exact, result) = encode_evidence_with_limit(
            &run,
            &evaluation,
            RoundTripLimits::default(),
            expected.len() as u64,
        )
        .expect("an exact evidence byte limit is inclusive");
        assert_eq!(exact, expected);
        assert_eq!(result, EvidenceResult::Passed);

        assert!(matches!(
            encode_evidence_with_limit(
                &run,
                &evaluation,
                RoundTripLimits::default(),
                expected.len() as u64 - 1,
            ),
            Err(EvidenceError::Resource(required)) if required == expected.len() as u64
        ));
    }

    #[test]
    fn every_post_link_boundary_is_indeterminate_and_exactly_reconcilable() {
        let directory = FixtureDirectory::new();
        let expected_hash = evidence_hash(CANONICAL_PUBLICATION);
        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalAcknowledgement,
        ] {
            let target = directory.path.join(format!("{boundary:?}.json"));

            let failure = publish_or_reconcile(
                &target,
                CANONICAL_PUBLICATION,
                expected_hash,
                &TestHook(TestAction::Failure(boundary)),
            )
            .expect_err("post-link failure cannot acknowledge publication");

            assert!(matches!(
                &failure,
                EvidenceError::Indeterminate {
                    expected_hash: actual_hash,
                    ..
                } if *actual_hash == expected_hash
            ));
            assert!(
                failure
                    .to_string()
                    .contains("injected evidence publication failure")
            );
            assert_eq!(fs::read(&target).unwrap(), CANONICAL_PUBLICATION);
            directory.assert_retained_stages_are_canonical();
            publish_or_reconcile(
                &target,
                CANONICAL_PUBLICATION,
                expected_hash,
                &ProductionPublicationHook,
            )
            .expect("retry reconciles exact complete evidence");
            assert_eq!(fs::read(&target).unwrap(), CANONICAL_PUBLICATION);
            directory.assert_retained_stages_are_canonical();
        }
    }

    #[test]
    fn every_no_replace_reconciliation_boundary_is_indeterminate_and_retryable() {
        let directory = FixtureDirectory::new();
        let target = directory.path.join("existing-exact.json");
        let expected_hash = evidence_hash(CANONICAL_PUBLICATION);
        write_synced(&target, CANONICAL_PUBLICATION, false).unwrap();

        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalAcknowledgement,
        ] {
            let failure = publish_or_reconcile(
                &target,
                CANONICAL_PUBLICATION,
                expected_hash,
                &TestHook(TestAction::Failure(boundary)),
            )
            .expect_err("an interrupted exact reconciliation has no receipt");

            assert!(matches!(
                &failure,
                EvidenceError::Indeterminate {
                    expected_hash: actual_hash,
                    ..
                } if *actual_hash == expected_hash
            ));
            assert!(
                failure
                    .to_string()
                    .contains("injected evidence publication failure")
            );
            assert_eq!(fs::read(&target).unwrap(), CANONICAL_PUBLICATION);
            directory.assert_retained_stages_are_canonical();
            publish_or_reconcile(
                &target,
                CANONICAL_PUBLICATION,
                expected_hash,
                &ProductionPublicationHook,
            )
            .expect("retry reconciles the exact existing evidence");
            directory.assert_retained_stages_are_canonical();
        }
    }

    #[test]
    fn no_replace_create_races_reconcile_exact_and_preserve_conflicts() {
        let directory = FixtureDirectory::new();
        let expected_hash = evidence_hash(CANONICAL_PUBLICATION);
        let exact = directory.path.join("exact-race.json");

        publish_or_reconcile(
            &exact,
            CANONICAL_PUBLICATION,
            expected_hash,
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::BeforeLink,
                target: &exact,
                bytes: CANONICAL_PUBLICATION,
                replace: false,
            }),
        )
        .expect("an exact no-replace create race reconciles");
        assert_eq!(fs::read(&exact).unwrap(), CANONICAL_PUBLICATION);
        directory.assert_retained_stages_are_canonical();

        let conflict = directory.path.join("conflicting-race.json");
        let caller_bytes = b"caller-owned conflicting evidence\n";
        let failure = publish_or_reconcile(
            &conflict,
            CANONICAL_PUBLICATION,
            expected_hash,
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::BeforeLink,
                target: &conflict,
                bytes: caller_bytes,
                replace: false,
            }),
        )
        .expect_err("a conflicting no-replace create race fails closed");
        let canonical_conflict = fs::canonicalize(conflict.parent().unwrap())
            .unwrap()
            .join(conflict.file_name().unwrap());
        assert!(
            matches!(&failure, EvidenceError::Conflict(path) if path == &canonical_conflict),
            "unexpected conflict-race error: {failure:?}"
        );
        assert_eq!(fs::read(&conflict).unwrap(), caller_bytes);
        directory.assert_retained_stages_are_canonical();
    }

    #[test]
    fn exact_reconciliation_is_bounded_across_publication_buffer_chunks() {
        let directory = FixtureDirectory::new();
        let target = directory.path.join("chunked-exact.json");
        let expected = vec![b'x'; MAX_PUBLICATION_BUFFER_BYTES * 2 + 17];
        let expected_hash = evidence_hash(&expected);
        write_synced(&target, &expected, false).unwrap();

        publish_or_reconcile(
            &target,
            &expected,
            expected_hash,
            &ProductionPublicationHook,
        )
        .expect("exact reconciliation reads through the fixed publication buffer");

        assert_eq!(fs::read(&target).unwrap(), expected);
        directory.assert_retained_stages_are_canonical();
    }

    #[test]
    fn post_link_target_replacement_is_preserved_and_never_acknowledged() {
        let directory = FixtureDirectory::new();
        let target = directory.path.join("replaced-target.json");
        let replacement = b"caller replacement after publication\n";
        let expected_hash = evidence_hash(CANONICAL_PUBLICATION);

        let failure = publish_or_reconcile(
            &target,
            CANONICAL_PUBLICATION,
            expected_hash,
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::TargetVerification,
                target: &target,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("a replaced post-link target has no receipt");

        assert!(matches!(failure, EvidenceError::Indeterminate { .. }));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        directory.assert_retained_stages_are_canonical();
    }

    #[test]
    fn post_link_in_place_modification_is_never_acknowledged() {
        let directory = FixtureDirectory::new();
        let target = directory.path.join("modified-target.json");
        let replacement = vec![b'x'; CANONICAL_PUBLICATION.len()];
        let expected_hash = evidence_hash(CANONICAL_PUBLICATION);

        let failure = publish_or_reconcile(
            &target,
            CANONICAL_PUBLICATION,
            expected_hash,
            &TestHook(TestAction::Overwrite {
                boundary: PublicationBoundary::TerminalAcknowledgement,
                target: &target,
                bytes: &replacement,
            }),
        )
        .expect_err("an in-place modified target has no receipt");

        assert!(matches!(failure, EvidenceError::Indeterminate { .. }));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        directory.assert_retained_stages_are_canonical();
    }

    #[cfg(unix)]
    #[test]
    fn ancestor_symlink_retarget_never_publishes_inside_the_qualified_run() {
        use std::os::unix::fs::symlink;

        let directory = FixtureDirectory::new();
        let expected_hash = evidence_hash(CANONICAL_PUBLICATION);
        for boundary in [
            PublicationBoundary::BeforeLink,
            PublicationBoundary::TargetVerification,
        ] {
            let outside = directory.path.join(format!("outside-{boundary:?}"));
            let run = directory.path.join(format!("run-{boundary:?}"));
            let alias = directory.path.join(format!("outside-alias-{boundary:?}"));
            fs::create_dir(&outside).unwrap();
            fs::create_dir(&run).unwrap();
            symlink(&outside, &alias).unwrap();
            let requested_target = alias.join("evidence.json");
            let bound = BoundEvidenceTarget::bind(&requested_target, &run).unwrap();

            let failure = publish_bound_or_reconcile(
                &bound,
                CANONICAL_PUBLICATION,
                expected_hash,
                &TestHook(TestAction::RetargetAncestor {
                    boundary,
                    alias: &alias,
                    replacement: &run,
                }),
            )
            .expect_err("retargeting an outside ancestor invalidates its binding");

            if boundary == PublicationBoundary::BeforeLink {
                assert!(matches!(failure, EvidenceError::Io { .. }));
                assert!(!outside.join("evidence.json").exists());
            } else {
                assert!(matches!(failure, EvidenceError::Indeterminate { .. }));
                assert_eq!(
                    fs::read(outside.join("evidence.json")).unwrap(),
                    CANONICAL_PUBLICATION
                );
            }
            assert!(
                !run.join("evidence.json").exists(),
                "publication must never follow a retargeted ancestor into the Run"
            );
            for entry in fs::read_dir(&outside).unwrap() {
                let entry = entry.unwrap();
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".punctra-round-trip-evidence-")
                {
                    assert!(fs::symlink_metadata(entry.path()).unwrap().is_file());
                    let bytes = fs::read(entry.path()).unwrap();
                    assert!(bytes.is_empty() || bytes == CANONICAL_PUBLICATION);
                }
            }
        }
    }

    fn publish_or_reconcile(
        target: &Path,
        expected: &[u8],
        expected_hash: [u8; 32],
        hook: &impl PublicationHook,
    ) -> Result<(), EvidenceError> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let qualified_run = parent.join(".qualified-run-fixture");
        fs::create_dir_all(&qualified_run).unwrap();
        let binding = BoundEvidenceTarget::bind(target, &qualified_run)?;
        publish_bound_or_reconcile(&binding, expected, expected_hash, hook)
    }

    fn fixed_run_snapshot() -> CompleteRunQualificationSnapshot {
        CompleteRunQualificationSnapshot {
            run: WorkflowRunId::new([0x11; 16]).unwrap(),
            request_hash: [0x12; 32],
            terminal_journal_hash: [0x13; 32],
            journal_bytes: 2_804,
            source: [0x14; 32],
            workspace: [0x15; 16],
            baseline_revision: [0x16; 32],
            operation: [0x17; 16],
            revision: [0x18; 32],
            audit_hash: [0x19; 32],
            surface_hash: [0x1a; 32],
            qa_hash: [0x1b; 32],
            landxml_hash: *blake3::hash(REFERENCE.as_bytes()).as_bytes(),
            landxml_bytes: REFERENCE.len() as u64,
            report_hash: [0x1c; 32],
            report_bytes: 11_490,
            options_hash: [0x1d; 32],
            path_bindings: [[0x1e; 32], [0x1f; 32], [0x20; 32], [0x21; 32]],
        }
    }

    struct FixtureDirectory {
        path: PathBuf,
    }

    impl FixtureDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "punctra-round-trip-evidence-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn assert_retained_stages_are_canonical(&self) {
            for entry in fs::read_dir(&self.path).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(".punctra-round-trip-evidence-") {
                    assert!(name.ends_with(".tmp"), "stage alias remains recognizable");
                    let metadata = fs::symlink_metadata(entry.path()).unwrap();
                    assert!(metadata.file_type().is_file());
                    let bytes = fs::read(entry.path()).unwrap();
                    assert!(
                        bytes.len() <= MAX_PUBLICATION_BUFFER_BYTES,
                        "a retained evidence stage remains publication-buffer bounded"
                    );
                }
            }
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[derive(Clone, Copy)]
    enum TestAction<'a> {
        Failure(PublicationBoundary),
        Install {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
            replace: bool,
        },
        Overwrite {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
        },
        #[cfg(unix)]
        RetargetAncestor {
            boundary: PublicationBoundary,
            alias: &'a Path,
            replacement: &'a Path,
        },
    }

    struct TestHook<'a>(TestAction<'a>);

    impl PublicationHook for TestHook<'_> {
        fn reach(&self, boundary: PublicationBoundary) -> io::Result<()> {
            match self.0 {
                TestAction::Failure(expected) if expected == boundary => {
                    Err(io::Error::other("injected evidence publication failure"))
                }
                TestAction::Install {
                    boundary: expected,
                    target,
                    bytes,
                    replace,
                } if expected == boundary => write_synced(target, bytes, replace),
                TestAction::Overwrite {
                    boundary: expected,
                    target,
                    bytes,
                } if expected == boundary => overwrite_synced(target, bytes),
                #[cfg(unix)]
                TestAction::RetargetAncestor {
                    boundary: expected,
                    alias,
                    replacement,
                } if expected == boundary => {
                    fs::remove_file(alias)?;
                    std::os::unix::fs::symlink(replacement, alias)
                }
                TestAction::Failure(_)
                | TestAction::Install { .. }
                | TestAction::Overwrite { .. } => Ok(()),
                #[cfg(unix)]
                TestAction::RetargetAncestor { .. } => Ok(()),
            }
        }
    }

    fn write_synced(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
        if replace {
            fs::remove_file(path)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))
    }

    fn overwrite_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))
    }
}
