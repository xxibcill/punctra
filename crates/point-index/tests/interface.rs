//! Public-interface construction, hierarchy, and node-read conformance.

mod support;

use std::fs;

use foundation_runtime::{ProgressPhase, RuntimeError};
use point_contracts::WorldBounds;
use point_index::{
    CandidateLimits, DisplayCoverage, IndexError, IndexNode, IndexNodeId, NodeReadBudget,
    PrepareDisposition, PrepareLimits, PreparedIndex, prepare,
};
use point_source::{ReadLimit, Source};

use support::{
    BLOCK_POINTS, ObservedNodeRead, TemporaryTarget, clustered_ticks, open_budgeted_source,
    open_controlled_source, open_source, open_source_with_transform, read_node, samples,
    ticks_for_ordinal, transform,
};

#[test]
fn empty_build_and_warm_open_publish_complete_zero_facts() {
    let source = open_source(Vec::new());
    let target = TemporaryTarget::new("empty");

    let built = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        built.prepare_report().disposition(),
        PrepareDisposition::Built
    );
    assert_eq!(built.prepare_report().source_points_read(), 0);
    assert_eq!(built.prepare_report().durable_points_reused(), 0);
    assert_eq!(built.descriptor().source(), source.identity());
    assert_eq!(built.descriptor().source_point_count(), 0);
    assert_eq!(built.descriptor().position_transform(), transform());
    assert_eq!(built.descriptor().world_bounds(), None);
    assert_eq!(built.descriptor().node_count(), 0);
    assert_eq!(built.descriptor().leaf_count(), 0);
    assert_eq!(built.hierarchy().root(), None);
    assert!(built.hierarchy().nodes().is_empty());
    assert_eq!(
        fs::metadata(target.path()).unwrap().len(),
        built.prepare_report().artifact_bytes()
    );
    assert_eq!(fs::metadata(target.work_path()).unwrap().len(), 200);

    let arbitrary = WorldBounds::new([-1.0; 3], [1.0; 3]).unwrap();
    let plan = built
        .candidates(arbitrary, CandidateLimits::new(0, 0, 0, 0))
        .unwrap();
    assert!(plan.spans().is_empty());
    assert_eq!(plan.candidate_point_count(), 0);
    assert_eq!(plan.visited_node_count(), 0);

    let opened = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        opened.prepare_report().disposition(),
        PrepareDisposition::Opened
    );
    assert_eq!(opened.prepare_report().source_points_read(), 0);
    assert_eq!(opened.prepare_report().durable_points_reused(), 0);
    assert_eq!(opened.descriptor(), built.descriptor());
    assert_eq!(opened.hierarchy(), built.hierarchy());
}

