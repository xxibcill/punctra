//! Streaming `LandXML` subset reader for the full v0.7 export-byte ceiling.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufReader},
    path::Path,
};

use foundation_runtime::OperationControl;
use quick_xml::{
    XmlVersion,
    events::{BytesStart, Event},
    name::ResolveResult,
    reader::NsReader,
};

use crate::{
    roundtrip::{
        InputSide, ParsedRoundTrip, ParsedSurface, Position, RoundTripDeclaration,
        RoundTripEvaluation, RoundTripFailure, RoundTripFileFacts, RoundTripLimits,
        RoundTripReason, RoundTripTolerances, Triangle, evaluate_parsed_round_trip,
        reject_duplicate_faces, semantic_evaluation_failure, validate_face,
        validate_utf8_declaration,
    },
    stable_file::StableFile,
};

const LANDXML_NAMESPACE: &[u8] = b"http://www.landxml.org/schema/LandXML-1.2";
const XINCLUDE_NAMESPACE: &[u8] = b"http://www.w3.org/2001/XInclude";
const STREAM_BUFFER_BYTES: usize = 64 * 1024;

#[cfg(test)]
pub(crate) fn evaluate_streaming_round_trip(
    reference_path: &Path,
    returned_path: &Path,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
) -> Result<RoundTripEvaluation, RoundTripFailure> {
    evaluate_streaming_round_trip_with_control(
        reference_path,
        returned_path,
        declaration,
        tolerances,
        limits,
        &OperationControl::new(),
    )
    .map(|evaluation| evaluation.evaluation)
}

#[derive(Debug)]
pub(crate) struct StreamingRoundTripEvaluation {
    pub(crate) evaluation: RoundTripEvaluation,
    reference: StableFile,
    returned: StableFile,
}

impl StreamingRoundTripEvaluation {
    pub(crate) fn verify_inputs(&self) -> Result<(), RoundTripFailure> {
        self.reference.verify().map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} changed after streaming comparison: {error}",
                InputSide::Reference
            ))
        })?;
        self.returned.verify().map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} changed after streaming comparison: {error}",
                InputSide::Returned
            ))
        })
    }
}

pub(crate) fn evaluate_streaming_round_trip_with_control(
    reference_path: &Path,
    returned_path: &Path,
    declaration: RoundTripDeclaration,
    tolerances: RoundTripTolerances,
    limits: RoundTripLimits,
    control: &OperationControl,
) -> Result<StreamingRoundTripEvaluation, RoundTripFailure> {
    check_cancelled(control)?;
    let (reference, returned) = capture_streaming_pair(reference_path, returned_path, limits)?;
    let reference = parse_streaming_file(InputSide::Reference, reference, limits, control)?;
    let returned = parse_streaming_file(InputSide::Returned, returned, limits, control)?;
    let exact_bytes = reference.facts == returned.facts;
    let evaluation = match (reference.surface, returned.surface) {
        (Ok(reference_surface), Ok(returned_surface)) => evaluate_parsed_round_trip(
            ParsedRoundTrip {
                declaration,
                tolerances,
                limits,
                reference_facts: reference.facts,
                returned_facts: returned.facts,
                exact_bytes,
                reference_surface,
                returned_surface,
            },
            Some(control),
        ),
        (Err(error), Ok(returned_surface)) => semantic_evaluation_failure(
            declaration,
            tolerances,
            reference.facts,
            returned.facts,
            error,
            Some(returned_surface),
        ),
        (Ok(_), Err(error)) | (Err(error), Err(_)) => semantic_evaluation_failure(
            declaration,
            tolerances,
            reference.facts,
            returned.facts,
            error,
            None,
        ),
    }?;
    Ok(StreamingRoundTripEvaluation {
        evaluation,
        reference: reference.witness,
        returned: returned.witness,
    })
}

struct StreamingParse {
    facts: RoundTripFileFacts,
    surface: Result<ParsedSurface, RoundTripFailure>,
    witness: StableFile,
}

fn capture_streaming_pair(
    reference_path: &Path,
    returned_path: &Path,
    limits: RoundTripLimits,
) -> Result<(StableFile, StableFile), RoundTripFailure> {
    let reference = capture_streaming_file(InputSide::Reference, reference_path, limits)?;
    let returned = capture_streaming_file(InputSide::Returned, returned_path, limits)?;
    Ok((reference, returned))
}

fn capture_streaming_file(
    side: InputSide,
    path: &Path,
    limits: RoundTripLimits,
) -> Result<StableFile, RoundTripFailure> {
    let witness = StableFile::capture(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be captured: {error}"))
    })?;
    require_file_bytes(side, witness.byte_length(), limits.file_bytes())?;
    Ok(witness)
}

