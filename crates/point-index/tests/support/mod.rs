//! Shared deterministic fixtures for independent integration-test crates.

#![allow(
    dead_code,
    reason = "each integration-test crate uses a different fixture subset"
)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};

use foundation_runtime::OperationReporter;
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, ContentHash, CoordinateReference, PointBatch, PositionTransform,
    QuantizedPositions, SourceId, SourceMetadata, WorldBounds,
};
use point_index::{IndexNodeId, IndexPointBatch, IndexReadSummary, NodeReadBudget, PreparedIndex};
use point_source::adapter::{
    AdapterContract, AdapterRead, AdapterReadRequest, AdapterVerified, CandidateAdapter,
    FullVerification, ReadAdapter,
};
use point_source::{
    OpenOptions, ReadLimit, Source, SourceCandidate, SourceError, SourcePreview, SourceSpan,
};
use source_memory::{MemoryFaultControl, MemorySource};

pub const BLOCK_POINTS: usize = 65_536;
const fn attribute_id(value: u32) -> AttributeId {
    match AttributeId::new(value) {
        Ok(id) => id,
        Err(_) => panic!("fixture Attribute identity is nonzero"),
    }
}

pub const INTENSITY_ID: AttributeId = attribute_id(1);
pub const CLASSIFICATION_ID: AttributeId = attribute_id(6);
pub const RGB_IDS: [AttributeId; 3] = [attribute_id(16), attribute_id(17), attribute_id(18)];

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub struct TemporaryTarget {
    directory: PathBuf,
    target: PathBuf,
}

