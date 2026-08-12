//! Streaming `LandXML` subset reader for the full v0.7 export-byte ceiling.

use std::{
    fs::{self, File, Metadata},
    io::{self, BufReader, Read as _, Seek as _, SeekFrom},
    mem::size_of,
    path::{Path, PathBuf},
};

use foundation_runtime::OperationControl;
use quick_xml::{
    XmlVersion,
    events::{BytesStart, Event},
    name::ResolveResult,
    reader::NsReader,
};

use crate::{
    publication::same_file_identity,
    roundtrip::{
        ParsedRoundTrip, ParsedSurface, Position, RoundTripDeclaration, RoundTripEvaluation,
        RoundTripFailure, RoundTripFileFacts, RoundTripLimits, RoundTripReason,
        RoundTripTolerances, Triangle, evaluate_parsed_round_trip, semantic_evaluation_failure,
        validate_face, validate_utf8_declaration,
    },
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
    reference: StableInputWitness,
    returned: StableInputWitness,
}

impl StreamingRoundTripEvaluation {
    pub(crate) fn verify_inputs(&self) -> Result<(), RoundTripFailure> {
        self.reference.verify()?;
        self.returned.verify()
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
    // Capture both descriptors and path identities before consuming either
    // stream. This closes the otherwise-open window in which RETURNED could
    // be replaced while REFERENCE was being parsed.
    let reference_input = capture_streaming_input("REFERENCE", reference_path, limits)?;
    let returned_input = capture_streaming_input("RETURNED", returned_path, limits)?;
    if same_file_identity(&reference_input.identity, &returned_input.identity) {
        return Err(RoundTripFailure::invalid(
            "REFERENCE and RETURNED must be distinct regular files",
        ));
    }
    let reference = parse_streaming_file(reference_input, limits, control)?;
    let returned = parse_streaming_file(returned_input, limits, control)?;
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
    witness: StableInputWitness,
}

#[derive(Debug)]
struct StableInputWitness {
    side: &'static str,
    path: PathBuf,
    file: File,
    identity: Metadata,
    facts: RoundTripFileFacts,
}

impl StableInputWitness {
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
        require_same_file(self.side, &self.identity, &opened, &current)?;
        let mut reader = self.file.try_clone().map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} descriptor cannot be cloned for terminal verification: {error}",
                self.side
            ))
        })?;
        reader.seek(SeekFrom::Start(0)).map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} descriptor cannot be rewound for terminal verification: {error}",
                self.side
            ))
        })?;
        let mut hasher = blake3::Hasher::new();
        let mut remaining = self.facts.byte_length;
        let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES].into_boxed_slice();
        while remaining != 0 {
            let requested = usize::try_from(remaining.min(buffer.len() as u64))
                .expect("bounded witness read fits usize");
            let read = reader.read(&mut buffer[..requested]).map_err(|error| {
                RoundTripFailure::invalid(format_args!(
                    "{} descriptor cannot be rehashed: {error}",
                    self.side
                ))
            })?;
            if read == 0 {
                return Err(RoundTripFailure::invalid(format_args!(
                    "{} was truncated after capture",
                    self.side
                )));
            }
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        let mut sentinel = [0_u8; 1];
        if reader.read(&mut sentinel).map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} descriptor cannot be checked for growth: {error}",
                self.side
            ))
        })? != 0
            || hasher.finalize().as_bytes() != &self.facts.content_hash
        {
            return Err(RoundTripFailure::invalid(format_args!(
                "{} content changed after capture",
                self.side
            )));
        }
        let opened_after = self.file.metadata().map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} metadata cannot be terminally rechecked: {error}",
                self.side
            ))
        })?;
        let current_after = fs::symlink_metadata(&self.path).map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} path cannot be terminally rechecked: {error}",
                self.side
            ))
        })?;
        require_same_file(self.side, &self.identity, &opened_after, &current_after)
    }
}

