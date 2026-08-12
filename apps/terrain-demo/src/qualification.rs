//! Private Complete-Run qualification and canonical Round-Trip Evidence.

#![allow(clippy::too_many_lines)]

use std::{
    fs::{self, File},
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    mem::size_of,
    path::{Path, PathBuf},
};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use quick_xml::{
    events::{BytesStart, Event},
    name::{Namespace, ResolveResult},
    reader::NsReader,
};
use serde_json::Value;

use crate::{
    journal::{CompleteRunSnapshot, JournalLimits, bind_path, read_complete_run},
    publication::{DirectoryWitness, same_file_identity},
    report::{ReportDisposition, ReportError, ReportLimits, ensure_evidence},
    roundtrip::{
        ComparisonFacts, Point, PointMatchError, RoundTripDeclaration, RoundTripLimits,
        RoundTripTolerances, face_is_degenerate, match_points_evidence,
    },
};

const LANDXML_NAMESPACE: &[u8] = b"http://www.landxml.org/schema/LandXML-1.2";
const XINCLUDE_NAMESPACE: &[u8] = b"http://www.w3.org/2001/XInclude";
const REPORT_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-report-bytes-v1";
const EVIDENCE_SCHEMA: &str = "punctra.terrain-demo.landxml-round-trip-evidence.v1";
const MATCHER_VERSION: u64 = 1;
const MAX_EVENT_BYTES: u64 = 64 * 1024;
const MAX_DEPTH: usize = 16;
const EVIDENCE_BYTES: u64 = 4 * 1024 * 1024;
const PATH_BINDING_BYTES: u64 = 4 * 1024;

pub(crate) struct QualificationRequest {
    pub(crate) run_root: PathBuf,
    pub(crate) returned_landxml: PathBuf,
    pub(crate) evidence_target: PathBuf,
    pub(crate) declaration: RoundTripDeclaration,
    /// Canonically key-sorted, caller-declared settings. The legacy
    /// `RoundTripDeclaration` profile is deliberately not used to encode them.
    pub(crate) downstream_settings: Vec<(String, String)>,
    pub(crate) tolerances: RoundTripTolerances,
}

pub(crate) struct QualificationReceipt {
    pub(crate) passed: bool,
    pub(crate) disposition: ReportDisposition,
    pub(crate) content_hash: [u8; 32],
    pub(crate) byte_length: u64,
}

pub(crate) fn verify_round_trip(
    request: &QualificationRequest,
) -> Result<QualificationReceipt, QualificationError> {
    let control = OperationControl::new();
    verify_round_trip_with_limits(request, RoundTripLimits::qualification(), &control)
}

fn verify_round_trip_with_limits(
    request: &QualificationRequest,
    limits: RoundTripLimits,
    control: &OperationControl,
) -> Result<QualificationReceipt, QualificationError> {
    validate_paths(request)?;
    let evidence_parent_path = request
        .evidence_target
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let evidence_parent = DirectoryWitness::capture(evidence_parent_path).map_err(|source| {
        QualificationError::io("witness evidence parent", evidence_parent_path, source)
    })?;
    let run_root = DirectoryWitness::capture(&request.run_root)
        .map_err(|source| QualificationError::io("witness Run root", &request.run_root, source))?;
    let journal = read_complete_run(&request.run_root.join("run.pwf"), JournalLimits::default())
        .map_err(QualificationError::Journal)?;
    let original_path = request.run_root.join("terrain.xml");
    let report_path = request.run_root.join("audit.json");
    // Retain every leaf descriptor before parsing any potentially large XML so
    // the invocation has one simultaneous, immutable input snapshot.
    let original_open = open_regular(
        &original_path,
        StreamSide::Original.label(),
        limits.file_bytes,
    )?;
    let returned_open = open_regular(
        &request.returned_landxml,
        StreamSide::Returned.label(),
        limits.file_bytes,
    )?;
    if same_file_identity(&original_open.1, &returned_open.1) {
        return Err(QualificationError::Invalid(
            "returned LandXML must be a distinct file from terrain.xml",
        ));
    }
    let report_open = open_regular(&report_path, "audit.json", EVIDENCE_BYTES)?;
    let original = read_open_landxml(
        &original_path,
        StreamSide::Original,
        limits,
        control,
        original_open,
    )?;
    let returned = read_open_landxml(
        &request.returned_landxml,
        StreamSide::Returned,
        limits,
        control,
        returned_open,
    )?;
    let report = read_open_bound_report(&report_path, EVIDENCE_BYTES, control, report_open)?;
    bind_complete_run(&journal, &original, &report, &request.run_root)?;
    run_root.verify().map_err(|source| {
        QualificationError::io("revalidate Run root", &request.run_root, source)
    })?;
    evidence_parent.verify().map_err(|source| {
        QualificationError::io("revalidate evidence parent", evidence_parent_path, source)
    })?;

    let outcome = evaluate_semantics(
        &original.surface,
        &returned.surface,
        returned.metric_metres,
        request.tolerances,
        limits,
    )?;
    let evidence = Evidence {
        journal: &journal,
        original: &original,
        report: &report,
        returned: &returned,
        declaration: &request.declaration,
        downstream_settings: &request.downstream_settings,
        tolerances: request.tolerances,
        limits,
        outcome,
    };
    let receipt = ensure_evidence(
        &request.evidence_target,
        ReportLimits {
            max_output_bytes: EVIDENCE_BYTES,
            max_staging_bytes: EVIDENCE_BYTES,
            max_write_buffer_bytes: 8 * 1024,
            max_working_bytes: 64 * 1024,
        },
        control,
        |writer| write_evidence(writer, &evidence),
        || {
            verify_input_witnesses(
                &journal,
                &original,
                &returned,
                &report,
                &run_root,
                &evidence_parent,
            )
        },
    )
    .map_err(QualificationError::Publication)?;
    Ok(QualificationReceipt {
        passed: evidence.outcome.passed,
        disposition: receipt.disposition,
        content_hash: receipt.content_hash,
        byte_length: receipt.byte_length,
    })
}

fn verify_input_witnesses(
    journal: &CompleteRunSnapshot,
    original: &CapturedLandXml,
    returned: &CapturedLandXml,
    report: &BoundReport,
    run_root: &DirectoryWitness,
    evidence_parent: &DirectoryWitness,
) -> io::Result<()> {
    journal.verify_unchanged().map_err(io::Error::other)?;
    original
        .witness
        .verify()
        .map_err(|error| io::Error::other(error.to_string()))?;
    returned
        .witness
        .verify()
        .map_err(|error| io::Error::other(error.to_string()))?;
    report
        .witness
        .verify()
        .map_err(|error| io::Error::other(error.to_string()))?;
    run_root.verify()?;
    evidence_parent.verify()
}

#[derive(Clone, Copy)]
enum StreamSide {
    Original,
    Returned,
}

impl StreamSide {
    const fn label(self) -> &'static str {
        match self {
            Self::Original => "original terrain.xml",
            Self::Returned => "returned LandXML",
        }
    }
}

struct CapturedLandXml {
    hash: [u8; 32],
    bytes: u64,
    surface_name: Box<str>,
    metric_metres: bool,
    witness: InputWitness,
    ignored_top_level_sections: Box<[Box<str>]>,
    surface: StreamSurface,
}

#[derive(Clone)]
struct StreamSurface {
    points: Vec<Point>,
    faces: Vec<[usize; 3]>,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "the flags are independent structural facts accumulated by the streaming parser"
)]
struct StreamState {
    stack: Vec<Box<str>>,
    point_ids: Vec<(u64, usize)>,
    points: Vec<Point>,
    pending_faces: Vec<[u64; 3]>,
    surface_name: Option<Box<str>>,
    unit_declarations: u64,
    metric_count: u64,
    imperial_count: u64,
    metric_metres: bool,
    units_count: u64,
    surfaces_count: u64,
    project_count: u64,
    application_count: u64,
    landxml_version: bool,
    tin_definition: bool,
    surface_count: u64,
    definition_count: u64,
    pnts_count: u64,
    faces_count: u64,
    nodes: u64,
    text_bytes: u64,
    root_seen: bool,
    ignored_metadata_depth: usize,
}

#[cfg(test)]
fn read_landxml(
    path: &Path,
    side: StreamSide,
    limits: RoundTripLimits,
    control: &OperationControl,
) -> Result<CapturedLandXml, QualificationError> {
    let opened = open_regular(path, side.label(), limits.file_bytes)?;
    read_open_landxml(path, side, limits, control, opened)
}

