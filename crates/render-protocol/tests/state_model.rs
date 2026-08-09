//! Public-interface tests for the protocol reference state model.

use render_protocol::{
    BatchKey, BatchVersion, PointBatch, PointId, ProtocolError, RenderLimits, RenderPoint,
    RenderStateModel, RenderUpdate, ResidentResource, UpdateEffect, UpdateKind, ViewGenerationKey,
    ViewId,
};

#[test]
fn reset_begins_exactly_one_generation_and_is_observable() {
    let mut state = RenderStateModel::new(RenderLimits::new(1_024, 32, 4));
    let view_generation = ViewGenerationKey::new(ViewId::new(10), 4);

    let report = state
        .apply(&RenderUpdate::Reset { view_generation })
        .unwrap()
        .report();

    assert_eq!(report.kind(), UpdateKind::Reset);
    assert_eq!(report.view_generation(), view_generation);
    assert_eq!(report.resident().batch_count(), 0);
    assert_eq!(
        state.snapshot().active_view_generation(),
        Some(view_generation)
    );

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Reset { view_generation }),
        Err(ProtocolError::GenerationAlreadyStarted { view_generation })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn accepted_updates_describe_their_renderer_effects() {
    let view_generation = view_generation(11, 0);
    let mut state = RenderStateModel::new(RenderLimits::new(1_024, 32, 4));
    let reset = RenderUpdate::Reset { view_generation };
    assert_eq!(
        state.apply(&reset).unwrap().effect(),
        UpdateEffect::GenerationReset
    );

    let upsert = RenderUpdate::Upsert {
        batch: batch(view_generation, 7, 2, &[10, 11]),
    };
    let accepted_upsert = state.apply(&upsert).unwrap();
    let RenderUpdate::Upsert { batch } = &upsert else {
        unreachable!("the fixture is an upsert")
    };
    assert_eq!(
        accepted_upsert.effect(),
        UpdateEffect::BatchUpserted { batch }
    );

    let highlights = RenderUpdate::SetHighlights {
        view_generation,
        point_ids: vec![PointId::new(10)],
    };
    assert_eq!(
        state.apply(&highlights).unwrap().effect(),
        UpdateEffect::HighlightsSet
    );

    let remove = RenderUpdate::Remove {
        view_generation,
        key: BatchKey::new(7),
        expected_version: BatchVersion::new(2),
    };
    assert_eq!(
        state.apply(&remove).unwrap().effect(),
        UpdateEffect::BatchRemoved {
            key: BatchKey::new(7),
        }
    );
}

#[test]
fn upsert_inserts_and_strictly_newer_versions_replace_atomically() {
    let view_generation = view_generation(1, 0);
    let mut state = started_state(view_generation, RenderLimits::new(1_024, 32, 4));

    let inserted = state
        .apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 8, 3, &[10, 11]),
        })
        .unwrap()
        .report();

    assert_eq!(inserted.kind(), UpdateKind::BatchInserted);
    assert_eq!(inserted.uploaded_points(), 2);
    assert_eq!(inserted.uploaded_bytes(), 64);
    assert_eq!(inserted.removed_points(), 0);
    assert_eq!(inserted.resident().batch_count(), 1);
    assert_eq!(inserted.resident().point_count(), 2);
    assert_eq!(inserted.resident().estimated_gpu_bytes(), 64);
    assert_resident_batch(&state, view_generation, 8, 3, 2, 64);

    let replaced = state
        .apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 8, 4, &[12]),
        })
        .unwrap()
        .report();

    assert_eq!(replaced.kind(), UpdateKind::BatchReplaced);
    assert_eq!(replaced.uploaded_points(), 1);
    assert_eq!(replaced.uploaded_bytes(), 32);
    assert_eq!(replaced.removed_points(), 2);
    assert_eq!(replaced.removed_bytes(), 64);
    assert_eq!(replaced.resident().batch_count(), 1);
    assert_eq!(replaced.resident().point_count(), 1);
    assert_resident_batch(&state, view_generation, 8, 4, 1, 32);

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 8, 4, &[13, 14]),
        }),
        Err(ProtocolError::BatchVersionNotIncreasing {
            key: BatchKey::new(8),
            previous: BatchVersion::new(4),
            received: BatchVersion::new(4),
        })
    );
    assert_eq!(state.snapshot(), before);
    assert_resident_batch(&state, view_generation, 8, 4, 1, 32);
}

