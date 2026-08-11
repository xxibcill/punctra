use std::{
    fmt::{self, Write as _},
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use blake3::Hasher;
use foundation_runtime::{Job, OperationControl, ProgressPhase, ProgressSnapshot};
use point_contracts::ContentHash;

use crate::{
    LandXmlJob, LandXmlLimits, LandXmlReceipt, TerrainError, TerrainSurface,
    numeric::canonical_zero,
};

const LANDXML_NAMESPACE: &str = "http://www.landxml.org/schema/LandXML-1.2";
const XML_SCHEMA_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
const MAX_SURFACE_NAME_BYTES: usize = 1_024;
const CANCELLATION_STRIDE: usize = 4_096;
const STACK_TOKEN_BYTES: usize = 512;
const PUBLICATION_STEPS: u64 = 3;

static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationBoundary {
    BeforeLink,
    TargetVerification,
    ParentSync,
    StageRemoval,
    CleanupSync,
    TerminalProgress,
}

trait PublicationHook {
    fn reach(&self, boundary: PublicationBoundary, control: &OperationControl) -> io::Result<()>;
}

struct ProductionPublicationHook;

impl PublicationHook for ProductionPublicationHook {
    fn reach(&self, _boundary: PublicationBoundary, _control: &OperationControl) -> io::Result<()> {
        Ok(())
    }
}

/// Checked deterministic facts for the v0.6 metric-metre `LandXML` subset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LandXmlOptions {
    surface_name: Box<str>,
    document_date: Box<str>,
    document_time: Box<str>,
    allow_unknown_coordinate_reference: bool,
}

impl LandXmlOptions {
    /// Creates checked metric-metre options with explicit `LandXML` root date/time.
    ///
    /// `document_date` must be `YYYY-MM-DD`. `document_time` must be
    /// `HH:MM:SSZ`. The encoder never reads a clock or filesystem timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::InvalidArgument`] for an empty, oversized, or
    /// XML-invalid Surface name, an impossible calendar date, or an invalid
    /// UTC time.
    pub fn metric_metres(
        surface_name: impl Into<String>,
        document_date: impl Into<String>,
        document_time: impl Into<String>,
    ) -> Result<Self, TerrainError> {
        let surface_name = surface_name.into();
        validate_surface_name(&surface_name)?;
        let document_date = document_date.into();
        validate_date(&document_date)?;
        let document_time = document_time.into();
        validate_time(&document_time)?;
        Ok(Self {
            surface_name: surface_name.into_boxed_str(),
            document_date: document_date.into_boxed_str(),
            document_time: document_time.into_boxed_str(),
            allow_unknown_coordinate_reference: false,
        })
    }

    /// Explicitly asserts that an unknown Source reference uses metric metres.
    ///
    /// This does not infer, transform, or attach a Coordinate Reference. It is
    /// the caller's checked assertion that Source X/Y/Z already mean easting,
    /// northing, and elevation in metres.
    #[must_use]
    pub fn allow_unknown_coordinate_reference_as_metric_metres(mut self) -> Self {
        self.allow_unknown_coordinate_reference = true;
        self
    }

    /// Returns the caller-supplied Surface name.
    #[must_use]
    pub fn surface_name(&self) -> &str {
        &self.surface_name
    }

    /// Returns the explicit `YYYY-MM-DD` document date.
    #[must_use]
    pub fn document_date(&self) -> &str {
        &self.document_date
    }

    /// Returns the explicit `HH:MM:SSZ` UTC document time.
    #[must_use]
    pub fn document_time(&self) -> &str {
        &self.document_time
    }

    /// Reports the explicit unknown-reference metric-metre assertion.
    #[must_use]
    pub const fn allows_unknown_coordinate_reference(&self) -> bool {
        self.allow_unknown_coordinate_reference
    }
}

pub(crate) fn start(
    surface: &TerrainSurface,
    target: impl AsRef<Path>,
    options: LandXmlOptions,
    limits: LandXmlLimits,
) -> LandXmlJob {
    let surface = surface.clone();
    let target = target.as_ref().to_path_buf();
    Job::spawn(move |control| {
        publish(
            &surface,
            &target,
            &options,
            limits,
            &control,
            &ProductionPublicationHook,
        )
    })
}

