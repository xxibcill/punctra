//! Private, bounded semantic verification for returned `LandXML` terrain files.

use std::{
    borrow::Cow,
    error::Error,
    fmt,
    fs::{self, File, Metadata},
    io::{self, BufRead, Read, Seek as _, SeekFrom},
    mem::size_of,
    path::Path,
};

use quick_xml::events::BytesStart;
use robust::{Coord, orient2d};

use crate::{
    bounded_diagnostic::BoundedDiagnostic,
    diagnostic::{FailureCode, RecoveryAction},
};

const LANDXML_NAMESPACE: &str = "http://www.landxml.org/schema/LandXML-1.2";
const XINCLUDE_NAMESPACE: &str = "http://www.w3.org/2001/XInclude";
const MAX_APPLICATION_BYTES: usize = 128;
const MAX_VERSION_BYTES: usize = 128;
const MAX_SETTINGS_PROFILE_BYTES: usize = 1_024;

const DEFAULT_MAX_FILE_BYTES: u64 = 4 * 1_024 * 1_024 * 1_024;
const DEFAULT_MAX_XML_NODES: u64 = 70_000_128;
const DEFAULT_MAX_XML_TEXT_BYTES: u64 = DEFAULT_MAX_FILE_BYTES;
const DEFAULT_MAX_XML_TOKEN_BYTES: u64 = 4 * 1_024;
const DEFAULT_MAX_PARSER_WORKING_BYTES: u64 = 8 * 1_024 * 1_024;
const DEFAULT_MAX_RETAINED_WORKING_BYTES: u64 = 4 * 1_024 * 1_024 * 1_024;
const DEFAULT_MAX_POINTS: u64 = 10_000_000;
const DEFAULT_MAX_FACES: u64 = 20_000_000;
const DEFAULT_MAX_COMPARISONS: u64 = 32_000_000;
const PARSER_READ_BUFFER_BYTES: usize = 64 * 1_024;
const EXACT_COMPARE_BUFFER_BYTES: usize = 64 * 1_024;
const BOUNDED_XML_IO_ERROR: &str = "punctra bounded XML input rejected";
pub(crate) const MATCHER_VERSION: &str = "punctra-landxml-tin-matcher-v1";
const FACE_DIAGNOSTIC_SAMPLE: usize = 8;
pub(crate) const ADDED_FACE_HASH_DOMAIN: &[u8] = b"punctra-round-trip-added-faces-v1";
pub(crate) const REMOVED_FACE_HASH_DOMAIN: &[u8] = b"punctra-round-trip-removed-faces-v1";

struct FallibleBufReader<R> {
    inner: R,
    buffer: Vec<u8>,
    position: usize,
    filled: usize,
}

impl<R: Read> FallibleBufReader<R> {
    fn new(inner: R, side: InputSide, bytes: usize) -> Result<Self, RoundTripFailure> {
        Ok(Self {
            inner,
            buffer: fallible_zeroed_buffer(side, "parser input", bytes)?,
            position: 0,
            filled: 0,
        })
    }

    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }
}

impl<R: Read> Read for FallibleBufReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl<R: Read> BufRead for FallibleBufReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.filled {
            self.filled = self.inner.read(&mut self.buffer)?;
            self.position = 0;
        }
        Ok(&self.buffer[self.position..self.filled])
    }

    fn consume(&mut self, amount: usize) {
        self.position = self.position.saturating_add(amount).min(self.filled);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputSide {
    Reference,
    Returned,
}

impl InputSide {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "REFERENCE",
            Self::Returned => "RETURNED",
        }
    }
}

impl fmt::Display for InputSide {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoundTripDeclaration {
    application: String,
    version: String,
    settings_profile: String,
}

impl RoundTripDeclaration {
    pub(crate) fn new(
        application: impl AsRef<str>,
        version: impl AsRef<str>,
        settings_profile: impl AsRef<str>,
    ) -> Result<Self, RoundTripFailure> {
        let application =
            fallible_declaration_field("application", application.as_ref(), MAX_APPLICATION_BYTES)?;
        let version = fallible_declaration_field("version", version.as_ref(), MAX_VERSION_BYTES)?;
        let settings_profile = fallible_declaration_field(
            "settings profile",
            settings_profile.as_ref(),
            MAX_SETTINGS_PROFILE_BYTES,
        )?;
        Ok(Self {
            application,
            version,
            settings_profile,
        })
    }

    pub(crate) fn declared_application(&self) -> &str {
        &self.application
    }

    pub(crate) fn declared_version(&self) -> &str {
        &self.version
    }

    pub(crate) fn declared_settings_profile(&self) -> &str {
        &self.settings_profile
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoundTripTolerances {
    horizontal_metres: f64,
    vertical_metres: f64,
}

impl RoundTripTolerances {
    pub(crate) fn new(
        horizontal_metres: f64,
        vertical_metres: f64,
    ) -> Result<Self, RoundTripFailure> {
        validate_tolerance("horizontal", horizontal_metres)?;
        validate_tolerance("vertical", vertical_metres)?;
        Ok(Self {
            horizontal_metres: canonical_zero(horizontal_metres),
            vertical_metres: canonical_zero(vertical_metres),
        })
    }

    pub(crate) const fn horizontal_metres(self) -> f64 {
        self.horizontal_metres
    }

    pub(crate) const fn vertical_metres(self) -> f64 {
        self.vertical_metres
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RoundTripLimits {
    file_bytes: u64,
    xml_nodes: u64,
    xml_text_bytes: u64,
    xml_token_bytes: u64,
    parser_working_bytes: u64,
    points: u64,
    faces: u64,
    comparisons: u64,
    retained_working_bytes: u64,
}

impl RoundTripLimits {
    #[cfg(test)]
    const fn new(
        max_file_bytes: u64,
        max_xml_nodes: u64,
        max_xml_text_bytes: u64,
        max_points: u64,
        max_faces: u64,
        max_comparisons: u64,
    ) -> Self {
        Self {
            file_bytes: max_file_bytes,
            xml_nodes: max_xml_nodes,
            xml_text_bytes: max_xml_text_bytes,
            xml_token_bytes: DEFAULT_MAX_XML_TOKEN_BYTES,
            parser_working_bytes: DEFAULT_MAX_PARSER_WORKING_BYTES,
            points: max_points,
            faces: max_faces,
            comparisons: max_comparisons,
            retained_working_bytes: DEFAULT_MAX_RETAINED_WORKING_BYTES,
        }
    }

    #[cfg(test)]
    const fn with_working_limits(
        mut self,
        max_xml_token_bytes: u64,
        max_parser_working_bytes: u64,
        max_retained_working_bytes: u64,
    ) -> Self {
        self.xml_token_bytes = max_xml_token_bytes;
        self.parser_working_bytes = max_parser_working_bytes;
        self.retained_working_bytes = max_retained_working_bytes;
        self
    }

    const fn default_const() -> Self {
        Self {
            file_bytes: DEFAULT_MAX_FILE_BYTES,
            xml_nodes: DEFAULT_MAX_XML_NODES,
            xml_text_bytes: DEFAULT_MAX_XML_TEXT_BYTES,
            xml_token_bytes: DEFAULT_MAX_XML_TOKEN_BYTES,
            parser_working_bytes: DEFAULT_MAX_PARSER_WORKING_BYTES,
            points: DEFAULT_MAX_POINTS,
            faces: DEFAULT_MAX_FACES,
            comparisons: DEFAULT_MAX_COMPARISONS,
            retained_working_bytes: DEFAULT_MAX_RETAINED_WORKING_BYTES,
        }
    }

    pub(crate) const fn file_bytes(self) -> u64 {
        self.file_bytes
    }

    pub(crate) const fn xml_nodes(self) -> u64 {
        self.xml_nodes
    }

    pub(crate) const fn xml_text_bytes(self) -> u64 {
        self.xml_text_bytes
    }

    pub(crate) const fn xml_token_bytes(self) -> u64 {
        self.xml_token_bytes
    }

    pub(crate) const fn parser_working_bytes(self) -> u64 {
        self.parser_working_bytes
    }

    pub(crate) const fn points(self) -> u64 {
        self.points
    }

    pub(crate) const fn faces(self) -> u64 {
        self.faces
    }

    pub(crate) const fn comparisons(self) -> u64 {
        self.comparisons
    }

    pub(crate) const fn retained_working_bytes(self) -> u64 {
        self.retained_working_bytes
    }
}

impl Default for RoundTripLimits {
    fn default() -> Self {
        Self::default_const()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoundTripFailureKind {
    InvalidInput,
    ResourceLimit,
    SemanticMismatch,
}

impl RoundTripFailureKind {
    pub(crate) const fn as_str(self) -> &'static str {
        self.workflow_mapping().0.as_str()
    }

    pub(crate) const fn workflow_mapping(self) -> (FailureCode, RecoveryAction) {
        match self {
            Self::InvalidInput => (
                FailureCode::RoundTripInvalidInput,
                RecoveryAction::CorrectRoundTripInput,
            ),
            Self::ResourceLimit => (
                FailureCode::RoundTripResourceLimit,
                RecoveryAction::UseSupportedRoundTripSize,
            ),
            Self::SemanticMismatch => (
                FailureCode::RoundTripSemanticMismatch,
                RecoveryAction::ReviewReturnedLandXml,
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoundTripFailure {
    kind: RoundTripFailureKind,
    reason: Option<RoundTripReasonCode>,
    topology_difference: Vec<TopologyDifference>,
    diagnostic: BoundedDiagnostic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TopologyDifference {
    pub(crate) added_count: u64,
    pub(crate) removed_count: u64,
    pub(crate) added_hash: [u8; 32],
    pub(crate) removed_hash: [u8; 32],
    pub(crate) added_sample: Vec<[u64; 3]>,
    pub(crate) removed_sample: Vec<[u64; 3]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoundTripReasonCode {
    UnitDrift,
    PointCountDrift,
    VertexUnmatched,
    VertexAmbiguous,
    ToleranceDrift,
    TopologyDrift,
}

impl RoundTripReasonCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::UnitDrift => "PRT_UNIT_DRIFT",
            Self::PointCountDrift => "PRT_POINT_COUNT_DRIFT",
            Self::VertexUnmatched => "PRT_VERTEX_UNMATCHED",
            Self::VertexAmbiguous => "PRT_VERTEX_AMBIGUOUS",
            Self::ToleranceDrift => "PRT_TOLERANCE_DRIFT",
            Self::TopologyDrift => "PRT_TOPOLOGY_DRIFT",
        }
    }
}

impl RoundTripFailure {
    fn invalid(error: impl fmt::Display) -> Self {
        Self::new(RoundTripFailureKind::InvalidInput, None, error)
    }

    fn resource(error: impl fmt::Display) -> Self {
        Self::new(RoundTripFailureKind::ResourceLimit, None, error)
    }

    fn mismatch(reason: RoundTripReasonCode, error: impl fmt::Display) -> Self {
        Self::new(RoundTripFailureKind::SemanticMismatch, Some(reason), error)
    }

    fn new(
        kind: RoundTripFailureKind,
        reason: Option<RoundTripReasonCode>,
        error: impl fmt::Display,
    ) -> Self {
        Self {
            kind,
            reason,
            topology_difference: Vec::new(),
            diagnostic: BoundedDiagnostic::new(error),
        }
    }

    fn topology_mismatch(
        difference: TopologyDifference,
        error: impl fmt::Display,
    ) -> Result<Self, Self> {
        let mut topology_difference = Vec::new();
        topology_difference
            .try_reserve_exact(1)
            .map_err(|_| Self::resource("topology-difference diagnostic allocation failed"))?;
        topology_difference.push(difference);
        Ok(Self {
            kind: RoundTripFailureKind::SemanticMismatch,
            reason: Some(RoundTripReasonCode::TopologyDrift),
            topology_difference,
            diagnostic: BoundedDiagnostic::new(error),
        })
    }

    pub(crate) const fn kind(&self) -> RoundTripFailureKind {
        self.kind
    }

    pub(crate) fn diagnostic(&self) -> &str {
        self.diagnostic.as_str()
    }

    pub(crate) const fn reason(&self) -> Option<RoundTripReasonCode> {
        self.reason
    }

    pub(crate) fn topology_difference(&self) -> Option<&TopologyDifference> {
        self.topology_difference.first()
    }
}

impl fmt::Display for RoundTripFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind.as_str(), self.diagnostic)
    }
}

impl Error for RoundTripFailure {}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoundTripReport {
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    reference_content_hash: [u8; 32],
    returned_content_hash: [u8; 32],
    reference_bytes: u64,
    returned_bytes: u64,
    vertex_count: u64,
    face_count: u64,
    comparison_count: u64,
    max_easting_drift_metres: f64,
    max_northing_drift_metres: f64,
    max_horizontal_drift_metres: f64,
    max_vertical_drift_metres: f64,
    exact_bytes: bool,
    topology_matches: bool,
    reference_parser_peak_bytes: u64,
    returned_parser_peak_bytes: u64,
    retained_peak_bytes: u64,
    returned_surface_name: String,
    returned_ignored_top_level_sections: Vec<String>,
}

impl RoundTripReport {
    pub(crate) fn declared_application(&self) -> &str {
        self.declaration.declared_application()
    }

    pub(crate) fn declared_version(&self) -> &str {
        self.declaration.declared_version()
    }

    pub(crate) fn declared_settings_profile(&self) -> &str {
        self.declaration.declared_settings_profile()
    }

    pub(crate) const fn tolerances(&self) -> RoundTripTolerances {
        self.tolerances
    }

    pub(crate) const fn reference_content_hash(&self) -> [u8; 32] {
        self.reference_content_hash
    }

    pub(crate) const fn returned_content_hash(&self) -> [u8; 32] {
        self.returned_content_hash
    }

    pub(crate) const fn reference_bytes(&self) -> u64 {
        self.reference_bytes
    }

    pub(crate) const fn returned_bytes(&self) -> u64 {
        self.returned_bytes
    }

    pub(crate) const fn vertex_count(&self) -> u64 {
        self.vertex_count
    }

    pub(crate) const fn face_count(&self) -> u64 {
        self.face_count
    }

    pub(crate) const fn comparison_count(&self) -> u64 {
        self.comparison_count
    }

    pub(crate) const fn max_easting_drift_metres(&self) -> f64 {
        self.max_easting_drift_metres
    }

    pub(crate) const fn max_northing_drift_metres(&self) -> f64 {
        self.max_northing_drift_metres
    }

    pub(crate) const fn max_horizontal_drift_metres(&self) -> f64 {
        self.max_horizontal_drift_metres
    }

    pub(crate) const fn max_vertical_drift_metres(&self) -> f64 {
        self.max_vertical_drift_metres
    }

    pub(crate) const fn exact_bytes(&self) -> bool {
        self.exact_bytes
    }

    pub(crate) const fn topology_matches(&self) -> bool {
        self.topology_matches
    }

    pub(crate) const fn reference_parser_peak_bytes(&self) -> u64 {
        self.reference_parser_peak_bytes
    }

    pub(crate) const fn returned_parser_peak_bytes(&self) -> u64 {
        self.returned_parser_peak_bytes
    }

    pub(crate) const fn retained_peak_bytes(&self) -> u64 {
        self.retained_peak_bytes
    }

    pub(crate) fn returned_surface_name(&self) -> &str {
        &self.returned_surface_name
    }

    pub(crate) fn returned_ignored_top_level_sections(&self) -> &[String] {
        &self.returned_ignored_top_level_sections
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RoundTripFailedReport {
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    reference_content_hash: [u8; 32],
    returned_content_hash: [u8; 32],
    reference_bytes: u64,
    returned_bytes: u64,
    reference_parser_peak_bytes: u64,
    returned_parser_peak_bytes: u64,
    retained_peak_bytes: u64,
    comparison: Option<ComparisonFacts>,
    reference_surface: SurfaceEvidenceFacts,
    returned_surface: Option<SurfaceEvidenceFacts>,
    failure: RoundTripFailure,
}

#[derive(Clone, Debug)]
struct SurfaceEvidenceFacts {
    name: String,
    points: u64,
    faces: u64,
    ignored_top_level_sections: Vec<String>,
}

impl RoundTripFailedReport {
    pub(crate) fn declaration(&self) -> &RoundTripDeclaration {
        &self.declaration
    }

    pub(crate) const fn tolerances(&self) -> RoundTripTolerances {
        self.tolerances
    }

    pub(crate) const fn reference_content_hash(&self) -> [u8; 32] {
        self.reference_content_hash
    }

    pub(crate) const fn returned_content_hash(&self) -> [u8; 32] {
        self.returned_content_hash
    }

    pub(crate) const fn reference_bytes(&self) -> u64 {
        self.reference_bytes
    }

    pub(crate) const fn returned_bytes(&self) -> u64 {
        self.returned_bytes
    }

    pub(crate) const fn reference_parser_peak_bytes(&self) -> u64 {
        self.reference_parser_peak_bytes
    }

    pub(crate) const fn returned_parser_peak_bytes(&self) -> u64 {
        self.returned_parser_peak_bytes
    }

    pub(crate) const fn retained_peak_bytes(&self) -> u64 {
        self.retained_peak_bytes
    }

    pub(crate) const fn comparison_count(&self) -> Option<u64> {
        match self.comparison {
            Some(facts) => Some(facts.comparison_count),
            None => None,
        }
    }

    pub(crate) const fn max_easting_drift_metres(&self) -> Option<f64> {
        match self.comparison {
            Some(facts) => Some(facts.max_easting_drift_metres),
            None => None,
        }
    }

    pub(crate) const fn max_northing_drift_metres(&self) -> Option<f64> {
        match self.comparison {
            Some(facts) => Some(facts.max_northing_drift_metres),
            None => None,
        }
    }

    pub(crate) const fn max_horizontal_drift_metres(&self) -> Option<f64> {
        match self.comparison {
            Some(facts) => Some(facts.max_horizontal_drift_metres),
            None => None,
        }
    }

    pub(crate) const fn max_vertical_drift_metres(&self) -> Option<f64> {
        match self.comparison {
            Some(facts) => Some(facts.max_vertical_drift_metres),
            None => None,
        }
    }

    pub(crate) fn returned_surface_name(&self) -> Option<&str> {
        self.returned_surface
            .as_ref()
            .map(|facts| facts.name.as_ref())
    }

    pub(crate) const fn reference_point_count(&self) -> u64 {
        self.reference_surface.points
    }

    pub(crate) const fn reference_face_count(&self) -> u64 {
        self.reference_surface.faces
    }

    pub(crate) fn returned_ignored_top_level_sections(&self) -> Option<&[String]> {
        self.returned_surface
            .as_ref()
            .map(|facts| facts.ignored_top_level_sections.as_ref())
    }

    pub(crate) fn returned_point_count(&self) -> Option<u64> {
        self.returned_surface.as_ref().map(|facts| facts.points)
    }

    pub(crate) fn returned_face_count(&self) -> Option<u64> {
        self.returned_surface.as_ref().map(|facts| facts.faces)
    }

    pub(crate) fn failure(&self) -> &RoundTripFailure {
        &self.failure
    }

    pub(crate) fn into_failure(self) -> RoundTripFailure {
        self.failure
    }
}

pub(crate) enum RoundTripEvaluation {
    Passed(RoundTripReport),
    Failed(RoundTripFailedReport),
}

pub(crate) fn verify_landxml_round_trip(
    reference_path: &Path,
    returned_path: &Path,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
) -> Result<RoundTripReport, RoundTripFailure> {
    match evaluate_landxml_round_trip(
        reference_path,
        returned_path,
        declaration,
        tolerances,
        limits,
    )? {
        RoundTripEvaluation::Passed(report) => Ok(report),
        RoundTripEvaluation::Failed(report) => Err(report.into_failure()),
    }
}

pub(crate) fn evaluate_landxml_round_trip(
    reference_path: &Path,
    returned_path: &Path,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
) -> Result<RoundTripEvaluation, RoundTripFailure> {
    validate_limits(limits)?;
    let (mut reference_witness, mut returned_witness) =
        capture_file_pair(reference_path, returned_path, limits.file_bytes)?;
    let result = evaluate_captured_pair(
        &mut reference_witness,
        &mut returned_witness,
        declaration,
        tolerances,
        limits,
    );
    let reference_check = reference_witness.revalidate(InputSide::Reference);
    let returned_check = returned_witness.revalidate(InputSide::Returned);
    reference_check?;
    returned_check?;
    result
}

fn evaluate_captured_pair(
    reference_witness: &mut FileWitness<'_>,
    returned_witness: &mut FileWitness<'_>,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
) -> Result<RoundTripEvaluation, RoundTripFailure> {
    let (reference_digest, returned_digest) =
        hash_input_pair(reference_witness, returned_witness, limits)?;
    let reference_parse = parse_surface(
        InputSide::Reference,
        &mut reference_witness.file,
        limits,
        0,
        reference_witness.metadata.len(),
    )
    .map_err(|failure| failure.failure)?;
    let reference_retained = reference_parse.surface.retained_bytes();
    let returned_parse = match parse_surface(
        InputSide::Returned,
        &mut returned_witness.file,
        limits,
        reference_retained,
        returned_witness.metadata.len(),
    ) {
        Ok(parsed) => parsed,
        Err(parsed) if parsed.failure.kind() == RoundTripFailureKind::SemanticMismatch => {
            return Ok(failed_returned_parse_evaluation(
                declaration,
                tolerances,
                reference_digest,
                returned_digest,
                reference_parse,
                parsed,
            ));
        }
        Err(parsed) => return Err(parsed.failure),
    };
    let comparison = match compare_surfaces(
        &reference_parse.surface,
        &returned_parse.surface,
        tolerances,
        limits,
    ) {
        Ok(comparison) => comparison,
        Err(failure) if failure.failure.kind() == RoundTripFailureKind::SemanticMismatch => {
            return Ok(failed_comparison_evaluation(
                declaration,
                tolerances,
                reference_digest,
                returned_digest,
                reference_parse,
                returned_parse,
                failure,
            ));
        }
        Err(failure) => return Err(failure.failure),
    };
    let (exact_bytes, retained_peak_bytes) = finish_exact_comparison(
        reference_witness,
        returned_witness,
        &reference_parse,
        &returned_parse,
        comparison.retained_peak_bytes,
        limits,
    )?;
    let reference_surface = reference_parse.surface;
    let returned_surface = returned_parse.surface;

    Ok(RoundTripEvaluation::Passed(RoundTripReport {
        declaration,
        tolerances,
        reference_content_hash: reference_digest.hash,
        returned_content_hash: returned_digest.hash,
        reference_bytes: reference_digest.bytes,
        returned_bytes: returned_digest.bytes,
        vertex_count: reference_surface.points.len() as u64,
        face_count: reference_surface.faces.len() as u64,
        comparison_count: comparison.comparison_count,
        max_easting_drift_metres: comparison.max_easting_drift_metres,
        max_northing_drift_metres: comparison.max_northing_drift_metres,
        max_horizontal_drift_metres: comparison.max_horizontal_drift_metres,
        max_vertical_drift_metres: comparison.max_vertical_drift_metres,
        exact_bytes,
        topology_matches: true,
        reference_parser_peak_bytes: reference_parse
            .parser_peak_bytes
            .max(reference_digest.working_peak_bytes),
        returned_parser_peak_bytes: returned_parse
            .parser_peak_bytes
            .max(returned_digest.working_peak_bytes),
        retained_peak_bytes,
        returned_surface_name: returned_surface.surface_name,
        returned_ignored_top_level_sections: returned_surface.ignored_top_level_sections,
    }))
}

fn failed_comparison_evaluation(
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    reference_digest: FileDigest,
    returned_digest: FileDigest,
    reference: ParsedInput,
    returned: ParsedInput,
    failure: ComparisonFailure,
) -> RoundTripEvaluation {
    let reference_parser_peak_bytes = reference
        .parser_peak_bytes
        .max(reference_digest.working_peak_bytes);
    let returned_parser_peak_bytes = returned
        .parser_peak_bytes
        .max(returned_digest.working_peak_bytes);
    let retained_peak_bytes = reference
        .retained_peak_bytes
        .max(returned.retained_peak_bytes)
        .max(failure.comparison.retained_peak_bytes);
    RoundTripEvaluation::Failed(RoundTripFailedReport {
        declaration,
        tolerances,
        reference_content_hash: reference_digest.hash,
        returned_content_hash: returned_digest.hash,
        reference_bytes: reference_digest.bytes,
        returned_bytes: returned_digest.bytes,
        reference_parser_peak_bytes,
        returned_parser_peak_bytes,
        retained_peak_bytes,
        comparison: failure.comparison_available.then_some(failure.comparison),
        reference_surface: reference.surface.into_evidence_facts(),
        returned_surface: Some(returned.surface.into_evidence_facts()),
        failure: failure.failure,
    })
}

fn failed_returned_parse_evaluation(
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    reference_digest: FileDigest,
    returned_digest: FileDigest,
    reference: ParsedInput,
    returned: ParseFailure,
) -> RoundTripEvaluation {
    RoundTripEvaluation::Failed(RoundTripFailedReport {
        declaration,
        tolerances,
        reference_content_hash: reference_digest.hash,
        returned_content_hash: returned_digest.hash,
        reference_bytes: reference_digest.bytes,
        returned_bytes: returned_digest.bytes,
        reference_parser_peak_bytes: reference
            .parser_peak_bytes
            .max(reference_digest.working_peak_bytes),
        returned_parser_peak_bytes: returned
            .parser_peak_bytes
            .max(returned_digest.working_peak_bytes),
        retained_peak_bytes: reference
            .retained_peak_bytes
            .max(returned.retained_peak_bytes),
        comparison: None,
        reference_surface: reference.surface.into_evidence_facts(),
        returned_surface: None,
        failure: returned.failure,
    })
}

fn finish_exact_comparison(
    reference_witness: &mut FileWitness<'_>,
    returned_witness: &mut FileWitness<'_>,
    reference: &ParsedInput,
    returned: &ParsedInput,
    comparison_peak_bytes: u64,
    limits: RoundTripLimits,
) -> Result<(bool, u64), RoundTripFailure> {
    let surface_bytes = reference
        .surface
        .retained_bytes()
        .saturating_add(returned.surface.retained_bytes());
    let (exact_bytes, exact_compare_peak) =
        files_are_equal(reference_witness, returned_witness, limits, surface_bytes)?;
    let retained_peak_bytes = reference
        .retained_peak_bytes
        .max(returned.retained_peak_bytes)
        .max(comparison_peak_bytes)
        .max(exact_compare_peak);
    Ok((exact_bytes, retained_peak_bytes))
}

fn hash_input_pair(
    reference: &mut FileWitness<'_>,
    returned: &mut FileWitness<'_>,
    limits: RoundTripLimits,
) -> Result<(FileDigest, FileDigest), RoundTripFailure> {
    Ok((
        hash_regular_file(InputSide::Reference, reference, limits)?,
        hash_regular_file(InputSide::Returned, returned, limits)?,
    ))
}

fn validate_declaration_field(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), RoundTripFailure> {
    if value.is_empty() || value.trim() != value {
        return Err(RoundTripFailure::invalid(format_args!(
            "declared {field} must be nonempty with no surrounding whitespace"
        )));
    }
    if value.len() > max_bytes {
        return Err(RoundTripFailure::resource(format_args!(
            "declared {field} uses {} bytes; limit is {max_bytes}",
            value.len()
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(RoundTripFailure::invalid(format_args!(
            "declared {field} contains a control character"
        )));
    }
    Ok(())
}

fn fallible_declaration_field(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<String, RoundTripFailure> {
    validate_declaration_field(field, value, max_bytes)?;
    let mut owned = String::new();
    owned.try_reserve_exact(value.len()).map_err(|_| {
        RoundTripFailure::resource(format_args!("declared {field} allocation failed"))
    })?;
    if owned.capacity() > max_bytes {
        return Err(RoundTripFailure::resource(format_args!(
            "declared {field} storage uses {} bytes; limit is {max_bytes}",
            owned.capacity()
        )));
    }
    owned.push_str(value);
    Ok(owned)
}

fn validate_tolerance(axis: &str, value: f64) -> Result<(), RoundTripFailure> {
    if !value.is_finite() || value < 0.0 {
        return Err(RoundTripFailure::invalid(format_args!(
            "{axis} tolerance must be finite and nonnegative metres"
        )));
    }
    Ok(())
}

fn validate_limits(limits: RoundTripLimits) -> Result<(), RoundTripFailure> {
    if limits.file_bytes == u64::MAX {
        return Err(RoundTripFailure::invalid(
            "file-byte limit must leave room for an over-limit sentinel byte",
        ));
    }
    if limits.xml_nodes == 0
        || limits.xml_text_bytes == 0
        || limits.xml_token_bytes == 0
        || limits.parser_working_bytes == 0
        || limits.retained_working_bytes == 0
    {
        return Err(RoundTripFailure::invalid(
            "XML and working-memory limits must be nonzero",
        ));
    }
    Ok(())
}

struct FileWitness<'a> {
    path: &'a Path,
    file: File,
    metadata: Metadata,
}

impl FileWitness<'_> {
    fn rewind(&mut self, side: InputSide) -> Result<(), RoundTripFailure> {
        self.file.seek(SeekFrom::Start(0)).map_err(|error| {
            RoundTripFailure::invalid(format_args!("{side} cannot be rewound: {error}"))
        })?;
        Ok(())
    }

    fn revalidate(&self, side: InputSide) -> Result<(), RoundTripFailure> {
        let final_metadata = self.file.metadata().map_err(|error| {
            RoundTripFailure::invalid(format_args!("{side} metadata cannot be rechecked: {error}"))
        })?;
        let final_path_metadata = fs::symlink_metadata(self.path).map_err(|error| {
            RoundTripFailure::invalid(format_args!("{side} path cannot be rechecked: {error}"))
        })?;
        if !same_file_state(&self.metadata, &final_metadata)
            || final_path_metadata.file_type().is_symlink()
            || !final_path_metadata.is_file()
            || !same_file_state(&self.metadata, &final_path_metadata)
        {
            return Err(RoundTripFailure::invalid(format_args!(
                "{side} changed while it was being read"
            )));
        }
        Ok(())
    }
}

fn capture_file_pair<'a>(
    reference_path: &'a Path,
    returned_path: &'a Path,
    max_file_bytes: u64,
) -> Result<(FileWitness<'a>, FileWitness<'a>), RoundTripFailure> {
    let reference = capture_regular_file(InputSide::Reference, reference_path, max_file_bytes)?;
    let returned = capture_regular_file(InputSide::Returned, returned_path, max_file_bytes)?;
    Ok((reference, returned))
}

fn capture_regular_file(
    side: InputSide,
    path: &Path,
    max_file_bytes: u64,
) -> Result<FileWitness<'_>, RoundTripFailure> {
    let path_metadata = fs::symlink_metadata(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be inspected: {error}"))
    })?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} must be a regular file and not a symbolic link"
        )));
    }
    let path_identity = require_file_identity(side, &path_metadata)?;
    check_file_bytes(side, path_metadata.len(), max_file_bytes)?;

    let file = File::open(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be opened: {error}"))
    })?;
    let open_metadata = file.metadata().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} metadata cannot be read: {error}"))
    })?;
    let open_identity = require_file_identity(side, &open_metadata)?;
    if !open_metadata.is_file()
        || path_identity != open_identity
        || !same_file_state(&path_metadata, &open_metadata)
    {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being opened"
        )));
    }
    Ok(FileWitness {
        path,
        file,
        metadata: open_metadata,
    })
}