#[test]
fn conditional_remove_preserves_newer_batches_and_versions_survive_removal() {
    let view_generation = view_generation(2, 1);
    let mut state = started_state(view_generation, RenderLimits::new(1_024, 32, 4));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 5, 7, &[1, 2]),
        })
        .unwrap();

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Remove {
            view_generation,
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
            view_generation,
            key: BatchKey::new(5),
            expected_version: BatchVersion::new(7),
        })
        .unwrap()
        .report();
    assert_eq!(removed.kind(), UpdateKind::BatchRemoved);
    assert_eq!(removed.removed_points(), 2);
    assert_eq!(removed.removed_bytes(), 64);
    assert_eq!(removed.resident().batch_count(), 0);

    let empty = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 5, 7, &[3]),
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
            batch: batch(view_generation, 5, 8, &[3]),
        })
        .unwrap();
}

#[test]
fn every_non_reset_update_must_match_the_active_view_generation() {
    let expected = view_generation(3, 4);
    let received = view_generation(3, 3);
    let update = RenderUpdate::Upsert {
        batch: batch(received, 1, 0, &[1]),
    };
    let mut state = RenderStateModel::new(RenderLimits::new(1_024, 32, 4));

    assert_eq!(
        state.apply(&update),
        Err(ProtocolError::GenerationNotStarted { received })
    );
    assert_eq!(state.snapshot().active_view_generation(), None);

    state
        .apply(&RenderUpdate::Reset {
            view_generation: expected,
        })
        .unwrap();
    let before = state.snapshot();
    assert_eq!(
        state.apply(&update),
        Err(ProtocolError::ViewGenerationMismatch {
            active: expected,
            received,
        })
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn reset_requires_forward_progress_per_view_and_clears_generation_state() {
    let first = view_generation(4, 5);
    let mut state = started_state(first, RenderLimits::new(1_024, 32, 4));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(first, 1, 0, &[1]),
        })
        .unwrap();
    state
        .apply(&RenderUpdate::SetHighlights {
            view_generation: first,
            point_ids: vec![PointId::new(1)],
        })
        .unwrap();

    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Reset {
            view_generation: view_generation(4, 4),
        }),
        Err(ProtocolError::StaleGeneration {
            view: ViewId::new(4),
            last_generation: 5,
            received_generation: 4,
        })
    );
    assert_eq!(state.snapshot(), before);

    let next = view_generation(4, 6);
    let report = state
        .apply(&RenderUpdate::Reset {
            view_generation: next,
        })
        .unwrap()
        .report();
    assert_eq!(report.removed_points(), 1);
    assert_eq!(report.removed_bytes(), 32);
    assert_eq!(report.resident().batch_count(), 0);
    assert_batch_not_resident(&state, next, 1);
    assert!(state.snapshot().highlights().is_empty());
    assert_eq!(state.snapshot().active_view_generation(), Some(next));

    let other_view = view_generation(40, 0);
    state
        .apply(&RenderUpdate::Reset {
            view_generation: other_view,
        })
        .unwrap();
    assert_eq!(state.snapshot().active_view_generation(), Some(other_view));
    assert_eq!(
        state.apply(&RenderUpdate::Reset {
            view_generation: next,
        }),
        Err(ProtocolError::GenerationAlreadyStarted {
            view_generation: next,
        })
    );
}

