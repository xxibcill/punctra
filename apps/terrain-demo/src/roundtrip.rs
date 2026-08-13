//! Private, bounded semantic verification for returned `LandXML` terrain files.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    fs::{self, File, Metadata},
    io::{Read as _, Seek as _, SeekFrom},
    path::Path,
};

use foundation_runtime::OperationControl;
use num_bigint::BigInt;
use quick_xml::events::{BytesDecl, Event};
use robust::{Coord, orient2d};
use roxmltree::{Document, Node, ParsingOptions};

use crate::{
    bounded_diagnostic::BoundedDiagnostic,
    diagnostic::{FailureCode, RecoveryAction},
};

const LANDXML_NAMESPACE: &str = "http://www.landxml.org/schema/LandXML-1.2";
const XINCLUDE_NAMESPACE: &str = "http://www.w3.org/2001/XInclude";
const MAX_APPLICATION_BYTES: usize = 128;
const MAX_VERSION_BYTES: usize = 128;
const MAX_SETTINGS_PROFILE_BYTES: usize = 1_024;

const DEFAULT_MAX_FILE_BYTES: u64 = 256 * 1_024 * 1_024;
const DEFAULT_MAX_XML_NODES: u64 = 8_000_000;
const DEFAULT_MAX_XML_TEXT_BYTES: u64 = 256 * 1_024 * 1_024;
const DEFAULT_MAX_POINTS: u64 = 2_000_000;
const DEFAULT_MAX_FACES: u64 = 4_000_000;
const DEFAULT_MAX_COMPARISONS: u64 = 32_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputSide {
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
    application: Box<str>,
    version: Box<str>,
    settings_profile: Box<str>,
}

