//! Builds, queries, and reads a deterministic index through its public API.

use std::{fs, path::PathBuf, time::SystemTime};

use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform, WorldBounds};
use point_index::{CandidateLimits, NodeReadBudget, PrepareLimits, prepare};
use source_memory::MemorySource;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let transform = PositionTransform::new([500_000.0, 2_000_000.0, 0.0], [0.01; 3])?;
    let ticks = vec![[0, 0, 100], [100, 0, 110], [0, 100, 120], [100, 100, 130]];
    let input = MemorySource::from_columns(
        transform,
        CoordinateReference::Unknown,
        ticks,
        AttributeColumns::empty(4),
    )?;
    let source = source_memory::open(input).blocking_wait()?;
    let directory = ExampleDirectory::create()?;
    let target = directory.path().join("example.pidx");

    let index = prepare(source, &target, PrepareLimits::default()).blocking_wait()?;
    let request = WorldBounds::new([500_000.0, 2_000_000.0, 0.0], [500_001.0, 2_000_001.0, 2.0])?;
    let candidates = index.candidates(request, CandidateLimits::default())?;
    println!(
        "{} nodes, {} candidate Points",
        index.descriptor().node_count(),
        candidates.candidate_point_count()
    );

    if let Some(root) = index.hierarchy().root() {
        let mut batches = index.read_node(root.id(), NodeReadBudget::default())?;
        while let Some(batch) = batches.next()? {
            for sample in batch.samples() {
                println!(
                    "ordinal {} at {:?}",
                    sample.ordinal(),
                    sample.world_position(batch.transform())
                );
            }
        }
        let summary = batches.summary().ok_or_else(|| {
            std::io::Error::other("successful root read ended without an exact summary")
        })?;
        assert_eq!(summary.node(), root.id());
        assert_eq!(summary.emitted_point_count(), root.display_point_count());
        assert_eq!(
            summary.covered_source_point_count(),
            root.covered_point_count()
        );
    }

    drop(index);
    Ok(())
}

struct ExampleDirectory {
    path: PathBuf,
}

impl ExampleDirectory {
    fn create() -> std::io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..1_000_u16 {
            let path = std::env::temp_dir().join(format!(
                "punctra-point-index-example-{}-{timestamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create an isolated example directory",
        ))
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for ExampleDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