#[cfg(unix)]
// The shared caller is fallible because Windows can omit stable identity fields
// and unsupported platforms fail closed; Unix always supplies device/inode.
#[allow(clippy::unnecessary_wraps)]
fn require_file_identity(
    _side: InputSide,
    metadata: &Metadata,
) -> Result<FileIdentity, RoundTripFailure> {
    Ok(FileIdentity::from_metadata(metadata))
}

#[cfg(windows)]
fn require_file_identity(
    side: InputSide,
    metadata: &Metadata,
) -> Result<FileIdentity, RoundTripFailure> {
    FileIdentity::from_metadata(metadata).ok_or_else(|| {
        RoundTripFailure::invalid(format_args!(
            "{side} filesystem does not expose a stable file identity"
        ))
    })
}

#[cfg(not(any(unix, windows)))]
fn require_file_identity(
    side: InputSide,
    _metadata: &Metadata,
) -> Result<FileIdentity, RoundTripFailure> {
    Err(RoundTripFailure::invalid(format_args!(
        "{side} filesystem does not expose a stable file identity"
    )))
}

fn check_file_bytes(side: InputSide, actual: u64, allowed: u64) -> Result<(), RoundTripFailure> {
    if actual > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} file bytes required {actual}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn check_working_limit(
    resource: impl fmt::Display,
    required: u64,
    allowed: u64,
) -> Result<(), RoundTripFailure> {
    if required > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{resource} bytes required {required}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn collection_bytes<T>(values: &Vec<T>) -> u64 {
    capacity_bytes::<T>(values.capacity())
}

fn capacity_bytes<T>(capacity: usize) -> u64 {
    u64::try_from(capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(size_of::<T>() as u64)
}

fn overlapping_capacity(old_capacity: usize, new_capacity: usize) -> usize {
    old_capacity.saturating_add(new_capacity)
}

fn growth_capacity(
    current_capacity: usize,
    required_capacity: usize,
) -> Result<usize, RoundTripFailure> {
    if required_capacity > current_capacity {
        current_capacity
            .checked_add(required_capacity)
            .ok_or_else(|| RoundTripFailure::resource("collection growth charge overflowed"))
    } else {
        Ok(current_capacity)
    }
}

fn projected_len<T>(values: &[T], additional: usize) -> Result<usize, RoundTripFailure> {
    let required = values.len().checked_add(additional).ok_or_else(|| {
        RoundTripFailure::resource("retained collection charge calculation overflowed")
    })?;
    Ok(required)
}

fn projected_string_len(value: &str, additional: usize) -> Result<usize, RoundTripFailure> {
    let required = value
        .len()
        .checked_add(additional)
        .ok_or_else(|| RoundTripFailure::resource("XML text charge calculation overflowed"))?;
    Ok(required)
}

fn add_text_bytes(
    side: InputSide,
    current: u64,
    additional: usize,
    allowed: u64,
) -> Result<u64, RoundTripFailure> {
    let required = current.saturating_add(additional as u64);
    if required > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} XML text bytes required at least {required}; limit is {allowed}"
        )));
    }
    Ok(required)
}

#[derive(Clone, Copy)]
struct FileDigest {
    hash: [u8; 32],
    bytes: u64,
    working_peak_bytes: u64,
}

fn hash_regular_file(
    side: InputSide,
    witness: &mut FileWitness<'_>,
    limits: RoundTripLimits,
) -> Result<FileDigest, RoundTripFailure> {
    check_working_limit(
        format_args!("{side} input read buffer"),
        PARSER_READ_BUFFER_BYTES as u64,
        limits.parser_working_bytes,
    )?;
    witness.rewind(side)?;
    let mut buffer = fallible_zeroed_buffer(side, "input read", PARSER_READ_BUFFER_BYTES)?;
    let working_peak_bytes = buffer.capacity() as u64;
    check_working_limit(
        format_args!("{side} input read buffer"),
        working_peak_bytes,
        limits.parser_working_bytes,
    )?;
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0_u64;
    let mut remaining = witness.metadata.len();
    while remaining > 0 {
        let requested = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let count = witness
            .file
            .read(&mut buffer[..requested])
            .map_err(|error| {
                RoundTripFailure::invalid(format_args!("{side} cannot be read: {error}"))
            })?;
        if count == 0 {
            return Err(RoundTripFailure::invalid(format_args!(
                "{side} changed while it was being read"
            )));
        }
        bytes = bytes.saturating_add(count as u64);
        check_file_bytes(side, bytes, limits.file_bytes)?;
        hasher.update(&buffer[..count]);
        remaining = remaining.saturating_sub(count as u64);
    }
    let mut sentinel = [0_u8; 1];
    let extra = witness.file.read(&mut sentinel).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be rechecked: {error}"))
    })?;
    if bytes != witness.metadata.len() || extra != 0 {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read"
        )));
    }
    Ok(FileDigest {
        hash: *hasher.finalize().as_bytes(),
        bytes,
        working_peak_bytes,
    })
}

#[cfg(test)]
fn read_regular_file(
    side: InputSide,
    mut witness: FileWitness<'_>,
    max_file_bytes: u64,
) -> Result<FileDigest, RoundTripFailure> {
    let limits = RoundTripLimits {
        file_bytes: max_file_bytes,
        ..RoundTripLimits::default()
    };
    let digest = hash_regular_file(side, &mut witness, limits)?;
    witness.revalidate(side)?;
    Ok(digest)
}

fn files_are_equal(
    reference: &mut FileWitness<'_>,
    returned: &mut FileWitness<'_>,
    limits: RoundTripLimits,
    retained_base_bytes: u64,
) -> Result<(bool, u64), RoundTripFailure> {
    if reference.metadata.len() != returned.metadata.len() {
        return Ok((false, retained_base_bytes));
    }
    reference.rewind(InputSide::Reference)?;
    returned.rewind(InputSide::Returned)?;
    let (mut reference_buffer, mut returned_buffer, retained_peak_bytes) =
        exact_compare_buffers(limits, retained_base_bytes)?;
    let mut remaining = reference.metadata.len();
    while remaining > 0 {
        let requested = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(EXACT_COMPARE_BUFFER_BYTES);
        let reference_count = reference
            .file
            .read(&mut reference_buffer[..requested])
            .map_err(|error| {
                RoundTripFailure::invalid(format_args!("REFERENCE cannot be read: {error}"))
            })?;
        let returned_count = returned
            .file
            .read(&mut returned_buffer[..requested])
            .map_err(|error| {
                RoundTripFailure::invalid(format_args!("RETURNED cannot be read: {error}"))
            })?;
        if reference_count == 0 || returned_count == 0 {
            return Err(RoundTripFailure::invalid(
                "captured input changed during exact-byte comparison",
            ));
        }
        if reference_count != returned_count
            || reference_buffer[..reference_count] != returned_buffer[..returned_count]
        {
            return Ok((false, retained_peak_bytes));
        }
        remaining = remaining.saturating_sub(reference_count as u64);
    }
    let mut sentinel = [0_u8; 1];
    let reference_extra = reference.file.read(&mut sentinel).map_err(|error| {
        RoundTripFailure::invalid(format_args!("REFERENCE cannot be rechecked: {error}"))
    })?;
    let returned_extra = returned.file.read(&mut sentinel).map_err(|error| {
        RoundTripFailure::invalid(format_args!("RETURNED cannot be rechecked: {error}"))
    })?;
    if reference_extra != 0 || returned_extra != 0 {
        return Err(RoundTripFailure::invalid(
            "captured input grew during exact-byte comparison",
        ));
    }
    Ok((true, retained_peak_bytes))
}

fn exact_compare_buffers(
    limits: RoundTripLimits,
    retained_base_bytes: u64,
) -> Result<(Vec<u8>, Vec<u8>, u64), RoundTripFailure> {
    let requested = EXACT_COMPARE_BUFFER_BYTES as u64;
    check_exact_compare_capacity(limits, retained_base_bytes, requested)?;
    let reference = fallible_zeroed_buffer(
        InputSide::Reference,
        "exact-byte comparison",
        EXACT_COMPARE_BUFFER_BYTES,
    )?;
    let reference_capacity = reference.capacity() as u64;
    check_exact_compare_capacity(limits, retained_base_bytes, reference_capacity)?;
    check_exact_compare_capacity(
        limits,
        retained_base_bytes,
        reference_capacity.saturating_add(requested),
    )?;
    let returned = fallible_zeroed_buffer(
        InputSide::Returned,
        "exact-byte comparison",
        EXACT_COMPARE_BUFFER_BYTES,
    )?;
    let total_capacity = reference_capacity.saturating_add(returned.capacity() as u64);
    check_exact_compare_capacity(limits, retained_base_bytes, total_capacity)?;
    Ok((
        reference,
        returned,
        retained_base_bytes.saturating_add(total_capacity),
    ))
}

fn check_exact_compare_capacity(
    limits: RoundTripLimits,
    retained_base_bytes: u64,
    buffer_bytes: u64,
) -> Result<(), RoundTripFailure> {
    check_working_limit(
        "exact-byte comparison",
        buffer_bytes,
        limits.parser_working_bytes,
    )?;
    check_working_limit(
        "retained round-trip data",
        retained_base_bytes.saturating_add(buffer_bytes),
        limits.retained_working_bytes,
    )
}

fn fallible_zeroed_buffer(
    side: InputSide,
    purpose: &str,
    bytes: usize,
) -> Result<Vec<u8>, RoundTripFailure> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(bytes).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} {purpose} buffer cannot reserve {bytes} bytes"
        ))
    })?;
    buffer.resize(bytes, 0);
    Ok(buffer)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
}

impl FileIdentity {
    #[cfg(unix)]
    fn from_metadata(metadata: &Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    #[cfg(windows)]
    fn from_metadata(metadata: &Metadata) -> Option<Self> {
        use std::os::windows::fs::MetadataExt as _;
        Some(Self {
            volume_serial_number: metadata.volume_serial_number()?,
            file_index: metadata.file_index()?,
        })
    }
}

#[cfg(unix)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    FileIdentity::from_metadata(left) == FileIdentity::from_metadata(right)
}

#[cfg(windows)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    matches!(
        (
            FileIdentity::from_metadata(left),
            FileIdentity::from_metadata(right)
        ),
        (Some(left), Some(right)) if left == right
    )
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

#[cfg(unix)]
fn same_file_state(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_file_state(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_state(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

#[derive(Clone, Copy, Debug)]
struct Position {
    easting: f64,
    northing: f64,
    elevation: f64,
}

#[derive(Clone, Copy, Debug)]
struct IndexedPosition {
    id: u64,
    position: Position,
}

impl Position {
    fn key(self) -> [u64; 3] {
        [
            self.easting.to_bits(),
            self.northing.to_bits(),
            self.elevation.to_bits(),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    first: u64,
    second: u64,
    third: u64,
}

impl Triangle {
    const fn new(first: u64, second: u64, third: u64) -> Self {
        Self {
            first,
            second,
            third,
        }
    }

    const fn has_repeated_point(self) -> bool {
        self.first == self.second || self.second == self.third || self.first == self.third
    }

    fn positions(self, points: &[Position]) -> [Position; 3] {
        let first = usize::try_from(self.first).expect("validated point index fits usize");
        let second = usize::try_from(self.second).expect("validated point index fits usize");
        let third = usize::try_from(self.third).expect("validated point index fits usize");
        [points[first], points[second], points[third]]
    }

    fn canonical_point_indices(self) -> [u64; 3] {
        let mut indices = [self.first, self.second, self.third];
        indices.sort_unstable();
        indices
    }

    fn remap(self, point_mapping: &[usize]) -> Self {
        let first = usize::try_from(self.first).expect("validated point index fits usize");
        let second = usize::try_from(self.second).expect("validated point index fits usize");
        let third = usize::try_from(self.third).expect("validated point index fits usize");
        Self::new(
            point_mapping[first] as u64,
            point_mapping[second] as u64,
            point_mapping[third] as u64,
        )
    }
}

#[derive(Debug)]
struct ParsedSurface {
    points: Vec<Position>,
    faces: Vec<Triangle>,
    surface_name: String,
    ignored_top_level_sections: Vec<String>,
}

impl ParsedSurface {
    fn into_evidence_facts(self) -> SurfaceEvidenceFacts {
        SurfaceEvidenceFacts {
            name: self.surface_name,
            points: self.points.len() as u64,
            faces: self.faces.len() as u64,
            ignored_top_level_sections: self.ignored_top_level_sections,
        }
    }

    fn retained_bytes(&self) -> u64 {
        collection_bytes::<Position>(&self.points)
            .saturating_add(collection_bytes::<Triangle>(&self.faces))
            .saturating_add(self.surface_name.capacity() as u64)
            .saturating_add(
                self.ignored_top_level_sections
                    .iter()
                    .map(|section| section.capacity() as u64)
                    .sum::<u64>(),
            )
            .saturating_add(collection_bytes::<String>(&self.ignored_top_level_sections))
    }
}

struct ParsedInput {
    surface: ParsedSurface,
    parser_peak_bytes: u64,
    retained_peak_bytes: u64,
}

struct ParseFailure {
    failure: RoundTripFailure,
    parser_peak_bytes: u64,
    retained_peak_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ElementKind {
    LandXml,
    Units,
    Metric,
    Project,
    Application,
    Surfaces,
    Surface,
    Definition,
    Pnts,
    Point,
    Faces,
    Face,
    Ignored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamNamespace {
    LandXml,
    XInclude,
    Other,
    Unbound,
}

#[derive(Clone, Copy, Debug, Default)]
struct NamespaceFrame {
    buffer_len: usize,
    binding_len: usize,
}

#[derive(Clone, Copy, Debug)]
struct NamespaceBinding {
    prefix_start: usize,
    prefix_len: usize,
    value_start: usize,
    value_len: usize,
}

struct NamespaceState {
    buffer: Vec<u8>,
    bindings: Vec<NamespaceBinding>,
}

#[derive(Clone, Copy)]
struct XmlAttribute<'a> {
    name: &'a [u8],
    value: &'a str,
}

struct XmlAttributes<'a> {
    side: InputSide,
    remaining: &'a [u8],
}

impl<'a> XmlAttributes<'a> {
    fn new(side: InputSide, raw: &'a [u8]) -> Self {
        Self {
            side,
            remaining: raw,
        }
    }
}

impl<'a> Iterator for XmlAttributes<'a> {
    type Item = Result<XmlAttribute<'a>, RoundTripFailure>;

    fn next(&mut self) -> Option<Self::Item> {
        let untrimmed = self.remaining;
        self.remaining = trim_ascii_whitespace(untrimmed);
        if self.remaining.is_empty() {
            return None;
        }
        if self.remaining.len() == untrimmed.len() {
            self.remaining = &[];
            return Some(Err(RoundTripFailure::invalid(format_args!(
                "{} XML attributes must be separated by whitespace",
                self.side
            ))));
        }
        let name_end = self
            .remaining
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || *byte == b'=')
            .unwrap_or(self.remaining.len());
        let name = &self.remaining[..name_end];
        if name.is_empty() {
            self.remaining = &[];
            return Some(Err(RoundTripFailure::invalid(format_args!(
                "{} XML attribute name is missing",
                self.side
            ))));
        }
        let after_name = trim_ascii_whitespace(&self.remaining[name_end..]);
        let Some(after_equals) = after_name.strip_prefix(b"=") else {
            self.remaining = &[];
            return Some(Err(RoundTripFailure::invalid(format_args!(
                "{} XML attribute is missing '='",
                self.side
            ))));
        };
        let after_equals = trim_ascii_whitespace(after_equals);
        let Some(quote @ (b'\'' | b'"')) = after_equals.first().copied() else {
            self.remaining = &[];
            return Some(Err(RoundTripFailure::invalid(format_args!(
                "{} XML attribute value is not quoted",
                self.side
            ))));
        };
        let value_bytes = &after_equals[1..];
        let Some(value_end) = value_bytes.iter().position(|byte| *byte == quote) else {
            self.remaining = &[];
            return Some(Err(RoundTripFailure::invalid(format_args!(
                "{} XML attribute value is incomplete",
                self.side
            ))));
        };
        self.remaining = &value_bytes[value_end + 1..];
        let value = match std::str::from_utf8(&value_bytes[..value_end]) {
            Ok(value) => value,
            Err(error) => {
                return Some(Err(RoundTripFailure::invalid(format_args!(
                    "{} XML attribute value is not UTF-8: {error}",
                    self.side
                ))));
            }
        };
        Some(Ok(XmlAttribute { name, value }))
    }
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    value
}

fn element_attributes<'a>(side: InputSide, element: &'a BytesStart<'a>) -> XmlAttributes<'a> {
    XmlAttributes::new(side, element.attributes_raw())
}

impl NamespaceState {
    const fn new() -> Self {
        Self {
            buffer: Vec::new(),
            bindings: Vec::new(),
        }
    }

    fn push(
        &mut self,
        state: &mut SurfaceStreamParser,
        element: &BytesStart<'_>,
    ) -> Result<NamespaceFrame, RoundTripFailure> {
        let frame = NamespaceFrame {
            buffer_len: self.buffer.len(),
            binding_len: self.bindings.len(),
        };
        let mut additional_buffer = 0usize;
        let mut additional_bindings = 0usize;
        for attribute in element_attributes(state.side, element) {
            let attribute = attribute?;
            if let Some(prefix) = namespace_declaration_prefix(attribute.name) {
                validate_namespace_prefix(state.side, prefix)?;
                let value_len = normalized_attribute_len(attribute.value)?;
                additional_buffer = additional_buffer
                    .checked_add(prefix.len())
                    .and_then(|bytes| bytes.checked_add(value_len))
                    .ok_or_else(|| {
                        RoundTripFailure::resource("XML namespace storage length overflow")
                    })?;
                additional_bindings = additional_bindings.checked_add(1).ok_or_else(|| {
                    RoundTripFailure::resource("XML namespace binding count overflow")
                })?;
            }
        }
        self.reserve(state, additional_buffer, additional_bindings)?;
        for attribute in element_attributes(state.side, element) {
            let attribute = attribute?;
            let Some(prefix) = namespace_declaration_prefix(attribute.name) else {
                continue;
            };
            let prefix_start = self.buffer.len();
            self.buffer.extend_from_slice(prefix);
            let value_start = self.buffer.len();
            append_normalized_attribute_bytes(attribute.value, &mut self.buffer)?;
            let value_len = self.buffer.len() - value_start;
            validate_namespace_declaration(
                state.side,
                prefix,
                &self.buffer[value_start..value_start + value_len],
            )?;
            self.bindings.push(NamespaceBinding {
                prefix_start,
                prefix_len: prefix.len(),
                value_start,
                value_len,
            });
        }
        for attribute in element_attributes(state.side, element) {
            let attribute = attribute?;
            let name = attribute.name;
            if namespace_declaration_prefix(name).is_some() {
                continue;
            }
            let Some(colon) = name.iter().position(|byte| *byte == b':') else {
                continue;
            };
            let prefix = &name[..colon];
            if prefix != b"xml"
                && !self
                    .bindings
                    .iter()
                    .rev()
                    .any(|binding| self.prefix(binding) == prefix)
            {
                return Err(RoundTripFailure::invalid(format_args!(
                    "{} XML attribute uses an unbound namespace prefix",
                    state.side
                )));
            }
        }
        self.validate_attribute_expanded_names(state.side, element)?;
        state.set_namespace_charges(self.buffer.capacity(), self.bindings.capacity())?;
        Ok(frame)
    }

    fn validate_attribute_expanded_names(
        &self,
        side: InputSide,
        element: &BytesStart<'_>,
    ) -> Result<(), RoundTripFailure> {
        for (index, attribute) in element_attributes(side, element).enumerate() {
            let attribute = attribute?;
            if namespace_declaration_prefix(attribute.name).is_some() {
                continue;
            }
            let (prefix, local) = split_qualified_name(side, attribute.name)?;
            let namespace = self.attribute_namespace(side, prefix)?;
            for prior in element_attributes(side, element).take(index) {
                let prior = prior?;
                if namespace_declaration_prefix(prior.name).is_some() {
                    continue;
                }
                let (prior_prefix, prior_local) = split_qualified_name(side, prior.name)?;
                if local == prior_local
                    && namespace == self.attribute_namespace(side, prior_prefix)?
                {
                    return Err(RoundTripFailure::invalid(format_args!(
                        "{side} XML contains attributes with the same expanded name"
                    )));
                }
            }
        }
        Ok(())
    }

    fn attribute_namespace<'a>(
        &'a self,
        side: InputSide,
        prefix: &[u8],
    ) -> Result<&'a [u8], RoundTripFailure> {
        if prefix.is_empty() {
            return Ok(b"");
        }
        if prefix == b"xml" {
            return Ok(XML_NAMESPACE);
        }
        self.bindings
            .iter()
            .rev()
            .find(|binding| self.prefix(binding) == prefix)
            .map(|binding| self.value(binding))
            .ok_or_else(|| {
                RoundTripFailure::invalid(format_args!(
                    "{side} XML attribute uses an unbound namespace prefix"
                ))
            })
    }

    fn reserve(
        &mut self,
        state: &mut SurfaceStreamParser,
        additional_buffer: usize,
        additional_bindings: usize,
    ) -> Result<(), RoundTripFailure> {
        let required_buffer = self
            .buffer
            .len()
            .checked_add(additional_buffer)
            .ok_or_else(|| RoundTripFailure::resource("XML namespace storage length overflow"))?;
        let required_bindings = self
            .bindings
            .len()
            .checked_add(additional_bindings)
            .ok_or_else(|| RoundTripFailure::resource("XML namespace binding count overflow"))?;
        let old_buffer_capacity = self.buffer.capacity();
        let old_binding_capacity = self.bindings.capacity();
        let growth_buffer_charge = growth_capacity(old_buffer_capacity, required_buffer)?;
        let growth_binding_charge = growth_capacity(old_binding_capacity, required_bindings)?;
        state.observe_namespace_projection(growth_buffer_charge, old_binding_capacity)?;
        self.buffer
            .try_reserve_exact(additional_buffer)
            .map_err(|_| RoundTripFailure::resource("XML namespace storage allocation failed"))?;
        let actual_buffer_capacity = self.buffer.capacity();
        let post_buffer_charge = if required_buffer > old_buffer_capacity {
            overlapping_capacity(old_buffer_capacity, actual_buffer_capacity)
        } else {
            actual_buffer_capacity
        };
        state.observe_namespace_projection(post_buffer_charge, old_binding_capacity)?;
        state.observe_namespace_projection(actual_buffer_capacity, growth_binding_charge)?;
        self.bindings
            .try_reserve_exact(additional_bindings)
            .map_err(|_| RoundTripFailure::resource("XML namespace binding allocation failed"))?;
        let actual_binding_capacity = self.bindings.capacity();
        let post_binding_charge = if required_bindings > old_binding_capacity {
            overlapping_capacity(old_binding_capacity, actual_binding_capacity)
        } else {
            actual_binding_capacity
        };
        state.observe_namespace_projection(actual_buffer_capacity, post_binding_charge)?;
        Ok(())
    }

    fn pop(&mut self, frame: NamespaceFrame) {
        self.buffer.truncate(frame.buffer_len);
        self.bindings.truncate(frame.binding_len);
    }

    fn resolve_element<'a>(
        &self,
        side: InputSide,
        qualified_name: &'a [u8],
    ) -> Result<(StreamNamespace, &'a [u8]), RoundTripFailure> {
        let (prefix, local) = split_qualified_name(side, qualified_name)?;
        if prefix == b"xml" {
            return Ok((StreamNamespace::Other, local));
        }
        let namespace = self
            .bindings
            .iter()
            .rev()
            .find(|binding| self.prefix(binding) == prefix)
            .map_or_else(
                || {
                    if prefix.is_empty() {
                        Ok(StreamNamespace::Unbound)
                    } else {
                        Err(RoundTripFailure::invalid(format_args!(
                            "{side} XML uses an unbound namespace prefix"
                        )))
                    }
                },
                |binding| Ok(classify_namespace(self.value(binding))),
            )?;
        Ok((namespace, local))
    }

    fn prefix<'a>(&'a self, binding: &NamespaceBinding) -> &'a [u8] {
        &self.buffer[binding.prefix_start..binding.prefix_start + binding.prefix_len]
    }

    fn value<'a>(&'a self, binding: &NamespaceBinding) -> &'a [u8] {
        &self.buffer[binding.value_start..binding.value_start + binding.value_len]
    }
}