fn publish<H: PublicationHook>(
    surface: &TerrainSurface,
    target: &Path,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
    control: &OperationControl,
    hook: &H,
) -> Result<LandXmlReceipt, TerrainError> {
    control.check_cancelled()?;
    validate_export(surface, target, options, limits)?;
    let parent = target_parent(target);
    match fs::symlink_metadata(target) {
        Ok(_) => {
            return Err(TerrainError::TargetExists {
                path: crate::TerrainDiagnostic::new(target.display().to_string()),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(TerrainError::io(
                "inspect LandXML target",
                target.display(),
                error,
            ));
        }
    }

    let buffer_bytes = choose_buffer_bytes(limits)?;
    let (stage_path, stage_file) = create_stage(parent)?;
    let mut stage = StageGuard::new(stage_path);
    let total_elements = surface
        .descriptor()
        .vertex_count()
        .checked_add(surface.descriptor().face_count())
        .ok_or_else(|| TerrainError::numeric("LandXML element count overflowed"))?;
    let total_progress = total_elements
        .checked_add(PUBLICATION_STEPS)
        .ok_or_else(|| TerrainError::numeric("LandXML progress count overflowed"))?;

    let expected = encode_stage(
        surface,
        options,
        limits,
        control,
        StageEncoding {
            file: stage_file,
            path: stage.path(),
            buffer_bytes,
        },
        EncodingProgress {
            elements: total_elements,
            total: total_progress,
        },
    )?;
    control.report_progress(ProgressSnapshot::new(
        ProgressPhase::new(2),
        total_elements.saturating_add(1),
        Some(total_progress),
    )?)?;
    // Verification reopens the synced, closed stage through read-only
    // `File::open`; no mutable encoder handle or timestamp is reused.
    let verified = verify_file(stage.path(), buffer_bytes, limits, Some(control))?;
    if verified != expected {
        return Err(TerrainError::topology(
            "staged LandXML bytes changed during read-back verification",
        ));
    }
    control.check_cancelled()?;

    control.report_progress(ProgressSnapshot::new(
        ProgressPhase::new(3),
        total_elements.saturating_add(2),
        Some(total_progress),
    )?)?;
    hook.reach(PublicationBoundary::BeforeLink, control)
        .map_err(|error| {
            TerrainError::io("run LandXML pre-link boundary", target.display(), error)
        })?;
    control.check_cancelled()?;
    // Once this create-new link succeeds, every remaining failure is
    // indeterminate because a complete target may be durably observable.
    publish_target(stage.path(), target)?;
    finish_publication(
        surface,
        &mut stage,
        &expected,
        control,
        hook,
        PublicationCompletion {
            target,
            parent,
            buffer_bytes,
            limits,
            total_progress,
        },
    )
}

#[derive(Clone, Copy)]
struct PublicationCompletion<'a> {
    target: &'a Path,
    parent: &'a Path,
    buffer_bytes: usize,
    limits: LandXmlLimits,
    total_progress: u64,
}

fn finish_publication<H: PublicationHook>(
    surface: &TerrainSurface,
    stage: &mut StageGuard,
    expected: &FileFacts,
    control: &OperationControl,
    hook: &H,
    completion: PublicationCompletion<'_>,
) -> Result<LandXmlReceipt, TerrainError> {
    require_post_link_boundary(
        hook,
        PublicationBoundary::TargetVerification,
        control,
        expected.hash,
    )?;
    let published = verify_file(
        completion.target,
        completion.buffer_bytes,
        completion.limits,
        None,
    )
    .map_err(|_| TerrainError::ExportIndeterminate {
        expected_hash: expected.hash,
    })?;
    if published != *expected {
        return Err(TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        });
    }
    require_post_link_boundary(
        hook,
        PublicationBoundary::ParentSync,
        control,
        expected.hash,
    )?;
    if sync_directory(completion.parent).is_err() {
        return Err(TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        });
    }

    require_post_link_boundary(
        hook,
        PublicationBoundary::StageRemoval,
        control,
        expected.hash,
    )?;
    stage
        .remove()
        .map_err(|_| TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        })?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::CleanupSync,
        control,
        expected.hash,
    )?;
    sync_directory(completion.parent).map_err(|_| TerrainError::ExportIndeterminate {
        expected_hash: expected.hash,
    })?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::TerminalProgress,
        control,
        expected.hash,
    )?;
    control
        .complete_progress(completion.total_progress)
        .map_err(|_| TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        })?;
    Ok(LandXmlReceipt::new(
        surface.descriptor().artifact_hash(),
        surface.descriptor().geometry_hash(),
        surface.descriptor().topology_hash(),
        expected.hash,
        expected.bytes,
        surface.descriptor().vertex_count(),
        surface.descriptor().face_count(),
    ))
}

