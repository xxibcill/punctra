use std::mem;

use foundation_runtime::{
    BatchStream, OperationControl, OperationHandle, ProgressPhase, ProgressSnapshot,
};
use point_contracts::{AttributeColumns, PointId, PositionTransform, SourceId, SourceProvenance};
use point_source::{
    AttributeSelection, PointBatches, ReadBudget, ReadRequest, SourceProvenanceHandle, SourceSpan,
};

use crate::{
    DisplayAttributes, DisplayCoverage, DisplaySampleContract, IndexError, IndexLimit, IndexNode,
    IndexNodeId, NodeReadBudget, PreparedIndex, limits::require,
};

const SOURCE_POSITION_BYTES: u64 = 24;
const POSITION_SAMPLE_BYTES: u64 = 32;
const ATTRIBUTED_SAMPLE_BYTES: u64 = 42;

/// One exact sparse display sample retained in canonical Source identity space.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct IndexSample {
    ordinal: u64,
    ticks: [i64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StoredSample {
    encoded: [u8; 42],
}

impl StoredSample {
    pub(crate) const fn position_only(ordinal: u64, ticks: [i64; 3]) -> Self {
        let mut encoded = [0; 42];
        let ordinal_bytes = ordinal.to_le_bytes();
        let mut byte = 0;
        while byte < 8 {
            encoded[byte] = ordinal_bytes[byte];
            byte += 1;
        }
        let mut axis = 0;
        while axis < 3 {
            let tick_bytes = ticks[axis].to_le_bytes();
            let mut tick_byte = 0;
            while tick_byte < 8 {
                encoded[8 + axis * 8 + tick_byte] = tick_bytes[tick_byte];
                tick_byte += 1;
            }
            axis += 1;
        }
        Self { encoded }
    }

    pub(crate) const fn attributed(
        ordinal: u64,
        ticks: [i64; 3],
        attributes: DisplayAttributes,
    ) -> Self {
        let mut stored = Self::position_only(ordinal, ticks);
        let intensity = attributes.intensity().to_le_bytes();
        stored.encoded[32] = intensity[0];
        stored.encoded[33] = intensity[1];
        stored.encoded[34] = attributes.classification();
        stored.encoded[35] = 1;
        let rgb = attributes.rgb();
        let mut channel = 0;
        while channel < 3 {
            let channel_bytes = rgb[channel].to_le_bytes();
            stored.encoded[36 + channel * 2] = channel_bytes[0];
            stored.encoded[37 + channel * 2] = channel_bytes[1];
            channel += 1;
        }
        stored
    }

    pub(crate) const fn sample(self) -> IndexSample {
        IndexSample::new(self.ordinal(), self.ticks())
    }

    pub(crate) const fn attributes(self) -> Option<DisplayAttributes> {
        if self.encoded[35] == 0 {
            return None;
        }
        Some(DisplayAttributes::new(
            u16::from_le_bytes([self.encoded[32], self.encoded[33]]),
            self.encoded[34],
            [
                u16::from_le_bytes([self.encoded[36], self.encoded[37]]),
                u16::from_le_bytes([self.encoded[38], self.encoded[39]]),
                u16::from_le_bytes([self.encoded[40], self.encoded[41]]),
            ],
        ))
    }

    pub(crate) const fn ordinal(self) -> u64 {
        u64::from_le_bytes([
            self.encoded[0],
            self.encoded[1],
            self.encoded[2],
            self.encoded[3],
            self.encoded[4],
            self.encoded[5],
            self.encoded[6],
            self.encoded[7],
        ])
    }

    pub(crate) const fn ticks(self) -> [i64; 3] {
        [
            i64::from_le_bytes(self.tick_bytes(0)),
            i64::from_le_bytes(self.tick_bytes(1)),
            i64::from_le_bytes(self.tick_bytes(2)),
        ]
    }

    pub(crate) fn world_position(self, transform: PositionTransform) -> [f64; 3] {
        transform.world_f64(self.ticks())
    }

    pub(crate) const fn wire_bytes(self) -> [u8; 42] {
        let mut wire = self.encoded;
        wire[35] = 0;
        wire
    }

    const fn tick_bytes(self, axis: usize) -> [u8; 8] {
        let start = 8 + axis * 8;
        [
            self.encoded[start],
            self.encoded[start + 1],
            self.encoded[start + 2],
            self.encoded[start + 3],
            self.encoded[start + 4],
            self.encoded[start + 5],
            self.encoded[start + 6],
            self.encoded[start + 7],
        ]
    }
}

impl Ord for StoredSample {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.ordinal(), self.ticks(), self.attributes()).cmp(&(
            other.ordinal(),
            other.ticks(),
            other.attributes(),
        ))
    }
}