fn read_open_landxml(
    path: &Path,
    side: StreamSide,
    limits: RoundTripLimits,
    control: &OperationControl,
    (file, identity): (File, fs::Metadata),
) -> Result<CapturedLandXml, QualificationError> {
    let bytes = identity.len();
    let mut hasher = Hasher::new();
    let hashing_file = file
        .try_clone()
        .map_err(|source| QualificationError::io("retain open LandXML input", path, source))?;
    let hashing = HashingRead::new(hashing_file, &mut hasher, bytes, control, MAX_EVENT_BYTES);
    let mut reader = NsReader::from_reader(BufReader::with_capacity(64 * 1024, hashing));
    reader.config_mut().check_comments = true;
    let mut state = StreamState::new(limits)?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(usize::try_from(MAX_EVENT_BYTES).expect("event limit fits usize"))
        .map_err(|_| {
            QualificationError::resource(
                "LandXML event buffer bytes",
                MAX_EVENT_BYTES,
                MAX_EVENT_BYTES,
            )
        })?;
    if buffer.capacity() as u64 > MAX_EVENT_BYTES {
        return Err(QualificationError::resource(
            "LandXML event buffer bytes",
            buffer.capacity() as u64,
            MAX_EVENT_BYTES,
        ));
    }
    loop {
        control
            .check_cancelled()
            .map_err(|_| QualificationError::Cancelled)?;
        buffer.clear();
        let (namespace, event) =
            reader
                .read_resolved_event_into(&mut buffer)
                .map_err(|source| match &source {
                    quick_xml::Error::Io(error) if error.kind() == io::ErrorKind::FileTooLarge => {
                        QualificationError::resource(
                            "LandXML XML token bytes",
                            MAX_EVENT_BYTES.saturating_add(1),
                            MAX_EVENT_BYTES,
                        )
                    }
                    _ => QualificationError::Xml(format!(
                        "{} is malformed XML: {source}",
                        side.label()
                    )),
                })?;
        match event {
            Event::Start(tag) => state.start(side, &namespace, &tag, limits)?,
            Event::Empty(tag) => state.empty(side, &namespace, &tag, limits)?,
            Event::End(_) => state.end(side)?,
            Event::Text(text) => state.text(
                side,
                text.decode()
                    .map_err(|source| {
                        QualificationError::Xml(format!(
                            "{} is not UTF-8 XML: {source}",
                            side.label()
                        ))
                    })?
                    .as_ref(),
                limits,
            )?,
            Event::GeneralRef(reference) => {
                state.general_reference(side, reference.as_ref(), limits)?;
            }
            Event::CData(text) => state.text(
                side,
                text.decode()
                    .map_err(|source| {
                        QualificationError::Xml(format!(
                            "{} is not UTF-8 XML: {source}",
                            side.label()
                        ))
                    })?
                    .as_ref(),
                limits,
            )?,
            Event::DocType(_) => {
                return Err(QualificationError::Xml(format!(
                    "{} contains an unsupported declaration or entity",
                    side.label()
                )));
            }
            Event::Decl(declaration) => {
                if declaration
                    .version()
                    .map_err(|source| {
                        QualificationError::Xml(format!(
                            "{} has an invalid XML declaration: {source}",
                            side.label()
                        ))
                    })?
                    .as_ref()
                    != b"1.0"
                {
                    return Err(QualificationError::Xml(format!(
                        "{} must use XML version 1.0",
                        side.label()
                    )));
                }
                if declaration
                    .encoding()
                    .transpose()
                    .map_err(|source| {
                        QualificationError::Xml(format!(
                            "{} has an invalid XML declaration: {source}",
                            side.label()
                        ))
                    })?
                    .is_some_and(|encoding| !encoding.as_ref().eq_ignore_ascii_case(b"utf-8"))
                {
                    return Err(QualificationError::Xml(format!(
                        "{} must declare UTF-8 when it declares an encoding",
                        side.label()
                    )));
                }
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    drop(reader);
    let final_metadata = fs::symlink_metadata(path)
        .map_err(|source| QualificationError::io("reinspect LandXML input", path, source))?;
    if !same_file_identity(&identity, &final_metadata)
        || !same_file_state(&identity, &final_metadata)
    {
        return Err(QualificationError::InputChanged(path.to_path_buf()));
    }
    let surface_name = state
        .surface_name
        .clone()
        .ok_or(QualificationError::Subset("Surface name is absent"))?;
    let metric_metres = state.units_count == 1
        && state.metric_count == 1
        && state.imperial_count == 0
        && state.unit_declarations == 1
        && state.metric_metres;
    let ignored_top_level_sections = state.ignored_top_level_sections();
    let surface = state.finish(side, limits)?;
    let hash = *hasher.finalize().as_bytes();
    let witness = InputWitness::new(path, &file, identity.clone(), hash, b"")?;
    Ok(CapturedLandXml {
        hash,
        bytes,
        surface_name,
        metric_metres,
        witness,
        ignored_top_level_sections,
        surface,
    })
}

impl StreamState {
    fn new(limits: RoundTripLimits) -> Result<Self, QualificationError> {
        Self::validate_retained_limit(limits)?;
        let point_capacity = usize::try_from(limits.points).map_err(|_| {
            QualificationError::resource(
                "addressable LandXML points",
                limits.points,
                limits.retained_model_bytes,
            )
        })?;
        let face_capacity = usize::try_from(limits.faces).map_err(|_| {
            QualificationError::resource(
                "addressable LandXML faces",
                limits.faces,
                limits.retained_model_bytes,
            )
        })?;
        let mut stack = Vec::new();
        let mut point_ids = Vec::new();
        let mut points = Vec::new();
        let mut pending_faces = Vec::new();
        reserve_items(&mut stack, MAX_DEPTH, "LandXML element stack", limits)?;
        reserve_items(
            &mut point_ids,
            point_capacity,
            "LandXML Point identifier index",
            limits,
        )?;
        reserve_items(&mut points, point_capacity, "LandXML Point storage", limits)?;
        reserve_items(
            &mut pending_faces,
            face_capacity,
            "LandXML unresolved face storage",
            limits,
        )?;
        Ok(Self {
            stack,
            point_ids,
            points,
            pending_faces,
            surface_name: None,
            unit_declarations: 0,
            metric_count: 0,
            imperial_count: 0,
            metric_metres: false,
            units_count: 0,
            surfaces_count: 0,
            project_count: 0,
            application_count: 0,
            landxml_version: false,
            tin_definition: false,
            surface_count: 0,
            definition_count: 0,
            pnts_count: 0,
            faces_count: 0,
            nodes: 0,
            text_bytes: 0,
            root_seen: false,
            ignored_metadata_depth: 0,
        })
    }

    fn ignored_top_level_sections(&self) -> Box<[Box<str>]> {
        let mut names = Vec::new();
        if self.project_count == 1 {
            names.push(Box::<str>::from("Project"));
        }
        if self.application_count == 1 {
            names.push(Box::<str>::from("Application"));
        }
        names.into_boxed_slice()
    }

    fn validate_retained_limit(limits: RoundTripLimits) -> Result<(), QualificationError> {
        // Peak point overlap: the two retained surfaces (2 Point), the exact
        // matcher's keyed returned index ([u64; 3], usize), and its mapping
        // (usize). Peak face overlap: both retained surfaces plus both sorted
        // topology projections (4 [usize; 3]). Parser-only Point IDs and
        // unresolved faces overlap fewer retained surfaces and fit below these
        // peaks. Stack slots and the fixed XML event buffer are included as a
        // fixed surcharge. Every Vec reservation is also checked below so an
        // allocator that returns excess capacity fails closed.
        let required = Self::required_retained_model_bytes(limits);
        if required > limits.retained_model_bytes {
            return Err(QualificationError::resource(
                "LandXML retained model bytes",
                required,
                limits.retained_model_bytes,
            ));
        }
        Ok(())
    }

    fn required_retained_model_bytes(limits: RoundTripLimits) -> u64 {
        let point_bytes = limits.points.saturating_mul(
            (2 * size_of::<Point>() + size_of::<([u64; 3], usize)>() + size_of::<usize>()) as u64,
        );
        let face_bytes = limits
            .faces
            .saturating_mul((4 * size_of::<[usize; 3]>()) as u64);
        let fixed_bytes = MAX_EVENT_BYTES.saturating_add(
            u64::try_from(MAX_DEPTH.saturating_mul(size_of::<Box<str>>())).unwrap_or(u64::MAX),
        );
        point_bytes
            .saturating_add(face_bytes)
            .saturating_add(fixed_bytes)
    }

    fn start(
        &mut self,
        side: StreamSide,
        namespace: &ResolveResult<'_>,
        tag: &BytesStart<'_>,
        limits: RoundTripLimits,
    ) -> Result<(), QualificationError> {
        self.count_node(limits)?;
        if self.stack.len() == MAX_DEPTH {
            return Err(QualificationError::Subset(
                "LandXML nesting exceeds the supported depth",
            ));
        }
        let name = str::from_utf8(tag.local_name().as_ref())
            .map_err(|_| QualificationError::Xml(format!("{} has a non-UTF-8 tag", side.label())))?
            .to_owned();
        if self.ignored_metadata_depth != 0 {
            reject_xinclude(side, namespace)?;
            self.read_attributes(side, &name, tag, limits)?;
            self.stack.push(name.into_boxed_str());
            self.ignored_metadata_depth += 1;
            return Ok(());
        }
        require_namespace(side, namespace)?;
        self.validate_child(side, &name)?;
        self.read_attributes(side, &name, tag, limits)?;
        let begins_ignored_metadata = matches!(name.as_str(), "Project" | "Application");
        self.stack.push(name.into_boxed_str());
        if begins_ignored_metadata {
            self.ignored_metadata_depth = 1;
        }
        Ok(())
    }

    fn empty(
        &mut self,
        side: StreamSide,
        namespace: &ResolveResult<'_>,
        tag: &BytesStart<'_>,
        limits: RoundTripLimits,
    ) -> Result<(), QualificationError> {
        self.start(side, namespace, tag, limits)?;
        self.end(side)
    }

    fn end(&mut self, side: StreamSide) -> Result<(), QualificationError> {
        if self.ignored_metadata_depth != 0 {
            self.ignored_metadata_depth -= 1;
        }
        self.stack.pop().ok_or_else(|| {
            QualificationError::Xml(format!("{} has an unmatched closing tag", side.label()))
        })?;
        Ok(())
    }

    fn text(
        &mut self,
        side: StreamSide,
        text: &str,
        limits: RoundTripLimits,
    ) -> Result<(), QualificationError> {
        self.text_bytes = self.text_bytes.saturating_add(text.len() as u64);
        if self.text_bytes > limits.xml_text_bytes {
            return Err(QualificationError::resource(
                "LandXML XML text and attribute bytes",
                self.text_bytes,
                limits.xml_text_bytes,
            ));
        }
        if self.ignored_metadata_depth != 0 {
            return Ok(());
        }
        match self.stack.last().map(std::convert::AsRef::as_ref) {
            Some("P") => self.add_point(side, text, limits.points),
            Some("F") => self.add_face(side, text, limits.faces),
            _ if text.trim().is_empty() => Ok(()),
            _ => Err(QualificationError::Subset(
                "only P and F elements may contain semantic text",
            )),
        }
    }

    fn validate_child(&mut self, side: StreamSide, child: &str) -> Result<(), QualificationError> {
        let parent = self.stack.last().map(std::convert::AsRef::as_ref);
        let allowed = match parent {
            None => child == "LandXML" && !self.root_seen,
            Some("LandXML") => matches!(child, "Units" | "Project" | "Application" | "Surfaces"),
            Some("Units") => matches!(child, "Metric" | "Imperial"),
            Some("Surfaces") => child == "Surface",
            Some("Surface") => child == "Definition",
            Some("Definition") => matches!(child, "Pnts" | "Faces"),
            Some("Pnts") => child == "P",
            Some("Faces") => child == "F",
            Some(_) => false,
        };
        if !allowed {
            return Err(QualificationError::Subset(
                "LandXML contains a child outside the accepted single-TIN subset",
            ));
        }
        if parent.is_none() {
            self.root_seen = true;
        }
        if child == "Surface" {
            self.surface_count += 1;
        } else if child == "Definition" {
            self.definition_count += 1;
        } else if child == "Pnts" {
            self.pnts_count += 1;
        } else if child == "Faces" {
            self.faces_count += 1;
        } else if child == "Units" {
            self.units_count += 1;
        } else if child == "Metric" {
            self.metric_count += 1;
        } else if child == "Imperial" {
            self.imperial_count += 1;
        } else if child == "Surfaces" {
            self.surfaces_count += 1;
        } else if child == "Project" {
            self.project_count += 1;
        } else if child == "Application" {
            self.application_count += 1;
        }
        if self.surfaces_count > 1 || self.project_count > 1 || self.application_count > 1 {
            return Err(QualificationError::Subset(
                "LandXML contains a duplicated effective top-level container",
            ));
        }
        let _ = side;
        Ok(())
    }

    fn read_attributes(
        &mut self,
        side: StreamSide,
        name: &str,
        tag: &BytesStart<'_>,
        limits: RoundTripLimits,
    ) -> Result<(), QualificationError> {
        // BytesStart retains the exact source between `<` and `>` (or `/>`).
        // Counting everything after the qualified element name includes
        // whitespace, qualified attribute names, `=`, quotes, and values. It
        // therefore bounds the complete raw attribute region, not merely the
        // decoded values that quick-xml exposes below.
        let attribute_bytes = tag.as_ref().len().saturating_sub(tag.name().as_ref().len());
        self.text_bytes = self.text_bytes.saturating_add(attribute_bytes as u64);
        if self.text_bytes > limits.xml_text_bytes {
            return Err(QualificationError::resource(
                "LandXML XML text and attribute bytes",
                self.text_bytes,
                limits.xml_text_bytes,
            ));
        }
        let mut id = None;
        for attribute in tag.attributes() {
            let attribute = attribute.map_err(|source| {
                QualificationError::Xml(format!(
                    "{} has invalid attributes: {source}",
                    side.label()
                ))
            })?;
            let local_name = attribute.key.local_name();
            let key = str::from_utf8(local_name.as_ref()).map_err(|_| {
                QualificationError::Xml(format!("{} has a non-UTF-8 attribute", side.label()))
            })?;
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .map_err(|source| {
                    QualificationError::Xml(format!(
                        "{} has an invalid attribute: {source}",
                        side.label()
                    ))
                })?;
            if attribute.key.prefix().is_some() {
                continue;
            }
            match (name, key) {
                ("LandXML", "version") if value.as_ref() == "1.2" => {
                    self.landxml_version = true;
                }
                ("LandXML", "version") => {
                    return Err(QualificationError::Subset("LandXML version must be 1.2"));
                }
                ("Metric", "linearUnit") => {
                    self.unit_declarations += 1;
                    self.metric_metres = value.as_ref() == "meter";
                }
                ("Imperial", "linearUnit") => self.unit_declarations += 1,
                ("Surface", "name") => {
                    self.surface_name = Some(value.into_owned().into_boxed_str());
                }
                ("Definition", "surfType") if value.as_ref() == "TIN" => {
                    self.tin_definition = true;
                }
                ("Definition", "surfType") => {
                    return Err(QualificationError::Subset("Surface Definition must be TIN"));
                }
                ("P", "id") => id = Some(parse_positive_id(side, value.as_ref())?),
                _ => {}
            }
        }
        if name == "P" {
            let id = id.ok_or(QualificationError::Subset("every P requires an id"))?;
            self.point_ids.push((id, self.points.len()));
        }
        Ok(())
    }

    fn general_reference(
        &mut self,
        side: StreamSide,
        reference: &[u8],
        limits: RoundTripLimits,
    ) -> Result<(), QualificationError> {
        if self.ignored_metadata_depth == 0 || !valid_builtin_reference(reference) {
            return Err(QualificationError::Xml(format!(
                "{} contains an unsupported entity reference",
                side.label()
            )));
        }
        self.text_bytes = self
            .text_bytes
            .saturating_add(reference.len() as u64)
            .saturating_add(2);
        if self.text_bytes > limits.xml_text_bytes {
            return Err(QualificationError::resource(
                "LandXML XML text and attribute bytes",
                self.text_bytes,
                limits.xml_text_bytes,
            ));
        }
        Ok(())
    }

    fn add_point(
        &mut self,
        side: StreamSide,
        text: &str,
        limit: u64,
    ) -> Result<(), QualificationError> {
        if self.points.len() as u64 >= limit {
            return Err(QualificationError::resource(
                "LandXML points",
                self.points.len() as u64 + 1,
                limit,
            ));
        }
        let mut fields = text.split_whitespace();
        let northing = parse_coordinate(side, fields.next())?;
        let easting = parse_coordinate(side, fields.next())?;
        let elevation = parse_coordinate(side, fields.next())?;
        if fields.next().is_some() {
            return Err(QualificationError::Subset(
                "P must contain exactly three coordinates",
            ));
        }
        self.points.push(Point {
            position: [
                canonical_zero(easting),
                canonical_zero(northing),
                canonical_zero(elevation),
            ],
        });
        Ok(())
    }

    fn add_face(
        &mut self,
        side: StreamSide,
        text: &str,
        limit: u64,
    ) -> Result<(), QualificationError> {
        if self.pending_faces.len() as u64 >= limit {
            return Err(QualificationError::resource(
                "LandXML faces",
                self.pending_faces.len() as u64 + 1,
                limit,
            ));
        }
        let mut fields = text.split_whitespace();
        let mut ids = [
            parse_positive_id(side, fields.next().unwrap_or(""))?,
            parse_positive_id(side, fields.next().unwrap_or(""))?,
            parse_positive_id(side, fields.next().unwrap_or(""))?,
        ];
        if fields.next().is_some() || ids[0] == ids[1] || ids[0] == ids[2] || ids[1] == ids[2] {
            return Err(QualificationError::Subset(
                "F must contain three distinct Point identifiers",
            ));
        }
        ids.sort_unstable();
        self.pending_faces.push(ids);
        Ok(())
    }

    fn count_node(&mut self, limits: RoundTripLimits) -> Result<(), QualificationError> {
        self.nodes += 1;
        if self.nodes > limits.xml_nodes {
            Err(QualificationError::resource(
                "LandXML XML nodes",
                self.nodes,
                limits.xml_nodes,
            ))
        } else {
            Ok(())
        }
    }

    fn finish(
        mut self,
        _side: StreamSide,
        limits: RoundTripLimits,
    ) -> Result<StreamSurface, QualificationError> {
        if !self.stack.is_empty() || !self.root_seen {
            return Err(QualificationError::Xml(
                "LandXML document is incomplete".to_owned(),
            ));
        }
        if !self.landxml_version
            || !self.tin_definition
            || self.surfaces_count != 1
            || self.surface_count != 1
            || self.definition_count != 1
            || self.pnts_count != 1
            || self.faces_count != 1
            || self.points.len() < 3
            || self.pending_faces.is_empty()
            || self.point_ids.len() != self.points.len()
        {
            return Err(QualificationError::Subset(
                "LandXML must contain one metric-metre single TIN with Points and faces",
            ));
        }
        if self.text_bytes > limits.xml_text_bytes {
            return Err(QualificationError::resource(
                "LandXML XML text and attribute bytes",
                self.text_bytes,
                limits.xml_text_bytes,
            ));
        }
        self.point_ids.sort_unstable_by_key(|entry| entry.0);
        if self.point_ids.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(QualificationError::Subset("duplicate Point identifier"));
        }
        self.pending_faces.sort_unstable();
        let mut faces = Vec::new();
        reserve_items(
            &mut faces,
            self.pending_faces.len(),
            "LandXML resolved face storage",
            limits,
        )?;
        for ids in self.pending_faces {
            let face = ids.map(|id| {
                self.point_ids
                    .binary_search_by_key(&id, |entry| entry.0)
                    .ok()
                    .map(|position| self.point_ids[position].1)
            });
            let [Some(a), Some(b), Some(c)] = face else {
                return Err(QualificationError::Subset(
                    "face references an unknown Point",
                ));
            };
            if face_is_degenerate([self.points[a], self.points[b], self.points[c]]) {
                return Err(QualificationError::Subset("degenerate face is unsupported"));
            }
            faces.push([a, b, c]);
        }
        Ok(StreamSurface {
            points: self.points,
            faces,
        })
    }
}

#[derive(Clone, Copy)]
struct SemanticOutcome {
    passed: bool,
    reason: &'static str,
    comparison: ComparisonFacts,
    mapped: u64,
    unmatched: u64,
    ambiguous: u64,
    added_faces: u64,
    removed_faces: u64,
    added_hash: [u8; 32],
    removed_hash: [u8; 32],
}

fn evaluate_semantics(
    original: &StreamSurface,
    returned: &StreamSurface,
    returned_metric_metres: bool,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
) -> Result<SemanticOutcome, QualificationError> {
    if !returned_metric_metres {
        return Ok(failed("PRT_UNIT_DRIFT", 0, 0));
    }
    if original.points.len() != returned.points.len() {
        return Ok(failed(
            "PRT_POINT_COUNT_DRIFT",
            original.points.len().abs_diff(returned.points.len()) as u64,
            0,
        ));
    }
    let (mapping, comparison) = match match_points_evidence(
        &original.points,
        &returned.points,
        tolerances,
        limits.comparisons,
    ) {
        Ok(value) => value,
        Err(PointMatchError::Resource { required, allowed }) => {
            return Err(QualificationError::resource(
                "LandXML candidate vertex comparisons",
                required,
                allowed,
            ));
        }
        Err(PointMatchError::Allocation {
            buffer,
            required_bytes,
        }) => {
            return Err(QualificationError::resource(
                buffer,
                required_bytes,
                limits.retained_model_bytes,
            ));
        }
        Err(PointMatchError::Unmatched) => {
            let reason =
                if tolerances.horizontal_metres() == 0.0 && tolerances.vertical_metres() == 0.0 {
                    "PRT_VERTEX_UNMATCHED"
                } else {
                    "PRT_TOLERANCE_DRIFT"
                };
            return Ok(failed(reason, 1, 0));
        }
        Err(PointMatchError::Ambiguous) => {
            return Ok(failed("PRT_VERTEX_AMBIGUOUS", 0, 1));
        }
    };
    let topology = compare_topology(original, returned, &mapping, limits)?;
    if topology.removed_faces != 0 || topology.added_faces != 0 {
        return Ok(SemanticOutcome {
            passed: false,
            reason: "PRT_TOPOLOGY_DRIFT",
            comparison,
            mapped: mapping.len() as u64,
            unmatched: 0,
            ambiguous: 0,
            added_faces: topology.added_faces,
            removed_faces: topology.removed_faces,
            added_hash: topology.added_hash,
            removed_hash: topology.removed_hash,
        });
    }
    Ok(SemanticOutcome {
        passed: true,
        reason: "none",
        comparison,
        mapped: mapping.len() as u64,
        unmatched: 0,
        ambiguous: 0,
        added_faces: 0,
        removed_faces: 0,
        added_hash: hash_faces(&[]),
        removed_hash: hash_faces(&[]),
    })
}

fn failed(reason: &'static str, unmatched: u64, ambiguous: u64) -> SemanticOutcome {
    SemanticOutcome {
        passed: false,
        reason,
        comparison: ComparisonFacts::default(),
        mapped: 0,
        unmatched,
        ambiguous,
        added_faces: 0,
        removed_faces: 0,
        added_hash: hash_faces(&[]),
        removed_hash: hash_faces(&[]),
    }
}

struct TopologyDiff {
    added_faces: u64,
    removed_faces: u64,
    added_hash: [u8; 32],
    removed_hash: [u8; 32],
}

fn compare_topology(
    original: &StreamSurface,
    returned: &StreamSurface,
    mapping: &[usize],
    limits: RoundTripLimits,
) -> Result<TopologyDiff, QualificationError> {
    let original_faces = projected_faces(original, None, limits)?;
    let returned_faces = projected_faces(returned, Some(mapping), limits)?;
    let mut added_hasher = face_diff_hasher();
    let mut removed_hasher = face_diff_hasher();
    let mut added_faces = 0_u64;
    let mut removed_faces = 0_u64;
    let (mut original_index, mut returned_index) = (0, 0);
    while original_index < original_faces.len() || returned_index < returned_faces.len() {
        match (
            original_faces.get(original_index),
            returned_faces.get(returned_index),
        ) {
            (Some(original_face), Some(returned_face)) if original_face == returned_face => {
                original_index += 1;
                returned_index += 1;
            }
            (Some(original_face), Some(returned_face)) if original_face < returned_face => {
                removed_faces += 1;
                update_face_hash(&mut removed_hasher, original_face);
                original_index += 1;
            }
            (Some(_) | None, Some(returned_face)) => {
                added_faces += 1;
                update_face_hash(&mut added_hasher, returned_face);
                returned_index += 1;
            }
            (Some(original_face), None) => {
                removed_faces += 1;
                update_face_hash(&mut removed_hasher, original_face);
                original_index += 1;
            }
            (None, None) => break,
        }
    }
    Ok(TopologyDiff {
        added_faces,
        removed_faces,
        added_hash: *added_hasher.finalize().as_bytes(),
        removed_hash: *removed_hasher.finalize().as_bytes(),
    })
}

fn projected_faces(
    surface: &StreamSurface,
    mapping: Option<&[usize]>,
    limits: RoundTripLimits,
) -> Result<Vec<[usize; 3]>, QualificationError> {
    let mut projected = Vec::new();
    reserve_items(
        &mut projected,
        surface.faces.len(),
        "LandXML topology projection",
        limits,
    )?;
    for face in &surface.faces {
        let mut face = mapping.map_or(*face, |map| face.map(|index| map[index]));
        face.sort_unstable();
        projected.push(face);
    }
    projected.sort_unstable();
    Ok(projected)
}

fn hash_faces(faces: &[[usize; 3]]) -> [u8; 32] {
    let mut hasher = face_diff_hasher();
    for face in faces {
        update_face_hash(&mut hasher, face);
    }
    *hasher.finalize().as_bytes()
}

fn face_diff_hasher() -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(b"punctra-round-trip-face-diff-v1");
    hasher
}

fn update_face_hash(hasher: &mut Hasher, face: &[usize; 3]) {
    for vertex in face {
        hasher.update(&(*vertex as u64).to_le_bytes());
    }
}

fn reserve_items<T>(
    values: &mut Vec<T>,
    items: usize,
    limit: &'static str,
    limits: RoundTripLimits,
) -> Result<(), QualificationError> {
    let requested_bytes = retained_capacity_bytes::<T>(items);
    values.try_reserve_exact(items).map_err(|_| {
        QualificationError::resource(limit, requested_bytes, limits.retained_model_bytes)
    })?;
    let retained_bytes = retained_capacity_bytes::<T>(values.capacity());
    if retained_bytes > requested_bytes {
        return Err(QualificationError::resource(
            limit,
            retained_bytes,
            limits.retained_model_bytes,
        ));
    }
    Ok(())
}

fn retained_capacity_bytes<T>(items: usize) -> u64 {
    u64::try_from(items)
        .unwrap_or(u64::MAX)
        .saturating_mul(size_of::<T>() as u64)
}

struct BoundReport {
    hash: [u8; 32],
    bytes: u64,
    run: [u8; 16],
    source: [u8; 32],
    workspace: [u8; 16],
    baseline_revision: [u8; 32],
    operation: [u8; 16],
    request_hash: [u8; 32],
    ordinal_hash: [u8; 32],
    recipe_hash: [u8; 32],
    qa_input_hash: [u8; 32],
    options_hash: [u8; 32],
    path_bindings: [[u8; 32]; 4],
    revision: [u8; 32],
    audit_hash: [u8; 32],
    surface_hash: [u8; 32],
    qa_hash: [u8; 32],
    landxml_hash: [u8; 32],
    landxml_bytes: u64,
    witness: InputWitness,
}

#[cfg(test)]
fn read_bound_report(
    path: &Path,
    max_bytes: u64,
    control: &OperationControl,
) -> Result<BoundReport, QualificationError> {
    let opened = open_regular(path, "audit.json", max_bytes)?;
    read_open_bound_report(path, max_bytes, control, opened)
}

fn read_open_bound_report(
    path: &Path,
    max_bytes: u64,
    control: &OperationControl,
    (mut file, identity): (File, fs::Metadata),
) -> Result<BoundReport, QualificationError> {
    let mut bytes = Vec::new();
    let retained_capacity = identity.len().checked_add(1).ok_or_else(|| {
        QualificationError::resource("addressable audit.json bytes", u64::MAX, max_bytes)
    })?;
    bytes
        .try_reserve_exact(usize::try_from(retained_capacity).map_err(|_| {
            QualificationError::resource("addressable audit.json bytes", identity.len(), max_bytes)
        })?)
        .map_err(|_| {
            QualificationError::resource("audit.json retained bytes", identity.len(), max_bytes)
        })?;
    Read::by_ref(&mut file)
        .take(retained_capacity)
        .read_to_end(&mut bytes)
        .map_err(|source| QualificationError::io("read audit.json", path, source))?;
    if bytes.len() as u64 != identity.len() {
        return Err(QualificationError::InputChanged(path.to_path_buf()));
    }
    control
        .check_cancelled()
        .map_err(|_| QualificationError::Cancelled)?;
    let final_metadata = fs::symlink_metadata(path)
        .map_err(|source| QualificationError::io("reinspect audit.json", path, source))?;
    if !same_file_identity(&identity, &final_metadata)
        || !same_file_state(&identity, &final_metadata)
    {
        return Err(QualificationError::InputChanged(path.to_path_buf()));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|source| {
        QualificationError::Report(format!("audit.json is invalid JSON: {source}"))
    })?;
    if value.get("schema").and_then(Value::as_str) != Some("punctra.terrain-workflow.audit.v1") {
        return Err(QualificationError::Report(
            "audit.json schema differs".to_owned(),
        ));
    }
    let hash = report_hash(&bytes);
    let witness = InputWitness::new(path, &file, identity, hash, REPORT_HASH_DOMAIN)?;
    Ok(BoundReport {
        source: parse_json_hex(&value, &["identities", "source"])?,
        workspace: parse_json_hex(&value, &["identities", "workspace"])?,
        baseline_revision: parse_json_hex(&value, &["identities", "baseline_revision"])?,
        operation: parse_json_hex(&value, &["identities", "operation"])?,
        ordinal_hash: parse_json_hex(&value, &["request", "ordinal_hash"])?,
        recipe_hash: parse_json_hex(&value, &["request", "recipe_hash"])?,
        qa_input_hash: parse_json_hex(&value, &["request", "qa_input_hash"])?,
        options_hash: parse_json_hex(&value, &["request", "landxml_options_hash"])?,
        path_bindings: parse_json_path_bindings(&value)?,
        hash,
        bytes: bytes.len() as u64,
        run: parse_json_hex(&value, &["identities", "run"])?,
        request_hash: parse_json_hex(&value, &["request", "request_hash"])?,
        revision: parse_json_hex(&value, &["identities", "changed_revision"])?,
        landxml_hash: parse_json_hex(&value, &["landxml", "content_hash"])?,
        audit_hash: parse_json_hex(&value, &["edit", "audit_hash"])?,
        surface_hash: parse_json_hex(&value, &["terrain", "changed", "artifact_hash"])?,
        qa_hash: parse_json_hex(&value, &["qa", "result_hash"])?,
        landxml_bytes: value
            .pointer("/landxml/byte_length")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                QualificationError::Report("audit.json LandXML byte length is absent".to_owned())
            })?,
        witness,
    })
}