fn parse_streaming_file(
    side: InputSide,
    mut witness: StableFile,
    limits: RoundTripLimits,
    control: &OperationControl,
) -> Result<StreamingParse, RoundTripFailure> {
    check_cancelled(control)?;
    let expected_bytes = witness.byte_length();
    let hashing = HashingReader::new(witness.file_mut(), limits.file_bytes());
    let mut reader = NsReader::from_reader(BufReader::with_capacity(STREAM_BUFFER_BYTES, hashing));
    reader.config_mut().expand_empty_elements = true;
    reader.config_mut().check_end_names = true;
    let surface = StreamParser::new(side, limits, control).parse(&mut reader);
    if let Err(error) = &surface
        && error.reason().is_none()
    {
        return Err(error.clone());
    }
    let hashing = reader.into_inner().into_inner();
    let (facts, utf8_valid) = hashing.finish(side)?;
    let surface = if utf8_valid {
        surface
    } else {
        Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} is not UTF-8 XML"),
        ))
    };
    witness.verify().map_err(|error| {
        RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read: {error}"
        ))
    })?;
    if facts.byte_length != expected_bytes {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read"
        )));
    }
    Ok(StreamingParse {
        facts,
        surface,
        witness,
    })
}

fn drain_after_terminal_xml_error<R: io::BufRead>(
    side: InputSide,
    reader: &mut NsReader<R>,
    control: &OperationControl,
) -> Result<(), RoundTripFailure> {
    let mut buffer = vec![0; STREAM_BUFFER_BYTES].into_boxed_slice();
    loop {
        check_cancelled(control)?;
        let read = reader.get_mut().read(&mut buffer).map_err(|error| {
            if error.kind() == io::ErrorKind::FileTooLarge {
                RoundTripFailure::resource(format_args!("{side} file exceeded its byte limit"))
            } else {
                RoundTripFailure::invalid(format_args!(
                    "{side} cannot be drained after terminal XML failure: {error}"
                ))
            }
        })?;
        if read == 0 {
            return Ok(());
        }
    }
}

struct HashingReader<'a> {
    file: &'a mut File,
    hasher: blake3::Hasher,
    bytes: u64,
    max_bytes: u64,
    over_limit: bool,
    utf8: Utf8Validator,
}

impl<'a> HashingReader<'a> {
    fn new(file: &'a mut File, max_bytes: u64) -> Self {
        Self {
            file,
            hasher: blake3::Hasher::new(),
            bytes: 0,
            max_bytes,
            over_limit: false,
            utf8: Utf8Validator::default(),
        }
    }

    fn finish(self, side: InputSide) -> Result<(RoundTripFileFacts, bool), RoundTripFailure> {
        if self.over_limit {
            return Err(RoundTripFailure::resource(format_args!(
                "{side} file bytes exceed the {} byte limit",
                self.max_bytes
            )));
        }
        Ok((
            RoundTripFileFacts {
                content_hash: *self.hasher.finalize().as_bytes(),
                byte_length: self.bytes,
            },
            self.utf8.is_valid_at_end(),
        ))
    }
}

impl io::Read for HashingReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buffer)?;
        self.bytes = self.bytes.saturating_add(read as u64);
        if self.bytes > self.max_bytes {
            self.over_limit = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "LandXML input exceeded its byte limit",
            ));
        }
        self.hasher.update(&buffer[..read]);
        self.utf8.update(&buffer[..read]);
        Ok(read)
    }
}

#[derive(Default)]
struct Utf8Validator {
    pending: [u8; 4],
    pending_len: usize,
    invalid: bool,
}

impl Utf8Validator {
    fn update(&mut self, mut bytes: &[u8]) {
        if self.invalid {
            return;
        }
        if self.pending_len != 0 {
            while self.pending_len < self.pending.len() && !bytes.is_empty() {
                self.pending[self.pending_len] = bytes[0];
                self.pending_len += 1;
                bytes = &bytes[1..];
                match std::str::from_utf8(&self.pending[..self.pending_len]) {
                    Ok(_) => {
                        self.pending_len = 0;
                        break;
                    }
                    Err(error) if error.error_len().is_some() => {
                        self.invalid = true;
                        return;
                    }
                    Err(_) => {}
                }
            }
            if self.pending_len != 0 {
                return;
            }
        }
        if let Err(error) = std::str::from_utf8(bytes) {
            if error.error_len().is_some() {
                self.invalid = true;
                return;
            }
            let trailing = &bytes[error.valid_up_to()..];
            self.pending[..trailing.len()].copy_from_slice(trailing);
            self.pending_len = trailing.len();
        }
    }

