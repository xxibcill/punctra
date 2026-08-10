use std::sync::Arc;

use foundation_runtime::{
    BatchStream, OperationControl, OperationHandle, ProgressPhase, ProgressSnapshot,
};
use point_contracts::{
    AttributeId, PointBatch, PositionTransform, SourceId, SourceMetadata, SourceProvenance,
    WorldBounds,
};

use crate::adapter::{AdapterRead, AdapterReadRequest, ReadAdapter};
use crate::{
    NormalizedRead, ReadBudget, ReadLimit, ReadRequest, SourceError, SourceSpan, normalize_request,
    publish_complete,
};

/// Exact terminal facts for one successfully completed Source read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceReadSummary {
    provenance: SourceProvenance,
    spans: Arc<[SourceSpan]>,
    exact_count: u64,
    attributes: Arc<[AttributeId]>,
    budget: ReadBudget,
}

impl SourceReadSummary {
    /// Returns the immutable Source Identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.provenance.source()
    }

    /// Returns immutable detached Source provenance.
    #[must_use]
    pub const fn provenance(&self) -> &SourceProvenance {
        &self.provenance
    }

    /// Returns the exact number of emitted Points.
    #[must_use]
    pub const fn exact_count(&self) -> u64 {
        self.exact_count
    }

    /// Returns the exact normalized, sorted Source spans that were emitted.
    #[must_use]
    pub fn spans(&self) -> &[SourceSpan] {
        &self.spans
    }

    /// Returns sorted, duplicate-free Attribute identities emitted per Point.
    #[must_use]
    pub fn attributes(&self) -> &[AttributeId] {
        &self.attributes
    }

    /// Returns the exact hard budget applied to this completed read.
    #[must_use]
    pub const fn budget(&self) -> ReadBudget {
        self.budget
    }
}

/// Pull-based bounded stream of canonical Point Batches.
pub struct PointBatches {
    source: SourceId,
    transform: PositionTransform,
    world_bounds: Option<WorldBounds>,
    metadata: Arc<SourceMetadata>,
    spans: Arc<[SourceSpan]>,
    expected_attributes: Arc<[AttributeId]>,
    budget: ReadBudget,
    provenance: SourceProvenance,
    control: OperationControl,
    adapter_read: Option<Box<dyn AdapterRead>>,
    span_index: usize,
    next_ordinal: Option<u64>,
    expected_count: u64,
    emitted_count: u64,
    summary: Option<SourceReadSummary>,
    terminal: bool,
}

impl PointBatches {
    pub(crate) fn start(
        source: SourceId,
        metadata: Arc<SourceMetadata>,
        provenance: SourceProvenance,
        reader: &dyn ReadAdapter,
        request: ReadRequest,
    ) -> Result<Self, SourceError> {
        let NormalizedRead {
            spans,
            expected_attributes,
            attributes,
            budget,
            exact_count,
            max_output_batch_points,
        } = normalize_request(metadata.as_ref(), request)?;
        let next_ordinal = spans.first().map(|span| span.first_ordinal());
        let expected_attributes = Arc::from(expected_attributes);
        let control = OperationControl::new();
        let adapter_request = AdapterReadRequest::new(
            Arc::clone(&spans),
            attributes,
            budget,
            max_output_batch_points,
        );
        let adapter_read = if spans.is_empty() {
            None
        } else {
            Some(reader.start_read(adapter_request, source, control.reporter())?)
        };

        Ok(Self {
            source,
            transform: metadata.position_transform(),
            world_bounds: metadata.world_bounds(),
            metadata,
            spans,
            expected_attributes,
            budget,
            provenance,
            control,
            adapter_read,
            span_index: 0,
            next_ordinal,
            expected_count: exact_count,
            emitted_count: 0,
            summary: None,
            terminal: false,
        })
    }

    /// Returns the next canonical Point Batch, or terminal `None`.
    ///
    /// A terminal error is returned once; later calls are fused to `None`.
    ///
    /// # Errors
    ///
    /// Returns an adapter, cancellation, contract, or resource error once.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<PointBatch>, SourceError> {
        if self.terminal {
            return Ok(None);
        }
        if self.control.token().is_cancelled() {
            return self.fail(SourceError::Cancelled);
        }
        let Some(adapter_read) = self.adapter_read.as_mut() else {
            return self.finish();
        };