fn require_post_link_boundary<H: PublicationHook>(
    hook: &H,
    boundary: PublicationBoundary,
    control: &OperationControl,
    expected_hash: ContentHash,
) -> Result<(), TerrainError> {
    hook.reach(boundary, control)
        .map_err(|_| TerrainError::ExportIndeterminate { expected_hash })
}

fn encode_stage(
    surface: &TerrainSurface,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
    control: &OperationControl,
    stage: StageEncoding<'_>,
    progress: EncodingProgress,
) -> Result<FileFacts, TerrainError> {
    let bounded = BoundedHashWriter::new(stage.file, limits);
    let mut writer = BufWriter::with_capacity(stage.buffer_bytes, bounded);
    write_document(
        &mut writer,
        surface,
        options,
        limits,
        control,
        progress.elements,
        progress.total,
    )?;
    writer.flush().map_err(map_write_error)?;
    let bounded = writer
        .into_inner()
        .map_err(|error| map_write_error(error.into_error()))?;
    let (file, hash, bytes) = bounded.finish();
    file.sync_all()
        .map_err(|error| TerrainError::io("sync LandXML stage", stage.path.display(), error))?;
    drop(file);
    Ok(FileFacts { bytes, hash })
}

struct StageEncoding<'a> {
    file: File,
    path: &'a Path,
    buffer_bytes: usize,
}

#[derive(Clone, Copy)]
struct EncodingProgress {
    elements: u64,
    total: u64,
}

fn publish_target(stage: &Path, target: &Path) -> Result<(), TerrainError> {
    match fs::hard_link(stage, target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(TerrainError::TargetExists {
                path: crate::TerrainDiagnostic::new(target.display().to_string()),
            });
        }
        Err(error) => {
            return Err(TerrainError::io(
                "publish LandXML target",
                target.display(),
                error,
            ));
        }
    }
    Ok(())
}

fn validate_export(
    surface: &TerrainSurface,
    target: &Path,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
) -> Result<(), TerrainError> {
    if target.file_name().is_none() {
        return Err(TerrainError::invalid(
            "LandXML target",
            "target must name a file",
        ));
    }
    require_limit(
        "LandXML vertices",
        surface.descriptor().vertex_count(),
        limits.max_vertices(),
    )?;
    require_limit(
        "LandXML faces",
        surface.descriptor().face_count(),
        limits.max_faces(),
    )?;
    if surface.descriptor().coordinate_reference().is_unknown()
        && !options.allows_unknown_coordinate_reference()
    {
        return Err(TerrainError::invalid(
            "LandXML Coordinate Reference",
            "unknown Source reference requires an explicit metric-metre assertion",
        ));
    }
    let escaped_name_bytes = escaped_len(options.surface_name())?;
    check_token_len("LandXML Surface name token", escaped_name_bytes, limits)?;
    Ok(())
}

fn write_document(
    writer: &mut impl Write,
    surface: &TerrainSurface,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
    control: &OperationControl,
    total_elements: u64,
    total_progress: u64,
) -> Result<(), TerrainError> {
    write_header(writer, options, limits)?;
    write_vertices(writer, surface, limits, control, total_progress)?;
    write_token(writer, "        </Pnts>\n        <Faces>\n", limits)?;
    write_faces(writer, surface, limits, control, total_progress)?;
    report_encoding_progress(
        control,
        usize::try_from(total_elements).unwrap_or(usize::MAX),
        total_progress,
    )?;
    write_token(
        writer,
        "        </Faces>\n      </Definition>\n    </Surface>\n  </Surfaces>\n</LandXML>\n",
        limits,
    )
}

