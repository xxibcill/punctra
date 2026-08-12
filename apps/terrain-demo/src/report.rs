// Canonical JSON key order is intentionally expressed by one linear encoder;
// splitting it would make byte-order review harder. Limit field names mirror
// the public resource vocabulary.
#![allow(clippy::struct_field_names, clippy::too_many_lines)]

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::{fmt, fs::OpenOptions};

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::WorldBounds;
use point_terrain::{CheckPointOutcome, CheckPointReport, LandXmlReceipt, TerrainSurface};
use point_workspace::RevisionAudit;
use thiserror::Error;

use crate::{
    journal::{Digest, WorkflowRunId},
    publication::{
        DirectoryWitness, StageCreationError, StageGuard, create_stage as create_publication_stage,
        same_file_identity, sync_directory,
    },
};

const REPORT_SCHEMA: &str = "punctra.terrain-workflow.audit.v1";
const REPORT_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-report-bytes-v1";
const HASH_BUFFER_BYTES: usize = 8 * 1024;

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
    fn reach(&self, boundary: PublicationBoundary, control: &OperationControl) -> io::Result<()>;
}

struct ProductionPublicationHook;

impl PublicationHook for ProductionPublicationHook {
    fn reach(&self, _boundary: PublicationBoundary, _control: &OperationControl) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReportLimits {
    pub(crate) max_output_bytes: u64,
    pub(crate) max_staging_bytes: u64,
    pub(crate) max_write_buffer_bytes: u64,
    pub(crate) max_working_bytes: u64,
}

impl Default for ReportLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 1024 * 1024,
            max_staging_bytes: 1024 * 1024,
            max_write_buffer_bytes: 8 * 1024,
            max_working_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SurfaceChangeEnvelope {
    pub(crate) added_face_count: u64,
    pub(crate) removed_face_count: u64,
    pub(crate) added_face_hash: Digest,
    pub(crate) removed_face_hash: Digest,
    pub(crate) bounds_bits: Option<[[u64; 2]; 3]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LimitFact {
    pub(crate) name: &'static str,
    pub(crate) value: u64,
}

pub(crate) struct ReportFacts<'a> {
    pub(crate) run: WorkflowRunId,
    pub(crate) request_hash: Digest,
    pub(crate) source: Digest,
    pub(crate) workspace: [u8; 16],
    pub(crate) operation: [u8; 16],
    pub(crate) baseline_revision: Digest,
    pub(crate) changed_revision: Digest,
    pub(crate) correction_ordinals: &'a [u64],
    pub(crate) non_ground_classification: u8,
    pub(crate) ordinal_hash: Digest,
    pub(crate) recipe_hash: Digest,
    pub(crate) qa_input_hash: Digest,
    pub(crate) options_hash: Digest,
    pub(crate) semantic_results_hash: Digest,
    pub(crate) path_bindings: [Digest; 4],
    pub(crate) audit: &'a RevisionAudit,
    pub(crate) baseline: &'a TerrainSurface,
    pub(crate) changed: &'a TerrainSurface,
    pub(crate) envelope: SurfaceChangeEnvelope,
    pub(crate) qa: &'a CheckPointReport,
    pub(crate) qa_hash: Digest,
    pub(crate) landxml: LandXmlReceipt,
    pub(crate) limits: &'a [LimitFact],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReportDisposition {
    Created,
    ReconciledExisting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReportReceipt {
    pub(crate) disposition: ReportDisposition,
    pub(crate) content_hash: Digest,
    pub(crate) byte_length: u64,
}

#[derive(Debug, Error)]
pub(crate) enum ReportError {
    #[error("invalid report request: {0}")]
    Invalid(&'static str),
    #[error("report exceeded {limit}: required {required}, limit {allowed}")]
    Resource {
        limit: &'static str,
        required: u64,
        allowed: u64,
    },
    #[error("report operation was cancelled")]
    Cancelled,
    #[error("report target conflicts with canonical bytes: {path}")]
    Conflict {
        path: PathBuf,
        expected_hash: Digest,
        actual_hash: Digest,
    },
    #[error("report target is conflicting at {path}: {reason}")]
    TargetConflict { path: PathBuf, reason: &'static str },
    #[error("report target changed during verification: {path}")]
    TargetChanged { path: PathBuf },
    #[error("report publication is indeterminate for {path}")]
    Indeterminate {
        path: PathBuf,
        expected_hash: Digest,
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
}

impl ReportError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

pub(crate) fn ensure_report(
    target: &Path,
    facts: &ReportFacts<'_>,
    limits: ReportLimits,
    control: &OperationControl,
) -> Result<ReportReceipt, ReportError> {
    ensure_report_with_hook(target, facts, limits, control, &ProductionPublicationHook)
}

fn ensure_report_with_hook<H: PublicationHook>(
    target: &Path,
    facts: &ReportFacts<'_>,
    limits: ReportLimits,
    control: &OperationControl,
    hook: &H,
) -> Result<ReportReceipt, ReportError> {
    check_cancelled(control)?;
    validate_limits(limits)?;
    if target.file_name().is_none() {
        return Err(ReportError::Invalid("report target must name a file"));
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let parent_witness = DirectoryWitness::capture(parent)
        .map_err(|source| ReportError::io("witness report parent", parent, source))?;
    let (mut guard, stage_file) = create_stage(parent, control)?;
    let mut writer = HashingWriter::new(
        stage_file,
        limits.max_output_bytes.min(limits.max_staging_bytes),
        control,
    );
    write_report(&mut writer, facts).map_err(|source| map_write_error(source, &writer, limits))?;
    let (stage, expected_hash, byte_length) = writer.finish(guard.path())?;
    stage
        .sync_all()
        .map_err(|source| ReportError::io("sync report stage", guard.path(), source))?;
    drop(stage);
    let expected = FileFacts {
        hash: expected_hash,
        bytes: byte_length,
    };
    let stage_metadata = fs::symlink_metadata(guard.path())
        .map_err(|source| ReportError::io("inspect report stage", guard.path(), source))?;
    let readback = verify_existing_regular_file(guard.path(), &stage_metadata, limits, control)?;
    if readback != expected {
        return Err(ReportError::Invalid(
            "report stage changed during read-back",
        ));
    }
    parent_witness
        .verify()
        .map_err(|source| ReportError::io("revalidate report parent", parent, source))?;
    check_cancelled(control)?;

    publish_or_reconcile(
        PublicationContext {
            target,
            parent,
            parent_witness: &parent_witness,
            expected,
            limits,
        },
        &mut guard,
        control,
        hook,
    )
}

#[derive(Clone, Copy)]
struct PublicationContext<'a> {
    target: &'a Path,
    parent: &'a Path,
    parent_witness: &'a DirectoryWitness,
    expected: FileFacts,
    limits: ReportLimits,
}

fn publish_or_reconcile(
    context: PublicationContext<'_>,
    guard: &mut StageGuard,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<ReportReceipt, ReportError> {
    let target = context.target;
    let parent = context.parent;
    let parent_witness = context.parent_witness;
    match fs::symlink_metadata(target) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            hook.reach(PublicationBoundary::BeforeLink, control)
                .map_err(|source| {
                    ReportError::io("run report pre-link boundary", target, source)
                })?;
            check_cancelled(control)?;
            guard
                .verify()
                .map_err(|source| ReportError::io("verify report stage", guard.path(), source))?;
            parent_witness
                .verify()
                .map_err(|source| ReportError::io("revalidate report parent", parent, source))?;
            match fs::hard_link(guard.path(), target) {
                Ok(()) => finish_publication(context, guard, control, hook),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(target).map_err(|source| {
                        ReportError::io("inspect raced report target", target, source)
                    })?;
                    reconcile_existing(context, &metadata, guard, control, hook)
                }
                Err(source) => Err(ReportError::io("publish report", target, source)),
            }
        }
        Ok(metadata) => reconcile_existing(context, &metadata, guard, control, hook),
        Err(source) => Err(ReportError::io("inspect report target", target, source)),
    }
}

fn reconcile_existing(
    context: PublicationContext<'_>,
    initial_metadata: &fs::Metadata,
    stage: &mut StageGuard,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<ReportReceipt, ReportError> {
    let PublicationContext {
        target,
        parent,
        parent_witness,
        expected,
        limits,
    } = context;
    check_cancelled(control)?;
    hook.reach(PublicationBoundary::TargetVerification, control)
        .map_err(|source| ReportError::io("verify reconciled report boundary", target, source))?;
    parent_witness
        .verify()
        .map_err(|_| changed_target_error(target))?;
    let actual = verify_existing_regular_file(target, initial_metadata, limits, control)?;
    if actual != expected {
        return Err(ReportError::Conflict {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            actual_hash: actual.hash,
        });
    }
    check_cancelled(control)?;
    parent_witness
        .verify()
        .map_err(|_| changed_target_error(target))?;
    hook.reach(PublicationBoundary::ParentSync, control)
        .map_err(|source| ReportError::io("sync reconciled report boundary", target, source))?;
    sync_directory(parent)
        .map_err(|source| ReportError::io("sync reconciled report parent", parent, source))?;
    check_cancelled(control)?;
    hook.reach(PublicationBoundary::StageRemoval, control)
        .map_err(|source| ReportError::io("remove reconciled report boundary", target, source))?;
    stage
        .remove()
        .map_err(|source| ReportError::io("remove reconciled report stage", target, source))?;
    hook.reach(PublicationBoundary::CleanupSync, control)
        .map_err(|source| ReportError::io("sync reconciled cleanup boundary", target, source))?;
    sync_directory(parent)
        .map_err(|source| ReportError::io("sync reconciled report cleanup", parent, source))?;
    parent_witness
        .verify()
        .map_err(|_| changed_target_error(target))?;
    hook.reach(PublicationBoundary::TerminalAcknowledgement, control)
        .map_err(|source| ReportError::io("acknowledge reconciled report", target, source))?;
    check_cancelled(control)?;
    Ok(ReportReceipt {
        disposition: ReportDisposition::ReconciledExisting,
        content_hash: expected.hash,
        byte_length: expected.bytes,
    })
}

fn finish_publication(
    context: PublicationContext<'_>,
    stage: &mut StageGuard,
    control: &OperationControl,
    hook: &impl PublicationHook,
) -> Result<ReportReceipt, ReportError> {
    let PublicationContext {
        target,
        parent,
        parent_witness,
        expected,
        limits,
    } = context;
    // A complete target may be observable once the hard link succeeds. Every
    // subsequent failure is therefore indeterminate, including cancellation.
    require_post_link_boundary(
        hook,
        PublicationBoundary::TargetVerification,
        control,
        target,
        expected.hash,
    )?;
    parent_witness
        .verify()
        .map_err(|source| ReportError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            source,
        })?;
    verify_linked_target(stage.path(), target, limits, control)
        .and_then(|actual| {
            if actual == expected {
                Ok(())
            } else {
                Err(ReportError::Invalid(
                    "published report bytes differ from the staged report",
                ))
            }
        })
        .map_err(|error| indeterminate(target, expected.hash, error))?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::ParentSync,
        control,
        target,
        expected.hash,
    )?;
    sync_directory(parent).map_err(|source| ReportError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash: expected.hash,
        source,
    })?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::StageRemoval,
        control,
        target,
        expected.hash,
    )?;
    stage
        .remove()
        .map_err(|source| ReportError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            source,
        })?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::CleanupSync,
        control,
        target,
        expected.hash,
    )?;
    sync_directory(parent).map_err(|source| ReportError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash: expected.hash,
        source,
    })?;
    parent_witness
        .verify()
        .map_err(|source| ReportError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash: expected.hash,
            source,
        })?;
    require_post_link_boundary(
        hook,
        PublicationBoundary::TerminalAcknowledgement,
        control,
        target,
        expected.hash,
    )?;
    Ok(ReportReceipt {
        disposition: ReportDisposition::Created,
        content_hash: expected.hash,
        byte_length: expected.bytes,
    })
}