struct CapturedStreamingInput {
    side: &'static str,
    path: PathBuf,
    file: File,
    identity: Metadata,
}

fn capture_streaming_input(
    side: &'static str,
    path: &Path,
    limits: RoundTripLimits,
) -> Result<CapturedStreamingInput, RoundTripFailure> {
    let initial = fs::symlink_metadata(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be inspected: {error}"))
    })?;
    require_regular(side, &initial)?;
    require_file_bytes(side, initial.len(), limits.file_bytes())?;
    let file = open_input_file(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be opened: {error}"))
    })?;
    let opened = file.metadata().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} metadata cannot be read: {error}"))
    })?;
    let current = fs::symlink_metadata(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} path cannot be rechecked: {error}"))
    })?;
    require_same_file(side, &initial, &opened, &current)?;
    Ok(CapturedStreamingInput {
        side,
        path: path.to_path_buf(),
        file,
        identity: opened,
    })
}

fn parse_streaming_file(
    input: CapturedStreamingInput,
    limits: RoundTripLimits,
    control: &OperationControl,
) -> Result<StreamingParse, RoundTripFailure> {
    check_cancelled(control)?;
    let CapturedStreamingInput {
        side,
        path,
        file,
        identity,
    } = input;
    let hashing = HashingReader::new(
        file,
        identity.len(),
        limits.file_bytes(),
        STREAM_BUFFER_BYTES as u64,
    );
    let mut reader = NsReader::from_reader(BufReader::with_capacity(STREAM_BUFFER_BYTES, hashing));
    reader.config_mut().expand_empty_elements = true;
    reader.config_mut().check_end_names = true;
    let surface = StreamParser::new(side, limits, control)?.parse(&mut reader);
    if let Err(error) = &surface
        && error.reason().is_none()
    {
        return Err(error.clone());
    }
    if surface
        .as_ref()
        .is_err_and(|error| error.reason().is_some())
    {
        drain_after_semantic_failure(side, &mut reader, control)?;
    }
    let hashing = reader.into_inner().into_inner();
    let (file, facts, utf8_valid) = hashing.finish(side)?;
    let surface = if utf8_valid {
        surface
    } else {
        Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} is not UTF-8 XML"),
        ))
    };
    let final_opened = file.metadata().map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} metadata cannot be rechecked: {error}"))
    })?;
    let final_path = fs::symlink_metadata(&path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} path cannot be rechecked: {error}"))
    })?;
    require_same_file(side, &identity, &final_opened, &final_path)?;
    if facts.byte_length != identity.len() {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read"
        )));
    }
    Ok(StreamingParse {
        facts,
        surface,
        witness: StableInputWitness {
            side,
            path,
            file,
            identity: final_opened,
            facts,
        },
    })
}

fn drain_after_semantic_failure<R: io::BufRead>(
    side: &str,
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
                    "{side} cannot be drained after semantic failure: {error}"
                ))
            }
        })?;
        if read == 0 {
            return Ok(());
        }
    }
}

struct HashingReader {
    file: File,
    hasher: blake3::Hasher,
    bytes: u64,
    max_bytes: u64,
    remaining: u64,
    utf8: Utf8Validator,
    token_limit: u64,
    token_bytes: u64,
    lexical_state: XmlLexicalState,
}

impl HashingReader {
    fn new(file: File, expected_bytes: u64, max_bytes: u64, token_limit: u64) -> Self {
        Self {
            file,
            hasher: blake3::Hasher::new(),
            bytes: 0,
            max_bytes,
            remaining: expected_bytes,
            utf8: Utf8Validator::default(),
            token_limit,
            token_bytes: 0,
            lexical_state: XmlLexicalState::Text,
        }
    }