impl RoundTripDeclaration {
    pub(crate) fn new(
        application: impl Into<String>,
        version: impl Into<String>,
        settings_profile: impl Into<String>,
    ) -> Result<Self, RoundTripFailure> {
        let application = application.into();
        let version = version.into();
        let settings_profile = settings_profile.into();
        validate_declaration_field("application", &application, MAX_APPLICATION_BYTES)?;
        validate_declaration_field("version", &version, MAX_VERSION_BYTES)?;
        validate_declaration_field(
            "settings profile",
            &settings_profile,
            MAX_SETTINGS_PROFILE_BYTES,
        )?;
        Ok(Self {
            application: application.into_boxed_str(),
            version: version.into_boxed_str(),
            settings_profile: settings_profile.into_boxed_str(),
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
    points: u64,
    faces: u64,
    comparisons: u64,
}

impl RoundTripLimits {
    pub(crate) const fn full_v07_export() -> Self {
        Self {
            file_bytes: 4 * 1024 * 1024 * 1024,
            xml_nodes: 60_000_000,
            xml_text_bytes: 4 * 1024 * 1024 * 1024,
            points: 10_000_000,
            faces: 20_000_000,
            comparisons: 160_000_000,
        }
    }

    #[cfg(test)]
    pub(crate) const fn new(
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
            points: max_points,
            faces: max_faces,
            comparisons: max_comparisons,
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

    pub(crate) const fn points(self) -> u64 {
        self.points
    }

    pub(crate) const fn faces(self) -> u64 {
        self.faces
    }

    pub(crate) const fn comparisons(self) -> u64 {
        self.comparisons
    }
}

impl Default for RoundTripLimits {
    fn default() -> Self {
        Self {
            file_bytes: DEFAULT_MAX_FILE_BYTES,
            xml_nodes: DEFAULT_MAX_XML_NODES,
            xml_text_bytes: DEFAULT_MAX_XML_TEXT_BYTES,
            points: DEFAULT_MAX_POINTS,
            faces: DEFAULT_MAX_FACES,
            comparisons: DEFAULT_MAX_COMPARISONS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoundTripFailureKind {
    InvalidInput,
    ResourceLimit,
    SemanticMismatch,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoundTripReason {
    XmlInvalid,
    SubsetUnsupported,
    CoordinateReferenceUnsupported,
    UnitDrift,
    PointCountDrift,
    VertexUnmatched,
    VertexAmbiguous,
    ToleranceDrift,
    TopologyDrift,
}

impl RoundTripReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::XmlInvalid => "PRT_XML_INVALID",
            Self::SubsetUnsupported => "PRT_SUBSET_UNSUPPORTED",
            Self::CoordinateReferenceUnsupported => "PRT_COORDINATE_REFERENCE_UNSUPPORTED",
            Self::UnitDrift => "PRT_UNIT_DRIFT",
            Self::PointCountDrift => "PRT_POINT_COUNT_DRIFT",
            Self::VertexUnmatched => "PRT_VERTEX_UNMATCHED",
            Self::VertexAmbiguous => "PRT_VERTEX_AMBIGUOUS",
            Self::ToleranceDrift => "PRT_TOLERANCE_DRIFT",
            Self::TopologyDrift => "PRT_TOPOLOGY_DRIFT",
        }
    }

    pub(crate) const fn failure_code(self) -> FailureCode {
        match self {
            Self::XmlInvalid => FailureCode::RoundTripXmlInvalid,
            Self::SubsetUnsupported => FailureCode::RoundTripSubsetUnsupported,
            Self::CoordinateReferenceUnsupported => {
                FailureCode::RoundTripCoordinateReferenceUnsupported
            }
            Self::UnitDrift => FailureCode::RoundTripUnitDrift,
            Self::PointCountDrift => FailureCode::RoundTripPointCountDrift,
            Self::VertexUnmatched => FailureCode::RoundTripVertexUnmatched,
            Self::VertexAmbiguous => FailureCode::RoundTripVertexAmbiguous,
            Self::ToleranceDrift => FailureCode::RoundTripToleranceDrift,
            Self::TopologyDrift => FailureCode::RoundTripTopologyDrift,
        }
    }
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
            Self::Cancelled => (FailureCode::Cancelled, RecoveryAction::ResumeSameRun),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoundTripFailure {
    kind: RoundTripFailureKind,
    reason: Option<RoundTripReason>,
    topology: Option<Box<TopologyDrift>>,
    comparison: Option<ComparisonFacts>,
    diagnostic: BoundedDiagnostic,
}

impl RoundTripFailure {
    pub(crate) fn invalid(error: impl fmt::Display) -> Self {
        Self::new(RoundTripFailureKind::InvalidInput, error)
    }

    pub(crate) fn resource(error: impl fmt::Display) -> Self {
        Self::new(RoundTripFailureKind::ResourceLimit, error)
    }

    fn new(kind: RoundTripFailureKind, error: impl fmt::Display) -> Self {
        Self {
            kind,
            reason: None,
            topology: None,
            comparison: None,
            diagnostic: BoundedDiagnostic::new(error),
        }
    }

    pub(crate) fn semantic(reason: RoundTripReason, error: impl fmt::Display) -> Self {
        Self {
            kind: RoundTripFailureKind::SemanticMismatch,
            reason: Some(reason),
            topology: None,
            comparison: None,
            diagnostic: BoundedDiagnostic::new(error),
        }
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(
            RoundTripFailureKind::Cancelled,
            "round-trip cancellation was requested",
        )
    }

    fn topology(topology: TopologyDrift, error: impl fmt::Display) -> Self {
        Self {
            kind: RoundTripFailureKind::SemanticMismatch,
            reason: Some(RoundTripReason::TopologyDrift),
            topology: Some(Box::new(topology)),
            comparison: None,
            diagnostic: BoundedDiagnostic::new(error),
        }
    }

    pub(crate) const fn kind(&self) -> RoundTripFailureKind {
        self.kind
    }

    pub(crate) const fn reason(&self) -> Option<RoundTripReason> {
        self.reason
    }

    pub(crate) fn diagnostic(&self) -> &str {
        self.diagnostic.as_str()
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
    returned_surface_name: Option<Box<str>>,
    returned_ignored_sections: Box<[Box<str>]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoundTripMismatch {
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    reason: RoundTripReason,
    diagnostic: BoundedDiagnostic,
    reference_content_hash: [u8; 32],
    returned_content_hash: [u8; 32],
    reference_bytes: u64,
    returned_bytes: u64,
    topology: Option<Box<TopologyDrift>>,
    returned_surface_name: Option<Box<str>>,
    returned_ignored_sections: Box<[Box<str>]>,
    returned_point_count: Option<u64>,
    returned_face_count: Option<u64>,
    comparison: Option<ComparisonFacts>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RoundTripEvaluation {
    Passed(RoundTripReport),
    Failed(RoundTripMismatch),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RoundTripFileFacts {
    pub(crate) content_hash: [u8; 32],
    pub(crate) byte_length: u64,
}

pub(crate) struct ParsedRoundTrip {
    pub(crate) declaration: RoundTripDeclaration,
    pub(crate) tolerances: RoundTripTolerances,
    pub(crate) limits: RoundTripLimits,
    pub(crate) reference_facts: RoundTripFileFacts,
    pub(crate) returned_facts: RoundTripFileFacts,
    pub(crate) exact_bytes: bool,
    pub(crate) reference_surface: ParsedSurface,
    pub(crate) returned_surface: ParsedSurface,
}

impl RoundTripReport {
    pub(crate) fn declaration(&self) -> &RoundTripDeclaration {
        &self.declaration
    }

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

    pub(crate) fn returned_surface_name(&self) -> Option<&str> {
        self.returned_surface_name.as_deref()
    }

    pub(crate) fn returned_ignored_sections(&self) -> &[Box<str>] {
        &self.returned_ignored_sections
    }
}

impl RoundTripMismatch {
    pub(crate) fn declaration(&self) -> &RoundTripDeclaration {
        &self.declaration
    }

    pub(crate) const fn tolerances(&self) -> RoundTripTolerances {
        self.tolerances
    }

    pub(crate) const fn reason(&self) -> RoundTripReason {
        self.reason
    }

    pub(crate) fn diagnostic(&self) -> &str {
        self.diagnostic.as_str()
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

    pub(crate) fn topology(&self) -> Option<&TopologyDrift> {
        self.topology.as_deref()
    }

    pub(crate) const fn returned_was_parsed(&self) -> bool {
        self.returned_point_count.is_some()
    }

    pub(crate) fn returned_surface_name(&self) -> Option<&str> {
        self.returned_surface_name.as_deref()
    }

    pub(crate) fn returned_ignored_sections(&self) -> &[Box<str>] {
        &self.returned_ignored_sections
    }

    pub(crate) const fn returned_point_count(&self) -> Option<u64> {
        self.returned_point_count
    }

    pub(crate) const fn returned_face_count(&self) -> Option<u64> {
        self.returned_face_count
    }

    pub(crate) const fn completed_mapping_point_count(&self) -> Option<u64> {
        if self.comparison.is_some() {
            self.returned_point_count
        } else {
            None
        }
    }

    pub(crate) fn completed_mapping_maximum_deltas(&self) -> Option<(f64, f64)> {
        self.comparison.map(|comparison| {
            (
                comparison.max_horizontal_drift_metres,
                comparison.max_vertical_drift_metres,
            )
        })
    }
}

impl RoundTripEvaluation {
    pub(crate) fn reason(&self) -> Option<RoundTripReason> {
        match self {
            Self::Passed(_) => None,
            Self::Failed(mismatch) => Some(mismatch.reason()),
        }
    }
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
        RoundTripEvaluation::Failed(mismatch) => Err(RoundTripFailure::semantic(
            mismatch.reason(),
            mismatch.diagnostic(),
        )),
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
    let (reference_witness, returned_witness) =
        capture_file_pair(reference_path, returned_path, limits.file_bytes)?;
    let reference = read_regular_file(InputSide::Reference, reference_witness, limits.file_bytes)?;
    let returned = read_regular_file(InputSide::Returned, returned_witness, limits.file_bytes)?;
    let reference_hash = *blake3::hash(&reference.bytes).as_bytes();
    let returned_hash = *blake3::hash(&returned.bytes).as_bytes();
    let evaluated = (|| {
        let reference_surface = parse_surface(InputSide::Reference, &reference.bytes, limits)?;
        let returned_surface = parse_surface(InputSide::Returned, &returned.bytes, limits)?;
        let comparison = compare_surfaces(
            &reference_surface,
            &returned_surface,
            tolerances,
            limits.comparisons,
            None,
        )?;
        Ok::<_, RoundTripFailure>((reference_surface, returned_surface, comparison))
    })();
    match evaluated {
        Ok((reference_surface, returned_surface, comparison)) => {
            Ok(RoundTripEvaluation::Passed(RoundTripReport {
                declaration,
                tolerances,
                reference_content_hash: reference_hash,
                returned_content_hash: returned_hash,
                reference_bytes: reference.bytes.len() as u64,
                returned_bytes: returned.bytes.len() as u64,
                vertex_count: reference_surface.points.len() as u64,
                face_count: reference_surface.faces.len() as u64,
                comparison_count: comparison.comparison_count,
                max_easting_drift_metres: comparison.max_easting_drift_metres,
                max_northing_drift_metres: comparison.max_northing_drift_metres,
                max_horizontal_drift_metres: comparison.max_horizontal_drift_metres,
                max_vertical_drift_metres: comparison.max_vertical_drift_metres,
                exact_bytes: reference.bytes == returned.bytes,
                topology_matches: true,
                returned_surface_name: returned_surface.surface_name,
                returned_ignored_sections: returned_surface.ignored_top_level_sections,
            }))
        }
        Err(error)
            if error.kind() == RoundTripFailureKind::SemanticMismatch
                && error.reason().is_some() =>
        {
            Ok(RoundTripEvaluation::Failed(RoundTripMismatch {
                declaration,
                tolerances,
                reason: error.reason().expect("guarded semantic reason"),
                diagnostic: BoundedDiagnostic::new(error.diagnostic()),
                reference_content_hash: reference_hash,
                returned_content_hash: returned_hash,
                reference_bytes: reference.bytes.len() as u64,
                returned_bytes: returned.bytes.len() as u64,
                topology: error.topology,
                returned_point_count: None,
                returned_face_count: None,
                comparison: error.comparison,
                returned_surface_name: None,
                returned_ignored_sections: Box::default(),
            }))
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn evaluate_parsed_round_trip(
    input: ParsedRoundTrip,
    control: Option<&OperationControl>,
) -> Result<RoundTripEvaluation, RoundTripFailure> {
    let ParsedRoundTrip {
        declaration,
        tolerances,
        limits,
        reference_facts,
        returned_facts,
        exact_bytes,
        reference_surface,
        returned_surface,
    } = input;
    match compare_surfaces(
        &reference_surface,
        &returned_surface,
        tolerances,
        limits.comparisons,
        control,
    ) {
        Ok(comparison) => Ok(RoundTripEvaluation::Passed(RoundTripReport {
            declaration,
            tolerances,
            reference_content_hash: reference_facts.content_hash,
            returned_content_hash: returned_facts.content_hash,
            reference_bytes: reference_facts.byte_length,
            returned_bytes: returned_facts.byte_length,
            vertex_count: reference_surface.points.len() as u64,
            face_count: reference_surface.faces.len() as u64,
            comparison_count: comparison.comparison_count,
            max_easting_drift_metres: comparison.max_easting_drift_metres,
            max_northing_drift_metres: comparison.max_northing_drift_metres,
            max_horizontal_drift_metres: comparison.max_horizontal_drift_metres,
            max_vertical_drift_metres: comparison.max_vertical_drift_metres,
            exact_bytes,
            topology_matches: true,
            returned_surface_name: returned_surface.surface_name,
            returned_ignored_sections: returned_surface.ignored_top_level_sections,
        })),
        Err(error)
            if error.kind() == RoundTripFailureKind::SemanticMismatch
                && error.reason().is_some() =>
        {
            Ok(RoundTripEvaluation::Failed(RoundTripMismatch {
                declaration,
                tolerances,
                reason: error.reason().expect("guarded semantic reason"),
                diagnostic: BoundedDiagnostic::new(error.diagnostic()),
                reference_content_hash: reference_facts.content_hash,
                returned_content_hash: returned_facts.content_hash,
                reference_bytes: reference_facts.byte_length,
                returned_bytes: returned_facts.byte_length,
                topology: error.topology,
                returned_point_count: Some(returned_surface.points.len() as u64),
                returned_face_count: Some(returned_surface.faces.len() as u64),
                comparison: error.comparison,
                returned_surface_name: returned_surface.surface_name,
                returned_ignored_sections: returned_surface.ignored_top_level_sections,
            }))
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn semantic_evaluation_failure(
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    reference_facts: RoundTripFileFacts,
    returned_facts: RoundTripFileFacts,
    error: RoundTripFailure,
    returned_surface: Option<ParsedSurface>,
) -> Result<RoundTripEvaluation, RoundTripFailure> {
    if error.kind() != RoundTripFailureKind::SemanticMismatch || error.reason().is_none() {
        return Err(error);
    }
    let returned_point_count = returned_surface
        .as_ref()
        .map(|surface| surface.points.len() as u64);
    let returned_face_count = returned_surface
        .as_ref()
        .map(|surface| surface.faces.len() as u64);
    let (returned_surface_name, returned_ignored_sections) = returned_surface.map_or_else(
        || (None, Box::default()),
        |surface| (surface.surface_name, surface.ignored_top_level_sections),
    );
    Ok(RoundTripEvaluation::Failed(RoundTripMismatch {
        declaration,
        tolerances,
        reason: error.reason().expect("guarded semantic reason"),
        diagnostic: BoundedDiagnostic::new(error.diagnostic()),
        reference_content_hash: reference_facts.content_hash,
        returned_content_hash: returned_facts.content_hash,
        reference_bytes: reference_facts.byte_length,
        returned_bytes: returned_facts.byte_length,
        topology: error.topology,
        returned_surface_name,
        returned_ignored_sections,
        returned_point_count,
        returned_face_count,
        comparison: error.comparison,
    }))
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

fn validate_tolerance(axis: &str, value: f64) -> Result<(), RoundTripFailure> {
    if !value.is_finite() || value < 0.0 {
        return Err(RoundTripFailure::invalid(format_args!(
            "{axis} tolerance must be finite and nonnegative metres"
        )));
    }
    Ok(())
}

fn validate_limits(limits: RoundTripLimits) -> Result<(), RoundTripFailure> {
    if limits.xml_nodes == 0 || limits.xml_nodes > u64::from(u32::MAX) {
        return Err(RoundTripFailure::invalid(
            "XML node limit must be between 1 and u32::MAX",
        ));
    }
    if limits.file_bytes == u64::MAX {
        return Err(RoundTripFailure::invalid(
            "file-byte limit must leave room for an over-limit sentinel byte",
        ));
    }
    Ok(())
}

struct FileSnapshot {
    bytes: Vec<u8>,
}

pub(crate) struct CapturedRoundTripFile {
    pub(crate) bytes: Vec<u8>,
    witness: RetainedFileWitness,
}

impl CapturedRoundTripFile {
    pub(crate) fn verify(&self) -> Result<(), RoundTripFailure> {
        self.witness.verify()
    }
}

pub(crate) fn capture_round_trip_file(
    path: &Path,
    max_file_bytes: u64,
) -> Result<CapturedRoundTripFile, RoundTripFailure> {
    let witness = capture_regular_file(InputSide::Returned, path, max_file_bytes)?;
    let (snapshot, witness) =
        read_regular_file_retained(InputSide::Returned, witness, max_file_bytes)?;
    Ok(CapturedRoundTripFile {
        bytes: snapshot.bytes,
        witness,
    })
}

struct RetainedFileWitness {
    side: InputSide,
    path: std::path::PathBuf,
    file: File,
    identity: Metadata,
}

impl RetainedFileWitness {
    fn verify(&self) -> Result<(), RoundTripFailure> {
        let opened = self.file.metadata().map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} metadata cannot be rechecked: {error}",
                self.side
            ))
        })?;
        let current = fs::symlink_metadata(&self.path).map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} path cannot be rechecked: {error}",
                self.side
            ))
        })?;
        if !same_file_state(&self.identity, &opened)
            || current.file_type().is_symlink()
            || !current.is_file()
            || !same_file_state(&self.identity, &current)
        {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} changed after it was captured",
                self.side
            )));
        }
        Ok(())
    }
}

struct FileWitness<'a> {
    path: &'a Path,
    file: File,
    metadata: Metadata,
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
    capture_inspected_regular_file(side, path, &path_metadata, max_file_bytes)
}

fn capture_inspected_regular_file<'a>(
    side: InputSide,
    path: &'a Path,
    path_metadata: &Metadata,
    max_file_bytes: u64,
) -> Result<FileWitness<'a>, RoundTripFailure> {
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} must be a regular file and not a symbolic link"
        )));
    }
    let path_identity = require_file_identity(side, path_metadata)?;
    check_file_bytes(side, path_metadata.len(), max_file_bytes)?;

    let file = open_input_file(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be opened: {error}"))
    })?;
    #[cfg(windows)]
    require_disk_file(side, &file)?;
    let open_metadata = file.metadata().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} metadata cannot be read: {error}"))
    })?;
    let open_identity = require_file_identity(side, &open_metadata)?;
    if !open_metadata.is_file()
        || path_identity != open_identity
        || !same_file_state(path_metadata, &open_metadata)
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
fn open_input_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(windows)]
fn open_input_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_input_file(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "stable no-follow input capture is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn require_disk_file(side: InputSide, file: &File) -> Result<(), RoundTripFailure> {
    let file_type = winapi_util::file::typ(file).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} handle type cannot be read: {error}"))
    })?;
    if !file_type.is_disk() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} must use a disk-backed regular file"
        )));
    }
    Ok(())
}

