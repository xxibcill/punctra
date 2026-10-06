//! Conditional raster coverage controls preserve the resident batch authority.

use render_protocol::{
    BatchKey, BatchVersion, PointBatch, PointId, ProtocolError, RasterTransition,
    RasterTransitionSide, RenderLimits, RenderPoint, RenderStateModel, RenderUpdate, SourceId,
    UpdateEffect, UpdateKind, ViewGenerationKey, ViewId,
};

#[test]
fn complementary_progress_is_bounded_and_full_is_the_default() {
    assert_eq!(RasterTransition::default(), RasterTransition::FULL);
    assert_eq!(RasterTransition::FULL.coverage_eighths(), 8);
    assert_eq!(RasterTransition::HIDDEN.coverage_eighths(), 0);
    for step in 0..=8 {
        let incoming =
            RasterTransition::new(u32::MAX, RasterTransitionSide::Incoming, step).unwrap();
        let outgoing =
            RasterTransition::new(u32::MAX, RasterTransitionSide::Outgoing, step).unwrap();
        assert_eq!(incoming.coverage_eighths() + outgoing.coverage_eighths(), 8);
        assert_eq!(incoming.seed(), outgoing.seed());
        assert_eq!(incoming.step(), step);
    }
    for step in [9, 127, 255] {
        assert_eq!(
            RasterTransition::new(0, RasterTransitionSide::Incoming, step),
            Err(ProtocolError::InvalidRasterTransitionStep { step }),
        );
    }
}

#[test]
fn raster_controls_authorize_only_an_exact_resident_version_without_accounting_changes() {
    let generation = ViewGenerationKey::new(ViewId::new(23), 4);
    let key = BatchKey::new(1);
    let version = BatchVersion::new(2);
    let transition = RasterTransition::new(0x2345, RasterTransitionSide::Incoming, 3).unwrap();
    let mut state = RenderStateModel::new(RenderLimits::new(24, 1, 1));
    let update = |view_generation, key, expected_version| RenderUpdate::SetBatchRasterTransition {
        view_generation,
        key,
        expected_version,
        transition,
    };
    let before = state.snapshot();
    assert!(state.apply(&update(generation, key, version)).is_err());
    assert_eq!(state.snapshot(), before);
    state
        .apply(&RenderUpdate::Reset {
            view_generation: generation,
        })
        .unwrap();
    let point =
        RenderPoint::new([0.0; 3], [255; 4], PointId::new(SourceId::new([23; 32]), 1)).unwrap();
    let batch = PointBatch::new(generation, key, version, [0.0; 3], vec![point]).unwrap();
    state.apply(&RenderUpdate::Upsert { batch }).unwrap();
    let before = state.snapshot();
    for rejected in [
        update(ViewGenerationKey::new(ViewId::new(23), 3), key, version),
        update(ViewGenerationKey::new(ViewId::new(24), 4), key, version),
        update(generation, BatchKey::new(2), version),
        update(generation, key, BatchVersion::new(1)),
    ] {
        assert!(state.apply(&rejected).is_err());
        assert_eq!(state.snapshot(), before);
    }
    let accepted = update(generation, key, version);
    let result = state.apply(&accepted).unwrap();
    assert_eq!(
        result.effect(),
        UpdateEffect::BatchRasterTransitionSet { key, transition }
    );
    assert_eq!(result.report().kind(), UpdateKind::BatchRasterTransitionSet);
    assert_eq!(result.report().resident(), before.resident());
    assert_eq!(result.report().uploaded_bytes(), 0);
    assert_eq!(result.report().removed_bytes(), 0);
    assert_eq!(state.snapshot(), before);
}