fn write_header(
    writer: &mut impl Write,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
) -> Result<(), TerrainError> {
    write_token(
        writer,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        limits,
    )?;
    let mut root = StackToken::new();
    writeln!(
        root,
        "<LandXML xmlns=\"{LANDXML_NAMESPACE}\" xmlns:xsi=\"{XML_SCHEMA_NAMESPACE}\" \
         xsi:schemaLocation=\"{LANDXML_NAMESPACE} {LANDXML_NAMESPACE}/LandXML-1.2.xsd\" \
         version=\"1.2\" date=\"{}\" time=\"{}\">",
        options.document_date(),
        options.document_time(),
    )
    .map_err(|_| TerrainError::numeric("LandXML root token exceeded its stack bound"))?;
    write_token(writer, root.as_str(), limits)?;
    write_token(writer, "  <Units>\n", limits)?;
    write_token(
        writer,
        "    <Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" \
         volumeUnit=\"cubicMeter\" temperatureUnit=\"celsius\" \
         pressureUnit=\"milliBars\" angularUnit=\"decimal dd.mm.ss\" \
         directionUnit=\"decimal dd.mm.ss\"/>\n",
        limits,
    )?;
    write_token(writer, "  </Units>\n  <Surfaces>\n", limits)?;
    let surface_token_len = "    <Surface name=\"\">\n"
        .len()
        .saturating_add(escaped_len(options.surface_name())?);
    check_token_len("LandXML Surface token", surface_token_len, limits)?;
    write_token(writer, "    <Surface name=\"", limits)?;
    write_escaped(writer, options.surface_name())?;
    write_token(
        writer,
        "\">\n      <Definition surfType=\"TIN\">\n        <Pnts>\n",
        limits,
    )
}

fn write_vertices(
    writer: &mut impl Write,
    surface: &TerrainSurface,
    limits: LandXmlLimits,
    control: &OperationControl,
    total_progress: u64,
) -> Result<(), TerrainError> {
    let transform = surface.descriptor().position_transform();
    for (index, vertex) in surface.vertices().iter().copied().enumerate() {
        if index.is_multiple_of(CANCELLATION_STRIDE) {
            control.check_cancelled()?;
            report_encoding_progress(control, index, total_progress)?;
        }
        let mut world = transform.world_f64(vertex.ticks());
        if world.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(TerrainError::numeric(
                "LandXML vertex world position is not finite",
            ));
        }
        for coordinate in &mut world {
            *coordinate = canonical_zero(*coordinate);
        }
        let mut token = StackToken::new();
        writeln!(
            token,
            "          <P id=\"{}\">{} {} {}</P>",
            vertex.id().get(),
            world[1],
            world[0],
            world[2],
        )
        .map_err(|_| TerrainError::numeric("LandXML point token exceeded its stack bound"))?;
        write_token(writer, token.as_str(), limits)?;
    }
    Ok(())
}

fn write_faces(
    writer: &mut impl Write,
    surface: &TerrainSurface,
    limits: LandXmlLimits,
    control: &OperationControl,
    total_progress: u64,
) -> Result<(), TerrainError> {
    let vertex_count = surface.vertices().len();
    for (index, face) in surface.faces().iter().copied().enumerate() {
        if index.is_multiple_of(CANCELLATION_STRIDE) {
            control.check_cancelled()?;
            report_encoding_progress(
                control,
                surface.vertices().len().saturating_add(index),
                total_progress,
            )?;
        }
        for vertex in face.vertices() {
            if vertex.zero_based() >= vertex_count {
                return Err(TerrainError::topology(
                    "a LandXML face references a missing Surface vertex",
                ));
            }
        }
        let vertices = face.vertices();
        let mut token = StackToken::new();
        writeln!(
            token,
            "          <F>{} {} {}</F>",
            vertices[0].get(),
            vertices[1].get(),
            vertices[2].get(),
        )
        .map_err(|_| TerrainError::numeric("LandXML face token exceeded its stack bound"))?;
        write_token(writer, token.as_str(), limits)?;
    }
    Ok(())
}