fn read_regular_file(
    side: InputSide,
    witness: FileWitness<'_>,
    max_file_bytes: u64,
) -> Result<FileSnapshot, RoundTripFailure> {
    read_regular_file_retained(side, witness, max_file_bytes).map(|(snapshot, _)| snapshot)
}

fn read_regular_file_retained(
    side: InputSide,
    witness: FileWitness<'_>,
    max_file_bytes: u64,
) -> Result<(FileSnapshot, RetainedFileWitness), RoundTripFailure> {
    let FileWitness {
        path,
        mut file,
        metadata: open_metadata,
    } = witness;
    let bytes = read_bounded_bytes(side, &mut file, open_metadata.len(), max_file_bytes)?;
    let final_metadata = file.metadata().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} metadata cannot be rechecked: {error}"))
    })?;
    let final_path_metadata = fs::symlink_metadata(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} path cannot be rechecked: {error}"))
    })?;
    let expected_bytes = open_metadata.len();
    if bytes.len() as u64 != expected_bytes
        || !same_file_state(&open_metadata, &final_metadata)
        || final_path_metadata.file_type().is_symlink()
        || !final_path_metadata.is_file()
        || !same_file_state(&open_metadata, &final_path_metadata)
    {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read"
        )));
    }
    Ok((
        FileSnapshot { bytes },
        RetainedFileWitness {
            side,
            path: path.to_path_buf(),
            file,
            identity: final_metadata,
        },
    ))
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