struct ElementFrame {
    kind: ElementKind,
    parser_charge: u64,
    namespace_frame: NamespaceFrame,
    qualified_name: String,
    point_id: Option<u64>,
    simple_text: String,
    nonempty_segments: u8,
    text_segment_open: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DocumentPhase {
    #[default]
    Prolog,
    Root,
    Epilog,
}

impl ElementFrame {
    fn new(kind: ElementKind, parser_charge: u64) -> Self {
        Self {
            kind,
            parser_charge,
            namespace_frame: NamespaceFrame::default(),
            qualified_name: String::new(),
            point_id: None,
            simple_text: String::new(),
            nonempty_segments: 0,
            text_segment_open: false,
        }
    }
}

struct SurfaceStreamParser {
    side: InputSide,
    limits: RoundTripLimits,
    retained_base_bytes: u64,
    stack: Vec<ElementFrame>,
    input_buffer_charge: usize,
    scan_buffer_charge: usize,
    event_buffer_charge: usize,
    lexical_stack_charge: usize,
    namespace_buffer_charge: usize,
    namespace_binding_charge: usize,
    parser_peak_bytes: u64,
    retained_peak_bytes: u64,
    xml_nodes: u64,
    xml_text_bytes: u64,
    document_phase: DocumentPhase,
    declaration_allowed: bool,
    previous_raw_carriage_return: bool,
    trailing_text_brackets: u8,
    units_count: u8,
    project_count: u8,
    application_count: u8,
    surfaces_count: u8,
    surface_count: u8,
    definition_count: u8,
    pnts_count: u8,
    faces_count: u8,
    metric_count: u8,
    points: Vec<IndexedPosition>,
    faces: Vec<Triangle>,
    surface_name: Option<String>,
    ignored_top_level_sections: Vec<String>,
}

#[derive(Clone, Copy)]
struct ParserProjection {
    event_buffer_charge: usize,
    stack_charge: usize,
    lexical_stack_charge: usize,
    simple_text_extra: usize,
    active_token_extra: u64,
}

impl SurfaceStreamParser {
    fn new(side: InputSide, limits: RoundTripLimits, retained_base_bytes: u64) -> Self {
        Self {
            side,
            limits,
            retained_base_bytes,
            stack: Vec::new(),
            input_buffer_charge: PARSER_READ_BUFFER_BYTES,
            scan_buffer_charge: PARSER_READ_BUFFER_BYTES,
            event_buffer_charge: 0,
            lexical_stack_charge: 0,
            namespace_buffer_charge: 0,
            namespace_binding_charge: 0,
            parser_peak_bytes: 0,
            retained_peak_bytes: retained_base_bytes,
            xml_nodes: 0,
            xml_text_bytes: 0,
            document_phase: DocumentPhase::Prolog,
            declaration_allowed: true,
            previous_raw_carriage_return: false,
            trailing_text_brackets: 0,
            units_count: 0,
            project_count: 0,
            application_count: 0,
            surfaces_count: 0,
            surface_count: 0,
            definition_count: 0,
            pnts_count: 0,
            faces_count: 0,
            metric_count: 0,
            points: Vec::new(),
            faces: Vec::new(),
            surface_name: None,
            ignored_top_level_sections: Vec::new(),
        }
    }

    fn observe_event_buffer(&mut self, charge: usize) -> Result<(), RoundTripFailure> {
        self.event_buffer_charge = charge;
        self.observe_working()
    }

    fn observe_working(&mut self) -> Result<(), RoundTripFailure> {
        self.observe_parser_projection(ParserProjection {
            event_buffer_charge: self.event_buffer_charge,
            stack_charge: self.stack.capacity(),
            lexical_stack_charge: self.lexical_stack_charge,
            simple_text_extra: 0,
            active_token_extra: 0,
        })
    }

    fn observe_parser_projection(
        &mut self,
        projection: ParserProjection,
    ) -> Result<(), RoundTripFailure> {
        let required = self.projected_parser_bytes(projection);
        self.parser_peak_bytes = self.parser_peak_bytes.max(required);
        check_working_limit(
            format_args!("{} XML parser", self.side),
            required,
            self.limits.parser_working_bytes,
        )
    }

    fn observe_external_parser_peak(&mut self, required: u64) -> Result<(), RoundTripFailure> {
        self.parser_peak_bytes = self.parser_peak_bytes.max(required);
        check_working_limit(
            format_args!("{} XML parser", self.side),
            required,
            self.limits.parser_working_bytes,
        )
    }

    fn parser_allocation_bytes(&self) -> u64 {
        capacity_bytes::<ElementFrame>(self.stack.capacity())
            .saturating_add(
                self.stack
                    .iter()
                    .map(|frame| {
                        (frame.simple_text.capacity() as u64)
                            .saturating_add(frame.qualified_name.capacity() as u64)
                    })
                    .sum::<u64>(),
            )
            .saturating_add(self.namespace_buffer_charge as u64)
            .saturating_add(capacity_bytes::<NamespaceBinding>(
                self.namespace_binding_charge,
            ))
    }

    fn projected_parser_bytes(&self, projection: ParserProjection) -> u64 {
        self.projected_parser_bytes_with_namespaces(
            projection,
            self.namespace_buffer_charge,
            self.namespace_binding_charge,
        )
    }

    fn projected_parser_bytes_with_namespaces(
        &self,
        projection: ParserProjection,
        namespace_buffer_charge: usize,
        namespace_binding_charge: usize,
    ) -> u64 {
        let simple_text_bytes = self
            .stack
            .iter()
            .map(|frame| {
                (frame.simple_text.capacity() as u64)
                    .saturating_add(frame.qualified_name.capacity() as u64)
            })
            .sum::<u64>()
            .saturating_add(projection.simple_text_extra as u64);
        let active_token_bytes = self
            .stack
            .iter()
            .map(|frame| frame.parser_charge)
            .sum::<u64>()
            .saturating_add(projection.active_token_extra);
        (self.input_buffer_charge as u64)
            .saturating_add(projection.event_buffer_charge as u64)
            .saturating_add(self.limits.xml_token_bytes)
            .saturating_add(size_of::<ElementFrame>() as u64)
            .saturating_add(self.scan_buffer_charge as u64)
            .saturating_add(
                (projection.lexical_stack_charge as u64).saturating_mul(size_of::<u64>() as u64),
            )
            .saturating_add(
                (projection.stack_charge as u64).saturating_mul(size_of::<ElementFrame>() as u64),
            )
            .saturating_add(namespace_buffer_charge as u64)
            .saturating_add(capacity_bytes::<NamespaceBinding>(namespace_binding_charge))
            .saturating_add(simple_text_bytes)
            .saturating_add(active_token_bytes)
    }

    fn observe_namespace_projection(
        &mut self,
        namespace_buffer_charge: usize,
        namespace_binding_charge: usize,
    ) -> Result<(), RoundTripFailure> {
        let required = self.projected_parser_bytes_with_namespaces(
            ParserProjection {
                event_buffer_charge: self.event_buffer_charge,
                stack_charge: self.stack.capacity(),
                lexical_stack_charge: self.lexical_stack_charge,
                simple_text_extra: 0,
                active_token_extra: 0,
            },
            namespace_buffer_charge,
            namespace_binding_charge,
        );
        self.parser_peak_bytes = self.parser_peak_bytes.max(required);
        check_working_limit(
            format_args!("{} XML parser", self.side),
            required,
            self.limits.parser_working_bytes,
        )
    }

    fn set_namespace_charges(
        &mut self,
        namespace_buffer_charge: usize,
        namespace_binding_charge: usize,
    ) -> Result<(), RoundTripFailure> {
        self.namespace_buffer_charge = namespace_buffer_charge;
        self.namespace_binding_charge = namespace_binding_charge;
        self.observe_working()
    }

    fn observe_retained(&mut self) -> Result<(), RoundTripFailure> {
        self.observe_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            0,
        )
    }

    fn observe_retained_projection(
        &mut self,
        points_charge: usize,
        faces_charge: usize,
        ignored_charge: usize,
        surface_name_charge: usize,
        transient_extra: u64,
    ) -> Result<(), RoundTripFailure> {
        let required = self.projected_retained_bytes(
            points_charge,
            faces_charge,
            ignored_charge,
            surface_name_charge,
            transient_extra,
        );
        self.retained_peak_bytes = self.retained_peak_bytes.max(required);
        check_working_limit(
            "retained round-trip data",
            required,
            self.limits.retained_working_bytes,
        )
    }

    fn projected_retained_bytes(
        &self,
        points_charge: usize,
        faces_charge: usize,
        ignored_charge: usize,
        surface_name_charge: usize,
        transient_extra: u64,
    ) -> u64 {
        self.retained_base_bytes
            .saturating_add(
                (points_charge as u64).saturating_mul(size_of::<IndexedPosition>() as u64),
            )
            .saturating_add((faces_charge as u64).saturating_mul(size_of::<Triangle>() as u64))
            .saturating_add((ignored_charge as u64).saturating_mul(size_of::<String>() as u64))
            .saturating_add(surface_name_charge as u64)
            .saturating_add(transient_extra)
            .saturating_add(
                self.ignored_top_level_sections
                    .iter()
                    .map(|section| section.capacity() as u64)
                    .sum::<u64>(),
            )
    }

    fn check_retained_projection(
        &mut self,
        points_charge: usize,
        faces_charge: usize,
        ignored_charge: usize,
        surface_name_charge: usize,
        transient_extra: u64,
    ) -> Result<(), RoundTripFailure> {
        self.observe_retained_projection(
            points_charge,
            faces_charge,
            ignored_charge,
            surface_name_charge,
            transient_extra,
        )
    }

    fn count_node(&mut self) -> Result<(), RoundTripFailure> {
        self.xml_nodes = self.xml_nodes.saturating_add(1);
        if self.xml_nodes > self.limits.xml_nodes {
            return Err(RoundTripFailure::resource(format_args!(
                "{} XML nodes exceed the {} node limit",
                self.side, self.limits.xml_nodes
            )));
        }
        Ok(())
    }

    fn add_text_bytes(&mut self, additional: usize) -> Result<(), RoundTripFailure> {
        self.xml_text_bytes = add_text_bytes(
            self.side,
            self.xml_text_bytes,
            additional,
            self.limits.xml_text_bytes,
        )?;
        Ok(())
    }

