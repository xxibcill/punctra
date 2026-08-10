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
    LandXmlDisposition, LandXmlJob, LandXmlLimits, LandXmlReceipt, TerrainError, TerrainSurface,
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

pub(crate) fn start_ensure(
    surface: &TerrainSurface,
    target: impl AsRef<Path>,
    options: LandXmlOptions,
    limits: LandXmlLimits,
) -> LandXmlJob {
    let surface = surface.clone();
    let target = target.as_ref().to_path_buf();
    Job::spawn(move |control| {
        ensure(
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

    let mut prepared = prepare_export(surface, target, options, limits, control)?;
    hook.reach(PublicationBoundary::BeforeLink, control)
        .map_err(|error| {
            TerrainError::io("run LandXML pre-link boundary", target.display(), error)
        })?;
    control.check_cancelled()?;
    prepared.stage.verify().map_err(|error| {
        TerrainError::io(
            "verify LandXML stage before publication",
            prepared.stage.path().display(),
            error,
        )
    })?;
    // Once this create-new link succeeds, every remaining failure is
    // indeterminate because a complete target may be durably observable.
    publish_target(prepared.stage.path(), target)?;
    finish_publication(
        surface,
        &mut prepared.stage,
        &prepared.expected,
        control,
        hook,
        PublicationCompletion {
            target,
            buffer_bytes: prepared.buffer_bytes,
            limits,
            total_progress: prepared.total_progress,
            disposition: LandXmlDisposition::Created,
        },
    )
}

fn ensure<H: PublicationHook>(
    surface: &TerrainSurface,
    target: &Path,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
    control: &OperationControl,
    hook: &H,
) -> Result<LandXmlReceipt, TerrainError> {
    control.check_cancelled()?;
    validate_export(surface, target, options, limits)?;
    let mut prepared = prepare_export(surface, target, options, limits, control)?;
    match fs::symlink_metadata(target) {
        Ok(metadata) => reconcile_existing(surface, target, &metadata, prepared, control, hook),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            hook.reach(PublicationBoundary::BeforeLink, control)
                .map_err(|error| {
                    TerrainError::io("run LandXML pre-link boundary", target.display(), error)
                })?;
            control.check_cancelled()?;
            prepared.stage.verify().map_err(|error| {
                TerrainError::io(
                    "verify LandXML stage before publication",
                    prepared.stage.path().display(),
                    error,
                )
            })?;
            match fs::hard_link(prepared.stage.path(), target) {
                Ok(()) => finish_publication(
                    surface,
                    &mut prepared.stage,
                    &prepared.expected,
                    control,
                    hook,
                    PublicationCompletion {
                        target,
                        buffer_bytes: prepared.buffer_bytes,
                        limits,
                        total_progress: prepared.total_progress,
                        disposition: LandXmlDisposition::Created,
                    },
                ),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(target).map_err(|error| {
                        TerrainError::io("inspect raced LandXML target", target.display(), error)
                    })?;
                    reconcile_existing(surface, target, &metadata, prepared, control, hook)
                }
                Err(error) => Err(TerrainError::io(
                    "publish LandXML target",
                    target.display(),
                    error,
                )),
            }
        }
        Err(error) => Err(TerrainError::io(
            "inspect LandXML target",
            target.display(),
            error,
        )),
    }
}

struct PreparedExport<'a> {
    stage: StageGuard,
    expected: FileFacts,
    target: &'a Path,
    buffer_bytes: usize,
    total_progress: u64,
    limits: LandXmlLimits,
}