impl PartialOrd for StoredSample {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
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
    samples: Vec<IndexSample>,
    display_attributes: Option<Vec<DisplayAttributes>>,
}

impl IndexPointBatch {
    fn position_only_samples(
        source: SourceId,
        transform: PositionTransform,
        node: IndexNodeId,
        rows: &[IndexSample],
        max_display_batch_bytes: u64,
    ) -> Result<Self, IndexError> {
        let mut samples =
            reserve_display_values::<IndexSample>(rows.len(), 0, max_display_batch_bytes)?;
        samples.extend_from_slice(rows);
        debug_assert!(!samples.is_empty());
        debug_assert!(
            samples
                .windows(2)
                .all(|pair| pair[0].ordinal() < pair[1].ordinal())
        );
        Ok(Self {
            source,
            transform,
            node,
            samples,
            display_attributes: None,
        })
    }

    fn position_only_ticks(
        source: SourceId,
        transform: PositionTransform,
        node: IndexNodeId,
        first_ordinal: u64,
        ticks: &[[i64; 3]],
        max_display_batch_bytes: u64,
    ) -> Result<Self, IndexError> {
        let last_row = u64::try_from(ticks.len().saturating_sub(1)).map_err(|_| {
            IndexError::CorruptArtifact {
                reason: "Source batch row count is not addressable",
            }
        })?;
        first_ordinal
            .checked_add(last_row)
            .ok_or(IndexError::CorruptArtifact {
                reason: "Source batch ordinal overflowed",
            })?;
        let mut samples =
            reserve_display_values::<IndexSample>(ticks.len(), 0, max_display_batch_bytes)?;
        samples.extend(ticks.iter().copied().enumerate().map(|(row, ticks)| {
            IndexSample::new(
                first_ordinal + u64::try_from(row).expect("preflighted Source batch rows fit u64"),
                ticks,
            )
        }));
        debug_assert!(!samples.is_empty());
        Ok(Self {
            source,
            transform,
            node,
            samples,
            display_attributes: None,
        })
    }

    fn attributed<I>(
        source: SourceId,
        transform: PositionTransform,
        node: IndexNodeId,
        sample_count: usize,
        max_display_batch_bytes: u64,
        rows: I,
    ) -> Result<Self, IndexError>
    where
        I: IntoIterator<Item = Result<StoredSample, IndexError>>,
    {
        let attribute_bytes = requested_allocation_bytes::<DisplayAttributes>(sample_count);
        let mut samples = reserve_display_values::<IndexSample>(
            sample_count,
            attribute_bytes,
            max_display_batch_bytes,
        )?;
        let sample_capacity_bytes = allocated_bytes::<IndexSample>(samples.capacity());
        let mut display_attributes = reserve_display_values::<DisplayAttributes>(
            sample_count,
            sample_capacity_bytes,
            max_display_batch_bytes,
        )?;
        for row in rows {
            if samples.len() == sample_count {
                return Err(IndexError::CorruptArtifact {
                    reason: "attributed display batch produced more rows than declared",
                });
            }
            let stored = row?;
            let Some(attributes) = stored.attributes() else {
                return Err(IndexError::CorruptArtifact {
                    reason: "attributed display batch omitted its raw Attribute row",
                });
            };
            samples.push(stored.sample());
            display_attributes.push(attributes);
        }
        if samples.len() != sample_count {
            return Err(IndexError::CorruptArtifact {
                reason: "attributed display batch produced fewer rows than declared",
            });
        }
        debug_assert!(!samples.is_empty());
        debug_assert!(
            samples
                .windows(2)
                .all(|pair| pair[0].ordinal() < pair[1].ordinal())
        );
        Ok(Self {
            source,
            transform,
            node,
            samples,
            display_attributes: Some(display_attributes),
        })
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

    /// Returns row-aligned raw inspection values for a v2 attributed batch.
    #[must_use]
    pub fn display_attributes(&self) -> Option<&[DisplayAttributes]> {
        self.display_attributes.as_deref()
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
        let sample_bytes = if self.display_attributes.is_some() {
            ATTRIBUTED_SAMPLE_BYTES
        } else {
            POSITION_SAMPLE_BYTES
        };
        u64::try_from(self.samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(sample_bytes)
    }
}

fn reserve_display_values<T>(
    count: usize,
    already_allocated_bytes: u64,
    allowed: u64,
) -> Result<Vec<T>, IndexError> {
    let requested = requested_allocation_bytes::<T>(count);
    require(
        already_allocated_bytes.saturating_add(requested),
        allowed,
        IndexLimit::DisplayBatchBytes,
    )?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::DisplayBatchBytes,
            required: already_allocated_bytes.saturating_add(requested),
            allowed,
        })?;
    require(
        already_allocated_bytes.saturating_add(allocated_bytes::<T>(values.capacity())),
        allowed,
        IndexLimit::DisplayBatchBytes,
    )?;
    Ok(values)
}

