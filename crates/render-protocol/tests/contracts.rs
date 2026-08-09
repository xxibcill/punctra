//! Public-interface tests for owned renderer-neutral values.

use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, PointId, ProtocolError,
    RenderPoint, ViewGenerationKey, ViewId,
};

#[test]
fn point_batch_owns_valid_renderer_neutral_points() {
    let view_generation = ViewGenerationKey::new(ViewId::new(7), 3);
    let points = vec![
        RenderPoint::new([1.0, 2.0, 3.0], [10, 20, 30, 40], PointId::new(11)).unwrap(),
        RenderPoint::new([-4.0, 5.0, 0.25], [50, 60, 70, 80], PointId::new(12)).unwrap(),
    ];

    let batch = PointBatch::new(
        view_generation,
        BatchKey::new(5),
        BatchVersion::new(9),
        [6_378_137.0, 1_000_000.0, -20.0],
        points,
    )
    .unwrap();

    assert_eq!(batch.view_generation(), view_generation);
    assert_eq!(batch.key(), BatchKey::new(5));
    assert_eq!(batch.version(), BatchVersion::new(9));
    assert_eq!(
        batch.world_origin().map(f64::to_bits),
        [6_378_137.0, 1_000_000.0, -20.0].map(f64::to_bits)
    );
    assert_eq!(batch.point_count(), 2);
    assert_eq!(
        batch.estimated_gpu_bytes(),
        2 * ESTIMATED_GPU_BYTES_PER_POINT
    );
    assert_eq!(
        batch.points()[0].relative_position().map(f32::to_bits),
        [1.0, 2.0, 3.0].map(f32::to_bits)
    );
    assert_eq!(batch.points()[0].color(), [10, 20, 30, 40]);
    assert_eq!(batch.points()[0].point_id(), PointId::new(11));
}

#[test]
fn point_contracts_reject_non_finite_and_empty_data() {
    assert_eq!(
        RenderPoint::new([f32::NAN, 0.0, 0.0], [0; 4], PointId::new(1)),
        Err(ProtocolError::NonFiniteRelativePosition { axis: 0 })
    );

    let view_generation = ViewGenerationKey::new(ViewId::new(1), 0);
    assert_eq!(
        PointBatch::new(
            view_generation,
            BatchKey::new(1),
            BatchVersion::new(0),
            [0.0, f64::INFINITY, 0.0],
            vec![point(1)],
        ),
        Err(ProtocolError::NonFiniteWorldOrigin { axis: 1 })
    );
    assert_eq!(
        PointBatch::new(
            view_generation,
            BatchKey::new(1),
            BatchVersion::new(0),
            [0.0; 3],
            Vec::new(),
        ),
        Err(ProtocolError::EmptyPointBatch)
    );
}

fn point(id: u64) -> RenderPoint {
    RenderPoint::new([0.0; 3], [255; 4], PointId::new(id)).unwrap()
}