fn parse_json_hex<const N: usize>(
    value: &Value,
    path: &[&str],
) -> Result<[u8; N], QualificationError> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            QualificationError::Report(format!("audit.json field {} is absent", path.join(".")))
        })?;
    }
    let text = current.as_str().ok_or_else(|| {
        QualificationError::Report(format!("audit.json field {} is not text", path.join(".")))
    })?;
    if text.len() != N * 2 {
        return Err(QualificationError::Report(format!(
            "audit.json field {} has the wrong width",
            path.join(".")
        )));
    }
    let mut output = [0; N];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Ok(output)
}

fn parse_json_path_bindings(value: &Value) -> Result<[[u8; 32]; 4], QualificationError> {
    let values = value
        .pointer("/request/path_bindings")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 4)
        .ok_or_else(|| {
            QualificationError::Report(
                "audit.json request.path_bindings must contain four hashes".to_owned(),
            )
        })?;
    let mut bindings = [[0_u8; 32]; 4];
    for (index, value) in values.iter().enumerate() {
        let text = value.as_str().ok_or_else(|| {
            QualificationError::Report(format!(
                "audit.json request.path_bindings[{index}] is not text"
            ))
        })?;
        if text.len() != 64 {
            return Err(QualificationError::Report(format!(
                "audit.json request.path_bindings[{index}] has the wrong width"
            )));
        }
        for (position, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            bindings[index][position] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
        }
    }
    Ok(bindings)
}