fn report_encoding_progress(
    control: &OperationControl,
    completed: usize,
    total_progress: u64,
) -> Result<(), TerrainError> {
    let completed = u64::try_from(completed).unwrap_or(u64::MAX);
    control.report_progress(ProgressSnapshot::new(
        ProgressPhase::RUNNING,
        completed,
        Some(total_progress),
    )?)?;
    Ok(())
}

fn write_escaped(writer: &mut impl Write, value: &str) -> Result<(), TerrainError> {
    for character in value.chars() {
        let escaped = match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            '\t' => "&#x9;",
            '\n' => "&#xA;",
            '\r' => "&#xD;",
            _ => {
                let mut buffer = [0_u8; 4];
                let encoded = character.encode_utf8(&mut buffer);
                writer
                    .write_all(encoded.as_bytes())
                    .map_err(map_write_error)?;
                continue;
            }
        };
        writer
            .write_all(escaped.as_bytes())
            .map_err(map_write_error)?;
    }
    Ok(())
}

fn write_token(
    writer: &mut impl Write,
    token: &str,
    limits: LandXmlLimits,
) -> Result<(), TerrainError> {
    check_token_len("LandXML XML token bytes", token.len(), limits)?;
    writer.write_all(token.as_bytes()).map_err(map_write_error)
}

fn check_token_len(
    name: &'static str,
    bytes: usize,
    limits: LandXmlLimits,
) -> Result<(), TerrainError> {
    require_limit(
        name,
        u64::try_from(bytes).unwrap_or(u64::MAX),
        limits.max_xml_token_bytes(),
    )
}

fn target_parent(target: &Path) -> &Path {
    match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn create_stage(parent: &Path) -> Result<(PathBuf, File), TerrainError> {
    for _ in 0..64 {
        let sequence = NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".punctra-landxml-{}-{sequence}.stage",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(TerrainError::io(
                    "create LandXML stage",
                    path.display(),
                    error,
                ));
            }
        }
    }
    Err(TerrainError::invalid(
        "LandXML target",
        "could not reserve a unique sibling stage",
    ))
}

fn verify_file(
    path: &Path,
    buffer_bytes: usize,
    limits: LandXmlLimits,
    control: Option<&OperationControl>,
) -> Result<FileFacts, TerrainError> {
    let mut file = File::open(path).map_err(|error| {
        TerrainError::io("open LandXML for verification", path.display(), error)
    })?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(buffer_bytes).map_err(|_| {
        TerrainError::resource(
            "LandXML verification buffer allocation",
            u64::try_from(buffer_bytes).unwrap_or(u64::MAX),
            limits.max_working_bytes(),
        )
    })?;
    buffer.resize(buffer_bytes, 0);
    require_limit(
        "LandXML verification working bytes",
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        limits.max_working_bytes(),
    )?;
    let mut hasher = Hasher::new();
    let mut bytes = 0_u64;
    loop {
        if let Some(control) = control {
            control.check_cancelled()?;
        }
        let read = file.read(&mut buffer).map_err(|error| {
            TerrainError::io("read LandXML for verification", path.display(), error)
        })?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| TerrainError::numeric("LandXML verification byte count overflowed"))?;
        require_limit("LandXML output bytes", bytes, limits.max_output_bytes())?;
        require_limit("LandXML staging bytes", bytes, limits.max_staging_bytes())?;
        hasher.update(&buffer[..read]);
    }
    Ok(FileFacts {
        bytes,
        hash: ContentHash::new(*hasher.finalize().as_bytes()),
    })
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn choose_buffer_bytes(limits: LandXmlLimits) -> Result<usize, TerrainError> {
    if limits.max_write_buffer_bytes() == 0 {
        return Err(TerrainError::resource(
            "LandXML write buffer bytes",
            1,
            limits.max_write_buffer_bytes(),
        ));
    }
    if limits.max_working_bytes() == 0 {
        return Err(TerrainError::resource(
            "LandXML working bytes",
            1,
            limits.max_working_bytes(),
        ));
    }
    let allowed = limits
        .max_write_buffer_bytes()
        .min(limits.max_working_bytes());
    usize::try_from(allowed.min(64 * 1024)).map_err(|_| {
        TerrainError::resource(
            "LandXML write buffer bytes",
            allowed,
            u64::try_from(usize::MAX).unwrap_or(u64::MAX),
        )
    })
}

