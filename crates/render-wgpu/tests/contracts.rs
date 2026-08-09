//! Public-interface tests for renderer values that require no GPU adapter.

use render_protocol::{ViewGenerationKey, ViewId};
use render_wgpu::{Camera, Frame, PointStyle};

#[test]
fn frame_uses_the_documented_default_point_style() {
    let style = PointStyle::default();

    assert_eq!(style.default_size_pixels().to_bits(), 3.0_f32.to_bits());
    assert_eq!(
        style.highlight_color().map(f32::to_bits),
        [1.0, 0.8, 0.1].map(f32::to_bits)
    );
    assert_eq!(
        style.clear_color().map(f64::to_bits),
        [0.015, 0.02, 0.03, 1.0].map(f64::to_bits)
    );

    let camera = Camera::perspective([0.0, -1.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], 1.0, 0.1, 100.0)
        .expect("the contract camera should be valid");
    let frame = Frame::new(ViewGenerationKey::new(ViewId::new(1), 1), camera, [1, 1])
        .expect("the contract frame should be valid");

    assert_eq!(frame.style(), style);
}