fn require_post_link_boundary(
    hook: &impl PublicationHook,
    boundary: PublicationBoundary,
    control: &OperationControl,
    target: &Path,
    expected_hash: Digest,
) -> Result<(), ReportError> {
    hook.reach(boundary, control)
        .map_err(|source| ReportError::Indeterminate {
            path: target.to_path_buf(),
            expected_hash,
            source,
        })?;
    check_cancelled(control).map_err(|error| indeterminate(target, expected_hash, error))
}

fn indeterminate(target: &Path, expected_hash: Digest, error: ReportError) -> ReportError {
    ReportError::Indeterminate {
        path: target.to_path_buf(),
        expected_hash,
        source: io::Error::other(error),
    }
}

fn write_report(writer: &mut HashingWriter<'_>, facts: &ReportFacts<'_>) -> io::Result<()> {
    let baseline = facts.baseline.descriptor();
    let changed = facts.changed.descriptor();
    let statistics = facts.qa.statistics();
    write!(writer, "{{\"schema\":")?;
    write_json_string(writer, REPORT_SCHEMA)?;
    write!(writer, ",\"identities\":{{\"run\":")?;
    write_json_hex(writer, &facts.run.into_bytes())?;
    write!(writer, ",\"source\":")?;
    write_json_hex(writer, &facts.source)?;
    write!(writer, ",\"workspace\":")?;
    write_json_hex(writer, &facts.workspace)?;
    write!(writer, ",\"baseline_revision\":")?;
    write_json_hex(writer, &facts.baseline_revision)?;
    write!(writer, ",\"changed_revision\":")?;
    write_json_hex(writer, &facts.changed_revision)?;
    write!(writer, ",\"operation\":")?;
    write_json_hex(writer, &facts.operation)?;
    write!(writer, "}},\"request\":{{\"request_hash\":")?;
    write_json_hex(writer, &facts.request_hash)?;
    write!(writer, ",\"ordinal_hash\":")?;
    write_json_hex(writer, &facts.ordinal_hash)?;
    write!(writer, ",\"recipe_hash\":")?;
    write_json_hex(writer, &facts.recipe_hash)?;
    write!(writer, ",\"qa_input_hash\":")?;
    write_json_hex(writer, &facts.qa_input_hash)?;
    write!(writer, ",\"landxml_options_hash\":")?;
    write_json_hex(writer, &facts.options_hash)?;
    write!(writer, ",\"semantic_results_hash\":")?;
    write_json_hex(writer, &facts.semantic_results_hash)?;
    write!(writer, ",\"path_bindings\":[")?;
    for (index, binding) in facts.path_bindings.iter().enumerate() {
        comma(writer, index)?;
        write_json_hex(writer, binding)?;
    }
    write!(
        writer,
        "]}},\"edit\":{{\"classification_after\":{},\"ordinals\":[",
        facts.non_ground_classification
    )?;
    for (index, ordinal) in facts.correction_ordinals.iter().enumerate() {
        comma(writer, index)?;
        write!(writer, "{ordinal}")?;
    }
    write!(
        writer,
        "],\"changed_point_count\":{},\"footprint\":",
        facts.audit.changed_point_count()
    )?;
    write_bounds(writer, facts.audit.edit_footprint())?;
    write!(writer, ",\"point_id_hash\":")?;
    write_json_hex(writer, facts.audit.point_id_hash().as_bytes())?;
    write!(writer, ",\"audit_hash\":")?;
    write_json_hex(writer, facts.audit.content_hash().as_bytes())?;
    write!(writer, ",\"transitions\":[")?;
    for (index, transition) in facts.audit.transitions().iter().enumerate() {
        comma(writer, index)?;
        write!(
            writer,
            "{{\"before\":{},\"after\":{},\"count\":{}}}",
            transition.before(),
            transition.after(),
            transition.count()
        )?;
    }
    write!(writer, "]}},\"terrain\":{{\"baseline\":")?;
    write_surface(writer, baseline)?;
    write!(writer, ",\"changed\":")?;
    write_surface(writer, changed)?;
    write!(
        writer,
        "}},\"surface_change_envelope\":{{\"meaning\":\"conservative incident-vertex bounds; not an exact change polygon\",\"added_face_count\":{},\"removed_face_count\":{},\"added_face_hash\":",
        facts.envelope.added_face_count, facts.envelope.removed_face_count
    )?;
    write_json_hex(writer, &facts.envelope.added_face_hash)?;
    write!(writer, ",\"removed_face_hash\":")?;
    write_json_hex(writer, &facts.envelope.removed_face_hash)?;
    write!(writer, ",\"bounds\":")?;
    write_bits_bounds(writer, facts.envelope.bounds_bits)?;
    write!(writer, "}},\"qa\":{{\"input_hash\":")?;
    write_json_hex(writer, &facts.qa_input_hash)?;
    write!(writer, ",\"result_hash\":")?;
    write_json_hex(writer, &facts.qa_hash)?;
    write!(writer, ",\"outcomes\":[")?;
    for (index, result) in facts.qa.results().iter().enumerate() {
        comma(writer, index)?;
        let check_point = result.check_point();
        let position = check_point.position();
        write!(writer, "{{\"id\":{},\"position\":[", check_point.id().get())?;
        write_f64(writer, position[0])?;
        write!(writer, ",")?;
        write_f64(writer, position[1])?;
        write!(writer, ",")?;
        write_f64(writer, position[2])?;
        match result.outcome() {
            CheckPointOutcome::Gap => write!(writer, "],\"outcome\":\"gap\"}}")?,
            CheckPointOutcome::Sampled {
                face,
                surface_z,
                residual,
            } => {
                write!(
                    writer,
                    "],\"outcome\":\"sampled\",\"face\":{},\"surface_z\":",
                    face.get()
                )?;
                write_f64(writer, surface_z)?;
                write!(writer, ",\"residual\":")?;
                write_f64(writer, residual)?;
                write!(writer, "}}")?;
            }
        }
    }
    write!(
        writer,
        "],\"statistics\":{{\"covered_count\":{},\"gap_count\":{},\"minimum\":",
        statistics.covered_count(),
        statistics.gap_count()
    )?;
    write_optional_f64(writer, statistics.minimum())?;
    write!(writer, ",\"maximum\":")?;
    write_optional_f64(writer, statistics.maximum())?;
    write!(writer, ",\"mean\":")?;
    write_optional_f64(writer, statistics.mean())?;
    write!(writer, ",\"root_mean_square\":")?;
    write_optional_f64(writer, statistics.root_mean_square())?;
    write!(
        writer,
        "}},\"face_tests\":{},\"accounted_peak_working_bytes\":{}}},\"landxml\":{{\"outcome\":\"ensured_exact\"",
        facts.qa.face_tests(),
        facts.qa.accounted_peak_working_bytes()
    )?;
    write!(writer, ",\"surface_artifact_hash\":")?;
    write_json_hex(writer, facts.landxml.surface_artifact_hash().as_bytes())?;
    write!(writer, ",\"content_hash\":")?;
    write_json_hex(writer, facts.landxml.content_hash().as_bytes())?;
    write!(
        writer,
        ",\"byte_length\":{},\"vertex_count\":{},\"face_count\":{}}},\"limits\":[",
        facts.landxml.byte_length(),
        facts.landxml.vertex_count(),
        facts.landxml.face_count()
    )?;
    for (index, limit) in facts.limits.iter().enumerate() {
        comma(writer, index)?;
        write!(writer, "{{\"name\":")?;
        write_json_string(writer, limit.name)?;
        write!(writer, ",\"value\":{}}}", limit.value)?;
    }
    writeln!(
        writer,
        "],\"external_evidence\":{{\"partner_acceptance_evaluated\":false,\"downstream_round_trip_evaluated\":false,\"human_workflow_acceptance_evaluated\":false}}}}"
    )
}

