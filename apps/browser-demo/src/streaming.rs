use render_protocol::{
    BatchKey, BatchVersion, PointBatch, PointId, ProtocolError, RenderPoint, RenderUpdate,
    SourceId, ViewGenerationKey, ViewId,
};
use serde::{Serialize, Serializer};
use thiserror::Error;

pub(crate) const STREAM_VIEW_GENERATION: ViewGenerationKey =
    ViewGenerationKey::new(ViewId::new(16), 1);
pub(crate) const TRANSFER_RECORD_BYTES: usize = 24;
pub(crate) const MAX_TRANSFER_BATCH_POINTS: u64 = 1_024;
pub(crate) const MAX_TRANSFER_BATCH_BYTES: u64 =
    MAX_TRANSFER_BATCH_POINTS * TRANSFER_RECORD_BYTES as u64;
pub(crate) const MAX_TRANSFER_BATCHES: u64 = 8;
pub(crate) const MAX_STREAM_POINTS: u64 = MAX_TRANSFER_BATCH_POINTS * MAX_TRANSFER_BATCHES;
pub(crate) const MAX_QUEUED_RANGES: u64 = 2;
pub(crate) const MAX_QUEUED_RANGE_BYTES: u64 = 512 * 1_024;
pub(crate) const MAX_RANGE_BYTES: u64 = 256 * 1_024;
pub(crate) const MAX_CONCURRENT_RESPONSE_BYTES: u64 = 256 * 1_024;
pub(crate) const MAX_WORKER_STAGING_BYTES: u64 = 320 * 1_024;
pub(crate) const MAX_MEMORY_CACHE_BYTES: u64 = 512 * 1_024;
pub(crate) const MAX_PERSISTENT_CACHE_BYTES: u64 = 4 * 1_024 * 1_024;
pub(crate) const MAX_CANCELLATION_MILLISECONDS: u64 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamPhase {
    Idle,
    Receiving,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct StreamFacts {
    phase: StreamPhase,
    #[serde(serialize_with = "serialize_source_identity")]
    source_identity: Option<SourceId>,
    coverage: &'static str,
    expected_points: u64,
    published_points: u64,
    published_batches: u64,
    transferred_bytes: u64,
    main_thread_batch_points_high_water: u64,
    main_thread_batch_bytes_high_water: u64,
    world_origin: Option<[f64; 3]>,
}

#[allow(clippy::ref_option)] // serde's serialize_with contract passes a field by reference.
fn serialize_source_identity<S>(source: &Option<SourceId>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match source {
        Some(source) => serializer.serialize_some(&source.to_string()),
        None => serializer.serialize_none(),
    }
}

impl StreamFacts {
    const fn idle() -> Self {
        Self {
            phase: StreamPhase::Idle,
            source_identity: None,
            coverage: "none",
            expected_points: 0,
            published_points: 0,
            published_batches: 0,
            transferred_bytes: 0,
            main_thread_batch_points_high_water: 0,
            main_thread_batch_bytes_high_water: 0,
            world_origin: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StreamingLimitFacts {
    active_operations: u64,
    concurrent_requests: u64,
    queued_ranges: u64,
    queued_range_bytes: u64,
    range_bytes: u64,
    concurrent_response_bytes: u64,
    worker_staging_bytes: u64,
    transfer_batch_points: u64,
    transfer_batch_bytes: u64,
    transfer_batches: u64,
    stream_points: u64,
    memory_cache_bytes: u64,
    persistent_cache_bytes: u64,
    cancellation_milliseconds: u64,
}

impl StreamingLimitFacts {
    pub(crate) const fn fixed() -> Self {
        Self {
            active_operations: 1,
            concurrent_requests: 1,
            queued_ranges: MAX_QUEUED_RANGES,
            queued_range_bytes: MAX_QUEUED_RANGE_BYTES,
            range_bytes: MAX_RANGE_BYTES,
            concurrent_response_bytes: MAX_CONCURRENT_RESPONSE_BYTES,
            worker_staging_bytes: MAX_WORKER_STAGING_BYTES,
            transfer_batch_points: MAX_TRANSFER_BATCH_POINTS,
            transfer_batch_bytes: MAX_TRANSFER_BATCH_BYTES,
            transfer_batches: MAX_TRANSFER_BATCHES,
            stream_points: MAX_STREAM_POINTS,
            memory_cache_bytes: MAX_MEMORY_CACHE_BYTES,
            persistent_cache_bytes: MAX_PERSISTENT_CACHE_BYTES,
            cancellation_milliseconds: MAX_CANCELLATION_MILLISECONDS,
        }
    }
}

#[derive(Clone)]
pub(crate) struct StreamingScene {
    facts: StreamFacts,
    source: Option<SourceId>,
    last_ordinal: Option<u64>,
}

impl StreamingScene {
    pub(crate) const fn idle() -> Self {
        Self {
            facts: StreamFacts::idle(),
            source: None,
            last_ordinal: None,
        }
    }

    pub(crate) fn begin(
        &mut self,
        source_identity: &str,
        expected_points: u64,
        world_origin: [f64; 3],
    ) -> Result<RenderUpdate, StreamError> {
        let source = parse_source_identity(source_identity)?;
        validate_begin(expected_points, world_origin)?;
        self.source = Some(source);
        self.last_ordinal = None;
        self.facts = StreamFacts {
            phase: StreamPhase::Receiving,
            source_identity: Some(source),
            coverage: "sampled",
            expected_points,
            published_points: 0,
            published_batches: 0,
            transferred_bytes: 0,
            main_thread_batch_points_high_water: 0,
            main_thread_batch_bytes_high_water: 0,
            world_origin: Some(world_origin),
        };
        Ok(RenderUpdate::Reset {
            view_generation: STREAM_VIEW_GENERATION,
        })
    }

    pub(crate) fn publish(
        &mut self,
        batch_index: u32,
        payload: &[u8],
    ) -> Result<RenderUpdate, StreamError> {
        self.require_receiving()?;
        self.validate_batch_index(batch_index)?;
        self.validate_payload_capacity(payload)?;
        let points = decode_points(payload, self.source(), self.last_ordinal)?;
        self.last_ordinal = points.last().map(|point| point.point_id().ordinal());
        self.record_batch(points.len(), payload.len())?;
        let batch = PointBatch::new(
            STREAM_VIEW_GENERATION,
            BatchKey::new(u64::from(batch_index) + 1),
            BatchVersion::new(1),
            self.facts
                .world_origin
                .expect("receiving streams have an origin"),
            points,
        )?;
        Ok(RenderUpdate::Upsert { batch })
    }

    pub(crate) fn complete(&mut self) -> Result<(), StreamError> {
        self.require_receiving()?;
        if self.facts.published_points != self.facts.expected_points {
            return Err(StreamError::Incomplete {
                expected: self.facts.expected_points,
                actual: self.facts.published_points,
            });
        }
        self.facts.phase = StreamPhase::Complete;
        Ok(())
    }

    pub(crate) const fn view_generation(&self) -> Option<ViewGenerationKey> {
        match self.facts.phase {
            StreamPhase::Idle => None,
            StreamPhase::Receiving | StreamPhase::Complete => Some(STREAM_VIEW_GENERATION),
        }
    }

    pub(crate) const fn facts(&self) -> StreamFacts {
        self.facts
    }

    fn source(&self) -> SourceId {
        self.source.expect("receiving streams have a Source")
    }

    fn require_receiving(&self) -> Result<(), StreamError> {
        if self.facts.phase == StreamPhase::Receiving {
            Ok(())
        } else {
            Err(StreamError::NotReceiving)
        }
    }

    fn validate_batch_index(&self, batch_index: u32) -> Result<(), StreamError> {
        if u64::from(batch_index) == self.facts.published_batches {
            Ok(())
        } else {
            Err(StreamError::BatchSequence {
                expected: self.facts.published_batches,
                actual: u64::from(batch_index),
            })
        }
    }

    fn validate_capacity(&self, points: usize, bytes: usize) -> Result<(), StreamError> {
        let points = u64::try_from(points).map_err(|_| StreamError::SizeOverflow)?;
        let bytes = u64::try_from(bytes).map_err(|_| StreamError::SizeOverflow)?;
        require_limit(points, MAX_TRANSFER_BATCH_POINTS, StreamLimit::BatchPoints)?;
        require_limit(bytes, MAX_TRANSFER_BATCH_BYTES, StreamLimit::BatchBytes)?;
        require_limit(
            self.facts.published_batches + 1,
            MAX_TRANSFER_BATCHES,
            StreamLimit::Batches,
        )?;
        require_limit(
            self.facts.published_points.saturating_add(points),
            self.facts.expected_points,
            StreamLimit::ExpectedPoints,
        )
    }

    fn validate_payload_capacity(&self, payload: &[u8]) -> Result<(), StreamError> {
        if payload.is_empty() || !payload.len().is_multiple_of(TRANSFER_RECORD_BYTES) {
            return Err(StreamError::InvalidPayloadLength);
        }
        self.validate_capacity(payload.len() / TRANSFER_RECORD_BYTES, payload.len())
    }

    fn record_batch(&mut self, points: usize, bytes: usize) -> Result<(), StreamError> {
        let points = u64::try_from(points).map_err(|_| StreamError::SizeOverflow)?;
        let bytes = u64::try_from(bytes).map_err(|_| StreamError::SizeOverflow)?;
        self.facts.published_points = self
            .facts
            .published_points
            .checked_add(points)
            .ok_or(StreamError::SizeOverflow)?;
        self.facts.published_batches += 1;
        self.facts.transferred_bytes = self
            .facts
            .transferred_bytes
            .checked_add(bytes)
            .ok_or(StreamError::SizeOverflow)?;
        self.facts.main_thread_batch_points_high_water =
            self.facts.main_thread_batch_points_high_water.max(points);
        self.facts.main_thread_batch_bytes_high_water =
            self.facts.main_thread_batch_bytes_high_water.max(bytes);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamLimit {
    BatchPoints,
    BatchBytes,
    Batches,
    ExpectedPoints,
    StreamPoints,
}

#[derive(Debug, Error)]
pub(crate) enum StreamError {
    #[error("Source identity must be exactly 64 lowercase hexadecimal characters")]
    InvalidSourceIdentity,
    #[error("stream world origin must contain three finite values")]
    InvalidWorldOrigin,
    #[error("stream expected Point count must be positive")]
    EmptyStream,
    #[error("stream is not receiving batches")]
    NotReceiving,
    #[error("stream batch {actual} does not match the next batch {expected}")]
    BatchSequence { expected: u64, actual: u64 },
    #[error("stream batch payload must be non-empty and a multiple of 24 bytes")]
    InvalidPayloadLength,
    #[error("stream batch ordinals must be strictly increasing across every transferred batch")]
    OrdinalOrder,
    #[error("stream batch color alpha must be 255")]
    InvalidAlpha,
    #[error("{limit:?} value {actual} exceeds the accepted limit {allowed}")]
    ResourceLimit {
        limit: StreamLimit,
        actual: u64,
        allowed: u64,
    },
    #[error("stream completed with {actual} Points instead of {expected}")]
    Incomplete { expected: u64, actual: u64 },
    #[error("stream accounting overflowed")]
    SizeOverflow,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

fn validate_begin(expected_points: u64, world_origin: [f64; 3]) -> Result<(), StreamError> {
    if expected_points == 0 {
        return Err(StreamError::EmptyStream);
    }
    require_limit(
        expected_points,
        MAX_STREAM_POINTS,
        StreamLimit::StreamPoints,
    )?;
    if world_origin.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(StreamError::InvalidWorldOrigin)
    }
}

fn decode_points(
    payload: &[u8],
    source: SourceId,
    previous_ordinal: Option<u64>,
) -> Result<Vec<RenderPoint>, StreamError> {
    let mut points = Vec::with_capacity(payload.len() / TRANSFER_RECORD_BYTES);
    let mut last = previous_ordinal;
    for record in payload.chunks_exact(TRANSFER_RECORD_BYTES) {
        let point = decode_point(record, source)?;
        if last.is_some_and(|ordinal| point.point_id().ordinal() <= ordinal) {
            return Err(StreamError::OrdinalOrder);
        }
        last = Some(point.point_id().ordinal());
        points.push(point);
    }
    Ok(points)
}

fn decode_point(record: &[u8], source: SourceId) -> Result<RenderPoint, StreamError> {
    let ordinal = u64::from_le_bytes(record[0..8].try_into().expect("record width is fixed"));
    let position = [
        f32::from_le_bytes(record[8..12].try_into().expect("record width is fixed")),
        f32::from_le_bytes(record[12..16].try_into().expect("record width is fixed")),
        f32::from_le_bytes(record[16..20].try_into().expect("record width is fixed")),
    ];
    let color: [u8; 4] = record[20..24].try_into().expect("record width is fixed");
    if color[3] != u8::MAX {
        return Err(StreamError::InvalidAlpha);
    }
    RenderPoint::new(position, color, PointId::new(source, ordinal)).map_err(StreamError::from)
}

fn require_limit(actual: u64, allowed: u64, limit: StreamLimit) -> Result<(), StreamError> {
    if actual <= allowed {
        Ok(())
    } else {
        Err(StreamError::ResourceLimit {
            limit,
            actual,
            allowed,
        })
    }
}

fn parse_source_identity(value: &str) -> Result<SourceId, StreamError> {
    if value.len() != 64 || !value.bytes().all(is_lower_hex) {
        return Err(StreamError::InvalidSourceIdentity);
    }
    let mut bytes = [0_u8; 32];
    for (target, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = hex_nibble(pair[0]).ok_or(StreamError::InvalidSourceIdentity)?;
        let low = hex_nibble(pair[1]).ok_or(StreamError::InvalidSourceIdentity)?;
        *target = (high << 4) | low;
    }
    Ok(SourceId::new(bytes))
}

const fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use render_protocol::RenderStateModel;

    use super::*;
    use crate::scene::render_limits;

    const SOURCE: &str = "1616161616161616161616161616161616161616161616161616161616161616";

    #[test]
    fn stream_preserves_identity_order_coverage_and_renderer_limits() {
        let mut stream = StreamingScene::idle();
        let reset = stream
            .begin(SOURCE, 3, [500_000.0, 4_600_000.0, 100.0])
            .unwrap();
        let first = stream
            .publish(0, &payload(&[(2, [0.0, 0.0, 0.0]), (5, [1.0, 0.0, 0.0])]))
            .unwrap();
        let second = stream
            .publish(1, &payload(&[(9, [0.0, 1.0, 0.0])]))
            .unwrap();
        stream.complete().unwrap();

        let mut renderer = RenderStateModel::new(render_limits());
        renderer.apply(&reset).unwrap();
        renderer.apply(&first).unwrap();
        renderer.apply(&second).unwrap();
        let snapshot = renderer.snapshot();
        assert_eq!(
            snapshot.active_view_generation(),
            Some(STREAM_VIEW_GENERATION)
        );
        assert_eq!(stream.view_generation(), Some(STREAM_VIEW_GENERATION));
        assert_eq!(snapshot.resident().point_count(), 3);
        assert_eq!(stream.facts().phase, StreamPhase::Complete);
        assert_eq!(stream.facts().coverage, "sampled");
        assert_eq!(stream.facts().published_batches, 2);
        assert_eq!(stream.facts().main_thread_batch_points_high_water, 2);
        assert_eq!(stream.facts().main_thread_batch_bytes_high_water, 48);
    }

    #[test]
    fn stream_rejects_invalid_identity_sequence_payload_and_completion() {
        let mut stream = StreamingScene::idle();
        assert!(matches!(
            stream.begin("AA", 1, [0.0; 3]),
            Err(StreamError::InvalidSourceIdentity)
        ));
        stream.begin(SOURCE, 2, [0.0; 3]).unwrap();
        assert!(matches!(
            stream.publish(1, &payload(&[(0, [0.0; 3])])),
            Err(StreamError::BatchSequence { .. })
        ));
        assert!(matches!(
            stream.publish(0, &[0; 23]),
            Err(StreamError::InvalidPayloadLength)
        ));
        stream.publish(0, &payload(&[(4, [0.0; 3])])).unwrap();
        assert!(matches!(
            stream.publish(1, &payload(&[(4, [1.0; 3])])),
            Err(StreamError::OrdinalOrder)
        ));
        assert!(matches!(
            stream.complete(),
            Err(StreamError::Incomplete { .. })
        ));
    }

    #[test]
    fn stream_rejects_an_oversized_batch_before_decoding_records() {
        let mut stream = StreamingScene::idle();
        stream.begin(SOURCE, MAX_STREAM_POINTS, [0.0; 3]).unwrap();
        let payload = vec![0; (max_transfer_batch_points() + 1) * TRANSFER_RECORD_BYTES];

        assert!(matches!(
            stream.publish(0, &payload),
            Err(StreamError::ResourceLimit {
                limit: StreamLimit::BatchPoints,
                actual: 1_025,
                allowed: MAX_TRANSFER_BATCH_POINTS,
            })
        ));
        assert_eq!(stream.facts().published_points, 0);
    }

    #[test]
    fn stream_point_limit_accepts_exact_and_rejects_one_over() {
        let mut stream = StreamingScene::idle();
        stream.begin(SOURCE, MAX_STREAM_POINTS, [0.0; 3]).unwrap();

        assert!(matches!(
            stream.begin(SOURCE, MAX_STREAM_POINTS + 1, [0.0; 3]),
            Err(StreamError::ResourceLimit {
                limit: StreamLimit::StreamPoints,
                actual,
                allowed: MAX_STREAM_POINTS,
            }) if actual == MAX_STREAM_POINTS + 1
        ));
    }

    #[test]
    fn transfer_batch_limits_accept_exact_and_reject_one_over() {
        let mut stream = StreamingScene::idle();
        stream.begin(SOURCE, MAX_STREAM_POINTS, [0.0; 3]).unwrap();

        stream
            .validate_capacity(max_transfer_batch_points(), max_transfer_batch_bytes())
            .unwrap();
        assert!(matches!(
            stream.validate_capacity(
                max_transfer_batch_points() + 1,
                max_transfer_batch_bytes(),
            ),
            Err(StreamError::ResourceLimit {
                limit: StreamLimit::BatchPoints,
                actual,
                allowed: MAX_TRANSFER_BATCH_POINTS,
            }) if actual == MAX_TRANSFER_BATCH_POINTS + 1
        ));
        assert!(matches!(
            stream.validate_capacity(
                max_transfer_batch_points(),
                max_transfer_batch_bytes() + 1,
            ),
            Err(StreamError::ResourceLimit {
                limit: StreamLimit::BatchBytes,
                actual,
                allowed: MAX_TRANSFER_BATCH_BYTES,
            }) if actual == MAX_TRANSFER_BATCH_BYTES + 1
        ));
    }

    #[test]
    fn transfer_batch_count_accepts_exact_and_rejects_one_over() {
        let mut stream = StreamingScene::idle();
        stream.begin(SOURCE, MAX_STREAM_POINTS, [0.0; 3]).unwrap();
        for batch_index in 0..max_transfer_batches() {
            stream
                .publish(batch_index, &payload(&[(u64::from(batch_index), [0.0; 3])]))
                .unwrap();
        }

        assert_eq!(stream.facts().published_batches, MAX_TRANSFER_BATCHES);
        assert!(matches!(
            stream.publish(
                max_transfer_batches(),
                &payload(&[(MAX_TRANSFER_BATCHES, [0.0; 3])]),
            ),
            Err(StreamError::ResourceLimit {
                limit: StreamLimit::Batches,
                actual,
                allowed: MAX_TRANSFER_BATCHES,
            }) if actual == MAX_TRANSFER_BATCHES + 1
        ));
    }

    #[test]
    fn expected_point_limit_accepts_exact_and_rejects_one_over() {
        let mut stream = StreamingScene::idle();
        stream.begin(SOURCE, 1, [0.0; 3]).unwrap();
        stream.publish(0, &payload(&[(0, [0.0; 3])])).unwrap();

        assert_eq!(stream.facts().published_points, 1);
        assert!(matches!(
            stream.publish(1, &payload(&[(1, [0.0; 3])])),
            Err(StreamError::ResourceLimit {
                limit: StreamLimit::ExpectedPoints,
                actual: 2,
                allowed: 1,
            })
        ));
    }

    #[test]
    fn fixed_limits_are_independent_and_serializable() {
        let value = serde_json::to_value(StreamingLimitFacts::fixed()).unwrap();
        assert_eq!(value["concurrent_requests"], 1);
        assert_eq!(value["queued_ranges"], 2);
        assert_eq!(value["range_bytes"], 262_144);
        assert_eq!(value["worker_staging_bytes"], 327_680);
        assert_eq!(value["transfer_batch_points"], 1_024);
        assert_eq!(value["stream_points"], 8_192);
        assert_eq!(value["persistent_cache_bytes"], 4_194_304);
        assert_eq!(value["cancellation_milliseconds"], 1_000);
    }

    fn max_transfer_batch_points() -> usize {
        usize::try_from(MAX_TRANSFER_BATCH_POINTS).expect("batch Point limit fits usize")
    }

    fn max_transfer_batch_bytes() -> usize {
        usize::try_from(MAX_TRANSFER_BATCH_BYTES).expect("batch byte limit fits usize")
    }

    fn max_transfer_batches() -> u32 {
        u32::try_from(MAX_TRANSFER_BATCHES).expect("batch-count limit fits u32")
    }

    fn payload(points: &[(u64, [f32; 3])]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(points.len() * TRANSFER_RECORD_BYTES);
        for (ordinal, position) in points {
            bytes.extend_from_slice(&ordinal.to_le_bytes());
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&[80, 120, 160, 255]);
        }
        bytes
    }
}