    #[allow(clippy::too_many_lines)]
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
                } => XmlLexicalState::Tag {
                    quote: (byte != expected).then_some(expected),
                },
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
                self.token_bytes = 0;
            }
            self.lexical_state = next;
        }
        Ok(())
    }

    fn tag_state(quote: Option<u8>, byte: u8) -> XmlLexicalState {
        if quote.is_none() && matches!(byte, b'\'' | b'"') {
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
        if matches!(byte, b'\'' | b'"') {
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

    fn finish(self, side: &str) -> Result<(File, RoundTripFileFacts, bool), RoundTripFailure> {
        if self.remaining != 0 {
            return Err(RoundTripFailure::resource(format_args!(
                "{side} was truncated with {} witnessed bytes unread",
                self.remaining
            )));
        }
        Ok((
            self.file,
            RoundTripFileFacts {
                content_hash: *self.hasher.finalize().as_bytes(),
                byte_length: self.bytes,
            },
            self.utf8.is_valid_at_end(),
        ))
    }
}

impl io::Read for HashingReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let requested = usize::try_from(self.remaining.min(buffer.len() as u64))
            .expect("bounded streaming read fits usize");
        let read = self.file.read(&mut buffer[..requested])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "LandXML input was truncated during capture",
            ));
        }
        self.bytes = self.bytes.saturating_add(read as u64);
        self.remaining -= read as u64;
        if self.bytes > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "LandXML input exceeded its byte limit",
            ));
        }
        self.check_tokens(&buffer[..read])?;
        self.hasher.update(&buffer[..read]);
        self.utf8.update(&buffer[..read]);
        Ok(read)
    }
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
    side: &'static str,
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
    point_ids: Vec<(u64, usize)>,
    faces: Vec<Triangle>,
    simple_text: String,
    control: &'a OperationControl,
}

impl<'a> StreamParser<'a> {
    fn new(
        side: &'static str,
        limits: RoundTripLimits,
        control: &'a OperationControl,
    ) -> Result<Self, RoundTripFailure> {
        validate_retained_model_limit(side, limits)?;
        let points = Vec::new();
        let faces = Vec::new();
        let point_ids = Vec::new();
        let stack = reserve_exact_model::<Tag>(side, 32, limits)?;
        let mut simple_text = String::new();
        simple_text
            .try_reserve_exact(STREAM_BUFFER_BYTES)
            .map_err(|_| {
                RoundTripFailure::resource(format_args!(
                    "{side} simple XML text buffer cannot reserve {STREAM_BUFFER_BYTES} bytes"
                ))
            })?;
        if simple_text.capacity() != STREAM_BUFFER_BYTES {
            return Err(RoundTripFailure::resource(format_args!(
                "{side} simple XML text buffer retained {} bytes; exact limit is {STREAM_BUFFER_BYTES}",
                simple_text.capacity()
            )));
        }
        Ok(Self {
            side,
            limits,
            stack,
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
            points,
            point_ids,
            faces,
            simple_text,
            control,
        })
    }