fn requested_allocation_bytes<T>(count: usize) -> u64 {
    u64::try_from(count)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
}

fn allocated_bytes<T>(capacity: usize) -> u64 {
    requested_allocation_bytes::<T>(capacity)
}

/// Exact terminal facts for one successfully completed node read.
#[derive(Clone, Debug)]
pub struct IndexReadSummary {
    node: IndexNodeId,
    emitted_point_count: u64,
    covered_source_point_count: u64,
    source: SourceId,
    provenance: SourceProvenanceHandle,
    coverage: DisplayCoverage,
    display_sample_contract: Option<DisplaySampleContract>,
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
        self.source
    }

    /// Returns immutable detached Source provenance.
    #[must_use]
    pub fn provenance(&self) -> &SourceProvenance {
        self.provenance.get()
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

    /// Returns the raw inspection sample contract for an attributed read.
    #[must_use]
    pub const fn display_sample_contract(&self) -> Option<DisplaySampleContract> {
        self.display_sample_contract
    }
}

impl PartialEq for IndexReadSummary {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
            && self.emitted_point_count == other.emitted_point_count
            && self.covered_source_point_count == other.covered_source_point_count
            && self.source == other.source
            && self.provenance == other.provenance
            && self.coverage == other.coverage
            && self.display_sample_contract == other.display_sample_contract
    }
}

impl Eq for IndexReadSummary {}

enum ReadState {
    InternalPosition {
        samples: Vec<IndexSample>,
        next_index: usize,
    },
    InternalAttributed {
        samples: Vec<StoredSample>,
        next_index: usize,
    },
    Leaf(Box<PointBatches>),
}

/// Pull-based bounded stream of display-only index batches.
pub struct IndexPointBatches {
    source: SourceId,
    provenance: SourceProvenanceHandle,
    transform: PositionTransform,
    node: IndexNodeId,
    coverage: DisplayCoverage,
    display_sample_contract: Option<DisplaySampleContract>,
    covered_source_point_count: u64,
    expected_point_count: u64,
    emitted_point_count: u64,
    max_batch_points: u64,
    max_display_batch_bytes: u64,
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

