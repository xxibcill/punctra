//! Public-interface tests for the protocol reference state model.

use render_protocol::{
    BatchKey, BatchVersion, FrameKey, PointBatch, PointId, ProtocolError, RenderLimits,
    RenderPoint, RenderStateModel, RenderUpdate, ResidentResource, UpdateKind, ViewId,
};

#[test]
fn reset_begins_exactly_one_generation_and_is_observable() {
    let mut state = RenderStateModel::new(RenderLimits::new(1_024, 32, 4));
    let frame = FrameKey::new(ViewId::new(10), 4);

    let report = state.apply(&RenderUpdate::Reset { frame }).unwrap();

    assert_eq!(report.kind(), UpdateKind::Reset);
    assert_eq!(report.frame(), frame);
    assert_eq!(report.resident().batch_count(), 0);
    assert_eq!(state.snapshot().active_frame(), Some(frame));

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Reset { frame }),
        Err(ProtocolError::GenerationAlreadyStarted { frame })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn upsert_inserts_and_strictly_newer_versions_replace_atomically() {
    let frame = frame(1, 0);
    let mut state = started_state(frame, RenderLimits::new(1_024, 32, 4));

    let inserted = state
        .apply(&RenderUpdate::Upsert {
            batch: batch(frame, 8, 3, &[10, 11]),
        })
        .unwrap();

    assert_eq!(inserted.kind(), UpdateKind::BatchInserted);
    assert_eq!(inserted.uploaded_points(), 2);
    assert_eq!(inserted.uploaded_bytes(), 64);
    assert_eq!(inserted.removed_points(), 0);
    assert_eq!(inserted.resident().batch_count(), 1);
    assert_eq!(inserted.resident().point_count(), 2);
    assert_eq!(inserted.resident().estimated_gpu_bytes(), 64);
    assert_eq!(state.snapshot().batches()[0].key(), BatchKey::new(8));
    assert_eq!(
        state.snapshot().batches()[0].version(),
        BatchVersion::new(3)
    );

    let replaced = state
        .apply(&RenderUpdate::Upsert {
            batch: batch(frame, 8, 4, &[12]),
        })
        .unwrap();

    assert_eq!(replaced.kind(), UpdateKind::BatchReplaced);
    assert_eq!(replaced.uploaded_points(), 1);
    assert_eq!(replaced.uploaded_bytes(), 32);
    assert_eq!(replaced.removed_points(), 2);
    assert_eq!(replaced.removed_bytes(), 64);
    assert_eq!(replaced.resident().batch_count(), 1);
    assert_eq!(replaced.resident().point_count(), 1);

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(frame, 8, 4, &[13, 14]),
        }),
        Err(ProtocolError::BatchVersionNotIncreasing {
            key: BatchKey::new(8),
            previous: BatchVersion::new(4),
            received: BatchVersion::new(4),
        })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn conditional_remove_preserves_newer_batches_and_versions_survive_removal() {
    let frame = frame(2, 1);
    let mut state = started_state(frame, RenderLimits::new(1_024, 32, 4));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(frame, 5, 7, &[1, 2]),
        })
        .unwrap();

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Remove {
            frame,
            key: BatchKey::new(5),
            expected_version: BatchVersion::new(6),
        }),
        Err(ProtocolError::BatchVersionMismatch {
            key: BatchKey::new(5),
            resident: BatchVersion::new(7),
            expected: BatchVersion::new(6),
        })
    );
    assert_eq!(state.snapshot(), before);

    let removed = state
        .apply(&RenderUpdate::Remove {
            frame,
            key: BatchKey::new(5),
            expected_version: BatchVersion::new(7),
        })
        .unwrap();
    assert_eq!(removed.kind(), UpdateKind::BatchRemoved);
    assert_eq!(removed.removed_points(), 2);
    assert_eq!(removed.removed_bytes(), 64);
    assert_eq!(removed.resident().batch_count(), 0);

    let empty = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(frame, 5, 7, &[3]),
        }),
        Err(ProtocolError::BatchVersionNotIncreasing {
            key: BatchKey::new(5),
            previous: BatchVersion::new(7),
            received: BatchVersion::new(7),
        })
    );
    assert_eq!(state.snapshot(), empty);

    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(frame, 5, 8, &[3]),
        })
        .unwrap();
}

