struct CameraUniform {
    view_projection: mat4x4<f32>,
    viewport_size: vec2<f32>,
    default_point_size: f32,
    _padding: f32,
    highlight_color: vec3<f32>,
    _highlight_padding: f32,
}

struct BatchUniform {
    origin_from_camera: vec4<f32>,
    presentation_weight: f32,
    _presentation_padding_0: f32,
    _presentation_padding_1: f32,
    _presentation_padding_2: f32,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> batch: BatchUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) flags: u32,
    @location(3) pick_token: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) @interpolate(flat) pick_token: u32,
    @location(3) @interpolate(flat) source_alpha: f32,
}

struct MultisampleVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(linear, sample) corner: vec2<f32>,
    @location(2) @interpolate(flat) pick_token: u32,
    @location(3) @interpolate(flat) source_alpha: f32,
}

struct VertexValues {
    clip_position: vec4<f32>,
    color: vec4<f32>,
    corner: vec2<f32>,
    pick_token: u32,
    source_alpha: f32,
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

fn point_vertex_values(input: VertexInput, vertex_index: u32) -> VertexValues {
    let corner = quad_corner(vertex_index);
    let camera_relative_position = input.position + batch.origin_from_camera.xyz;
    var clip_position = camera.view_projection * vec4<f32>(camera_relative_position, 1.0);
    let pixel_to_clip = vec2<f32>(1.0) / camera.viewport_size;
    let displaced_xy =
        clip_position.xy + corner * camera.default_point_size * pixel_to_clip * clip_position.w;
    clip_position = vec4<f32>(displaced_xy, clip_position.zw);

    var output: VertexValues;
    output.clip_position = clip_position;
    let highlighted_color = vec4<f32>(
        camera.highlight_color,
        input.color.a,
    );
    output.color = select(input.color, highlighted_color, (input.flags & HIGHLIGHTED) != 0u);
    output.color.a *= batch.presentation_weight;
    output.corner = corner;
    output.pick_token = input.pick_token;
    output.source_alpha = input.color.a;
    return output;
}

@vertex
fn point_vertex(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let values = point_vertex_values(input, vertex_index);
    var output: VertexOutput;
    output.clip_position = values.clip_position;
    output.color = values.color;
    output.corner = values.corner;
    output.pick_token = values.pick_token;
    output.source_alpha = values.source_alpha;
    return output;
}

@vertex
fn multisample_point_vertex(
    input: VertexInput,
    @builtin(vertex_index) vertex_index: u32,
) -> MultisampleVertexOutput {
    let values = point_vertex_values(input, vertex_index);
    var output: MultisampleVertexOutput;
    output.clip_position = values.clip_position;
    output.color = values.color;
    output.corner = values.corner;
    output.pick_token = values.pick_token;
    output.source_alpha = values.source_alpha;
    return output;
}

@fragment
fn point_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.source_alpha <= 0.0 || !inside_splat(input.corner) {
        discard;
    }
    return input.color;
}

@fragment
fn eye_dome_point_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.source_alpha <= 0.0 || !inside_splat(input.corner) {
        discard;
    }
    return input.color;
}

@fragment
fn multisample_point_fragment(input: MultisampleVertexOutput) -> @location(0) vec4<f32> {
    if input.source_alpha <= 0.0 || !inside_splat(input.corner) {
        discard;
    }
    return input.color;
}

@fragment
fn multisample_eye_dome_point_fragment(
    input: MultisampleVertexOutput,
) -> @location(0) vec4<f32> {
    if input.source_alpha <= 0.0 || !inside_splat(input.corner) {
        discard;
    }
    return input.color;
}

@fragment
fn eye_dome_depth_fragment(input: VertexOutput) {
    if input.source_alpha <= 0.0
        || batch.presentation_weight <= 0.0
        || !inside_splat(input.corner) {
        discard;
    }
}

@fragment
fn pick_fragment(input: VertexOutput) -> @location(0) u32 {
    if input.source_alpha <= 0.0 || !inside_splat(input.corner) {
        discard;
    }
    return input.pick_token;
}

fn inside_splat(corner: vec2<f32>) -> bool {
    return dot(corner, corner) <= 1.0;
}
