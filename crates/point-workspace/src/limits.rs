use point_index::CandidateLimits;
use point_source::ReadBudget;

use crate::WorkspaceError;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Hard resource ceilings for creating or reopening one Workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct OpenLimits {
    max_manifest_bytes: u64,
    max_operation_records: u64,
    max_revision_files: u64,
    max_revision_blocks: u64,
    max_revision_rows: u64,
    max_revision_block_bytes: u64,
    max_single_file_bytes: u64,
    max_total_persisted_bytes: u64,
    max_working_bytes: u64,
    max_resident_metadata_bytes: u64,
}

impl OpenLimits {
    /// Creates a fail-closed limit set with every ceiling set to zero.
    ///
    /// Every zero permits only zero use of that resource. Because a valid
    /// Workspace has a nonempty manifest and root Revision, zero manifest or
    /// Revision capacity makes every existing Workspace fail explicitly.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_manifest_bytes: 0,
            max_operation_records: 0,
            max_revision_files: 0,
            max_revision_blocks: 0,
            max_revision_rows: 0,
            max_revision_block_bytes: 0,
            max_single_file_bytes: 0,
            max_total_persisted_bytes: 0,
            max_working_bytes: 0,
            max_resident_metadata_bytes: 0,
        }
    }

    /// Sets the maximum accepted manifest file length.
    #[must_use]
    pub const fn with_max_manifest_bytes(mut self, value: u64) -> Self {
        self.max_manifest_bytes = value;
        self
    }

    /// Sets the maximum combined durable intent and rejection records.
    #[must_use]
    pub const fn with_max_operation_records(mut self, value: u64) -> Self {
        self.max_operation_records = value;
        self
    }

    /// Sets the maximum immutable Revision files, including the root.
    #[must_use]
    pub const fn with_max_revision_files(mut self, value: u64) -> Self {
        self.max_revision_files = value;
        self
    }

    /// Sets the maximum checksummed Revision blocks scanned in total.
    #[must_use]
    pub const fn with_max_revision_blocks(mut self, value: u64) -> Self {
        self.max_revision_blocks = value;
        self
    }

    /// Sets the cumulative changed-row ceiling across all Revisions.
    #[must_use]
    pub const fn with_max_revision_rows(mut self, value: u64) -> Self {
        self.max_revision_rows = value;
        self
    }

    /// Sets the maximum encoded payload bytes in one Revision block.
    #[must_use]
    pub const fn with_max_revision_block_bytes(mut self, value: u64) -> Self {
        self.max_revision_block_bytes = value;
        self
    }

    /// Sets the maximum accepted length of any one persisted file.
    #[must_use]
    pub const fn with_max_single_file_bytes(mut self, value: u64) -> Self {
        self.max_single_file_bytes = value;
        self
    }

    /// Sets the cumulative persisted-byte ceiling charged during open.
    #[must_use]
    pub const fn with_max_total_persisted_bytes(mut self, value: u64) -> Self {
        self.max_total_persisted_bytes = value;
        self
    }

    /// Sets the peak temporary working-memory ceiling.
    #[must_use]
    pub const fn with_max_working_bytes(mut self, value: u64) -> Self {
        self.max_working_bytes = value;
        self
    }

    /// Sets the retained Revision and operation metadata ceiling.
    #[must_use]
    pub const fn with_max_resident_metadata_bytes(mut self, value: u64) -> Self {
        self.max_resident_metadata_bytes = value;
        self
    }

    /// Returns the maximum accepted manifest file length.
    #[must_use]
    pub const fn max_manifest_bytes(self) -> u64 {
        self.max_manifest_bytes
    }

    /// Returns the maximum combined durable intent and rejection records.
    #[must_use]
    pub const fn max_operation_records(self) -> u64 {
        self.max_operation_records
    }

    /// Returns the maximum immutable Revision files, including the root.
    #[must_use]
    pub const fn max_revision_files(self) -> u64 {
        self.max_revision_files
    }

    /// Returns the maximum checksummed Revision blocks scanned in total.
    #[must_use]
    pub const fn max_revision_blocks(self) -> u64 {
        self.max_revision_blocks
    }

    /// Returns the cumulative changed-row ceiling across all Revisions.
    #[must_use]
    pub const fn max_revision_rows(self) -> u64 {
        self.max_revision_rows
    }

    /// Returns the maximum encoded payload bytes in one Revision block.
    #[must_use]
    pub const fn max_revision_block_bytes(self) -> u64 {
        self.max_revision_block_bytes
    }

    /// Returns the maximum accepted length of any one persisted file.
    #[must_use]
    pub const fn max_single_file_bytes(self) -> u64 {
        self.max_single_file_bytes
    }

    /// Returns the cumulative persisted-byte ceiling charged during open.
    #[must_use]
    pub const fn max_total_persisted_bytes(self) -> u64 {
        self.max_total_persisted_bytes
    }

    /// Returns the peak temporary working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }

    /// Returns the retained Revision and operation metadata ceiling.
    #[must_use]
    pub const fn max_resident_metadata_bytes(self) -> u64 {
        self.max_resident_metadata_bytes
    }
}

