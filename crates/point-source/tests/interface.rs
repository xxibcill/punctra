//! Caller-facing and adapter-conformance tests for verified Source reads.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use foundation_runtime::{OperationReporter, ProgressPhase, ProgressSnapshot, RuntimeError};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeSchema, AttributeValues, ContentHash, CoordinateReference, PointBatch,
    PositionTransform, QuantizedPositions, SourceId, SourceMetadata, WorldBounds,
};
use point_source::adapter::{
    AdapterContract, AdapterRead, AdapterReadRequest, AdapterVerified, CandidateAdapter,
    FullVerification, ReadAdapter,
};
use point_source::{
    AttributeSelection, MAX_ADAPTER_NAME_BYTES, MAX_ADAPTER_VERSION_BYTES, MAX_FAST_TOKEN_BYTES,
    MAX_INPUT_ATTRIBUTE_IDS, MAX_INPUT_SOURCE_SPANS, MAX_LOGICAL_ORDER_BYTES,
    MAX_SOURCE_DIAGNOSTIC_BYTES, OpenOptions, ReadBudget, ReadRequest, SourceCandidate,
    SourceDiagnostic, SourceError, SourcePreview, SourceSpan, VerificationPolicy,
};

const FAST_TOKEN: &[u8] = b"stable-fast-token";

#[derive(Clone, Copy)]
enum FastBehavior {
    Match,
    RequireFull,
    ReportMismatch,
    ReturnMismatchedEvidence,
}

#[derive(Default)]
struct VerificationCalls {
    full: AtomicUsize,
    fast: AtomicUsize,
    full_expectations: Mutex<Vec<Option<ContentHash>>>,
    last_fast_token: Mutex<Vec<u8>>,
}

struct FakeCandidate {
    preview: SourcePreview,
    adapter_version: &'static str,
    metadata: SourceMetadata,
    content_hash: ContentHash,
    reader: Arc<ScriptedReadAdapter>,
    calls: Arc<VerificationCalls>,
    fast_behavior: FastBehavior,
}

impl FakeCandidate {
    fn verified(&self) -> AdapterVerified {
        self.verified_with_hash(self.content_hash)
    }

    fn verified_with_hash(&self, content_hash: ContentHash) -> AdapterVerified {
        let reader: Arc<dyn ReadAdapter> = self.reader.clone();
        AdapterVerified::new(
            AdapterContract::new("fake", self.adapter_version, "input row order").unwrap(),
            Arc::new(self.metadata.clone()),
            content_hash,
            FAST_TOKEN.to_vec(),
            reader,
        )
    }
}

impl CandidateAdapter for FakeCandidate {
    fn preview(&self) -> &SourcePreview {
        &self.preview
    }

    fn full_verify(
        &self,
        verification: FullVerification,
        control: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        self.calls.full.fetch_add(1, Ordering::Relaxed);
        self.calls
            .full_expectations
            .lock()
            .unwrap()
            .push(verification.expected_content_hash());
        control.report_progress(ProgressSnapshot::new(ProgressPhase::RUNNING, 1, Some(2))?)?;
        Ok(self.verified())
    }

    fn fast_verify(
        &self,
        expected_fast_token: &[u8],
        control: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        self.calls.fast.fetch_add(1, Ordering::Relaxed);
        *self.calls.last_fast_token.lock().unwrap() = expected_fast_token.to_vec();
        control.report_progress(ProgressSnapshot::new(ProgressPhase::RUNNING, 1, Some(2))?)?;
        match self.fast_behavior {
            FastBehavior::Match => Ok(self.verified()),
            FastBehavior::RequireFull => Err(SourceError::VerificationRequired),
            FastBehavior::ReportMismatch => Err(SourceError::changed(
                "Fast token differs from recorded evidence",
            )),
            FastBehavior::ReturnMismatchedEvidence => {
                Ok(self.verified_with_hash(ContentHash::new([0xFE; 32])))
            }
        }
    }
}

#[derive(Default)]
struct ScriptedReadAdapter {
    batches: Mutex<Vec<PointBatch>>,
    requests: Mutex<Vec<AdapterReadRequest>>,
    next_gate: Mutex<Option<Arc<ReadGate>>>,
}

