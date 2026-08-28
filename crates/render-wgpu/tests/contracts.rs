//! Public-interface tests for renderer values that require no GPU adapter.

use render_protocol::{ViewGenerationKey, ViewId};
use render_wgpu::{
    Camera, Frame, PointFootprint, PointFootprintStatus, PointStyle, RendererConfig, Viewport,
};

#[test]
fn frame_uses_the_documented_default_point_style() {
    let style = PointStyle::default();

    assert_eq!(style.default_size_pixels().to_bits(), 3.0_f32.to_bits());
    assert_eq!(style.display_size_pixels().to_bits(), 3.0_f32.to_bits());
    assert_eq!(
        style.highlight_color().map(f32::to_bits),
        [1.0, 0.8, 0.1].map(f32::to_bits)
    );
    assert_eq!(
        style.clear_color().map(f64::to_bits),
        [0.015, 0.02, 0.03, 1.0].map(f64::to_bits)
    );

    let camera: render_protocol::Camera =
        Camera::perspective([0.0, -1.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], 1.0, 0.1, 100.0)
            .expect("the re-exported contract camera should be valid");
    let frame = Frame::new(
        ViewGenerationKey::new(ViewId::new(1), 1),
        camera,
        Viewport::new(1, 1).unwrap(),
    )
    .expect("the contract frame should be valid");

    assert_eq!(frame.style(), style);
}

#[test]
fn renderer_defaults_to_the_explicit_single_sample_footprint() {
    let limits = render_protocol::RenderLimits::new(24, 1, 1);
    let default = RendererConfig::new(wgpu::TextureFormat::Rgba8Unorm, limits);
    let antialiased = default.with_point_footprint(PointFootprint::Antialiased);

    assert_eq!(default.point_footprint(), PointFootprint::SingleSample);
    assert_eq!(antialiased.point_footprint(), PointFootprint::Antialiased);
    assert_ne!(default, antialiased);
}

#[test]
fn point_footprint_status_variants_remain_distinct() {
    let statuses = [
        PointFootprintStatus::SingleSample,
        PointFootprintStatus::Multisample4x,
        PointFootprintStatus::UnsupportedFallback,
        PointFootprintStatus::ResourceFallback,
    ];

    for (index, status) in statuses.into_iter().enumerate() {
        assert!(!statuses[index + 1..].contains(&status));
    }
}