    fn parse<R: io::BufRead>(
        mut self,
        reader: &mut NsReader<R>,
    ) -> Result<ParsedSurface, RoundTripFailure> {
        let mut buffer = Vec::with_capacity(STREAM_BUFFER_BYTES);
        loop {
            let (namespace, event) =
                reader
                    .read_resolved_event_into(&mut buffer)
                    .map_err(|error| {
                        if matches!(
                            &error,
                            quick_xml::Error::Io(source)
                                if source.kind() == io::ErrorKind::FileTooLarge
                        ) {
                            return RoundTripFailure::resource(format_args!(
                                "{} XML token exceeds the hard {} byte limit",
                                self.side, STREAM_BUFFER_BYTES
                            ));
                        }
                        RoundTripFailure::semantic(
                            RoundTripReason::XmlInvalid,
                            format_args!("{} XML is malformed: {error}", self.side),
                        )
                    })?;
            if !matches!(event, Event::End(_) | Event::Eof) {
                self.count_node()?;
            }
            match event {
                Event::Start(start) => self.start(&namespace, &start)?,
                Event::End(_) => self.end()?,
                Event::Text(text) => {
                    self.add_text_bytes(text.as_ref().len())?;
                    if matches!(self.stack.last(), Some(Tag::Point | Tag::Face)) {
                        let decoded = text.decode().map_err(|error| {
                            RoundTripFailure::semantic(
                                RoundTripReason::XmlInvalid,
                                format_args!("{} XML text is invalid: {error}", self.side),
                            )
                        })?;
                        self.simple_text.push_str(&decoded);
                    } else if self.metadata_depth == 0
                        && text.decode().is_ok_and(|value| !value.trim().is_empty())
                    {
                        return Err(self.unsupported("semantic container has unexpected text"));
                    }
                }
                Event::Comment(comment) => self.add_text_bytes(comment.as_ref().len())?,
                Event::Decl(declaration) => validate_utf8_declaration(
                    if self.side == "REFERENCE" {
                        crate::roundtrip::InputSide::Reference
                    } else {
                        crate::roundtrip::InputSide::Returned
                    },
                    &declaration,
                )?,
                Event::PI(_) => {}
                Event::DocType(_) | Event::GeneralRef(_) | Event::CData(_) => {
                    return Err(self.xml_invalid("DTD, entity, or CDATA input is unsupported"));
                }
                Event::Empty(_) => unreachable!("empty elements are expanded"),
                Event::Eof => break,
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
        self.finish()
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
            return Err(self.unsupported("foreign semantic element is unsupported"));
        }
        if start.local_name().as_ref() == b"CoordinateSystem" {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::CoordinateReferenceUnsupported,
                format_args!("{} CoordinateSystem semantics are unsupported", self.side),
            ));
        }
        let tag = tag(start.local_name().as_ref())
            .ok_or_else(|| self.unsupported("unknown LandXML semantic element"))?;
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
            Tag::Points => self.finish_point_ids(),
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
            let value = found.ok_or_else(|| self.unsupported("required attribute is absent"))?;
            match tag {
                Tag::Metric if value != expected => {
                    return Err(RoundTripFailure::semantic(
                        RoundTripReason::UnitDrift,
                        format_args!("{} units are not metric metres", self.side),
                    ));
                }
                Tag::Surface => {
                    self.surface_name = (!value.is_empty()).then(|| value.into_boxed_str());
                }
                Tag::Point => {
                    if self.point_ids.len() as u64 >= self.limits.points() {
                        return Err(RoundTripFailure::resource(format_args!(
                            "{} Point identifiers exceed the {} point limit",
                            self.side,
                            self.limits.points()
                        )));
                    }
                    ensure_model_slot(
                        &mut self.point_ids,
                        self.limits.points(),
                        self.side,
                        "Point identifier index",
                    )?;
                    let id = value
                        .parse::<u64>()
                        .map_err(|_| self.xml_invalid("Point id must be a positive integer"))?;
                    if id == 0 {
                        return Err(self.xml_invalid("Point id is zero"));
                    }
                    self.point_ids.push((id, self.points.len()));
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
        ensure_model_slot(
            &mut self.points,
            self.limits.points(),
            self.side,
            "Point storage",
        )?;
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
                .binary_search_by_key(&id, |entry| entry.0)
                .ok()
                .map(|index| self.point_ids[index].1)
                .ok_or_else(|| self.xml_invalid("Face has a dangling Point reference"))
        };
        let face = Triangle::new(resolve(a)?, resolve(b)?, resolve(c)?);
        validate_face(crate::roundtrip::InputSide::Returned, face, &self.points)?;
        ensure_model_slot(
            &mut self.faces,
            self.limits.faces(),
            self.side,
            "Face storage",
        )?;
        self.faces.push(face);
        Ok(())
    }

    fn finish_point_ids(&mut self) -> Result<(), RoundTripFailure> {
        self.point_ids.sort_unstable_by_key(|entry| entry.0);
        if self
            .point_ids
            .windows(2)
            .any(|entries| entries[0].0 == entries[1].0)
        {
            Err(self.xml_invalid("Point id is duplicated"))
        } else {
            Ok(())
        }
    }