impl ScriptedReadAdapter {
    fn replace_batches(&self, batches: Vec<PointBatch>) {
        *self.batches.lock().unwrap() = batches;
    }

    fn last_request(&self) -> AdapterReadRequest {
        self.requests.lock().unwrap().last().unwrap().clone()
    }

    fn block_next(&self, gate: Arc<ReadGate>) {
        *self.next_gate.lock().unwrap() = Some(gate);
    }
}

impl ReadAdapter for ScriptedReadAdapter {
    fn start_read(
        &self,
        request: AdapterReadRequest,
        _source: SourceId,
        _control: OperationReporter,
    ) -> Result<Box<dyn AdapterRead>, SourceError> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::new(ScriptedRead {
            batches: self.batches.lock().unwrap().clone().into(),
            next_gate: self.next_gate.lock().unwrap().take(),
        }))
    }
}

struct ScriptedRead {
    batches: VecDeque<PointBatch>,
    next_gate: Option<Arc<ReadGate>>,
}

impl AdapterRead for ScriptedRead {
    fn next(&mut self) -> Result<Option<PointBatch>, SourceError> {
        if let Some(gate) = self.next_gate.take() {
            gate.entered.wait();
            gate.release.wait();
        }
        Ok(self.batches.pop_front())
    }
}

struct ReadGate {
    entered: Barrier,
    release: Barrier,
}

impl ReadGate {
    fn new() -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

struct Fixture {
    reader: Arc<ScriptedReadAdapter>,
    calls: Arc<VerificationCalls>,
    metadata: SourceMetadata,
}

impl Fixture {
    fn candidate(&self, hash_byte: u8, fast_behavior: FastBehavior) -> SourceCandidate {
        self.candidate_with_version(hash_byte, fast_behavior, "1")
    }