#[test]
fn multi_block_artifacts_hierarchies_and_samples_ignore_source_batch_partitioning() {
    let point_count = BLOCK_POINTS * 2 + 37;
    let ticks = clustered_ticks(point_count);
    let source = open_source(ticks.clone());
    let narrow_target = TemporaryTarget::new("partition-narrow");
    let wide_target = TemporaryTarget::new("partition-wide");
    let narrow_limits = PrepareLimits::new(257, 257 * 24).unwrap();
    let wide_limits = PrepareLimits::new(4_097, 4_097 * 24).unwrap();

    let narrow = prepare(source.clone(), narrow_target.path(), narrow_limits)
        .blocking_wait()
        .unwrap();
    let wide = prepare(source.clone(), wide_target.path(), wide_limits)
        .blocking_wait()
        .unwrap();

    assert_eq!(narrow.descriptor(), wide.descriptor());
    assert_eq!(narrow.hierarchy(), wide.hierarchy());
    assert_eq!(
        fs::read(narrow_target.path()).unwrap(),
        fs::read(wide_target.path()).unwrap()
    );
    assert_eq!(narrow.descriptor().source(), source.identity());
    assert_eq!(
        narrow.descriptor().source_point_count(),
        u64::try_from(point_count).unwrap()
    );
    assert_eq!(narrow.descriptor().position_transform(), transform());
    assert_eq!(
        narrow.descriptor().world_bounds(),
        source.metadata().world_bounds()
    );
    assert_eq!(narrow.descriptor().leaf_count(), 3);
    assert_eq!(narrow.descriptor().node_count(), 5);
    assert_ne!(narrow.descriptor().artifact_checksum(), [0; 32]);
    assert_eq!(
        narrow.prepare_report().source_points_read(),
        u64::try_from(point_count).unwrap()
    );

    let nodes = narrow.hierarchy().nodes();
    assert_eq!(nodes.len(), 5);
    assert_eq!(nodes[0].id().get(), 1);
    assert_eq!(nodes[0].parent(), None);
    assert_eq!(
        nodes[0].covered_point_count(),
        u64::try_from(point_count).unwrap()
    );
    assert_eq!(nodes[0].bounds(), source.metadata().world_bounds().unwrap());

    let mut leaf_ranges = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        assert_eq!(node.id().get(), u64::try_from(index + 1).unwrap());
        if let Some(parent) = node.parent() {
            assert!(parent.get() < node.id().get());
            assert!(narrow.hierarchy().get(parent).is_some());
        } else {
            assert_eq!(node.id().get(), 1);
        }
        assert_bounds_are_finite_and_nested(&narrow, node.id());

        let budget = NodeReadBudget::new(node.display_point_count(), 137 * 32)
            .unwrap()
            .with_max_source_batch_points(113)
            .with_max_source_batch_payload_bytes(113 * 24);
        let observed = assert_node_read_facts(&narrow, &source, &ticks, node, budget);
        let observed_samples = samples(&observed);

        match node.coverage() {
            DisplayCoverage::Complete => {
                assert_eq!(node.geometric_error().to_bits(), 0.0_f64.to_bits());
                assert_eq!(node.display_point_count(), node.covered_point_count());
                assert!(node.covered_point_count() <= u64::try_from(BLOCK_POINTS).unwrap());
                let first = observed_samples.first().unwrap().0;
                assert!(
                    observed_samples
                        .iter()
                        .enumerate()
                        .all(|(row, sample)| { sample.0 == first + u64::try_from(row).unwrap() })
                );
                leaf_ranges.push((first, node.covered_point_count()));
            }
            DisplayCoverage::Sampled => {
                assert_eq!(node.display_point_count(), 4_096);
                assert_eq!(
                    node.geometric_error().to_bits(),
                    diagonal(node.bounds()).to_bits()
                );
                let wide_samples = samples(&read_node(&wide, node.id(), budget));
                assert_eq!(observed_samples, wide_samples);
            }
        }
    }

    leaf_ranges.sort_unstable();
    assert_eq!(
        leaf_ranges,
        vec![
            (0, u64::try_from(BLOCK_POINTS).unwrap()),
            (
                u64::try_from(BLOCK_POINTS).unwrap(),
                u64::try_from(BLOCK_POINTS).unwrap(),
            ),
            (u64::try_from(BLOCK_POINTS * 2).unwrap(), 37),
        ]
    );
    assert_parent_aggregates(&narrow);
}