fn require_limit(name: &'static str, required: u64, allowed: u64) -> Result<(), TerrainError> {
    if required > allowed {
        return Err(TerrainError::resource(name, required, allowed));
    }
    Ok(())
}

fn map_write_error(error: io::Error) -> TerrainError {
    if let Some(limit) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<OutputLimit>())
    {
        return TerrainError::resource(limit.name, limit.required, limit.allowed);
    }
    TerrainError::io("write LandXML stage", "staging file", error)
}

fn validate_surface_name(value: &str) -> Result<(), TerrainError> {
    if value.trim().is_empty() {
        return Err(TerrainError::invalid(
            "LandXML Surface name",
            "name must not be empty or whitespace-only",
        ));
    }
    if value.len() > MAX_SURFACE_NAME_BYTES {
        return Err(TerrainError::invalid(
            "LandXML Surface name",
            format!(
                "name is {} UTF-8 bytes; maximum is {MAX_SURFACE_NAME_BYTES}",
                value.len()
            ),
        ));
    }
    if value.chars().any(|character| !is_xml_character(character)) {
        return Err(TerrainError::invalid(
            "LandXML Surface name",
            "name contains a character forbidden by XML 1.0",
        ));
    }
    Ok(())
}

fn is_xml_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

fn validate_date(value: &str) -> Result<(), TerrainError> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return Err(TerrainError::invalid(
            "LandXML document date",
            "date must use YYYY-MM-DD",
        ));
    }
    let year = parse_decimal(&bytes[0..4]);
    let month = parse_decimal(&bytes[5..7]);
    let day = parse_decimal(&bytes[8..10]);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > max_day {
        return Err(TerrainError::invalid(
            "LandXML document date",
            "date is not a valid positive Gregorian calendar date",
        ));
    }
    Ok(())
}

fn validate_time(value: &str) -> Result<(), TerrainError> {
    let bytes = value.as_bytes();
    if bytes.len() != 9
        || bytes[2] != b':'
        || bytes[5] != b':'
        || bytes[8] != b'Z'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 2 | 5 | 8) || byte.is_ascii_digit())
    {
        return Err(TerrainError::invalid(
            "LandXML document time",
            "time must use HH:MM:SSZ",
        ));
    }
    let hour = parse_decimal(&bytes[0..2]);
    let minute = parse_decimal(&bytes[3..5]);
    let second = parse_decimal(&bytes[6..8]);
    if hour > 23 || minute > 59 || second > 59 {
        return Err(TerrainError::invalid(
            "LandXML document time",
            "time is outside the UTC clock range",
        ));
    }
    Ok(())
}

fn parse_decimal(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0_u32, |value, byte| {
        value * 10 + u32::from(byte.saturating_sub(b'0'))
    })
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn escaped_len(value: &str) -> Result<usize, TerrainError> {
    value.chars().try_fold(0_usize, |total, character| {
        let added = match character {
            '&' | '\t' | '\n' | '\r' => 5,
            '<' | '>' => 4,
            '"' | '\'' => 6,
            _ => character.len_utf8(),
        };
        total
            .checked_add(added)
            .ok_or_else(|| TerrainError::numeric("escaped LandXML text length overflowed"))
    })
}

struct BoundedHashWriter {
    file: File,
    hasher: Hasher,
    bytes: u64,
    limits: LandXmlLimits,
}

impl BoundedHashWriter {
    fn new(file: File, limits: LandXmlLimits) -> Self {
        Self {
            file,
            hasher: Hasher::new(),
            bytes: 0,
            limits,
        }
    }

    fn finish(self) -> (File, ContentHash, u64) {
        (
            self.file,
            ContentHash::new(*self.hasher.finalize().as_bytes()),
            self.bytes,
        )
    }
}

impl Write for BoundedHashWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let required = self
            .bytes
            .saturating_add(u64::try_from(buffer.len()).unwrap_or(u64::MAX));
        for (name, allowed) in [
            ("LandXML output bytes", self.limits.max_output_bytes()),
            ("LandXML staging bytes", self.limits.max_staging_bytes()),
        ] {
            if required > allowed {
                return Err(io::Error::other(OutputLimit {
                    name,
                    required,
                    allowed,
                }));
            }
        }
        let written = self.file.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[derive(Debug)]
