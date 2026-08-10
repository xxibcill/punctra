//! Public-interface tests for owned renderer-neutral values.

use render_protocol::{
    BatchKey, BatchVersion, Camera, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, PointId,
    ProtocolError, RenderPoint, SourceId, ViewGenerationKey, ViewId, Viewport, ViewportError,
};

const TEST_SOURCE: SourceId = SourceId::new([0x11; 32]);

#[test]
fn viewport_owns_physical_dimensions_and_projection_ratio() {
    let viewport = Viewport::new(1_920, 1_080).unwrap();

    assert_eq!(viewport.width(), 1_920);
    assert_eq!(viewport.height(), 1_080);
    assert_eq!(viewport.dimensions(), [1_920, 1_080]);
    assert_eq!(
        viewport.aspect_ratio().to_bits(),
        (1_920_f32 / 1_080_f32).to_bits()
    );
    assert_eq!(
        Viewport::new(1_920, 0),
        Err(ViewportError::Empty {
            dimensions: [1_920, 0],
        })
    );
}

#[test]
fn camera_is_a_renderer_neutral_projection_contract() {
    let camera = Camera::perspective(
        [1_000_000.0, 2_000_000.0, 100.0],
        [1_000_001.0, 2_000_000.0, 99.0],
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        10_000.0,
    )
    .expect("the contract camera should be valid");

    assert_eq!(
        camera.eye().map(f64::to_bits),
        [1_000_000.0, 2_000_000.0, 100.0].map(f64::to_bits)
    );
    assert_eq!(
        camera.target().map(f64::to_bits),
        [1_000_001.0, 2_000_000.0, 99.0].map(f64::to_bits)
    );
    assert_eq!(
        camera.up().map(f64::to_bits),
        [0.0, 0.0, 1.0].map(f64::to_bits)
    );
    assert_eq!(
        camera.vertical_field_of_view_radians().to_bits(),
        std::f32::consts::FRAC_PI_3.to_bits()
    );
    assert_eq!(camera.near_distance().to_bits(), 0.1_f32.to_bits());
    assert_eq!(camera.far_distance().to_bits(), 10_000.0_f32.to_bits());
    let basis = camera.world_basis();
    assert_vector_close(
        basis.forward(),
        [
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            -std::f64::consts::FRAC_1_SQRT_2,
        ],
    );
    assert_vector_close(basis.right(), [0.0, -1.0, 0.0]);
    assert_vector_close(
        basis.up(),
        [
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
    );

    let matrix = camera
        .view_projection_matrix(16.0 / 9.0)
        .expect("the validated camera should produce a finite projection");
    assert!(matrix.into_iter().all(f32::is_finite));
}

fn assert_vector_close(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < f64::EPSILON * 4.0);
    }
}

#[test]
fn point_batch_owns_valid_renderer_neutral_points() {
    let view_generation = ViewGenerationKey::new(ViewId::new(7), 3);
    let points = vec![
        RenderPoint::new([1.0, 2.0, 3.0], [10, 20, 30, 40], point_id(11)).unwrap(),
        RenderPoint::new([-4.0, 5.0, 0.25], [50, 60, 70, 80], point_id(12)).unwrap(),
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
    assert_eq!(batch.points()[0].point_id(), point_id(11));
}

#[test]
fn point_contracts_reject_non_finite_and_empty_data() {
    assert_eq!(
        RenderPoint::new([f32::NAN, 0.0, 0.0], [0; 4], point_id(1)),
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
    RenderPoint::new([0.0; 3], [255; 4], point_id(id)).unwrap()
}

const fn point_id(ordinal: u64) -> PointId {
    PointId::new(TEST_SOURCE, ordinal)
}