fn write_surface(
    writer: &mut HashingWriter<'_>,
    value: &point_terrain::TerrainDescriptor,
) -> io::Result<()> {
    write!(
        writer,
        "{{\"input_point_count\":{},\"vertex_count\":{},\"face_count\":{},\"hull_vertex_count\":{},\"input_hash\":",
        value.input_point_count(),
        value.vertex_count(),
        value.face_count(),
        value.hull_vertex_count()
    )?;
    write_json_hex(writer, value.input_hash().as_bytes())?;
    write!(writer, ",\"geometry_hash\":")?;
    write_json_hex(writer, value.geometry_hash().as_bytes())?;
    write!(writer, ",\"topology_hash\":")?;
    write_json_hex(writer, value.topology_hash().as_bytes())?;
    write!(writer, ",\"artifact_hash\":")?;
    write_json_hex(writer, value.artifact_hash().as_bytes())?;
    write!(writer, ",\"bounds\":")?;
    write_bounds(writer, Some(value.bounds()))?;
    write!(
        writer,
        ",\"accounted_peak_working_bytes\":{},\"retained_surface_bytes\":{},\"topology_steps\":{}}}",
        value.accounted_peak_working_bytes(),
        value.retained_surface_bytes(),
        value.topology_steps()
    )
}

fn write_bounds(writer: &mut HashingWriter<'_>, bounds: Option<WorldBounds>) -> io::Result<()> {
    write_bits_bounds(
        writer,
        bounds.map(|value| {
            let min = value.min();
            let max = value.max();
            [
                [min[0].to_bits(), max[0].to_bits()],
                [min[1].to_bits(), max[1].to_bits()],
                [min[2].to_bits(), max[2].to_bits()],
            ]
        }),
    )
}

