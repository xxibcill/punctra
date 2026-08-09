//! Opens one LAS or LAZ file and prints its verified canonical Source facts.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use point_source::Source;

fn main() {
    if let Err(error) = run() {
        eprintln!("source-las inspect failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let path = input_path()?;
    let source_file_bytes = std::fs::metadata(&path)?.len();

    let open_started = Instant::now();
    let source = source_las::open(&path).blocking_wait()?;
    let open_elapsed = open_started.elapsed();

    print_verified_source(&source, open_elapsed.as_secs_f64());

    let expected_points = source.metadata().point_count();
    let stream_started = Instant::now();
    let mut batches = source.points()?;
    let mut batch_count = 0_u64;
    let mut point_count = 0_u64;
    let mut canonical_payload_bytes = 0_u64;

    while let Some(batch) = batches.next()? {
        batch_count = batch_count.checked_add(1).ok_or("batch count overflow")?;
        point_count = point_count
            .checked_add(batch.point_count())
            .ok_or("Point count overflow")?;
        canonical_payload_bytes = canonical_payload_bytes
            .checked_add(batch.estimated_payload_bytes())
            .ok_or("canonical payload byte count overflow")?;
    }

    let elapsed = stream_started.elapsed();
    let summary = batches
        .summary()
        .ok_or("a successfully exhausted Source read must have an exact summary")?;
    if point_count != expected_points || point_count != summary.exact_count() {
        return Err(format!(
            "streamed {point_count} Points, metadata declares {expected_points}, summary declares {}",
            summary.exact_count()
        )
        .into());
    }

    let seconds = elapsed.as_secs_f64();
    println!("\nExact read summary");
    println!("  batches: {batch_count}");
    println!("  Points: {}", summary.exact_count());
    println!("  normalized spans: {}", summary.spans().len());
    println!("  Attributes per Point: {}", summary.attributes().len());
    println!("  canonical payload bytes: {canonical_payload_bytes}");
    println!("  elapsed: {seconds:.3} s");
    println!("  throughput: {:.0} Points/s", rate(point_count, seconds));
    println!(
        "  canonical throughput: {:.2} MiB/s",
        mebibytes_per_second(canonical_payload_bytes, seconds)
    );
    println!(
        "  source-file throughput: {:.2} MiB/s",
        mebibytes_per_second(source_file_bytes, seconds)
    );
    println!("  summary Source: {}", summary.source());
    println!(
        "  summary content hash: {}",
        summary.provenance().content_hash()
    );
    Ok(())
}

fn input_path() -> Result<PathBuf, Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments.next().ok_or(
        "usage: cargo run -p source-las --example inspect --release -- <file.las|file.laz>",
    )?;
    if arguments.next().is_some() {
        return Err("inspect accepts exactly one LAS or LAZ path".into());
    }
    Ok(PathBuf::from(path))
}

fn print_verified_source(source: &Source, open_seconds: f64) {
    let metadata = source.metadata();
    let record = source.record();

    println!("Verified Source");
    println!("  identity: {}", source.identity());
    println!("  content hash: {}", record.content_hash());
    println!(
        "  adapter: {} contract {}",
        record.adapter_name(),
        record.adapter_version()
    );
    println!("  logical order: {}", record.logical_order());
    println!("  format: {}", metadata.format_name());
    println!("  Points: {}", metadata.point_count());
    println!("  Full open: {open_seconds:.3} s");

    let transform = metadata.position_transform();
    println!("  position offset: {:?}", transform.offset());
    println!("  position scale: {:?}", transform.scale());
    match metadata.world_bounds() {
        Some(bounds) => {
            println!("  bounds min: {:?}", bounds.min());
            println!("  bounds max: {:?}", bounds.max());
        }
        None => println!("  bounds: none (empty Source)"),
    }
    match metadata.coordinate_reference().as_wkt() {
        Some(wkt) => println!("  Coordinate Reference WKT: {wkt:?}"),
        None => println!("  Coordinate Reference: explicitly unknown"),
    }

    println!("\nAttribute schema ({})", metadata.attributes().len());
    for definition in metadata.attributes().definitions() {
        println!(
            "  {:>5}  {:<30} {:?}",
            definition.id().get(),
            definition.name(),
            definition.data_type()
        );
    }

    println!("\nFormat metadata ({})", metadata.metadata_records().len());
    for (index, record) in metadata.metadata_records().iter().enumerate() {
        println!(
            "  {index:>5}  {} / {}  ({} bytes)",
            record.namespace(),
            record.name(),
            record.payload().len()
        );
    }
}

#[allow(clippy::cast_precision_loss)]
fn rate(units: u64, seconds: f64) -> f64 {
    if seconds == 0.0 {
        0.0
    } else {
        units as f64 / seconds
    }
}

fn mebibytes_per_second(bytes: u64, seconds: f64) -> f64 {
    rate(bytes, seconds) / (1024.0 * 1024.0)
}
