use foundation_runtime::{
    BatchStream, OperationControl, OperationHandle, ProgressPhase, ProgressSnapshot,
};
use point_contracts::{PointId, PositionTransform, SourceId, SourceProvenance};
use point_source::{AttributeSelection, PointBatches, ReadBudget, ReadRequest, SourceSpan};

use crate::{
    DisplayCoverage, IndexError, IndexLimit, IndexNode, IndexNodeId, NodeReadBudget, PreparedIndex,
    limits::require,
};

const SAMPLE_BYTES: u64 = 32;
const SOURCE_POSITION_BYTES: u64 = 24;

/// One exact sparse display sample retained in canonical Source identity space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndexSample {
    ordinal: u64,
    ticks: [i64; 3],
}

impl IndexSample {
    pub(crate) const fn new(ordinal: u64, ticks: [i64; 3]) -> Self {
        Self { ordinal, ticks }
    }

    /// Returns the canonical zero-based Source ordinal.
    #[must_use]
    pub const fn ordinal(self) -> u64 {
        self.ordinal
    }

    /// Returns exact signed Source position ticks.
    #[must_use]
    pub const fn ticks(self) -> [i64; 3] {
        self.ticks
    }

    /// Reconstructs exact Point Identity using the batch's Source identity.
    #[must_use]
    pub const fn point_id(self, source: SourceId) -> PointId {
        PointId::new(source, self.ordinal)
    }

    /// Decodes this sample into finite Source world coordinates.
    #[must_use]
    pub fn world_position(self, transform: PositionTransform) -> [f64; 3] {
        transform.world_f64(self.ticks)
    }
}

/// Bounded display-only samples for one index node.
///
/// Samples are sorted and unique by Source ordinal. This is deliberately not
/// `point_contracts::PointBatch`: sampled internal nodes do not represent
/// complete Query Coverage.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexPointBatch {
    source: SourceId,
    transform: PositionTransform,
    node: IndexNodeId,
    samples: Box<[IndexSample]>,
}

impl IndexPointBatch {
    fn new(
        source: SourceId,
        transform: PositionTransform,
        node: IndexNodeId,
        samples: Vec<IndexSample>,
    ) -> Self {
        debug_assert!(!samples.is_empty());
        debug_assert!(
            samples
                .windows(2)
                .all(|pair| pair[0].ordinal < pair[1].ordinal)
        );
        Self {
            source,
            transform,
            node,
            samples: samples.into_boxed_slice(),
        }
    }

    /// Returns the verified immutable Source identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Returns the exact Source position transform.
    #[must_use]
    pub const fn transform(&self) -> PositionTransform {
        self.transform
    }

    /// Returns the hierarchy node being materialized.
    #[must_use]
    pub const fn node(&self) -> IndexNodeId {
        self.node
    }

    /// Returns sorted, unique, display-only samples.
    #[must_use]
    pub fn samples(&self) -> &[IndexSample] {
        &self.samples
    }

    /// Returns the number of samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Reports whether this batch is empty.
    ///
    /// Constructed batches are always nonempty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Returns the exact sample payload bytes charged to display staging.
    #[must_use]
    pub fn estimated_payload_bytes(&self) -> u64 {
        u64::try_from(self.samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(SAMPLE_BYTES)
    }
}

/// Exact terminal facts for one successfully completed node read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexReadSummary {
    node: IndexNodeId,
    emitted_point_count: u64,
    covered_source_point_count: u64,
    provenance: SourceProvenance,
    coverage: DisplayCoverage,
}

impl IndexReadSummary {
    /// Returns the materialized node.
    #[must_use]
    pub const fn node(&self) -> IndexNodeId {
        self.node
    }

    /// Returns the exact number of emitted display samples.
    #[must_use]
    pub const fn emitted_point_count(&self) -> u64 {
        self.emitted_point_count
    }

    /// Returns the number of authoritative Source Points covered by the node.
    #[must_use]
    pub const fn covered_source_point_count(&self) -> u64 {
        self.covered_source_point_count
    }

    /// Returns the verified immutable Source identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.provenance.source()
    }

    /// Returns immutable detached Source provenance.
    #[must_use]
    pub const fn provenance(&self) -> &SourceProvenance {
        &self.provenance
    }

    /// Returns sampled or complete display Coverage.
    #[must_use]
    pub const fn coverage(&self) -> DisplayCoverage {
        self.coverage
    }

    /// Reports whether every covered Source Point was emitted.
    #[must_use]
    pub const fn coverage_complete(&self) -> bool {
        self.coverage.is_complete()
    }
}