fn hex_digit(value: u8) -> Result<u8, QualificationError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(QualificationError::Report(
            "audit.json contains noncanonical hexadecimal text".to_owned(),
        )),
    }
}

fn report_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(REPORT_HASH_DOMAIN);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn bind_complete_run(
    journal: &CompleteRunSnapshot,
    original: &CapturedLandXml,
    report: &BoundReport,
    run_root: &Path,
) -> Result<(), QualificationError> {
    let actual_run_binding =
        bind_path(run_root, PATH_BINDING_BYTES).map_err(QualificationError::Journal)?;
    if !original.metric_metres
        || journal.export.content_hash != original.hash
        || journal.export.byte_length != original.bytes
        || journal.export.revision != journal.complete.revision
        || journal.report.report_hash != report.hash
        || journal.report.byte_length != report.bytes
        || journal.report.revision != journal.complete.revision
        || journal.report.audit_hash != journal.complete.audit_hash
        || journal.report.surface_hash != journal.complete.surface_hash
        || journal.report.qa_hash != journal.complete.qa_hash
        || journal.report.landxml_hash != original.hash
        || journal.complete.report_hash != report.hash
        || journal.complete.landxml_hash != original.hash
        || journal.complete.request_hash != journal.intent.request_hash
        || journal.intent.path_bindings[3] != actual_run_binding
        || journal.export.target_binding != actual_run_binding
        || report.run != journal.run.into_bytes()
        || report.source != journal.intent.source
        || report.workspace != journal.intent.workspace
        || report.baseline_revision != journal.intent.baseline_revision
        || report.operation != journal.intent.operation
        || report.request_hash != journal.intent.request_hash
        || report.ordinal_hash != journal.intent.ordinal_hash
        || report.recipe_hash != journal.intent.recipe_hash
        || report.qa_input_hash != journal.intent.qa_input_hash
        || report.options_hash != journal.intent.options_hash
        || report.path_bindings != journal.intent.path_bindings
        || report.revision != journal.complete.revision
        || report.audit_hash != journal.complete.audit_hash
        || report.surface_hash != journal.complete.surface_hash
        || report.qa_hash != journal.complete.qa_hash
        || report.landxml_hash != original.hash
        || report.landxml_bytes != original.bytes
    {
        return Err(QualificationError::Provenance(
            "Complete journal, terrain.xml, and audit.json bindings differ",
        ));
    }
    Ok(())
}