fn read_bounded_bytes(
    side: InputSide,
    file: &mut File,
    expected_bytes: u64,
    max_file_bytes: u64,
) -> Result<Vec<u8>, RoundTripFailure> {
    let capacity = usize::try_from(expected_bytes).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} file length does not fit this platform"
        ))
    })?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} file buffer cannot reserve {expected_bytes} bytes"
        ))
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be rewound: {error}"))
    })?;
    file.take(max_file_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            RoundTripFailure::invalid(format_args!("{side} cannot be read: {error}"))
        })?;
    check_file_bytes(side, bytes.len() as u64, max_file_bytes)?;
    Ok(bytes)
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
pub(crate) struct Position {
    pub(crate) easting: f64,
    pub(crate) northing: f64,
    pub(crate) elevation: f64,
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
pub(crate) struct Triangle {
    first: usize,
    second: usize,
    third: usize,
}

impl Triangle {
    pub(crate) const fn new(first: usize, second: usize, third: usize) -> Self {
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
        [points[self.first], points[self.second], points[self.third]]
    }

    fn canonical_point_indices(self) -> [usize; 3] {
        let mut indices = [self.first, self.second, self.third];
        indices.sort_unstable();
        indices
    }

    fn remap(self, point_mapping: &[usize]) -> Self {
        Self::new(
            point_mapping[self.first],
            point_mapping[self.second],
            point_mapping[self.third],
        )
    }
}

#[derive(Debug)]
pub(crate) struct ParsedSurface {
    pub(crate) points: Vec<Position>,
    pub(crate) faces: Vec<Triangle>,
    pub(crate) surface_name: Option<Box<str>>,
    pub(crate) ignored_top_level_sections: Box<[Box<str>]>,
}

fn parse_surface(
    side: InputSide,
    bytes: &[u8],
    limits: RoundTripLimits,
) -> Result<ParsedSurface, RoundTripFailure> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} is not UTF-8 XML: {error}"),
        )
    })?;
    let mut declaration_reader = quick_xml::Reader::from_str(text);
    if let Ok(Event::Decl(declaration)) = declaration_reader.read_event() {
        validate_utf8_declaration(side, &declaration)?;
    }
    let nodes_limit = u32::try_from(limits.xml_nodes)
        .map_err(|_| RoundTripFailure::invalid("XML node limit exceeds parser capacity"))?;
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit,
        entity_resolver: None,
    };
    let document = Document::parse_with_options(text, options).map_err(|error| {
        if matches!(error, roxmltree::Error::NodesLimitReached) {
            RoundTripFailure::resource(format_args!(
                "{side} XML nodes exceed the {} node limit",
                limits.xml_nodes
            ))
        } else {
            RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} XML is malformed: {error}"),
            )
        }
    })?;
    reject_xinclude(side, &document)?;
    check_xml_text_bytes(side, text, &document, limits.xml_text_bytes)?;
    parse_landxml_document(side, &document, limits)
}

pub(crate) fn validate_utf8_declaration(
    side: InputSide,
    declaration: &BytesDecl<'_>,
) -> Result<(), RoundTripFailure> {
    let Some(encoding) = declaration.encoding() else {
        return Ok(());
    };
    let encoding = encoding.map_err(|error| {
        RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} XML declaration is invalid: {error}"),
        )
    })?;
    if encoding.eq_ignore_ascii_case(b"UTF-8") {
        Ok(())
    } else {
        Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} XML declaration does not specify UTF-8"),
        ))
    }
}

fn reject_xinclude(side: InputSide, document: &Document<'_>) -> Result<(), RoundTripFailure> {
    if document
        .descendants()
        .any(|node| node.is_element() && node.tag_name().namespace() == Some(XINCLUDE_NAMESPACE))
    {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::SubsetUnsupported,
            format_args!("{side} must not contain XInclude elements"),
        ));
    }
    Ok(())
}

fn check_xml_text_bytes(
    side: InputSide,
    source: &str,
    document: &Document<'_>,
    allowed: u64,
) -> Result<(), RoundTripFailure> {
    let mut total = 0_u64;
    for node in document.descendants() {
        if let Some(text) = node.text().filter(|_| node.is_text()) {
            total = add_text_bytes(side, total, text.len(), allowed)?;
        }
        if node.is_element() {
            total = add_text_bytes(
                side,
                total,
                element_attribute_bytes(side, source, node)?,
                allowed,
            )?;
        }
    }
    Ok(())
}

fn element_attribute_bytes(
    side: InputSide,
    source: &str,
    node: Node<'_, '_>,
) -> Result<usize, RoundTripFailure> {
    let range = node.range();
    let bytes = source.as_bytes();
    let mut cursor = range.start.saturating_add(1);
    while cursor < range.end
        && !bytes[cursor].is_ascii_whitespace()
        && bytes[cursor] != b'/'
        && bytes[cursor] != b'>'
    {
        cursor += 1;
    }
    let attributes_start = cursor;
    let mut quote = None;
    while cursor < range.end {
        match (quote, bytes[cursor]) {
            (Some(expected), actual) if actual == expected => quote = None,
            (None, actual @ (b'\'' | b'"')) => quote = Some(actual),
            (None, b'>') => break,
            (Some(_) | None, _) => {}
        }
        cursor += 1;
    }
    if cursor == range.end {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} XML element has no complete start tag"
        )));
    }
    let mut attributes_end = cursor;
    while attributes_end > attributes_start && bytes[attributes_end - 1].is_ascii_whitespace() {
        attributes_end -= 1;
    }
    if attributes_end > attributes_start && bytes[attributes_end - 1] == b'/' {
        attributes_end -= 1;
        while attributes_end > attributes_start && bytes[attributes_end - 1].is_ascii_whitespace() {
            attributes_end -= 1;
        }
    }
    Ok(attributes_end.saturating_sub(attributes_start))
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

fn parse_landxml_document(
    side: InputSide,
    document: &Document<'_>,
    limits: RoundTripLimits,
) -> Result<ParsedSurface, RoundTripFailure> {
    let root = document.root_element();
    require_tag(side, root, "LandXML")?;
    if unqualified_attribute(root, "version") != Some("1.2") {
        return Err(schema_error(side, "LandXML version must be 1.2"));
    }
    validate_root_children(side, root)?;
    let units = unique_child(side, root, "Units").map_err(|_| {
        RoundTripFailure::semantic(
            RoundTripReason::UnitDrift,
            format_args!("{side} must contain exactly one explicit metric-metre unit declaration"),
        )
    })?;
    validate_metric_units(side, units)?;
    let surfaces = unique_child(side, root, "Surfaces")?;
    validate_allowed_children(side, surfaces, &["Surface"])?;
    let surface = unique_child(side, surfaces, "Surface")?;
    let mut parsed = validate_surface(side, surface, limits)?;
    parsed.ignored_top_level_sections = element_children(root)
        .filter(|node| {
            node.has_tag_name((LANDXML_NAMESPACE, "Project"))
                || node.has_tag_name((LANDXML_NAMESPACE, "Application"))
        })
        .map(|node| node.tag_name().name().to_owned().into_boxed_str())
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(parsed)
}