fn write_bits_bounds(
    writer: &mut HashingWriter<'_>,
    bounds: Option<[[u64; 2]; 3]>,
) -> io::Result<()> {
    let Some(bounds) = bounds else {
        return write!(writer, "null");
    };
    write!(writer, "{{\"min\":[")?;
    for (index, axis) in bounds.iter().enumerate() {
        comma(writer, index)?;
        write_f64(writer, f64::from_bits(axis[0]))?;
    }
    write!(writer, "],\"max\":[")?;
    for (index, axis) in bounds.iter().enumerate() {
        comma(writer, index)?;
        write_f64(writer, f64::from_bits(axis[1]))?;
    }
    write!(writer, "]}}")
}

fn write_optional_f64(writer: &mut HashingWriter<'_>, value: Option<f64>) -> io::Result<()> {
    match value {
        Some(value) => write_f64(writer, value),
        None => write!(writer, "null"),
    }
}

fn write_f64(writer: &mut HashingWriter<'_>, value: f64) -> io::Result<()> {
    if !value.is_finite() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "non-finite report number",
        ));
    }
    write!(writer, "{value:.17}")
}

fn write_json_hex(writer: &mut HashingWriter<'_>, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for byte in bytes {
        write!(writer, "{byte:02x}")?;
    }
    writer.write_all(b"\"")
}

