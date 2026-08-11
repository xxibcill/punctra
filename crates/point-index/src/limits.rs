use crate::{IndexError, IndexLimit};

const DEFAULT_BATCH_POINTS: u64 = 65_536;
const DEFAULT_BATCH_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_ADAPTER_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_BUILD_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_INCOMPLETE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const DEFAULT_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const DEFAULT_HIERARCHY_NODES: u64 = 4_194_303;
const DEFAULT_METADATA_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_CANDIDATE_SPANS: u64 = 65_536;
const DEFAULT_CANDIDATE_POINTS: u64 = u64::MAX;
const DEFAULT_CANDIDATE_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_DISPLAY_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_INDEX_BUFFER_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn require(required: u64, allowed: u64, limit: IndexLimit) -> Result<(), IndexError> {
    if required > allowed {
        return Err(IndexError::ResourceLimit {
            limit,
            required,
            allowed,
        });
    }
    Ok(())
}

/// Separate hard limits for index preparation and opening.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct PrepareLimits {
    max_source_batch_points: u64,
    max_source_batch_payload_bytes: u64,
    max_adapter_working_bytes: u64,
    max_build_working_bytes: u64,
    max_incomplete_bytes: u64,
    max_artifact_bytes: u64,
    max_hierarchy_nodes: u64,
    max_resident_metadata_bytes: u64,
}

impl PrepareLimits {
    /// Creates limits with explicit nonzero Source batch ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::InvalidLimit`] when either value is zero.
    pub const fn new(
        max_source_batch_points: u64,
        max_source_batch_payload_bytes: u64,
    ) -> Result<Self, IndexError> {
        if max_source_batch_points == 0 {
            return Err(IndexError::InvalidLimit {
                limit: IndexLimit::MaxSourceBatchPoints,
            });
        }
        if max_source_batch_payload_bytes == 0 {
            return Err(IndexError::InvalidLimit {
                limit: IndexLimit::MaxSourceBatchPayloadBytes,
            });
        }
        Ok(Self {
            max_source_batch_points,
            max_source_batch_payload_bytes,
            max_adapter_working_bytes: DEFAULT_ADAPTER_BYTES,
            max_build_working_bytes: DEFAULT_BUILD_BYTES,
            max_incomplete_bytes: DEFAULT_INCOMPLETE_BYTES,
            max_artifact_bytes: DEFAULT_ARTIFACT_BYTES,
            max_hierarchy_nodes: DEFAULT_HIERARCHY_NODES,
            max_resident_metadata_bytes: DEFAULT_METADATA_BYTES,
        })
    }

    /// Sets the adapter decoder-memory ceiling. Zero permits no separate adapter memory.
    #[must_use]
    pub const fn with_max_adapter_working_bytes(mut self, value: u64) -> Self {
        self.max_adapter_working_bytes = value;
        self
    }

    /// Sets the index builder's separate working-memory ceiling.
    #[must_use]
    pub const fn with_max_build_working_bytes(mut self, value: u64) -> Self {
        self.max_build_working_bytes = value;
        self
    }

    /// Sets the maximum append-only incomplete-file size.
    #[must_use]
    pub const fn with_max_incomplete_bytes(mut self, value: u64) -> Self {
        self.max_incomplete_bytes = value;
        self
    }

    /// Sets the maximum complete-artifact size.
    #[must_use]
    pub const fn with_max_artifact_bytes(mut self, value: u64) -> Self {
        self.max_artifact_bytes = value;
        self
    }

    /// Sets the maximum hierarchy node count.
    #[must_use]
    pub const fn with_max_hierarchy_nodes(mut self, value: u64) -> Self {
        self.max_hierarchy_nodes = value;
        self
    }

    /// Sets the maximum resident hierarchy-metadata bytes.
    #[must_use]
    pub const fn with_max_resident_metadata_bytes(mut self, value: u64) -> Self {
        self.max_resident_metadata_bytes = value;
        self
    }

    /// Returns the Source batch Point ceiling.
    #[must_use]
    pub const fn max_source_batch_points(self) -> u64 {
        self.max_source_batch_points
    }

    /// Returns the Source batch payload-byte ceiling.
    #[must_use]
    pub const fn max_source_batch_payload_bytes(self) -> u64 {
        self.max_source_batch_payload_bytes
    }

    /// Returns the adapter decoder-memory ceiling.
    #[must_use]
    pub const fn max_adapter_working_bytes(self) -> u64 {
        self.max_adapter_working_bytes
    }

    /// Returns the index builder working-memory ceiling.
    #[must_use]
    pub const fn max_build_working_bytes(self) -> u64 {
        self.max_build_working_bytes
    }

    /// Returns the incomplete-file byte ceiling.
    #[must_use]
    pub const fn max_incomplete_bytes(self) -> u64 {
        self.max_incomplete_bytes
    }

    /// Returns the complete-artifact byte ceiling.
    #[must_use]
    pub const fn max_artifact_bytes(self) -> u64 {
        self.max_artifact_bytes
    }

    /// Returns the hierarchy node ceiling.
    #[must_use]
    pub const fn max_hierarchy_nodes(self) -> u64 {
        self.max_hierarchy_nodes
    }

    /// Returns the resident hierarchy-metadata byte ceiling.
    #[must_use]
    pub const fn max_resident_metadata_bytes(self) -> u64 {
        self.max_resident_metadata_bytes
    }
}