fn validate_root_children(side: InputSide, root: Node<'_, '_>) -> Result<(), RoundTripFailure> {
    if element_children(root).any(|node| node.has_tag_name((LANDXML_NAMESPACE, "CoordinateSystem")))
    {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::CoordinateReferenceUnsupported,
            format_args!("{side} CoordinateSystem semantics are unsupported"),
        ));
    }
    validate_allowed_children(side, root, &["Units", "Project", "Application", "Surfaces"])?;
    unique_child(side, root, "Surfaces")?;
    at_most_one_child(side, root, "Project")?;
    at_most_one_child(side, root, "Application")?;
    Ok(())
}

fn validate_metric_units(side: InputSide, units: Node<'_, '_>) -> Result<(), RoundTripFailure> {
    validate_allowed_children(side, units, &["Metric"]).map_err(|_| unit_drift(side))?;
    let metric = unique_child(side, units, "Metric").map_err(|_| unit_drift(side))?;
    validate_allowed_children(side, metric, &[]).map_err(|_| unit_drift(side))?;
    if unqualified_attribute(metric, "linearUnit") != Some("meter") {
        return Err(unit_drift(side));
    }
    Ok(())
}

fn unit_drift(side: InputSide) -> RoundTripFailure {
    RoundTripFailure::semantic(
        RoundTripReason::UnitDrift,
        format_args!("{side} units do not declare exactly one Metric linearUnit=\"meter\""),
    )
}

fn validate_surface(
    side: InputSide,
    surface: Node<'_, '_>,
    limits: RoundTripLimits,
) -> Result<ParsedSurface, RoundTripFailure> {
    let Some(surface_name) = unqualified_attribute(surface, "name") else {
        return Err(schema_error(side, "Surface requires a name attribute"));
    };
    validate_allowed_children(side, surface, &["Definition"])?;
    let definition = unique_child(side, surface, "Definition")?;
    if unqualified_attribute(definition, "surfType") != Some("TIN") {
        return Err(schema_error(side, "Surface Definition must be a TIN"));
    }
    validate_allowed_children(side, definition, &["Pnts", "Faces"])?;
    let pnts = unique_child(side, definition, "Pnts")?;
    let faces = unique_child(side, definition, "Faces")?;
    let (points, point_ids) = parse_points(side, pnts, limits.points)?;
    let faces = parse_faces(side, faces, &points, &point_ids, limits.faces)?;
    if points.len() < 3 || faces.is_empty() {
        return Err(schema_error(
            side,
            "TIN requires at least three points and one face",
        ));
    }
    Ok(ParsedSurface {
        points,
        faces,
        surface_name: (!surface_name.is_empty()).then(|| surface_name.to_owned().into_boxed_str()),
        ignored_top_level_sections: Box::new([]),
    })
}

fn parse_points(
    side: InputSide,
    pnts: Node<'_, '_>,
    max_points: u64,
) -> Result<(Vec<Position>, BTreeMap<u64, usize>), RoundTripFailure> {
    validate_allowed_children(side, pnts, &["P"])?;
    let point_count = element_children(pnts).count();
    check_item_limit(side, "points", point_count, max_points)?;
    let mut points = Vec::new();
    let mut point_ids = BTreeMap::new();
    points.try_reserve_exact(point_count).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} point storage cannot reserve {point_count} entries"
        ))
    })?;
    for node in element_children(pnts) {
        let id = parse_point_id(side, node)?;
        let index = points.len();
        if point_ids.insert(id, index).is_some() {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} contains duplicate point ID {id}"),
            ));
        }
        points.push(parse_position(side, node)?);
    }
    Ok((points, point_ids))
}

fn parse_point_id(side: InputSide, node: Node<'_, '_>) -> Result<u64, RoundTripFailure> {
    let value = unqualified_attribute(node, "id")
        .ok_or_else(|| schema_error(side, "every P requires an id"))?;
    let id = value.parse::<u64>().map_err(|_| {
        RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} point ID must be a positive integer"),
        )
    })?;
    if id == 0 {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} point ID must be positive"),
        ));
    }
    Ok(id)
}

fn parse_position(side: InputSide, node: Node<'_, '_>) -> Result<Position, RoundTripFailure> {
    let text = simple_text(side, node, "P")?;
    let mut values = text.split_whitespace();
    let northing = parse_coordinate(side, values.next())?;
    let easting = parse_coordinate(side, values.next())?;
    let elevation = parse_coordinate(side, values.next())?;
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
        return Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} P coordinates must be finite"),
        ));
    }
    Ok(Position {
        easting: canonical_zero(easting),
        northing: canonical_zero(northing),
        elevation: canonical_zero(elevation),
    })
}

fn parse_coordinate(side: InputSide, value: Option<&str>) -> Result<f64, RoundTripFailure> {
    value
        .ok_or_else(|| schema_error(side, "P must contain exactly northing easting elevation"))?
        .parse()
        .map_err(|_| {
            RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} P coordinates are invalid"),
            )
        })
}

fn parse_faces(
    side: InputSide,
    faces: Node<'_, '_>,
    points: &[Position],
    point_ids: &BTreeMap<u64, usize>,
    max_faces: u64,
) -> Result<Vec<Triangle>, RoundTripFailure> {
    validate_allowed_children(side, faces, &["F"])?;
    let face_count = element_children(faces).count();
    check_item_limit(side, "faces", face_count, max_faces)?;
    let mut parsed_faces = Vec::new();
    parsed_faces.try_reserve_exact(face_count).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} face storage cannot reserve {face_count} entries"
        ))
    })?;
    for node in element_children(faces) {
        let face = parse_face(side, node, point_ids)?;
        validate_face(side, face, points)?;
        parsed_faces.push(face);
    }
    reject_duplicate_faces(side, &mut parsed_faces)?;
    Ok(parsed_faces)
}

fn parse_face(
    side: InputSide,
    node: Node<'_, '_>,
    point_ids: &BTreeMap<u64, usize>,
) -> Result<Triangle, RoundTripFailure> {
    let text = simple_text(side, node, "F")?;
    let mut ids = text.split_whitespace();
    let a = parse_face_id(side, ids.next())?;
    let b = parse_face_id(side, ids.next())?;
    let c = parse_face_id(side, ids.next())?;
    if ids.next().is_some() {
        return Err(schema_error(side, "F must contain exactly three point IDs"));
    }
    let resolve = |id| {
        point_ids.get(&id).copied().ok_or_else(|| {
            RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} face has dangling point reference {id}"),
            )
        })
    };
    Ok(Triangle::new(resolve(a)?, resolve(b)?, resolve(c)?))
}

fn parse_face_id(side: InputSide, value: Option<&str>) -> Result<u64, RoundTripFailure> {
    value
        .ok_or_else(|| schema_error(side, "F must contain exactly three point IDs"))?
        .parse()
        .map_err(|_| {
            RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} F references are invalid"),
            )
        })
}