    fn start(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
        element: &BytesStart<'_>,
        namespace_frame: NamespaceFrame,
    ) -> Result<(), RoundTripFailure> {
        self.count_node()?;
        if namespace == StreamNamespace::XInclude {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} must not contain XInclude elements",
                self.side
            )));
        }
        let parent = self.stack.last().map(|frame| frame.kind);
        let kind = self.classify_start(parent, namespace, local, element)?;
        let attributes = element.attributes_raw().len();
        self.add_text_bytes(attributes)?;
        validate_attributes(self.side, element)?;
        let projected_stack_charge = projected_len(&self.stack, 1)?;
        let old_stack_capacity = self.stack.capacity();
        let growth_stack_charge = growth_capacity(old_stack_capacity, projected_stack_charge)?;
        self.observe_parser_projection(ParserProjection {
            event_buffer_charge: self.event_buffer_charge,
            stack_charge: growth_stack_charge,
            lexical_stack_charge: self.lexical_stack_charge,
            simple_text_extra: 0,
            active_token_extra: element.as_ref().len() as u64,
        })?;
        self.stack.try_reserve_exact(1).map_err(|_| {
            RoundTripFailure::resource(format_args!("{} parser stack allocation failed", self.side))
        })?;
        let actual_stack_capacity = self.stack.capacity();
        let post_reserve_stack_charge = if projected_stack_charge > old_stack_capacity {
            overlapping_capacity(old_stack_capacity, actual_stack_capacity)
        } else {
            actual_stack_capacity
        };
        self.observe_parser_projection(ParserProjection {
            event_buffer_charge: self.event_buffer_charge,
            stack_charge: post_reserve_stack_charge,
            lexical_stack_charge: self.lexical_stack_charge,
            simple_text_extra: 0,
            active_token_extra: element.as_ref().len() as u64,
        })?;
        let mut frame = ElementFrame::new(kind, element.as_ref().len() as u64);
        frame.namespace_frame = namespace_frame;
        let element_name = element.name();
        let qualified_name = std::str::from_utf8(element_name.as_ref()).map_err(|error| {
            RoundTripFailure::invalid(format_args!("{} is not UTF-8 XML: {error}", self.side))
        })?;
        self.observe_parser_projection(ParserProjection {
            event_buffer_charge: self.event_buffer_charge,
            stack_charge: self.stack.capacity(),
            lexical_stack_charge: self.lexical_stack_charge,
            simple_text_extra: qualified_name.len(),
            active_token_extra: element.as_ref().len() as u64,
        })?;
        frame
            .qualified_name
            .try_reserve_exact(qualified_name.len())
            .map_err(|_| {
                RoundTripFailure::resource(format_args!(
                    "{} element-name allocation failed",
                    self.side
                ))
            })?;
        self.observe_parser_projection(ParserProjection {
            event_buffer_charge: self.event_buffer_charge,
            stack_charge: self.stack.capacity(),
            lexical_stack_charge: self.lexical_stack_charge,
            simple_text_extra: frame.qualified_name.capacity(),
            active_token_extra: element.as_ref().len() as u64,
        })?;
        frame.qualified_name.push_str(qualified_name);
        if kind == ElementKind::Point {
            let value = raw_attribute_value(self.side, element, b"id")?
                .ok_or_else(|| schema_error(self.side, "every P requires an id"))?;
            let id = parse_normalized_u64(&value)?.ok_or_else(|| {
                RoundTripFailure::invalid(format_args!(
                    "{} point ID must be a positive integer",
                    self.side
                ))
            })?;
            if id == 0 {
                return Err(RoundTripFailure::invalid(format_args!(
                    "{} point ID must be positive",
                    self.side
                )));
            }
            frame.point_id = Some(id);
        }
        self.stack.push(frame);
        self.observe_working()
    }

    fn classify_start(
        &mut self,
        parent: Option<ElementKind>,
        namespace: StreamNamespace,
        local: &[u8],
        element: &BytesStart<'_>,
    ) -> Result<ElementKind, RoundTripFailure> {
        match parent {
            None => self.start_root(namespace, local, element),
            Some(ElementKind::LandXml) => self.start_root_child(namespace, local),
            Some(ElementKind::Units) => self.start_metric(namespace, local, element),
            Some(ElementKind::Metric) => Err(unit_drift(self.side)),
            Some(ElementKind::Project | ElementKind::Application | ElementKind::Ignored) => {
                Ok(ElementKind::Ignored)
            }
            Some(ElementKind::Surfaces) => self.start_surface(namespace, local, element),
            Some(ElementKind::Surface) => self.start_definition(namespace, local, element),
            Some(ElementKind::Definition) => self.start_definition_child(namespace, local),
            Some(ElementKind::Pnts) => {
                require_stream_tag(self.side, namespace, local, b"P")?;
                Ok(ElementKind::Point)
            }
            Some(ElementKind::Faces) => {
                require_stream_tag(self.side, namespace, local, b"F")?;
                Ok(ElementKind::Face)
            }
            Some(ElementKind::Point | ElementKind::Face) => Err(schema_error(
                self.side,
                "simple XML content cannot contain child markup",
            )),
        }
    }

    fn start_root(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
        element: &BytesStart<'_>,
    ) -> Result<ElementKind, RoundTripFailure> {
        if self.document_phase != DocumentPhase::Prolog {
            return Err(schema_error(self.side, "XML must contain exactly one root"));
        }
        require_stream_tag(self.side, namespace, local, b"LandXML")?;
        if !normalized_attribute_matches(
            raw_attribute_value(self.side, element, b"version")?,
            "1.2",
        )? {
            return Err(schema_error(self.side, "LandXML version must be 1.2"));
        }
        self.document_phase = DocumentPhase::Root;
        Ok(ElementKind::LandXml)
    }

    fn start_root_child(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
    ) -> Result<ElementKind, RoundTripFailure> {
        require_landxml_namespace(self.side, namespace)?;
        match local {
            b"Units" => {
                self.units_count = self.units_count.saturating_add(1);
                Ok(ElementKind::Units)
            }
            b"Project" => {
                self.project_count = self.project_count.saturating_add(1);
                if self.project_count > 1 {
                    return Err(schema_error(
                        self.side,
                        "LandXML permits at most one Project",
                    ));
                }
                self.push_ignored_section("Project")?;
                Ok(ElementKind::Project)
            }
            b"Application" => {
                self.application_count = self.application_count.saturating_add(1);
                if self.application_count > 1 {
                    return Err(schema_error(
                        self.side,
                        "LandXML permits at most one Application",
                    ));
                }
                self.push_ignored_section("Application")?;
                Ok(ElementKind::Application)
            }
            b"Surfaces" => {
                self.surfaces_count = self.surfaces_count.saturating_add(1);
                if self.surfaces_count > 1 {
                    return Err(schema_error(self.side, "LandXML requires one Surfaces"));
                }
                Ok(ElementKind::Surfaces)
            }
            _ => Err(schema_error(
                self.side,
                "LandXML has an unsupported child element",
            )),
        }
    }

    fn start_metric(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
        element: &BytesStart<'_>,
    ) -> Result<ElementKind, RoundTripFailure> {
        if namespace != StreamNamespace::LandXml || local != b"Metric" {
            return Err(unit_drift(self.side));
        }
        self.metric_count = self.metric_count.saturating_add(1);
        if self.metric_count > 1
            || !normalized_attribute_matches(
                raw_attribute_value(self.side, element, b"linearUnit")?,
                "meter",
            )?
        {
            return Err(unit_drift(self.side));
        }
        Ok(ElementKind::Metric)
    }

    fn start_surface(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
        element: &BytesStart<'_>,
    ) -> Result<ElementKind, RoundTripFailure> {
        require_stream_tag(self.side, namespace, local, b"Surface")?;
        self.surface_count = self.surface_count.saturating_add(1);
        if self.surface_count > 1 {
            return Err(schema_error(
                self.side,
                "Surfaces requires exactly one Surface",
            ));
        }
        let name = raw_attribute_value(self.side, element, b"name")?
            .ok_or_else(|| schema_error(self.side, "Surface requires a name attribute"))?;
        let normalized_name_len = normalized_attribute_len(&name)?;
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            normalized_name_len,
            0,
        )?;
        let mut owned_name = String::new();
        owned_name
            .try_reserve_exact(normalized_name_len)
            .map_err(|_| {
                RoundTripFailure::resource(format_args!(
                    "{} Surface name allocation failed",
                    self.side
                ))
            })?;
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            owned_name.capacity(),
            0,
        )?;
        append_normalized_attribute(&name, &mut owned_name)?;
        self.surface_name = Some(owned_name);
        self.observe_retained()?;
        Ok(ElementKind::Surface)
    }

    fn start_definition(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
        element: &BytesStart<'_>,
    ) -> Result<ElementKind, RoundTripFailure> {
        require_stream_tag(self.side, namespace, local, b"Definition")?;
        self.definition_count = self.definition_count.saturating_add(1);
        if self.definition_count > 1
            || !normalized_attribute_matches(
                raw_attribute_value(self.side, element, b"surfType")?,
                "TIN",
            )?
        {
            return Err(schema_error(self.side, "Surface Definition must be a TIN"));
        }
        Ok(ElementKind::Definition)
    }

    fn start_definition_child(
        &mut self,
        namespace: StreamNamespace,
        local: &[u8],
    ) -> Result<ElementKind, RoundTripFailure> {
        require_landxml_namespace(self.side, namespace)?;
        match local {
            b"Pnts" => {
                self.pnts_count = self.pnts_count.saturating_add(1);
                if self.pnts_count > 1 {
                    return Err(schema_error(self.side, "Definition requires one Pnts"));
                }
                Ok(ElementKind::Pnts)
            }
            b"Faces" => {
                self.faces_count = self.faces_count.saturating_add(1);
                if self.faces_count > 1 {
                    return Err(schema_error(self.side, "Definition requires one Faces"));
                }
                Ok(ElementKind::Faces)
            }
            _ => Err(schema_error(
                self.side,
                "Definition has an unsupported child element",
            )),
        }
    }

    fn push_ignored_section(&mut self, name: &'static str) -> Result<(), RoundTripFailure> {
        let ignored_charge = projected_len(&self.ignored_top_level_sections, 1)?;
        let old_ignored_capacity = self.ignored_top_level_sections.capacity();
        let growth_ignored_charge = growth_capacity(old_ignored_capacity, ignored_charge)?;
        let surface_name_charge = self.surface_name.as_ref().map_or(0, String::capacity);
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            growth_ignored_charge,
            surface_name_charge,
            name.len() as u64,
        )?;
        let mut owned = String::new();
        owned.try_reserve_exact(name.len()).map_err(|_| {
            RoundTripFailure::resource(format_args!(
                "{} ignored-section name allocation failed",
                self.side
            ))
        })?;
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            growth_ignored_charge,
            surface_name_charge,
            owned.capacity() as u64,
        )?;
        owned.push_str(name);
        self.ignored_top_level_sections
            .try_reserve_exact(1)
            .map_err(|_| {
                RoundTripFailure::resource(format_args!(
                    "{} ignored-section storage allocation failed",
                    self.side
                ))
            })?;
        let actual_ignored_capacity = self.ignored_top_level_sections.capacity();
        let post_reserve_ignored_charge = if ignored_charge > old_ignored_capacity {
            overlapping_capacity(old_ignored_capacity, actual_ignored_capacity)
        } else {
            actual_ignored_capacity
        };
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            post_reserve_ignored_charge,
            surface_name_charge,
            owned.capacity() as u64,
        )?;
        self.ignored_top_level_sections.push(owned);
        self.observe_retained()
    }

    fn note_document_content(&mut self) {
        self.declaration_allowed = false;
    }

    fn xml_declaration(&mut self, instruction: &str) -> Result<(), RoundTripFailure> {
        if !self.declaration_allowed {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} XML declaration must appear once at the start of the document",
                self.side
            )));
        }
        validate_xml_declaration(self.side, instruction)?;
        self.declaration_allowed = false;
        Ok(())
    }

    fn raw_text_boundary(&mut self) {
        self.previous_raw_carriage_return = false;
        self.trailing_text_brackets = 0;
    }

    fn text(&mut self, text: &str) -> Result<(), RoundTripFailure> {
        self.raw_text_boundary();
        self.text_with_xml10_eol_normalization(text, false)
    }

    fn xml10_text(&mut self, text: &str) -> Result<(), RoundTripFailure> {
        validate_raw_xml_characters(self.side, text)?;
        for byte in text.bytes() {
            match byte {
                b']' => self.trailing_text_brackets = (self.trailing_text_brackets + 1).min(2),
                b'>' if self.trailing_text_brackets == 2 => {
                    return Err(RoundTripFailure::invalid(format_args!(
                        "{} XML character data contains the forbidden ]]> sequence",
                        self.side
                    )));
                }
                _ => self.trailing_text_brackets = 0,
            }
        }
        let text = if self.previous_raw_carriage_return {
            text.strip_prefix('\n').unwrap_or(text)
        } else {
            text
        };
        self.previous_raw_carriage_return = text.ends_with('\r');
        self.text_with_xml10_eol_normalization(text, true)
    }

    fn xml10_cdata(&mut self, text: &str) -> Result<(), RoundTripFailure> {
        if self.stack.is_empty() {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} XML CDATA is outside the root element",
                self.side
            )));
        }
        validate_raw_xml_characters(self.side, text)?;
        self.raw_text_boundary();
        self.text_with_xml10_eol_normalization(text, true)
    }

    fn text_with_xml10_eol_normalization(
        &mut self,
        text: &str,
        normalize_eols: bool,
    ) -> Result<(), RoundTripFailure> {
        self.count_node()?;
        let text_len = if normalize_eols {
            xml10_normalized_text_len(text)
        } else {
            text.len()
        };
        self.add_text_bytes(text_len)?;
        let Some(frame_index) = self.stack.len().checked_sub(1) else {
            if text.trim().is_empty() {
                return Ok(());
            }
            return Err(schema_error(self.side, "text is outside the root element"));
        };
        match self.stack[frame_index].kind {
            ElementKind::Point | ElementKind::Face => {
                let stack_capacity = self.stack.capacity();
                let frame = &mut self.stack[frame_index];
                if !text.trim().is_empty() {
                    if !frame.text_segment_open {
                        frame.nonempty_segments = frame.nonempty_segments.saturating_add(1);
                        if frame.nonempty_segments > 1 {
                            return Err(schema_error(
                                self.side,
                                "simple XML content must be contiguous",
                            ));
                        }
                    }
                    frame.text_segment_open = true;
                }
                let projected_text_charge = projected_string_len(&frame.simple_text, text_len)?;
                let old_text_capacity = frame.simple_text.capacity();
                let charge_extra = if projected_text_charge > old_text_capacity {
                    projected_text_charge
                } else {
                    0
                };
                let projection = ParserProjection {
                    event_buffer_charge: self.event_buffer_charge,
                    stack_charge: stack_capacity,
                    lexical_stack_charge: self.lexical_stack_charge,
                    simple_text_extra: charge_extra,
                    active_token_extra: 0,
                };
                let _ = frame;
                self.observe_parser_projection(projection)?;
                let frame = &mut self.stack[frame_index];
                frame.simple_text.try_reserve_exact(text_len).map_err(|_| {
                    RoundTripFailure::resource(format_args!(
                        "{} simple-text allocation failed",
                        self.side
                    ))
                })?;
                let actual_text_capacity = frame.simple_text.capacity();
                let post_reserve_extra = if projected_text_charge > old_text_capacity {
                    old_text_capacity
                } else {
                    0
                };
                let _ = frame;
                self.observe_parser_projection(ParserProjection {
                    event_buffer_charge: self.event_buffer_charge,
                    stack_charge: self.stack.capacity(),
                    lexical_stack_charge: self.lexical_stack_charge,
                    simple_text_extra: post_reserve_extra,
                    active_token_extra: 0,
                })?;
                debug_assert!(actual_text_capacity >= projected_text_charge);
                let frame = &mut self.stack[frame_index];
                if normalize_eols {
                    append_xml10_normalized_text(text, &mut frame.simple_text);
                } else {
                    frame.simple_text.push_str(text);
                }
                self.observe_working()
            }
            ElementKind::Project | ElementKind::Application | ElementKind::Ignored => Ok(()),
            ElementKind::Metric if !text.trim().is_empty() => Err(unit_drift(self.side)),
            _ if text.trim().is_empty() => Ok(()),
            _ => Err(schema_error(
                self.side,
                "container has unexpected text content",
            )),
        }
    }

    fn markup_boundary(&mut self, processing_instruction: bool) -> Result<(), RoundTripFailure> {
        self.raw_text_boundary();
        self.count_node()?;
        if let Some(frame) = self.stack.last_mut()
            && matches!(frame.kind, ElementKind::Point | ElementKind::Face)
        {
            if processing_instruction {
                return Err(schema_error(
                    self.side,
                    "simple XML content cannot contain child markup",
                ));
            }
            frame.text_segment_open = false;
        }
        Ok(())
    }

    fn end(&mut self) -> Result<NamespaceFrame, RoundTripFailure> {
        let frame = self
            .stack
            .pop()
            .ok_or_else(|| schema_error(self.side, "unexpected closing element"))?;
        self.observe_parser_projection(ParserProjection {
            event_buffer_charge: self.event_buffer_charge,
            stack_charge: self.stack.capacity(),
            lexical_stack_charge: self.lexical_stack_charge,
            simple_text_extra: frame
                .simple_text
                .capacity()
                .saturating_add(frame.qualified_name.capacity()),
            active_token_extra: frame.parser_charge,
        })?;
        match frame.kind {
            ElementKind::Point => self.finish_point(&frame)?,
            ElementKind::Face => self.finish_face(&frame)?,
            ElementKind::Units if self.metric_count != 1 => return Err(unit_drift(self.side)),
            ElementKind::Surfaces if self.surface_count != 1 => {
                return Err(schema_error(
                    self.side,
                    "Surfaces requires exactly one Surface",
                ));
            }
            ElementKind::Surface if self.definition_count != 1 => {
                return Err(schema_error(
                    self.side,
                    "Surface requires exactly one Definition",
                ));
            }
            ElementKind::Definition if self.pnts_count != 1 || self.faces_count != 1 => {
                return Err(schema_error(
                    self.side,
                    "Definition requires exactly one Pnts and one Faces",
                ));
            }
            ElementKind::LandXml => {
                if self.units_count != 1 {
                    return Err(unit_drift(self.side));
                }
                if self.surfaces_count != 1 {
                    return Err(schema_error(
                        self.side,
                        "LandXML requires exactly one Surfaces",
                    ));
                }
                self.document_phase = DocumentPhase::Epilog;
            }
            ElementKind::Metric
            | ElementKind::Units
            | ElementKind::Surfaces
            | ElementKind::Surface
            | ElementKind::Definition
            | ElementKind::Project
            | ElementKind::Application
            | ElementKind::Pnts
            | ElementKind::Faces
            | ElementKind::Ignored => {}
        }
        let namespace_frame = frame.namespace_frame;
        drop(frame);
        self.observe_working()?;
        Ok(namespace_frame)
    }

    fn finish_point(&mut self, frame: &ElementFrame) -> Result<(), RoundTripFailure> {
        if frame.nonempty_segments == 0 {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} P requires text content",
                self.side
            )));
        }
        let position = parse_position_text(self.side, frame.simple_text.trim())?;
        check_next_item_limit(self.side, "points", self.points.len(), self.limits.points)?;
        let points_charge = projected_len(&self.points, 1)?;
        let old_points_capacity = self.points.capacity();
        let growth_points_charge = growth_capacity(old_points_capacity, points_charge)?;
        self.check_retained_projection(
            growth_points_charge,
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            0,
        )?;
        self.points.try_reserve_exact(1).map_err(|_| {
            RoundTripFailure::resource(format_args!(
                "{} point storage allocation failed",
                self.side
            ))
        })?;
        let actual_points_capacity = self.points.capacity();
        let post_reserve_points_charge = if points_charge > old_points_capacity {
            overlapping_capacity(old_points_capacity, actual_points_capacity)
        } else {
            actual_points_capacity
        };
        self.check_retained_projection(
            post_reserve_points_charge,
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            0,
        )?;
        self.points.push(IndexedPosition {
            id: frame.point_id.expect("P id is validated at start"),
            position,
        });
        self.observe_retained()
    }

    fn finish_face(&mut self, frame: &ElementFrame) -> Result<(), RoundTripFailure> {
        if frame.nonempty_segments == 0 {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} F requires text content",
                self.side
            )));
        }
        let face = parse_face_text(self.side, frame.simple_text.trim())?;
        if face.has_repeated_point() {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} contains a face with repeated point references",
                self.side
            )));
        }
        check_next_item_limit(self.side, "faces", self.faces.len(), self.limits.faces)?;
        let faces_charge = projected_len(&self.faces, 1)?;
        let old_faces_capacity = self.faces.capacity();
        let growth_faces_charge = growth_capacity(old_faces_capacity, faces_charge)?;
        self.check_retained_projection(
            self.points.capacity(),
            growth_faces_charge,
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            0,
        )?;
        self.faces.try_reserve_exact(1).map_err(|_| {
            RoundTripFailure::resource(format_args!("{} face storage allocation failed", self.side))
        })?;
        let actual_faces_capacity = self.faces.capacity();
        let post_reserve_faces_charge = if faces_charge > old_faces_capacity {
            overlapping_capacity(old_faces_capacity, actual_faces_capacity)
        } else {
            actual_faces_capacity
        };
        self.check_retained_projection(
            self.points.capacity(),
            post_reserve_faces_charge,
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            0,
        )?;
        self.faces.push(face);
        self.observe_retained()
    }

    fn finish(&mut self) -> Result<ParsedSurface, RoundTripFailure> {
        self.validate_complete_surface()?;
        let positions = self.finish_positions()?;
        let position_bytes = collection_bytes::<Position>(&positions);
        self.resolve_and_validate_faces(&positions, position_bytes)?;
        self.assemble_surface(positions)
    }

    fn validate_complete_surface(&self) -> Result<(), RoundTripFailure> {
        if self.document_phase != DocumentPhase::Epilog || !self.stack.is_empty() {
            return Err(schema_error(self.side, "XML document is incomplete"));
        }
        if self.points.len() < 3 || self.faces.is_empty() {
            return Err(schema_error(
                self.side,
                "TIN requires at least three points and one face",
            ));
        }
        Ok(())
    }

    fn finish_positions(&mut self) -> Result<Vec<Position>, RoundTripFailure> {
        self.points.sort_unstable_by_key(|point| point.id);
        if let Some(duplicate) = self.points.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} contains duplicate point ID {}",
                self.side, duplicate[0].id
            )));
        }
        let mut positions = Vec::new();
        let projected_positions_bytes =
            (self.points.len() as u64).saturating_mul(size_of::<Position>() as u64);
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            projected_positions_bytes,
        )?;
        positions
            .try_reserve_exact(self.points.len())
            .map_err(|_| {
                RoundTripFailure::resource(format_args!(
                    "{} point storage allocation failed",
                    self.side
                ))
            })?;
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            collection_bytes::<Position>(&positions),
        )?;
        for indexed in &self.points {
            positions.push(indexed.position);
        }
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            collection_bytes::<Position>(&positions),
        )?;
        Ok(positions)
    }

    fn resolve_and_validate_faces(
        &mut self,
        positions: &[Position],
        position_bytes: u64,
    ) -> Result<(), RoundTripFailure> {
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            position_bytes,
        )?;
        for face in &mut self.faces {
            face.first = resolve_point_id(self.side, &self.points, face.first)?;
            face.second = resolve_point_id(self.side, &self.points, face.second)?;
            face.third = resolve_point_id(self.side, &self.points, face.third)?;
            validate_face(self.side, *face, positions)?;
        }
        self.faces
            .sort_unstable_by_key(|face| face.canonical_point_indices());
        if self
            .faces
            .windows(2)
            .any(|pair| pair[0].canonical_point_indices() == pair[1].canonical_point_indices())
        {
            return Err(RoundTripFailure::mismatch(
                RoundTripReasonCode::TopologyDrift,
                format_args!("{} contains a duplicate face", self.side),
            ));
        }
        Ok(())
    }

    fn assemble_surface(
        &mut self,
        positions: Vec<Position>,
    ) -> Result<ParsedSurface, RoundTripFailure> {
        self.ignored_top_level_sections.sort_unstable();
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            self.surface_name.as_ref().map_or(0, String::capacity),
            collection_bytes::<Position>(&positions),
        )?;
        let surface_name = self.surface_name.take().expect("validated Surface name");
        let ignored_top_level_sections = std::mem::take(&mut self.ignored_top_level_sections);
        let faces = std::mem::take(&mut self.faces);
        let surface = ParsedSurface {
            points: positions,
            faces,
            surface_name,
            ignored_top_level_sections,
        };
        self.check_retained_projection(
            self.points.capacity(),
            self.faces.capacity(),
            self.ignored_top_level_sections.capacity(),
            0,
            surface.retained_bytes(),
        )?;
        Ok(surface)
    }
}

fn parse_surface(
    side: InputSide,
    file: &mut File,
    limits: RoundTripLimits,
    retained_base_bytes: u64,
    expected_bytes: u64,
) -> Result<ParsedInput, ParseFailure> {
    let mut state = SurfaceStreamParser::new(side, limits, retained_base_bytes);
    let result =
        parse_surface_stream(file, &mut state, expected_bytes).and_then(|()| state.finish());
    match result {
        Ok(surface) => Ok(ParsedInput {
            surface,
            parser_peak_bytes: state.parser_peak_bytes,
            retained_peak_bytes: state.retained_peak_bytes,
        }),
        Err(failure) => Err(ParseFailure {
            failure,
            parser_peak_bytes: state.parser_peak_bytes,
            retained_peak_bytes: state.retained_peak_bytes,
        }),
    }
}

fn parse_surface_stream(
    file: &mut File,
    state: &mut SurfaceStreamParser,
    expected_bytes: u64,
) -> Result<(), RoundTripFailure> {
    let parser_floor = (PARSER_READ_BUFFER_BYTES as u64)
        .saturating_add(PARSER_READ_BUFFER_BYTES as u64)
        .saturating_add(state.limits.xml_token_bytes.saturating_mul(2))
        .saturating_add(size_of::<ElementFrame>() as u64);
    check_working_limit(
        format_args!("{} XML parser", state.side),
        parser_floor,
        state.limits.parser_working_bytes,
    )?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{} cannot be rewound: {error}", state.side))
    })?;
    let input = FallibleBufReader::new(file, state.side, PARSER_READ_BUFFER_BYTES)?;
    state.input_buffer_charge = input.capacity();
    state.observe_working()?;
    let mut input = BoundedXmlReader::new(
        input,
        state.input_buffer_charge,
        state.side,
        state.limits,
        expected_bytes,
    )?;
    state.scan_buffer_charge = input.scan_scratch.capacity();
    let initial_capacity = usize::try_from(state.limits.xml_token_bytes).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{} XML token limit does not fit this platform",
            state.side
        ))
    })?;
    let mut buffer = Vec::new();
    state.observe_event_buffer(initial_capacity)?;
    buffer.try_reserve_exact(initial_capacity).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{} XML event buffer cannot reserve {initial_capacity} bytes",
            state.side
        ))
    })?;
    state.observe_event_buffer(buffer.capacity())?;
    let mut namespaces = NamespaceState::new();
    let mut first_segment = true;
    loop {
        input.set_parser_allocation_charge(state.parser_allocation_bytes());
        let (segment_len, segment_complete) = {
            let (available, ends_at_boundary) = match input.fill_lexical_chunk() {
                Ok(available) => available,
                Err(error) => {
                    if let Some(failure) = input.failure() {
                        let bounded_peak = input.parser_peak_bytes();
                        state.observe_external_parser_peak(bounded_peak)?;
                        return Err(failure);
                    }
                    return Err(RoundTripFailure::invalid(format_args!(
                        "{} XML cannot be read: {error}",
                        state.side
                    )));
                }
            };
            if available.is_empty() {
                break;
            }
            if matches!(available.first(), Some(b'<' | b'&')) {
                let required = buffer.len().checked_add(available.len()).ok_or_else(|| {
                    RoundTripFailure::resource("XML token storage length overflow")
                })?;
                check_xml_token(state.side, required, state.limits.xml_token_bytes)?;
                buffer.extend_from_slice(available);
                (available.len(), ends_at_boundary)
            } else {
                append_text_chunk(
                    state.side,
                    state.limits.xml_token_bytes,
                    &mut buffer,
                    available,
                )?
            }
        };
        input.consume(segment_len);
        state.lexical_stack_charge = input.open_element_charges.capacity();
        state.observe_external_parser_peak(input.parser_peak_bytes())?;
        state.observe_event_buffer(buffer.capacity())?;
        if !segment_complete {
            continue;
        }
        let text = std::str::from_utf8(&buffer).map_err(|error| {
            RoundTripFailure::invalid(format_args!("{} is not UTF-8 XML: {error}", state.side))
        })?;
        let text = first_xml_segment(text, &mut first_segment);
        if !text.is_empty() {
            validate_raw_xml_characters(state.side, text)?;
            process_lexical_segment(state, &mut namespaces, text)?;
        }
        buffer.clear();
    }
    state.raw_text_boundary();
    Ok(())
}

fn append_text_chunk(
    side: InputSide,
    max_token_bytes: u64,
    buffer: &mut Vec<u8>,
    available: &[u8],
) -> Result<(usize, bool), RoundTripFailure> {
    let segment = lexical_segment(available);
    let required = buffer
        .len()
        .checked_add(segment.len())
        .ok_or_else(|| RoundTripFailure::resource("XML text segment storage length overflow"))?;
    check_xml_token(side, required, max_token_bytes)?;
    buffer.extend_from_slice(segment);
    let complete = match std::str::from_utf8(buffer) {
        Ok(_) => true,
        Err(error)
            if error.error_len().is_none()
                && buffer.len().saturating_sub(error.valid_up_to()) <= 3 =>
        {
            false
        }
        Err(error) => {
            return Err(RoundTripFailure::invalid(format_args!(
                "{side} is not UTF-8 XML: {error}"
            )));
        }
    };
    Ok((segment.len(), complete))
}

fn first_xml_segment<'a>(text: &'a str, first_segment: &mut bool) -> &'a str {
    if std::mem::replace(first_segment, false) {
        text.strip_prefix('\u{feff}').unwrap_or(text)
    } else {
        text
    }
}

fn lexical_segment(available: &[u8]) -> &[u8] {
    if matches!(available.first(), Some(b'<' | b'&')) {
        return available;
    }
    let markup = available
        .iter()
        .position(|byte| matches!(byte, b'<' | b'&'))
        .unwrap_or(available.len());
    &available[..markup]
}

fn process_lexical_segment(
    state: &mut SurfaceStreamParser,
    namespaces: &mut NamespaceState,
    segment: &str,
) -> Result<(), RoundTripFailure> {
    if let Some(reference) = segment
        .strip_prefix('&')
        .and_then(|value| value.strip_suffix(';'))
    {
        if state.stack.is_empty() {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} XML reference is outside the root element",
                state.side
            )));
        }
        state.note_document_content();
        let reference = quick_xml::events::BytesRef::new(reference);
        let mut encoded = [0_u8; 4];
        let resolved = resolve_reference(state.side, &reference, &mut encoded)?;
        return state.text(resolved);
    }
    if !segment.starts_with('<') {
        state.note_document_content();
        return state.xml10_text(segment);
    }
    state.raw_text_boundary();
    if let Some(comment) = segment
        .strip_prefix("<!--")
        .and_then(|value| value.strip_suffix("-->"))
    {
        state.note_document_content();
        if comment.contains("--") || comment.ends_with('-') {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} XML comment contains an invalid hyphen sequence",
                state.side
            )));
        }
        return state.markup_boundary(false);
    }
    if let Some(cdata) = segment
        .strip_prefix("<![CDATA[")
        .and_then(|value| value.strip_suffix("]]>"))
    {
        state.note_document_content();
        return state.xml10_cdata(cdata);
    }
    if let Some(instruction) = segment
        .strip_prefix("<?")
        .and_then(|value| value.strip_suffix("?>"))
    {
        return process_instruction(state, instruction);
    }
    state.note_document_content();
    if let Some(end) = segment
        .strip_prefix("</")
        .and_then(|value| value.strip_suffix('>'))
    {
        let qualified_name = end.trim_end_matches(char::is_whitespace);
        validate_xml_name(state.side, qualified_name)?;
        validate_end_name(state.side, state.stack.last(), qualified_name.as_bytes())?;
        let namespace_frame = state.end()?;
        namespaces.pop(namespace_frame);
        return state
            .set_namespace_charges(namespaces.buffer.capacity(), namespaces.bindings.capacity());
    }
    let raw_content = segment
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .ok_or_else(|| RoundTripFailure::invalid("XML markup is malformed"))?;
    let content = raw_content.trim_end_matches(char::is_whitespace);
    if content.ends_with('/') && !raw_content.ends_with('/') {
        return Err(RoundTripFailure::invalid(format_args!(
            "{} XML empty-element slash must immediately precede '>'",
            state.side
        )));
    }
    let (content, empty) = content.strip_suffix('/').map_or((content, false), |value| {
        (value.trim_end_matches(char::is_whitespace), true)
    });
    let name_len = content
        .as_bytes()
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(content.len());
    validate_xml_name(state.side, &content[..name_len])?;
    let element = BytesStart::from_content(content, name_len);
    let namespace_frame = namespaces.push(state, &element)?;
    let element_name = element.name();
    let (namespace, local) = namespaces.resolve_element(state.side, element_name.as_ref())?;
    state.start(namespace, local, &element, namespace_frame)?;
    if empty {
        let namespace_frame = state.end()?;
        namespaces.pop(namespace_frame);
        state
            .set_namespace_charges(namespaces.buffer.capacity(), namespaces.bindings.capacity())?;
    }
    Ok(())
}

fn process_instruction(
    state: &mut SurfaceStreamParser,
    instruction: &str,
) -> Result<(), RoundTripFailure> {
    let target_len = instruction
        .as_bytes()
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(instruction.len());
    let target = &instruction[..target_len];
    validate_xml_name(state.side, target)?;
    if target.eq_ignore_ascii_case("xml") {
        if target != "xml" {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} XML processing-instruction target is reserved",
                state.side
            )));
        }
        return state.xml_declaration(instruction);
    }
    state.note_document_content();
    state.markup_boundary(true)
}