    fn finish(self) -> Result<ParsedSurface, RoundTripFailure> {
        if !self.stack.is_empty()
            || self.root_count != 1
            || self.units_count != 1
            || self.metric_count != 1
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
}

fn validate_retained_model_limit(
    side: &str,
    limits: RoundTripLimits,
) -> Result<(), RoundTripFailure> {
    // Peak model overlap includes both parsed surfaces, the exact/tolerant
    // point matcher indices and mapping, both sorted topology projections,
    // the parser Point-id index, and fixed parser/token buffers. BTreeMap node
    // layout is implementation-private, so charge a conservative four-word
    // node/link surcharge in addition to its key/value payload.
    let required = required_retained_model_bytes(limits);
    if required > limits.retained_model_bytes() {
        Err(RoundTripFailure::resource(format_args!(
            "{side} retained model requires {required} bytes; limit is {}",
            limits.retained_model_bytes()
        )))
    } else {
        Ok(())
    }
}

fn required_retained_model_bytes(limits: RoundTripLimits) -> u64 {
    let point_peak = limits.points().saturating_mul(
        (2 * size_of::<Position>()
            + size_of::<([u64; 3], usize)>()
            + size_of::<usize>()
            + size_of::<(u64, usize)>()
            + 4 * size_of::<usize>()) as u64,
    );
    let face_peak = limits
        .faces()
        .saturating_mul((4 * size_of::<[usize; 3]>()) as u64);
    let fixed = (3 * STREAM_BUFFER_BYTES
        + 32 * size_of::<Tag>()
        + 2 * 16 * size_of::<[usize; 3]>()
        + 4 * size_of::<Box<str>>()) as u64;
    point_peak.saturating_add(face_peak).saturating_add(fixed)
}

fn reserve_exact_model<T>(
    side: &str,
    count: u64,
    limits: RoundTripLimits,
) -> Result<Vec<T>, RoundTripFailure> {
    let count = usize::try_from(count).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} model element count exceeds addressable memory"
        ))
    })?;
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} model allocation exceeds the {} byte retained-model limit",
            limits.retained_model_bytes()
        ))
    })?;
    let retained = (values.capacity() as u64).saturating_mul(size_of::<T>() as u64);
    let requested = (count as u64).saturating_mul(size_of::<T>() as u64);
    if retained > requested {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} allocator retained {retained} bytes for a {requested} byte model request"
        )));
    }
    Ok(values)
}

fn ensure_model_slot<T>(
    values: &mut Vec<T>,
    max_items: u64,
    side: &str,
    label: &str,
) -> Result<(), RoundTripFailure> {
    if values.len() < values.capacity() {
        return Ok(());
    }
    let maximum = usize::try_from(max_items).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} {label} limit exceeds addressable memory"
        ))
    })?;
    let current = values.len();
    let target = current.saturating_mul(2).max(1_024).min(maximum);
    if target <= current {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} {label} exceeds the {max_items} item limit"
        )));
    }
    values.try_reserve_exact(target - current).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} {label} cannot reserve {} bytes",
            (target as u64).saturating_mul(size_of::<T>() as u64)
        ))
    })?;
    if values.capacity() > maximum {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} {label} allocator capacity {} exceeds the {max_items} item limit",
            values.capacity()
        )));
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

