//! Throughput benchmark for bounded reads from a large pre-opened memory Source.

#![allow(missing_docs)]

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_source::{ReadBudget, ReadRequest, Source};
use source_memory::MemorySource;

const POINT_COUNT: usize = 262_144;
const MAX_BATCH_POINTS: u64 = 8_192;
const BYTES_PER_POINT: u64 = 24 + 2 + 1;
const MAX_BATCH_PAYLOAD_BYTES: u64 = 4_096 * BYTES_PER_POINT;

fn read_benchmark(criterion: &mut Criterion) {
    let source = large_preopened_source();
    let budget = ReadBudget::new(MAX_BATCH_POINTS, MAX_BATCH_PAYLOAD_BYTES).unwrap();
    let mut group = criterion.benchmark_group("source_memory_read");
    group.throughput(Throughput::Elements(u64::try_from(POINT_COUNT).unwrap()));

    group.bench_function("262k_points_preopened_bounded", |bencher| {
        bencher.iter(|| read_all(&source, budget));
    });
    group.finish();
}

fn read_all(source: &Source, budget: ReadBudget) {
    let mut batches = source.read(ReadRequest::all().budget(budget)).unwrap();
    let mut point_count = 0_u64;

    while let Some(batch) = batches.next().unwrap() {
        assert!(batch.point_count() <= budget.max_batch_points());
        assert!(batch.estimated_payload_bytes() <= budget.max_batch_payload_bytes());
        point_count += batch.point_count();
        black_box(&batch);
    }

    assert_eq!(point_count, u64::try_from(POINT_COUNT).unwrap());
    assert_eq!(batches.summary().unwrap().exact_count(), point_count);
}

fn large_preopened_source() -> Source {
    let ticks = (0..POINT_COUNT)
        .map(|ordinal| {
            let ordinal = i64::try_from(ordinal).unwrap();
            [ordinal, ordinal % 4_096, ordinal / 4_096]
        })
        .collect::<Vec<_>>();
    let intensity = attribute_column(
        1,
        "intensity",
        AttributeDataType::U16,
        AttributeValues::u16(
            (0..POINT_COUNT)
                .map(|ordinal| u16::try_from(ordinal % 65_536).unwrap())
                .collect(),
        ),
    );
    let classification = attribute_column(
        2,
        "classification",
        AttributeDataType::U8,
        AttributeValues::u8(
            (0..POINT_COUNT)
                .map(|ordinal| u8::try_from(ordinal % 32).unwrap())
                .collect(),
        ),
    );
    let attributes = AttributeColumns::new(vec![intensity, classification], POINT_COUNT).unwrap();
    let input = MemorySource::from_columns(
        PositionTransform::new([0.0; 3], [0.001; 3]).unwrap(),
        CoordinateReference::Unknown,
        ticks,
        attributes,
    )
    .unwrap();
    source_memory::open(input).blocking_wait().unwrap()
}

fn attribute_column(
    id: u32,
    name: &str,
    data_type: AttributeDataType,
    values: AttributeValues,
) -> AttributeColumn {
    let definition =
        AttributeDefinition::new(AttributeId::new(id).unwrap(), name, data_type).unwrap();
    AttributeColumn::new(definition, values).unwrap()
}

criterion_group!(benches, read_benchmark);
criterion_main!(benches);