impl Default for OpenLimits {
    fn default() -> Self {
        Self::new()
            .with_max_manifest_bytes(MIB)
            .with_max_operation_records(100_000)
            .with_max_revision_files(100_001)
            .with_max_revision_blocks(10_000_000)
            .with_max_revision_rows(100_000_000)
            .with_max_revision_block_bytes(4 * MIB)
            .with_max_single_file_bytes(4 * GIB)
            .with_max_total_persisted_bytes(64 * GIB)
            .with_max_working_bytes(128 * MIB)
            .with_max_resident_metadata_bytes(128 * MIB)
    }
}

/// Cumulative hard ceilings for one exact Point Set selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct PointSetLimits {
    candidate_limits: CandidateLimits,
    source_read_budget: ReadBudget,
    max_input_point_ids: u64,
    max_output_points: u64,
    max_overlay_segments: u64,
    max_overlay_bytes: u64,
    max_working_bytes: u64,
    max_resident_bytes: u64,
    max_temporary_bytes: u64,
}

impl PointSetLimits {
    /// Creates one selection budget without hidden per-batch resets.
    ///
    /// `max_input_point_ids` is charged only by explicit Point-ID selection.
    /// `max_output_points` covers the complete unpublished result.
    /// `max_working_bytes` is the combined peak of retained candidate spans,
    /// the current Source batch, overlay state, and builder buffers; child
    /// limits are not independent extra allowances. `max_temporary_bytes`
    /// charges all spill bytes written during the selection. A zero resident
    /// ceiling forces every nonempty Point Set to spill; a simultaneous zero
    /// temporary ceiling therefore permits only an empty result.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        candidate_limits: CandidateLimits,
        source_read_budget: ReadBudget,
        max_input_point_ids: u64,
        max_output_points: u64,
        max_overlay_segments: u64,
        max_overlay_bytes: u64,
        max_working_bytes: u64,
        max_resident_bytes: u64,
        max_temporary_bytes: u64,
    ) -> Self {
        Self {
            candidate_limits,
            source_read_budget,
            max_input_point_ids,
            max_output_points,
            max_overlay_segments,
            max_overlay_bytes,
            max_working_bytes,
            max_resident_bytes,
            max_temporary_bytes,
        }
    }

    /// Returns conservative Spatial Index planning limits.
    #[must_use]
    pub const fn candidate_limits(self) -> CandidateLimits {
        self.candidate_limits
    }

    /// Returns bounded Source batch, payload, span, Point, and decoder limits.
    #[must_use]
    pub const fn source_read_budget(self) -> ReadBudget {
        self.source_read_budget
    }

    /// Returns the caller-supplied Point Identity ceiling.
    #[must_use]
    pub const fn max_input_point_ids(self) -> u64 {
        self.max_input_point_ids
    }

    /// Returns the exact result Point ceiling.
    #[must_use]
    pub const fn max_output_points(self) -> u64 {
        self.max_output_points
    }

    /// Returns the maximum Revision overlay segments inspected.
    #[must_use]
    pub const fn max_overlay_segments(self) -> u64 {
        self.max_overlay_segments
    }

    /// Returns the cumulative overlay payload bytes read across all segments.
    #[must_use]
    pub const fn max_overlay_bytes(self) -> u64 {
        self.max_overlay_bytes
    }

    /// Returns the combined peak selection working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }

    /// Returns the maximum Point Set bytes retained in memory.
    #[must_use]
    pub const fn max_resident_bytes(self) -> u64 {
        self.max_resident_bytes
    }

    /// Returns the cumulative Point Set spill-byte ceiling.
    #[must_use]
    pub const fn max_temporary_bytes(self) -> u64 {
        self.max_temporary_bytes
    }
}