#[test]
fn node_reads_enforce_each_budget_and_fail_or_cancel_fused() {
    let ticks = clustered_ticks(BLOCK_POINTS + 9);
    let source = open_source(ticks);
    let target = TemporaryTarget::new("read-budgets");
    let index = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let root = index.hierarchy().root().unwrap();
    let leaf = index
        .hierarchy()
        .nodes()
        .iter()
        .find(|node| node.coverage_complete() && node.display_point_count() > 1)
        .unwrap();

    assert!(matches!(IndexNodeId::new(0), Err(IndexError::ZeroNodeId)));
    let unknown = IndexNodeId::new(u64::MAX).unwrap();
    assert!(matches!(
        index.read_node(unknown, NodeReadBudget::default()),
        Err(IndexError::UnknownNode { node }) if node == u64::MAX
    ));

    assert_resource_error(&index.read_node(
        root.id(),
        NodeReadBudget::new(root.display_point_count() - 1, 32).unwrap(),
    ));
    assert_resource_error(&index.read_node(
        root.id(),
        NodeReadBudget::new(root.display_point_count(), 31).unwrap(),
    ));
    assert_resource_error(
        &index.read_node(
            root.id(),
            NodeReadBudget::new(root.display_point_count(), 32)
                .unwrap()
                .with_max_index_buffer_bytes((root.display_point_count() - 1) * 32),
        ),
    );
    assert_resource_error(
        &index.read_node(
            leaf.id(),
            NodeReadBudget::new(leaf.display_point_count(), 32)
                .unwrap()
                .with_max_source_spans(0),
        ),
    );
    assert_resource_error(
        &index.read_node(
            leaf.id(),
            NodeReadBudget::new(leaf.display_point_count(), 32)
                .unwrap()
                .with_max_source_batch_points(0),
        ),
    );
    assert_resource_error(
        &index.read_node(
            leaf.id(),
            NodeReadBudget::new(leaf.display_point_count(), 32)
                .unwrap()
                .with_max_source_batch_payload_bytes(23),
        ),
    );

    let mut cancelled = index
        .read_node(
            root.id(),
            NodeReadBudget::new(root.display_point_count(), 32 * 17).unwrap(),
        )
        .unwrap();
    let handle = cancelled.handle();
    let first = cancelled.next().unwrap().unwrap();
    assert_eq!(first.len(), 17);
    assert_eq!(handle.progress().phase(), ProgressPhase::RUNNING);
    assert_eq!(handle.progress().completed_units(), 17);
    handle.cancel();
    assert!(matches!(
        cancelled.next(),
        Err(IndexError::Runtime(RuntimeError::Cancelled))
    ));
    assert!(cancelled.next().unwrap().is_none());
    assert!(cancelled.summary().is_none());
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn source_backed_leaf_failure_keeps_exact_category_and_fuses() {
    let ticks = clustered_ticks(12);
    let (source, faults) = open_controlled_source(ticks);
    let target = TemporaryTarget::new("read-source-fault");
    let index = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let root = index.hierarchy().root().unwrap();
    assert!(root.coverage_complete());
    faults.fail_at_ordinal(3);

    let budget = NodeReadBudget::new(root.display_point_count(), 2 * 32)
        .unwrap()
        .with_max_source_batch_points(2)
        .with_max_source_batch_payload_bytes(2 * 24);
    let mut stream = index.read_node(root.id(), budget).unwrap();
    let mut accepted = Vec::new();
    loop {
        match stream.next() {
            Ok(Some(batch)) => {
                accepted.extend(batch.samples().iter().map(|sample| sample.ordinal()));
            }
            Err(IndexError::Source(point_source::SourceError::CorruptSource { reason })) => {
                assert!(reason.contains("ordinal 3"));
                break;
            }
            other => panic!("expected injected Source corruption, got {other:?}"),
        }
    }
    assert_eq!(accepted, vec![0, 1, 2]);
    assert!(stream.next().unwrap().is_none());
    assert!(stream.summary().is_none());
}

#[test]
fn adapter_working_memory_is_forwarded_to_prepare_and_leaf_reads() {
    const REQUIRED_ADAPTER_BYTES: u64 = 8_192;

    let source = open_budgeted_source(clustered_ticks(12), REQUIRED_ADAPTER_BYTES);
    let target = TemporaryTarget::new("adapter-budget");
    let tight_prepare =
        PrepareLimits::default().with_max_adapter_working_bytes(REQUIRED_ADAPTER_BYTES - 1);
    assert!(matches!(
        prepare(source.clone(), target.path(), tight_prepare).blocking_wait(),
        Err(IndexError::Source(point_source::SourceError::ResourceLimit {
            limit: ReadLimit::AdapterWorkingBytes,
            required: REQUIRED_ADAPTER_BYTES,
            allowed,
        })) if allowed == REQUIRED_ADAPTER_BYTES - 1
    ));
    assert!(!target.path().exists());
    assert!(target.work_path().exists());

    let exact_prepare =
        PrepareLimits::default().with_max_adapter_working_bytes(REQUIRED_ADAPTER_BYTES);
    let index = prepare(source, target.path(), exact_prepare)
        .blocking_wait()
        .unwrap();
    let root = index.hierarchy().root().unwrap();
    assert!(root.coverage_complete());

    let tight_read = NodeReadBudget::new(root.display_point_count(), 32)
        .unwrap()
        .with_max_adapter_working_bytes(REQUIRED_ADAPTER_BYTES - 1);
    assert!(matches!(
        index.read_node(root.id(), tight_read),
        Err(IndexError::Source(point_source::SourceError::ResourceLimit {
            limit: ReadLimit::AdapterWorkingBytes,
            required: REQUIRED_ADAPTER_BYTES,
            allowed,
        })) if allowed == REQUIRED_ADAPTER_BYTES - 1
    ));

    let exact_read = NodeReadBudget::new(root.display_point_count(), 32)
        .unwrap()
        .with_max_adapter_working_bytes(REQUIRED_ADAPTER_BYTES);
    let observed = read_node(&index, root.id(), exact_read);
    assert_eq!(observed.summary.emitted_point_count(), 12);
}

#[test]
fn overflowing_internal_diagonal_saturates_and_reopens_with_exact_bits() {
    let extreme_transform = point_contracts::PositionTransform::new([0.0; 3], [1.0e289; 3])
        .expect("extreme finite transform is valid");
    let mut ticks = vec![[i64::MIN + 1; 3]; BLOCK_POINTS];
    ticks.push([i64::MAX - 1; 3]);
    let source = open_source_with_transform(ticks, extreme_transform);
    let target = TemporaryTarget::new("overflowing-diagonal");

    let built = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let root = built.hierarchy().root().unwrap();
    assert!(!root.coverage_complete());
    assert_eq!(root.geometric_error().to_bits(), f64::MAX.to_bits());
    assert!(
        root.bounds()
            .min()
            .into_iter()
            .chain(root.bounds().max())
            .all(f64::is_finite)
    );

    let reopened = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        reopened.prepare_report().disposition(),
        PrepareDisposition::Opened
    );
    assert_eq!(
        reopened
            .hierarchy()
            .root()
            .unwrap()
            .geometric_error()
            .to_bits(),
        f64::MAX.to_bits()
    );
    assert_eq!(reopened.descriptor(), built.descriptor());
    assert_eq!(reopened.hierarchy(), built.hierarchy());
}