fn prepare_export<'a>(
    surface: &TerrainSurface,
    target: &'a Path,
    options: &LandXmlOptions,
    limits: LandXmlLimits,
    control: &OperationControl,
) -> Result<PreparedExport<'a>, TerrainError> {
    let parent_path = target_parent(target);
    let parent = DirectoryWitness::capture(parent_path).map_err(|error| {
        TerrainError::io(
            "capture LandXML parent directory",
            parent_path.display(),
            error,
        )
    })?;
    let buffer_bytes = choose_buffer_bytes(limits)?;
    let (stage, stage_file) = create_stage(parent)?;
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
    stage.verify().map_err(|error| {
        TerrainError::io(
            "verify LandXML stage ownership",
            stage.path().display(),
            error,
        )
    })?;
    let stage_metadata = stage.metadata();
    let verified =
        verify_existing_regular_file(stage.path(), stage_metadata, buffer_bytes, limits, control)?;
    stage.verify().map_err(|error| {
        TerrainError::io(
            "reverify LandXML stage ownership",
            stage.path().display(),
            error,
        )
    })?;
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
    Ok(PreparedExport {
        stage,
        expected,
        target,
        buffer_bytes,
        total_progress,
        limits,
    })
}

#[derive(Clone, Copy)]
struct PublicationCompletion<'a> {
    target: &'a Path,
    buffer_bytes: usize,
    limits: LandXmlLimits,
    total_progress: u64,
    disposition: LandXmlDisposition,
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
    let published = verify_linked_target(
        stage,
        completion.target,
        completion.buffer_bytes,
        completion.limits,
        control,
    )
    .map_err(|_| TerrainError::ExportIndeterminate {
        expected_hash: expected.hash,
    })?;
    if published != *expected {
        remove_mismatched_target_if_owned(stage, completion.target);
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
    if stage.sync_parent().is_err() {
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
    stage
        .sync_parent()
        .map_err(|_| TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        })?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::TerminalProgress,
        control,
        expected.hash,
    )?;
    stage
        .verify_parent()
        .map_err(|_| TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        })?;
    control
        .complete_progress(completion.total_progress)
        .map_err(|_| TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        })?;
    stage
        .verify_parent()
        .map_err(|_| TerrainError::ExportIndeterminate {
            expected_hash: expected.hash,
        })?;
    Ok(receipt(surface, expected, completion.disposition))
}

#[cfg(unix)]
fn remove_mismatched_target_if_owned(stage: &StageGuard, target: &Path) {
    let Ok(stage_metadata) = stage.verified_metadata() else {
        return;
    };
    let Ok(target_metadata) = fs::symlink_metadata(target) else {
        return;
    };
    if stage_metadata.file_type().is_file()
        && target_metadata.file_type().is_file()
        && same_file_identity(&stage_metadata, &target_metadata)
        && fs::remove_file(target).is_ok()
    {
        let _ = stage.sync_parent();
    }
}

#[cfg(not(unix))]
fn remove_mismatched_target_if_owned(_stage: &StageGuard, _target: &Path) {}

fn reconcile_existing(
    surface: &TerrainSurface,
    target: &Path,
    metadata: &fs::Metadata,
    mut prepared: PreparedExport<'_>,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<LandXmlReceipt, TerrainError> {
    require_reconciliation_boundary(
        hook,
        PublicationBoundary::TargetVerification,
        control,
        target,
    )?;
    prepared.stage.verify_parent().map_err(|error| {
        TerrainError::io(
            "verify reconciled LandXML parent directory",
            prepared.target.display(),
            error,
        )
    })?;
    let actual = verify_existing_regular_file(
        target,
        metadata,
        prepared.buffer_bytes,
        prepared.limits,
        control,
    )?;
    if actual != prepared.expected {
        return Err(TerrainError::ExportConflict {
            path: crate::TerrainDiagnostic::new(target.display().to_string()),
            expected_hash: prepared.expected.hash,
            actual_hash: actual.hash,
        });
    }
    require_reconciliation_boundary(hook, PublicationBoundary::ParentSync, control, target)?;
    prepared.stage.sync_parent().map_err(|error| {
        TerrainError::io(
            "sync reconciled LandXML parent directory",
            target_parent(prepared.target).display(),
            error,
        )
    })?;
    require_reconciliation_boundary(hook, PublicationBoundary::StageRemoval, control, target)?;
    let stage_path = prepared.stage.path().to_path_buf();
    prepared.stage.remove().map_err(|error| {
        TerrainError::io(
            "remove reconciled LandXML stage",
            stage_path.display(),
            error,
        )
    })?;
    require_reconciliation_boundary(hook, PublicationBoundary::CleanupSync, control, target)?;
    prepared.stage.sync_parent().map_err(|error| {
        TerrainError::io(
            "sync reconciled LandXML cleanup",
            target_parent(prepared.target).display(),
            error,
        )
    })?;
    require_reconciliation_boundary(hook, PublicationBoundary::TerminalProgress, control, target)?;
    prepared.stage.verify_parent().map_err(|error| {
        TerrainError::io(
            "verify reconciled LandXML parent directory",
            target_parent(prepared.target).display(),
            error,
        )
    })?;
    control.complete_progress(prepared.total_progress)?;
    prepared.stage.verify_parent().map_err(|error| {
        TerrainError::io(
            "reverify reconciled LandXML parent directory",
            target_parent(prepared.target).display(),
            error,
        )
    })?;
    Ok(receipt(
        surface,
        &prepared.expected,
        LandXmlDisposition::ReconciledExisting,
    ))
}