struct Evidence<'a> {
    journal: &'a CompleteRunSnapshot,
    original: &'a CapturedLandXml,
    report: &'a BoundReport,
    returned: &'a CapturedLandXml,
    declaration: &'a RoundTripDeclaration,
    downstream_settings: &'a [(String, String)],
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
    outcome: SemanticOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CheckFact {
    status: &'static str,
    reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EvidenceChecks {
    units: CheckFact,
    unique_mapping: CheckFact,
    tolerance: CheckFact,
    topology: CheckFact,
}

const PASSED_CHECK: CheckFact = CheckFact {
    status: "passed",
    reason: "none",
};
const NOT_EVALUATED_CHECK: CheckFact = CheckFact {
    status: "not_evaluated",
    reason: "none",
};

fn evidence_checks(metric_metres: bool, outcome: SemanticOutcome) -> EvidenceChecks {
    if !metric_metres || outcome.reason == "PRT_UNIT_DRIFT" {
        return EvidenceChecks {
            units: CheckFact {
                status: "failed",
                reason: "PRT_UNIT_DRIFT",
            },
            unique_mapping: NOT_EVALUATED_CHECK,
            tolerance: NOT_EVALUATED_CHECK,
            topology: NOT_EVALUATED_CHECK,
        };
    }
    if outcome.passed {
        return EvidenceChecks {
            units: PASSED_CHECK,
            unique_mapping: PASSED_CHECK,
            tolerance: PASSED_CHECK,
            topology: PASSED_CHECK,
        };
    }
    if outcome.reason == "PRT_TOPOLOGY_DRIFT" {
        return EvidenceChecks {
            units: PASSED_CHECK,
            unique_mapping: PASSED_CHECK,
            tolerance: PASSED_CHECK,
            topology: CheckFact {
                status: "failed",
                reason: "PRT_TOPOLOGY_DRIFT",
            },
        };
    }
    if outcome.reason == "PRT_TOLERANCE_DRIFT" {
        return EvidenceChecks {
            units: PASSED_CHECK,
            unique_mapping: CheckFact {
                status: "failed",
                reason: "PRT_TOLERANCE_DRIFT",
            },
            tolerance: CheckFact {
                status: "failed",
                reason: "PRT_TOLERANCE_DRIFT",
            },
            topology: NOT_EVALUATED_CHECK,
        };
    }
    EvidenceChecks {
        units: PASSED_CHECK,
        unique_mapping: CheckFact {
            status: "failed",
            reason: outcome.reason,
        },
        tolerance: NOT_EVALUATED_CHECK,
        topology: NOT_EVALUATED_CHECK,
    }
}

fn write_check(writer: &mut dyn Write, check: CheckFact) -> io::Result<()> {
    writer.write_all(b"{\"status\":")?;
    json_string(writer, check.status)?;
    writer.write_all(b",\"reason\":")?;
    json_string(writer, check.reason)?;
    writer.write_all(b"}")
}

fn write_evidence(writer: &mut dyn Write, evidence: &Evidence<'_>) -> io::Result<()> {
    let checks = evidence_checks(evidence.returned.metric_metres, evidence.outcome);
    write!(writer, "{{\"schema\":")?;
    json_string(writer, EVIDENCE_SCHEMA)?;
    write!(
        writer,
        ",\"result\":\"{}\",\"run\":{{\"run_identity\":",
        if evidence.outcome.passed {
            "passed"
        } else {
            "failed"
        }
    )?;
    json_hex(writer, &evidence.journal.run.into_bytes())?;
    write!(writer, ",\"request_hash\":")?;
    json_hex(writer, &evidence.journal.intent.request_hash)?;
    write!(writer, ",\"complete_journal_hash\":")?;
    json_hex(writer, &evidence.journal.journal_hash)?;
    write!(
        writer,
        ",\"complete_journal_bytes\":{},\"terrain_xml_hash\":",
        evidence.journal.journal_bytes
    )?;
    json_hex(writer, &evidence.original.hash)?;
    write!(
        writer,
        ",\"terrain_xml_bytes\":{},\"audit_json_hash\":",
        evidence.original.bytes
    )?;
    json_hex(writer, &evidence.report.hash)?;
    write!(
        writer,
        ",\"audit_json_bytes\":{}}},\"downstream_declaration\":{{\"application\":",
        evidence.report.bytes
    )?;
    json_string(writer, evidence.declaration.declared_application())?;
    write!(writer, ",\"version\":")?;
    json_string(writer, evidence.declaration.declared_version())?;
    write!(writer, ",\"settings\":[")?;
    for (index, (key, value)) in evidence.downstream_settings.iter().enumerate() {
        if index != 0 {
            writer.write_all(b",")?;
        }
        writer.write_all(b"{\"key\":")?;
        json_string(writer, key)?;
        writer.write_all(b",\"value\":")?;
        json_string(writer, value)?;
        writer.write_all(b"}")?;
    }
    writer.write_all(b"]")?;
    write!(
        writer,
        "}},\"comparison_policy\":{{\"horizontal_tolerance_metres\":"
    )?;
    json_f64(writer, evidence.tolerances.horizontal_metres())?;
    write!(writer, ",\"vertical_tolerance_metres\":")?;
    json_f64(writer, evidence.tolerances.vertical_metres())?;
    write!(
        writer,
        ",\"matcher_version\":{MATCHER_VERSION}}},\"returned_landxml\":{{\"content_hash\":"
    )?;
    json_hex(writer, &evidence.returned.hash)?;
    write!(
        writer,
        ",\"bytes\":{},\"namespace\":",
        evidence.returned.bytes
    )?;
    json_string(
        writer,
        str::from_utf8(LANDXML_NAMESPACE).expect("constant namespace is UTF-8"),
    )?;
    write!(writer, ",\"declared_units\":")?;
    json_string(
        writer,
        if evidence.returned.metric_metres {
            "metric_metre"
        } else {
            "non_metric_or_missing"
        },
    )?;
    write!(writer, ",\"surface_name\":")?;
    json_string(writer, &evidence.returned.surface_name)?;
    write!(
        writer,
        ",\"point_count\":{},\"face_count\":{},\"ignored_top_level_sections\":[",
        evidence.returned.surface.points.len(),
        evidence.returned.surface.faces.len(),
    )?;
    for (index, section) in evidence
        .returned
        .ignored_top_level_sections
        .iter()
        .enumerate()
    {
        if index != 0 {
            writer.write_all(b",")?;
        }
        json_string(writer, section)?;
    }
    writer.write_all(b"]},\"checks\":{\"provenance\":{\"status\":\"passed\",\"reason\":\"none\"},\"parse\":{\"status\":\"passed\",\"reason\":\"none\"},\"units\":")?;
    write_check(writer, checks.units)?;
    writer.write_all(b",\"unique_mapping\":")?;
    write_check(writer, checks.unique_mapping)?;
    writer.write_all(b",\"tolerance\":")?;
    write_check(writer, checks.tolerance)?;
    writer.write_all(b",\"topology\":")?;
    write_check(writer, checks.topology)?;
    write!(
        writer,
        "}},\"comparison\":{{\"mapped_point_count\":{},\"unmatched_point_count\":{},\"ambiguous_point_count\":{},\"candidate_comparison_count\":{},\"maximum_easting_delta_metres\":",
        evidence.outcome.mapped,
        evidence.outcome.unmatched,
        evidence.outcome.ambiguous,
        evidence.outcome.comparison.comparison_count
    )?;
    json_f64(writer, evidence.outcome.comparison.max_easting_drift_metres)?;
    write!(writer, ",\"maximum_northing_delta_metres\":")?;
    json_f64(
        writer,
        evidence.outcome.comparison.max_northing_drift_metres,
    )?;
    write!(writer, ",\"maximum_horizontal_delta_metres\":")?;
    json_f64(
        writer,
        evidence.outcome.comparison.max_horizontal_drift_metres,
    )?;
    write!(writer, ",\"maximum_vertical_delta_metres\":")?;
    json_f64(
        writer,
        evidence.outcome.comparison.max_vertical_drift_metres,
    )?;
    write!(
        writer,
        ",\"added_face_count\":{},\"removed_face_count\":{},\"added_face_hash\":",
        evidence.outcome.added_faces, evidence.outcome.removed_faces
    )?;
    json_hex(writer, &evidence.outcome.added_hash)?;
    write!(writer, ",\"removed_face_hash\":")?;
    json_hex(writer, &evidence.outcome.removed_hash)?;
    writeln!(
        writer,
        ",\"added_face_sample\":[],\"removed_face_sample\":[]}},\"limits\":{{\"file_bytes\":{},\"xml_nodes\":{},\"xml_text_and_attribute_bytes\":{},\"points\":{},\"faces\":{},\"candidate_comparisons\":{},\"xml_token_bytes\":{},\"retained_model_bytes\":{},\"evidence_output_bytes\":{}}},\"nonclaims\":{{\"punctra_observed_downstream_execution\":false,\"vendor_certification\":false,\"firm_acceptance\":false,\"paid_use\":false,\"conversion\":false,\"measured_labor_savings\":false}}}}",
        evidence.limits.file_bytes,
        evidence.limits.xml_nodes,
        evidence.limits.xml_text_bytes,
        evidence.limits.points,
        evidence.limits.faces,
        evidence.limits.comparisons,
        MAX_EVENT_BYTES,
        evidence.limits.retained_model_bytes,
        EVIDENCE_BYTES
    )
}

fn validate_paths(request: &QualificationRequest) -> Result<(), QualificationError> {
    let run_root = fs::canonicalize(&request.run_root)
        .map_err(|source| QualificationError::io("resolve Run root", &request.run_root, source))?;
    let evidence_parent = request
        .evidence_target
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let evidence_parent = fs::canonicalize(evidence_parent).map_err(|source| {
        QualificationError::io("resolve evidence parent", evidence_parent, source)
    })?;
    if evidence_parent.starts_with(&run_root) {
        return Err(QualificationError::Invalid(
            "evidence target must be outside the Run root",
        ));
    }
    Ok(())
}

fn open_regular(
    path: &Path,
    label: &'static str,
    max_bytes: u64,
) -> Result<(File, fs::Metadata), QualificationError> {
    let initial = fs::symlink_metadata(path)
        .map_err(|source| QualificationError::io("inspect qualification input", path, source))?;
    if !initial.file_type().is_file() {
        return Err(QualificationError::Invalid(
            "qualification inputs must be regular non-symlink files",
        ));
    }
    if initial.len() > max_bytes {
        return Err(QualificationError::resource(
            label,
            initial.len(),
            max_bytes,
        ));
    }
    let file = platform_open_nofollow(path)
        .map_err(|source| QualificationError::io("open qualification input", path, source))?;
    let opened = file.metadata().map_err(|source| {
        QualificationError::io("inspect open qualification input", path, source)
    })?;
    if !same_file_identity(&initial, &opened) || !same_file_state(&initial, &opened) {
        return Err(QualificationError::InputChanged(path.to_path_buf()));
    }
    Ok((file, opened))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn platform_open_nofollow(path: &Path) -> io::Result<File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};

    openat(
        CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_open_nofollow(path: &Path) -> io::Result<File> {
    File::open(path)
}

struct InputWitness {
    path: PathBuf,
    file: File,
    identity: fs::Metadata,
    expected_hash: [u8; 32],
    hash_domain: &'static [u8],
}

impl InputWitness {
    fn new(
        path: &Path,
        file: &File,
        identity: fs::Metadata,
        expected_hash: [u8; 32],
        hash_domain: &'static [u8],
    ) -> Result<Self, QualificationError> {
        let file = file
            .try_clone()
            .map_err(|source| QualificationError::io("retain qualification input", path, source))?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
            expected_hash,
            hash_domain,
        })
    }

    fn verify(&self) -> Result<(), QualificationError> {
        let opened = self.file.metadata().map_err(|source| {
            QualificationError::io("reinspect retained qualification input", &self.path, source)
        })?;
        let target = fs::symlink_metadata(&self.path).map_err(|source| {
            QualificationError::io("reinspect qualification input path", &self.path, source)
        })?;
        if !target.file_type().is_file()
            || !same_file_identity(&self.identity, &opened)
            || !same_file_identity(&opened, &target)
            || !same_file_state(&self.identity, &opened)
            || !same_file_state(&opened, &target)
        {
            return Err(QualificationError::InputChanged(self.path.clone()));
        }
        let mut reader = self.file.try_clone().map_err(|source| {
            QualificationError::io("clone retained qualification input", &self.path, source)
        })?;
        reader.seek(SeekFrom::Start(0)).map_err(|source| {
            QualificationError::io("seek retained qualification input", &self.path, source)
        })?;
        let mut hasher = Hasher::new();
        hasher.update(self.hash_domain);
        let mut remaining = self.identity.len();
        let mut buffer = [0_u8; 8 * 1024];
        while remaining != 0 {
            let requested = usize::try_from(remaining.min(buffer.len() as u64))
                .expect("bounded input hash read fits usize");
            let read = reader.read(&mut buffer[..requested]).map_err(|source| {
                QualificationError::io("rehash retained qualification input", &self.path, source)
            })?;
            if read == 0 {
                return Err(QualificationError::InputChanged(self.path.clone()));
            }
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        let opened_after = self.file.metadata().map_err(|source| {
            QualificationError::io("reinspect retained qualification input", &self.path, source)
        })?;
        let target_after = fs::symlink_metadata(&self.path).map_err(|source| {
            QualificationError::io("reinspect qualification input path", &self.path, source)
        })?;
        if hasher.finalize().as_bytes() != &self.expected_hash
            || !same_file_identity(&self.identity, &opened_after)
            || !same_file_identity(&opened_after, &target_after)
            || !same_file_state(&self.identity, &opened_after)
            || !same_file_state(&opened_after, &target_after)
        {
            return Err(QualificationError::InputChanged(self.path.clone()));
        }
        Ok(())
    }
}

fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && matches!(
            (left.modified(), right.modified()),
            (Ok(left_modified), Ok(right_modified)) if left_modified == right_modified
        )
}

struct HashingRead<'a> {
    file: File,
    hasher: &'a mut Hasher,
    remaining: u64,
    control: &'a OperationControl,
    token_limit: u64,
    token_bytes: u64,
    lexical_state: XmlLexicalState,
}

#[derive(Clone, Copy)]
enum XmlLexicalState {
    Text,
    MarkupStart,
    Tag {
        quote: Option<u8>,
    },
    BangStart,
    BangDash,
    CdataPrefix {
        matched: usize,
    },
    Comment {
        trailing_hyphens: u8,
    },
    Cdata {
        trailing_brackets: u8,
    },
    ProcessingInstruction {
        trailing_question: bool,
    },
    Declaration {
        quote: Option<u8>,
        internal_subset_depth: u32,
        comment_prefix: u8,
    },
    DeclarationComment {
        internal_subset_depth: u32,
        trailing_hyphens: u8,
    },
}

impl<'a> HashingRead<'a> {
    fn new(
        file: File,
        hasher: &'a mut Hasher,
        bytes: u64,
        control: &'a OperationControl,
        token_limit: u64,
    ) -> Self {
        Self {
            file,
            hasher,
            remaining: bytes,
            control,
            token_limit,
            token_bytes: 0,
            lexical_state: XmlLexicalState::Text,
        }
    }

    fn check_tokens(&mut self, bytes: &[u8]) -> io::Result<()> {
        const CDATA_PREFIX: &[u8] = b"CDATA[";

        for &byte in bytes {
            if matches!(self.lexical_state, XmlLexicalState::Text) && byte == b'<' {
                self.token_bytes = 1;
                self.ensure_token_limit()?;
                self.lexical_state = XmlLexicalState::MarkupStart;
                continue;
            }
            self.token_bytes = self.token_bytes.saturating_add(1);
            self.ensure_token_limit()?;
            let previous = self.lexical_state;
            let next = match previous {
                XmlLexicalState::Text => XmlLexicalState::Text,
                XmlLexicalState::MarkupStart => match byte {
                    b'?' => XmlLexicalState::ProcessingInstruction {
                        trailing_question: false,
                    },
                    b'!' => XmlLexicalState::BangStart,
                    _ => Self::tag_state(None, byte),
                },
                XmlLexicalState::Tag {
                    quote: Some(expected),
                } => {
                    if byte == expected {
                        XmlLexicalState::Tag { quote: None }
                    } else {
                        XmlLexicalState::Tag {
                            quote: Some(expected),
                        }
                    }
                }
                XmlLexicalState::Tag { quote: None } => Self::tag_state(None, byte),
                XmlLexicalState::BangStart => match byte {
                    b'-' => XmlLexicalState::BangDash,
                    b'[' => XmlLexicalState::CdataPrefix { matched: 0 },
                    _ => Self::declaration(),
                },
                XmlLexicalState::BangDash => {
                    if byte == b'-' {
                        XmlLexicalState::Comment {
                            trailing_hyphens: 0,
                        }
                    } else {
                        Self::declaration()
                    }
                }
                XmlLexicalState::CdataPrefix { matched }
                    if CDATA_PREFIX.get(matched) == Some(&byte) =>
                {
                    if matched + 1 == CDATA_PREFIX.len() {
                        XmlLexicalState::Cdata {
                            trailing_brackets: 0,
                        }
                    } else {
                        XmlLexicalState::CdataPrefix {
                            matched: matched + 1,
                        }
                    }
                }
                XmlLexicalState::CdataPrefix { .. } => Self::declaration(),
                XmlLexicalState::Comment { trailing_hyphens } => {
                    if byte == b'>' && trailing_hyphens >= 2 {
                        XmlLexicalState::Text
                    } else {
                        XmlLexicalState::Comment {
                            trailing_hyphens: if byte == b'-' {
                                trailing_hyphens.saturating_add(1)
                            } else {
                                0
                            },
                        }
                    }
                }
                XmlLexicalState::Cdata { trailing_brackets } => {
                    if byte == b'>' && trailing_brackets >= 2 {
                        XmlLexicalState::Text
                    } else {
                        XmlLexicalState::Cdata {
                            trailing_brackets: if byte == b']' {
                                trailing_brackets.saturating_add(1)
                            } else {
                                0
                            },
                        }
                    }
                }
                XmlLexicalState::ProcessingInstruction { trailing_question } => {
                    if byte == b'>' && trailing_question {
                        XmlLexicalState::Text
                    } else {
                        XmlLexicalState::ProcessingInstruction {
                            trailing_question: byte == b'?',
                        }
                    }
                }
                XmlLexicalState::Declaration {
                    quote,
                    internal_subset_depth,
                    comment_prefix,
                } => Self::declaration_state(quote, internal_subset_depth, comment_prefix, byte),
                XmlLexicalState::DeclarationComment {
                    internal_subset_depth,
                    trailing_hyphens,
                } => {
                    if byte == b'>' && trailing_hyphens >= 2 {
                        XmlLexicalState::Declaration {
                            quote: None,
                            internal_subset_depth,
                            comment_prefix: 0,
                        }
                    } else {
                        XmlLexicalState::DeclarationComment {
                            internal_subset_depth,
                            trailing_hyphens: if byte == b'-' {
                                trailing_hyphens.saturating_add(1)
                            } else {
                                0
                            },
                        }
                    }
                }
            };
            if matches!(next, XmlLexicalState::Text) && !matches!(previous, XmlLexicalState::Text) {
                self.finish_token();
            }
            self.lexical_state = next;
        }
        Ok(())
    }

