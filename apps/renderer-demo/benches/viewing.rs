//! Reproducible generated viewing-path benchmark for renderer-demo.

#![allow(missing_docs)]

use std::{hint::black_box, path::Path};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
use point_index::{IndexRecipe, NodeReadBudget, PrepareLimits, prepare, prepare_fresh_with_recipe};
use renderer_demo::display::{DisplayMode, PointColorizer};
use source_memory::MemorySource;

const POINT_COUNT_ENV: &str = "PUNCTRA_RENDERER_VIEW_BENCH_POINTS";
const DEFAULT_POINT_COUNT: usize = 100_000;

fn benchmark_viewing(criterion: &mut Criterion) {
    let point_count = configured_point_count();
    let ticks = generated_ticks(point_count);
    let input = MemorySource::from_columns(
        PositionTransform::new([500_000.0, 4_600_000.0, 120.0], [0.01; 3]).unwrap(),
        CoordinateReference::Unknown,
        ticks,
        AttributeColumns::empty(point_count),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("viewing.pidx");
    let source = source_memory::open(input).blocking_wait().unwrap();
    let index = prepare_fresh_with_recipe(
        source.clone(),
        &target,
        IndexRecipe::PositionOnlyV1,
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let root = index.hierarchy().root().unwrap().id();
    eprintln!(
        "renderer-demo generated viewing fixture: points={point_count} nodes={} artifact_bytes={} temporary_peak_bytes={}",
        index.descriptor().node_count(),
        index.prepare_report().artifact_bytes(),
        index.prepare_report().peak_temporary_disk_bytes(),
    );

    let mut group = criterion.benchmark_group("renderer-demo/viewing");
    group.throughput(Throughput::Elements(u64::try_from(point_count).unwrap()));
    group.bench_function("warm-index-open", |bencher| {
        bencher.iter(|| {
            let opened = prepare(source.clone(), Path::new(&target), PrepareLimits::default())
                .blocking_wait()
                .unwrap();
            black_box(opened.prepare_report());
        });
    });
    group.bench_function("first-indexed-display-batch", |bencher| {
        bencher.iter(|| {
            let mut stream = index.read_node(root, NodeReadBudget::default()).unwrap();
            let batch = stream.next().unwrap().unwrap();
            let colorizer = PointColorizer::for_source(
                DisplayMode::Elevation,
                index.descriptor().world_bounds(),
            );
            for sample in batch.samples() {
                let world = sample.world_position(batch.transform());
                black_box(colorizer.color(world[2], None));
            }
            black_box(batch);
        });
    });
    group.finish();
}

fn configured_point_count() -> usize {
    match std::env::var(POINT_COUNT_ENV) {
        Ok(value) => {
            let value = value
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("{POINT_COUNT_ENV} must be a positive integer"));
            assert!((1..=10_000_000).contains(&value));
            value
        }
        Err(std::env::VarError::NotPresent) => DEFAULT_POINT_COUNT,
        Err(error) => panic!("could not read {POINT_COUNT_ENV}: {error}"),
    }
}

fn generated_ticks(point_count: usize) -> Vec<[i64; 3]> {
    (0..point_count)
        .map(|ordinal| {
            let ordinal = i64::try_from(ordinal).unwrap();
            [ordinal, (ordinal * 97) % 65_521, ordinal / 256]
        })
        .collect()
}

criterion_group!(benches, benchmark_viewing);
criterion_main!(benches);