fn require_reconciliation_boundary(
    hook: &impl PublicationHook,
    boundary: PublicationBoundary,
    control: &OperationControl,
    target: &Path,
) -> Result<(), TerrainError> {
    hook.reach(boundary, control).map_err(|error| {
        TerrainError::io(
            "run LandXML reconciliation boundary",
            target.display(),
            error,
        )
    })?;
    Ok(control.check_cancelled()?)
}

fn receipt(
    surface: &TerrainSurface,
    expected: &FileFacts,
    disposition: LandXmlDisposition,
) -> LandXmlReceipt {
    LandXmlReceipt::new(
        disposition,
        surface.descriptor().artifact_hash(),
        surface.descriptor().geometry_hash(),
        surface.descriptor().topology_hash(),
        expected.hash,
        expected.bytes,
        surface.descriptor().vertex_count(),
        surface.descriptor().face_count(),
    )
}

fn require_post_link_boundary<H: PublicationHook>(
    hook: &H,
    boundary: PublicationBoundary,
    control: &OperationControl,
    expected_hash: ContentHash,
) -> Result<(), TerrainError> {
    hook.reach(boundary, control)
        .map_err(|_| TerrainError::ExportIndeterminate { expected_hash })?;
    control
        .check_cancelled()
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

fn create_stage(parent: DirectoryWitness) -> Result<(StageGuard, File), TerrainError> {
    for _ in 0..64 {
        parent.verify().map_err(|error| {
            TerrainError::io(
                "verify LandXML parent directory",
                parent.path().display(),
                error,
            )
        })?;
        let sequence = NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.path().join(format!(
            ".punctra-landxml-{}-{sequence}.stage",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                let metadata = file.metadata().map_err(|error| {
                    TerrainError::io("inspect open LandXML stage", path.display(), error)
                })?;
                let stage = StageGuard::new(path, metadata, parent);
                stage.verify().map_err(|error| {
                    TerrainError::io(
                        "verify created LandXML stage",
                        stage.path().display(),
                        error,
                    )
                })?;
                return Ok((stage, file));
            }
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

fn verify_linked_target(
    stage: &StageGuard,
    target: &Path,
    buffer_bytes: usize,
    limits: LandXmlLimits,
    control: &OperationControl,
) -> Result<FileFacts, TerrainError> {
    let stage_before = stage.verified_metadata().map_err(|error| {
        TerrainError::io(
            "inspect linked LandXML stage",
            stage.path().display(),
            error,
        )
    })?;
    let target_before = fs::symlink_metadata(target).map_err(|error| {
        TerrainError::io("inspect linked LandXML target", target.display(), error)
    })?;
    require_regular_target(&target_before)?;
    if !same_file_identity(&stage_before, &target_before) {
        return Err(changed_target_error());
    }
    let facts =
        verify_existing_regular_file(target, &target_before, buffer_bytes, limits, control)?;
    let stage_after = stage.verified_metadata().map_err(|error| {
        TerrainError::io(
            "reinspect linked LandXML stage",
            stage.path().display(),
            error,
        )
    })?;
    let target_after = fs::symlink_metadata(target).map_err(|error| {
        TerrainError::io("reinspect linked LandXML target", target.display(), error)
    })?;
    require_stable_target(&stage_before, &stage_after, &target_after)?;
    if !same_file_state(&stage_before, &stage_after) || stage_after.len() != facts.bytes {
        return Err(changed_target_error());
    }
    Ok(facts)
}

fn verify_existing_regular_file(
    path: &Path,
    initial_metadata: &fs::Metadata,
    buffer_bytes: usize,
    limits: LandXmlLimits,
    control: &OperationControl,
) -> Result<FileFacts, TerrainError> {
    require_regular_target(initial_metadata)?;
    let mut file = File::open(path)
        .map_err(|error| TerrainError::io("open existing LandXML target", path.display(), error))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| TerrainError::io("inspect open LandXML target", path.display(), error))?;
    require_regular_target(&opened_metadata)?;
    let current_metadata = fs::symlink_metadata(path)
        .map_err(|error| TerrainError::io("reinspect LandXML target", path.display(), error))?;
    require_stable_target(initial_metadata, &opened_metadata, &current_metadata)?;

    let facts = verify_open_file(&mut file, path, buffer_bytes, limits, Some(control))?;
    let verified_metadata = file.metadata().map_err(|error| {
        TerrainError::io("reinspect open LandXML target", path.display(), error)
    })?;
    let final_metadata = fs::symlink_metadata(path).map_err(|error| {
        TerrainError::io("reinspect verified LandXML target", path.display(), error)
    })?;
    require_stable_target(&opened_metadata, &verified_metadata, &final_metadata)?;
    if !same_file_state(&opened_metadata, &verified_metadata) {
        return Err(changed_target_error());
    }
    if final_metadata.len() != facts.bytes {
        return Err(changed_target_error());
    }
    Ok(facts)
}

fn verify_open_file(
    file: &mut File,
    path: &Path,
    buffer_bytes: usize,
    limits: LandXmlLimits,
    control: Option<&OperationControl>,
) -> Result<FileFacts, TerrainError> {
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

fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && matches!(
            (left.modified(), right.modified()),
            (Ok(left_modified), Ok(right_modified)) if left_modified == right_modified
        )
}

fn require_regular_target(metadata: &fs::Metadata) -> Result<(), TerrainError> {
    if metadata.file_type().is_file() {
        return Ok(());
    }
    Err(TerrainError::invalid(
        "LandXML target",
        "an existing target must be a regular non-symlink file",
    ))
}

fn require_stable_target(
    initial: &fs::Metadata,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> Result<(), TerrainError> {
    require_regular_target(current)?;
    if same_file_identity(initial, opened) && same_file_identity(opened, current) {
        return Ok(());
    }
    Err(changed_target_error())
}

fn changed_target_error() -> TerrainError {
    TerrainError::invalid(
        "LandXML target",
        "the existing target changed during verification",
    )
}

#[cfg(unix)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.file_attributes() == right.file_attributes()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
        && left.file_size() == right.file_size()
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

#[cfg(not(unix))]
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

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
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

struct DirectoryWitness {
    path: PathBuf,
    metadata: fs::Metadata,
    #[cfg(unix)]
    directory: File,
}

impl DirectoryWitness {
    fn capture(path: &Path) -> io::Result<Self> {
        let initial = fs::symlink_metadata(path)?;
        require_directory(&initial)?;
        #[cfg(unix)]
        {
            let directory = File::open(path)?;
            let opened = directory.metadata()?;
            let current = fs::symlink_metadata(path)?;
            require_directory(&opened)?;
            require_directory(&current)?;
            if !same_directory_identity(&initial, &opened)
                || !same_directory_identity(&opened, &current)
            {
                return Err(changed_parent_error());
            }
            Ok(Self {
                path: path.to_path_buf(),
                metadata: opened,
                directory,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                path: path.to_path_buf(),
                metadata: initial,
            })
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn verify(&self) -> io::Result<()> {
        let current = fs::symlink_metadata(&self.path)?;
        require_directory(&current)?;
        if !same_directory_identity(&self.metadata, &current) {
            return Err(changed_parent_error());
        }
        #[cfg(unix)]
        {
            let opened = self.directory.metadata()?;
            require_directory(&opened)?;
            if !same_directory_identity(&self.metadata, &opened) {
                return Err(changed_parent_error());
            }
        }
        Ok(())
    }

    fn sync(&self) -> io::Result<()> {
        self.verify()?;
        #[cfg(unix)]
        self.directory.sync_all()?;
        #[cfg(not(unix))]
        sync_directory(&self.path)?;
        self.verify()
    }
}

fn require_directory(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LandXML parent must remain a non-symlink directory",
        ))
    }
}

fn changed_parent_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "LandXML parent directory identity changed",
    )
}

#[cfg(unix)]
fn same_directory_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    same_file_identity(left, right)
}