fn validate_xml_declaration(side: InputSide, instruction: &str) -> Result<(), RoundTripFailure> {
    if !instruction
        .as_bytes()
        .get(3)
        .is_some_and(u8::is_ascii_whitespace)
    {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML declaration must begin with a version"
        )));
    }
    let declaration = BytesStart::from_content(instruction, 3);
    let mut phase = 0_u8;
    for attribute in element_attributes(side, &declaration) {
        let attribute = attribute?;
        match attribute.name {
            b"version" if phase == 0 && attribute.value == "1.0" => phase = 1,
            b"encoding" if phase == 1 && attribute.value.eq_ignore_ascii_case("UTF-8") => {
                phase = 2;
            }
            b"standalone" if matches!(phase, 1 | 2) && matches!(attribute.value, "yes" | "no") => {
                phase = 3;
            }
            _ => {
                return Err(RoundTripFailure::invalid(format_args!(
                    "{side} XML declaration must contain version 1.0 followed by optional UTF-8 encoding and yes/no standalone fields"
                )));
            }
        }
    }
    if phase == 0 {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML declaration must specify version 1.0"
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LexicalState {
    #[default]
    Text,
    MarkupStart,
    Tag,
    DeclarationStart,
    Comment,
    CData,
    ProcessingInstruction,
    Reference,
}

#[derive(Default)]
struct XmlTokenScanner {
    state: LexicalState,
    quote: Option<u8>,
    token_bytes: u64,
    prefix: [u8; 9],
    prefix_len: usize,
    trailing: [u8; 3],
    tag_last_unquoted_nonspace: Option<u8>,
}

struct BoundedXmlReader<R> {
    inner: R,
    input_buffer_charge: usize,
    side: InputSide,
    max_token_bytes: u64,
    max_parser_bytes: u64,
    parser_allocation_charge: u64,
    remaining_file_bytes: u64,
    scanner: XmlTokenScanner,
    open_element_charges: Vec<u64>,
    parser_peak_bytes: u64,
    deferred_error: Option<RoundTripFailure>,
    last_chunk_bytes: usize,
    last_chunk_ends_at_boundary: bool,
    scan_scratch: Vec<u8>,
}

impl<R: BufRead> BoundedXmlReader<R> {
    fn new(
        inner: R,
        input_buffer_charge: usize,
        side: InputSide,
        limits: RoundTripLimits,
        expected_file_bytes: u64,
    ) -> Result<Self, RoundTripFailure> {
        let open_element_charges = Vec::new();
        let preflight = (input_buffer_charge as u64)
            .saturating_add(PARSER_READ_BUFFER_BYTES as u64)
            .saturating_add(limits.xml_token_bytes.saturating_mul(2));
        check_working_limit(
            format_args!("{side} XML parser"),
            preflight,
            limits.parser_working_bytes,
        )?;
        let scan_scratch =
            fallible_zeroed_buffer(side, "XML scan scratch", PARSER_READ_BUFFER_BYTES)?;
        let mut reader = Self {
            inner,
            input_buffer_charge,
            side,
            max_token_bytes: limits.xml_token_bytes,
            max_parser_bytes: limits.parser_working_bytes,
            parser_allocation_charge: 0,
            remaining_file_bytes: expected_file_bytes,
            scanner: XmlTokenScanner::default(),
            open_element_charges,
            parser_peak_bytes: 0,
            deferred_error: None,
            last_chunk_bytes: 0,
            last_chunk_ends_at_boundary: false,
            scan_scratch,
        };
        reader.check_lexical_projection(0, 0)?;
        Ok(reader)
    }

    fn failure(&mut self) -> Option<RoundTripFailure> {
        self.deferred_error.take()
    }

    fn parser_peak_bytes(&self) -> u64 {
        self.parser_peak_bytes
    }

    fn set_parser_allocation_charge(&mut self, charge: u64) {
        self.parser_allocation_charge = charge;
    }

    fn fill_lexical_chunk(&mut self) -> io::Result<(&[u8], bool)> {
        if self.deferred_error.is_some() {
            return Err(io::Error::other(BOUNDED_XML_IO_ERROR));
        }
        if self.last_chunk_bytes > 0 {
            let ends_at_boundary = self.last_chunk_ends_at_boundary;
            let available = self.inner.fill_buf()?;
            return Ok((
                &available[..self.last_chunk_bytes.min(available.len())],
                ends_at_boundary,
            ));
        }
        let available = self.inner.fill_buf()?;
        let count = available.len().min(self.scan_scratch.len());
        self.scan_scratch[..count].copy_from_slice(&available[..count]);
        let scratch = std::mem::take(&mut self.scan_scratch);
        let result = self.scan_available(&scratch[..count]);
        self.scan_scratch = scratch;
        match result {
            Ok((exposed, ends_at_boundary)) => {
                self.last_chunk_bytes = exposed;
                self.last_chunk_ends_at_boundary = ends_at_boundary;
                let available = self.inner.fill_buf()?;
                Ok((&available[..exposed], ends_at_boundary))
            }
            Err(failure) => {
                self.deferred_error = Some(failure);
                Err(io::Error::other(BOUNDED_XML_IO_ERROR))
            }
        }
    }

    fn check_lexical_projection(
        &mut self,
        active_token_bytes: u64,
        stack_capacity_charge: usize,
    ) -> Result<(), RoundTripFailure> {
        let required = (self.input_buffer_charge as u64)
            .saturating_add(self.scan_scratch.capacity() as u64)
            .saturating_add(self.max_token_bytes.saturating_mul(2))
            .saturating_add(self.parser_allocation_charge)
            .saturating_add(active_token_bytes)
            .saturating_add((stack_capacity_charge as u64).saturating_mul(size_of::<u64>() as u64));
        self.parser_peak_bytes = self.parser_peak_bytes.max(required);
        check_working_limit(
            format_args!("{} XML parser", self.side),
            required,
            self.max_parser_bytes,
        )
    }

    fn scan_available(&mut self, available: &[u8]) -> Result<(usize, bool), RoundTripFailure> {
        if self.remaining_file_bytes == 0 {
            if available.is_empty() {
                self.scanner.finish(self.side)?;
                return Ok((0, false));
            }
            return Err(RoundTripFailure::invalid(format_args!(
                "{} changed while it was being read",
                self.side
            )));
        }
        if available.is_empty() {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} changed while it was being read",
                self.side
            )));
        }
        let allowed = usize::try_from(self.remaining_file_bytes)
            .unwrap_or(usize::MAX)
            .min(available.len());
        let mut exposed = 0;
        let mut ends_at_boundary = false;
        for byte in &available[..allowed] {
            let boundary = self.scanner.push(self.side, *byte, self.max_token_bytes)?;
            exposed += 1;
            if let Some(boundary) = boundary {
                self.apply_boundary(boundary)?;
                ends_at_boundary = true;
                break;
            }
        }
        Ok((exposed, ends_at_boundary))
    }

    fn apply_boundary(&mut self, boundary: LexicalBoundary) -> Result<(), RoundTripFailure> {
        match boundary {
            LexicalBoundary::StartElement { bytes, empty } => {
                let active = self
                    .open_element_charges
                    .iter()
                    .copied()
                    .sum::<u64>()
                    .saturating_add(bytes);
                if empty {
                    return self
                        .check_lexical_projection(active, self.open_element_charges.capacity());
                }

                let required_len = projected_len(&self.open_element_charges, 1)?;
                let old_capacity = self.open_element_charges.capacity();
                let pre_reserve_charge = growth_capacity(old_capacity, required_len)?;
                self.check_lexical_projection(active, pre_reserve_charge)?;
                self.open_element_charges
                    .try_reserve_exact(1)
                    .map_err(|_| {
                        RoundTripFailure::resource("XML lexical stack allocation failed")
                    })?;
                let actual_capacity = self.open_element_charges.capacity();
                let post_reserve_charge = if required_len > old_capacity {
                    overlapping_capacity(old_capacity, actual_capacity)
                } else {
                    actual_capacity
                };
                self.check_lexical_projection(active, post_reserve_charge)?;
                self.open_element_charges.push(bytes);
                self.check_lexical_projection(active, actual_capacity)?;
            }
            LexicalBoundary::EndElement => {
                self.open_element_charges.pop().ok_or_else(|| {
                    RoundTripFailure::invalid(format_args!(
                        "{} XML has an unmatched closing element",
                        self.side
                    ))
                })?;
                let active = self.open_element_charges.iter().copied().sum::<u64>();
                self.check_lexical_projection(active, self.open_element_charges.capacity())?;
            }
            LexicalBoundary::Other => {}
        }
        Ok(())
    }
}

impl<R: BufRead> Read for BoundedXmlReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl<R: BufRead> BufRead for BoundedXmlReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.fill_lexical_chunk().map(|(available, _)| available)
    }

    fn consume(&mut self, amount: usize) {
        let consumed = amount.min(self.last_chunk_bytes);
        self.remaining_file_bytes = self.remaining_file_bytes.saturating_sub(consumed as u64);
        self.last_chunk_bytes = self.last_chunk_bytes.saturating_sub(consumed);
        if self.last_chunk_bytes == 0 {
            self.last_chunk_ends_at_boundary = false;
        }
        self.inner.consume(consumed);
    }
}

#[derive(Clone, Copy)]
enum LexicalBoundary {
    StartElement { bytes: u64, empty: bool },
    EndElement,
    Other,
}

impl XmlTokenScanner {
    fn push(
        &mut self,
        side: InputSide,
        byte: u8,
        max_token_bytes: u64,
    ) -> Result<Option<LexicalBoundary>, RoundTripFailure> {
        let mut boundary = None;
        match self.state {
            LexicalState::Text => match byte {
                b'<' => self.start_token(LexicalState::MarkupStart),
                b'&' => self.start_token(LexicalState::Reference),
                _ => self.extend_token(side, max_token_bytes)?,
            },
            LexicalState::Reference => {
                self.extend_token(side, max_token_bytes)?;
                if byte == b';' {
                    boundary = Some(LexicalBoundary::Other);
                    self.reset_text();
                } else if byte == b'<' || byte == b'&' {
                    return Err(RoundTripFailure::invalid(format_args!(
                        "{side} XML reference is malformed"
                    )));
                }
            }
            LexicalState::MarkupStart => {
                self.extend_token(side, max_token_bytes)?;
                self.state = match byte {
                    b'!' => LexicalState::DeclarationStart,
                    b'?' => LexicalState::ProcessingInstruction,
                    _ => LexicalState::Tag,
                };
                self.remember(byte);
            }
            LexicalState::DeclarationStart => {
                self.extend_token(side, max_token_bytes)?;
                self.remember(byte);
                let prefix = &self.prefix[..self.prefix_len];
                if b"!--".starts_with(prefix) {
                    if prefix == b"!--" {
                        self.state = LexicalState::Comment;
                    }
                } else if b"![CDATA[".starts_with(prefix) {
                    if prefix == b"![CDATA[" {
                        self.state = LexicalState::CData;
                    }
                } else {
                    return Err(RoundTripFailure::invalid(format_args!(
                        "{side} must not contain a DTD or unsupported declaration"
                    )));
                }
            }
            LexicalState::Tag => {
                self.extend_token(side, max_token_bytes)?;
                if self.quote.is_none() && byte != b'>' {
                    self.remember(byte);
                    if !byte.is_ascii_whitespace() && !matches!(byte, b'\'' | b'"') {
                        self.tag_last_unquoted_nonspace = Some(byte);
                    }
                }
                match (self.quote, byte) {
                    (Some(expected), actual) if actual == expected => self.quote = None,
                    (None, actual @ (b'\'' | b'"')) => self.quote = Some(actual),
                    (None, b'>') => {
                        let token_bytes = self.token_bytes;
                        let prefix = self.tag_prefix();
                        boundary = Some(if prefix.starts_with(b"/") {
                            LexicalBoundary::EndElement
                        } else {
                            LexicalBoundary::StartElement {
                                bytes: token_bytes,
                                empty: self.tag_last_unquoted_nonspace == Some(b'/'),
                            }
                        });
                        self.reset_text();
                    }
                    _ => {}
                }
            }
            LexicalState::Comment => {
                self.extend_token(side, max_token_bytes)?;
                self.remember_trailing(byte);
                if self.trailing == *b"-->" {
                    boundary = Some(LexicalBoundary::Other);
                    self.reset_text();
                }
            }
            LexicalState::CData => {
                self.extend_token(side, max_token_bytes)?;
                self.remember_trailing(byte);
                if self.trailing == *b"]]>" {
                    boundary = Some(LexicalBoundary::Other);
                    self.reset_text();
                }
            }
            LexicalState::ProcessingInstruction => {
                self.extend_token(side, max_token_bytes)?;
                self.remember_trailing(byte);
                if self.trailing[1..] == *b"?>" {
                    boundary = Some(LexicalBoundary::Other);
                    self.reset_text();
                }
            }
        }
        Ok(boundary)
    }

    fn start_token(&mut self, state: LexicalState) {
        self.state = state;
        self.token_bytes = 1;
        self.quote = None;
        self.prefix = [0; 9];
        self.prefix_len = 0;
        self.trailing = [0; 3];
        self.tag_last_unquoted_nonspace = None;
    }

    fn extend_token(
        &mut self,
        side: InputSide,
        max_token_bytes: u64,
    ) -> Result<(), RoundTripFailure> {
        self.token_bytes = self.token_bytes.saturating_add(1);
        if self.token_bytes > max_token_bytes {
            return Err(RoundTripFailure::resource(format_args!(
                "{side} XML token bytes required at least {}; limit is {max_token_bytes}",
                self.token_bytes
            )));
        }
        Ok(())
    }

    fn reset_text(&mut self) {
        self.state = LexicalState::Text;
        self.quote = None;
        self.token_bytes = 0;
        self.prefix = [0; 9];
        self.prefix_len = 0;
        self.trailing = [0; 3];
        self.tag_last_unquoted_nonspace = None;
    }

    fn remember(&mut self, byte: u8) {
        if self.prefix_len < self.prefix.len() {
            self.prefix[self.prefix_len] = byte;
            self.prefix_len += 1;
        }
    }

    fn tag_prefix(&self) -> &[u8] {
        &self.prefix[..self.prefix_len]
    }

    fn remember_trailing(&mut self, byte: u8) {
        self.trailing.rotate_left(1);
        self.trailing[2] = byte;
    }

    fn finish(&self, side: InputSide) -> Result<(), RoundTripFailure> {
        match self.state {
            LexicalState::Text if self.quote.is_none() => Ok(()),
            LexicalState::Reference => Err(RoundTripFailure::invalid(format_args!(
                "{side} XML reference is incomplete"
            ))),
            _ => Err(RoundTripFailure::invalid(format_args!(
                "{side} XML markup is incomplete"
            ))),
        }
    }
}

const XML_NAMESPACE: &[u8] = b"http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &[u8] = b"http://www.w3.org/2000/xmlns/";

fn namespace_declaration_prefix(name: &[u8]) -> Option<&[u8]> {
    if name == b"xmlns" {
        Some(b"")
    } else {
        name.strip_prefix(b"xmlns:")
    }
}

fn validate_namespace_prefix(side: InputSide, prefix: &[u8]) -> Result<(), RoundTripFailure> {
    if prefix.is_empty() || prefix == b"xml" {
        return Ok(());
    }
    if prefix == b"xmlns" || prefix.contains(&b':') {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML namespace declaration uses a reserved or malformed prefix"
        )));
    }
    validate_xml_ncname(side, prefix)
}

fn validate_namespace_declaration(
    side: InputSide,
    prefix: &[u8],
    value: &[u8],
) -> Result<(), RoundTripFailure> {
    if prefix == b"xml" && value != XML_NAMESPACE {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML redeclares the reserved xml prefix"
        )));
    }
    if prefix != b"xml" && value == XML_NAMESPACE {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML binds a non-xml prefix to the reserved xml namespace"
        )));
    }
    if value == XMLNS_NAMESPACE {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML binds the reserved xmlns namespace"
        )));
    }
    if !prefix.is_empty() && value.is_empty() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML undeclares a named namespace prefix"
        )));
    }
    Ok(())
}

fn classify_namespace(namespace: &[u8]) -> StreamNamespace {
    if namespace == LANDXML_NAMESPACE.as_bytes() {
        StreamNamespace::LandXml
    } else if namespace == XINCLUDE_NAMESPACE.as_bytes() {
        StreamNamespace::XInclude
    } else if namespace.is_empty() {
        StreamNamespace::Unbound
    } else {
        StreamNamespace::Other
    }
}

fn split_qualified_name(side: InputSide, name: &[u8]) -> Result<(&[u8], &[u8]), RoundTripFailure> {
    let mut parts = name.split(|byte| *byte == b':');
    let first = parts.next().unwrap_or_default();
    let Some(second) = parts.next() else {
        return Ok((b"", first));
    };
    if first.is_empty() || second.is_empty() || parts.next().is_some() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML uses a malformed qualified element name"
        )));
    }
    if first == b"xmlns" {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML uses the reserved xmlns prefix as an element name"
        )));
    }
    validate_xml_ncname(side, first)?;
    validate_xml_ncname(side, second)?;
    Ok((first, second))
}

fn validate_xml_ncname(side: InputSide, name: &[u8]) -> Result<(), RoundTripFailure> {
    let name = std::str::from_utf8(name).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} XML name is not UTF-8: {error}"))
    })?;
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML qualified name component is empty"
        )));
    };
    if first == ':'
        || !valid_xml_name_start(first)
        || characters.any(|character| character == ':' || !valid_xml_name_char(character))
    {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML qualified name component is malformed"
        )));
    }
    Ok(())
}

fn validate_xml_name(side: InputSide, name: &str) -> Result<(), RoundTripFailure> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML element name is empty"
        )));
    };
    if !valid_xml_name_start(first) || characters.any(|character| !valid_xml_name_char(character)) {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML element name is malformed"
        )));
    }
    Ok(())
}

fn valid_xml_name_start(character: char) -> bool {
    matches!(character, ':' | 'A'..='Z' | '_' | 'a'..='z')
        || ('\u{C0}'..='\u{D6}').contains(&character)
        || ('\u{D8}'..='\u{F6}').contains(&character)
        || ('\u{F8}'..='\u{2FF}').contains(&character)
        || ('\u{370}'..='\u{37D}').contains(&character)
        || ('\u{37F}'..='\u{1FFF}').contains(&character)
        || ('\u{200C}'..='\u{200D}').contains(&character)
        || ('\u{2070}'..='\u{218F}').contains(&character)
        || ('\u{2C00}'..='\u{2FEF}').contains(&character)
        || ('\u{3001}'..='\u{D7FF}').contains(&character)
        || ('\u{F900}'..='\u{FDCF}').contains(&character)
        || ('\u{FDF0}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{EFFFF}').contains(&character)
}

fn valid_xml_name_char(character: char) -> bool {
    valid_xml_name_start(character)
        || matches!(character, '-' | '.' | '0'..='9' | '\u{B7}')
        || ('\u{300}'..='\u{36F}').contains(&character)
        || ('\u{203F}'..='\u{2040}').contains(&character)
}

fn require_landxml_namespace(
    side: InputSide,
    namespace: StreamNamespace,
) -> Result<(), RoundTripFailure> {
    if namespace != StreamNamespace::LandXml {
        return Err(schema_error(
            side,
            "container has an unsupported child element",
        ));
    }
    Ok(())
}

fn require_stream_tag(
    side: InputSide,
    namespace: StreamNamespace,
    actual: &[u8],
    expected: &[u8],
) -> Result<(), RoundTripFailure> {
    if namespace != StreamNamespace::LandXml || actual != expected {
        return Err(schema_error(
            side,
            "container has an unsupported child element",
        ));
    }
    Ok(())
}

fn validate_attributes(side: InputSide, element: &BytesStart<'_>) -> Result<(), RoundTripFailure> {
    for (index, attribute) in element_attributes(side, element).enumerate() {
        let attribute = attribute?;
        let name = std::str::from_utf8(attribute.name).map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{side} XML attribute name is not UTF-8: {error}"
            ))
        })?;
        validate_xml_name(side, name)?;
        if name.starts_with(':') || name.ends_with(':') || name.matches(':').count() > 1 {
            return Err(RoundTripFailure::invalid(format_args!(
                "{side} XML attribute name is malformed"
            )));
        }
        for component in name.as_bytes().split(|byte| *byte == b':') {
            validate_xml_ncname(side, component)?;
        }
        for prior in element_attributes(side, element).take(index) {
            if prior?.name == attribute.name {
                return Err(RoundTripFailure::invalid(format_args!(
                    "{side} XML contains a duplicate attribute"
                )));
            }
        }
        let value = attribute.value;
        if value.contains('<') {
            return Err(RoundTripFailure::invalid(format_args!(
                "{side} XML attribute contains an unescaped less-than sign"
            )));
        }
        for_each_normalized_attribute_char(value, |_| Ok(()))?;
    }
    Ok(())
}

fn raw_attribute_value<'a>(
    side: InputSide,
    element: &'a BytesStart<'a>,
    name: &[u8],
) -> Result<Option<Cow<'a, str>>, RoundTripFailure> {
    for attribute in element_attributes(side, element) {
        let attribute = attribute?;
        if attribute.name == name {
            return Ok(Some(Cow::Borrowed(attribute.value)));
        }
    }
    Ok(None)
}

fn for_each_normalized_attribute_char(
    value: &str,
    mut write: impl FnMut(char) -> Result<(), RoundTripFailure>,
) -> Result<(), RoundTripFailure> {
    let mut cursor = 0usize;
    while cursor < value.len() {
        let remaining = &value[cursor..];
        if remaining.starts_with("\r\n") {
            write(' ')?;
            cursor += 2;
            continue;
        }
        let character = remaining
            .chars()
            .next()
            .expect("cursor remains on a UTF-8 boundary");
        if character == '&' {
            let end = remaining.find(';').ok_or_else(|| {
                RoundTripFailure::invalid("XML attribute contains an incomplete reference")
            })?;
            let reference = &remaining[1..end];
            write(resolve_attribute_reference(reference)?)?;
            cursor += end + 1;
        } else {
            if !valid_xml_character(character) {
                return Err(RoundTripFailure::invalid(
                    "XML attribute contains a character forbidden by XML 1.0",
                ));
            }
            write(match character {
                '\t' | '\r' | '\n' => ' ',
                other => other,
            })?;
            cursor += character.len_utf8();
        }
    }
    Ok(())
}

fn resolve_attribute_reference(reference: &str) -> Result<char, RoundTripFailure> {
    let value = match reference {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "apos" => '\'',
        "quot" => '"',
        numeric if numeric.starts_with("#x") => {
            let value = u32::from_str_radix(&numeric[2..], 16).map_err(|_| {
                RoundTripFailure::invalid("XML attribute has a malformed character reference")
            })?;
            char::from_u32(value)
                .filter(|character| valid_xml_character(*character))
                .ok_or_else(|| {
                    RoundTripFailure::invalid("XML attribute references an invalid XML character")
                })?
        }
        numeric if numeric.starts_with('#') => {
            let value = numeric[1..].parse::<u32>().map_err(|_| {
                RoundTripFailure::invalid("XML attribute has a malformed character reference")
            })?;
            char::from_u32(value)
                .filter(|character| valid_xml_character(*character))
                .ok_or_else(|| {
                    RoundTripFailure::invalid("XML attribute references an invalid XML character")
                })?
        }
        _ => {
            return Err(RoundTripFailure::invalid(
                "XML attribute uses an undeclared entity reference",
            ));
        }
    };
    Ok(value)
}

fn valid_xml_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

fn validate_raw_xml_characters(side: InputSide, value: &str) -> Result<(), RoundTripFailure> {
    if value.chars().all(valid_xml_character) {
        return Ok(());
    }
    Err(RoundTripFailure::invalid(format_args!(
        "{side} XML contains a character forbidden by XML 1.0"
    )))
}

fn normalized_attribute_len(value: &str) -> Result<usize, RoundTripFailure> {
    let mut length = 0usize;
    for_each_normalized_attribute_char(value, |character| {
        length = length.checked_add(character.len_utf8()).ok_or_else(|| {
            RoundTripFailure::resource("normalized XML attribute length overflow")
        })?;
        Ok(())
    })?;
    Ok(length)
}

fn append_normalized_attribute(value: &str, output: &mut String) -> Result<(), RoundTripFailure> {
    for_each_normalized_attribute_char(value, |character| {
        output.push(character);
        Ok(())
    })
}

fn append_normalized_attribute_bytes(
    value: &str,
    output: &mut Vec<u8>,
) -> Result<(), RoundTripFailure> {
    let mut encoded = [0_u8; 4];
    for_each_normalized_attribute_char(value, |character| {
        output.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        Ok(())
    })
}

fn normalized_attribute_matches(
    value: Option<Cow<'_, str>>,
    expected: &str,
) -> Result<bool, RoundTripFailure> {
    let Some(value) = value else {
        return Ok(false);
    };
    let mut expected = expected.chars();
    let mut matches = true;
    for_each_normalized_attribute_char(&value, |character| {
        if expected.next() != Some(character) {
            matches = false;
        }
        Ok(())
    })?;
    Ok(matches && expected.next().is_none())
}

fn parse_normalized_u64(value: &str) -> Result<Option<u64>, RoundTripFailure> {
    let mut parsed = 0_u64;
    let mut digits = 0usize;
    let mut valid = true;
    for_each_normalized_attribute_char(value, |character| {
        let Some(digit) = character
            .to_digit(10)
            .filter(|_| character.is_ascii_digit())
        else {
            valid = false;
            return Ok(());
        };
        digits += 1;
        let Some(next) = parsed
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit)))
        else {
            valid = false;
            return Ok(());
        };
        parsed = next;
        Ok(())
    })?;
    Ok((valid && digits > 0).then_some(parsed))
}

fn xml10_normalized_text_len(value: &str) -> usize {
    value.len()
        - value
            .as_bytes()
            .windows(2)
            .filter(|pair| *pair == b"\r\n")
            .count()
}

fn append_xml10_normalized_text(value: &str, output: &mut String) {
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..].find('\r') {
        let carriage_return = cursor + relative;
        output.push_str(&value[cursor..carriage_return]);
        output.push('\n');
        cursor = carriage_return + 1;
        if value.as_bytes().get(cursor) == Some(&b'\n') {
            cursor += 1;
        }
    }
    output.push_str(&value[cursor..]);
}

fn validate_end_name(
    side: InputSide,
    frame: Option<&ElementFrame>,
    actual: &[u8],
) -> Result<(), RoundTripFailure> {
    let Some(frame) = frame else {
        return Err(schema_error(side, "unexpected closing element"));
    };
    if actual != frame.qualified_name.as_bytes() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML closing element does not match its opening element"
        )));
    }
    Ok(())
}

fn resolve_reference<'a>(
    side: InputSide,
    reference: &quick_xml::events::BytesRef<'_>,
    encoded: &'a mut [u8; 4],
) -> Result<&'a str, RoundTripFailure> {
    if let Some(character) = reference.resolve_char_ref().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} XML reference is malformed: {error}"))
    })? {
        if !valid_xml_character(character) {
            return Err(RoundTripFailure::invalid(format_args!(
                "{side} XML reference names a character forbidden by XML 1.0"
            )));
        }
        return Ok(character.encode_utf8(encoded));
    }
    let name = reference.decode().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} XML reference is malformed: {error}"))
    })?;
    match name.as_ref() {
        "amp" => Ok("&"),
        "lt" => Ok("<"),
        "gt" => Ok(">"),
        "apos" => Ok("'"),
        "quot" => Ok("\""),
        _ => Err(RoundTripFailure::invalid(format_args!(
            "{side} XML uses an undeclared entity reference"
        ))),
    }
}

