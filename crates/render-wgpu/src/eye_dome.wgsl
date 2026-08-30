struct EyeDomeConfig {
    strength: f32,
    radius_pixels: u32,
    clear_alpha: f32,
    _padding: u32,
}

@group(0) @binding(0)
var point_color: texture_2d<f32>;

@group(0) @binding(1)
var point_depth: texture_depth_2d;

@group(0) @binding(2)
var<uniform> config: EyeDomeConfig;

@vertex
fn fullscreen_vertex(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[vertex_index], 0.0, 1.0);
}

@fragment
fn eye_dome_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let dimensions = vec2<i32>(textureDimensions(point_depth));
    let color = textureLoad(point_color, pixel, 0);
    let center = textureLoad(point_depth, pixel, 0);
    if center >= 1.0 || color.a <= 0.0 {
        return vec4<f32>(color.rgb, config.clear_alpha);
    }

    let radius = i32(config.radius_pixels);
    let offsets = array<vec2<i32>, 4>(
        vec2<i32>(radius, 0),
        vec2<i32>(-radius, 0),
        vec2<i32>(0, radius),
        vec2<i32>(0, -radius),
    );
    var discontinuity = 0.0;
    for (var index = 0u; index < 4u; index += 1u) {
        let neighbor_pixel = clamp(pixel + offsets[index], vec2<i32>(0), dimensions - 1);
        let neighbor = textureLoad(point_depth, neighbor_pixel, 0);
        discontinuity = max(discontinuity, max(0.0, neighbor - center));
    }
    let shade = clamp(exp(-config.strength * discontinuity * 300.0), 0.35, 1.0);
    return vec4<f32>(color.rgb * shade, color.a);
}