    const fn is_valid_at_end(&self) -> bool {
        !self.invalid && self.pending_len == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tag {
    LandXml,
    Units,
    Metric,
    Project,
    Application,
    Surfaces,
    Surface,
    Definition,
    Points,
    Point,
    Faces,
    Face,
}

struct StreamParser<'a> {
    side: InputSide,
    limits: RoundTripLimits,
    stack: Vec<Tag>,
    nodes: u64,
    text_attribute_bytes: u64,
    root_count: u64,
    project_count: u64,
    application_count: u64,
    units_count: u64,
    metric_count: u64,
    surfaces_count: u64,
    surface_count: u64,
    definition_count: u64,
    points_count: u64,
    faces_count: u64,
    metadata_depth: usize,
    ignored_sections: Vec<Box<str>>,
    surface_name: Option<Box<str>>,
    points: Vec<Position>,
    point_ids: BTreeMap<u64, usize>,
    faces: Vec<Triangle>,
    simple_text: String,
    control: &'a OperationControl,
}

impl<'a> StreamParser<'a> {
    fn new(side: InputSide, limits: RoundTripLimits, control: &'a OperationControl) -> Self {
        Self {
            side,
            limits,
            stack: Vec::new(),
            nodes: 0,
            text_attribute_bytes: 0,
            root_count: 0,
            project_count: 0,
            application_count: 0,
            units_count: 0,
            metric_count: 0,
            surfaces_count: 0,
            surface_count: 0,
            definition_count: 0,
            points_count: 0,
            faces_count: 0,
            metadata_depth: 0,
            ignored_sections: Vec::new(),
            surface_name: None,
            points: Vec::new(),
            point_ids: BTreeMap::new(),
            faces: Vec::new(),
            simple_text: String::new(),
            control,
        }
    }

    fn parse<R: io::BufRead>(
        mut self,
        reader: &mut NsReader<R>,
    ) -> Result<ParsedSurface, RoundTripFailure> {
        let mut buffer = Vec::with_capacity(STREAM_BUFFER_BYTES);
        let mut semantic_failure = None;
        let mut xml_depth = 0_usize;
        loop {
            let (namespace, event) = match reader.read_resolved_event_into(&mut buffer) {
                Ok(event) => event,
                Err(error) => {
                    let failure =
                        self.xml_invalid_message(format_args!("XML is malformed: {error}"));
                    drain_after_terminal_xml_error(self.side, reader, self.control)?;
                    return Err(failure);
                }
            };
            if !matches!(event, Event::End(_) | Event::Eof) {
                self.count_node()?;
            }
            match event {
                Event::Start(start) => {
                    xml_depth = xml_depth.saturating_add(1);
                    let result = if semantic_failure.is_none() {
                        self.start(&namespace, &start)
                    } else {
                        self.validate_start_after_semantic_failure(&namespace, &start)
                    };
                    retain_semantic_failure(&mut semantic_failure, result)?;
                }
                Event::End(_) => {
                    if xml_depth == 0 {
                        retain_semantic_failure(
                            &mut semantic_failure,
                            Err(self.xml_invalid("unexpected closing element")),
                        )?;
                    } else {
                        xml_depth -= 1;
                    }
                    if semantic_failure.is_none() {
                        retain_semantic_failure(&mut semantic_failure, self.end())?;
                    }
                }
                Event::Text(text) => {
                    let result = self.text(&text, semantic_failure.is_none());
                    retain_semantic_failure(&mut semantic_failure, result)?;
                }
                Event::Comment(comment) => self.add_text_bytes(comment.as_ref().len())?,
                Event::Decl(declaration) => retain_semantic_failure(
                    &mut semantic_failure,
                    validate_utf8_declaration(self.side, &declaration),
                )?,
                Event::PI(_) => {}
                Event::DocType(_) | Event::GeneralRef(_) | Event::CData(_) => {
                    retain_semantic_failure(
                        &mut semantic_failure,
                        Err(self.xml_invalid("DTD, entity, or CDATA input is unsupported")),
                    )?;
                }
                Event::Empty(_) => unreachable!("empty elements are expanded"),
                Event::Eof => {
                    if xml_depth != 0 {
                        retain_semantic_failure(
                            &mut semantic_failure,
                            Err(self.xml_invalid("document has unclosed elements")),
                        )?;
                    }
                    break;
                }
            }
            if buffer.capacity() as u64 > self.limits.xml_text_bytes() {
                return Err(RoundTripFailure::resource(format_args!(
                    "{} XML token storage exceeds the {} byte limit",
                    self.side,
                    self.limits.xml_text_bytes()
                )));
            }
            buffer.clear();
        }
        match semantic_failure {
            Some(error) => Err(error),
            None => self.finish(),
        }
    }