    fn tag_state(quote: Option<u8>, byte: u8) -> XmlLexicalState {
        if quote.is_none() && matches!(byte, b'\'' | b'\"') {
            XmlLexicalState::Tag { quote: Some(byte) }
        } else if quote.is_none() && byte == b'>' {
            XmlLexicalState::Text
        } else {
            XmlLexicalState::Tag { quote }
        }
    }

    const fn declaration() -> XmlLexicalState {
        XmlLexicalState::Declaration {
            quote: None,
            internal_subset_depth: 0,
            comment_prefix: 0,
        }
    }

    fn declaration_state(
        quote: Option<u8>,
        internal_subset_depth: u32,
        comment_prefix: u8,
        byte: u8,
    ) -> XmlLexicalState {
        if let Some(expected) = quote {
            return XmlLexicalState::Declaration {
                quote: (byte != expected).then_some(expected),
                internal_subset_depth,
                comment_prefix: 0,
            };
        }
        if matches!(byte, b'\'' | b'\"') {
            return XmlLexicalState::Declaration {
                quote: Some(byte),
                internal_subset_depth,
                comment_prefix: 0,
            };
        }
        let internal_subset_depth = match byte {
            b'[' => internal_subset_depth.saturating_add(1),
            b']' => internal_subset_depth.saturating_sub(1),
            _ => internal_subset_depth,
        };
        if byte == b'>' && internal_subset_depth == 0 {
            return XmlLexicalState::Text;
        }
        let comment_prefix = match (comment_prefix, byte) {
            (1, b'!') => 2,
            (2, b'-') => 3,
            (3, b'-') => {
                return XmlLexicalState::DeclarationComment {
                    internal_subset_depth,
                    trailing_hyphens: 0,
                };
            }
            (_, b'<') => 1,
            _ => 0,
        };
        XmlLexicalState::Declaration {
            quote: None,
            internal_subset_depth,
            comment_prefix,
        }
    }

    fn ensure_token_limit(&self) -> io::Result<()> {
        if self.token_bytes > self.token_limit {
            Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "LandXML XML token exceeded the hard byte ceiling",
            ))
        } else {
            Ok(())
        }
    }

    fn finish_token(&mut self) {
        self.token_bytes = 0;
    }
}

impl Read for HashingRead<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.control.check_cancelled().map_err(io::Error::other)?;
        if self.remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let requested = usize::try_from(self.remaining.min(buffer.len() as u64))
            .expect("bounded qualification read fits usize");
        let read = self.file.read(&mut buffer[..requested])?;
        if read == 0 && self.remaining != 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "qualification input was truncated",
            ));
        }
        self.check_tokens(&buffer[..read])?;
        self.remaining -= read as u64;
        self.hasher.update(&buffer[..read]);
        Ok(read)
    }
}

fn require_namespace(
    side: StreamSide,
    namespace: &ResolveResult<'_>,
) -> Result<(), QualificationError> {
    match namespace {
        ResolveResult::Bound(Namespace(value)) if *value == LANDXML_NAMESPACE => Ok(()),
        _ => Err(QualificationError::Subset(match side {
            StreamSide::Original => "original terrain.xml namespace differs",
            StreamSide::Returned => "returned LandXML namespace differs",
        })),
    }
}

fn reject_xinclude(
    side: StreamSide,
    namespace: &ResolveResult<'_>,
) -> Result<(), QualificationError> {
    if matches!(namespace, ResolveResult::Bound(Namespace(value)) if *value == XINCLUDE_NAMESPACE) {
        Err(QualificationError::Subset(match side {
            StreamSide::Original => "original terrain.xml must not contain XInclude elements",
            StreamSide::Returned => "returned LandXML must not contain XInclude elements",
        }))
    } else {
        Ok(())
    }
}

fn valid_builtin_reference(reference: &[u8]) -> bool {
    if matches!(reference, b"amp" | b"lt" | b"gt" | b"apos" | b"quot") {
        return true;
    }
    let Some(numeric) = reference.strip_prefix(b"#") else {
        return false;
    };
    let (digits, radix) = numeric
        .strip_prefix(b"x")
        .map_or((numeric, 10), |digits| (digits, 16));
    if digits.is_empty() {
        return false;
    }
    let Ok(digits) = str::from_utf8(digits) else {
        return false;
    };
    let Ok(value) = u32::from_str_radix(digits, radix) else {
        return false;
    };
    matches!(
        value,
        0x9 | 0xa | 0xd | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x0010_ffff
    )
}

fn parse_positive_id(side: StreamSide, value: &str) -> Result<u64, QualificationError> {
    let id = value.parse::<u64>().map_err(|_| {
        QualificationError::Xml(format!(
            "{} contains an invalid Point identifier",
            side.label()
        ))
    })?;
    if id == 0 {
        Err(QualificationError::Subset(
            "Point identifiers must be positive",
        ))
    } else {
        Ok(id)
    }
}

fn parse_coordinate(side: StreamSide, value: Option<&str>) -> Result<f64, QualificationError> {
    let value = value.ok_or(QualificationError::Subset("P requires three coordinates"))?;
    let value = value.parse::<f64>().map_err(|_| {
        QualificationError::Xml(format!("{} contains an invalid coordinate", side.label()))
    })?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(QualificationError::Subset(
            "Point coordinates must be finite",
        ))
    }
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn json_string(writer: &mut dyn Write, value: &str) -> io::Result<()> {
    write!(
        writer,
        "{}",
        serde_json::to_string(value).map_err(io::Error::other)?
    )
}

fn json_hex(writer: &mut dyn Write, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for byte in bytes {
        write!(writer, "{byte:02x}")?;
    }
    writer.write_all(b"\"")
}

fn json_f64(writer: &mut dyn Write, value: f64) -> io::Result<()> {
    if value.is_finite() {
        write!(writer, "{value:.17}")
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "non-finite evidence number",
        ))
    }
}

#[derive(Debug)]
pub(crate) enum QualificationError {
    Invalid(&'static str),
    Resource {
        limit: &'static str,
        required: u64,
        allowed: u64,
    },
    Cancelled,
    InputChanged(PathBuf),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Journal(crate::journal::JournalError),
    Xml(String),
    Subset(&'static str),
    Report(String),
    Provenance(&'static str),
    Publication(ReportError),
}

impl QualificationError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }

    fn resource(limit: &'static str, required: u64, allowed: u64) -> Self {
        Self::Resource {
            limit,
            required,
            allowed,
        }
    }

    pub(crate) const fn is_publication_indeterminate(&self) -> bool {
        matches!(self, Self::Publication(ReportError::Indeterminate { .. }))
    }
}

impl std::fmt::Display for QualificationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(value) | Self::Subset(value) | Self::Provenance(value) => {
                formatter.write_str(value)
            }
            Self::Resource {
                limit,
                required,
                allowed,
            } => write!(formatter, "{limit} requires {required}; limit is {allowed}"),
            Self::Cancelled => formatter.write_str("qualification was cancelled"),
            Self::InputChanged(path) => {
                write!(formatter, "qualification input changed: {}", path.display())
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::Journal(value) => write!(formatter, "{value}"),
            Self::Xml(value) | Self::Report(value) => formatter.write_str(value),
            Self::Publication(value) => write!(formatter, "{value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{File, FileTimes, OpenOptions},
        io::{Read as _, Write as _},
    };

    use super::*;

    #[test]
    fn qualification_stream_ceiling_covers_the_v07_export_ceiling() {
        let limits = RoundTripLimits::qualification();
        assert_eq!(limits.file_bytes, 4 * 1024 * 1024 * 1024);
        assert!(limits.xml_text_bytes >= limits.file_bytes);
        assert_eq!(limits.xml_nodes, 60_000_000);
        assert_eq!(limits.points, 10_000_000);
        assert_eq!(limits.faces, 20_000_000);
        assert_eq!(limits.comparisons, 160_000_000);
        assert_eq!(limits.retained_model_bytes, 4 * 1024 * 1024 * 1024);
        assert!(StreamState::required_retained_model_bytes(limits) <= limits.retained_model_bytes);

        let directory = TestDirectory::new("sparse-file-ceiling");
        let path = directory.path.join("ceiling.xml");
        let file = File::create(&path).unwrap();
        file.set_len(limits.file_bytes).unwrap();
        let (_, metadata) = open_regular(&path, "sparse ceiling", limits.file_bytes).unwrap();
        assert_eq!(metadata.len(), limits.file_bytes);
        file.set_len(limits.file_bytes + 1).unwrap();
        assert!(matches!(
            open_regular(&path, "sparse ceiling", limits.file_bytes),
            Err(QualificationError::Resource { required, allowed, .. })
                if required == limits.file_bytes + 1 && allowed == limits.file_bytes
        ));
    }

    #[test]
    fn checked_in_qualification_corpus_pins_bytes_versions_identities_and_semantics() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qualification-v1");
        let before = corpus_snapshot(&root);
        let manifest: Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest["schema"],
            "punctra.terrain-demo.qualification-corpus.v1"
        );
        assert_eq!(manifest["owner"], "terrain-demo");
        assert_eq!(manifest["path_base"], "manifest_directory");
        assert_eq!(manifest["run_versions"]["disk"], 1);
        assert_eq!(manifest["run_versions"]["semantic"], 1);
        assert_eq!(manifest["run_versions"]["frame"], 1);
        assert_eq!(
            manifest["report_schema"],
            "punctra.terrain-workflow.audit.v1"
        );
        assert_eq!(manifest["evidence_schema"], EVIDENCE_SCHEMA);
        assert_eq!(manifest["matcher_version"], MATCHER_VERSION);
        assert_eq!(manifest["declaration"]["application"], "generated-fixture");
        assert_eq!(manifest["declaration"]["version"], "test-only");
        assert_eq!(manifest["tolerances_metres"]["horizontal"], 0.001);
        assert_eq!(manifest["tolerances_metres"]["vertical"], 0.001);

        let entries = manifest["entries"].as_object().unwrap();
        assert_eq!(entries.len(), 15);
        for (name, facts) in entries {
            let bytes = fs::read(root.join(name)).unwrap();
            assert_eq!(facts["byte_length"], bytes.len() as u64, "{name}");
            assert_eq!(
                facts["blake3"],
                blake3::hash(&bytes).to_hex().to_string(),
                "{name}"
            );
            let expected_support = if name.starts_with("run-") {
                "authoritative"
            } else if matches!(name.as_str(), "returned-pass.xml" | "returned-fail.xml") {
                "test_only_input"
            } else {
                "caller_owned_published"
            };
            assert_eq!(facts["support_class"], expected_support, "{name}");
        }

        let complete_bytes = fs::read(root.join("run-complete.pwf")).unwrap();
        let prefixes = manifest["journal_checkpoint_prefixes"].as_array().unwrap();
        assert_eq!(prefixes.len(), 8);
        let mut prior_length = 0;
        for (index, name) in prefixes.iter().enumerate() {
            let name = name.as_str().unwrap();
            assert_eq!(name, format!("run-prefix-{:02}.pwf", index + 1));
            let path = root.join(name);
            let bytes = fs::read(&path).unwrap();
            assert!(complete_bytes.starts_with(&bytes));
            assert!(bytes.len() > prior_length);
            prior_length = bytes.len();
            if index == 7 {
                assert!(read_complete_run(&path, JournalLimits::default()).is_ok());
            } else {
                assert!(read_complete_run(&path, JournalLimits::default()).is_err());
            }
        }
        assert_eq!(
            fs::read(root.join("run-prefix-08.pwf")).unwrap(),
            complete_bytes
        );

        let journal =
            read_complete_run(&root.join("run-complete.pwf"), JournalLimits::default()).unwrap();
        let control = OperationControl::new();
        let limits = RoundTripLimits::qualification();
        let original = read_landxml(
            &root.join("terrain.xml"),
            StreamSide::Original,
            limits,
            &control,
        )
        .unwrap();
        let returned_pass = read_landxml(
            &root.join("returned-pass.xml"),
            StreamSide::Returned,
            limits,
            &control,
        )
        .unwrap();
        let returned_fail = read_landxml(
            &root.join("returned-fail.xml"),
            StreamSide::Returned,
            limits,
            &control,
        )
        .unwrap();
        let report = read_bound_report(&root.join("audit.json"), EVIDENCE_BYTES, &control).unwrap();
        assert_eq!(journal.export.content_hash, original.hash);
        assert_eq!(journal.export.byte_length, original.bytes);
        assert_eq!(journal.report.report_hash, report.hash);
        assert_eq!(journal.report.byte_length, report.bytes);
        assert_eq!(report.path_bindings, journal.intent.path_bindings);
        assert_eq!(report.source, journal.intent.source);
        assert_eq!(report.workspace, journal.intent.workspace);
        assert_eq!(report.operation, journal.intent.operation);