impl Default for PointSetLimits {
    fn default() -> Self {
        Self::new(
            CandidateLimits::default(),
            ReadBudget::default().with_max_points(50_000_000),
            10_000_000,
            10_000_000,
            100_000,
            4 * GIB,
            128 * MIB,
            64 * MIB,
            4 * GIB,
        )
    }
}

/// Cumulative hard ceilings for one exact Snapshot Point-row stream.
///
/// These limits deliberately do not include Point Set resident or spill
/// storage: a row stream retains only its current Source and output batches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct PointRowLimits {
    candidate_limits: CandidateLimits,
    source_read_budget: ReadBudget,
    max_overlay_segments: u64,
    max_overlay_bytes: u64,
    max_output_points: u64,
    max_batch_points: u64,
    max_batch_payload_bytes: u64,
    max_working_bytes: u64,
}

impl PointRowLimits {
    /// Creates one Point-row budget without hidden per-batch resets.
    ///
    /// Candidate and Source limits cover the complete normalized Query.
    /// Overlay segment and byte ceilings accumulate across every Source batch,
    /// while output batch ceilings apply to each returned nonempty batch.
    /// `max_working_bytes` is the combined peak of retained candidate spans,
    /// the current Source batch, adapter allowance, effective-classification
    /// copy, overlay block, and unpublished output columns.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        candidate_limits: CandidateLimits,
        source_read_budget: ReadBudget,
        max_overlay_segments: u64,
        max_overlay_bytes: u64,
        max_output_points: u64,
        max_batch_points: u64,
        max_batch_payload_bytes: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            candidate_limits,
            source_read_budget,
            max_overlay_segments,
            max_overlay_bytes,
            max_output_points,
            max_batch_points,
            max_batch_payload_bytes,
            max_working_bytes,
        }
    }

    /// Returns conservative Spatial Index planning limits.
    #[must_use]
    pub const fn candidate_limits(self) -> CandidateLimits {
        self.candidate_limits
    }

    /// Returns bounded Source span, Point, batch, payload, and decoder limits.
    #[must_use]
    pub const fn source_read_budget(self) -> ReadBudget {
        self.source_read_budget
    }

    /// Returns the cumulative Revision-overlay segment ceiling.
    #[must_use]
    pub const fn max_overlay_segments(self) -> u64 {
        self.max_overlay_segments
    }

    /// Returns the cumulative Revision-overlay payload-byte ceiling.
    #[must_use]
    pub const fn max_overlay_bytes(self) -> u64 {
        self.max_overlay_bytes
    }

    /// Returns the exact complete emitted-row ceiling.
    #[must_use]
    pub const fn max_output_points(self) -> u64 {
        self.max_output_points
    }

    /// Returns the maximum rows in one emitted batch.
    #[must_use]
    pub const fn max_batch_points(self) -> u64 {
        self.max_batch_points
    }

    /// Returns the maximum exact column payload bytes in one emitted batch.
    #[must_use]
    pub const fn max_batch_payload_bytes(self) -> u64 {
        self.max_batch_payload_bytes
    }

    /// Returns the combined peak incremental working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for PointRowLimits {
    fn default() -> Self {
        Self::new(
            CandidateLimits::default(),
            ReadBudget::default().with_max_points(50_000_000),
            100_000,
            4 * GIB,
            10_000_000,
            65_536,
            4 * MIB,
            128 * MIB,
        )
    }
}

