//! Public-interface helpers shared by the v0.5 evidence suites.

use std::mem;

use point_contracts::PointId;
use point_index::CandidateLimits;
use point_source::ReadBudget;
use point_workspace::{
    PointIdReadLimits, PointSet, PointSetLimits, Workspace, WorkspaceSchema, create,
};

use crate::support::{TemporaryFixture, classification_attribute, prepare_fixture};

const MIB: u64 = 1024 * 1024;

pub fn create_fixture_workspace(
    label: &str,
    point_count: usize,
) -> (
    TemporaryFixture,
    point_index::PreparedIndex,
    Workspace,
    Vec<[i64; 3]>,
    Vec<u8>,
) {
    let (temporary, index, ticks, classifications) = prepare_fixture(label, point_count);
    let workspace = create(
        temporary.workspace_path(),
        index.clone(),
        WorkspaceSchema::new(classification_attribute()),
        point_workspace::OpenLimits::default(),
    )
    .blocking_wait()
    .expect("fixture Workspace creates");
    (temporary, index, workspace, ticks, classifications)
}

pub fn selection_limits(source_batch_points: u64, resident_bytes: u64) -> PointSetLimits {
    let read_budget = ReadBudget::new(source_batch_points, 8 * MIB)
        .expect("evidence Source batch ceilings are nonzero")
        .with_max_points(10_000_000)
        .with_max_adapter_working_bytes(16 * MIB);
    PointSetLimits::new(
        CandidateLimits::default(),
        read_budget,
        10_000_000,
        10_000_000,
        1_000_000,
        128 * MIB,
        256 * MIB,
        resident_bytes,
        256 * MIB,
    )
}

pub fn forced_spill_limits(source_batch_points: u64) -> PointSetLimits {
    selection_limits(source_batch_points, 0)
}

pub fn collect_ids(point_set: &PointSet, batch_points: u64) -> Vec<PointId> {
    let batch_bytes = batch_points
        .saturating_mul(u64::try_from(mem::size_of::<PointId>()).expect("PointId size fits u64"));
    let limits = PointIdReadLimits::new(
        point_set.metadata().exact_count(),
        batch_points,
        batch_bytes.max(1),
        8 * MIB,
        16 * MIB,
    );
    let mut batches = point_set
        .ids(limits)
        .expect("Point Set identity stream opens");
    let mut ids = Vec::new();
    while let Some(batch) = batches.next().expect("Point Set identity batch validates") {
        ids.extend_from_slice(batch.ids());
    }
    ids
}

pub fn ordinals(point_set: &PointSet, batch_points: u64) -> Vec<u64> {
    collect_ids(point_set, batch_points)
        .into_iter()
        .map(PointId::ordinal)
        .collect()
}