        let tolerances = RoundTripTolerances::new(0.001, 0.001).unwrap();
        let passed = evaluate_semantics(
            &original.surface,
            &returned_pass.surface,
            returned_pass.metric_metres,
            tolerances,
            limits,
        )
        .unwrap();
        assert!(passed.passed);
        assert_eq!(passed.mapped, 62);
        let failed = evaluate_semantics(
            &original.surface,
            &returned_fail.surface,
            returned_fail.metric_metres,
            tolerances,
            limits,
        )
        .unwrap();
        assert!(!failed.passed);
        assert_eq!(failed.reason, "PRT_TOLERANCE_DRIFT");
        assert_eq!(failed.unmatched, 1);

        let declaration =
            RoundTripDeclaration::new("generated-fixture", "test-only", "structured-settings-v1")
                .unwrap();
        let settings = vec![
            ("format".to_owned(), "LandXML;1.2".to_owned()),
            ("units".to_owned(), "meter".to_owned()),
        ];
        for (name, returned, outcome) in [
            ("evidence-pass.json", &returned_pass, passed),
            ("evidence-fail.json", &returned_fail, failed),
        ] {
            let evidence = Evidence {
                journal: &journal,
                original: &original,
                report: &report,
                returned,
                declaration: &declaration,
                downstream_settings: &settings,
                tolerances,
                limits,
                outcome,
            };
            let mut rendered = Vec::new();
            write_evidence(&mut rendered, &evidence).unwrap();
            assert_eq!(rendered, fs::read(root.join(name)).unwrap(), "{name}");
        }