/// Hard ceilings for one complete immutable Revision Audit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct RevisionAuditLimits {
    source_read_budget: ReadBudget,
    max_revision_blocks: u64,
    max_revision_bytes: u64,
    max_changed_points: u64,
    max_transition_entries: u64,
    max_result_bytes: u64,
    max_working_bytes: u64,
}

impl RevisionAuditLimits {
    /// Creates independent Source, Revision, result, and memory ceilings.
    ///
    /// Every value is a hard ceiling. Zero Revision or Source capacity permits
    /// only the Root Revision's empty input. Result and working ceilings still
    /// apply to that canonical empty report.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        source_read_budget: ReadBudget,
        max_revision_blocks: u64,
        max_revision_bytes: u64,
        max_changed_points: u64,
        max_transition_entries: u64,
        max_result_bytes: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            source_read_budget,
            max_revision_blocks,
            max_revision_bytes,
            max_changed_points,
            max_transition_entries,
            max_result_bytes,
            max_working_bytes,
        }
    }

    /// Returns the exact Source span, Point, batch, payload, and decoder limits.
    #[must_use]
    pub const fn source_read_budget(self) -> ReadBudget {
        self.source_read_budget
    }

    /// Returns the maximum checksummed Revision blocks read.
    #[must_use]
    pub const fn max_revision_blocks(self) -> u64 {
        self.max_revision_blocks
    }

    /// Returns the maximum encoded immutable Revision file bytes read.
    #[must_use]
    pub const fn max_revision_bytes(self) -> u64 {
        self.max_revision_bytes
    }

    /// Returns the maximum exact changed rows in the Revision.
    #[must_use]
    pub const fn max_changed_points(self) -> u64 {
        self.max_changed_points
    }

    /// Returns the maximum distinct `(before, after)` transition entries.
    #[must_use]
    pub const fn max_transition_entries(self) -> u64 {
        self.max_transition_entries
    }

    /// Returns the maximum complete retained report bytes.
    #[must_use]
    pub const fn max_result_bytes(self) -> u64 {
        self.max_result_bytes
    }

    /// Returns the combined peak incremental audit working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for RevisionAuditLimits {
    fn default() -> Self {
        Self::new(
            ReadBudget::default().with_max_points(10_000_000),
            1_000_000,
            2 * GIB,
            10_000_000,
            65_280,
            2 * MIB,
            128 * MIB,
        )
    }
}

/// Hard ceilings for streaming exact Point Identities from a completed Point Set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct PointIdReadLimits {
    max_points: u64,
    max_batch_points: u64,
    max_batch_bytes: u64,
    max_read_buffer_bytes: u64,
    max_working_bytes: u64,
}

impl PointIdReadLimits {
    /// Creates explicit read ceilings.
    ///
    /// Zero `max_points` permits only an empty Point Set. Zero batch or working
    /// capacity likewise permits only a read that emits no batch.
    #[must_use]
    pub const fn new(
        max_points: u64,
        max_batch_points: u64,
        max_batch_bytes: u64,
        max_read_buffer_bytes: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            max_points,
            max_batch_points,
            max_batch_bytes,
            max_read_buffer_bytes,
            max_working_bytes,
        }
    }

    /// Returns the total exact Point Identity ceiling.
    #[must_use]
    pub const fn max_points(self) -> u64 {
        self.max_points
    }

    /// Returns the maximum identities in one emitted batch.
    #[must_use]
    pub const fn max_batch_points(self) -> u64 {
        self.max_batch_points
    }

    /// Returns the maximum canonical payload bytes in one emitted batch.
    #[must_use]
    pub const fn max_batch_bytes(self) -> u64 {
        self.max_batch_bytes
    }

    /// Returns the maximum bytes used to decode one resident or spilled batch.
    #[must_use]
    pub const fn max_read_buffer_bytes(self) -> u64 {
        self.max_read_buffer_bytes
    }

    /// Returns the peak stream working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for PointIdReadLimits {
    fn default() -> Self {
        Self::new(u64::MAX, 65_536, 4 * MIB, 4 * MIB, 8 * MIB)
    }
}