enum ReadState {
    Internal {
        samples: Vec<IndexSample>,
        next_index: usize,
    },
    Leaf(Box<PointBatches>),
}

/// Pull-based bounded stream of display-only index batches.
pub struct IndexPointBatches {
    source: SourceId,
    provenance: SourceProvenance,
    transform: PositionTransform,
    node: IndexNodeId,
    coverage: DisplayCoverage,
    covered_source_point_count: u64,
    expected_point_count: u64,
    emitted_point_count: u64,
    max_batch_points: u64,
    expected_source_span: Option<SourceSpan>,
    state: ReadState,
    control: OperationControl,
    summary: Option<IndexReadSummary>,
    terminal: bool,
}

impl IndexPointBatches {
    /// Returns the next bounded display-only batch, or terminal `None`.
    ///
    /// An error is returned once; subsequent calls are fused to `Ok(None)`.
    ///
    /// # Errors
    ///
    /// Returns cancellation, Source, artifact, or resource failures.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<IndexPointBatch>, IndexError> {
        <Self as BatchStream>::next(self)
    }

    /// Returns terminal facts only after successful completion.
    #[must_use]
    pub const fn summary(&self) -> Option<&IndexReadSummary> {
        self.summary.as_ref()
    }

    /// Returns a cloneable observation and cancellation handle.
    #[must_use]
    pub fn handle(&self) -> OperationHandle {
        self.control.handle()
    }

    fn pull(&mut self) -> Result<Option<IndexPointBatch>, IndexError> {
        if self.terminal {
            return Ok(None);
        }
        if let Err(error) = self.control.check_cancelled() {
            if let ReadState::Leaf(source) = &self.state {
                source.handle().cancel();
            }
            return self.fail(error.into());
        }
        match &mut self.state {
            ReadState::Internal {
                samples,
                next_index,
            } => {
                if *next_index == samples.len() {
                    return self.finish();
                }
                let count = usize::try_from(self.max_batch_points)
                    .unwrap_or(usize::MAX)
                    .min(samples.len() - *next_index);
                let Some(end) = next_index.checked_add(count) else {
                    return self.fail(IndexError::CorruptArtifact {
                        reason: "internal sample cursor overflowed",
                    });
                };
                let batch = samples[*next_index..end].to_vec();
                *next_index = end;
                self.publish_batch(batch)
            }
            ReadState::Leaf(source) => {
                let next = source.next();
                if let Err(error) = self.control.check_cancelled() {
                    source.handle().cancel();
                    return self.fail(error.into());
                }
                match next {
                    Ok(Some(batch)) => {
                        let first = batch.first_ordinal();
                        let samples = batch
                            .positions()
                            .ticks()
                            .iter()
                            .copied()
                            .enumerate()
                            .map(|(row, ticks)| {
                                let row = u64::try_from(row).expect("Point Batch rows fit u64");
                                IndexSample::new(first + row, ticks)
                            })
                            .collect();
                        self.publish_batch(samples)
                    }
                    Ok(None) => {
                        let Some(summary) = source.summary().cloned() else {
                            return self.fail(IndexError::CorruptArtifact {
                                reason: "Source leaf ended without a terminal summary",
                            });
                        };
                        if summary.source() != self.source
                            || summary.provenance() != &self.provenance
                            || summary.exact_count() != self.expected_point_count
                            || summary.spans() != self.expected_source_span.as_slice()
                            || !summary.attributes().is_empty()
                        {
                            return self.fail(IndexError::CorruptArtifact {
                                reason: "Source leaf summary differs from the index request",
                            });
                        }
                        self.finish()
                    }
                    Err(error) => self.fail(error.into()),
                }
            }
        }
    }

    fn publish_batch(
        &mut self,
        samples: Vec<IndexSample>,
    ) -> Result<Option<IndexPointBatch>, IndexError> {
        let count = u64::try_from(samples.len()).unwrap_or(u64::MAX);
        let emitted = self.emitted_point_count.saturating_add(count);
        if samples.is_empty() || emitted > self.expected_point_count {
            return self.fail(IndexError::CorruptArtifact {
                reason: "node stream emitted incoherent sample coverage",
            });
        }
        self.emitted_point_count = emitted;
        let progress = match ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            emitted,
            Some(self.expected_point_count),
        ) {
            Ok(progress) => progress,
            Err(error) => return self.fail(error.into()),
        };
        if let Err(error) = self.control.report_progress(progress) {
            return self.fail(error.into());
        }
        Ok(Some(IndexPointBatch::new(
            self.source,
            self.transform,
            self.node,
            samples,
        )))
    }

    fn finish(&mut self) -> Result<Option<IndexPointBatch>, IndexError> {
        if self.emitted_point_count != self.expected_point_count {
            return self.fail(IndexError::CorruptArtifact {
                reason: "node stream ended before expected display coverage",
            });
        }
        if let Err(error) = self.control.complete_progress(self.expected_point_count) {
            return self.fail(error.into());
        }
        self.summary = Some(IndexReadSummary {
            node: self.node,
            emitted_point_count: self.emitted_point_count,
            covered_source_point_count: self.covered_source_point_count,
            provenance: self.provenance.clone(),
            coverage: self.coverage,
        });
        self.terminal = true;
        Ok(None)
    }

    fn fail(&mut self, error: IndexError) -> Result<Option<IndexPointBatch>, IndexError> {
        self.summary = None;
        self.terminal = true;
        Err(error)
    }
}