    fn validate_start_after_semantic_failure(
        &mut self,
        namespace: &ResolveResult<'_>,
        start: &BytesStart<'_>,
    ) -> Result<(), RoundTripFailure> {
        self.add_text_bytes(start.attributes_raw().len())?;
        match namespace {
            ResolveResult::Unknown(_) => Err(self.xml_invalid("unknown XML prefix")),
            ResolveResult::Bound(namespace) if namespace.into_inner() == XINCLUDE_NAMESPACE => {
                Err(self.unsupported("XInclude is unsupported"))
            }
            ResolveResult::Bound(_) | ResolveResult::Unbound => Ok(()),
        }
    }

    fn text(
        &mut self,
        text: &quick_xml::events::BytesText<'_>,
        parse_semantics: bool,
    ) -> Result<(), RoundTripFailure> {
        self.add_text_bytes(text.as_ref().len())?;
        if !parse_semantics {
            return Ok(());
        }
        if matches!(self.stack.last(), Some(Tag::Point | Tag::Face)) {
            return text.decode().map_or_else(
                |error| {
                    Err(RoundTripFailure::semantic(
                        RoundTripReason::XmlInvalid,
                        format_args!("{} XML text is invalid: {error}", self.side),
                    ))
                },
                |decoded| {
                    self.simple_text.push_str(&decoded);
                    Ok(())
                },
            );
        }
        if self.metadata_depth == 0 && text.decode().is_ok_and(|value| !value.trim().is_empty()) {
            return Err(self.unsupported("semantic container has unexpected text"));
        }
        Ok(())
    }