fn assert_node_read_facts(
    index: &PreparedIndex,
    source: &Source,
    ticks: &[[i64; 3]],
    node: &IndexNode,
    budget: NodeReadBudget,
) -> ObservedNodeRead {
    let maximum_batch = if node.coverage_complete() { 113 } else { 137 };
    let observed = read_node(index, node.id(), budget);
    assert_eq!(observed.summary.node(), node.id());
    assert_eq!(observed.summary.source(), source.identity());
    assert_eq!(observed.summary.provenance(), source.provenance());
    assert_eq!(
        observed.summary.emitted_point_count(),
        node.display_point_count()
    );
    assert_eq!(
        observed.summary.covered_source_point_count(),
        node.covered_point_count()
    );
    assert_eq!(observed.summary.coverage(), node.coverage());
    assert_eq!(
        observed.summary.coverage_complete(),
        node.coverage_complete()
    );

    let mut previous = None;
    for batch in &observed.batches {
        assert_eq!(batch.source(), source.identity());
        assert_eq!(batch.transform(), transform());
        assert_eq!(batch.node(), node.id());
        assert!(!batch.is_empty());
        assert!(batch.len() <= maximum_batch);
        assert_eq!(
            batch.estimated_payload_bytes(),
            u64::try_from(batch.len()).unwrap() * 32
        );
        for sample in batch.samples() {
            assert!(previous.is_none_or(|ordinal| ordinal < sample.ordinal()));
            previous = Some(sample.ordinal());
            let exact_ticks = ticks[usize::try_from(sample.ordinal()).unwrap()];
            assert_eq!(sample.ticks(), exact_ticks);
            assert_eq!(
                sample.point_id(source.identity()).ordinal(),
                sample.ordinal()
            );
            assert_eq!(
                sample.point_id(source.identity()).source(),
                source.identity()
            );
            assert_eq!(
                sample.world_position(transform()).map(f64::to_bits),
                transform().world_f64(exact_ticks).map(f64::to_bits)
            );
        }
    }
    assert_eq!(
        observed
            .batches
            .iter()
            .map(point_index::IndexPointBatch::len)
            .sum::<usize>(),
        usize::try_from(node.display_point_count()).unwrap()
    );
    observed
}

fn assert_bounds_are_finite_and_nested(index: &PreparedIndex, node: IndexNodeId) {
    let node = index.hierarchy().get(node).unwrap();
    assert!(
        node.bounds()
            .min()
            .into_iter()
            .chain(node.bounds().max())
            .all(f64::is_finite)
    );
    if let Some(parent) = node.parent() {
        let parent = index.hierarchy().get(parent).unwrap();
        for axis in 0..3 {
            assert!(parent.bounds().min()[axis] <= node.bounds().min()[axis]);
            assert!(node.bounds().max()[axis] <= parent.bounds().max()[axis]);
        }
    }
}

fn assert_parent_aggregates(index: &point_index::PreparedIndex) {
    for parent in index
        .hierarchy()
        .nodes()
        .iter()
        .filter(|node| !node.coverage_complete())
    {
        let children = index
            .hierarchy()
            .nodes()
            .iter()
            .filter(|node| node.parent() == Some(parent.id()))
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert_eq!(
            parent.covered_point_count(),
            children
                .iter()
                .map(|child| child.covered_point_count())
                .sum()
        );
        for axis in 0..3 {
            assert_eq!(
                parent.bounds().min()[axis].to_bits(),
                children
                    .iter()
                    .map(|child| child.bounds().min()[axis])
                    .min_by(f64::total_cmp)
                    .unwrap()
                    .to_bits()
            );
            assert_eq!(
                parent.bounds().max()[axis].to_bits(),
                children
                    .iter()
                    .map(|child| child.bounds().max()[axis])
                    .max_by(f64::total_cmp)
                    .unwrap()
                    .to_bits()
            );
        }
    }
}

fn diagonal(bounds: WorldBounds) -> f64 {
    let extent = [
        bounds.max()[0] - bounds.min()[0],
        bounds.max()[1] - bounds.min()[1],
        bounds.max()[2] - bounds.min()[2],
    ];
    extent[0].hypot(extent[1]).hypot(extent[2])
}

fn assert_resource_error(result: &Result<point_index::IndexPointBatches, IndexError>) {
    assert!(matches!(result, Err(IndexError::ResourceLimit { .. })));
}

#[test]
fn fixture_generation_is_stable_at_block_boundaries() {
    assert_eq!(ticks_for_ordinal(0), [-128, -125, -14]);
    assert_eq!(ticks_for_ordinal(BLOCK_POINTS), [99_872, -125, 9_986]);
}