    fn candidate_with_version(
        &self,
        hash_byte: u8,
        fast_behavior: FastBehavior,
        adapter_version: &'static str,
    ) -> SourceCandidate {
        SourceCandidate::new_adapter(FakeCandidate {
            preview: SourcePreview::new("memory", Some("fixture".to_owned())),
            adapter_version,
            metadata: self.metadata.clone(),
            content_hash: ContentHash::new([hash_byte; 32]),
            reader: self.reader.clone(),
            calls: self.calls.clone(),
            fast_behavior,
        })
    }
}

fn fixture() -> Fixture {
    let transform = PositionTransform::new([1_000.0, 2_000.0, 10.0], [0.01, 0.01, 0.01]).unwrap();
    let intensity = AttributeDefinition::new(
        AttributeId::new(1).unwrap(),
        "intensity",
        AttributeDataType::U16,
    )
    .unwrap();
    let schema = AttributeSchema::new(vec![intensity]).unwrap();
    let world_bounds = WorldBounds::new(
        transform.world_f64([0, 0, 0]),
        transform.world_f64([7, 0, 0]),
    )
    .unwrap();
    let metadata = SourceMetadata::new(
        8,
        transform,
        CoordinateReference::Unknown,
        schema,
        Some(world_bounds),
        "memory",
        Vec::new(),
    )
    .unwrap();
    Fixture {
        reader: Arc::new(ScriptedReadAdapter::default()),
        calls: Arc::new(VerificationCalls::default()),
        metadata,
    }
}

fn batch(
    source: SourceId,
    metadata: &SourceMetadata,
    first_ordinal: u64,
    point_count: usize,
) -> PointBatch {
    let ticks = (0..point_count)
        .map(|offset| {
            let ordinal = first_ordinal + u64::try_from(offset).unwrap();
            [i64::try_from(ordinal).unwrap(), 0, 0]
        })
        .collect();
    batch_with_ticks(source, metadata, first_ordinal, ticks)
}

fn batch_with_ticks(
    source: SourceId,
    metadata: &SourceMetadata,
    first_ordinal: u64,
    ticks: Vec<[i64; 3]>,
) -> PointBatch {
    let point_count = ticks.len();
    let positions = QuantizedPositions::new(metadata.position_transform(), ticks).unwrap();
    let definition = metadata.attributes().definitions()[0].clone();
    let values = (0..point_count)
        .map(|offset| u16::try_from(offset).unwrap())
        .collect();
    let column = AttributeColumn::new(definition, AttributeValues::u16(values)).unwrap();
    let attributes = AttributeColumns::new(vec![column], point_count).unwrap();
    PointBatch::new(source, first_ordinal, positions, attributes).unwrap()
}

#[test]
fn identify_forces_full_and_recorded_fast_open_matches() {
    let first = fixture();
    let open = first
        .candidate(7, FastBehavior::Match)
        .open(OpenOptions::identify());
    let open_handle = open.handle();
    let source = open.blocking_wait().unwrap();
    assert_eq!(first.calls.full.load(Ordering::Relaxed), 1);
    assert_eq!(first.calls.fast.load(Ordering::Relaxed), 0);
    assert_eq!(*first.calls.full_expectations.lock().unwrap(), [None]);
    assert_eq!(open_handle.progress().phase(), ProgressPhase::COMPLETE);
    assert_eq!(open_handle.progress().completed_units(), 2);
    assert_eq!(open_handle.progress().total_units(), Some(2));

    let second = fixture();
    let reopened = second
        .candidate(7, FastBehavior::Match)
        .open(OpenOptions::match_record(
            source.record().clone(),
            VerificationPolicy::FastOnly,
        ))
        .blocking_wait()
        .unwrap();

    assert_eq!(reopened.identity(), source.identity());
    assert_eq!(second.calls.full.load(Ordering::Relaxed), 0);
    assert_eq!(second.calls.fast.load(Ordering::Relaxed), 1);
    assert_eq!(*second.calls.last_fast_token.lock().unwrap(), FAST_TOKEN);
}

#[test]
fn fast_then_full_falls_back_and_changed_content_is_explicit() {
    let original = fixture();
    let source = original
        .candidate(3, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();

    let fallback = fixture();
    let fallback_open =
        fallback
            .candidate(3, FastBehavior::RequireFull)
            .open(OpenOptions::match_record(
                source.record().clone(),
                VerificationPolicy::FastThenFull,
            ));
    let fallback_handle = fallback_open.handle();
    let reopened = fallback_open.blocking_wait().unwrap();
    assert_eq!(reopened.identity(), source.identity());
    assert_eq!(fallback.calls.fast.load(Ordering::Relaxed), 1);
    assert_eq!(fallback.calls.full.load(Ordering::Relaxed), 1);
    assert_eq!(
        *fallback.calls.full_expectations.lock().unwrap(),
        [Some(source.record().content_hash())]
    );
    assert_eq!(fallback_handle.progress().phase(), ProgressPhase::COMPLETE);
    assert_eq!(fallback_handle.progress().completed_units(), 2);

    for fast_behavior in [
        FastBehavior::ReportMismatch,
        FastBehavior::ReturnMismatchedEvidence,
    ] {
        let mismatch = fixture();
        let reopened = mismatch
            .candidate(3, fast_behavior)
            .open(OpenOptions::match_record(
                source.record().clone(),
                VerificationPolicy::FastThenFull,
            ))
            .blocking_wait()
            .unwrap();
        assert_eq!(reopened.identity(), source.identity());
        assert_eq!(mismatch.calls.fast.load(Ordering::Relaxed), 1);
        assert_eq!(mismatch.calls.full.load(Ordering::Relaxed), 1);
    }

    let fast_only = fixture()
        .candidate(3, FastBehavior::ReportMismatch)
        .open(OpenOptions::match_record(
            source.record().clone(),
            VerificationPolicy::FastOnly,
        ))
        .blocking_wait();
    assert!(matches!(fast_only, Err(SourceError::VerificationRequired)));

    let changed = fixture()
        .candidate(4, FastBehavior::Match)
        .open(OpenOptions::match_record(
            source.record().clone(),
            VerificationPolicy::Full,
        ))
        .blocking_wait();
    assert!(matches!(changed, Err(SourceError::SourceChanged { .. })));
}

#[test]
fn full_reopen_reports_record_mismatches_as_source_changed() {
    let original = fixture();
    let source = original
        .candidate(31, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();

    let changed_adapter = fixture()
        .candidate_with_version(31, FastBehavior::Match, "2")
        .open(OpenOptions::match_record(
            source.record().clone(),
            VerificationPolicy::Full,
        ))
        .blocking_wait();
    assert!(matches!(
        changed_adapter,
        Err(SourceError::SourceChanged { .. })
    ));

    let mut changed_metadata = fixture();
    changed_metadata.metadata = SourceMetadata::new(
        source.metadata().point_count(),
        source.metadata().position_transform(),
        source.metadata().coordinate_reference().clone(),
        source.metadata().attributes().clone(),
        source.metadata().world_bounds(),
        "different-format-name",
        source.metadata().metadata_records().to_vec(),
    )
    .unwrap();
    let changed_metadata = changed_metadata
        .candidate(31, FastBehavior::Match)
        .open(OpenOptions::match_record(
            source.record().clone(),
            VerificationPolicy::Full,
        ))
        .blocking_wait();
    assert!(matches!(
        changed_metadata,
        Err(SourceError::SourceChanged { .. })
    ));
}

#[test]
fn failed_open_never_publishes_complete_progress() {
    let original = fixture();
    let source = original
        .candidate(41, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    let changed = fixture()
        .candidate(42, FastBehavior::Match)
        .open(OpenOptions::match_record(
            source.record().clone(),
            VerificationPolicy::Full,
        ));
    let control = changed.handle();

    assert!(matches!(
        changed.blocking_wait(),
        Err(SourceError::SourceChanged { .. })
    ));
    assert_ne!(control.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn overlap_is_normalized_and_success_has_one_exact_summary() {
    let fixture = fixture();
    let source = fixture
        .candidate(9, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    fixture
        .reader
        .replace_batches(vec![batch(source.identity(), source.metadata(), 0, 5)]);

    let mut points = source
        .read(ReadRequest::all().spans([
            SourceSpan::new(2, 3).unwrap(),
            SourceSpan::new(0, 3).unwrap(),
        ]))
        .unwrap();
    let emitted = points.next().unwrap().unwrap();
    assert_eq!(
        emitted
            .point_ids()
            .map(point_contracts::PointId::ordinal)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(points.handle().progress().phase(), ProgressPhase::RUNNING);
    assert_eq!(points.handle().progress().completed_units(), 5);
    assert_eq!(points.handle().progress().total_units(), Some(5));
    assert!(points.next().unwrap().is_none());
    assert!(points.next().unwrap().is_none());
    let summary = points.summary().unwrap();
    assert_eq!(summary.source(), source.identity());
    assert_eq!(summary.exact_count(), 5);
    assert_eq!(summary.spans(), &[SourceSpan::new(0, 5).unwrap()]);
    assert_eq!(
        summary.attributes(),
        &[source.metadata().attributes().definitions()[0].id()]
    );
    assert_eq!(summary.budget(), ReadBudget::default());
    assert_eq!(points.handle().progress().phase(), ProgressPhase::COMPLETE);

    let request = fixture.reader.last_request();
    assert_eq!(request.spans(), &[SourceSpan::new(0, 5).unwrap()]);
    assert_eq!(
        request.attributes(),
        &AttributeSelection::only([source.metadata().attributes().definitions()[0].id()])
    );
}

#[test]
fn attribute_ids_are_resolved_sorted_and_deduplicated_for_adapter_and_summary() {
    let fixture = fixture();
    let source = fixture
        .candidate(51, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    let attribute = source.metadata().attributes().definitions()[0].id();
    fixture
        .reader
        .replace_batches(vec![batch(source.identity(), source.metadata(), 0, 1)]);

    let mut points = source
        .read(
            ReadRequest::all()
                .spans([SourceSpan::new(0, 1).unwrap()])
                .attributes(AttributeSelection::only([attribute, attribute])),
        )
        .unwrap();
    assert!(points.next().unwrap().is_some());
    assert!(points.next().unwrap().is_none());

    assert_eq!(points.summary().unwrap().attributes(), &[attribute]);
    assert_eq!(
        fixture.reader.last_request().attributes(),
        &AttributeSelection::only([attribute])
    );
}

#[test]
fn unavailable_requested_attribute_is_an_unsupported_schema() {
    let fixture = fixture();
    let source = fixture
        .candidate(63, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    let unavailable = AttributeId::new(999).unwrap();

    let Err(error) =
        source.read(ReadRequest::all().attributes(AttributeSelection::only([unavailable])))
    else {
        panic!("the unavailable Attribute must fail before reading");
    };
    assert!(matches!(error, SourceError::UnsupportedSchema { .. }));
}

#[test]
fn max_spans_applies_after_overlap_normalization() {
    let fixture = fixture();
    let source = fixture
        .candidate(52, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    let budget = ReadBudget::default().with_max_spans(1);
    fixture
        .reader
        .replace_batches(vec![batch(source.identity(), source.metadata(), 0, 3)]);

    let mut merged = source
        .read(
            ReadRequest::all()
                .spans([
                    SourceSpan::new(0, 2).unwrap(),
                    SourceSpan::new(1, 2).unwrap(),
                ])
                .budget(budget),
        )
        .unwrap();
    assert!(merged.next().unwrap().is_some());
    assert!(merged.next().unwrap().is_none());
    assert_eq!(
        merged.summary().unwrap().spans(),
        &[SourceSpan::new(0, 3).unwrap()]
    );

    let disjoint = source.read(
        ReadRequest::all()
            .spans([
                SourceSpan::new(0, 1).unwrap(),
                SourceSpan::new(2, 1).unwrap(),
            ])
            .budget(budget),
    );
    assert!(matches!(
        disjoint,
        Err(SourceError::ResourceLimit {
            limit: "normalized Source spans",
            required: 2,
            allowed: 1,
        })
    ));
}

struct OversizedSpanIterator;

impl Iterator for OversizedSpanIterator {
    type Item = SourceSpan;

    fn next(&mut self) -> Option<Self::Item> {
        panic!("the lower-bound guard should reject this iterator before polling it")
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = MAX_INPUT_SOURCE_SPANS.saturating_add(1);
        (length, Some(length))
    }
}

#[test]
fn raw_span_safety_cap_does_not_eagerly_poll_known_oversized_input() {
    let fixture = fixture();
    let source = fixture
        .candidate(53, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();

    let result = source.read(ReadRequest::all().spans(OversizedSpanIterator));
    assert!(matches!(
        result,
        Err(SourceError::ResourceLimit {
            limit: "input Source spans",
            ..
        })
    ));
}

struct OversizedAttributeIterator;

impl Iterator for OversizedAttributeIterator {
    type Item = AttributeId;

    fn next(&mut self) -> Option<Self::Item> {
        panic!("the lower-bound guard should reject this iterator before polling it")
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = MAX_INPUT_ATTRIBUTE_IDS.saturating_add(1);
        (length, Some(length))
    }
}

#[test]
fn attribute_selection_is_bounded_before_collection() {
    let fixture = fixture();
    let source = fixture
        .candidate(56, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();

    let result = source
        .read(ReadRequest::all().attributes(AttributeSelection::only(OversizedAttributeIterator)));
    assert!(matches!(
        result,
        Err(SourceError::ResourceLimit {
            limit: "input Attribute identities",
            ..
        })
    ));
}

#[test]
fn total_point_budget_is_enforced_before_adapter_start() {
    let fixture = fixture();
    let source = fixture
        .candidate(57, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();

    let result = source.read(
        ReadRequest::all()
            .spans([SourceSpan::new(0, 3).unwrap()])
            .budget(ReadBudget::default().with_max_points(2)),
    );
    assert!(matches!(
        result,
        Err(SourceError::ResourceLimit {
            limit: "requested Point count",
            required: 3,
            allowed: 2,
        })
    ));
}

#[test]
fn empty_selection_completes_with_exact_facts_and_complete_progress() {
    let fixture = fixture();
    let source = fixture
        .candidate(54, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    let mut points = source
        .read(ReadRequest::all().spans(std::iter::empty::<SourceSpan>()))
        .unwrap();
    let control = points.handle();

    assert!(points.next().unwrap().is_none());
    assert!(points.summary().unwrap().spans().is_empty());
    assert_eq!(points.summary().unwrap().exact_count(), 0);
    assert_eq!(control.progress().phase(), ProgressPhase::COMPLETE);
    assert_eq!(control.progress().total_units(), Some(0));
}

#[test]
fn adapter_positions_must_stay_inside_verified_world_bounds() {
    let fixture = fixture();
    let source = fixture
        .candidate(55, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    fixture.reader.replace_batches(vec![batch_with_ticks(
        source.identity(),
        source.metadata(),
        0,
        vec![[8, 0, 0]],
    )]);
    let mut points = source
        .read(ReadRequest::all().spans([SourceSpan::new(0, 1).unwrap()]))
        .unwrap();
    let control = points.handle();

    assert!(matches!(
        points.next(),
        Err(SourceError::AdapterPositionOutOfBounds {
            ordinal: 0,
            axis: 0,
            ..
        })
    ));
    assert!(points.summary().is_none());
    assert_ne!(control.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn point_budget_failure_is_fused_without_summary() {
    let fixture = fixture();
    let source = fixture
        .candidate(5, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    fixture
        .reader
        .replace_batches(vec![batch(source.identity(), source.metadata(), 0, 3)]);
    let budget = ReadBudget::new(2, 1_000_000).unwrap();
    let mut points = source
        .read(
            ReadRequest::all()
                .spans([SourceSpan::new(0, 3).unwrap()])
                .budget(budget),
        )
        .unwrap();
    let control = points.handle();

    assert!(matches!(
        points.next(),
        Err(SourceError::ResourceLimit {
            limit: "batch Points",
            ..
        })
    ));
    assert!(points.next().unwrap().is_none());
    assert!(points.summary().is_none());
    assert_ne!(control.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn invalid_adapter_output_and_cancellation_are_fused() {
    let fixture = fixture();
    let source = fixture
        .candidate(6, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    fixture.reader.replace_batches(vec![batch(
        SourceId::new([99; 32]),
        source.metadata(),
        0,
        2,
    )]);
    let request = ReadRequest::all().spans([SourceSpan::new(0, 2).unwrap()]);
    let mut invalid = source.read(request.clone()).unwrap();
    assert!(matches!(
        invalid.next(),
        Err(SourceError::AdapterSourceMismatch { .. })
    ));
    assert!(invalid.next().unwrap().is_none());
    assert!(invalid.summary().is_none());

    fixture
        .reader
        .replace_batches(vec![batch(source.identity(), source.metadata(), 0, 2)]);
    let mut cancelled = source.read(request).unwrap();
    cancelled.handle().cancel();
    assert!(matches!(cancelled.next(), Err(SourceError::Cancelled)));
    assert!(cancelled.next().unwrap().is_none());
    assert!(cancelled.summary().is_none());
}

#[test]
fn cancellation_during_adapter_next_prevents_batch_publication() {
    let fixture = fixture();
    let source = fixture
        .candidate(62, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    fixture
        .reader
        .replace_batches(vec![batch(source.identity(), source.metadata(), 0, 2)]);
    let gate = Arc::new(ReadGate::new());
    fixture.reader.block_next(Arc::clone(&gate));
    let points = source
        .read(ReadRequest::all().spans([SourceSpan::new(0, 2).unwrap()]))
        .unwrap();
    let handle = points.handle();

    let (first, fused, has_summary) = std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let mut points = points;
            let first = points.next();
            let fused = points.next();
            (first, fused, points.summary().is_some())
        });
        gate.entered.wait();
        handle.cancel();
        gate.release.wait();
        worker.join().unwrap()
    });

    assert!(matches!(first, Err(SourceError::Cancelled)));
    assert!(matches!(fused, Ok(None)));
    assert!(!has_summary);
    assert_eq!(handle.progress().completed_units(), 0);
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn early_adapter_end_is_an_error_then_fuses() {
    let fixture = fixture();
    let source = fixture
        .candidate(8, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    fixture.reader.replace_batches(Vec::new());
    let mut points = source
        .read(ReadRequest::all().spans([SourceSpan::new(0, 2).unwrap()]))
        .unwrap();

    assert!(matches!(
        points.next(),
        Err(SourceError::AdapterEndedEarly {
            emitted: 0,
            expected: 2
        })
    ));
    assert!(points.next().unwrap().is_none());
    assert!(points.summary().is_none());
}

#[test]
fn source_record_deserialization_enforces_all_adapter_owned_bounds() {
    let fixture = fixture();
    let source = fixture
        .candidate(61, FastBehavior::Match)
        .open(OpenOptions::identify())
        .blocking_wait()
        .unwrap();
    let value = serde_json::to_value(source.record()).unwrap();
    let round_trip: point_source::SourceRecord = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(&round_trip, source.record());

    for (field, max_bytes) in [
        ("adapter_name", MAX_ADAPTER_NAME_BYTES),
        ("adapter_version", MAX_ADAPTER_VERSION_BYTES),
        ("logical_order", MAX_LOGICAL_ORDER_BYTES),
    ] {
        let mut empty = value.clone();
        empty.as_object_mut().unwrap().insert(
            field.to_owned(),
            serde_json::Value::String("   ".to_owned()),
        );
        assert!(serde_json::from_value::<point_source::SourceRecord>(empty).is_err());

        let mut oversized = value.clone();
        oversized.as_object_mut().unwrap().insert(
            field.to_owned(),
            serde_json::Value::String("x".repeat(max_bytes + 1)),
        );
        assert!(serde_json::from_value::<point_source::SourceRecord>(oversized).is_err());
    }

    let mut oversized_token = value;
    oversized_token.as_object_mut().unwrap().insert(
        "fast_token".to_owned(),
        serde_json::Value::Array(
            std::iter::repeat_n(serde_json::Value::from(0), MAX_FAST_TOKEN_BYTES + 1).collect(),
        ),
    );
    assert!(serde_json::from_value::<point_source::SourceRecord>(oversized_token).is_err());
}

#[test]
fn adapter_contract_validates_identity_fields_once() {
    let contract = AdapterContract::new("adapter", "1", "input row order").unwrap();
    assert_eq!(contract.name(), "adapter");
    assert_eq!(contract.version(), "1");
    assert_eq!(contract.logical_order(), "input row order");

    for invalid in [
        AdapterContract::new(" ", "1", "input row order"),
        AdapterContract::new("adapter", " ", "input row order"),
        AdapterContract::new("adapter", "1", " "),
        AdapterContract::new(
            "x".repeat(MAX_ADAPTER_NAME_BYTES + 1),
            "1",
            "input row order",
        ),
        AdapterContract::new(
            "adapter",
            "x".repeat(MAX_ADAPTER_VERSION_BYTES + 1),
            "input row order",
        ),
        AdapterContract::new("adapter", "1", "x".repeat(MAX_LOGICAL_ORDER_BYTES + 1)),
    ] {
        assert!(matches!(
            invalid,
            Err(SourceError::SourceContractMismatch { .. })
        ));
    }
}

#[test]
fn runtime_cancellation_has_one_domain_error() {
    assert!(matches!(
        SourceError::from(RuntimeError::Cancelled),
        SourceError::Cancelled
    ));
    assert!(matches!(
        SourceError::from(RuntimeError::WorkerPanicked),
        SourceError::Runtime(RuntimeError::WorkerPanicked)
    ));
}

#[test]
fn adapter_diagnostics_are_utf8_safe_and_bounded() {
    let oversized = "é".repeat(MAX_SOURCE_DIAGNOSTIC_BYTES);
    let diagnostic = SourceDiagnostic::new(oversized);

    assert!(diagnostic.len() <= MAX_SOURCE_DIAGNOSTIC_BYTES);
    assert!(diagnostic.ends_with('…'));
    assert!(std::str::from_utf8(diagnostic.as_bytes()).is_ok());
}