pub(crate) fn validate_face(
    side: InputSide,
    face: Triangle,
    points: &[Position],
) -> Result<(), RoundTripFailure> {
    if face.has_repeated_point() {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} contains a face with repeated point references"),
        ));
    }
    let [a, b, c] = face.positions(points);
    let robust_orientation = normalized_orientation_xy(a, b, c);
    let is_collinear = match robust_orientation {
        Some(orientation) if orientation != 0.0 => false,
        Some(_) | None => exact_orientation_is_zero(a, b, c),
    };
    if is_collinear {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} contains a geometrically degenerate face"),
        ));
    }
    Ok(())
}

pub(crate) fn reject_duplicate_faces(
    side: InputSide,
    faces: &mut [Triangle],
) -> Result<(), RoundTripFailure> {
    faces.sort_unstable_by_key(|face| face.canonical_point_indices());
    if faces
        .windows(2)
        .any(|pair| pair[0].canonical_point_indices() == pair[1].canonical_point_indices())
    {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::TopologyDrift,
            format_args!("{side} contains duplicate faces"),
        ));
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
    let (a_easting_delta, a_easting_exponent) = exact_difference(a.easting, c.easting);
    let (a_northing_delta, a_northing_exponent) = exact_difference(a.northing, c.northing);
    let (b_easting_delta, b_easting_exponent) = exact_difference(b.easting, c.easting);
    let (b_northing_delta, b_northing_exponent) = exact_difference(b.northing, c.northing);
    let left = a_easting_delta * b_northing_delta;
    let right = a_northing_delta * b_easting_delta;
    exact_scaled_integers_equal(
        left,
        a_easting_exponent + b_northing_exponent,
        right,
        a_northing_exponent + b_easting_exponent,
    )
}

fn exact_difference(left: f64, right: f64) -> (BigInt, i32) {
    let (left_significand, left_exponent) = exact_dyadic(left);
    let (right_significand, right_exponent) = exact_dyadic(right);
    let exponent = left_exponent.min(right_exponent);
    let left_shift = nonnegative_shift(left_exponent - exponent);
    let right_shift = nonnegative_shift(right_exponent - exponent);
    (
        (left_significand << left_shift) - (right_significand << right_shift),
        exponent,
    )
}

fn exact_dyadic(value: f64) -> (BigInt, i32) {
    const FRACTION_BITS: u64 = (1_u64 << 52) - 1;
    const SIGN_BIT: u64 = 1_u64 << 63;
    let bits = value.to_bits();
    let Ok(encoded_exponent) = i32::try_from((bits >> 52) & 0x7ff) else {
        unreachable!("a binary64 encoded exponent fits i32");
    };
    let fraction = bits & FRACTION_BITS;
    let (significand, exponent) = if encoded_exponent == 0 {
        (fraction, -1_074)
    } else {
        ((1_u64 << 52) | fraction, encoded_exponent - 1_023 - 52)
    };
    let significand = BigInt::from(significand);
    if bits & SIGN_BIT == 0 {
        (significand, exponent)
    } else {
        (-significand, exponent)
    }
}

fn exact_scaled_integers_equal(
    left: BigInt,
    left_exponent: i32,
    right: BigInt,
    right_exponent: i32,
) -> bool {
    if left_exponent == right_exponent {
        return left == right;
    }
    if left_exponent < right_exponent {
        let shift = nonnegative_shift(right_exponent - left_exponent);
        left == right << shift
    } else {
        let shift = nonnegative_shift(left_exponent - right_exponent);
        left << shift == right
    }
}

fn nonnegative_shift(value: i32) -> usize {
    let Ok(value) = usize::try_from(value) else {
        unreachable!("an exponent difference is nonnegative");
    };
    value
}

fn check_item_limit(
    side: InputSide,
    item: &str,
    actual: usize,
    allowed: u64,
) -> Result<(), RoundTripFailure> {
    if actual as u64 > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} {item} required {actual}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn simple_text<'a>(
    side: InputSide,
    node: Node<'a, '_>,
    element: &str,
) -> Result<&'a str, RoundTripFailure> {
    let mut content = None;
    for child in node.children() {
        if child.is_text() && child.text().is_some_and(|text| !text.trim().is_empty()) {
            if content.is_some() {
                return Err(schema_error(side, "simple XML content must be contiguous"));
            }
            content = child.text().map(str::trim);
        } else if child.is_element() || child.is_pi() {
            return Err(schema_error(
                side,
                "simple XML content cannot contain child markup",
            ));
        }
    }
    content.ok_or_else(|| {
        RoundTripFailure::invalid(format_args!("{side} {element} requires text content"))
    })
}

fn validate_allowed_children(
    side: InputSide,
    parent: Node<'_, '_>,
    allowed: &[&str],
) -> Result<(), RoundTripFailure> {
    for child in parent.children() {
        if child.is_text() && child.text().is_some_and(|text| !text.trim().is_empty()) {
            return Err(schema_error(side, "container has unexpected text content"));
        }
        if child.is_element()
            && (child.tag_name().namespace() != Some(LANDXML_NAMESPACE)
                || !allowed.contains(&child.tag_name().name()))
        {
            return Err(schema_error(
                side,
                "container has an unsupported child element",
            ));
        }
    }
    Ok(())
}

fn require_tag(side: InputSide, node: Node<'_, '_>, name: &str) -> Result<(), RoundTripFailure> {
    if !node.has_tag_name((LANDXML_NAMESPACE, name)) {
        return Err(schema_error(side, "root is not LandXML 1.2"));
    }
    Ok(())
}

fn unique_child<'a, 'input>(
    side: InputSide,
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Node<'a, 'input>, RoundTripFailure> {
    let mut matches =
        element_children(parent).filter(|node| node.has_tag_name((LANDXML_NAMESPACE, name)));
    let child = matches.next().ok_or_else(|| {
        RoundTripFailure::invalid(format_args!("{side} requires exactly one {name} element"))
    })?;
    if matches.next().is_some() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} requires exactly one {name} element"
        )));
    }
    Ok(child)
}

fn at_most_one_child(
    side: InputSide,
    parent: Node<'_, '_>,
    name: &str,
) -> Result<(), RoundTripFailure> {
    if element_children(parent)
        .filter(|node| node.has_tag_name((LANDXML_NAMESPACE, name)))
        .count()
        > 1
    {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} permits at most one {name} element"
        )));
    }
    Ok(())
}

fn element_children<'a, 'input>(node: Node<'a, 'input>) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children().filter(Node::is_element)
}

fn unqualified_attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == name)
        .map(|attribute| attribute.value())
}