/// Hard resource ceilings for staging and publishing one classification Edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct CommitLimits {
    max_selected_points: u64,
    max_changed_points: u64,
    max_input_frames: u64,
    max_block_points: u64,
    max_block_bytes: u64,
    max_working_bytes: u64,
    max_temporary_bytes: u64,
    max_revision_bytes: u64,
    max_total_durable_bytes: u64,
}

impl CommitLimits {
    /// Creates a fail-closed limit set with every ceiling set to zero.
    ///
    /// Each value is a hard ceiling. Selected Points and input frames are
    /// charged even when many rows are unchanged. Zero selected, changed,
    /// block, temporary, Revision, or total-durable capacity prevents
    /// publication of a nonempty Edit. Temporary bytes are cumulative across
    /// all staging files; working bytes are peak simultaneous memory.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_selected_points: 0,
            max_changed_points: 0,
            max_input_frames: 0,
            max_block_points: 0,
            max_block_bytes: 0,
            max_working_bytes: 0,
            max_temporary_bytes: 0,
            max_revision_bytes: 0,
            max_total_durable_bytes: 0,
        }
    }

    /// Sets the maximum selected Points inspected by one commit.
    #[must_use]
    pub const fn with_max_selected_points(mut self, value: u64) -> Self {
        self.max_selected_points = value;
        self
    }

    /// Sets the maximum Points whose effective value may change.
    #[must_use]
    pub const fn with_max_changed_points(mut self, value: u64) -> Self {
        self.max_changed_points = value;
        self
    }

    /// Sets the maximum Point Set or prior-Revision frames consumed.
    #[must_use]
    pub const fn with_max_input_frames(mut self, value: u64) -> Self {
        self.max_input_frames = value;
        self
    }

    /// Sets the maximum change rows in one checksummed Revision block.
    #[must_use]
    pub const fn with_max_block_points(mut self, value: u64) -> Self {
        self.max_block_points = value;
        self
    }

    /// Sets the maximum encoded bytes in one Revision block.
    #[must_use]
    pub const fn with_max_block_bytes(mut self, value: u64) -> Self {
        self.max_block_bytes = value;
        self
    }

    /// Sets the combined peak commit working-memory ceiling.
    #[must_use]
    pub const fn with_max_working_bytes(mut self, value: u64) -> Self {
        self.max_working_bytes = value;
        self
    }

    /// Sets the cumulative commit staging-byte ceiling.
    #[must_use]
    pub const fn with_max_temporary_bytes(mut self, value: u64) -> Self {
        self.max_temporary_bytes = value;
        self
    }

    /// Sets the maximum final immutable Revision file length.
    #[must_use]
    pub const fn with_max_revision_bytes(mut self, value: u64) -> Self {
        self.max_revision_bytes = value;
        self
    }

    /// Sets the maximum total durable Workspace bytes after publication.
    #[must_use]
    pub const fn with_max_total_durable_bytes(mut self, value: u64) -> Self {
        self.max_total_durable_bytes = value;
        self
    }

    /// Returns the maximum selected Points inspected by one commit.
    #[must_use]
    pub const fn max_selected_points(self) -> u64 {
        self.max_selected_points
    }

    /// Returns the maximum Points whose effective value may change.
    #[must_use]
    pub const fn max_changed_points(self) -> u64 {
        self.max_changed_points
    }

    /// Returns the maximum Point Set or prior-Revision frames consumed.
    #[must_use]
    pub const fn max_input_frames(self) -> u64 {
        self.max_input_frames
    }

    /// Returns the maximum change rows in one checksummed Revision block.
    #[must_use]
    pub const fn max_block_points(self) -> u64 {
        self.max_block_points
    }

    /// Returns the maximum encoded bytes in one Revision block.
    #[must_use]
    pub const fn max_block_bytes(self) -> u64 {
        self.max_block_bytes
    }

    /// Returns the combined peak commit working-memory ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }

    /// Returns the cumulative commit staging-byte ceiling.
    #[must_use]
    pub const fn max_temporary_bytes(self) -> u64 {
        self.max_temporary_bytes
    }

    /// Returns the maximum final immutable Revision file length.
    #[must_use]
    pub const fn max_revision_bytes(self) -> u64 {
        self.max_revision_bytes
    }

    /// Returns the maximum total durable Workspace bytes after publication.
    #[must_use]
    pub const fn max_total_durable_bytes(self) -> u64 {
        self.max_total_durable_bytes
    }
}