impl Default for PrepareLimits {
    fn default() -> Self {
        Self::new(DEFAULT_BATCH_POINTS, DEFAULT_BATCH_BYTES)
            .expect("nonzero default preparation limits")
    }
}

/// Hard limits for one conservative candidate plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct CandidateLimits {
    max_visited_nodes: u64,
    max_output_spans: u64,
    max_candidate_points: u64,
    max_working_bytes: u64,
}

impl CandidateLimits {
    /// Creates caller-selected ceilings. Zero permits only empty work/output.
    #[must_use]
    pub const fn new(
        max_visited_nodes: u64,
        max_output_spans: u64,
        max_candidate_points: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            max_visited_nodes,
            max_output_spans,
            max_candidate_points,
            max_working_bytes,
        }
    }

    /// Returns the hierarchy-visit ceiling.
    #[must_use]
    pub const fn max_visited_nodes(self) -> u64 {
        self.max_visited_nodes
    }

    /// Returns the retained Source-span ceiling.
    #[must_use]
    pub const fn max_output_spans(self) -> u64 {
        self.max_output_spans
    }

    /// Returns the candidate Point ceiling.
    #[must_use]
    pub const fn max_candidate_points(self) -> u64 {
        self.max_candidate_points
    }

    /// Returns the candidate-planner working-byte ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for CandidateLimits {
    fn default() -> Self {
        Self::new(
            DEFAULT_HIERARCHY_NODES,
            DEFAULT_CANDIDATE_SPANS,
            DEFAULT_CANDIDATE_POINTS,
            DEFAULT_CANDIDATE_BYTES,
        )
    }
}

/// Separate hard limits for one node materialization stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct NodeReadBudget {
    max_emitted_points: u64,
    max_source_spans: u64,
    max_source_batch_points: u64,
    max_source_batch_payload_bytes: u64,
    max_display_batch_bytes: u64,
    max_index_buffer_bytes: u64,
    max_adapter_working_bytes: u64,
}

impl NodeReadBudget {
    /// Creates limits with explicit nonzero total-Point and display-batch ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::InvalidLimit`] when either value is zero.
    pub const fn new(
        max_emitted_points: u64,
        max_display_batch_bytes: u64,
    ) -> Result<Self, IndexError> {
        if max_emitted_points == 0 {
            return Err(IndexError::InvalidLimit {
                limit: IndexLimit::MaxEmittedPoints,
            });
        }
        if max_display_batch_bytes == 0 {
            return Err(IndexError::InvalidLimit {
                limit: IndexLimit::MaxDisplayBatchBytes,
            });
        }
        Ok(Self {
            max_emitted_points,
            max_source_spans: 1,
            max_source_batch_points: DEFAULT_BATCH_POINTS,
            max_source_batch_payload_bytes: DEFAULT_BATCH_BYTES,
            max_display_batch_bytes,
            max_index_buffer_bytes: DEFAULT_INDEX_BUFFER_BYTES,
            max_adapter_working_bytes: DEFAULT_ADAPTER_BYTES,
        })
    }

    /// Sets the Source-span ceiling.
    #[must_use]
    pub const fn with_max_source_spans(mut self, value: u64) -> Self {
        self.max_source_spans = value;
        self
    }

    /// Sets the Source batch Point ceiling.
    #[must_use]
    pub const fn with_max_source_batch_points(mut self, value: u64) -> Self {
        self.max_source_batch_points = value;
        self
    }

    /// Sets the Source batch payload-byte ceiling.
    #[must_use]
    pub const fn with_max_source_batch_payload_bytes(mut self, value: u64) -> Self {
        self.max_source_batch_payload_bytes = value;
        self
    }

    /// Sets the index reader's separate buffer ceiling.
    #[must_use]
    pub const fn with_max_index_buffer_bytes(mut self, value: u64) -> Self {
        self.max_index_buffer_bytes = value;
        self
    }

    /// Sets the adapter decoder-memory ceiling.
    #[must_use]
    pub const fn with_max_adapter_working_bytes(mut self, value: u64) -> Self {
        self.max_adapter_working_bytes = value;
        self
    }

    /// Returns the total emitted Point ceiling.
    #[must_use]
    pub const fn max_emitted_points(self) -> u64 {
        self.max_emitted_points
    }

    /// Returns the Source-span ceiling.
    #[must_use]
    pub const fn max_source_spans(self) -> u64 {
        self.max_source_spans
    }

    /// Returns the Source batch Point ceiling.
    #[must_use]
    pub const fn max_source_batch_points(self) -> u64 {
        self.max_source_batch_points
    }

    /// Returns the Source batch payload-byte ceiling.
    #[must_use]
    pub const fn max_source_batch_payload_bytes(self) -> u64 {
        self.max_source_batch_payload_bytes
    }

    /// Returns the display-batch byte ceiling.
    #[must_use]
    pub const fn max_display_batch_bytes(self) -> u64 {
        self.max_display_batch_bytes
    }

    /// Returns the index reader buffer ceiling.
    #[must_use]
    pub const fn max_index_buffer_bytes(self) -> u64 {
        self.max_index_buffer_bytes
    }

    /// Returns the adapter decoder-memory ceiling.
    #[must_use]
    pub const fn max_adapter_working_bytes(self) -> u64 {
        self.max_adapter_working_bytes
    }
}

impl Default for NodeReadBudget {
    fn default() -> Self {
        Self::new(DEFAULT_BATCH_POINTS, DEFAULT_DISPLAY_BYTES)
            .expect("nonzero default node-read limits")
    }
}