    #[allow(clippy::too_many_lines)]
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
            ReadState::InternalPosition {
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
                let batch = IndexPointBatch::position_only_samples(
                    self.source,
                    self.transform,
                    self.node,
                    &samples[*next_index..end],
                    self.max_display_batch_bytes,
                );
                let batch = match batch {
                    Ok(batch) => batch,
                    Err(error) => return self.fail(error),
                };
                *next_index = end;
                self.publish_batch(batch)
            }
            ReadState::InternalAttributed {
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
                let batch = IndexPointBatch::attributed(
                    self.source,
                    self.transform,
                    self.node,
                    count,
                    self.max_display_batch_bytes,
                    samples[*next_index..end].iter().copied().map(Ok),
                );
                let batch = match batch {
                    Ok(batch) => batch,
                    Err(error) => return self.fail(error),
                };
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
                        let contract = self.display_sample_contract;
                        let sample_count = batch.positions().ticks().len();
                        if let Some(contract) = contract {
                            let rows = batch.positions().ticks().iter().copied().enumerate().map(
                                |(row, ticks)| -> Result<StoredSample, IndexError> {
                                    let ordinal_row = u64::try_from(row).map_err(|_| {
                                        IndexError::CorruptArtifact {
                                            reason: "Source batch row is not addressable",
                                        }
                                    })?;
                                    let ordinal = first.checked_add(ordinal_row).ok_or(
                                        IndexError::CorruptArtifact {
                                            reason: "Source batch ordinal overflowed",
                                        },
                                    )?;
                                    Ok(StoredSample::attributed(
                                        ordinal,
                                        ticks,
                                        attributes_at(batch.attributes(), row, contract)?,
                                    ))
                                },
                            );
                            match IndexPointBatch::attributed(
                                self.source,
                                self.transform,
                                self.node,
                                sample_count,
                                self.max_display_batch_bytes,
                                rows,
                            ) {
                                Ok(batch) => self.publish_batch(batch),
                                Err(error) => self.fail(error),
                            }
                        } else {
                            match IndexPointBatch::position_only_ticks(
                                self.source,
                                self.transform,
                                self.node,
                                first,
                                batch.positions().ticks(),
                                self.max_display_batch_bytes,
                            ) {
                                Ok(batch) => self.publish_batch(batch),
                                Err(error) => self.fail(error),
                            }
                        }
                    }
                    Ok(None) => {
                        let Some(summary) = source.summary() else {
                            return self.fail(IndexError::CorruptArtifact {
                                reason: "Source leaf ended without a terminal summary",
                            });
                        };
                        if summary.source() != self.source
                            || summary.provenance() != self.provenance.get()
                            || summary.exact_count() != self.expected_point_count
                            || summary.spans() != self.expected_source_span.as_slice()
                            || !selected_attributes_match(
                                self.display_sample_contract,
                                summary.attributes(),
                            )
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
        batch: IndexPointBatch,
    ) -> Result<Option<IndexPointBatch>, IndexError> {
        self.publish_count(batch.len())?;
        Ok(Some(batch))
    }