impl Default for CommitLimits {
    fn default() -> Self {
        Self::new()
            .with_max_selected_points(10_000_000)
            .with_max_changed_points(10_000_000)
            .with_max_input_frames(1_000_000)
            .with_max_block_points(65_536)
            .with_max_block_bytes(4 * MIB)
            .with_max_working_bytes(128 * MIB)
            .with_max_temporary_bytes(2 * GIB)
            .with_max_revision_bytes(2 * GIB)
            .with_max_total_durable_bytes(64 * GIB)
    }
}

pub(crate) fn require(
    required: u64,
    allowed: u64,
    limit: &'static str,
) -> Result<(), WorkspaceError> {
    if required > allowed {
        return Err(WorkspaceError::ResourceLimit {
            limit,
            required,
            allowed,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use point_index::CandidateLimits;
    use point_source::ReadBudget;

    use super::{CommitLimits, PointIdReadLimits, PointSetLimits};

    #[test]
    fn zero_resident_bytes_expresses_forced_spill_without_hidden_minimum() {
        let limits = PointSetLimits::new(
            CandidateLimits::new(1, 1, 1, 1),
            ReadBudget::new(1, 1).expect("nonzero Source batch limits"),
            1,
            1,
            1,
            1,
            1,
            0,
            9,
        );
        assert_eq!(limits.max_resident_bytes(), 0);
        assert_eq!(limits.max_temporary_bytes(), 9);
        assert_eq!(limits.max_overlay_bytes(), 1);
    }

    #[test]
    fn point_id_read_limits_keep_total_batch_and_buffer_caps_separate() {
        let limits = PointIdReadLimits::new(10, 4, 160, 80, 240);
        assert_eq!(limits.max_points(), 10);
        assert_eq!(limits.max_batch_points(), 4);
        assert_eq!(limits.max_batch_bytes(), 160);
        assert_eq!(limits.max_read_buffer_bytes(), 80);
        assert_eq!(limits.max_working_bytes(), 240);
    }

    #[test]
    fn commit_limits_charge_selected_changed_and_durable_growth_separately() {
        let limits = CommitLimits::new()
            .with_max_selected_points(10)
            .with_max_changed_points(4)
            .with_max_input_frames(2)
            .with_max_block_points(3)
            .with_max_block_bytes(30)
            .with_max_working_bytes(40)
            .with_max_temporary_bytes(50)
            .with_max_revision_bytes(60)
            .with_max_total_durable_bytes(70);
        assert_eq!(limits.max_selected_points(), 10);
        assert_eq!(limits.max_changed_points(), 4);
        assert_eq!(limits.max_input_frames(), 2);
        assert_eq!(limits.max_total_durable_bytes(), 70);
    }
}