impl BatchStream for IndexPointBatches {
    type Batch = IndexPointBatch;
    type Summary = IndexReadSummary;
    type Error = IndexError;

    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error> {
        self.pull()
    }

    fn summary(&self) -> Option<&Self::Summary> {
        self.summary()
    }

    fn handle(&self) -> OperationHandle {
        self.handle()
    }
}

pub(crate) fn start(
    index: &PreparedIndex,
    node: &IndexNode,
    budget: NodeReadBudget,
) -> Result<IndexPointBatches, IndexError> {
    validate_budget(node, budget)?;
    let max_batch_points = max_batch_points(node, budget)?;
    let state = if let Some(span) = node.source_span {
        let source_budget = ReadBudget::new(
            max_batch_points.min(budget.max_source_batch_points()),
            budget.max_source_batch_payload_bytes(),
        )?
        .with_max_spans(budget.max_source_spans())
        .with_max_points(node.display_point_count)
        .with_max_adapter_working_bytes(budget.max_adapter_working_bytes());
        let request = ReadRequest::all()
            .spans([span])
            .attributes(AttributeSelection::only([]))
            .budget(source_budget);
        ReadState::Leaf(Box::new(index.source.read(request)?))
    } else {
        let samples = index.artifact.read_sample_block(
            node.sample_offset,
            node.display_point_count,
            node.sample_checksum,
            budget.max_index_buffer_bytes(),
        )?;
        ReadState::Internal {
            samples,
            next_index: 0,
        }
    };
    Ok(IndexPointBatches {
        source: index.descriptor.source,
        provenance: index.source.provenance().clone(),
        transform: index.descriptor.position_transform,
        node: node.id,
        coverage: node.coverage,
        covered_source_point_count: node.covered_point_count,
        expected_point_count: node.display_point_count,
        emitted_point_count: 0,
        max_batch_points,
        expected_source_span: node.source_span,
        state,
        control: OperationControl::new(),
        summary: None,
        terminal: false,
    })
}

fn validate_budget(node: &IndexNode, budget: NodeReadBudget) -> Result<(), IndexError> {
    require(
        node.display_point_count,
        budget.max_emitted_points(),
        IndexLimit::EmittedDisplayPoints,
    )?;
    if node.source_span.is_some() {
        require(1, budget.max_source_spans(), IndexLimit::SourceSpans)?;
        require(
            1,
            budget.max_source_batch_points(),
            IndexLimit::SourceBatchPoints,
        )?;
        require(
            SOURCE_POSITION_BYTES,
            budget.max_source_batch_payload_bytes(),
            IndexLimit::SourceBatchPayloadBytes,
        )?;
    }
    require(
        SAMPLE_BYTES,
        budget.max_display_batch_bytes(),
        IndexLimit::DisplayBatchBytes,
    )?;
    if node.source_span.is_none() {
        require(
            node.display_point_count.saturating_mul(SAMPLE_BYTES),
            budget.max_index_buffer_bytes(),
            IndexLimit::IndexBufferBytes,
        )?;
    }
    Ok(())
}

fn max_batch_points(node: &IndexNode, budget: NodeReadBudget) -> Result<u64, IndexError> {
    let display = budget.max_display_batch_bytes() / SAMPLE_BYTES;
    let mut points = display.min(node.display_point_count);
    if node.source_span.is_some() {
        points = points.min(budget.max_source_batch_points());
    } else {
        points = points.min(budget.max_index_buffer_bytes() / SAMPLE_BYTES);
    }
    if points == 0 {
        return Err(IndexError::ResourceLimit {
            limit: IndexLimit::DisplayBatchPoints,
            required: 1,
            allowed: 0,
        });
    }
    Ok(points)
}