fn check_xml_token(side: InputSide, actual: usize, allowed: u64) -> Result<(), RoundTripFailure> {
    if actual as u64 > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} XML token bytes required {actual}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn check_next_item_limit(
    side: InputSide,
    item: &str,
    current: usize,
    allowed: u64,
) -> Result<(), RoundTripFailure> {
    let required = (current as u64).saturating_add(1);
    if required > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} {item} required at least {required}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn parse_position_text(side: InputSide, text: &str) -> Result<Position, RoundTripFailure> {
    let mut values = text.split_whitespace();
    let northing = parse_coordinate_text(side, values.next())?;
    let easting = parse_coordinate_text(side, values.next())?;
    let elevation = parse_coordinate_text(side, values.next())?;
    if values.next().is_some() {
        return Err(schema_error(
            side,
            "P must contain exactly northing easting elevation",
        ));
    }
    if [northing, easting, elevation]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} P coordinates must be finite"
        )));
    }
    Ok(Position {
        easting: canonical_zero(easting),
        northing: canonical_zero(northing),
        elevation: canonical_zero(elevation),
    })
}

fn parse_coordinate_text(side: InputSide, value: Option<&str>) -> Result<f64, RoundTripFailure> {
    value
        .ok_or_else(|| schema_error(side, "P must contain exactly northing easting elevation"))?
        .parse()
        .map_err(|_| RoundTripFailure::invalid(format_args!("{side} P coordinates are invalid")))
}

fn parse_face_text(side: InputSide, text: &str) -> Result<Triangle, RoundTripFailure> {
    let mut values = text.split_whitespace();
    let first = parse_face_id_text(side, values.next())?;
    let second = parse_face_id_text(side, values.next())?;
    let third = parse_face_id_text(side, values.next())?;
    if values.next().is_some() {
        return Err(schema_error(side, "F must contain exactly three point IDs"));
    }
    Ok(Triangle::new(first, second, third))
}

fn parse_face_id_text(side: InputSide, value: Option<&str>) -> Result<u64, RoundTripFailure> {
    value
        .ok_or_else(|| schema_error(side, "F must contain exactly three point IDs"))?
        .parse()
        .map_err(|_| RoundTripFailure::invalid(format_args!("{side} F references are invalid")))
}

fn resolve_point_id(
    side: InputSide,
    points: &[IndexedPosition],
    id: u64,
) -> Result<u64, RoundTripFailure> {
    let index = points
        .binary_search_by_key(&id, |point| point.id)
        .map_err(|_| {
            RoundTripFailure::invalid(format_args!(
                "{side} face has dangling point reference {id}"
            ))
        })?;
    Ok(index as u64)
}

fn unit_drift(side: InputSide) -> RoundTripFailure {
    RoundTripFailure::mismatch(
        RoundTripReasonCode::UnitDrift,
        format_args!("{side} units do not declare exactly one Metric linearUnit=\"meter\""),
    )
}

fn validate_face(
    side: InputSide,
    face: Triangle,
    points: &[Position],
) -> Result<(), RoundTripFailure> {
    if face.has_repeated_point() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} contains a face with repeated point references"
        )));
    }
    let [a, b, c] = face.positions(points);
    let robust_orientation = normalized_orientation_xy(a, b, c);
    let is_collinear = match robust_orientation {
        Some(orientation) if orientation != 0.0 => false,
        Some(_) | None => exact_orientation_is_zero(a, b, c),
    };
    if is_collinear {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} contains a geometrically degenerate face"
        )));
    }
    Ok(())
}

fn normalized_orientation_xy(a: Position, b: Position, c: Position) -> Option<f64> {
    let [ax, bx, cx] = scale_axis_exact([a.easting, b.easting, c.easting])?;
    let [ay, by, cy] = scale_axis_exact([a.northing, b.northing, c.northing])?;
    let orientation = orient2d(
        Coord { x: ax, y: ay },
        Coord { x: bx, y: by },
        Coord { x: cx, y: cy },
    );
    orientation.is_finite().then_some(orientation)
}

fn scale_axis_exact(values: [f64; 3]) -> Option<[f64; 3]> {
    let maximum = values
        .into_iter()
        .fold(0.0_f64, |current, value| current.max(value.abs()));
    if maximum == 0.0 {
        return Some(values);
    }
    let shift = (-binary_exponent(maximum)).clamp(-1_022, 1_023);
    let factor = normal_power_of_two(shift);
    let mut scaled = [0.0; 3];
    for (target, value) in scaled.iter_mut().zip(values) {
        *target = value * factor;
        if !target.is_finite()
            || (value != 0.0 && *target == 0.0)
            || (*target / factor).to_bits() != value.to_bits()
        {
            return None;
        }
    }
    Some(scaled)
}

fn binary_exponent(value: f64) -> i32 {
    const FRACTION_BITS: u64 = (1_u64 << 52) - 1;
    let bits = value.to_bits() & i64::MAX as u64;
    let encoded = ((bits >> 52) & 0x7ff) as i32;
    if encoded != 0 {
        encoded - 1_023
    } else {
        let Ok(highest_fraction_bit) =
            i32::try_from(63_u32.saturating_sub((bits & FRACTION_BITS).leading_zeros()))
        else {
            unreachable!("a binary64 fraction bit index fits i32");
        };
        highest_fraction_bit - 1_074
    }
}

fn normal_power_of_two(exponent: i32) -> f64 {
    debug_assert!((-1_022..=1_023).contains(&exponent));
    let Ok(encoded) = u64::try_from(exponent + 1_023) else {
        unreachable!("a validated binary64 exponent is nonnegative");
    };
    f64::from_bits(encoded << 52)
}

fn exact_orientation_is_zero(a: Position, b: Position, c: Position) -> bool {
    let a_easting_delta = exact_difference(a.easting, c.easting);
    let a_northing_delta = exact_difference(a.northing, c.northing);
    let b_easting_delta = exact_difference(b.easting, c.easting);
    let b_northing_delta = exact_difference(b.northing, c.northing);
    exact_product(&a_easting_delta, &b_northing_delta)
        == exact_product(&a_northing_delta, &b_easting_delta)
}

const EXACT_COORDINATE_LIMBS: usize = 33;
const EXACT_PRODUCT_LIMBS: usize = EXACT_COORDINATE_LIMBS * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExactCoordinate {
    negative: bool,
    limbs: [u64; EXACT_COORDINATE_LIMBS],
}

impl ExactCoordinate {
    const ZERO: Self = Self {
        negative: false,
        limbs: [0; EXACT_COORDINATE_LIMBS],
    };