    fn publish_count(&mut self, sample_count: usize) -> Result<(), IndexError> {
        let count = u64::try_from(sample_count).unwrap_or(u64::MAX);
        let emitted = self.emitted_point_count.saturating_add(count);
        if sample_count == 0 || emitted > self.expected_point_count {
            self.summary = None;
            self.terminal = true;
            return Err(IndexError::CorruptArtifact {
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
            Err(error) => {
                self.summary = None;
                self.terminal = true;
                return Err(error.into());
            }
        };
        if let Err(error) = self.control.report_progress(progress) {
            self.summary = None;
            self.terminal = true;
            return Err(error.into());
        }
        Ok(())
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
            source: self.source,
            provenance: self.provenance.clone(),
            coverage: self.coverage,
            display_sample_contract: self.display_sample_contract,
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
    let contract = index.descriptor.display_sample_contract;
    let sample_bytes = index.descriptor.recipe.sample_bytes();
    validate_budget(node, budget, contract, sample_bytes)?;
    let max_batch_points = max_batch_points(node, budget, sample_bytes)?;
    let state = if let Some(span) = node.source_span {
        let source_budget = ReadBudget::new(
            max_batch_points.min(budget.max_source_batch_points()),
            budget.max_source_batch_payload_bytes(),
        )?
        .with_max_spans(budget.max_source_spans())
        .with_max_points(node.display_point_count)
        .with_max_adapter_working_bytes(budget.max_adapter_working_bytes());
        let attributes = match contract {
            Some(contract) => AttributeSelection::only(contract.selected_ids()),
            None => AttributeSelection::only([]),
        };
        let request = ReadRequest::all()
            .spans([span])
            .attributes(attributes)
            .budget(source_budget);
        ReadState::Leaf(Box::new(index.source.read(request)?))
    } else if contract.is_some() {
        let samples = index.artifact.read_sample_block(
            node.sample_offset,
            node.display_point_count,
            node.sample_checksum,
            budget.max_index_buffer_bytes(),
        )?;
        ReadState::InternalAttributed {
            samples,
            next_index: 0,
        }
    } else {
        let samples = index.artifact.read_position_sample_block(
            node.sample_offset,
            node.display_point_count,
            node.sample_checksum,
            budget.max_index_buffer_bytes(),
        )?;
        ReadState::InternalPosition {
            samples,
            next_index: 0,
        }
    };
    Ok(IndexPointBatches {
        source: index.descriptor.source,
        provenance: index.source.provenance_handle(),
        transform: index.descriptor.position_transform,
        node: node.id,
        coverage: node.coverage,
        display_sample_contract: contract,
        covered_source_point_count: node.covered_point_count,
        expected_point_count: node.display_point_count,
        emitted_point_count: 0,
        max_batch_points,
        max_display_batch_bytes: budget.max_display_batch_bytes(),
        expected_source_span: node.source_span,
        state,
        control: OperationControl::new(),
        summary: None,
        terminal: false,
    })
}

fn selected_attributes_match(
    contract: Option<DisplaySampleContract>,
    actual: &[point_contracts::AttributeId],
) -> bool {
    match contract {
        Some(contract) => actual.iter().copied().eq(contract.selected_ids()),
        None => actual.is_empty(),
    }
}

fn validate_budget(
    node: &IndexNode,
    budget: NodeReadBudget,
    contract: Option<DisplaySampleContract>,
    sample_bytes: u64,
) -> Result<(), IndexError> {
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
            contract.map_or(
                SOURCE_POSITION_BYTES,
                DisplaySampleContract::source_bytes_per_point,
            ),
            budget.max_source_batch_payload_bytes(),
            IndexLimit::SourceBatchPayloadBytes,
        )?;
    }
    require(
        sample_bytes,
        budget.max_display_batch_bytes(),
        IndexLimit::DisplayBatchBytes,
    )?;
    if node.source_span.is_none() {
        require(
            node.display_point_count.saturating_mul(sample_bytes),
            budget.max_index_buffer_bytes(),
            IndexLimit::IndexBufferBytes,
        )?;
    }
    Ok(())
}

fn max_batch_points(
    node: &IndexNode,
    budget: NodeReadBudget,
    sample_bytes: u64,
) -> Result<u64, IndexError> {
    let display = budget.max_display_batch_bytes() / sample_bytes;
    let mut points = display.min(node.display_point_count);
    if node.source_span.is_some() {
        points = points.min(budget.max_source_batch_points());
    } else {
        points = points.min(budget.max_index_buffer_bytes() / sample_bytes);
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

pub(crate) fn attributes_at(
    columns: &AttributeColumns,
    row: usize,
    contract: DisplaySampleContract,
) -> Result<DisplayAttributes, IndexError> {
    let intensity = columns
        .get(contract.intensity())
        .and_then(|column| column.values().as_u16())
        .and_then(|values| values.get(row))
        .copied()
        .ok_or(IndexError::CorruptArtifact {
            reason: "Source leaf omitted or mistyped inspection intensity values",
        })?;
    let classification = columns
        .get(contract.classification())
        .and_then(|column| column.values().as_u8())
        .and_then(|values| values.get(row))
        .copied()
        .ok_or(IndexError::CorruptArtifact {
            reason: "Source leaf omitted or mistyped inspection classification values",
        })?;
    let rgb = match contract.rgb() {
        Some(ids) => {
            let mut rgb = [0; 3];
            for (channel, id) in ids.into_iter().enumerate() {
                rgb[channel] = columns
                    .get(id)
                    .and_then(|column| column.values().as_u16())
                    .and_then(|values| values.get(row))
                    .copied()
                    .ok_or(IndexError::CorruptArtifact {
                        reason: "Source leaf omitted or mistyped inspection RGB values",
                    })?;
            }
            rgb
        }
        None => [0; 3],
    };
    Ok(DisplayAttributes::new(intensity, classification, rgb))
}
