struct CameraUniform {
    view_projection: mat4x4<f32>,
    viewport_size: vec2<f32>,
    default_point_size: f32,
    _padding: f32,
    highlight_color: vec4<f32>,
}

struct BatchUniform {
    origin_from_camera: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> batch: BatchUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) point_size: f32,
    @location(2) color: vec4<f32>,
    @location(3) flags: u32,
    @location(4) pick_token: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) @interpolate(flat) pick_token: u32,
}

const HIGHLIGHTED: u32 = 1u;

fn quad_corner(vertex_index: u32) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    return corners[vertex_index];
}

@vertex
fn point_vertex(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let corner = quad_corner(vertex_index);
    let camera_relative_position = input.position + batch.origin_from_camera.xyz;
    var clip_position = camera.view_projection * vec4<f32>(camera_relative_position, 1.0);
    let configured_size = select(camera.default_point_size, input.point_size, input.point_size > 0.0);
    let pixel_to_clip = vec2<f32>(1.0) / camera.viewport_size;
    let displaced_xy =
        clip_position.xy + corner * configured_size * pixel_to_clip * clip_position.w;
    clip_position = vec4<f32>(displaced_xy, clip_position.zw);

    var output: VertexOutput;
    output.clip_position = clip_position;
    let highlighted_color = vec4<f32>(
        camera.highlight_color.rgb,
        input.color.a * camera.highlight_color.a,
    );
    output.color = select(input.color, highlighted_color, (input.flags & HIGHLIGHTED) != 0u);
    output.corner = corner;
    output.pick_token = input.pick_token;
    return output;
}

@fragment
fn point_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    if !splat_is_visible(input) {
        discard;
    }
    return input.color;
}

@fragment
fn pick_fragment(input: VertexOutput) -> @location(0) u32 {
    if !splat_is_visible(input) {
        discard;
    }
    return input.pick_token;
}

fn splat_is_visible(input: VertexOutput) -> bool {
    return input.color.a > 0.0 && dot(input.corner, input.corner) <= 1.0;
}