#[test]
fn highlights_are_a_sorted_distinct_replaceable_set() {
    let view_generation = view_generation(5, 0);
    let mut state = started_state(view_generation, RenderLimits::new(0, 0, 0));

    let report = state
        .apply(&RenderUpdate::SetHighlights {
            view_generation,
            point_ids: vec![PointId::new(9), PointId::new(2), PointId::new(9)],
        })
        .unwrap()
        .report();

    assert_eq!(report.kind(), UpdateKind::HighlightsSet);
    assert_eq!(report.highlight_count(), 2);
    assert_eq!(
        state.snapshot().highlights(),
        &[PointId::new(2), PointId::new(9)]
    );

    state
        .apply(&RenderUpdate::SetHighlights {
            view_generation,
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

    let view_generation = view_generation(6, 0);
    let mut state = started_state(view_generation, RenderLimits::new(1_024, 10, 1));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 1, 0, &[1]),
        })
        .unwrap();
    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 2, 0, &[2]),
        }),
        Err(ProtocolError::ResidentLimitExceeded {
            resource: ResidentResource::Batches,
            limit: 1,
            attempted: 2,
        })
    );
    assert_eq!(state.snapshot(), before);
    assert_resident_batch(&state, view_generation, 1, 0, 1, 32);
}

#[test]
fn rejected_replacement_keeps_the_complete_previous_batch() {
    let view_generation = view_generation(7, 0);
    let mut state = started_state(view_generation, RenderLimits::new(64, 2, 1));
    state
        .apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 1, 1, &[1]),
        })
        .unwrap();
    let before = state.snapshot();

    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 1, 2, &[2, 3, 4]),
        }),
        Err(ProtocolError::ResidentLimitExceeded {
            resource: ResidentResource::EstimatedGpuBytes,
            limit: 64,
            attempted: 96,
        })
    );
    assert_eq!(state.snapshot(), before);
    assert_resident_batch(&state, view_generation, 1, 1, 1, 32);
}

#[test]
fn removing_a_missing_batch_is_rejected_without_side_effects() {
    let view_generation = view_generation(8, 0);
    let mut state = started_state(view_generation, RenderLimits::new(1_024, 32, 4));
    let before = state.snapshot();

    assert_eq!(
        state.apply(&RenderUpdate::Remove {
            view_generation,
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
    let view_generation = view_generation(50, 0);
    let mut state = started_state(view_generation, limits);
    let before = state.snapshot();
    assert_eq!(
        state.apply(&RenderUpdate::Upsert {
            batch: batch(view_generation, 1, 0, point_ids),
        }),
        Err(ProtocolError::ResidentLimitExceeded {
            resource,
            limit,
            attempted,
        })
    );
    assert_eq!(state.snapshot(), before);
}

fn assert_resident_batch(
    state: &RenderStateModel,
    view_generation: ViewGenerationKey,
    key: u64,
    version: u64,
    point_count: u64,
    estimated_gpu_bytes: u64,
) {
    let mut probe = state.clone();
    let report = probe
        .apply(&RenderUpdate::Remove {
            view_generation,
            key: BatchKey::new(key),
            expected_version: BatchVersion::new(version),
        })
        .expect("the expected resident batch should be removable from a cloned model")
        .report();
    assert_eq!(report.removed_points(), point_count);
    assert_eq!(report.removed_bytes(), estimated_gpu_bytes);
}

fn assert_batch_not_resident(
    state: &RenderStateModel,
    view_generation: ViewGenerationKey,
    key: u64,
) {
    let mut probe = state.clone();
    assert_eq!(
        probe.apply(&RenderUpdate::Remove {
            view_generation,
            key: BatchKey::new(key),
            expected_version: BatchVersion::new(0),
        }),
        Err(ProtocolError::BatchNotResident {
            key: BatchKey::new(key),
        })
    );
}

fn started_state(view_generation: ViewGenerationKey, limits: RenderLimits) -> RenderStateModel {
    let mut state = RenderStateModel::new(limits);
    state
        .apply(&RenderUpdate::Reset { view_generation })
        .unwrap();
    state
}

fn batch(
    view_generation: ViewGenerationKey,
    key: u64,
    version: u64,
    point_ids: &[u64],
) -> PointBatch {
    let points = point_ids
        .iter()
        .map(|id| RenderPoint::new([0.0; 3], [255; 4], PointId::new(*id)).unwrap())
        .collect();
    PointBatch::new(
        view_generation,
        BatchKey::new(key),
        BatchVersion::new(version),
        [0.0; 3],
        points,
    )
    .unwrap()
}

fn view_generation(view: u64, generation: u64) -> ViewGenerationKey {
    ViewGenerationKey::new(ViewId::new(view), generation)
}