#[test]
fn every_non_reset_update_must_match_the_active_frame() {
    let expected = frame(3, 4);
    let received = frame(3, 3);
    let update = RenderUpdate::Upsert {
        batch: batch(received, 1, 0, &[1]),
    };
    let mut state = RenderStateModel::new(RenderLimits::new(1_024, 32, 4));

    assert_eq!(
        state.apply(&update),
        Err(ProtocolError::GenerationNotStarted { received })
    );
    assert_eq!(state.snapshot().active_frame(), None);

    state
        .apply(&RenderUpdate::Reset { frame: expected })
        .unwrap();
    let before = state.snapshot();
    assert_eq!(
        state.apply(&update),
        Err(ProtocolError::FrameMismatch {
            active: expected,
            received,
        })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn reset_requires_forward_progress_per_view_and_clears_generation_state() {
    let first = frame(4, 5);
    let mut state = started_state(first, RenderLimits::new(1_024, 32, 4));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(first, 1, 0, &[1]),
        })
        .unwrap();
    state
        .apply(&RenderUpdate::SetHighlights {
            frame: first,
            point_ids: vec![PointId::new(1)],
        })
        .unwrap();

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Reset { frame: frame(4, 4) }),
        Err(ProtocolError::StaleGeneration {
            view: ViewId::new(4),
            last_generation: 5,
            received_generation: 4,
        })
    );
    assert_eq!(state.snapshot(), before);

    let next = frame(4, 6);
    let report = state.apply(&RenderUpdate::Reset { frame: next }).unwrap();
    assert_eq!(report.removed_points(), 1);
    assert_eq!(report.removed_bytes(), 32);
    assert!(state.snapshot().batches().is_empty());
    assert!(state.snapshot().highlights().is_empty());
    assert_eq!(state.snapshot().active_frame(), Some(next));

    let other_view = frame(40, 0);
    state
        .apply(&RenderUpdate::Reset { frame: other_view })
        .unwrap();
    assert_eq!(state.snapshot().active_frame(), Some(other_view));
    assert_eq!(
        state.apply(&RenderUpdate::Reset { frame: next }),
        Err(ProtocolError::GenerationAlreadyStarted { frame: next })
    );
}

#[test]
fn highlights_are_a_sorted_distinct_replaceable_set() {
    let frame = frame(5, 0);
    let mut state = started_state(frame, RenderLimits::new(0, 0, 0));

    let report = state
        .apply(&RenderUpdate::SetHighlights {
            frame,
            point_ids: vec![PointId::new(9), PointId::new(2), PointId::new(9)],
        })
        .unwrap();

    assert_eq!(report.kind(), UpdateKind::HighlightsSet);
    assert_eq!(report.highlight_count(), 2);
    assert_eq!(
        state.snapshot().highlights(),
        &[PointId::new(2), PointId::new(9)]
    );

    state
        .apply(&RenderUpdate::SetHighlights {
            frame,
            point_ids: Vec::new(),
        })
        .unwrap();
    assert!(state.snapshot().highlights().is_empty());
}

#[test]
fn byte_point_and_batch_limits_are_hard_and_transactional() {
    assert_limit_rejection(
        RenderLimits::new(31, 10, 10),
        &[1],
        ResidentResource::EstimatedGpuBytes,
        31,
        32,
    );
    assert_limit_rejection(
        RenderLimits::new(1_024, 1, 10),
        &[1, 2],
        ResidentResource::Points,
        1,
        2,
    );

    let frame = frame(6, 0);
    let mut state = started_state(frame, RenderLimits::new(1_024, 10, 1));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(frame, 1, 0, &[1]),
        })
        .unwrap();
    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(frame, 2, 0, &[2]),
        }),
        Err(ProtocolError::ResidentLimitExceeded {
            resource: ResidentResource::Batches,
            limit: 1,
            attempted: 2,
        })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn rejected_replacement_keeps_the_complete_previous_batch() {
    let frame = frame(7, 0);
    let mut state = started_state(frame, RenderLimits::new(64, 2, 1));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(frame, 1, 1, &[1]),
        })
        .unwrap();
    let before = state.snapshot();

    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(frame, 1, 2, &[2, 3, 4]),
        }),
        Err(ProtocolError::ResidentLimitExceeded {
            resource: ResidentResource::EstimatedGpuBytes,
            limit: 64,
            attempted: 96,
        })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn removing_a_missing_batch_is_rejected_without_side_effects() {
    let frame = frame(8, 0);
    let mut state = started_state(frame, RenderLimits::new(1_024, 32, 4));
    let before = state.snapshot();

    assert_eq!(
        state.apply(&RenderUpdate::Remove {
            frame,
            key: BatchKey::new(99),
            expected_version: BatchVersion::new(0),
        }),
        Err(ProtocolError::BatchNotResident {
            key: BatchKey::new(99),
        })
    );
    assert_eq!(state.snapshot(), before);
}

fn assert_limit_rejection(
    limits: RenderLimits,
    point_ids: &[u64],
    resource: ResidentResource,
    limit: u64,
    attempted: u64,
) {
    let frame = frame(50, 0);
    let mut state = started_state(frame, limits);
    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(frame, 1, 0, point_ids),
        }),
        Err(ProtocolError::ResidentLimitExceeded {
            resource,
            limit,
            attempted,
        })
    );
    assert_eq!(state.snapshot(), before);
}

fn started_state(frame: FrameKey, limits: RenderLimits) -> RenderStateModel {
    let mut state = RenderStateModel::new(limits);
    state.apply(&RenderUpdate::Reset { frame }).unwrap();
    state
}

fn batch(frame: FrameKey, key: u64, version: u64, point_ids: &[u64]) -> PointBatch {
    let points = point_ids
        .iter()
        .map(|id| RenderPoint::new([0.0; 3], [255; 4], PointId::new(*id)).unwrap())
        .collect();
    PointBatch::new(
        frame,
        BatchKey::new(key),
        BatchVersion::new(version),
        [0.0; 3],
        points,
    )
    .unwrap()
}

fn frame(view: u64, generation: u64) -> FrameKey {
    FrameKey::new(ViewId::new(view), generation)
}