        let pass_evidence: Value =
            serde_json::from_slice(&fs::read(root.join("evidence-pass.json")).unwrap()).unwrap();
        let fail_evidence: Value =
            serde_json::from_slice(&fs::read(root.join("evidence-fail.json")).unwrap()).unwrap();
        let expected = &manifest["expected"];
        assert_eq!(pass_evidence["schema"], EVIDENCE_SCHEMA);
        assert_eq!(pass_evidence["result"], expected["passing_result"]);
        assert_eq!(
            pass_evidence["run"]["run_identity"],
            expected["run_identity"]
        );
        assert_eq!(
            pass_evidence["run"]["request_hash"],
            expected["request_hash"]
        );
        assert_eq!(
            pass_evidence["run"]["complete_journal_hash"],
            expected["complete_journal_hash"]
        );
        assert_eq!(
            pass_evidence["run"]["terrain_xml_hash"],
            expected["terrain_xml_hash"]
        );
        assert_eq!(
            pass_evidence["run"]["terrain_xml_bytes"],
            expected["terrain_xml_bytes"]
        );
        assert_eq!(
            pass_evidence["run"]["audit_json_hash"],
            expected["audit_json_hash"]
        );
        assert_eq!(
            pass_evidence["run"]["audit_json_bytes"],
            expected["audit_json_bytes"]
        );
        assert_eq!(
            pass_evidence["returned_landxml"]["point_count"],
            expected["passing_point_count"]
        );
        assert_eq!(
            pass_evidence["returned_landxml"]["face_count"],
            expected["passing_face_count"]
        );
        assert_eq!(
            pass_evidence["comparison"]["mapped_point_count"],
            expected["passing_mapped_point_count"]
        );
        assert_eq!(
            pass_evidence["comparison"]["added_face_count"],
            expected["passing_added_face_count"]
        );
        assert_eq!(
            pass_evidence["comparison"]["removed_face_count"],
            expected["passing_removed_face_count"]
        );
        assert_eq!(fail_evidence["result"], expected["failed_result"]);
        assert_eq!(
            fail_evidence["checks"]["unique_mapping"]["reason"],
            expected["failed_reason"]
        );
        assert_eq!(
            fail_evidence["comparison"]["unmatched_point_count"],
            expected["failed_unmatched_point_count"]
        );
        assert_eq!(
            manifest["expected"]["source_identity"],
            hex_bytes(&journal.intent.source)
        );
        assert_eq!(
            manifest["expected"]["workspace_identity"],
            hex_bytes(&journal.intent.workspace)
        );
        assert_eq!(
            manifest["expected"]["baseline_revision"],
            hex_bytes(&journal.intent.baseline_revision)
        );
        assert_eq!(
            manifest["expected"]["operation_identity"],
            hex_bytes(&journal.intent.operation)
        );
        assert_eq!(
            manifest["expected"]["revision"],
            hex_bytes(&report.revision)
        );
        assert_eq!(
            corpus_snapshot(&root),
            before,
            "corpus consumers are read-only"
        );
    }

    #[test]
    fn hard_token_bound_precedes_xml_parser_buffer_growth() {
        let directory = TestDirectory::new("token-bound");
        let path = directory.path.join("token.xml");
        fs::write(&path, b"<abcdef>").unwrap();
        let file = File::open(&path).unwrap();
        let control = OperationControl::new();
        let mut hasher = Hasher::new();
        let mut reader = HashingRead::new(file, &mut hasher, 8, &control, 4);
        let mut buffer = [0; 16];
        let error = reader.read(&mut buffer).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    }

    #[test]
    fn lexical_token_bound_covers_comment_cdata_pi_and_doctype_terminators() {
        let repeated_angles = "<>".repeat(1_024);
        for (label, token) in [
            ("comment", format!("<!--{repeated_angles}-->")),
            ("cdata", format!("<![CDATA[{repeated_angles}]]>")),
            ("pi", format!("<?probe {repeated_angles}?>")),
            (
                "doctype",
                format!("<!DOCTYPE LandXML [{}]>", "<!ELEMENT A ANY>".repeat(256)),
            ),
        ] {
            assert_oversized_lexical_token_is_bounded(label, token.as_bytes(), 512);
        }
    }

    #[test]
    fn hashing_reader_never_consumes_growth_past_the_witnessed_length() {
        let directory = TestDirectory::new("fixed-read-length");
        let path = directory.path.join("input.xml");
        fs::write(&path, b"abc").unwrap();
        let file = File::open(&path).unwrap();
        let control = OperationControl::new();
        let mut hasher = Hasher::new();
        let mut reader = HashingRead::new(file, &mut hasher, 3, &control, MAX_EVENT_BYTES);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"def")
            .unwrap();

        let mut captured = Vec::new();
        reader.read_to_end(&mut captured).unwrap();
        assert_eq!(captured, b"abc");
        assert_eq!(hasher.finalize(), blake3::hash(b"abc"));
    }

    #[test]
    fn input_witness_rehash_detects_same_length_content_with_restored_timestamp() {
        let directory = TestDirectory::new("same-length-rehash");
        let path = directory.path.join("input.xml");
        fs::write(&path, b"original").unwrap();
        let file = File::open(&path).unwrap();
        let identity = file.metadata().unwrap();
        let witness = InputWitness::new(
            &path,
            &file,
            identity.clone(),
            *blake3::hash(b"original").as_bytes(),
            b"",
        )
        .unwrap();
        let mut replacement = OpenOptions::new().write(true).open(&path).unwrap();
        replacement.write_all(b"replaced").unwrap();
        replacement.sync_all().unwrap();
        replacement
            .set_times(FileTimes::new().set_modified(identity.modified().unwrap()))
            .unwrap();
        assert!(same_file_state(&identity, &fs::metadata(&path).unwrap()));

        assert!(matches!(
            witness.verify(),
            Err(QualificationError::InputChanged(changed)) if changed == path
        ));
    }

    #[test]
    fn report_capture_rejects_growth_past_the_simultaneous_descriptor_snapshot() {
        let directory = TestDirectory::new("report-growth");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/qualification-v1/audit.json");
        let path = directory.path.join("audit.json");
        fs::copy(source, &path).unwrap();
        let opened = open_regular(&path, "audit.json", EVIDENCE_BYTES).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b" ")
            .unwrap();
        assert!(matches!(
            read_open_bound_report(&path, EVIDENCE_BYTES, &OperationControl::new(), opened),
            Err(QualificationError::InputChanged(changed)) if changed == path
        ));
    }

    #[cfg(unix)]
    #[test]
    fn qualification_input_open_rejects_symbolic_links_without_following() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("nofollow");
        let target = directory.path.join("target.xml");
        let link = directory.path.join("link.xml");
        fs::write(&target, b"regular").unwrap();
        symlink(&target, &link).unwrap();
        assert!(open_regular(&link, "linked input", 1024).is_err());
    }

    #[test]
    fn unit_element_counts_reject_empty_imperial_and_duplicate_metric_semantically() {
        let directory = TestDirectory::new("unit-elements");
        for (name, units_markup) in [
            (
                "imperial",
                "<Units><Metric linearUnit=\"meter\"/><Imperial/></Units>",
            ),
            (
                "duplicate-metric",
                "<Units><Metric linearUnit=\"meter\"/><Metric/></Units>",
            ),
            ("missing", ""),
            (
                "duplicate-units",
                "<Units><Metric linearUnit=\"meter\"/></Units><Units><Metric linearUnit=\"meter\"/></Units>",
            ),
            (
                "non-meter",
                "<Units><Metric linearUnit=\"millimeter\"/></Units>",
            ),
        ] {
            let path = directory.path.join(format!("{name}.xml"));
            fs::write(&path, landxml_document(units_markup, "", "<F>1 2 3</F>")).unwrap();
            let captured = read_landxml(
                &path,
                StreamSide::Returned,
                RoundTripLimits::qualification(),
                &OperationControl::new(),
            )
            .unwrap();
            assert!(!captured.metric_metres, "{name} must be unit drift");
            let outcome = evaluate_semantics(
                &captured.surface,
                &captured.surface,
                captured.metric_metres,
                RoundTripTolerances::new(0.0, 0.0).unwrap(),
                RoundTripLimits::qualification(),
            )
            .unwrap();
            assert_eq!(outcome.reason, "PRT_UNIT_DRIFT");
        }
    }

    #[test]
    fn nested_presentation_metadata_is_bounded_but_semantically_ignored() {
        let directory = TestDirectory::new("metadata-rewrite");
        let plain_path = directory.path.join("plain.xml");
        let rewritten_path = directory.path.join("rewritten.xml");
        fs::write(&plain_path, landxml("<Metric linearUnit=\"meter\"/>", "")).unwrap();
        fs::write(
            &rewritten_path,
            landxml(
                "<Metric linearUnit=\"meter\"/>",
                "<Project name=\"rewritten\"><Display xmlns=\"urn:presentation\"><Label>display &amp; only &#65;</Label></Display></Project><Application><Vendor xmlns=\"urn:vendor\">metadata</Vendor></Application>",
            ),
        )
        .unwrap();
        let control = OperationControl::new();
        let plain = read_landxml(
            &plain_path,
            StreamSide::Original,
            RoundTripLimits::qualification(),
            &control,
        )
        .unwrap();
        let rewritten = read_landxml(
            &rewritten_path,
            StreamSide::Returned,
            RoundTripLimits::qualification(),
            &control,
        )
        .unwrap();
        assert_eq!(
            rewritten
                .ignored_top_level_sections
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<&str>>(),
            ["Project", "Application"]
        );
        assert!(
            evaluate_semantics(
                &plain.surface,
                &rewritten.surface,
                rewritten.metric_metres,
                RoundTripTolerances::new(0.0, 0.0).unwrap(),
                RoundTripLimits::qualification(),
            )
            .unwrap()
            .passed
        );
    }

    #[test]
    fn comments_containing_declaration_tokens_are_presentation_only() {
        let directory = TestDirectory::new("declaration-comment");
        let path = directory.path.join("comment.xml");
        let document = landxml("<Metric linearUnit=\"meter\"/>", "").replace(
            "<Units>",
            "<!-- generated without <!DOCTYPE or <!ENTITY declarations -->\n<Units>",
        );
        fs::write(&path, document).unwrap();
        assert!(
            read_landxml(
                &path,
                StreamSide::Returned,
                RoundTripLimits::qualification(),
                &OperationControl::new(),
            )
            .is_ok()
        );
    }

    #[test]
    fn ignored_metadata_still_rejects_xinclude() {
        let directory = TestDirectory::new("metadata-xinclude");
        let path = directory.path.join("xinclude.xml");
        fs::write(
            &path,
            landxml(
                "<Metric linearUnit=\"meter\"/>",
                "<Project><xi:include xmlns:xi=\"http://www.w3.org/2001/XInclude\" href=\"elsewhere.xml\"/></Project>",
            ),
        )
        .unwrap();
        assert!(matches!(
            read_landxml(
                &path,
                StreamSide::Returned,
                RoundTripLimits::qualification(),
                &OperationControl::new(),
            ),
            Err(QualificationError::Subset(message)) if message.contains("XInclude")
        ));
    }

    #[test]
    fn ignored_metadata_rejects_undeclared_general_references() {
        let directory = TestDirectory::new("metadata-entity");
        let path = directory.path.join("entity.xml");
        fs::write(
            &path,
            landxml(
                "<Metric linearUnit=\"meter\"/>",
                "<Project><Label>unsafe &declaredElsewhere;</Label></Project>",
            ),
        )
        .unwrap();
        assert!(matches!(
            read_landxml(
                &path,
                StreamSide::Returned,
                RoundTripLimits::qualification(),
                &OperationControl::new(),
            ),
            Err(QualificationError::Xml(message)) if message.contains("unsupported entity reference")
        ));
    }

    #[test]
    fn prefixed_semantic_attribute_names_never_satisfy_required_attributes() {
        let directory = TestDirectory::new("prefixed-attributes");
        let metric_path = directory.path.join("metric.xml");
        fs::write(
            &metric_path,
            landxml("<Metric xmlns:x=\"urn:spoof\" x:linearUnit=\"meter\"/>", ""),
        )
        .unwrap();
        let metric = read_landxml(
            &metric_path,
            StreamSide::Returned,
            RoundTripLimits::qualification(),
            &OperationControl::new(),
        )
        .unwrap();
        assert!(!metric.metric_metres);

        for (name, document) in [
            (
                "point-id",
                landxml("<Metric linearUnit=\"meter\"/>", "")
                    .replace("id=\"1\"", "xmlns:x=\"urn:spoof\" x:id=\"1\""),
            ),
            (
                "surface-kind",
                landxml("<Metric linearUnit=\"meter\"/>", "").replace(
                    "surfType=\"TIN\"",
                    "xmlns:x=\"urn:spoof\" x:surfType=\"TIN\"",
                ),
            ),
            (
                "version",
                landxml("<Metric linearUnit=\"meter\"/>", "")
                    .replace("version=\"1.2\"", "xmlns:x=\"urn:spoof\" x:version=\"1.2\""),
            ),
        ] {
            let path = directory.path.join(format!("{name}.xml"));
            fs::write(&path, document).unwrap();
            assert!(
                read_landxml(
                    &path,
                    StreamSide::Returned,
                    RoundTripLimits::qualification(),
                    &OperationControl::new(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn xml_attribute_budget_counts_names_and_complete_source_syntax() {
        let directory = TestDirectory::new("attribute-source-budget");
        let base_path = directory.path.join("base.xml");
        let oversized_path = directory.path.join("oversized.xml");
        let base = landxml("<Metric linearUnit=\"meter\"/>", "<Project harmless=\"\"/>");
        let long_name = "a".repeat(1_024);
        let oversized = base.replace("harmless", &long_name);
        fs::write(&base_path, base).unwrap();
        fs::write(&oversized_path, oversized).unwrap();
        let limits = RoundTripLimits {
            xml_text_bytes: 512,
            ..RoundTripLimits::qualification()
        };
        let control = OperationControl::new();
        read_landxml(&base_path, StreamSide::Original, limits, &control).unwrap();
        assert!(matches!(
            read_landxml(&oversized_path, StreamSide::Returned, limits, &control),
            Err(QualificationError::Resource { limit, allowed: 512, .. })
                if limit == "LandXML XML text and attribute bytes"
        ));
    }

    #[test]
    fn duplicate_faces_remain_semantic_topology_drift() {
        let directory = TestDirectory::new("duplicate-face");
        let original_path = directory.path.join("original.xml");
        let returned_path = directory.path.join("returned.xml");
        fs::write(
            &original_path,
            landxml_document(
                "<Units><Metric linearUnit=\"meter\"/></Units>",
                "",
                "<F>1 2 3</F>",
            ),
        )
        .unwrap();
        fs::write(
            &returned_path,
            landxml_document(
                "<Units><Metric linearUnit=\"meter\"/></Units>",
                "",
                "<F>1 2 3</F><F>1 2 3</F>",
            ),
        )
        .unwrap();
        let control = OperationControl::new();
        let original = read_landxml(
            &original_path,
            StreamSide::Original,
            RoundTripLimits::qualification(),
            &control,
        )
        .unwrap();
        let returned = read_landxml(
            &returned_path,
            StreamSide::Returned,
            RoundTripLimits::qualification(),
            &control,
        )
        .unwrap();
        let outcome = evaluate_semantics(
            &original.surface,
            &returned.surface,
            returned.metric_metres,
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::qualification(),
        )
        .unwrap();
        assert_eq!(outcome.reason, "PRT_TOPOLOGY_DRIFT");
        assert_eq!((outcome.added_faces, outcome.removed_faces), (1, 0));
    }

    #[test]
    fn check_dependencies_are_canonical_for_every_semantic_reason_family() {
        let failed_check = |reason| CheckFact {
            status: "failed",
            reason,
        };
        for (metric, reason, expected) in [
            (
                false,
                "PRT_UNIT_DRIFT",
                EvidenceChecks {
                    units: failed_check("PRT_UNIT_DRIFT"),
                    unique_mapping: NOT_EVALUATED_CHECK,
                    tolerance: NOT_EVALUATED_CHECK,
                    topology: NOT_EVALUATED_CHECK,
                },
            ),
            (
                true,
                "PRT_POINT_COUNT_DRIFT",
                EvidenceChecks {
                    units: PASSED_CHECK,
                    unique_mapping: failed_check("PRT_POINT_COUNT_DRIFT"),
                    tolerance: NOT_EVALUATED_CHECK,
                    topology: NOT_EVALUATED_CHECK,
                },
            ),
            (
                true,
                "PRT_VERTEX_UNMATCHED",
                EvidenceChecks {
                    units: PASSED_CHECK,
                    unique_mapping: failed_check("PRT_VERTEX_UNMATCHED"),
                    tolerance: NOT_EVALUATED_CHECK,
                    topology: NOT_EVALUATED_CHECK,
                },
            ),
            (
                true,
                "PRT_TOLERANCE_DRIFT",
                EvidenceChecks {
                    units: PASSED_CHECK,
                    unique_mapping: failed_check("PRT_TOLERANCE_DRIFT"),
                    tolerance: failed_check("PRT_TOLERANCE_DRIFT"),
                    topology: NOT_EVALUATED_CHECK,
                },
            ),
            (
                true,
                "PRT_VERTEX_AMBIGUOUS",
                EvidenceChecks {
                    units: PASSED_CHECK,
                    unique_mapping: failed_check("PRT_VERTEX_AMBIGUOUS"),
                    tolerance: NOT_EVALUATED_CHECK,
                    topology: NOT_EVALUATED_CHECK,
                },
            ),
            (
                true,
                "PRT_TOPOLOGY_DRIFT",
                EvidenceChecks {
                    units: PASSED_CHECK,
                    unique_mapping: PASSED_CHECK,
                    tolerance: PASSED_CHECK,
                    topology: failed_check("PRT_TOPOLOGY_DRIFT"),
                },
            ),
        ] {
            assert_eq!(evidence_checks(metric, failed(reason, 0, 0)), expected);
        }
        let mut passed = failed("none", 0, 0);
        passed.passed = true;
        assert_eq!(
            evidence_checks(true, passed),
            EvidenceChecks {
                units: PASSED_CHECK,
                unique_mapping: PASSED_CHECK,
                tolerance: PASSED_CHECK,
                topology: PASSED_CHECK,
            }
        );
    }

    #[test]
    fn tiny_retained_memory_ceiling_fails_before_allocation() {
        let limits = RoundTripLimits {
            retained_model_bytes: 1,
            ..RoundTripLimits::qualification()
        };
        let Err(error) = StreamState::new(limits) else {
            panic!("one byte cannot retain the model");
        };
        assert!(
            matches!(error, QualificationError::Resource { limit, .. } if limit == "LandXML retained model bytes")
        );
    }

    #[test]
    fn retained_model_peak_accounting_has_an_exact_small_limit_boundary() {
        let base = RoundTripLimits {
            points: 3,
            faces: 1,
            ..RoundTripLimits::qualification()
        };
        let required = StreamState::required_retained_model_bytes(base);
        assert!(required > MAX_EVENT_BYTES);
        StreamState::new(RoundTripLimits {
            retained_model_bytes: required,
            ..base
        })
        .unwrap();
        assert!(matches!(
            StreamState::new(RoundTripLimits {
                retained_model_bytes: required - 1,
                ..base
            }),
            Err(QualificationError::Resource {
                limit,
                required: observed,
                allowed,
            }) if limit == "LandXML retained model bytes"
                && observed == required
                && allowed == required - 1
        ));
    }

    #[test]
    fn semantic_reasons_distinguish_unmatched_tolerance_and_topology() {
        let reference = surface(
            [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            vec![[0, 1, 2]],
        );
        let shifted = surface(
            [[1.0, 0.0, 0.0], [3.0, 0.0, 0.0], [1.0, 2.0, 0.0]],
            vec![[0, 1, 2]],
        );
        let exact = evaluate_semantics(
            &reference,
            &shifted,
            true,
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::qualification(),
        )
        .unwrap();
        assert_eq!(exact.reason, "PRT_VERTEX_UNMATCHED");
        let tolerance = evaluate_semantics(
            &reference,
            &shifted,
            true,
            RoundTripTolerances::new(0.5, 0.0).unwrap(),
            RoundTripLimits::qualification(),
        )
        .unwrap();
        assert_eq!(tolerance.reason, "PRT_TOLERANCE_DRIFT");

        let square = surface(
            [
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        let diagonal = surface(
            [
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
            ],
            vec![[0, 1, 3], [1, 2, 3]],
        );
        let topology = evaluate_semantics(
            &square,
            &diagonal,
            true,
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::qualification(),
        )
        .unwrap();
        assert_eq!(topology.reason, "PRT_TOPOLOGY_DRIFT");
        assert_eq!((topology.added_faces, topology.removed_faces), (2, 2));
    }

    fn surface<const N: usize>(points: [[f64; 3]; N], faces: Vec<[usize; 3]>) -> StreamSurface {
        StreamSurface {
            points: points.map(|position| Point { position }).into(),
            faces,
        }
    }

    fn landxml(units: &str, metadata: &str) -> String {
        landxml_document(&format!("<Units>{units}</Units>"), metadata, "<F>1 2 3</F>")
    }

    fn landxml_document(units: &str, metadata: &str, faces: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" version=\"1.2\">{units}{metadata}<Surfaces><Surface name=\"S\"><Definition surfType=\"TIN\"><Pnts><P id=\"1\">0 0 0</P><P id=\"2\">0 1 0</P><P id=\"3\">1 0 0</P></Pnts><Faces>{faces}</Faces></Definition></Surface></Surfaces></LandXML>"
        )
    }

    fn hex_bytes(bytes: &[u8]) -> String {
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut output, "{byte:02x}").unwrap();
        }
        output
    }

    fn corpus_snapshot(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    fn assert_oversized_lexical_token_is_bounded(label: &str, token: &[u8], limit: u64) {
        assert!(token.len() as u64 > limit);
        let directory = TestDirectory::new(label);
        let path = directory.path.join("oversized-token.xml");
        fs::write(&path, token).unwrap();
        let file = File::open(&path).unwrap();
        let control = OperationControl::new();
        let mut hasher = Hasher::new();
        let hashing = HashingRead::new(file, &mut hasher, token.len() as u64, &control, limit);
        let mut reader = NsReader::from_reader(BufReader::with_capacity(64, hashing));
        reader.config_mut().check_comments = true;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(usize::try_from(limit).unwrap())
            .unwrap();
        loop {
            buffer.clear();
            match reader.read_resolved_event_into(&mut buffer) {
                Err(quick_xml::Error::Io(error)) if error.kind() == io::ErrorKind::FileTooLarge => {
                    assert!(buffer.len() as u64 <= limit);
                    assert!(buffer.capacity() as u64 <= limit);
                    break;
                }
                Err(error) => panic!("{label} failed before the hard token bound: {error}"),
                Ok((_, Event::Eof)) => panic!("{label} escaped the hard token bound"),
                Ok(_) => assert!(buffer.len() as u64 <= limit),
            }
        }
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "punctra-qualification-{label}-{}-{:x}",
                std::process::id(),
                u64::from_le_bytes(random)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