struct OutputLimit {
    name: &'static str,
    required: u64,
    allowed: u64,
}

impl fmt::Display for OutputLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} requires {} bytes; limit is {}",
            self.name, self.required, self.allowed
        )
    }
}

impl std::error::Error for OutputLimit {}

#[derive(Eq, PartialEq)]
struct FileFacts {
    bytes: u64,
    hash: ContentHash,
}

struct StageGuard {
    path: Option<PathBuf>,
}

impl StageGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("a live stage has a path")
    }

    fn remove(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        fs::remove_file(path)?;
        self.path = None;
        Ok(())
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

struct StackToken {
    bytes: [u8; STACK_TOKEN_BYTES],
    len: usize,
}

impl StackToken {
    const fn new() -> Self {
        Self {
            bytes: [0; STACK_TOKEN_BYTES],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len])
            .expect("formatted LandXML tokens are valid UTF-8")
    }
}

impl fmt::Write for StackToken {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len.checked_add(value.len()).ok_or(fmt::Error)?;
        let destination = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use foundation_runtime::ProgressPhase;
    use point_contracts::{
        AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
        AttributeValues, CoordinateReference, PositionTransform,
    };
    use point_index::{PrepareLimits, prepare};
    use point_workspace::{OpenLimits, WorkspaceSchema, create};
    use source_memory::MemorySource;

    use super::*;
    use crate::{TerrainLimits, TerrainRecipe};

    const GROUND: u8 = 2;

    #[derive(Clone, Copy)]
    enum TestAction {
        FailAt(PublicationBoundary),
        CancelBeforeLink,
    }

    struct TestPublicationHook(TestAction);

    impl PublicationHook for TestPublicationHook {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            control: &OperationControl,
        ) -> io::Result<()> {
            match self.0 {
                TestAction::FailAt(expected) if boundary == expected => Err(io::Error::other(
                    format!("injected publication failure at {boundary:?}"),
                )),
                TestAction::CancelBeforeLink if boundary == PublicationBoundary::BeforeLink => {
                    control.cancel();
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }

    struct CorruptTargetHook(PathBuf);

    impl PublicationHook for CorruptTargetHook {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            _control: &OperationControl,
        ) -> io::Result<()> {
            if boundary == PublicationBoundary::TargetVerification {
                fs::write(&self.0, b"corrupted after publication")?;
            }
            Ok(())
        }
    }

    #[test]
    fn pre_link_failure_and_cancellation_publish_nothing_and_clean_the_stage() {
        let fixture = ExportFixture::new("pre-link");
        let failed_target = fixture.path("failed.xml");
        let failed_control = OperationControl::new();
        let failure = publish(
            &fixture.surface,
            &failed_target,
            &options(),
            LandXmlLimits::default(),
            &failed_control,
            &TestPublicationHook(TestAction::FailAt(PublicationBoundary::BeforeLink)),
        )
        .expect_err("injected pre-link failure returns no receipt");
        assert!(matches!(failure, TerrainError::Io { .. }));
        assert!(!failed_target.exists());
        assert_ne!(failed_control.progress().phase(), ProgressPhase::COMPLETE);
        fixture.assert_no_stages();

        let cancelled_target = fixture.path("cancelled.xml");
        let cancelled_control = OperationControl::new();
        let cancellation = publish(
            &fixture.surface,
            &cancelled_target,
            &options(),
            LandXmlLimits::default(),
            &cancelled_control,
            &TestPublicationHook(TestAction::CancelBeforeLink),
        )
        .expect_err("injected pre-link cancellation returns no receipt");
        assert!(matches!(cancellation, TerrainError::Cancelled));
        assert!(!cancelled_target.exists());
        assert_ne!(
            cancelled_control.progress().phase(),
            ProgressPhase::COMPLETE
        );
        fixture.assert_no_stages();
    }

    #[test]
    fn every_post_link_boundary_is_indeterminate_and_never_returns_a_receipt() {
        let fixture = ExportFixture::new("post-link");
        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalProgress,
        ] {
            let target = fixture.path(&format!("{boundary:?}.xml"));
            let control = OperationControl::new();
            let failure = publish(
                &fixture.surface,
                &target,
                &options(),
                LandXmlLimits::default(),
                &control,
                &TestPublicationHook(TestAction::FailAt(boundary)),
            )
            .expect_err("post-link failure returns no receipt");
            let TerrainError::ExportIndeterminate { expected_hash } = failure else {
                panic!("{boundary:?} must be indeterminate after the target link");
            };
            let bytes = fs::read(&target).expect("post-link target remains inspectable");
            assert_eq!(
                expected_hash,
                ContentHash::new(*blake3::hash(&bytes).as_bytes())
            );
            assert_ne!(control.progress().phase(), ProgressPhase::COMPLETE);
            fixture.assert_no_stages();
        }
    }