    fn start(
        &mut self,
        namespace: &ResolveResult<'_>,
        start: &BytesStart<'_>,
    ) -> Result<(), RoundTripFailure> {
        self.add_text_bytes(start.attributes_raw().len())?;
        let namespace = match namespace {
            ResolveResult::Bound(namespace) => namespace.into_inner(),
            ResolveResult::Unbound => &[][..],
            ResolveResult::Unknown(_) => return Err(self.xml_invalid("unknown XML prefix")),
        };
        if namespace == XINCLUDE_NAMESPACE {
            return Err(self.unsupported("XInclude is unsupported"));
        }
        if self.metadata_depth != 0 {
            self.metadata_depth += 1;
            return Ok(());
        }
        if namespace != LANDXML_NAMESPACE {
            if matches!(self.stack.last(), Some(Tag::Units | Tag::Metric)) {
                return Err(self.unit_drift());
            }
            return Err(self.unsupported("foreign semantic element is unsupported"));
        }
        if start.local_name().as_ref() == b"CoordinateSystem" {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::CoordinateReferenceUnsupported,
                format_args!("{} CoordinateSystem semantics are unsupported", self.side),
            ));
        }
        let tag = tag(start.local_name().as_ref()).ok_or_else(|| {
            if matches!(self.stack.last(), Some(Tag::Units | Tag::Metric)) {
                self.unit_drift()
            } else {
                self.unsupported("unknown LandXML semantic element")
            }
        })?;
        self.require_parent(tag)?;
        self.validate_attributes(tag, start)?;
        if matches!(tag, Tag::Project | Tag::Application) {
            self.ignored_sections
                .push(tag_name(tag).to_owned().into_boxed_str());
            self.metadata_depth = 1;
            self.stack.push(tag);
            return Ok(());
        }
        if matches!(tag, Tag::Point | Tag::Face) {
            self.simple_text.clear();
        }
        self.stack.push(tag);
        Ok(())
    }

    fn end(&mut self) -> Result<(), RoundTripFailure> {
        if self.metadata_depth != 0 {
            self.metadata_depth -= 1;
            if self.metadata_depth == 0 {
                self.stack.pop();
            }
            return Ok(());
        }
        let tag = self
            .stack
            .pop()
            .ok_or_else(|| self.xml_invalid("unexpected closing element"))?;
        match tag {
            Tag::Point => self.finish_point(),
            Tag::Face => self.finish_face(),
            _ => Ok(()),
        }
    }

    fn require_parent(&mut self, tag: Tag) -> Result<(), RoundTripFailure> {
        let parent = self.stack.last().copied();
        let valid = matches!(
            (parent, tag),
            (None, Tag::LandXml)
                | (
                    Some(Tag::LandXml),
                    Tag::Units | Tag::Project | Tag::Application | Tag::Surfaces
                )
                | (Some(Tag::Units), Tag::Metric)
                | (Some(Tag::Surfaces), Tag::Surface)
                | (Some(Tag::Surface), Tag::Definition)
                | (Some(Tag::Definition), Tag::Points | Tag::Faces)
                | (Some(Tag::Points), Tag::Point)
                | (Some(Tag::Faces), Tag::Face)
        );
        if !valid {
            if matches!(parent, Some(Tag::Units | Tag::Metric))
                || matches!(tag, Tag::Units | Tag::Metric)
            {
                return Err(self.unit_drift());
            }
            return Err(self.unsupported("LandXML element is in an unsupported container"));
        }
        let counter = match tag {
            Tag::LandXml => &mut self.root_count,
            Tag::Units => &mut self.units_count,
            Tag::Metric => &mut self.metric_count,
            Tag::Surfaces => &mut self.surfaces_count,
            Tag::Surface => &mut self.surface_count,
            Tag::Definition => &mut self.definition_count,
            Tag::Points => &mut self.points_count,
            Tag::Faces => &mut self.faces_count,
            Tag::Project => &mut self.project_count,
            Tag::Application => &mut self.application_count,
            Tag::Point | Tag::Face => return Ok(()),
        };
        *counter = counter.saturating_add(1);
        if *counter > 1 {
            if matches!(tag, Tag::Units | Tag::Metric) {
                return Err(self.unit_drift());
            }
            return Err(self.unsupported("semantic element appears more than once"));
        }
        Ok(())
    }

    fn validate_attributes(
        &mut self,
        tag: Tag,
        start: &BytesStart<'_>,
    ) -> Result<(), RoundTripFailure> {
        let required = match tag {
            Tag::LandXml => Some((b"version".as_slice(), "1.2")),
            Tag::Metric => Some((b"linearUnit".as_slice(), "meter")),
            Tag::Surface => Some((b"name".as_slice(), "")),
            Tag::Definition => Some((b"surfType".as_slice(), "TIN")),
            Tag::Point => Some((b"id".as_slice(), "")),
            _ => None,
        };
        let mut found = None;
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|error| {
                self.xml_invalid_message(format_args!("XML attribute is invalid: {error}"))
            })?;
            if attribute.key.as_ref() == b"xmlns"
                || attribute.key.as_ref().starts_with(b"xmlns:")
                || attribute.key.as_ref().contains(&b':')
            {
                continue;
            }
            if required.is_some_and(|(name, _)| attribute.key.as_ref() == name) {
                let value = attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map_err(|error| {
                        self.xml_invalid_message(format_args!(
                            "XML attribute value is invalid: {error}"
                        ))
                    })?;
                if found.replace(value.into_owned()).is_some() {
                    return Err(self.xml_invalid("required attribute is duplicated"));
                }
            }
        }
        if let Some((_name, expected)) = required {
            let value = found.ok_or_else(|| {
                if tag == Tag::Metric {
                    self.unit_drift()
                } else {
                    self.unsupported("required attribute is absent")
                }
            })?;
            match tag {
                Tag::Metric if value != expected => {
                    return Err(self.unit_drift());
                }
                Tag::Surface => {
                    self.surface_name = (!value.is_empty()).then(|| value.into_boxed_str());
                }
                Tag::Point => {
                    let id = value
                        .parse::<u64>()
                        .map_err(|_| self.xml_invalid("Point id must be a positive integer"))?;
                    if id == 0 || self.point_ids.contains_key(&id) {
                        return Err(self.xml_invalid("Point id is zero or duplicated"));
                    }
                    self.point_ids.insert(id, self.points.len());
                }
                _ if value != expected => {
                    return Err(self.unsupported("required attribute value differs"));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn finish_point(&mut self) -> Result<(), RoundTripFailure> {
        if self.points.len() as u64 >= self.limits.points() {
            return Err(RoundTripFailure::resource(format_args!(
                "{} points exceed the {} point limit",
                self.side,
                self.limits.points()
            )));
        }
        let mut values = self.simple_text.split_whitespace();
        let northing = parse_number(self.side, values.next())?;
        let easting = parse_number(self.side, values.next())?;
        let elevation = parse_number(self.side, values.next())?;
        if values.next().is_some() {
            return Err(self.xml_invalid("Point must contain exactly three coordinates"));
        }
        self.points.push(Position {
            easting: canonical_zero(easting),
            northing: canonical_zero(northing),
            elevation: canonical_zero(elevation),
        });
        Ok(())
    }

    fn finish_face(&mut self) -> Result<(), RoundTripFailure> {
        if self.faces.len() as u64 >= self.limits.faces() {
            return Err(RoundTripFailure::resource(format_args!(
                "{} faces exceed the {} face limit",
                self.side,
                self.limits.faces()
            )));
        }
        let mut values = self.simple_text.split_whitespace();
        let a = parse_id(self.side, values.next())?;
        let b = parse_id(self.side, values.next())?;
        let c = parse_id(self.side, values.next())?;
        if values.next().is_some() {
            return Err(self.xml_invalid("Face must contain exactly three Point ids"));
        }
        let resolve = |id| {
            self.point_ids
                .get(&id)
                .copied()
                .ok_or_else(|| self.xml_invalid("Face has a dangling Point reference"))
        };
        let face = Triangle::new(resolve(a)?, resolve(b)?, resolve(c)?);
        validate_face(self.side, face, &self.points)?;
        self.faces.push(face);
        Ok(())
    }

    fn finish(mut self) -> Result<ParsedSurface, RoundTripFailure> {
        if self.units_count != 1 || self.metric_count != 1 {
            return Err(self.unit_drift());
        }
        if !self.stack.is_empty()
            || self.root_count != 1
            || self.surfaces_count != 1
            || self.surface_count != 1
            || self.definition_count != 1
            || self.points_count != 1
            || self.faces_count != 1
            || self.points.len() < 3
            || self.faces.is_empty()
        {
            return Err(self.unsupported("LandXML TIN subset is incomplete"));
        }
        reject_duplicate_faces(self.side, &mut self.faces)?;
        Ok(ParsedSurface {
            points: self.points,
            faces: self.faces,
            surface_name: self.surface_name,
            ignored_top_level_sections: self.ignored_sections.into_boxed_slice(),
        })
    }

    fn count_node(&mut self) -> Result<(), RoundTripFailure> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes.is_multiple_of(4096) {
            check_cancelled(self.control)?;
        }
        if self.nodes > self.limits.xml_nodes() {
            Err(RoundTripFailure::resource(format_args!(
                "{} XML nodes exceed the {} node limit",
                self.side,
                self.limits.xml_nodes()
            )))
        } else {
            Ok(())
        }
    }

    fn add_text_bytes(&mut self, additional: usize) -> Result<(), RoundTripFailure> {
        self.text_attribute_bytes = self.text_attribute_bytes.saturating_add(additional as u64);
        if self.text_attribute_bytes > self.limits.xml_text_bytes() {
            Err(RoundTripFailure::resource(format_args!(
                "{} XML text and attribute bytes exceed the {} byte limit",
                self.side,
                self.limits.xml_text_bytes()
            )))
        } else {
            Ok(())
        }
    }

    fn xml_invalid(&self, message: &'static str) -> RoundTripFailure {
        self.xml_invalid_message(message)
    }

    fn xml_invalid_message(&self, message: impl std::fmt::Display) -> RoundTripFailure {
        RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{} XML is invalid: {message}", self.side),
        )
    }

    fn unsupported(&self, message: &'static str) -> RoundTripFailure {
        RoundTripFailure::semantic(
            RoundTripReason::SubsetUnsupported,
            format_args!("{} subset is unsupported: {message}", self.side),
        )
    }

    fn unit_drift(&self) -> RoundTripFailure {
        RoundTripFailure::semantic(
            RoundTripReason::UnitDrift,
            format_args!(
                "{} units do not declare exactly one metric metre unit",
                self.side
            ),
        )
    }
}