fn parse_number(side: &str, value: Option<&str>) -> Result<f64, RoundTripFailure> {
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

fn parse_id(side: &str, value: Option<&str>) -> Result<u64, RoundTripFailure> {
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

fn require_regular(side: &str, metadata: &Metadata) -> Result<(), RoundTripFailure> {
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(RoundTripFailure::invalid(format_args!(
            "{side} must be a regular non-symlink file"
        )))
    }
}

fn require_file_bytes(side: &str, actual: u64, allowed: u64) -> Result<(), RoundTripFailure> {
    if actual <= allowed {
        Ok(())
    } else {
        Err(RoundTripFailure::resource(format_args!(
            "{side} file bytes required {actual}; limit is {allowed}"
        )))
    }
}

fn require_same_file(
    side: &str,
    initial: &Metadata,
    opened: &Metadata,
    current: &Metadata,
) -> Result<(), RoundTripFailure> {
    require_regular(side, opened)?;
    require_regular(side, current)?;
    if same_file_identity(initial, opened)
        && same_file_identity(opened, current)
        && same_state(initial, opened)
        && same_state(opened, current)
    {
        Ok(())
    } else {
        Err(RoundTripFailure::invalid(format_args!(
            "{side} changed during streaming capture"
        )))
    }
}

#[cfg(unix)]
fn same_state(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_state(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_state(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

#[cfg(unix)]
fn open_input_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(windows)]
fn open_input_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_input_file(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable no-follow input capture is unavailable on this platform",
    ))
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
        HashingReader, STREAM_BUFFER_BYTES, Utf8Validator, evaluate_streaming_round_trip,
        evaluate_streaming_round_trip_with_control, required_retained_model_bytes,
        validate_retained_model_limit,
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

    #[test]
    fn retained_model_accounting_has_an_exact_limit_boundary() {
        let base = RoundTripLimits::new(10_000, 1_000, 10_000, 10, 10, 100);
        let required = required_retained_model_bytes(base);
        validate_retained_model_limit("TEST", base.with_retained_model_bytes(required))
            .expect("the accounted peak fits its exact retained-model boundary");
        let error =
            validate_retained_model_limit("TEST", base.with_retained_model_bytes(required - 1))
                .expect_err("one byte below the accounted peak must fail closed");
        assert_eq!(error.kind(), RoundTripFailureKind::ResourceLimit);
    }

    #[test]
    fn lexical_token_bound_covers_comment_cdata_pi_and_doctype_terminators() {
        let repeated_angles = "<>".repeat(STREAM_BUFFER_BYTES);
        let repeated_text = "x".repeat(STREAM_BUFFER_BYTES * 2);
        for (label, token) in [
            ("text", repeated_text),
            ("comment", format!("<!--{repeated_angles}-->")),
            ("cdata", format!("<![CDATA[{repeated_angles}]]>")),
            ("pi", format!("<?probe {repeated_angles}?>")),
            (
                "doctype",
                format!("<!DOCTYPE LandXML [{}]>", "<!ELEMENT A ANY>".repeat(8_192)),
            ),
        ] {
            let directory = Directory::new();
            let path = directory.path.join(format!("{label}.xml"));
            fs::write(&path, &token).unwrap();
            let file = fs::File::open(&path).unwrap();
            let hashing = HashingReader::new(
                file,
                token.len() as u64,
                token.len() as u64,
                STREAM_BUFFER_BYTES as u64,
            );
            let mut reader = quick_xml::Reader::from_reader(std::io::BufReader::with_capacity(
                8 * 1024,
                hashing,
            ));
            let mut buffer = Vec::new();
            buffer.try_reserve_exact(STREAM_BUFFER_BYTES).unwrap();
            loop {
                buffer.clear();
                match reader.read_event_into(&mut buffer) {
                    Err(quick_xml::Error::Io(error))
                        if error.kind() == std::io::ErrorKind::FileTooLarge =>
                    {
                        assert!(buffer.len() <= STREAM_BUFFER_BYTES, "{label}");
                        assert!(buffer.capacity() <= STREAM_BUFFER_BYTES, "{label}");
                        break;
                    }
                    Err(error) => panic!("{label} failed before hard token bound: {error}"),
                    Ok(quick_xml::events::Event::Eof) => {
                        panic!("{label} escaped the hard lexical token bound")
                    }
                    Ok(_) => assert!(
                        buffer.len() <= STREAM_BUFFER_BYTES,
                        "{label}: len={} capacity={}",
                        buffer.len(),
                        buffer.capacity()
                    ),
                }
            }
        }
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