        let next = adapter_read.next();
        if self.control.token().is_cancelled() {
            return self.fail(SourceError::Cancelled);
        }

        match next {
            Ok(Some(batch)) => self.accept_batch(batch),
            Ok(None) => self.finish(),
            Err(error) => self.fail(error),
        }
    }

    /// Returns a cloneable read-only progress and cancellation handle.
    #[must_use]
    pub fn handle(&self) -> OperationHandle {
        self.control.handle()
    }

    /// Returns exact terminal facts only after successful completion.
    #[must_use]
    pub const fn summary(&self) -> Option<&SourceReadSummary> {
        self.summary.as_ref()
    }

    fn accept_batch(&mut self, batch: PointBatch) -> Result<Option<PointBatch>, SourceError> {
        if let Err(error) = self.validate_batch(&batch) {
            return self.fail(error);
        }

        if let Err(error) = self.advance_coverage(&batch) {
            return self.fail(error);
        }
        if let Err(error) = self.control.check_cancelled().map_err(SourceError::from) {
            return self.fail(error);
        }
        let accepted = ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            self.emitted_count,
            Some(self.expected_count),
        )
        .map_err(SourceError::from);
        let accepted = match accepted {
            Ok(accepted) => accepted,
            Err(error) => return self.fail(error),
        };
        if let Err(error) = self
            .control
            .report_progress(accepted)
            .map_err(SourceError::from)
        {
            return self.fail(error);
        }
        Ok(Some(batch))
    }

    fn validate_batch(&self, batch: &PointBatch) -> Result<(), SourceError> {
        self.validate_identity_and_transform(batch)?;
        self.validate_budget(batch)?;
        self.validate_ordinal_coverage(batch)?;
        self.validate_position_bounds(batch)?;
        self.validate_attributes(batch)
    }

    fn validate_identity_and_transform(&self, batch: &PointBatch) -> Result<(), SourceError> {
        if batch.source() != self.source {
            return Err(SourceError::AdapterSourceMismatch {
                expected: self.source,
                actual: batch.source(),
            });
        }
        if batch.positions().transform() != self.transform {
            return Err(SourceError::AdapterTransformMismatch);
        }
        Ok(())
    }

    fn validate_position_bounds(&self, batch: &PointBatch) -> Result<(), SourceError> {
        let bounds = self.world_bounds;
        for (row, &ticks) in batch.positions().ticks().iter().enumerate() {
            if row % 4_096 == 0 {
                self.control.check_cancelled().map_err(SourceError::from)?;
            }
            let world = self.transform.world_f64(ticks);
            let ordinal = batch.first_ordinal()
                + u64::try_from(row).expect("a validated batch row index fits u64");
            for (axis, &coordinate) in world.iter().enumerate() {
                if !coordinate.is_finite() {
                    return Err(SourceError::AdapterPositionOutOfBounds {
                        ordinal,
                        axis,
                        reason: "decoded world coordinate is not finite".into(),
                    });
                }
                if let Some(bounds) = bounds {
                    let min = bounds.min()[axis];
                    let max = bounds.max()[axis];
                    if coordinate < min || coordinate > max {
                        return Err(SourceError::AdapterPositionOutOfBounds {
                            ordinal,
                            axis,
                            reason: format!(
                                "decoded coordinate {coordinate} is outside inclusive bounds [{min}, {max}]"
                            )
                            .into(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_budget(&self, batch: &PointBatch) -> Result<(), SourceError> {
        let points = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        if points > self.budget.max_batch_points() {
            return Err(SourceError::ResourceLimit {
                limit: ReadLimit::BatchPoints,
                required: points,
                allowed: self.budget.max_batch_points(),
            });
        }
        let payload = batch.estimated_payload_bytes();
        if payload > self.budget.max_batch_payload_bytes() {
            return Err(SourceError::ResourceLimit {
                limit: ReadLimit::BatchPayloadBytes,
                required: payload,
                allowed: self.budget.max_batch_payload_bytes(),
            });
        }
        Ok(())
    }

    fn validate_ordinal_coverage(&self, batch: &PointBatch) -> Result<(), SourceError> {
        let expected = self
            .next_ordinal
            .ok_or(SourceError::AdapterOrdinalMismatch {
                expected: self.expected_count,
                actual: batch.first_ordinal(),
            })?;
        if batch.first_ordinal() != expected {
            return Err(SourceError::AdapterOrdinalMismatch {
                expected,
                actual: batch.first_ordinal(),
            });
        }
        let batch_points = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        let batch_end = batch.first_ordinal().checked_add(batch_points).ok_or(
            SourceError::AdapterSpanOverflow {
                batch_end: u64::MAX,
                span_end: self.spans[self.span_index].end_ordinal(),
            },
        )?;
        let span_end = self.spans[self.span_index].end_ordinal();
        if batch_end > span_end {
            return Err(SourceError::AdapterSpanOverflow {
                batch_end,
                span_end,
            });
        }
        Ok(())
    }

    fn validate_attributes(&self, batch: &PointBatch) -> Result<(), SourceError> {
        if batch.attributes().columns().len() != self.expected_attributes.len()
            && let Some(column) = batch
                .attributes()
                .columns()
                .iter()
                .find(|column| !self.expected_attributes.contains(&column.definition().id()))
        {
            return Err(SourceError::AdapterUnexpectedAttribute {
                attribute: column.definition().id(),
            });
        }

        for &attribute in self.expected_attributes.iter() {
            self.control.check_cancelled().map_err(SourceError::from)?;
            let expected = self
                .metadata
                .attributes()
                .get(attribute)
                .expect("requested Attributes were resolved against this schema");
            let Some(actual) = batch.attributes().get(attribute) else {
                return Err(SourceError::AdapterAttributeMismatch {
                    attribute,
                    reason: "requested column is missing".into(),
                });
            };
            if actual.definition() != expected {
                return Err(SourceError::AdapterAttributeMismatch {
                    attribute,
                    reason: "column definition differs from verified schema".into(),
                });
            }
            if actual.values().len() != batch.len() {
                return Err(SourceError::AdapterAttributeMismatch {
                    attribute,
                    reason: "column row count differs from Point count".into(),
                });
            }
        }

        for column in batch.attributes().columns() {
            let attribute = column.definition().id();
            if !self.expected_attributes.contains(&attribute) {
                return Err(SourceError::AdapterUnexpectedAttribute { attribute });
            }
        }
        Ok(())
    }

    fn advance_coverage(&mut self, batch: &PointBatch) -> Result<(), SourceError> {
        let batch_points = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        let batch_end = batch.first_ordinal() + batch_points;
        self.emitted_count =
            self.emitted_count
                .checked_add(batch_points)
                .ok_or(SourceError::ResourceLimit {
                    limit: ReadLimit::EmittedPoints,
                    required: u64::MAX,
                    allowed: self.expected_count,
                })?;

        if batch_end == self.spans[self.span_index].end_ordinal() {
            self.span_index += 1;
            self.next_ordinal = self
                .spans
                .get(self.span_index)
                .map(|span| span.first_ordinal());
        } else {
            self.next_ordinal = Some(batch_end);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<Option<PointBatch>, SourceError> {
        if self.span_index != self.spans.len() || self.emitted_count != self.expected_count {
            return self.fail(SourceError::AdapterEndedEarly {
                emitted: self.emitted_count,
                expected: self.expected_count,
            });
        }
        let summary = SourceReadSummary {
            provenance: self.provenance.clone(),
            spans: Arc::clone(&self.spans),
            exact_count: self.emitted_count,
            attributes: Arc::clone(&self.expected_attributes),
            budget: self.budget,
        };
        if let Err(error) = publish_complete(&self.control, Some(self.expected_count)) {
            return self.fail(error);
        }
        self.terminal = true;
        self.adapter_read = None;
        self.summary = Some(summary);
        Ok(None)
    }

    fn fail<T>(&mut self, error: SourceError) -> Result<T, SourceError> {
        self.terminal = true;
        self.adapter_read = None;
        self.summary = None;
        Err(error)
    }
}

impl BatchStream for PointBatches {
    type Batch = PointBatch;
    type Summary = SourceReadSummary;
    type Error = SourceError;

    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error> {
        Self::next(self)
    }

    fn summary(&self) -> Option<&Self::Summary> {
        Self::summary(self)
    }

    fn handle(&self) -> OperationHandle {
        Self::handle(self)
    }
}

impl Drop for PointBatches {
    fn drop(&mut self) {
        if !self.terminal {
            self.control.cancel();
        }
    }
}