fn retain_semantic_failure(
    retained: &mut Option<RoundTripFailure>,
    result: Result<(), RoundTripFailure>,
) -> Result<(), RoundTripFailure> {
    let Err(error) = result else {
        return Ok(());
    };
    let Some(reason) = error.reason() else {
        return Err(error);
    };
    let should_replace = retained.as_ref().is_none_or(|current| {
        reason == RoundTripReason::XmlInvalid
            && current.reason() != Some(RoundTripReason::XmlInvalid)
    });
    if should_replace {
        *retained = Some(error);
    }
    Ok(())
}

fn check_cancelled(control: &OperationControl) -> Result<(), RoundTripFailure> {
    control
        .check_cancelled()
        .map_err(|_| RoundTripFailure::cancelled())
}

fn tag(local: &[u8]) -> Option<Tag> {
    match local {
        b"LandXML" => Some(Tag::LandXml),
        b"Units" => Some(Tag::Units),
        b"Metric" => Some(Tag::Metric),
        b"Project" => Some(Tag::Project),
        b"Application" => Some(Tag::Application),
        b"Surfaces" => Some(Tag::Surfaces),
        b"Surface" => Some(Tag::Surface),
        b"Definition" => Some(Tag::Definition),
        b"Pnts" => Some(Tag::Points),
        b"P" => Some(Tag::Point),
        b"Faces" => Some(Tag::Faces),
        b"F" => Some(Tag::Face),
        _ => None,
    }
}

const fn tag_name(tag: Tag) -> &'static str {
    match tag {
        Tag::Project => "Project",
        Tag::Application => "Application",
        _ => "",
    }
}

fn parse_number(side: InputSide, value: Option<&str>) -> Result<f64, RoundTripFailure> {
    let value = value
        .ok_or_else(|| {
            RoundTripFailure::semantic(RoundTripReason::XmlInvalid, "coordinate absent")
        })?
        .parse::<f64>()
        .map_err(|_| {
            RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} coordinate is invalid"),
            )
        })?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} coordinate is non-finite"),
        ))
    }
}

fn parse_id(side: InputSide, value: Option<&str>) -> Result<u64, RoundTripFailure> {
    value
        .ok_or_else(|| RoundTripFailure::semantic(RoundTripReason::XmlInvalid, "Face id absent"))?
        .parse()
        .map_err(|_| {
            RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} Face id is invalid"),
            )
        })
}