fn write_json_string(writer: &mut HashingWriter<'_>, value: &str) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for character in value.chars() {
        match character {
            '"' => writer.write_all(b"\\\"")?,
            '\\' => writer.write_all(b"\\\\")?,
            '\n' => writer.write_all(b"\\n")?,
            '\r' => writer.write_all(b"\\r")?,
            '\t' => writer.write_all(b"\\t")?,
            value if value < '\u{20}' => write!(writer, "\\u{:04x}", u32::from(value))?,
            value => {
                let mut encoded = [0; 4];
                writer.write_all(value.encode_utf8(&mut encoded).as_bytes())?;
            }
        }
    }
    writer.write_all(b"\"")
}

fn comma(writer: &mut HashingWriter<'_>, index: usize) -> io::Result<()> {
    if index != 0 {
        writer.write_all(b",")?;
    }
    Ok(())
}

struct HashingWriter<'a> {
    file: File,
    hasher: Hasher,
    bytes: u64,
    max_bytes: u64,
    required: u64,
    control: &'a OperationControl,
}

impl<'a> HashingWriter<'a> {
    fn new(file: File, max_bytes: u64, control: &'a OperationControl) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(REPORT_HASH_DOMAIN);
        Self {
            file,
            hasher,
            bytes: 0,
            max_bytes,
            required: 0,
            control,
        }
    }