fn schema_error(side: InputSide, message: &'static str) -> RoundTripFailure {
    RoundTripFailure::semantic(
        RoundTripReason::SubsetUnsupported,
        format_args!("{side} schema is unsupported: {message}"),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ComparisonFacts {
    comparison_count: u64,
    max_easting_drift_metres: f64,
    max_northing_drift_metres: f64,
    max_horizontal_drift_metres: f64,
    max_vertical_drift_metres: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TopologyDrift {
    added_count: u64,
    removed_count: u64,
    added_hash: [u8; 32],
    removed_hash: [u8; 32],
    added_sample: Box<[[usize; 3]]>,
    removed_sample: Box<[[usize; 3]]>,
}

impl TopologyDrift {
    pub(crate) const fn added_count(&self) -> u64 {
        self.added_count
    }

    pub(crate) const fn removed_count(&self) -> u64 {
        self.removed_count
    }

    pub(crate) const fn added_hash(&self) -> [u8; 32] {
        self.added_hash
    }

    pub(crate) const fn removed_hash(&self) -> [u8; 32] {
        self.removed_hash
    }

    pub(crate) fn added_sample(&self) -> &[[usize; 3]] {
        &self.added_sample
    }

    pub(crate) fn removed_sample(&self) -> &[[usize; 3]] {
        &self.removed_sample
    }
}

fn compare_surfaces(
    reference: &ParsedSurface,
    returned: &ParsedSurface,
    tolerances: RoundTripTolerances,
    max_comparisons: u64,
    control: Option<&OperationControl>,
) -> Result<ComparisonFacts, RoundTripFailure> {
    check_round_trip_cancelled(control)?;
    if reference.points.len() != returned.points.len() {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::PointCountDrift,
            format_args!(
                "vertex counts differ: REFERENCE has {}, RETURNED has {}",
                reference.points.len(),
                returned.points.len()
            ),
        ));
    }
    let (returned_to_reference, facts) = match_points(
        &reference.points,
        &returned.points,
        tolerances,
        max_comparisons,
        control,
    )?;
    if let Err(mut error) = compare_topology(reference, returned, &returned_to_reference, control) {
        error.comparison = Some(facts);
        return Err(error);
    }
    Ok(facts)
}

fn match_points(
    reference: &[Position],
    returned: &[Position],
    tolerances: RoundTripTolerances,
    max_comparisons: u64,
    control: Option<&OperationControl>,
) -> Result<(Vec<usize>, ComparisonFacts), RoundTripFailure> {
    if tolerances.horizontal_metres() == 0.0 && tolerances.vertical_metres() == 0.0 {
        return match_exact_points(reference, returned, max_comparisons, control);
    }
    let mut returned_by_easting = (0..returned.len()).collect::<Vec<_>>();
    returned_by_easting.sort_unstable_by(|left, right| {
        returned[*left].easting.total_cmp(&returned[*right].easting)
    });
    let mut returned_to_reference = vec![usize::MAX; returned.len()];
    let mut facts = ComparisonFacts::default();
    for (reference_index, reference_point) in reference.iter().enumerate() {
        check_round_trip_cancelled(control)?;
        let (returned_index, drift) = unique_point_match(
            *reference_point,
            returned,
            &returned_by_easting,
            tolerances,
            max_comparisons,
            &mut facts,
            control,
        )?;
        if returned_to_reference[returned_index] != usize::MAX {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::VertexAmbiguous,
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
    control: Option<&OperationControl>,
) -> Result<(Vec<usize>, ComparisonFacts), RoundTripFailure> {
    let comparison_count = u64::try_from(reference.len()).unwrap_or(u64::MAX);
    if comparison_count > max_comparisons {
        return Err(RoundTripFailure::resource(format_args!(
            "vertex comparisons require {comparison_count}; limit is {max_comparisons}"
        )));
    }
    let mut returned_positions = BTreeMap::new();
    for (index, point) in returned.iter().enumerate() {
        if index.is_multiple_of(4096) {
            check_round_trip_cancelled(control)?;
        }
        if returned_positions.insert(point.key(), index).is_some() {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::VertexAmbiguous,
                "RETURNED contains duplicate coordinates, so vertex matching is ambiguous",
            ));
        }
    }
    let mut returned_to_reference = vec![usize::MAX; returned.len()];
    for (reference_index, point) in reference.iter().enumerate() {
        if reference_index.is_multiple_of(4096) {
            check_round_trip_cancelled(control)?;
        }
        let returned_index = returned_positions
            .get(&point.key())
            .copied()
            .ok_or_else(|| {
                RoundTripFailure::semantic(
                    RoundTripReason::ToleranceDrift,
                    "a REFERENCE vertex has no exact RETURNED coordinate match",
                )
            })?;
        if returned_to_reference[returned_index] != usize::MAX {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::VertexAmbiguous,
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

fn unique_point_match(
    reference: Position,
    returned: &[Position],
    returned_by_easting: &[usize],
    tolerances: RoundTripTolerances,
    max_comparisons: u64,
    facts: &mut ComparisonFacts,
    control: Option<&OperationControl>,
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
        if facts.comparison_count.is_multiple_of(4096) {
            check_round_trip_cancelled(control)?;
        }
        facts.comparison_count = facts.comparison_count.saturating_add(1);
        if facts.comparison_count > max_comparisons {
            return Err(RoundTripFailure::resource(format_args!(
                "vertex comparisons exceed the {max_comparisons} comparison limit"
            )));
        }
        let drift = CoordinateDrift::between(reference, returned[*returned_index]);
        if drift.is_within(tolerances) && matched.replace((*returned_index, drift)).is_some() {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::VertexAmbiguous,
                "vertex matching is ambiguous under the declared tolerances",
            ));
        }
    }
    matched.ok_or_else(|| {
        RoundTripFailure::semantic(
            RoundTripReason::VertexUnmatched,
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
    returned_to_reference: &[usize],
    control: Option<&OperationControl>,
) -> Result<(), RoundTripFailure> {
    check_round_trip_cancelled(control)?;
    let mut reference_faces = reference
        .faces
        .iter()
        .copied()
        .map(Triangle::canonical_point_indices)
        .collect::<Vec<_>>();
    let mut returned_faces = returned
        .faces
        .iter()
        .copied()
        .map(|face| face.remap(returned_to_reference).canonical_point_indices())
        .collect::<Vec<_>>();
    reference_faces.sort_unstable();
    returned_faces.sort_unstable();
    check_round_trip_cancelled(control)?;
    if reference_faces != returned_faces {
        return Err(RoundTripFailure::topology(
            topology_drift(&reference_faces, &returned_faces, control)?,
            "TIN topology differs after point-ID, face-order, and winding normalization",
        ));
    }
    Ok(())
}

fn check_round_trip_cancelled(control: Option<&OperationControl>) -> Result<(), RoundTripFailure> {
    if control.is_some_and(|control| control.check_cancelled().is_err()) {
        Err(RoundTripFailure::cancelled())
    } else {
        Ok(())
    }
}

fn topology_drift(
    reference: &[[usize; 3]],
    returned: &[[usize; 3]],
    control: Option<&OperationControl>,
) -> Result<TopologyDrift, RoundTripFailure> {
    let mut added = FaceDifference::new(b"punctra-round-trip-added-faces-v1");
    let mut removed = FaceDifference::new(b"punctra-round-trip-removed-faces-v1");
    let (mut reference_index, mut returned_index) = (0, 0);
    while reference_index < reference.len() && returned_index < returned.len() {
        if (reference_index + returned_index).is_multiple_of(4096) {
            check_round_trip_cancelled(control)?;
        }
        match reference[reference_index].cmp(&returned[returned_index]) {
            std::cmp::Ordering::Less => {
                removed.push(reference[reference_index]);
                reference_index += 1;
            }
            std::cmp::Ordering::Greater => {
                added.push(returned[returned_index]);
                returned_index += 1;
            }
            std::cmp::Ordering::Equal => {
                reference_index += 1;
                returned_index += 1;
            }
        }
    }
    for (index, face) in reference[reference_index..].iter().copied().enumerate() {
        if index.is_multiple_of(4096) {
            check_round_trip_cancelled(control)?;
        }
        removed.push(face);
    }
    for (index, face) in returned[returned_index..].iter().copied().enumerate() {
        if index.is_multiple_of(4096) {
            check_round_trip_cancelled(control)?;
        }
        added.push(face);
    }
    let (added_count, added_hash, added_sample) = added.finish();
    let (removed_count, removed_hash, removed_sample) = removed.finish();
    Ok(TopologyDrift {
        added_count,
        removed_count,
        added_hash,
        removed_hash,
        added_sample,
        removed_sample,
    })
}

struct FaceDifference {
    count: u64,
    hasher: blake3::Hasher,
    sample: Vec<[usize; 3]>,
}

impl FaceDifference {
    const SAMPLE_LIMIT: usize = 16;

    fn new(domain: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain);
        Self {
            count: 0,
            hasher,
            sample: Vec::with_capacity(Self::SAMPLE_LIMIT),
        }
    }

    fn push(&mut self, face: [usize; 3]) {
        self.count = self.count.saturating_add(1);
        for vertex in face {
            self.hasher.update(&(vertex as u64).to_le_bytes());
        }
        if self.sample.len() < Self::SAMPLE_LIMIT {
            self.sample.push(face);
        }
    }

    fn finish(mut self) -> (u64, [u8; 32], Box<[[usize; 3]]>) {
        self.hasher.update(&self.count.to_le_bytes());
        (
            self.count,
            *self.hasher.finalize().as_bytes(),
            self.sample.into_boxed_slice(),
        )
    }
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        RoundTripDeclaration, RoundTripFailureKind, RoundTripLimits, RoundTripReason,
        RoundTripTolerances, verify_landxml_round_trip,
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
    fn comments_containing_declaration_tokens_are_ignored() {
        let fixture = Fixture::new("declaration-token-comments");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let returned_xml = reference_xml.replacen(
            "<Units>",
            "<!-- generated without <!DOCTYPE or <!ENTITY declarations -->\n<Units>",
            1,
        );
        let (reference, returned) = fixture.write_pair(&reference_xml, &returned_xml);

        let report = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("declaration tokens in comment text are non-semantic");

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
    fn identical_duplicate_faces_fail_qualification() {
        let fixture = Fixture::new("identical-duplicate-faces");
        let duplicate_xml = landxml(REFERENCE_POINTS, &["1 2 3", "3 2 1"], false);
        let (reference, returned) = fixture.write_pair(&duplicate_xml, &duplicate_xml);

        let error = verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect_err("duplicate faces in both inputs must not compare equal");
        assert_eq!(error.kind(), RoundTripFailureKind::SemanticMismatch);
        assert_eq!(error.reason(), Some(RoundTripReason::TopologyDrift));
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
            &reference_xml.replace("UTF-8", "UTF-16"),
            &reference_xml.replacen("<LandXML", "<!DOCTYPE LandXML []>\n<LandXML", 1),
            &reference_xml.replacen(
                "<LandXML",
                "<!DOCTYPE LandXML [<!ENTITY generated \"value\">]>\n<LandXML",
                1,
            ),
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
                RoundTripFailureKind::SemanticMismatch,
            );
        }
    }

    #[test]
    fn foreign_namespaced_attributes_cannot_supply_landxml_semantics() {
        let fixture = Fixture::new("foreign-attribute-lookalikes");
        let reference_xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);
        let variants = [
            (
                reference_xml.replacen(
                    "version=\"1.2\"",
                    "xmlns:meta=\"urn:generated:metadata\" meta:version=\"1.2\"",
                    1,
                ),
                RoundTripFailureKind::SemanticMismatch,
            ),
            (
                reference_xml.replacen(
                    "linearUnit=\"meter\"",
                    "xmlns:meta=\"urn:generated:metadata\" meta:linearUnit=\"meter\"",
                    1,
                ),
                RoundTripFailureKind::SemanticMismatch,
            ),
            (
                reference_xml.replacen(
                    "name=\"Ground\"",
                    "xmlns:meta=\"urn:generated:metadata\" meta:name=\"Ground\"",
                    1,
                ),
                RoundTripFailureKind::SemanticMismatch,
            ),
            (
                reference_xml.replacen(
                    "surfType=\"TIN\"",
                    "xmlns:meta=\"urn:generated:metadata\" meta:surfType=\"TIN\"",
                    1,
                ),
                RoundTripFailureKind::SemanticMismatch,
            ),
            (
                reference_xml.replacen(
                    "<P id=\"1\">",
                    "<P xmlns:meta=\"urn:generated:metadata\" meta:id=\"1\">",
                    1,
                ),
                RoundTripFailureKind::SemanticMismatch,
            ),
        ];
        for (index, (returned_xml, expected)) in variants.iter().enumerate() {
            let reference = fixture.write(&format!("reference-{index}.xml"), &reference_xml);
            let returned = fixture.write(&format!("returned-{index}.xml"), returned_xml);
            assert_kind(
                verify(
                    &reference,
                    &returned,
                    tolerances(0.0, 0.0),
                    default_limits(),
                ),
                *expected,
            );
        }

        let returned_xml = reference_xml
            .replacen(
                "<LandXML ",
                "<LandXML xmlns:meta=\"urn:generated:metadata\" meta:version=\"9.9\" ",
                1,
            )
            .replacen("<Metric ", "<Metric meta:linearUnit=\"foot\" ", 1)
            .replacen("<Surface ", "<Surface meta:name=\"Ignored\" ", 1)
            .replacen("<Definition ", "<Definition meta:surfType=\"GRID\" ", 1)
            .replacen("<P id=\"1\">", "<P meta:id=\"99\" id=\"1\">", 1);
        let reference = fixture.write("reference-with-metadata.xml", &reference_xml);
        let returned = fixture.write("returned-with-metadata.xml", &returned_xml);
        verify(
            &reference,
            &returned,
            tolerances(0.0, 0.0),
            default_limits(),
        )
        .expect("foreign namespaced attributes remain ignored metadata");
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

    #[cfg(unix)]
    #[test]
    fn raced_symlink_and_fifo_replacements_fail_without_blocking() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("raced-non-regular-inputs");
        let xml = landxml(REFERENCE_POINTS, REFERENCE_FACES, false);

        let fifo = fixture.write("raced-fifo.xml", &xml);
        let fifo_metadata = fs::symlink_metadata(&fifo).expect("inspect initial FIFO path");
        fs::remove_file(&fifo).expect("remove initial FIFO-path file");
        create_fifo(&fifo);
        assert_capture_rejects_promptly(fifo, fifo_metadata);

        let link = fixture.write("raced-link.xml", &xml);
        let link_metadata = fs::symlink_metadata(&link).expect("inspect initial link path");
        fs::remove_file(&link).expect("remove initial link-path file");
        let link_target = fixture.path("raced-link-target.fifo");
        create_fifo(&link_target);
        symlink(link_target, &link).expect("replace regular file with FIFO link");
        assert_capture_rejects_promptly(link, link_metadata);
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

    #[cfg(unix)]
    fn create_fifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("invoke POSIX mkfifo");
        assert!(
            status.success(),
            "create FIFO fixture at {}",
            path.display()
        );
    }

    #[cfg(unix)]
    fn assert_capture_rejects_promptly(path: PathBuf, metadata: fs::Metadata) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let kind = super::capture_inspected_regular_file(
                super::InputSide::Returned,
                &path,
                &metadata,
                default_limits().file_bytes,
            )
            .err()
            .map(|error| error.kind());
            let _ = sender.send(kind);
        });
        let kind = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("input capture must not block on a raced non-regular path");
        assert_eq!(kind, Some(RoundTripFailureKind::InvalidInput));
        worker.join().expect("input-capture worker must finish");
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