#[cfg(windows)]
fn same_directory_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.file_attributes() == right.file_attributes()
        && left.creation_time() == right.creation_time()
}

#[cfg(not(any(unix, windows)))]
fn same_directory_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

struct StageGuard {
    path: Option<PathBuf>,
    metadata: fs::Metadata,
    parent: DirectoryWitness,
}

impl StageGuard {
    fn new(path: PathBuf, metadata: fs::Metadata, parent: DirectoryWitness) -> Self {
        Self {
            path: Some(path),
            metadata,
            parent,
        }
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("a live stage has a path")
    }

    fn metadata(&self) -> &fs::Metadata {
        &self.metadata
    }

    fn verify_parent(&self) -> io::Result<()> {
        self.parent.verify()
    }

    fn sync_parent(&self) -> io::Result<()> {
        self.parent.sync()
    }

    fn verify(&self) -> io::Result<()> {
        self.verified_metadata().map(|_| ())
    }

    fn verified_metadata(&self) -> io::Result<fs::Metadata> {
        self.parent.verify()?;
        let current = fs::symlink_metadata(self.path())?;
        if !current.file_type().is_file() || !same_file_identity(&self.metadata, &current) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LandXML stage identity changed",
            ));
        }
        Ok(current)
    }

    fn remove(&mut self) -> io::Result<()> {
        if self.path.is_none() {
            return self.parent.verify();
        }
        self.verify()?;
        fs::remove_file(self.path())?;
        self.path = None;
        self.parent.verify()
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = self.remove();
        if self.path.is_none() {
            let _ = self.sync_parent();
        }
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
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
        CancelAt(PublicationBoundary),
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
                TestAction::CancelAt(expected) if boundary == expected => {
                    control.cancel();
                    Ok(())
                }
                _ => Ok(()),
            }
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
            &TestPublicationHook(TestAction::CancelAt(PublicationBoundary::BeforeLink)),
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
    fn every_post_link_boundary_observes_cancellation_as_indeterminate() {
        let fixture = ExportFixture::new("post-link-cancellation");
        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalProgress,
        ] {
            let target = fixture.path(&format!("cancel-{boundary:?}.xml"));
            let control = OperationControl::new();
            let failure = ensure(
                &fixture.surface,
                &target,
                &options(),
                LandXmlLimits::default(),
                &control,
                &TestPublicationHook(TestAction::CancelAt(boundary)),
            )
            .expect_err("post-link cancellation cannot acknowledge an ensure receipt");
            let TerrainError::ExportIndeterminate { expected_hash } = failure else {
                panic!("{boundary:?} cancellation must be indeterminate after the target link");
            };
            assert!(control.check_cancelled().is_err());
            assert_ne!(control.progress().phase(), ProgressPhase::COMPLETE);
            assert_eq!(
                expected_hash,
                ContentHash::new(*blake3::hash(&fs::read(&target).unwrap()).as_bytes())
            );

            let recovered = ensure(
                &fixture.surface,
                &target,
                &options(),
                LandXmlLimits::default(),
                &OperationControl::new(),
                &ProductionPublicationHook,
            )
            .expect("retry reconciles the complete target after cancellation");
            assert_eq!(
                recovered.disposition(),
                LandXmlDisposition::ReconciledExisting
            );
            fixture.assert_no_stages();
        }
    }

    #[test]
    fn ensure_reconciles_an_exact_create_race_and_rejects_a_conflicting_race() {
        let fixture = ExportFixture::new("create-race");
        let seed = fixture.path("seed.xml");
        publish(
            &fixture.surface,
            &seed,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("seed export succeeds");
        let exact = fs::read(&seed).expect("seed bytes are readable");

        let exact_target = fixture.path("exact-race.xml");
        let exact_receipt = ensure(
            &fixture.surface,
            &exact_target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &CreateTargetBeforeLink::new(&exact_target, exact.clone()),
        )
        .expect("an exact raced target reconciles");
        assert_eq!(
            exact_receipt.disposition(),
            LandXmlDisposition::ReconciledExisting
        );
        assert_eq!(fs::read(&exact_target).unwrap(), exact);

        let conflicting_target = fixture.path("conflicting-race.xml");
        let conflicting_bytes = b"raced caller-owned target".to_vec();
        let error = ensure(
            &fixture.surface,
            &conflicting_target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &CreateTargetBeforeLink::new(&conflicting_target, conflicting_bytes.clone()),
        )
        .expect_err("a conflicting raced target fails closed");
        assert!(matches!(error, TerrainError::ExportConflict { .. }));
        assert_eq!(fs::read(&conflicting_target).unwrap(), conflicting_bytes);
        fixture.assert_no_stages();
    }

    #[test]
    fn ensure_post_link_failures_are_indeterminate_and_retry_to_reconciliation() {
        let fixture = ExportFixture::new("ensure-post-link");
        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalProgress,
        ] {
            let target = fixture.path(&format!("ensure-{boundary:?}.xml"));
            let failure = ensure(
                &fixture.surface,
                &target,
                &options(),
                LandXmlLimits::default(),
                &OperationControl::new(),
                &TestPublicationHook(TestAction::FailAt(boundary)),
            )
            .expect_err("a post-link ensure failure has no receipt");
            let TerrainError::ExportIndeterminate { expected_hash } = failure else {
                panic!("{boundary:?} must remain indeterminate after the create-new link");
            };
            assert_eq!(
                expected_hash,
                ContentHash::new(*blake3::hash(&fs::read(&target).unwrap()).as_bytes())
            );

            let recovered = ensure(
                &fixture.surface,
                &target,
                &options(),
                LandXmlLimits::default(),
                &OperationControl::new(),
                &ProductionPublicationHook,
            )
            .expect("retry reconciles the complete target");
            assert_eq!(
                recovered.disposition(),
                LandXmlDisposition::ReconciledExisting
            );
            fixture.assert_no_stages();
        }
    }

    #[test]
    fn a_post_link_path_replacement_is_never_deleted_as_failed_publication_cleanup() {
        let fixture = ExportFixture::new("post-link-replacement");
        let target = fixture.path("raced.xml");
        let replacement = b"caller-owned post-link replacement".to_vec();

        let failure = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ReplaceTargetAtVerification::new(&target, replacement.clone()),
        )
        .expect_err("the raced publication has no receipt");

        assert!(matches!(failure, TerrainError::ExportIndeterminate { .. }));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        fixture.assert_no_stages();
    }

    #[cfg(unix)]
    #[test]
    fn an_exact_post_link_symlink_replacement_is_indeterminate_and_preserved() {
        let fixture = ExportFixture::new("post-link-symlink");
        let seed = fixture.path("seed.xml");
        publish(
            &fixture.surface,
            &seed,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("seed export succeeds");
        let target = fixture.path("raced.xml");

        let failure = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ReplaceTargetWithSymlinkAtVerification::new(&target, &seed),
        )
        .expect_err("a symlink cannot satisfy ownership of the published target");

        assert!(matches!(failure, TerrainError::ExportIndeterminate { .. }));
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), fs::read(&seed).unwrap());
        fixture.assert_no_stages();
    }

    #[test]
    fn parent_replacement_before_link_never_publishes_into_the_replacement() {
        let fixture = ExportFixture::new("parent-before-link");
        let target = fixture.path("terrain.xml");
        let replacement =
            ParentReplacement::new(&fixture.directory, &target, PublicationBoundary::BeforeLink);

        let failure = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &replacement,
        )
        .expect_err("a replaced parent cannot receive the target link");

        assert!(matches!(failure, TerrainError::Io { .. }));
        assert!(!target.exists());
        replacement.restore();
        fixture.remove_stages();
        fixture.assert_no_stages();
    }

    #[test]
    fn parent_replacement_after_link_is_indeterminate_even_with_exact_mirrors() {
        let fixture = ExportFixture::new("parent-after-link");
        let target = fixture.path("terrain.xml");
        let replacement = ParentReplacement::new(
            &fixture.directory,
            &target,
            PublicationBoundary::TargetVerification,
        );

        let failure = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &replacement,
        )
        .expect_err("a post-link parent replacement cannot produce a receipt");

        assert!(matches!(failure, TerrainError::ExportIndeterminate { .. }));
        replacement.restore();
        assert!(target.is_file());
        fixture.remove_stages();
        fixture.assert_no_stages();
    }

    #[test]
    fn reconciliation_rejects_parent_replacement_with_exact_inode_mirrors() {
        let fixture = ExportFixture::new("reconcile-parent");
        let target = fixture.path("terrain.xml");
        publish(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("seed export succeeds");
        let exact = fs::read(&target).unwrap();
        let replacement = ParentReplacement::new(
            &fixture.directory,
            &target,
            PublicationBoundary::TargetVerification,
        );

        let failure = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &replacement,
        )
        .expect_err("reconciliation is bound to the witnessed parent directory");

        assert!(matches!(failure, TerrainError::Io { .. }));
        assert_eq!(fs::read(&target).unwrap(), exact);
        replacement.restore();
        assert_eq!(fs::read(&target).unwrap(), exact);
        fixture.remove_stages();
        fixture.assert_no_stages();
    }

    #[test]
    fn reconciliation_faults_and_cancellation_never_change_the_exact_target() {
        let fixture = ExportFixture::new("reconcile-faults");
        let target = fixture.path("existing.xml");
        publish(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("seed export succeeds");
        let exact = fs::read(&target).expect("seed bytes are readable");

        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalProgress,
        ] {
            let failure = ensure(
                &fixture.surface,
                &target,
                &options(),
                LandXmlLimits::default(),
                &OperationControl::new(),
                &TestPublicationHook(TestAction::FailAt(boundary)),
            )
            .expect_err("injected reconciliation fault has no receipt");
            assert!(matches!(failure, TerrainError::Io { .. }));
            assert_eq!(fs::read(&target).unwrap(), exact);
            fixture.assert_no_stages();
        }

        let cancellation = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &TestPublicationHook(TestAction::CancelAt(
                PublicationBoundary::TargetVerification,
            )),
        )
        .expect_err("reconciliation cancellation has no receipt");
        assert!(matches!(cancellation, TerrainError::Cancelled));
        assert_eq!(fs::read(&target).unwrap(), exact);
        fixture.assert_no_stages();
    }

    #[test]
    fn reconciliation_rejects_a_same_length_inode_replacement_during_verification() {
        let fixture = ExportFixture::new("reconcile-replacement");
        let target = fixture.path("existing.xml");
        publish(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("seed export succeeds");
        let exact_length = usize::try_from(fs::metadata(&target).unwrap().len())
            .expect("test export length fits usize");
        let replacement = vec![b'X'; exact_length];

        let failure = ensure(
            &fixture.surface,
            &target,
            &options(),
            LandXmlLimits::default(),
            &OperationControl::new(),
            &ReplaceTargetAtVerification::new(&target, replacement.clone()),
        )
        .expect_err("a path identity change fails closed even at the same byte length");

        assert!(matches!(failure, TerrainError::InvalidArgument { .. }));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        fixture.assert_no_stages();
    }

    struct CreateTargetBeforeLink {
        target: PathBuf,
        bytes: Vec<u8>,
    }

    impl CreateTargetBeforeLink {
        fn new(target: &Path, bytes: Vec<u8>) -> Self {
            Self {
                target: target.to_path_buf(),
                bytes,
            }
        }
    }

    impl PublicationHook for CreateTargetBeforeLink {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            _control: &OperationControl,
        ) -> io::Result<()> {
            if boundary == PublicationBoundary::BeforeLink {
                fs::write(&self.target, &self.bytes)?;
            }
            Ok(())
        }
    }

    struct ReplaceTargetAtVerification {
        target: PathBuf,
        bytes: Vec<u8>,
    }

    #[cfg(unix)]
    struct ReplaceTargetWithSymlinkAtVerification {
        target: PathBuf,
        exact: PathBuf,
    }

    #[cfg(unix)]
    impl ReplaceTargetWithSymlinkAtVerification {
        fn new(target: &Path, exact: &Path) -> Self {
            Self {
                target: target.to_path_buf(),
                exact: exact.to_path_buf(),
            }
        }
    }

    #[cfg(unix)]
    impl PublicationHook for ReplaceTargetWithSymlinkAtVerification {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            _control: &OperationControl,
        ) -> io::Result<()> {
            use std::os::unix::fs::symlink;

            if boundary == PublicationBoundary::TargetVerification {
                fs::remove_file(&self.target)?;
                symlink(&self.exact, &self.target)?;
            }
            Ok(())
        }
    }

    struct ParentReplacement {
        original: PathBuf,
        moved: PathBuf,
        target_name: std::ffi::OsString,
        boundary: PublicationBoundary,
        replaced: AtomicBool,
        restored: AtomicBool,
    }

    impl ParentReplacement {
        fn new(original: &Path, target: &Path, boundary: PublicationBoundary) -> Self {
            Self {
                original: original.to_path_buf(),
                moved: original.with_extension("moved"),
                target_name: target.file_name().unwrap().to_os_string(),
                boundary,
                replaced: AtomicBool::new(false),
                restored: AtomicBool::new(false),
            }
        }

        fn restore(&self) {
            if self.restored.load(Ordering::Relaxed) {
                return;
            }
            fs::remove_dir_all(&self.original).expect("remove replacement parent");
            fs::rename(&self.moved, &self.original).expect("restore witnessed parent");
            self.restored.store(true, Ordering::Relaxed);
        }
    }

    impl Drop for ParentReplacement {
        fn drop(&mut self) {
            if self.replaced.load(Ordering::Relaxed) && !self.restored.load(Ordering::Relaxed) {
                let _ = fs::remove_dir_all(&self.original);
                if fs::rename(&self.moved, &self.original).is_ok() {
                    self.restored.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    impl PublicationHook for ParentReplacement {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            _control: &OperationControl,
        ) -> io::Result<()> {
            if boundary != self.boundary || self.replaced.swap(true, Ordering::Relaxed) {
                return Ok(());
            }
            fs::rename(&self.original, &self.moved)?;
            fs::create_dir(&self.original)?;
            for entry in fs::read_dir(&self.moved)? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".punctra-landxml-")
                {
                    fs::copy(entry.path(), self.original.join(entry.file_name()))?;
                }
            }
            let moved_target = self.moved.join(&self.target_name);
            if moved_target.is_file() {
                fs::hard_link(moved_target, self.original.join(&self.target_name))?;
            }
            Ok(())
        }
    }

    impl ReplaceTargetAtVerification {
        fn new(target: &Path, bytes: Vec<u8>) -> Self {
            Self {
                target: target.to_path_buf(),
                bytes,
            }
        }
    }

    impl PublicationHook for ReplaceTargetAtVerification {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            _control: &OperationControl,
        ) -> io::Result<()> {
            if boundary == PublicationBoundary::TargetVerification {
                fs::remove_file(&self.target)?;
                fs::write(&self.target, &self.bytes)?;
            }
            Ok(())
        }
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

        fn remove_stages(&self) {
            for entry in fs::read_dir(&self.directory).expect("read fault-test directory") {
                let entry = entry.expect("read fault-test entry");
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".punctra-landxml-")
                {
                    fs::remove_file(entry.path()).expect("remove recovered fault-test stage");
                }
            }
        }
    }

    impl Drop for ExportFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