    fn finish(mut self, path: &Path) -> Result<(File, Digest, u64), ReportError> {
        self.file
            .flush()
            .map_err(|source| ReportError::io("flush report stage", path, source))?;
        Ok((self.file, *self.hasher.finalize().as_bytes(), self.bytes))
    }
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.control
            .check_cancelled()
            .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
        let requested = self
            .bytes
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        self.required = self.required.max(requested);
        if requested > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "report byte limit",
            ));
        }
        let written = self.file.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn map_write_error(
    source: io::Error,
    writer: &HashingWriter<'_>,
    limits: ReportLimits,
) -> ReportError {
    if source.kind() == io::ErrorKind::FileTooLarge {
        let (limit, allowed) = if writer.required > limits.max_output_bytes {
            ("report output bytes", limits.max_output_bytes)
        } else {
            ("report staging bytes", limits.max_staging_bytes)
        };
        ReportError::Resource {
            limit,
            required: writer.required,
            allowed,
        }
    } else if source.kind() == io::ErrorKind::Interrupted {
        ReportError::Cancelled
    } else {
        ReportError::io("encode report", Path::new("report stage"), source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileFacts {
    hash: Digest,
    bytes: u64,
}

fn verify_existing_regular_file(
    path: &Path,
    initial_metadata: &fs::Metadata,
    limits: ReportLimits,
    control: &OperationControl,
) -> Result<FileFacts, ReportError> {
    require_regular_target(path, initial_metadata)?;
    let mut file = File::open(path)
        .map_err(|source| ReportError::io("open existing report target", path, source))?;
    let opened_metadata = file
        .metadata()
        .map_err(|source| ReportError::io("inspect open report target", path, source))?;
    require_regular_target(path, &opened_metadata)?;
    let current_metadata = fs::symlink_metadata(path)
        .map_err(|source| ReportError::io("reinspect report target", path, source))?;
    require_stable_target(path, initial_metadata, &opened_metadata, &current_metadata)?;

    let facts = verify_open_file(&mut file, path, limits, control)?;
    let verified_metadata = file
        .metadata()
        .map_err(|source| ReportError::io("reinspect open report target", path, source))?;
    let final_metadata = fs::symlink_metadata(path)
        .map_err(|source| ReportError::io("reinspect verified report target", path, source))?;
    require_stable_target(path, &opened_metadata, &verified_metadata, &final_metadata)?;
    if !same_file_state(&opened_metadata, &verified_metadata) || final_metadata.len() != facts.bytes
    {
        return Err(changed_target_error(path));
    }
    Ok(facts)
}

fn verify_linked_target(
    stage: &Path,
    target: &Path,
    limits: ReportLimits,
    control: &OperationControl,
) -> Result<FileFacts, ReportError> {
    let stage_before = fs::symlink_metadata(stage)
        .map_err(|source| ReportError::io("inspect linked report stage", stage, source))?;
    let target_before = fs::symlink_metadata(target)
        .map_err(|source| ReportError::io("inspect linked report target", target, source))?;
    require_regular_target(stage, &stage_before)?;
    require_regular_target(target, &target_before)?;
    if !same_file_identity(&stage_before, &target_before) {
        return Err(changed_target_error(target));
    }
    let facts = verify_existing_regular_file(target, &target_before, limits, control)?;
    let stage_after = fs::symlink_metadata(stage)
        .map_err(|source| ReportError::io("reinspect linked report stage", stage, source))?;
    let target_after = fs::symlink_metadata(target)
        .map_err(|source| ReportError::io("reinspect linked report target", target, source))?;
    require_stable_target(target, &stage_before, &stage_after, &target_after)?;
    if !same_file_state(&stage_before, &stage_after) || stage_after.len() != facts.bytes {
        return Err(changed_target_error(target));
    }
    Ok(facts)
}

fn verify_open_file(
    file: &mut File,
    path: &Path,
    limits: ReportLimits,
    control: &OperationControl,
) -> Result<FileFacts, ReportError> {
    let buffer_bytes =
        HASH_BUFFER_BYTES.min(usize::try_from(limits.max_write_buffer_bytes).unwrap_or(0));
    if buffer_bytes == 0 {
        return Err(ReportError::Resource {
            limit: "report write buffer bytes",
            required: 1,
            allowed: limits.max_write_buffer_bytes,
        });
    }
    require(
        u64::try_from(buffer_bytes).unwrap_or(u64::MAX),
        limits.max_working_bytes,
        "report working bytes",
    )?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(buffer_bytes)
        .map_err(|_| ReportError::Resource {
            limit: "report verification buffer allocation",
            required: u64::try_from(buffer_bytes).unwrap_or(u64::MAX),
            allowed: limits.max_working_bytes,
        })?;
    require(
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        limits.max_working_bytes,
        "report working bytes",
    )?;
    buffer.resize(buffer_bytes, 0);
    let mut hasher = Hasher::new();
    hasher.update(REPORT_HASH_DOMAIN);
    let mut bytes = 0_u64;
    loop {
        check_cancelled(control)?;
        let read = file
            .read(&mut buffer)
            .map_err(|source| ReportError::io("hash report", path, source))?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        require(bytes, limits.max_output_bytes, "report output bytes")?;
        require(bytes, limits.max_staging_bytes, "report staging bytes")?;
        hasher.update(&buffer[..read]);
    }
    Ok(FileFacts {
        hash: *hasher.finalize().as_bytes(),
        bytes,
    })
}

fn require_regular_target(path: &Path, metadata: &fs::Metadata) -> Result<(), ReportError> {
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(ReportError::TargetConflict {
            path: path.to_path_buf(),
            reason: "an existing target must be a regular non-symlink file",
        })
    }
}

fn require_stable_target(
    path: &Path,
    initial: &fs::Metadata,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> Result<(), ReportError> {
    require_regular_target(path, current)?;
    if same_file_identity(initial, opened) && same_file_identity(opened, current) {
        Ok(())
    } else {
        Err(changed_target_error(path))
    }
}

fn changed_target_error(path: &Path) -> ReportError {
    ReportError::TargetChanged {
        path: path.to_path_buf(),
    }
}

fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && matches!(
            (left.modified(), right.modified()),
            (Ok(left_modified), Ok(right_modified)) if left_modified == right_modified
        )
}

fn validate_limits(limits: ReportLimits) -> Result<(), ReportError> {
    require(1, limits.max_output_bytes, "report output bytes")?;
    require(1, limits.max_staging_bytes, "report staging bytes")?;
    require(
        1,
        limits.max_write_buffer_bytes,
        "report write buffer bytes",
    )?;
    require(
        limits.max_write_buffer_bytes.min(HASH_BUFFER_BYTES as u64),
        limits.max_working_bytes,
        "report working bytes",
    )
}

fn require(required: u64, allowed: u64, limit: &'static str) -> Result<(), ReportError> {
    if required > allowed {
        Err(ReportError::Resource {
            limit,
            required,
            allowed,
        })
    } else {
        Ok(())
    }
}

fn check_cancelled(control: &OperationControl) -> Result<(), ReportError> {
    control
        .check_cancelled()
        .map_err(|_| ReportError::Cancelled)
}

fn create_stage(
    parent: &Path,
    control: &OperationControl,
) -> Result<(StageGuard, File), ReportError> {
    create_publication_stage(
        parent,
        "report",
        || check_cancelled(control),
        |error| match error {
            StageCreationError::RandomnessUnavailable => {
                ReportError::Invalid("system randomness is unavailable")
            }
            StageCreationError::NamespaceExhausted => {
                ReportError::Invalid("report staging name space is exhausted")
            }
            StageCreationError::Inspect { path, source } => {
                ReportError::io("inspect report stage", &path, source)
            }
            StageCreationError::Create { path, source } => {
                ReportError::io("create report stage", &path, source)
            }
        },
    )
}

#[cfg(test)]
struct Hex<'a>(&'a [u8]);

#[cfg(test)]
impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[derive(Clone, Copy)]
    enum TestAction<'a> {
        Failure(PublicationBoundary),
        Cancellation(PublicationBoundary),
        Install {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
            replace: bool,
        },
        ModifyInPlace {
            boundary: PublicationBoundary,
            target: &'a Path,
            bytes: &'a [u8],
        },
    }

    struct TestHook<'a>(TestAction<'a>);

    impl PublicationHook for TestHook<'_> {
        fn reach(
            &self,
            boundary: PublicationBoundary,
            control: &OperationControl,
        ) -> io::Result<()> {
            match self.0 {
                TestAction::Failure(expected) if boundary == expected => Err(io::Error::other(
                    format!("injected report failure at {boundary:?}"),
                )),
                TestAction::Cancellation(expected) if boundary == expected => {
                    control.cancel();
                    Ok(())
                }
                TestAction::Install {
                    boundary: expected,
                    target,
                    bytes,
                    replace,
                } if boundary == expected => {
                    if replace {
                        fs::remove_file(target)?;
                    }
                    write_synced(target, bytes)
                }
                TestAction::ModifyInPlace {
                    boundary: expected,
                    target,
                    bytes,
                } if boundary == expected => overwrite_synced(target, bytes),
                _ => Ok(()),
            }
        }
    }

    #[test]
    fn pre_link_failure_and_cancellation_leave_no_target_or_stage() {
        let directory = Directory::new("pre-link");
        for action in [
            TestAction::Failure(PublicationBoundary::BeforeLink),
            TestAction::Cancellation(PublicationBoundary::BeforeLink),
        ] {
            let target = directory
                .path
                .join(format!("target-{}.json", action_name(action)));
            let (mut stage, expected) = directory.prepared(b"canonical report\n");
            let control = OperationControl::new();
            let failure =
                publish_prepared(&target, &mut stage, expected, &control, &TestHook(action))
                    .expect_err("a pre-link stop cannot return a receipt");
            assert!(matches!(
                failure,
                ReportError::Io { .. } | ReportError::Cancelled
            ));
            assert!(!target.exists());
            drop(stage);
            directory.assert_no_stages();
        }
    }

    #[test]
    fn every_post_link_boundary_is_indeterminate_with_complete_bytes() {
        let directory = Directory::new("post-link");
        let canonical = b"canonical report\n";
        for boundary in [
            PublicationBoundary::TargetVerification,
            PublicationBoundary::ParentSync,
            PublicationBoundary::StageRemoval,
            PublicationBoundary::CleanupSync,
            PublicationBoundary::TerminalAcknowledgement,
        ] {
            let target = directory.path.join(format!("{boundary:?}.json"));
            let (mut stage, expected) = directory.prepared(canonical);
            let failure = publish_prepared(
                &target,
                &mut stage,
                expected,
                &OperationControl::new(),
                &TestHook(TestAction::Failure(boundary)),
            )
            .expect_err("post-link failure cannot acknowledge publication");
            assert!(matches!(
                failure,
                ReportError::Indeterminate {
                    expected_hash,
                    ..
                } if expected_hash == expected.hash
            ));
            assert_eq!(fs::read(&target).unwrap(), canonical);
            drop(stage);
            directory.assert_no_stages();
        }
    }

    #[test]
    fn lost_acknowledgement_cancellation_is_indeterminate_and_reconcilable() {
        let directory = Directory::new("lost-ack");
        let target = directory.path.join("audit.json");
        let canonical = b"canonical report\n";
        let (mut stage, expected) = directory.prepared(canonical);
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Cancellation(
                PublicationBoundary::TerminalAcknowledgement,
            )),
        )
        .expect_err("lost acknowledgement has no receipt");
        assert!(matches!(failure, ReportError::Indeterminate { .. }));
        drop(stage);

        let (mut retry_stage, retry_expected) = directory.prepared(canonical);
        let receipt = publish_prepared(
            &target,
            &mut retry_stage,
            retry_expected,
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect("retry reconciles exact durable bytes");
        assert_eq!(receipt.disposition, ReportDisposition::ReconciledExisting);
        assert_eq!(receipt.content_hash, expected.hash);
        directory.assert_no_stages();
    }

    #[test]
    fn already_exists_race_reconciles_exact_and_preserves_conflict() {
        let directory = Directory::new("already-exists");
        let canonical = b"canonical report\n";
        let exact_target = directory.path.join("exact.json");
        let (mut exact_stage, expected) = directory.prepared(canonical);
        let exact_receipt = publish_prepared(
            &exact_target,
            &mut exact_stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::BeforeLink,
                target: &exact_target,
                bytes: canonical,
                replace: false,
            }),
        )
        .expect("an exact create race reconciles");
        assert_eq!(
            exact_receipt.disposition,
            ReportDisposition::ReconciledExisting
        );

        let conflict_target = directory.path.join("conflict.json");
        let caller_bytes = b"caller-owned conflict\n";
        let (mut conflict_stage, conflict_expected) = directory.prepared(canonical);
        let failure = publish_prepared(
            &conflict_target,
            &mut conflict_stage,
            conflict_expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::BeforeLink,
                target: &conflict_target,
                bytes: caller_bytes,
                replace: false,
            }),
        )
        .expect_err("a conflicting create race fails closed");
        assert!(matches!(
            failure,
            ReportError::Conflict {
                expected_hash,
                actual_hash,
                ..
            } if expected_hash == conflict_expected.hash
                && actual_hash == facts_for(caller_bytes).hash
        ));
        assert_eq!(fs::read(&conflict_target).unwrap(), caller_bytes);
        drop(exact_stage);
        drop(conflict_stage);
        directory.assert_no_stages();
    }

    #[test]
    fn post_link_replacement_is_preserved_and_never_acknowledged() {
        let directory = Directory::new("replacement");
        let target = directory.path.join("audit.json");
        let canonical = b"canonical report\n";
        let replacement = b"caller replacement\n";
        let (mut stage, expected) = directory.prepared(canonical);
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::Install {
                boundary: PublicationBoundary::TargetVerification,
                target: &target,
                bytes: replacement,
                replace: true,
            }),
        )
        .expect_err("a replaced post-link target has no receipt");
        assert!(matches!(failure, ReportError::Indeterminate { .. }));
        assert_eq!(fs::read(&target).unwrap(), replacement);
        drop(stage);
        directory.assert_no_stages();
    }

    #[cfg(unix)]
    #[test]
    fn post_link_in_place_modification_is_preserved() {
        let directory = Directory::new("in-place-modification");
        let target = directory.path.join("audit.json");
        let concurrent_bytes = b"concurrent writer bytes\n";
        let (mut stage, expected) = directory.prepared(b"canonical report\n");
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &TestHook(TestAction::ModifyInPlace {
                boundary: PublicationBoundary::TargetVerification,
                target: &target,
                bytes: concurrent_bytes,
            }),
        )
        .expect_err("a modified post-link target has no receipt");
        assert!(matches!(failure, ReportError::Indeterminate { .. }));
        assert_eq!(fs::read(&target).unwrap(), concurrent_bytes);
        drop(stage);
        assert_eq!(fs::read(&target).unwrap(), concurrent_bytes);
        fs::remove_file(target).unwrap();
        directory.assert_no_stages();
    }

    #[test]
    fn stage_guard_never_removes_a_replacement_path() {
        let directory = Directory::new("stage-replacement");
        let (stage, _) = directory.prepared(b"canonical report\n");
        let stage_path = stage.path().to_path_buf();
        fs::remove_file(&stage_path).unwrap();
        write_synced(&stage_path, b"unowned replacement\n").unwrap();
        drop(stage);
        assert_eq!(fs::read(&stage_path).unwrap(), b"unowned replacement\n");
        fs::remove_file(stage_path).unwrap();
    }

    #[test]
    fn non_regular_target_has_target_conflict_taxonomy() {
        let directory = Directory::new("target-kind");
        let target = directory.path.join("audit.json");
        fs::create_dir(&target).unwrap();
        let (mut stage, expected) = directory.prepared(b"canonical report\n");
        let failure = publish_prepared(
            &target,
            &mut stage,
            expected,
            &OperationControl::new(),
            &ProductionPublicationHook,
        )
        .expect_err("a directory target fails closed");
        assert!(matches!(failure, ReportError::TargetConflict { .. }));
        assert!(target.is_dir());
        drop(stage);
        directory.assert_no_stages();
    }

    #[test]
    fn staging_and_actual_verification_capacity_are_bounded() {
        let directory = Directory::new("limits");
        let stage_path = directory.path.join("stage.json");
        let file = File::create(&stage_path).unwrap();
        let control = OperationControl::new();
        let mut writer = HashingWriter::new(file, 3, &control);
        let write_error = writer.write_all(b"four").unwrap_err();
        let limits = ReportLimits {
            max_output_bytes: 10,
            max_staging_bytes: 3,
            max_write_buffer_bytes: 8,
            max_working_bytes: 8,
        };
        assert!(matches!(
            map_write_error(write_error, &writer, limits),
            ReportError::Resource {
                limit: "report staging bytes",
                required: 4,
                allowed: 3
            }
        ));
        drop(writer);

        fs::remove_file(&stage_path).unwrap();
        write_synced(&stage_path, b"bounded").unwrap();
        let metadata = fs::symlink_metadata(&stage_path).unwrap();
        let verification_limits = ReportLimits {
            max_output_bytes: 10,
            max_staging_bytes: 10,
            max_write_buffer_bytes: 8,
            max_working_bytes: 7,
        };
        assert!(matches!(
            verify_existing_regular_file(
                &stage_path,
                &metadata,
                verification_limits,
                &OperationControl::new()
            ),
            Err(ReportError::Resource {
                limit: "report working bytes",
                required: 8,
                allowed: 7
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn directory_witness_detects_same_path_replacement() {
        let directory = Directory::new("parent-witness");
        let witness = DirectoryWitness::capture(&directory.path).unwrap();
        fs::remove_dir(&directory.path).unwrap();
        fs::create_dir(&directory.path).unwrap();
        assert!(witness.verify().is_err());
    }

    fn publish_prepared(
        target: &Path,
        stage: &mut StageGuard,
        expected: FileFacts,
        control: &OperationControl,
        hook: &impl PublicationHook,
    ) -> Result<ReportReceipt, ReportError> {
        let parent = target.parent().unwrap();
        let parent_witness = DirectoryWitness::capture(parent).unwrap();
        publish_or_reconcile(
            PublicationContext {
                target,
                parent,
                parent_witness: &parent_witness,
                expected,
                limits: ReportLimits::default(),
            },
            stage,
            control,
            hook,
        )
    }

    fn facts_for(bytes: &[u8]) -> FileFacts {
        let mut hasher = Hasher::new();
        hasher.update(REPORT_HASH_DOMAIN);
        hasher.update(bytes);
        FileFacts {
            hash: *hasher.finalize().as_bytes(),
            bytes: u64::try_from(bytes.len()).unwrap(),
        }
    }

    fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        sync_directory(path.parent().unwrap())
    }

    fn overwrite_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    const fn action_name(action: TestAction<'_>) -> &'static str {
        match action {
            TestAction::Failure(_) => "failure",
            TestAction::Cancellation(_) => "cancellation",
            TestAction::Install { .. } => "install",
            TestAction::ModifyInPlace { .. } => "modify-in-place",
        }
    }

    struct Directory {
        path: PathBuf,
    }

    impl Directory {
        fn new(label: &str) -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "punctra-terrain-report-{label}-{}-{}",
                std::process::id(),
                Hex(&random)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn prepared(&self, bytes: &[u8]) -> (StageGuard, FileFacts) {
            let (stage, mut file) = create_stage(&self.path, &OperationControl::new()).unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            drop(file);
            stage.verify().unwrap();
            (stage, facts_for(bytes))
        }

        fn assert_no_stages(&self) {
            let stages = fs::read_dir(&self.path)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".punctra-report-")
                })
                .count();
            assert_eq!(stages, 0, "recognized report stages must be cleaned");
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