    fn is_zero(self) -> bool {
        self.limbs.iter().all(|limb| *limb == 0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExactProduct {
    negative: bool,
    limbs: [u64; EXACT_PRODUCT_LIMBS],
}

fn exact_difference(left: f64, right: f64) -> ExactCoordinate {
    let left = exact_scaled_coordinate(left);
    let right = exact_scaled_coordinate(right);
    if left.negative != right.negative {
        let mut result = ExactCoordinate {
            negative: left.negative,
            limbs: add_magnitudes(&left.limbs, &right.limbs),
        };
        if result.is_zero() {
            result.negative = false;
        }
        return result;
    }

    match compare_magnitudes(&left.limbs, &right.limbs) {
        std::cmp::Ordering::Equal => ExactCoordinate::ZERO,
        std::cmp::Ordering::Greater => ExactCoordinate {
            negative: left.negative,
            limbs: subtract_magnitudes(&left.limbs, &right.limbs),
        },
        std::cmp::Ordering::Less => ExactCoordinate {
            negative: !left.negative,
            limbs: subtract_magnitudes(&right.limbs, &left.limbs),
        },
    }
}

fn exact_scaled_coordinate(value: f64) -> ExactCoordinate {
    const FRACTION_BITS: u64 = (1_u64 << 52) - 1;
    const SIGN_BIT: u64 = 1_u64 << 63;
    let bits = value.to_bits();
    let encoded_exponent =
        usize::try_from((bits >> 52) & 0x7ff).expect("a binary64 encoded exponent fits usize");
    let fraction = bits & FRACTION_BITS;
    let (significand, shift) = if encoded_exponent == 0 {
        (fraction, 0)
    } else {
        ((1_u64 << 52) | fraction, encoded_exponent - 1)
    };
    let mut limbs = [0_u64; EXACT_COORDINATE_LIMBS];
    if significand != 0 {
        let limb = shift / u64::BITS as usize;
        let offset = shift % u64::BITS as usize;
        limbs[limb] |= significand << offset;
        if offset != 0 {
            limbs[limb + 1] |= significand >> (u64::BITS as usize - offset);
        }
    }
    ExactCoordinate {
        negative: significand != 0 && bits & SIGN_BIT != 0,
        limbs,
    }
}

fn compare_magnitudes(
    left: &[u64; EXACT_COORDINATE_LIMBS],
    right: &[u64; EXACT_COORDINATE_LIMBS],
) -> std::cmp::Ordering {
    for (left, right) in left.iter().zip(right).rev() {
        match left.cmp(right) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

fn add_magnitudes(
    left: &[u64; EXACT_COORDINATE_LIMBS],
    right: &[u64; EXACT_COORDINATE_LIMBS],
) -> [u64; EXACT_COORDINATE_LIMBS] {
    let mut output = [0_u64; EXACT_COORDINATE_LIMBS];
    let mut carry = 0_u128;
    for (output, (left, right)) in output.iter_mut().zip(left.iter().zip(right)) {
        let total = u128::from(*left) + u128::from(*right) + carry;
        *output = u64::try_from(total & u128::from(u64::MAX)).expect("low limb fits u64");
        carry = total >> u64::BITS;
    }
    debug_assert_eq!(carry, 0);
    output
}

fn subtract_magnitudes(
    larger: &[u64; EXACT_COORDINATE_LIMBS],
    smaller: &[u64; EXACT_COORDINATE_LIMBS],
) -> [u64; EXACT_COORDINATE_LIMBS] {
    let mut output = [0_u64; EXACT_COORDINATE_LIMBS];
    let mut borrow = false;
    for (output, (larger, smaller)) in output.iter_mut().zip(larger.iter().zip(smaller)) {
        let (difference, first_borrow) = larger.overflowing_sub(*smaller);
        let (difference, second_borrow) = difference.overflowing_sub(u64::from(borrow));
        *output = difference;
        borrow = first_borrow || second_borrow;
    }
    debug_assert!(!borrow);
    output
}

fn exact_product(left: &ExactCoordinate, right: &ExactCoordinate) -> ExactProduct {
    let mut limbs = [0_u64; EXACT_PRODUCT_LIMBS];
    for (left_index, left_limb) in left.limbs.iter().copied().enumerate() {
        let mut carry = 0_u128;
        for (right_index, right_limb) in right.limbs.iter().copied().enumerate() {
            let index = left_index + right_index;
            let total = u128::from(left_limb)
                .saturating_mul(u128::from(right_limb))
                .saturating_add(u128::from(limbs[index]))
                .saturating_add(carry);
            limbs[index] = u64::try_from(total & u128::from(u64::MAX)).expect("low limb fits u64");
            carry = total >> u64::BITS;
        }
        let mut index = left_index + EXACT_COORDINATE_LIMBS;
        while carry != 0 {
            let total = u128::from(limbs[index]).saturating_add(carry);
            limbs[index] = u64::try_from(total & u128::from(u64::MAX)).expect("low limb fits u64");
            carry = total >> u64::BITS;
            index += 1;
        }
    }
    let zero = limbs.iter().all(|limb| *limb == 0);
    ExactProduct {
        negative: !zero && left.negative != right.negative,
        limbs,
    }
}

fn schema_error(side: InputSide, message: &'static str) -> RoundTripFailure {
    RoundTripFailure::invalid(format_args!("{side} schema is unsupported: {message}"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ComparisonFacts {
    comparison_count: u64,
    max_easting_drift_metres: f64,
    max_northing_drift_metres: f64,
    max_horizontal_drift_metres: f64,
    max_vertical_drift_metres: f64,
    retained_peak_bytes: u64,
}

struct ComparisonFailure {
    failure: RoundTripFailure,
    comparison: ComparisonFacts,
    comparison_available: bool,
}

struct ComparisonRetainedTracker {
    base: u64,
    limit: u64,
    peak: u64,
}

impl ComparisonRetainedTracker {
    fn new(
        reference: &ParsedSurface,
        returned: &ParsedSurface,
        preflight_bytes: u64,
        allowed_bytes: u64,
    ) -> Result<Self, RoundTripFailure> {
        check_working_limit("retained round-trip data", preflight_bytes, allowed_bytes)?;
        Ok(Self {
            base: reference
                .retained_bytes()
                .saturating_add(returned.retained_bytes()),
            limit: allowed_bytes,
            peak: preflight_bytes,
        })
    }

    fn observe_extra(&mut self, extra_bytes: u64) -> Result<(), RoundTripFailure> {
        let required = self.base.saturating_add(extra_bytes);
        self.peak = self.peak.max(required);
        check_working_limit("retained round-trip data", required, self.limit)
    }
}

fn compare_surfaces(
    reference: &ParsedSurface,
    returned: &ParsedSurface,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
) -> Result<ComparisonFacts, ComparisonFailure> {
    let preflight_bytes = comparison_retained_bytes(reference, returned);
    let mut retained = match ComparisonRetainedTracker::new(
        reference,
        returned,
        preflight_bytes,
        limits.retained_working_bytes,
    ) {
        Ok(retained) => retained,
        Err(failure) => {
            return Err(ComparisonFailure {
                failure,
                comparison: ComparisonFacts {
                    retained_peak_bytes: preflight_bytes,
                    ..ComparisonFacts::default()
                },
                comparison_available: false,
            });
        }
    };
    compare_surfaces_inner(
        reference,
        returned,
        tolerances,
        limits.comparisons,
        &mut retained,
    )
}

fn comparison_retained_bytes(reference: &ParsedSurface, returned: &ParsedSurface) -> u64 {
    let points = reference.points.len().max(returned.points.len()) as u64;
    let faces = reference.faces.len().saturating_add(returned.faces.len()) as u64;
    reference
        .retained_bytes()
        .saturating_add(returned.retained_bytes())
        .saturating_add(
            points.saturating_mul((size_of::<([u64; 3], usize)>() + 2 * size_of::<usize>()) as u64),
        )
        .saturating_add(faces.saturating_mul(size_of::<[u64; 3]>() as u64))
        .saturating_add(
            (2 * FACE_DIAGNOSTIC_SAMPLE as u64).saturating_mul(size_of::<[u64; 3]>() as u64),
        )
}

fn compare_surfaces_inner(
    reference: &ParsedSurface,
    returned: &ParsedSurface,
    tolerances: RoundTripTolerances,
    max_comparisons: u64,
    retained: &mut ComparisonRetainedTracker,
) -> Result<ComparisonFacts, ComparisonFailure> {
    if reference.points.len() != returned.points.len() {
        return Err(ComparisonFailure {
            failure: RoundTripFailure::mismatch(
                RoundTripReasonCode::PointCountDrift,
                format_args!(
                    "vertex counts differ: REFERENCE has {}, RETURNED has {}",
                    reference.points.len(),
                    returned.points.len()
                ),
            ),
            comparison: ComparisonFacts {
                retained_peak_bytes: retained.peak,
                ..ComparisonFacts::default()
            },
            comparison_available: false,
        });
    }
    let (returned_to_reference, mut facts) = match_points(
        &reference.points,
        &returned.points,
        tolerances,
        max_comparisons,
        retained,
    )
    .map_err(|failure| ComparisonFailure {
        failure,
        comparison: ComparisonFacts {
            retained_peak_bytes: retained.peak,
            ..ComparisonFacts::default()
        },
        comparison_available: false,
    })?;
    if let Err(failure) = compare_topology(reference, returned, &returned_to_reference, retained) {
        facts.retained_peak_bytes = retained.peak;
        return Err(ComparisonFailure {
            failure,
            comparison: facts,
            comparison_available: true,
        });
    }
    facts.retained_peak_bytes = retained.peak;
    Ok(facts)
}

fn match_points(
    reference: &[Position],
    returned: &[Position],
    tolerances: RoundTripTolerances,
    max_comparisons: u64,
    retained: &mut ComparisonRetainedTracker,
) -> Result<(Vec<usize>, ComparisonFacts), RoundTripFailure> {
    if tolerances.horizontal_metres() == 0.0 && tolerances.vertical_metres() == 0.0 {
        return match_exact_points(reference, returned, max_comparisons, retained);
    }
    let mut returned_by_easting = Vec::new();
    reserve_comparison_vec(
        &mut returned_by_easting,
        returned.len(),
        0,
        retained,
        "easting index",
    )?;
    returned_by_easting.extend(0..returned.len());
    returned_by_easting.sort_unstable_by(|left, right| {
        returned[*left].easting.total_cmp(&returned[*right].easting)
    });
    let mut returned_to_reference = Vec::new();
    reserve_comparison_vec(
        &mut returned_to_reference,
        returned.len(),
        collection_bytes::<usize>(&returned_by_easting),
        retained,
        "tolerance point mapping",
    )?;
    returned_to_reference.resize(returned.len(), usize::MAX);
    let mut facts = ComparisonFacts::default();
    for (reference_index, reference_point) in reference.iter().enumerate() {
        let (returned_index, drift) = unique_point_match(
            *reference_point,
            returned,
            &returned_by_easting,
            tolerances,
            max_comparisons,
            &mut facts,
        )?;
        if returned_to_reference[returned_index] != usize::MAX {
            return Err(RoundTripFailure::mismatch(
                RoundTripReasonCode::VertexAmbiguous,
                "vertex matching is ambiguous under the declared tolerances",
            ));
        }
        returned_to_reference[returned_index] = reference_index;
        update_drift_facts(&mut facts, drift);
    }
    Ok((returned_to_reference, facts))
}

fn match_exact_points(
    reference: &[Position],
    returned: &[Position],
    max_comparisons: u64,
    retained: &mut ComparisonRetainedTracker,
) -> Result<(Vec<usize>, ComparisonFacts), RoundTripFailure> {
    let comparison_count = u64::try_from(reference.len()).unwrap_or(u64::MAX);
    if comparison_count > max_comparisons {
        return Err(RoundTripFailure::resource(format_args!(
            "vertex comparisons require {comparison_count}; limit is {max_comparisons}"
        )));
    }
    let mut returned_positions = Vec::new();
    reserve_comparison_vec(
        &mut returned_positions,
        returned.len(),
        0,
        retained,
        "exact-coordinate index",
    )?;
    returned_positions.extend(
        returned
            .iter()
            .enumerate()
            .map(|(index, point)| (point.key(), index)),
    );
    returned_positions.sort_unstable_by_key(|entry| entry.0);
    if returned_positions
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0)
    {
        return Err(RoundTripFailure::mismatch(
            RoundTripReasonCode::VertexAmbiguous,
            "RETURNED contains duplicate coordinates, so vertex matching is ambiguous",
        ));
    }
    let mut returned_to_reference = Vec::new();
    reserve_comparison_vec(
        &mut returned_to_reference,
        returned.len(),
        collection_bytes::<([u64; 3], usize)>(&returned_positions),
        retained,
        "exact point mapping",
    )?;
    returned_to_reference.resize(returned.len(), usize::MAX);
    for (reference_index, point) in reference.iter().enumerate() {
        let returned_index = returned_positions
            .binary_search_by_key(&point.key(), |entry| entry.0)
            .ok()
            .map(|index| returned_positions[index].1)
            .ok_or_else(|| {
                RoundTripFailure::mismatch(
                    RoundTripReasonCode::VertexUnmatched,
                    "a REFERENCE vertex has no exact RETURNED coordinate match",
                )
            })?;
        if returned_to_reference[returned_index] != usize::MAX {
            return Err(RoundTripFailure::mismatch(
                RoundTripReasonCode::VertexAmbiguous,
                "REFERENCE contains duplicate coordinates, so vertex matching is ambiguous",
            ));
        }
        returned_to_reference[returned_index] = reference_index;
    }
    Ok((
        returned_to_reference,
        ComparisonFacts {
            comparison_count,
            ..ComparisonFacts::default()
        },
    ))
}

fn reserve_comparison_vec<T>(
    values: &mut Vec<T>,
    additional: usize,
    live_extra_bytes: u64,
    retained: &mut ComparisonRetainedTracker,
    purpose: &str,
) -> Result<(), RoundTripFailure> {
    let required_len = values
        .len()
        .checked_add(additional)
        .ok_or_else(|| RoundTripFailure::resource(format_args!("{purpose} size overflow")))?;
    let old_capacity = values.capacity();
    let pre_reserve_capacity = growth_capacity(old_capacity, required_len)?;
    retained.observe_extra(
        live_extra_bytes.saturating_add(capacity_bytes::<T>(pre_reserve_capacity)),
    )?;
    values
        .try_reserve_exact(additional)
        .map_err(|_| RoundTripFailure::resource(format_args!("{purpose} allocation failed")))?;
    let actual_capacity = values.capacity();
    let post_reserve_capacity = if required_len > old_capacity {
        overlapping_capacity(old_capacity, actual_capacity)
    } else {
        actual_capacity
    };
    retained.observe_extra(
        live_extra_bytes.saturating_add(capacity_bytes::<T>(post_reserve_capacity)),
    )?;
    retained.observe_extra(live_extra_bytes.saturating_add(capacity_bytes::<T>(actual_capacity)))
}

fn unique_point_match(
    reference: Position,
    returned: &[Position],
    returned_by_easting: &[usize],
    tolerances: RoundTripTolerances,
    max_comparisons: u64,
    facts: &mut ComparisonFacts,
) -> Result<(usize, CoordinateDrift), RoundTripFailure> {
    let reference_easting = reference.easting;
    let horizontal_tolerance = tolerances.horizontal_metres();
    let start = returned_by_easting.partition_point(|index| {
        easting_is_below_window(
            returned[*index].easting,
            reference_easting,
            horizontal_tolerance,
        )
    });
    let end = returned_by_easting.partition_point(|index| {
        !easting_is_above_window(
            returned[*index].easting,
            reference_easting,
            horizontal_tolerance,
        )
    });
    let mut matched = None;
    for returned_index in &returned_by_easting[start..end] {
        facts.comparison_count = facts.comparison_count.saturating_add(1);
        if facts.comparison_count > max_comparisons {
            return Err(RoundTripFailure::resource(format_args!(
                "vertex comparisons exceed the {max_comparisons} comparison limit"
            )));
        }
        let drift = CoordinateDrift::between(reference, returned[*returned_index]);
        if drift.is_within(tolerances) && matched.replace((*returned_index, drift)).is_some() {
            return Err(RoundTripFailure::mismatch(
                RoundTripReasonCode::VertexAmbiguous,
                "vertex matching is ambiguous under the declared tolerances",
            ));
        }
    }
    matched.ok_or_else(|| {
        RoundTripFailure::mismatch(
            RoundTripReasonCode::ToleranceDrift,
            "a REFERENCE vertex has no RETURNED match within the declared tolerances",
        )
    })
}

fn easting_is_below_window(candidate: f64, reference: f64, tolerance: f64) -> bool {
    candidate < reference && reference - candidate > tolerance
}

fn easting_is_above_window(candidate: f64, reference: f64, tolerance: f64) -> bool {
    candidate > reference && candidate - reference > tolerance
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CoordinateDrift {
    easting: f64,
    northing: f64,
    horizontal: f64,
    vertical: f64,
}

impl CoordinateDrift {
    fn between(reference: Position, returned: Position) -> Self {
        let easting = (returned.easting - reference.easting).abs();
        let northing = (returned.northing - reference.northing).abs();
        let vertical = (returned.elevation - reference.elevation).abs();
        Self {
            easting,
            northing,
            horizontal: easting.hypot(northing),
            vertical,
        }
    }

    fn is_within(self, tolerances: RoundTripTolerances) -> bool {
        self.horizontal <= tolerances.horizontal_metres()
            && self.vertical <= tolerances.vertical_metres()
    }
}

fn update_drift_facts(facts: &mut ComparisonFacts, drift: CoordinateDrift) {
    facts.max_easting_drift_metres = facts.max_easting_drift_metres.max(drift.easting);
    facts.max_northing_drift_metres = facts.max_northing_drift_metres.max(drift.northing);
    facts.max_horizontal_drift_metres = facts.max_horizontal_drift_metres.max(drift.horizontal);
    facts.max_vertical_drift_metres = facts.max_vertical_drift_metres.max(drift.vertical);
}

fn compare_topology(
    reference: &ParsedSurface,
    returned: &ParsedSurface,
    returned_to_reference: &Vec<usize>,
    retained: &mut ComparisonRetainedTracker,
) -> Result<(), RoundTripFailure> {
    let mapping_bytes = collection_bytes::<usize>(returned_to_reference);
    let mut reference_faces = Vec::new();
    reserve_comparison_vec(
        &mut reference_faces,
        reference.faces.len(),
        mapping_bytes,
        retained,
        "reference topology",
    )?;
    reference_faces.extend(
        reference
            .faces
            .iter()
            .copied()
            .map(Triangle::canonical_point_indices),
    );
    let reference_face_bytes = collection_bytes::<[u64; 3]>(&reference_faces);
    let mut returned_faces = Vec::new();
    reserve_comparison_vec(
        &mut returned_faces,
        returned.faces.len(),
        mapping_bytes.saturating_add(reference_face_bytes),
        retained,
        "returned topology",
    )?;
    returned_faces.extend(
        returned
            .faces
            .iter()
            .copied()
            .map(|face| face.remap(returned_to_reference).canonical_point_indices()),
    );
    reference_faces.sort_unstable();
    returned_faces.sort_unstable();
    if reference_faces != returned_faces {
        let topology_bytes = mapping_bytes
            .saturating_add(reference_face_bytes)
            .saturating_add(collection_bytes::<[u64; 3]>(&returned_faces));
        let added = summarize_face_difference(
            ADDED_FACE_HASH_DOMAIN,
            &returned_faces,
            &reference_faces,
            topology_bytes,
            retained,
        )?;
        let added_sample_bytes = collection_bytes::<[u64; 3]>(&added.sample);
        let removed = summarize_face_difference(
            REMOVED_FACE_HASH_DOMAIN,
            &reference_faces,
            &returned_faces,
            topology_bytes.saturating_add(added_sample_bytes),
            retained,
        )?;
        retained.observe_extra(
            topology_bytes
                .saturating_add(added_sample_bytes)
                .saturating_add(collection_bytes::<[u64; 3]>(&removed.sample)),
        )?;
        let live_diagnostic_bytes = topology_bytes
            .saturating_add(added_sample_bytes)
            .saturating_add(collection_bytes::<[u64; 3]>(&removed.sample));
        retained.observe_extra(
            live_diagnostic_bytes.saturating_add(size_of::<TopologyDifference>() as u64),
        )?;
        let failure = RoundTripFailure::topology_mismatch(
            TopologyDifference {
                added_count: added.count,
                removed_count: removed.count,
                added_hash: added.hash,
                removed_hash: removed.hash,
                added_sample: added.sample,
                removed_sample: removed.sample,
            },
            "TIN topology differs after point-ID, face-order, and winding normalization",
        )?;
        retained.observe_extra(live_diagnostic_bytes.saturating_add(capacity_bytes::<
            TopologyDifference,
        >(
            failure.topology_difference.capacity(),
        )))?;
        return Err(failure);
    }
    Ok(())
}

struct FaceDifferenceSummary {
    count: u64,
    hash: [u8; 32],
    sample: Vec<[u64; 3]>,
}

fn summarize_face_difference(
    domain: &[u8],
    candidates: &[[u64; 3]],
    excluded: &[[u64; 3]],
    live_extra_bytes: u64,
    retained: &mut ComparisonRetainedTracker,
) -> Result<FaceDifferenceSummary, RoundTripFailure> {
    let count = face_difference_iter(candidates, excluded).count() as u64;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&count.to_le_bytes());
    let mut sample = Vec::new();
    let sample_count = usize::try_from(count)
        .unwrap_or(usize::MAX)
        .min(FACE_DIAGNOSTIC_SAMPLE);
    reserve_comparison_vec(
        &mut sample,
        sample_count,
        live_extra_bytes,
        retained,
        "topology sample",
    )?;
    for face in face_difference_iter(candidates, excluded) {
        for point in face {
            hasher.update(&point.to_le_bytes());
        }
        if sample.len() < FACE_DIAGNOSTIC_SAMPLE {
            sample.push(*face);
        }
    }
    retained
        .observe_extra(live_extra_bytes.saturating_add(collection_bytes::<[u64; 3]>(&sample)))?;
    Ok(FaceDifferenceSummary {
        count,
        hash: *hasher.finalize().as_bytes(),
        sample,
    })
}

fn face_difference_iter<'a>(
    candidates: &'a [[u64; 3]],
    excluded: &'a [[u64; 3]],
) -> impl Iterator<Item = &'a [u64; 3]> {
    let mut excluded_index = 0;
    candidates.iter().filter(move |candidate| {
        while excluded_index < excluded.len() && excluded[excluded_index] < **candidate {
            excluded_index += 1;
        }
        excluded_index == excluded.len() || excluded[excluded_index] != **candidate
    })
}

pub(crate) fn hash_face_difference(domain: &[u8], faces: &[[u64; 3]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&(faces.len() as u64).to_le_bytes());
    for face in faces {
        for point in face {
            hasher.update(&point.to_le_bytes());
        }
    }
    *hasher.finalize().as_bytes()
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        fs::{self, OpenOptions},
        io::{BufRead as _, BufReader, Cursor, Write as _},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        RoundTripDeclaration, RoundTripFailureKind, RoundTripLimits, RoundTripTolerances,
        verify_landxml_round_trip,
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    const REFERENCE_POINTS: &[(&str, &str)] = &[
        ("1", "0 0 0"),
        ("2", "0 10 0"),
        ("3", "10 10 0"),
        ("4", "10 0 0"),
    ];
    const REFERENCE_FACES: &[&str] = &["1 2 3", "1 3 4"];

    #[test]
    fn exact_distinct_files_report_bound_content_and_declaration() {
        let fixture = Fixture::new("exact");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&xml, &xml);

        let report = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("exact semantic round trip succeeds");

        assert!(report.exact_bytes());
        assert!(report.topology_matches());
        assert_eq!(
            report.reference_content_hash(),
            report.returned_content_hash()
        );
        assert_eq!(report.reference_bytes(), xml.len() as u64);
        assert_eq!(report.returned_bytes(), xml.len() as u64);
        assert_eq!(report.vertex_count(), 4);
        assert_eq!(report.face_count(), 2);
        assert_eq!(report.max_horizontal_drift_metres().to_bits(), 0);
        assert_eq!(report.max_vertical_drift_metres().to_bits(), 0);
        assert_eq!(report.declared_application(), "generated-fixture");
        assert_eq!(report.declared_version(), "test-only");
        assert_eq!(report.declared_settings_profile(), "metric-tin-v1");
        assert_eq!(report.tolerances().horizontal_metres().to_bits(), 0);
        assert!(report.comparison_count() >= report.vertex_count());
    }

    #[test]
    fn point_ids_order_face_order_and_winding_are_normalized() {
        let fixture = Fixture::new("normalize");
        let returned_points = &[
            ("40", "10 0 0"),
            ("20", "0 10 0"),
            ("10", "0 0 0"),
            ("30", "10 10 0"),
        ];
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(returned_points, &["40 30 10", "30 20 10"], true);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        let report = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("renumbered and reordered topology is equal");

        assert!(!report.exact_bytes());
        assert!(report.topology_matches());
        assert_eq!(report.vertex_count(), 4);
        assert_eq!(report.face_count(), 2);
    }

    #[test]
    fn empty_surface_name_is_ignored_semantic_metadata() {
        let fixture = Fixture::new("empty-surface-name");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml =
            reference_xml.replacen("<Surface name=\"Ground\">", "<Surface name=\"\">", 1);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        let report = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("an empty surface name does not change terrain semantics");

        assert!(!report.exact_bytes());
        assert!(report.topology_matches());
    }

    #[test]
    fn finite_coordinate_drift_within_both_tolerances_is_reported() {
        let fixture = Fixture::new("within-tolerance");
        let returned_points = &[
            ("1", "0 0 0"),
            ("2", "0.04 10.03 0.02"),
            ("3", "10 10 0"),
            ("4", "10 0 0"),
        ];
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(returned_points, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        let report = verify(
            &reference,
            &returned,
            tolerances(0.051, 0.02),
            default_limits(),
        )
        .expect("drift inside radial horizontal and vertical tolerances succeeds");

        assert!((report.max_easting_drift_metres() - 0.03).abs() < 1.0e-12);
        assert!((report.max_northing_drift_metres() - 0.04).abs() < 1.0e-12);
        assert!((report.max_horizontal_drift_metres() - 0.05).abs() < 1.0e-12);
        assert!((report.max_vertical_drift_metres() - 0.02).abs() < 1.0e-12);
    }

    #[test]
    fn inclusive_horizontal_boundary_survives_spatial_lookup_rounding() {
        let fixture = Fixture::new("inclusive-horizontal-boundary");
        let reference_points = &[("1", "0 -10 1"), ("2", "20 0 2"), ("3", "0 10 3")];
        let returned_points = &[("1", "0 -3.9 1"), ("2", "20 0 2"), ("3", "0 10 3")];
        let reference_xml = landxml(reference_points, &["1 2 3"], false);
        let returned_xml = landxml(returned_points, &["1 2 3"], false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        let report = verify(
            &reference,
            &returned,
            tolerances(6.1, 0.0),
            default_limits(),
        )
        .expect("the inclusive horizontal boundary is a candidate");

        assert_eq!(
            report.max_horizontal_drift_metres().to_bits(),
            6.1_f64.to_bits()
        );
    }

    #[test]
    fn coordinate_drift_outside_tolerance_is_a_semantic_mismatch() {
        let fixture = Fixture::new("coordinate-mismatch");
        let returned_points = &[
            ("1", "0 0 0"),
            ("2", "0 10.1 0"),
            ("3", "10 10 0"),
            ("4", "10 0 0"),
        ];
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(returned_points, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.01, 0.0),
                default_limits(),
            ),
            RoundTripFailureKind::SemanticMismatch,
        );
    }

    #[test]
    fn changed_diagonal_is_a_topology_mismatch() {
        let fixture = Fixture::new("topology-mismatch");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(REFERENCE_POINTS, &["1 2 4", "2 3 4"], false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            ),
            RoundTripFailureKind::SemanticMismatch,
        );
    }

    #[test]
    fn topology_failure_retained_peak_charges_both_live_samples() {
        let fixture = Fixture::new("topology-retained-peak");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(REFERENCE_POINTS, &["1 2 4", "2 3 4"], false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);
        let declaration = || {
            RoundTripDeclaration::new("generated-fixture", "test-only", "metric-tin-v1")
                .expect("valid declaration")
        };

        let baseline = super::evaluate_landxml_round_trip(
            &reference,
            &returned,
            declaration(),
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("topology drift is a reported semantic result");
        let super::RoundTripEvaluation::Failed(baseline) = baseline else {
            panic!("changed diagonal must fail topology verification");
        };
        let difference = baseline
            .failure()
            .topology_difference()
            .expect("topology details are retained");
        assert!(!difference.added_sample.is_empty());
        assert!(!difference.removed_sample.is_empty());
        let retained_peak = baseline.retained_peak_bytes();

        let exact = super::evaluate_landxml_round_trip(
            &reference,
            &returned,
            declaration(),
            tolerances(0.0, 0.0),
            RoundTripLimits {
                retained_working_bytes: retained_peak,
                ..default_limits()
            },
        )
        .expect("the exact topology-failure peak is inclusive");
        assert!(matches!(exact, super::RoundTripEvaluation::Failed(_)));

        let Err(one_under) = super::evaluate_landxml_round_trip(
            &reference,
            &returned,
            declaration(),
            tolerances(0.0, 0.0),
            RoundTripLimits {
                retained_working_bytes: retained_peak - 1,
                ..default_limits()
            },
        ) else {
            panic!("one byte under the observed topology peak must fail");
        };
        assert_eq!(one_under.kind(), RoundTripFailureKind::ResourceLimit);
    }

    #[test]
    fn duplicate_face_is_a_topology_mismatch() {
        let fixture = Fixture::new("duplicate-face");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(REFERENCE_POINTS, &["1 2 3", "3 2 1"], false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            ),
            RoundTripFailureKind::SemanticMismatch,
        );
    }

    #[test]
    fn ambiguous_vertex_matching_fails_closed() {
        let fixture = Fixture::new("ambiguous");
        let returned_points = &[
            ("1", "0 0.04 0"),
            ("2", "0 0.08 0"),
            ("3", "10 10 0"),
            ("4", "10 0 0"),
        ];
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = landxml(returned_points, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.1, 0.0),
                default_limits(),
            ),
            RoundTripFailureKind::SemanticMismatch,
        );
    }

    #[test]
    fn malformed_schema_ids_references_faces_and_coordinates_are_invalid() {
        let fixture = Fixture::new("invalid-inputs");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let invalid_documents = [
            "<LandXML",
            &reference_xml.replacen("<LandXML", "<!DOCTYPE LandXML []>\n<LandXML", 1),
            &reference_xml.replace(super::LANDXML_NAMESPACE, "urn:wrong-landxml"),
            &reference_xml.replace(
                "<Application name=\"Declared elsewhere\" version=\"ignored\"/>",
                "<Application name=\"generated\" version=\"test-only\"><xi:include xmlns:xi=\"http://www.w3.org/2001/XInclude\" href=\"ignored.xml\"/></Application>",
            ),
            &landxml(
                &[("1", "0 0 0"), ("1", "0 10 0"), ("3", "10 10 0")],
                &["1 2 3"],
                false,
            ),
            &landxml(REFERENCE_POINTS, &["1 2 99"], false),
            &landxml(REFERENCE_POINTS, &["1 1 3"], false),
            &landxml(
                &[("1", "0 0 0"), ("2", "0 10 0"), ("3", "NaN 10 0")],
                &["1 2 3"],
                false,
            ),
            &landxml(
                &[("1", "0 0 0"), ("2", "0 10 0"), ("3", "0 20 0")],
                &["1 2 3"],
                false,
            ),
            &landxml(
                &[("1", "0.1 0.1 0"), ("2", "0.2 0.2 0"), ("3", "0.3 0.3 0")],
                &["1 2 3"],
                false,
            ),
            &landxml(
                &[("1", "1 0 0"), ("2", "3 1 0"), ("3", "7 3 0")],
                &["1 2 3"],
                false,
            ),
            &landxml(
                &[
                    (
                        "1",
                        "-1.7976931348623157e308 -1.7976931348623157e308 0",
                    ),
                    ("2", "0 0 0"),
                    (
                        "3",
                        "1.7976931348623157e308 1.7976931348623157e308 0",
                    ),
                ],
                &["1 2 3"],
                false,
            ),
        ];
        for (index, invalid) in invalid_documents.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), invalid);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    default_limits(),
                ),
                RoundTripFailureKind::InvalidInput,
            );
        }
    }

    #[test]
    fn non_metric_or_missing_units_are_semantic_mismatches() {
        let fixture = Fixture::new("unit-drift");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let variants = [
            reference_xml.replace("linearUnit=\"meter\"", "linearUnit=\"foot\""),
            reference_xml.replace(
                "<Units><Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\"/></Units>\n",
                "",
            ),
            reference_xml.replace(
                "<Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\"/>",
                "<Imperial linearUnit=\"foot\"/>",
            ),
            reference_xml.replace(
                "<Units><Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\"/></Units>",
                "<Units><Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\"/></Units><Units><Metric linearUnit=\"meter\"/></Units>",
            ),
        ];
        for (index, returned_xml) in variants.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), returned_xml);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    default_limits(),
                ),
                RoundTripFailureKind::SemanticMismatch,
            );
        }
    }

    #[test]
    fn exact_axis_scaling_preserves_subnormal_and_mixed_range_triangles() {
        let fixture = Fixture::new("subnormal-orientation");
        let variants = [
            landxml(
                &[
                    ("1", "0 0 0"),
                    ("2", "0 4.9406564584124654e-324 0"),
                    ("3", "4.9406564584124654e-324 0 0"),
                ],
                &["1 2 3"],
                false,
            ),
            landxml(
                &[
                    ("1", "0 0 0"),
                    ("2", "0 1e308 0"),
                    ("3", "4.9406564584124654e-324 0 0"),
                ],
                &["1 2 3"],
                false,
            ),
            landxml(
                &[
                    ("1", "0 0 0"),
                    ("2", "4.9406564584124654e-324 1 0"),
                    ("3", "0 4.9406564584124654e-324 0"),
                ],
                &["1 2 3"],
                false,
            ),
            landxml(
                &[
                    ("1", "0 0 0"),
                    ("2", "4.9406564584124654e-324 4.9406564584124654e-324 0"),
                    ("3", "1.0000000000000002 1 0"),
                ],
                &["1 2 3"],
                false,
            ),
        ];
        for (index, xml) in variants.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), xml);
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            )
            .expect("exact axis scaling preserves the nondegenerate triangle");
        }
    }

    #[test]
    fn every_resource_ceiling_is_enforced() {
        let fixture = Fixture::new("limits");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let mut cases = [
            RoundTripLimits::new(1, 1_000, 1_000, 10, 10, 100),
            RoundTripLimits::new(10_000, 4, 10_000, 10, 10, 100),
            RoundTripLimits::new(10_000, 1_000, 1, 10, 10, 100),
            RoundTripLimits::new(10_000, 1_000, 10_000, 3, 10, 100),
            RoundTripLimits::new(10_000, 1_000, 10_000, 10, 1, 100),
            RoundTripLimits::new(10_000, 1_000, 10_000, 10, 10, 0),
        ];
        for (index, limits) in cases.iter_mut().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), &xml);
            assert_kind(
                verify(&reference, &returned, tolerances(0.0, 0.0), *limits),
                RoundTripFailureKind::ResourceLimit,
            );
        }
    }

    #[test]
    fn default_streaming_limits_match_exporter_ceilings() {
        let limits = default_limits();
        assert_eq!(limits.file_bytes(), 4 * 1_024 * 1_024 * 1_024);
        assert_eq!(limits.points(), 10_000_000);
        assert_eq!(limits.faces(), 20_000_000);
        assert_eq!(limits.xml_token_bytes(), 4 * 1_024);
        assert_eq!(limits.parser_working_bytes(), 8 * 1_024 * 1_024);
        assert_eq!(limits.retained_working_bytes(), 4 * 1_024 * 1_024 * 1_024);
    }

    #[test]
    fn parser_and_retained_working_limits_fail_before_their_allocations() {
        let fixture = Fixture::new("working-limits");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&xml, &xml);
        let parser_floor = (super::PARSER_READ_BUFFER_BYTES as u64)
            + super::PARSER_READ_BUFFER_BYTES as u64
            + 2 * super::DEFAULT_MAX_XML_TOKEN_BYTES
            + std::mem::size_of::<super::ElementFrame>() as u64;
        let parser_result = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits().with_working_limits(
                super::DEFAULT_MAX_XML_TOKEN_BYTES,
                parser_floor - 1,
                super::DEFAULT_MAX_RETAINED_WORKING_BYTES,
            ),
        );
        assert_kind(parser_result, RoundTripFailureKind::ResourceLimit);

        let retained_result = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits().with_working_limits(
                super::DEFAULT_MAX_XML_TOKEN_BYTES,
                super::DEFAULT_MAX_PARSER_WORKING_BYTES,
                1,
            ),
        );
        assert_kind(retained_result, RoundTripFailureKind::ResourceLimit);
    }

    #[test]
    fn exact_observed_working_and_typed_limits_are_inclusive() {
        let fixture = Fixture::new("inclusive-limits");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&xml, &xml);
        let baseline = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("baseline reports the deterministic accounted peaks");
        let parser_peak = baseline
            .reference_parser_peak_bytes()
            .max(baseline.returned_parser_peak_bytes());
        let retained_peak = baseline.retained_peak_bytes();
        let exact_limits = RoundTripLimits {
            file_bytes: xml.len() as u64,
            points: REFERENCE_POINTS.len() as u64,
            faces: REFERENCE_FACES.len() as u64,
            comparisons: baseline.comparison_count(),
            parser_working_bytes: parser_peak,
            retained_working_bytes: retained_peak,
            ..default_limits()
        };

        let exact = verify(&reference, &returned, tolerances(0.0, 0.0), exact_limits)
            .expect("limits equal to every observed requirement are accepted");
        assert_eq!(exact.reference_bytes(), xml.len() as u64);
        assert_eq!(exact.vertex_count(), REFERENCE_POINTS.len() as u64);
        assert_eq!(exact.face_count(), REFERENCE_FACES.len() as u64);
        assert_eq!(exact.comparison_count(), baseline.comparison_count());

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                RoundTripLimits {
                    parser_working_bytes: parser_peak - 1,
                    ..exact_limits
                },
            ),
            RoundTripFailureKind::ResourceLimit,
        );
        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                RoundTripLimits {
                    retained_working_bytes: retained_peak - 1,
                    ..exact_limits
                },
            ),
            RoundTripFailureKind::ResourceLimit,
        );
    }

    #[test]
    fn retained_point_growth_charges_overlapping_old_and_new_capacities() {
        let item_bytes = std::mem::size_of::<super::IndexedPosition>() as u64;
        let mut probe = Vec::new();
        probe.try_reserve_exact(1).unwrap();
        probe.push(super::IndexedPosition {
            id: 1,
            position: super::Position {
                easting: 0.0,
                northing: 0.0,
                elevation: 0.0,
            },
        });
        let old_capacity = u64::try_from(probe.capacity()).expect("capacity fits u64");
        probe.try_reserve_exact(1).unwrap();
        let new_capacity = u64::try_from(probe.capacity()).expect("capacity fits u64");
        let growth_peak = old_capacity
            .saturating_add(new_capacity)
            .saturating_mul(item_bytes);

        let finish_with_limit = |retained_working_bytes| {
            let mut state = super::SurfaceStreamParser::new(
                super::InputSide::Returned,
                RoundTripLimits {
                    retained_working_bytes,
                    ..default_limits()
                },
                0,
            );
            state.points.try_reserve_exact(1).unwrap();
            state.points.push(super::IndexedPosition {
                id: 1,
                position: super::Position {
                    easting: 0.0,
                    northing: 0.0,
                    elevation: 0.0,
                },
            });
            let mut frame = super::ElementFrame::new(super::ElementKind::Point, 0);
            frame.point_id = Some(2);
            frame.simple_text.push_str("1 1 0");
            frame.nonempty_segments = 1;
            state
                .finish_point(&frame)
                .map(|()| state.retained_peak_bytes)
        };

        assert_eq!(
            finish_with_limit(growth_peak).expect("the exact live growth peak is inclusive"),
            growth_peak
        );
        assert_kind(
            finish_with_limit(growth_peak - 1),
            RoundTripFailureKind::ResourceLimit,
        );
    }

    #[test]
    fn parser_stack_growth_and_post_pop_capacity_are_both_charged() {
        let frame_bytes = std::mem::size_of::<super::ElementFrame>() as u64;
        let mut probe = Vec::new();
        probe.try_reserve_exact(1).unwrap();
        probe.push(super::ElementFrame::new(super::ElementKind::LandXml, 0));
        let old_capacity = u64::try_from(probe.capacity()).expect("capacity fits u64");
        probe.try_reserve_exact(1).unwrap();
        let new_capacity = u64::try_from(probe.capacity()).expect("capacity fits u64");
        let element = quick_xml::events::BytesStart::new("Project");
        let fixed = (2 * super::PARSER_READ_BUFFER_BYTES) as u64
            + super::DEFAULT_MAX_XML_TOKEN_BYTES
            + frame_bytes;
        let growth_peak = fixed
            .saturating_add(
                old_capacity
                    .saturating_add(new_capacity)
                    .saturating_mul(frame_bytes),
            )
            .saturating_add(element.as_ref().len() as u64);

        let start_with_limit = |parser_working_bytes| {
            let mut state = super::SurfaceStreamParser::new(
                super::InputSide::Returned,
                RoundTripLimits {
                    parser_working_bytes,
                    ..default_limits()
                },
                0,
            );
            state.stack.try_reserve_exact(1).unwrap();
            state
                .stack
                .push(super::ElementFrame::new(super::ElementKind::LandXml, 0));
            state
                .start(
                    super::StreamNamespace::LandXml,
                    b"Project",
                    &element,
                    super::NamespaceFrame::default(),
                )
                .map(|()| state.parser_peak_bytes)
        };

        assert_eq!(
            start_with_limit(growth_peak).expect("the exact parser growth peak is inclusive"),
            growth_peak
        );
        assert_kind(
            start_with_limit(growth_peak - 1),
            RoundTripFailureKind::ResourceLimit,
        );

        let retained_capacity_peak = fixed.saturating_add(new_capacity.saturating_mul(frame_bytes));
        let observe_after_pop = |parser_working_bytes| {
            let mut state = super::SurfaceStreamParser::new(
                super::InputSide::Returned,
                RoundTripLimits {
                    parser_working_bytes,
                    ..default_limits()
                },
                0,
            );
            state
                .stack
                .try_reserve_exact(usize::try_from(new_capacity).expect("capacity fits usize"))
                .unwrap();
            state
                .stack
                .push(super::ElementFrame::new(super::ElementKind::LandXml, 0));
            state.stack.pop();
            state.observe_working().map(|()| state.parser_peak_bytes)
        };
        assert_eq!(
            observe_after_pop(retained_capacity_peak)
                .expect("the exact retained parser-stack capacity is inclusive"),
            retained_capacity_peak
        );
        assert_kind(
            observe_after_pop(retained_capacity_peak - 1),
            RoundTripFailureKind::ResourceLimit,
        );
    }

    #[test]
    fn lexical_stack_growth_charges_overlapping_old_and_new_capacities() {
        let mut probe = Vec::<u64>::new();
        probe.try_reserve_exact(1).unwrap();
        probe.push(3);
        let old_capacity = u64::try_from(probe.capacity()).expect("capacity fits u64");
        probe.try_reserve_exact(1).unwrap();
        let new_capacity = u64::try_from(probe.capacity()).expect("capacity fits u64");
        let item_bytes = std::mem::size_of::<u64>() as u64;
        let fixed =
            (2 * super::PARSER_READ_BUFFER_BYTES) as u64 + 2 * super::DEFAULT_MAX_XML_TOKEN_BYTES;
        let growth_peak = fixed.saturating_add(3 + 5).saturating_add(
            old_capacity
                .saturating_add(new_capacity)
                .saturating_mul(item_bytes),
        );

        let grow_with_limit = |parser_working_bytes| {
            let inner = BufReader::new(Cursor::new(Vec::<u8>::new()));
            let mut reader = super::BoundedXmlReader::new(
                inner,
                super::PARSER_READ_BUFFER_BYTES,
                super::InputSide::Returned,
                RoundTripLimits {
                    parser_working_bytes,
                    ..default_limits()
                },
                0,
            )?;
            reader.open_element_charges.try_reserve_exact(1).unwrap();
            reader.open_element_charges.push(3);
            reader.apply_boundary(super::LexicalBoundary::StartElement {
                bytes: 5,
                empty: false,
            })?;
            Ok::<_, super::RoundTripFailure>(reader.open_element_charges.capacity())
        };

        assert_eq!(
            grow_with_limit(growth_peak).expect("the exact lexical growth peak is inclusive"),
            usize::try_from(new_capacity).expect("capacity fits usize")
        );
        assert_kind(
            grow_with_limit(growth_peak - 1),
            RoundTripFailureKind::ResourceLimit,
        );
    }

    #[test]
    fn namespace_arena_growth_has_an_inclusive_exact_peak() {
        let base = quick_xml::events::BytesStart::from_content("node xmlns:base=\"urn:base\"", 4);
        let growth = quick_xml::events::BytesStart::from_content(
            "node xmlns:a=\"urn:one\" xmlns:b=\"urn:two\" xmlns:c=\"urn:three\"",
            4,
        );
        let push_with_limit = |parser_working_bytes| {
            let mut state = super::SurfaceStreamParser::new(
                super::InputSide::Returned,
                RoundTripLimits {
                    parser_working_bytes,
                    ..default_limits()
                },
                0,
            );
            let mut namespaces = super::NamespaceState::new();
            namespaces.push(&mut state, &base)?;
            namespaces.push(&mut state, &growth)?;
            Ok::<_, super::RoundTripFailure>(state.parser_peak_bytes)
        };
        let peak = push_with_limit(u64::MAX).expect("measure namespace arena peak");

        assert_eq!(
            push_with_limit(peak).expect("the exact namespace peak is inclusive"),
            peak
        );
        assert_kind(
            push_with_limit(peak - 1),
            RoundTripFailureKind::ResourceLimit,
        );
    }

    #[test]
    fn bounded_reader_does_not_rescan_partially_consumed_bytes() {
        let input = b"<root><empty attribute=\"longer-than-prefix\"/></root>";
        let inner = BufReader::with_capacity(7, Cursor::new(input.as_slice()));
        let limits = default_limits().with_working_limits(64, 256 * 1_024, 1024 * 1024);
        let mut reader = super::BoundedXmlReader::new(
            inner,
            7,
            super::InputSide::Returned,
            limits,
            input.len() as u64,
        )
        .expect("construct bounded reader");
        let mut observed = Vec::new();
        loop {
            let (first, available_len) = {
                let available = reader.fill_buf().expect("scan the next bounded chunk");
                if available.is_empty() {
                    break;
                }
                (available[0], available.len())
            };
            assert_eq!(
                reader
                    .fill_buf()
                    .expect("repeated fill preserves the exposed chunk")
                    .len(),
                available_len
            );
            observed.push(first);
            reader.consume(1);
        }
        assert_eq!(observed, input);
        assert!(reader.failure().is_none());
        assert!(reader.open_element_charges.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn exact_bytes_reader_rejects_same_inode_mutation_before_token_allocation() {
        let fixture = Fixture::new("same-inode-token-mutation");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let harmless_suffix = "<!--x-->".repeat(256 * 1_024);
        let captured = format!("{xml}{harmless_suffix}");
        let path = fixture.write("returned.xml", &captured);
        let limits = default_limits().with_working_limits(256, 256 * 1_024, 1024 * 1024);
        let mut witness =
            super::capture_regular_file(super::InputSide::Returned, &path, limits.file_bytes)
                .expect("capture original inode and length");
        let expected_bytes = witness.metadata.len();
        let oversized_body = "x".repeat(harmless_suffix.len() - 7);
        let replacement = format!("<!--{oversized_body}-->{xml}");
        assert_eq!(replacement.len(), captured.len());
        fs::write(&path, replacement).expect("rewrite the captured inode in place");
        assert!(super::same_file_identity(
            &witness.metadata,
            &fs::metadata(&path).expect("inspect rewritten inode")
        ));

        let mut result = None;
        let allocations = allocation_counter::measure(|| {
            result = Some(super::parse_surface(
                super::InputSide::Returned,
                &mut witness.file,
                limits,
                0,
                expected_bytes,
            ));
        });
        let Err(failure) = result.expect("measurement closure records result") else {
            panic!("mutated oversized token must fail");
        };
        assert_eq!(failure.failure.kind(), RoundTripFailureKind::ResourceLimit);
        assert!(
            allocations.bytes_max < 512 * 1_024,
            "mutated token allocated {} peak bytes",
            allocations.bytes_max
        );
    }

    #[cfg(unix)]
    #[test]
    fn exact_bytes_reader_rejects_append_past_captured_length() {
        let fixture = Fixture::new("captured-length-append");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let path = fixture.write("returned.xml", &xml);
        let limits = default_limits().with_working_limits(256, 256 * 1_024, 1024 * 1024);
        let mut witness =
            super::capture_regular_file(super::InputSide::Returned, &path, limits.file_bytes)
                .expect("capture original inode and length");
        let expected_bytes = witness.metadata.len();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open captured inode for append")
            .write_all(format!("<!--{}-->", "x".repeat(2 * 1_024 * 1_024)).as_bytes())
            .expect("append an oversized token");

        let Err(failure) = super::parse_surface(
            super::InputSide::Returned,
            &mut witness.file,
            limits,
            0,
            expected_bytes,
        ) else {
            panic!("bytes past the captured length must fail");
        };
        assert_eq!(failure.failure.kind(), RoundTripFailureKind::InvalidInput);
    }

    #[test]
    fn deep_namespace_nesting_is_rejected_before_parser_state_growth() {
        let fixture = Fixture::new("deep-namespace-bound");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let opening = "<n xmlns:p=\"urn:bounded\">".repeat(256);
        let closing = "</n>".repeat(256);
        let returned_xml = xml.replace(
            "<Application name=\"Declared elsewhere\" version=\"ignored\"/>",
            &format!(
                "<Application name=\"Declared elsewhere\" version=\"ignored\">{opening}{closing}</Application>"
            ),
        );
        let (reference, returned) = fixture.write_pair(&xml, &returned_xml);
        let parser_floor = 2 * super::PARSER_READ_BUFFER_BYTES as u64
            + 2 * super::DEFAULT_MAX_XML_TOKEN_BYTES
            + std::mem::size_of::<super::ElementFrame>() as u64;
        let limits = default_limits().with_working_limits(
            super::DEFAULT_MAX_XML_TOKEN_BYTES,
            parser_floor + 512,
            1024 * 1024,
        );
        let mut result = None;
        let allocations = allocation_counter::measure(|| {
            result = Some(verify(&reference, &returned, tolerances(0.0, 0.0), limits));
        });
        assert_kind(
            result.expect("measurement closure records result"),
            RoundTripFailureKind::ResourceLimit,
        );
        assert!(
            allocations.bytes_max < 512 * 1_024,
            "deep nesting allocated {} peak bytes",
            allocations.bytes_max
        );
    }

    #[test]
    fn lexical_preflight_caps_every_local_token_kind_before_allocation() {
        let fixture = Fixture::new("lexical-event-cap");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let body = "x".repeat(2_048);
        let variants = [
            format!("<!-- > < {body} -->{xml}"),
            xml.replace(
                "<P id=\"1\">0 0 0</P>",
                &format!("<P id=\"1\"><![CDATA[> < {body}]]></P>"),
            ),
            format!("<?probe > < {body}?>{xml}"),
            format!("<!DOCTYPE LandXML [ <!ELEMENT x (#PCDATA)> > {body} ]>{xml}"),
        ];
        for (index, returned_xml) in variants.iter().enumerate() {
            let reference = fixture.write(&format!("reference-token-{index}.xml"), &xml);
            let returned = fixture.write(&format!("returned-token-{index}.xml"), returned_xml);
            let result = verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits().with_working_limits(256, 256 * 1_024, 1024 * 1024),
            );
            let error = result.expect_err("oversized lexical event fails closed");
            assert!(
                matches!(
                    error.kind(),
                    RoundTripFailureKind::ResourceLimit | RoundTripFailureKind::InvalidInput
                ),
                "variant {index}: {error}"
            );
        }
    }

    #[test]
    fn oversized_event_rejection_has_small_measured_peak_allocation() {
        let fixture = Fixture::new("oversized-event-allocation");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = format!("<!-- > < {} -->{xml}", "x".repeat(2 * 1_024 * 1_024));
        let (reference, returned) = fixture.write_pair(&xml, &returned_xml);
        let limits = default_limits().with_working_limits(256, 256 * 1_024, 1024 * 1024);
        let mut result = None;
        let allocations = allocation_counter::measure(|| {
            result = Some(verify(&reference, &returned, tolerances(0.0, 0.0), limits));
        });
        assert_kind(
            result.expect("measurement closure records result"),
            RoundTripFailureKind::ResourceLimit,
        );
        assert!(
            allocations.bytes_max < 512 * 1_024,
            "oversized token allocated {} peak bytes",
            allocations.bytes_max
        );
    }

    #[test]
    fn attribute_names_and_namespace_declarations_are_text_bounded() {
        let fixture = Fixture::new("attribute-text-limits");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let long_name = "a".repeat(1_500);
        let long_namespace = format!("urn:generated:{}", "n".repeat(1_500));
        let variants = [
            xml.replace(
                "<Surface name=\"Ground\">",
                &format!("<Surface name=\"Ground\" {long_name}=\"\">"),
            ),
            xml.replace(
                "<Surface name=\"Ground\">",
                &format!("<Surface name=\"Ground\" xmlns:ignored=\"{long_namespace}\">"),
            ),
        ];
        for (index, returned_xml) in variants.iter().enumerate() {
            let reference = fixture.write(&format!("reference-attribute-{index}.xml"), &xml);
            let returned = fixture.write(&format!("returned-attribute-{index}.xml"), returned_xml);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    RoundTripLimits::new(10_000, 1_000, 1_000, 10, 10, 100),
                ),
                RoundTripFailureKind::ResourceLimit,
            );
        }
    }

    #[test]
    fn escaped_attributes_and_cr_text_normalize_without_parser_owned_values() {
        let fixture = Fixture::new("normalized-xml-values");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = reference_xml
            .replace(
                super::LANDXML_NAMESPACE,
                "http://www.landxml.org/schema/LandXML-1.&#50;",
            )
            .replace("version=\"1.2\"", "version=\"1&#x2e;2\"")
            .replace("linearUnit=\"meter\"", "linearUnit=\"me&#116;er\"")
            .replace("name=\"Ground\"", "name=\"Ground &amp; Return\"")
            .replace("surfType=\"TIN\"", "surfType=\"T&#73;N\"")
            .replace("id=\"1\"", "id=\"&#49;\"")
            .replace(">0 0 0</P>", ">0\r\n0 0</P>");
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("XML 1.0 normalization preserves the semantic surface");
    }

    #[test]
    fn lexical_tokens_crossing_the_read_buffer_boundary_are_streamed() {
        let fixture = Fixture::new("split-lexical-tokens");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let point = "<P id=\"1\">";
        let application = "<Application name=\"Declared elsewhere\" version=\"ignored\"/>";
        let entity_xml = reference_xml.replacen("0 0 0</P>", "0 0 &#48;</P>", 1);
        let comment_xml = reference_xml.replacen(
            application,
            &format!("<!--split-comment-->{application}"),
            1,
        );
        let cdata_xml = reference_xml.replacen("0 0 0</P>", "<![CDATA[0 0 0]]></P>", 1);
        let instruction_xml =
            reference_xml.replacen(application, &format!("<?split?>{application}"), 1);
        let variants = [
            align_marker_at_buffer_boundary(&reference_xml, point, point, 2),
            align_marker_at_buffer_boundary(&reference_xml, point, "</P>", 2),
            align_marker_at_buffer_boundary(&entity_xml, point, "&#48;", 2),
            align_marker_at_buffer_boundary(
                &comment_xml,
                "<!--split-comment-->",
                "<!--split-comment-->",
                2,
            ),
            align_marker_at_buffer_boundary(&cdata_xml, point, "<![CDATA[0 0 0]]>", 2),
            align_marker_at_buffer_boundary(&instruction_xml, "<?split?>", "<?split?>", 2),
        ];

        for (index, returned_xml) in variants.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), returned_xml);
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            )
            .unwrap_or_else(|error| panic!("split lexical variant {index} failed: {error}"));
        }
    }

    #[test]
    fn crlf_split_at_the_read_buffer_boundary_normalizes_once() {
        let fixture = Fixture::new("split-crlf");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = reference_xml.replacen("0 0 0</P>", "\r\n0 0 0</P>", 1);
        let returned_xml =
            align_marker_at_buffer_boundary(&returned_xml, "<P id=\"1\">", "\r\n0 0 0</P>", 1);
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("a CRLF split across raw input chunks normalizes to one newline");
    }

    #[test]
    fn utf8_scalars_split_at_the_read_buffer_boundary_are_streamed() {
        let fixture = Fixture::new("split-utf8-scalars");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let application = "<Application name=\"Declared elsewhere\" version=\"ignored\"/>";

        for (index, scalar) in ["¢", "€", "😀"].iter().enumerate() {
            let marker = format!("ignored {scalar} text");
            let returned_xml = reference_xml.replacen(
                application,
                &format!("<Application>{marker}</Application>"),
                1,
            );
            let returned_xml = align_marker_at_buffer_boundary(&returned_xml, &marker, scalar, 1);
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), &returned_xml);

            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "a {}-byte UTF-8 scalar split after its first byte failed: {error}",
                    scalar.len()
                )
            });
        }
    }

    #[test]
    fn carriage_returns_around_a_split_utf8_scalar_keep_xml10_normalization() {
        let fixture = Fixture::new("split-utf8-crlf");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let application = "<Application name=\"Declared elsewhere\" version=\"ignored\"/>";
        for (index, text) in ["ignored\r€ text", "ignored\r\n€ text"].iter().enumerate() {
            let returned_xml = reference_xml.replacen(
                application,
                &format!("<Application>{text}</Application>"),
                1,
            );
            let returned_xml = align_marker_at_buffer_boundary(&returned_xml, text, "€", 1);
            let reference = fixture.write(&format!("reference-cr-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-cr-{index}.xml"), &returned_xml);

            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            )
            .unwrap_or_else(|error| panic!("split UTF-8/CR variant {index} failed: {error}"));
        }
    }

    #[test]
    fn local_xml_parser_rejects_forbidden_characters_and_sequences() {
        let fixture = Fixture::new("xml-well-formedness");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let invalid_documents = [
            reference_xml.replacen("0 0 0</P>", "0\0 0 0</P>", 1),
            reference_xml.replacen("name=\"Ground\"", "name=\"Ground\u{1}\"", 1),
            reference_xml.replacen("0 0 0</P>", "0 0 0]]></P>", 1),
            reference_xml.replacen("0 0 0</P>", "0 0 &#x1;</P>", 1),
            reference_xml.replacen(
                "<Application name=\"Declared elsewhere\" version=\"ignored\"/>",
                "<!--foo---><Application name=\"Declared elsewhere\" version=\"ignored\"/>",
                1,
            ),
            format!("<![CDATA[]]>{reference_xml}"),
            format!("&#xA;{reference_xml}"),
            reference_xml.replacen("<Application ", "<Application xmlns:1bad=\"urn:x\" ", 1),
            reference_xml.replacen("<Application ", "<Application p:1bad=\"x\" ", 1),
            reference_xml.replacen("<Application name=", "<Application a=\"x\"b=\"y\" name=", 1),
            reference_xml.replacen("version=\"ignored\"/>", "version=\"ignored\"/ >", 1),
            reference_xml.replacen(
                "<Application name=",
                "<Application xmlns:p=\"urn:x\" xmlns:q=\"urn:x\" p:a=\"1\" q:a=\"2\" name=",
                1,
            ),
        ];
        for (index, invalid) in invalid_documents.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), invalid);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    default_limits(),
                ),
                RoundTripFailureKind::InvalidInput,
            );
        }
    }

    #[test]
    fn xml_declaration_is_well_formed_and_only_at_document_start() {
        let fixture = Fixture::new("xml-declaration");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let declaration = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>";
        let invalid_documents = [
            reference_xml.replacen(declaration, "<?xml rubbish?>", 1),
            reference_xml.replacen(declaration, "<?xml version=\"1.1\"?>", 1),
            reference_xml.replacen(declaration, "<?XML version=\"1.0\"?>", 1),
            format!(" \n{reference_xml}"),
            reference_xml.replacen("<LandXML", "<?xml version=\"1.0\"?>\n<LandXML", 1),
            reference_xml.replacen(
                declaration,
                "<?xml version=\"1.0\" standalone=\"yes\" encoding=\"UTF-8\"?>",
                1,
            ),
        ];
        for (index, invalid) in invalid_documents.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), invalid);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    default_limits(),
                ),
                RoundTripFailureKind::InvalidInput,
            );
        }
    }

    #[test]
    fn utf8_bom_is_accepted_only_once_at_byte_zero() {
        let fixture = Fixture::new("utf8-bom");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = format!("\u{feff}{reference_xml}");
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);
        verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("one UTF-8 BOM at byte zero is accepted");

        for (index, invalid) in [
            format!(" \u{feff}{reference_xml}"),
            format!("\u{feff}\u{feff}{reference_xml}"),
            reference_xml.replacen("<LandXML", "\u{feff}<LandXML", 1),
        ]
        .iter()
        .enumerate()
        {
            let reference =
                fixture.write(&format!("reference-invalid-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-invalid-{index}.xml"), invalid);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    default_limits(),
                ),
                RoundTripFailureKind::InvalidInput,
            );
        }
    }

    #[test]
    fn many_namespace_bindings_fail_before_local_arena_growth() {
        let fixture = Fixture::new("many-namespace-bindings");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let mut declarations = String::new();
        for index in 0..96 {
            write!(declarations, " xmlns:p{index}=\"urn:bounded:{index}\"")
                .expect("writing to String cannot fail");
        }
        let returned_xml = xml.replace(
            "<Application name=\"Declared elsewhere\" version=\"ignored\"/>",
            &format!(
                "<Application name=\"Declared elsewhere\" version=\"ignored\"{declarations}/>"
            ),
        );
        let (reference, returned) = fixture.write_pair(&xml, &returned_xml);
        let parser_floor = 2 * super::PARSER_READ_BUFFER_BYTES as u64
            + 2 * super::DEFAULT_MAX_XML_TOKEN_BYTES
            + std::mem::size_of::<super::ElementFrame>() as u64;

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits().with_working_limits(
                    super::DEFAULT_MAX_XML_TOKEN_BYTES,
                    parser_floor + 1024,
                    1024 * 1024,
                ),
            ),
            RoundTripFailureKind::ResourceLimit,
        );
    }

    #[test]
    fn nested_namespace_bindings_are_popped_with_their_element() {
        let fixture = Fixture::new("namespace-scope-pop");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = reference_xml.replacen(
            "<Application name=\"Declared elsewhere\" version=\"ignored\"/>",
            "<Application xmlns:local=\"urn:local\" name=\"Declared elsewhere\" version=\"ignored\"><local:child/></Application><local:escaped/>",
            1,
        );
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        assert_kind(
            verify(
                &reference,
                &returned,
                tolerances(0.0, 0.0),
                default_limits(),
            ),
            RoundTripFailureKind::InvalidInput,
        );
    }

    #[cfg(unix)]
    #[test]
    fn captured_input_pair_rejects_replacement_before_consumption() {
        let fixture = Fixture::new("captured-pair-replacement");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let (reference, returned) = fixture.write_pair(&xml, &xml);
        let limits = default_limits();
        let (reference_witness, returned_witness) =
            super::capture_file_pair(&reference, &returned, limits.file_bytes)
                .expect("capture both input witnesses");

        let replacement = fixture.write("replacement.xml", &xml.replace("Ground", "Return"));
        fs::rename(replacement, &returned).expect("replace returned path after capture");

        super::read_regular_file(
            super::InputSide::Reference,
            reference_witness,
            limits.file_bytes,
        )
        .expect("read unchanged reference witness");
        let Err(error) = super::read_regular_file(
            super::InputSide::Returned,
            returned_witness,
            limits.file_bytes,
        ) else {
            panic!("replacement after pair capture must fail");
        };
        assert_eq!(error.kind(), RoundTripFailureKind::InvalidInput, "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_rejected_and_hard_links_compare_by_content() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("file-identity");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let actual = fixture.write("actual.xml", &xml);
        let returned = fixture.write("returned.xml", &xml);
        let link = fixture.path("link.xml");
        symlink(&actual, &link).expect("create fixture symbolic link");
        assert_kind(
            verify(&link, &returned, tolerances(0.0, 0.0), default_limits()),
            RoundTripFailureKind::InvalidInput,
        );

        let hard_link = fixture.path("hard-link.xml");
        fs::hard_link(&actual, &hard_link).expect("create fixture hard link");
        let report = verify(&actual, &hard_link, tolerances(0.0, 0.0), default_limits())
            .expect("hard-linked inputs compare by captured content");
        assert!(report.exact_bytes());
    }

    #[test]
    fn the_same_resolved_path_compares_captured_content() {
        let fixture = Fixture::new("same-path");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let actual = fixture.write("actual.xml", &xml);
        let aliased = fixture.path(".").join("actual.xml");
        let report = verify(&actual, &aliased, tolerances(0.0, 0.0), default_limits())
            .expect("resolved path aliases compare by captured content");
        assert!(report.exact_bytes());
    }

    #[test]
    fn declaration_tolerances_and_diagnostics_are_bounded() {
        assert_kind(
            RoundTripDeclaration::new(" Civil 3D", "2026", "metric"),
            RoundTripFailureKind::InvalidInput,
        );
        assert_kind(
            RoundTripDeclaration::new("x".repeat(129), "2026", "metric"),
            RoundTripFailureKind::ResourceLimit,
        );
        for (horizontal, vertical) in [(-1.0, 0.0), (f64::NAN, 0.0), (0.0, f64::INFINITY)] {
            assert_kind(
                RoundTripTolerances::new(horizontal, vertical),
                RoundTripFailureKind::InvalidInput,
            );
        }
        let error = RoundTripDeclaration::new("x".repeat(2_000), "2026", "metric")
            .expect_err("oversized declaration fails");
        assert!(error.diagnostic().len() <= 1_024);
        assert!(error.to_string().len() <= 1_024 + "PRT_RESOURCE_LIMIT: ".len());
    }

    #[test]
    fn named_and_character_references_resolve_without_owned_text() {
        let mut encoded = [0_u8; 4];
        assert_eq!(
            super::resolve_reference(
                super::InputSide::Returned,
                &quick_xml::events::BytesRef::new("amp"),
                &mut encoded,
            )
            .unwrap(),
            "&"
        );
        assert_eq!(
            super::resolve_reference(
                super::InputSide::Returned,
                &quick_xml::events::BytesRef::new("#x1f600"),
                &mut encoded,
            )
            .unwrap(),
            "😀"
        );
    }

    fn verify(
        reference: &Path,
        returned: &Path,
        tolerances: RoundTripTolerances,
        limits: RoundTripLimits,
    ) -> Result<super::RoundTripReport, super::RoundTripFailure> {
        verify_landxml_round_trip(
            reference,
            returned,
            RoundTripDeclaration::new("generated-fixture", "test-only", "metric-tin-v1")
                .expect("valid declaration"),
            tolerances,
            limits,
        )
    }

    fn tolerances(horizontal: f64, vertical: f64) -> RoundTripTolerances {
        RoundTripTolerances::new(horizontal, vertical).expect("valid fixture tolerances")
    }

    fn default_limits() -> RoundTripLimits {
        RoundTripLimits::default()
    }

    fn assert_kind<T: std::fmt::Debug>(
        result: Result<T, super::RoundTripFailure>,
        expected: RoundTripFailureKind,
    ) {
        let error = result.expect_err("operation must fail");
        assert_eq!(error.kind(), expected, "{error}");
    }

    fn landxml(points: &[(&str, &str)], faces: &[&str], faces_first: bool) -> String {
        let mut point_xml = String::new();
        for (id, position) in points {
            writeln!(point_xml, "          <P id=\"{id}\">{position}</P>")
                .expect("writing to String cannot fail");
        }
        let mut face_xml = String::new();
        for face in faces {
            writeln!(face_xml, "          <F>{face}</F>").expect("writing to String cannot fail");
        }
        let pnts = format!("        <Pnts>\n{point_xml}        </Pnts>\n");
        let faces = format!("        <Faces>\n{face_xml}        </Faces>\n");
        let definition_children = if faces_first {
            format!("{faces}{pnts}")
        } else {
            format!("{pnts}{faces}")
        };
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <LandXML xmlns=\"{namespace}\" version=\"1.2\" date=\"2026-08-11\" time=\"00:00:00Z\">\n\
             <Application name=\"Declared elsewhere\" version=\"ignored\"/>\n\
             <Units><Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\"/></Units>\n\
             <Surfaces><Surface name=\"Ground\"><Definition surfType=\"TIN\">\n\
             {definition_children}\
             </Definition></Surface></Surfaces>\n\
             </LandXML>\n",
            namespace = super::LANDXML_NAMESPACE,
        )
    }

    fn align_marker_at_buffer_boundary(
        xml: &str,
        insertion_marker: &str,
        target_marker: &str,
        bytes_before_boundary: usize,
    ) -> String {
        let insertion = xml.find(insertion_marker).expect("insertion marker exists");
        let target = xml.find(target_marker).expect("target marker exists");
        assert!(target >= insertion);
        let desired = super::PARSER_READ_BUFFER_BYTES - bytes_before_boundary;
        let padding_len = (desired + super::PARSER_READ_BUFFER_BYTES
            - target % super::PARSER_READ_BUFFER_BYTES)
            % super::PARSER_READ_BUFFER_BYTES;
        let mut padding = "<!--p-->".repeat(padding_len / 8);
        padding.push_str(&" ".repeat(padding_len % 8));
        let mut aligned = String::with_capacity(xml.len() + padding.len());
        aligned.push_str(&xml[..insertion]);
        aligned.push_str(&padding);
        aligned.push_str(&xml[insertion..]);
        assert_eq!(
            (target + padding_len) % super::PARSER_READ_BUFFER_BYTES,
            desired
        );
        aligned
    }

    struct Fixture {
        directory: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "punctra-roundtrip-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&directory).expect("create isolated fixture directory");
            Self { directory }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.join(name)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, contents).expect("write fixture XML");
            path
        }

        fn write_pair(&self, reference: &str, returned: &str) -> (PathBuf, PathBuf) {
            (
                self.write("reference.xml", reference),
                self.write("returned.xml", returned),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.directory).expect("remove fixture directory");
        }
    }
}