impl TemporaryTarget {
    pub fn new(label: &str) -> Self {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "punctra-point-index-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create isolated point-index test directory");
        let target = directory.join("fixture.pidx");
        Self { directory, target }
    }

    pub fn path(&self) -> &Path {
        &self.target
    }

    pub fn work_path(&self) -> PathBuf {
        self.directory.join("fixture.pidx.work")
    }

    pub fn copied_target(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

impl Drop for TemporaryTarget {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

pub fn transform() -> PositionTransform {
    PositionTransform::new([1_000_000.25, -2_000_000.5, 17.0], [0.25, 0.5, 2.0])
        .expect("fixture transform is valid")
}

pub fn clustered_ticks(point_count: usize) -> Vec<[i64; 3]> {
    (0..point_count).map(ticks_for_ordinal).collect()
}

pub fn ticks_for_ordinal(ordinal: usize) -> [i64; 3] {
    let block = ordinal / BLOCK_POINTS;
    let row = ordinal % BLOCK_POINTS;
    let center = match block % 4 {
        0 => [0, 0, 0],
        1 => [100_000, 0, 10_000],
        2 => [0, 100_000, 20_000],
        _ => [200_000, 200_000, 30_000],
    };
    let x = i64::try_from(row % 257).expect("fixture row fits i64") - 128;
    let y = i64::try_from((row / 257) % 251).expect("fixture row fits i64") - 125;
    let z = i64::try_from(row % 29).expect("fixture row fits i64") - 14;
    [center[0] + x, center[1] + y, center[2] + z]
}

pub fn open_source(ticks: Vec<[i64; 3]>) -> Source {
    open_source_with_transform(ticks, transform())
}

pub fn open_source_with_columns(ticks: Vec<[i64; 3]>, columns: AttributeColumns) -> Source {
    let input =
        MemorySource::from_columns(transform(), CoordinateReference::Unknown, ticks, columns)
            .expect("fixture Attribute columns are valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("fixture attributed Source opens")
}

pub fn open_attributed_source(ticks: Vec<[i64; 3]>, with_rgb: bool) -> Source {
    let point_count = ticks.len();
    let columns = attributed_columns(point_count, with_rgb);
    let input =
        MemorySource::from_columns(transform(), CoordinateReference::Unknown, ticks, columns)
            .expect("attributed fixture memory Source is valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("attributed fixture Source opens")
}

pub fn open_controlled_attributed_source(
    ticks: Vec<[i64; 3]>,
    with_rgb: bool,
) -> (Source, MemoryFaultControl) {
    let columns = attributed_columns(ticks.len(), with_rgb);
    let initial = MemorySource::from_columns(
        transform(),
        CoordinateReference::Unknown,
        ticks.clone(),
        columns.clone(),
    )
    .expect("attributed fixture memory Source is valid");
    let initial = source_memory::open(initial)
        .blocking_wait()
        .expect("attributed fixture Source opens");
    let (controlled, faults) =
        MemorySource::with_fault_control(initial.metadata().clone(), ticks, columns)
            .expect("controlled attributed fixture matches inferred metadata");
    let source = source_memory::open(controlled)
        .blocking_wait()
        .expect("controlled attributed fixture opens");
    assert_eq!(source.identity(), initial.identity());
    (source, faults)
}

fn attributed_columns(point_count: usize, with_rgb: bool) -> AttributeColumns {
    let mut columns = vec![
        AttributeColumn::new(
            AttributeDefinition::new(INTENSITY_ID, "intensity", AttributeDataType::U16).unwrap(),
            AttributeValues::u16(
                (0..point_count)
                    .map(|ordinal| attributed_values(ordinal).0)
                    .collect(),
            ),
        )
        .unwrap(),
        AttributeColumn::new(
            AttributeDefinition::new(CLASSIFICATION_ID, "classification", AttributeDataType::U8)
                .unwrap(),
            AttributeValues::u8(
                (0..point_count)
                    .map(|ordinal| attributed_values(ordinal).1)
                    .collect(),
            ),
        )
        .unwrap(),
    ];
    if with_rgb {
        for (channel, (id, name)) in RGB_IDS
            .into_iter()
            .zip(["red", "green", "blue"])
            .enumerate()
        {
            columns.push(
                AttributeColumn::new(
                    AttributeDefinition::new(id, name, AttributeDataType::U16).unwrap(),
                    AttributeValues::u16(
                        (0..point_count)
                            .map(|ordinal| attributed_values(ordinal).2[channel])
                            .collect(),
                    ),
                )
                .unwrap(),
            );
        }
    }
    AttributeColumns::new(columns, point_count).unwrap()
}

pub fn attributed_values(ordinal: usize) -> (u16, u8, [u16; 3]) {
    let value = u64::try_from(ordinal).unwrap();
    let low_u16 = |value: u64| u16::try_from(value & u64::from(u16::MAX)).unwrap();
    let low_u8 = |value: u64| u8::try_from(value & u64::from(u8::MAX)).unwrap();
    (
        low_u16(value.wrapping_mul(257)),
        low_u8(value.wrapping_mul(17)),
        [
            low_u16(value.wrapping_mul(3)),
            low_u16(value.wrapping_mul(5).wrapping_add(7)),
            low_u16(value.wrapping_mul(11).wrapping_add(13)),
        ],
    )
}

pub fn open_source_with_transform(
    ticks: Vec<[i64; 3]>,
    position_transform: PositionTransform,
) -> Source {
    let point_count = ticks.len();
    let input = MemorySource::from_columns(
        position_transform,
        CoordinateReference::Unknown,
        ticks,
        AttributeColumns::empty(point_count),
    )
    .expect("fixture memory Source is valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("fixture memory Source opens")
}

pub fn open_controlled_source(ticks: Vec<[i64; 3]>) -> (Source, MemoryFaultControl) {
    let point_count = ticks.len();
    let initial = MemorySource::from_columns(
        transform(),
        CoordinateReference::Unknown,
        ticks.clone(),
        AttributeColumns::empty(point_count),
    )
    .expect("fixture memory Source is valid");
    let initial = source_memory::open(initial)
        .blocking_wait()
        .expect("fixture memory Source opens");
    let (controlled, faults) = MemorySource::with_fault_control(
        initial.metadata().clone(),
        ticks,
        AttributeColumns::empty(point_count),
    )
    .expect("controlled fixture matches inferred metadata");
    let source = source_memory::open(controlled)
        .blocking_wait()
        .expect("controlled fixture opens");
    assert_eq!(source.identity(), initial.identity());
    (source, faults)
}

pub fn open_budgeted_source(ticks: Vec<[i64; 3]>, required_adapter_bytes: u64) -> Source {
    let metadata = open_source(ticks.clone()).metadata().clone();
    SourceCandidate::new_adapter(BudgetedCandidate {
        preview: SourcePreview::new("budgeted-test", None),
        metadata: Arc::new(metadata),
        ticks: Arc::from(ticks),
        required_adapter_bytes,
    })
    .open(OpenOptions::identify())
    .blocking_wait()
    .expect("budgeted fixture Source opens")
}

struct BudgetedCandidate {
    preview: SourcePreview,
    metadata: Arc<SourceMetadata>,
    ticks: Arc<[[i64; 3]]>,
    required_adapter_bytes: u64,
}

impl BudgetedCandidate {
    fn verified(&self) -> AdapterVerified {
        let reader: Arc<dyn ReadAdapter> = Arc::new(BudgetedReadAdapter {
            metadata: Arc::clone(&self.metadata),
            ticks: Arc::clone(&self.ticks),
            required_adapter_bytes: self.required_adapter_bytes,
        });
        AdapterVerified::new(
            AdapterContract::new("budgeted-test", "1", "canonical fixture row order").unwrap(),
            Arc::clone(&self.metadata),
            ContentHash::new([0xB4; 32]),
            vec![1],
            reader,
        )
    }
}

impl CandidateAdapter for BudgetedCandidate {
    fn preview(&self) -> &SourcePreview {
        &self.preview
    }

    fn full_verify(
        &self,
        _verification: FullVerification,
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        reporter.check_cancelled()?;
        Ok(self.verified())
    }

    fn fast_verify(
        &self,
        _expected_fast_token: &[u8],
        _reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        Err(SourceError::VerificationRequired)
    }
}

struct BudgetedReadAdapter {
    metadata: Arc<SourceMetadata>,
    ticks: Arc<[[i64; 3]]>,
    required_adapter_bytes: u64,
}

impl ReadAdapter for BudgetedReadAdapter {
    fn start_read(
        &self,
        request: AdapterReadRequest,
        source: SourceId,
        reporter: OperationReporter,
    ) -> Result<Box<dyn AdapterRead>, SourceError> {
        let allowed = request.budget().max_adapter_working_bytes();
        if self.required_adapter_bytes > allowed {
            return Err(SourceError::ResourceLimit {
                limit: ReadLimit::AdapterWorkingBytes,
                required: self.required_adapter_bytes,
                allowed,
            });
        }
        Ok(Box::new(BudgetedRead {
            metadata: Arc::clone(&self.metadata),
            ticks: Arc::clone(&self.ticks),
            spans: request.spans().to_vec(),
            budget: request.budget(),
            source,
            reporter,
            span_index: 0,
            next_ordinal: request
                .spans()
                .first()
                .map_or(0, |span| span.first_ordinal()),
        }))
    }
}

struct BudgetedRead {
    metadata: Arc<SourceMetadata>,
    ticks: Arc<[[i64; 3]]>,
    spans: Vec<SourceSpan>,
    budget: point_source::ReadBudget,
    source: SourceId,
    reporter: OperationReporter,
    span_index: usize,
    next_ordinal: u64,
}

impl AdapterRead for BudgetedRead {
    fn next(&mut self) -> Result<Option<PointBatch>, SourceError> {
        self.reporter.check_cancelled()?;
        let Some(span) = self.spans.get(self.span_index).copied() else {
            return Ok(None);
        };
        if self.next_ordinal == span.end_ordinal() {
            self.span_index += 1;
            self.next_ordinal = self
                .spans
                .get(self.span_index)
                .map_or(0, |next| next.first_ordinal());
            return self.next();
        }
        let count = (span.end_ordinal() - self.next_ordinal)
            .min(self.budget.max_batch_points())
            .min(self.budget.max_batch_payload_bytes() / 24);
        let first = self.next_ordinal;
        let end = first + count;
        let start = usize::try_from(first).expect("fixture ordinal is addressable");
        let end_index = usize::try_from(end).expect("fixture ordinal is addressable");
        let positions = QuantizedPositions::new(
            self.metadata.position_transform(),
            self.ticks[start..end_index].to_vec(),
        )
        .map_err(|error| SourceError::adapter(format!("budget fixture positions: {error}")))?;
        self.next_ordinal = end;
        let batch = PointBatch::new(
            self.source,
            first,
            positions,
            AttributeColumns::empty(usize::try_from(count).expect("fixture count is addressable")),
        )
        .map_err(|error| SourceError::adapter(format!("budget fixture batch: {error}")))?;
        Ok(Some(batch))
    }
}

pub fn bounds_around(world: [f64; 3], radius: [f64; 3]) -> WorldBounds {
    WorldBounds::new(
        [
            world[0] - radius[0],
            world[1] - radius[1],
            world[2] - radius[2],
        ],
        [
            world[0] + radius[0],
            world[1] + radius[1],
            world[2] + radius[2],
        ],
    )
    .expect("fixture bounds are valid")
}

pub fn point_is_inside(world: [f64; 3], bounds: WorldBounds) -> bool {
    (0..3).all(|axis| bounds.min()[axis] <= world[axis] && world[axis] <= bounds.max()[axis])
}

pub fn ordinal_is_covered(ordinal: u64, spans: &[point_source::SourceSpan]) -> bool {
    spans
        .iter()
        .any(|span| span.first_ordinal() <= ordinal && ordinal < span.end_ordinal())
}

pub struct ObservedNodeRead {
    pub batches: Vec<IndexPointBatch>,
    pub summary: IndexReadSummary,
}

pub fn read_node(
    index: &PreparedIndex,
    node: IndexNodeId,
    budget: NodeReadBudget,
) -> ObservedNodeRead {
    let mut stream = index
        .read_node(node, budget)
        .expect("fixture node read starts");
    let mut batches = Vec::new();
    while let Some(batch) = stream.next().expect("fixture node read succeeds") {
        batches.push(batch);
    }
    assert!(stream.next().expect("successful stream is fused").is_none());
    let summary = stream
        .summary()
        .expect("successful node read publishes a summary")
        .clone();
    ObservedNodeRead { batches, summary }
}

pub fn samples(read: &ObservedNodeRead) -> Vec<(u64, [i64; 3])> {
    read.batches
        .iter()
        .flat_map(IndexPointBatch::samples)
        .map(|sample| (sample.ordinal(), sample.ticks()))
        .collect()
}
