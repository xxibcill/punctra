// Canonical JSON key order is intentionally expressed by one linear encoder;
// splitting it would make byte-order review harder. Limit field names mirror
// the public resource vocabulary.
#![allow(clippy::struct_field_names, clippy::too_many_lines)]

use std::{
    io::{self, Write},
    path::Path,
};

use foundation_runtime::OperationControl;
use point_contracts::WorldBounds;
use point_terrain::{CheckPointOutcome, CheckPointReport, LandXmlReceipt, TerrainSurface};
use point_workspace::RevisionAudit;

use crate::{
    canonical_output::{
        CanonicalOutputError, CanonicalOutputLimits, CanonicalOutputReceipt, CanonicalOutputSpec,
        ensure_output,
    },
    journal::{Digest, WorkflowRunId},
};

pub(crate) const REPORT_SCHEMA: &str = "punctra.terrain-workflow.audit.v1";
pub(crate) const REPORT_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-report-bytes-v1";
const REPORT_OUTPUT: CanonicalOutputSpec =
    CanonicalOutputSpec::new("report", "report", REPORT_HASH_DOMAIN);

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

pub(crate) fn ensure_report(
    target: &Path,
    facts: &ReportFacts<'_>,
    limits: CanonicalOutputLimits,
    control: &OperationControl,
) -> Result<CanonicalOutputReceipt, CanonicalOutputError> {
    ensure_output(
        target,
        REPORT_OUTPUT,
        limits,
        control,
        |writer| write_report(writer, facts),
        || Ok(()),
    )
}

fn write_report(writer: &mut dyn Write, facts: &ReportFacts<'_>) -> io::Result<()> {
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
    writer: &mut dyn Write,
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

fn write_bounds(writer: &mut dyn Write, bounds: Option<WorldBounds>) -> io::Result<()> {
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

fn write_bits_bounds(writer: &mut dyn Write, bounds: Option<[[u64; 2]; 3]>) -> io::Result<()> {
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

fn write_optional_f64(writer: &mut dyn Write, value: Option<f64>) -> io::Result<()> {
    match value {
        Some(value) => write_f64(writer, value),
        None => write!(writer, "null"),
    }
}

fn write_f64(writer: &mut dyn Write, value: f64) -> io::Result<()> {
    if !value.is_finite() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "non-finite report number",
        ));
    }
    write!(writer, "{value:.17}")
}

fn write_json_hex(writer: &mut dyn Write, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for byte in bytes {
        write!(writer, "{byte:02x}")?;
    }
    writer.write_all(b"\"")
}

fn write_json_string(writer: &mut dyn Write, value: &str) -> io::Result<()> {
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

fn comma(writer: &mut dyn Write, index: usize) -> io::Result<()> {
    if index != 0 {
        writer.write_all(b",")?;
    }
    Ok(())
}