    #[test]
    fn verification_mismatch_is_indeterminate_and_preserves_the_target() {
        let fixture = ExportFixture::new("verification-mismatch");
        let target = fixture.path("corrupted.xml");
        let control = OperationControl::new();
        let failure = publish(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &control,
            &CorruptTargetHook(target.clone()),
        )
        .expect_err("post-link verification mismatch returns no receipt");
        let TerrainError::ExportIndeterminate { expected_hash } = failure else {
            panic!("post-link verification mismatch must be indeterminate");
        };
        let published = fs::read(&target).expect("indeterminate target remains inspectable");
        assert_ne!(
            expected_hash,
            ContentHash::new(*blake3::hash(&published).as_bytes())
        );
        assert_ne!(control.progress().phase(), ProgressPhase::COMPLETE);
        fixture.assert_no_stages();
    }

    fn options() -> LandXmlOptions {
        LandXmlOptions::metric_metres("Fault Fixture", "2026-08-10", "12:34:56Z")
            .expect("fault fixture options are valid")
            .allow_unknown_coordinate_reference_as_metric_metres()
    }

    struct ExportFixture {
        directory: PathBuf,
        surface: TerrainSurface,
    }

    impl ExportFixture {
        fn new(label: &str) -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "punctra-landxml-fault-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&directory).expect("create isolated fault-test directory");

            let ticks = vec![[0, 0, 0], [10, 0, 10], [0, 10, 20]];
            let definition = AttributeDefinition::new(
                AttributeId::new(701).expect("fixture Attribute identity is nonzero"),
                "classification",
                AttributeDataType::U8,
            )
            .expect("fixture classification definition is valid");
            let column =
                AttributeColumn::new(definition, AttributeValues::u8(vec![GROUND; ticks.len()]))
                    .expect("fixture classification column is valid");
            let columns = AttributeColumns::new(vec![column], ticks.len())
                .expect("fixture Attribute rows align");
            let transform =
                PositionTransform::new([0.0; 3], [1.0; 3]).expect("fixture transform is valid");
            let memory =
                MemorySource::from_columns(transform, CoordinateReference::Unknown, ticks, columns)
                    .expect("fixture Source is valid");
            let source = source_memory::open(memory)
                .blocking_wait()
                .expect("fixture Source opens");
            let index = prepare(
                source,
                directory.join("fixture.pidx"),
                PrepareLimits::default(),
            )
            .blocking_wait()
            .expect("fixture index prepares");
            let workspace = create(
                directory.join("fixture.pcw"),
                index,
                WorkspaceSchema::new(AttributeId::new(701).unwrap()),
                OpenLimits::default(),
            )
            .blocking_wait()
            .expect("fixture Workspace creates");
            let surface = crate::derive(
                workspace.head(),
                TerrainRecipe::new(GROUND),
                TerrainLimits::default(),
            )
            .blocking_wait()
            .expect("fixture Terrain derives");
            Self { directory, surface }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.join(name)
        }

        fn assert_no_stages(&self) {
            let stages = fs::read_dir(&self.directory)
                .expect("read fault-test directory")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .filter(|name| name.to_string_lossy().starts_with(".punctra-landxml-"))
                .collect::<Vec<_>>();
            assert!(stages.is_empty(), "staging files remain: {stages:?}");
        }
    }

    impl Drop for ExportFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
