//! Shared deterministic fixtures for public `point-workspace` tests.

#![allow(
    dead_code,
    reason = "each integration test uses a different fixture subset"
)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_index::{PrepareLimits, PreparedIndex, prepare};
use point_source::Source;
use source_memory::MemorySource;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub fn classification_attribute() -> AttributeId {
    AttributeId::new(101).expect("fixture Attribute identity is nonzero")
}

pub struct TemporaryFixture {
    directory: PathBuf,
}

impl TemporaryFixture {
    pub fn new(label: &str) -> Self {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "punctra-point-workspace-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create isolated point-workspace test directory");
        Self { directory }
    }

    pub fn index_path(&self) -> PathBuf {
        self.directory.join("fixture.pidx")
    }

    pub fn workspace_path(&self) -> PathBuf {
        self.directory.join("fixture.pcw")
    }

    pub fn path(&self) -> &Path {
        &self.directory
    }
}

impl Drop for TemporaryFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

pub fn transform() -> PositionTransform {
    PositionTransform::new([100.25, -50.5, 1_000.0], [0.25, 0.5, 2.0])
        .expect("fixture transform is valid")
}

pub fn ticks_for_ordinal(ordinal: usize) -> [i64; 3] {
    let ordinal = i64::try_from(ordinal).expect("fixture ordinal fits i64");
    [
        ordinal.rem_euclid(97) - 48,
        (ordinal / 97).rem_euclid(89) - 44,
        ordinal.rem_euclid(17) - 8,
    ]
}

pub fn classification_for_ordinal(ordinal: usize) -> u8 {
    u8::try_from((ordinal * 7 + ordinal / 11) % 8).expect("fixture class fits u8")
}

pub fn fixture_rows(point_count: usize) -> (Vec<[i64; 3]>, Vec<u8>) {
    let ticks = (0..point_count).map(ticks_for_ordinal).collect();
    let classifications = (0..point_count).map(classification_for_ordinal).collect();
    (ticks, classifications)
}

pub fn open_source(ticks: Vec<[i64; 3]>, classifications: Vec<u8>) -> Source {
    open_source_with_reference(ticks, classifications, CoordinateReference::Unknown)
}

pub fn open_source_with_reference(
    ticks: Vec<[i64; 3]>,
    classifications: Vec<u8>,
    coordinate_reference: CoordinateReference,
) -> Source {
    assert_eq!(ticks.len(), classifications.len());
    let definition = AttributeDefinition::new(
        classification_attribute(),
        "classification",
        AttributeDataType::U8,
    )
    .expect("fixture classification definition is valid");
    let column = AttributeColumn::new(definition, AttributeValues::u8(classifications))
        .expect("fixture classification column is valid");
    let columns = AttributeColumns::new(vec![column], ticks.len())
        .expect("fixture Attribute columns are row-aligned");
    let input = MemorySource::from_columns(transform(), coordinate_reference, ticks, columns)
        .expect("fixture memory Source is valid");
    source_memory::open(input)
        .blocking_wait()
        .expect("fixture Source opens")
}

pub fn prepare_fixture(
    label: &str,
    point_count: usize,
) -> (TemporaryFixture, PreparedIndex, Vec<[i64; 3]>, Vec<u8>) {
    let temporary = TemporaryFixture::new(label);
    let (ticks, classifications) = fixture_rows(point_count);
    let source = open_source(ticks.clone(), classifications.clone());
    let index = prepare(source, temporary.index_path(), PrepareLimits::default())
        .blocking_wait()
        .expect("fixture index prepares");
    (temporary, index, ticks, classifications)
}

pub fn inclusive(bounds: point_contracts::WorldBounds, position: [f64; 3]) -> bool {
    let min = bounds.min();
    let max = bounds.max();
    (0..3).all(|axis| position[axis] >= min[axis] && position[axis] <= max[axis])
}