fn require_file_bytes(side: InputSide, actual: u64, allowed: u64) -> Result<(), RoundTripFailure> {
    if actual <= allowed {
        Ok(())
    } else {
        Err(RoundTripFailure::resource(format_args!(
            "{side} file bytes required {actual}; limit is {allowed}"
        )))
    }
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use foundation_runtime::OperationControl;

    use crate::roundtrip::{
        RoundTripDeclaration, RoundTripEvaluation, RoundTripFailureKind, RoundTripLimits,
        RoundTripReason, RoundTripTolerances,
    };

    use super::{
        Utf8Validator, capture_streaming_pair, evaluate_streaming_round_trip,
        evaluate_streaming_round_trip_with_control,
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn utf8_validation_preserves_split_code_points_and_rejects_bad_tails() {
        let mut split = Utf8Validator::default();
        split.update(b"valid \xf0");
        split.update(b"\x9f\x92");
        split.update(b"\xa9 text");
        assert!(split.is_valid_at_end());

        let mut incomplete = Utf8Validator::default();
        incomplete.update(b"\xe2\x82");
        assert!(!incomplete.is_valid_at_end());

        let mut malformed = Utf8Validator::default();
        malformed.update(b"\xf0\x28\x8c\xbc");
        assert!(!malformed.is_valid_at_end());
    }

    #[test]
    fn streaming_reader_accepts_presentation_rewrites_and_full_export_limits() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        fs::write(&reference, xml("1", "2", "3", "1 2 3")).unwrap();
        fs::write(&returned, xml("30", "20", "10", "10 20 30")).unwrap();
        let limits = RoundTripLimits::full_v07_export();
        assert_eq!(limits.file_bytes(), 4 * 1024 * 1024 * 1024);
        assert_eq!(limits.points(), 10_000_000);
        assert_eq!(limits.faces(), 20_000_000);

        let evaluation = evaluate_streaming_round_trip(
            &reference,
            &returned,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            limits,
        )
        .expect("streamed semantic comparison succeeds");
        let RoundTripEvaluation::Passed(report) = evaluation else {
            panic!("presentation rewrite must pass");
        };
        assert_eq!(report.vertex_count(), 3);
        assert_eq!(report.face_count(), 1);
        assert!(!report.exact_bytes());
    }

    #[test]
    fn streaming_reader_honors_cancellation_before_evaluation() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        fs::write(&reference, xml("1", "2", "3", "1 2 3")).unwrap();
        fs::write(&returned, xml("1", "2", "3", "1 2 3")).unwrap();
        let control = OperationControl::new();
        control.cancel();

        let error = evaluate_streaming_round_trip_with_control(
            &reference,
            &returned,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::full_v07_export(),
            &control,
        )
        .expect_err("cancelled comparison cannot evaluate or publish evidence");

        assert_eq!(error.kind(), RoundTripFailureKind::Cancelled);
    }

    #[test]
    fn streaming_witness_rejects_post_evaluation_input_change() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let original = xml("1", "2", "3", "1 2 3");
        fs::write(&reference, &original).unwrap();
        fs::write(&returned, &original).unwrap();
        let control = OperationControl::new();
        let evaluation = evaluate_streaming_round_trip_with_control(
            &reference,
            &returned,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::full_v07_export(),
            &control,
        )
        .expect("initial capture is stable");

        let changed = original.replacen("0 0 0", "0 0 1", 1);
        assert_eq!(changed.len(), original.len());
        fs::write(&returned, changed).unwrap();

        evaluation
            .verify_inputs()
            .expect_err("same-length post-capture mutation must be rejected");
    }

    #[test]
    fn streaming_pair_witnesses_both_inputs_before_consumption() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let original = xml("1", "2", "3", "1 2 3");
        fs::write(&reference, &original).unwrap();
        fs::write(&returned, &original).unwrap();

        let (_reference_witness, returned_witness) =
            capture_streaming_pair(&reference, &returned, RoundTripLimits::full_v07_export())
                .expect("both inputs are witnessed together");
        let changed = original.replacen("0 0 0", "0 0 1", 1);
        assert_eq!(changed.len(), original.len());
        fs::write(&returned, changed).unwrap();

        returned_witness
            .verify()
            .expect_err("returned input mutation after pair capture must be rejected");
    }

    #[test]
    fn streaming_reader_rejects_non_utf8_bytes_and_declarations() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let valid = xml("1", "2", "3", "1 2 3");
        fs::write(&reference, &valid).unwrap();

        let declared_other_encoding = valid.replace("UTF-8", "UTF-16");
        fs::write(&returned, declared_other_encoding).unwrap();
        assert_failed_reason(&reference, &returned, RoundTripReason::XmlInvalid);

        let mut invalid_metadata = valid
            .replace("<Units>", "<Project>ok</Project><Units>")
            .into_bytes();
        let metadata = invalid_metadata
            .windows(2)
            .position(|bytes| bytes == b"ok")
            .expect("inserted metadata text exists");
        invalid_metadata[metadata] = 0xff;
        fs::write(&returned, invalid_metadata).unwrap();
        assert_failed_reason(&reference, &returned, RoundTripReason::XmlInvalid);
    }

    #[test]
    fn streaming_semantic_failure_does_not_hide_later_invalid_or_limits() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let valid = xml("1", "2", "3", "1 2 3");
        fs::write(&reference, &valid).unwrap();

        let unit_drift = valid.replace("linearUnit=\"meter\"", "linearUnit=\"foot\"");
        fs::write(&returned, format!("{unit_drift}<")).unwrap();
        assert_failed_reason(&reference, &returned, RoundTripReason::XmlInvalid);

        let comments = "<!-- generated -->".repeat(64);
        let over_nodes = unit_drift.replace("</LandXML>", &format!("{comments}</LandXML>"));
        fs::write(&returned, over_nodes).unwrap();
        let error = evaluate_streaming_round_trip(
            &reference,
            &returned,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::new(10_000, 40, 10_000, 10, 10, 100),
        )
        .expect_err("a later XML node excess must remain an operational failure");
        assert_eq!(error.kind(), RoundTripFailureKind::ResourceLimit);
    }

    #[test]
    fn streaming_unit_matrix_uses_unit_drift_reason() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let valid = xml("1", "2", "3", "1 2 3");
        fs::write(&reference, &valid).unwrap();
        let units = "<Units><Metric linearUnit=\"meter\"/></Units>";
        let variants = [
            valid.replace(units, ""),
            valid.replace(
                "<Metric linearUnit=\"meter\"/>",
                "<Imperial linearUnit=\"foot\"/>",
            ),
            valid.replace(units, &format!("{units}{units}")),
            valid.replace("linearUnit=\"meter\"", "linearUnit=\"foot\""),
        ];

        for returned_xml in variants {
            fs::write(&returned, returned_xml).unwrap();
            assert_failed_reason(&reference, &returned, RoundTripReason::UnitDrift);
        }
    }

    #[test]
    fn streaming_identical_duplicate_faces_fail_qualification() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let duplicate =
            xml("1", "2", "3", "1 2 3").replace("<F>1 2 3</F>", "<F>1 2 3</F><F>3 2 1</F>");
        fs::write(&reference, &duplicate).unwrap();
        fs::write(&returned, duplicate).unwrap();

        assert_failed_reason(&reference, &returned, RoundTripReason::TopologyDrift);
    }

    #[test]
    fn streaming_resource_failures_keep_their_operational_class() {
        let directory = Directory::new();
        let reference = directory.path.join("reference.xml");
        let returned = directory.path.join("returned.xml");
        let valid = xml("1", "2", "3", "1 2 3");
        fs::write(&reference, &valid).unwrap();
        fs::write(&returned, &valid).unwrap();

        let error = evaluate_streaming_round_trip(
            &reference,
            &returned,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::new(10_000, 1, 10_000, 10, 10, 100),
        )
        .expect_err("XML node ceiling is an operational resource failure");

        assert_eq!(error.kind(), RoundTripFailureKind::ResourceLimit);
    }

    fn assert_failed_reason(reference: &Path, returned: &Path, reason: RoundTripReason) {
        let evaluation = evaluate_streaming_round_trip(
            reference,
            returned,
            RoundTripDeclaration::new("generated", "test", "metric").unwrap(),
            RoundTripTolerances::new(0.0, 0.0).unwrap(),
            RoundTripLimits::full_v07_export(),
        )
        .expect("well-bounded semantic rejection evaluates to failed evidence");
        let RoundTripEvaluation::Failed(mismatch) = evaluation else {
            panic!("invalid UTF-8 input cannot pass");
        };
        assert_eq!(mismatch.reason(), reason);
    }

    fn xml(first: &str, second: &str, third: &str, face: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" version=\"1.2\"><Units><Metric linearUnit=\"meter\"/></Units><Surfaces><Surface name=\"Generated\"><Definition surfType=\"TIN\"><Pnts><P id=\"{first}\">0 0 0</P><P id=\"{second}\">0 1 0</P><P id=\"{third}\">1 0 0</P></Pnts><Faces><F>{face}</F></Faces></Definition></Surface></Surfaces></LandXML>"
        )
    }

    struct Directory {
        path: PathBuf,
    }

    impl Directory {
        fn new() -> Self {
            loop {
                let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "punctra-streaming-roundtrip-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create streaming fixture: {error}"),
                }
            }
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
