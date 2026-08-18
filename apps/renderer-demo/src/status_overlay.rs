use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const FONT_WIDTH: u32 = 5;
const FONT_HEIGHT: u32 = 7;
const GLYPH_ADVANCE: u32 = 6;
const LINE_ADVANCE: u32 = 9;
const PANEL_MARGIN: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayVertex {
    position: [f32; 2],
    color: [f32; 4],
}

pub(crate) struct StatusOverlay {
    pipeline: wgpu::RenderPipeline,
}

impl StatusOverlay {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("punctra status overlay shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("status_overlay.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("punctra status overlay pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("punctra status overlay pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<OverlayVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    pub(crate) fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: [u32; 2],
        scale_factor: f64,
        lines: &[String],
    ) {
        let scale = if scale_factor >= 1.5 { 4 } else { 2 };
        let mut vertices = Vec::new();
        let columns = u32::try_from(lines.iter().map(String::len).max().unwrap_or_default())
            .unwrap_or(u32::MAX);
        let panel_width = columns * GLYPH_ADVANCE * scale + PANEL_MARGIN * 2;
        let panel_height = u32::try_from(lines.len()).unwrap_or(u32::MAX) * LINE_ADVANCE * scale
            + PANEL_MARGIN * 2;
        push_rect(
            &mut vertices,
            viewport,
            [0, 0],
            [panel_width.min(viewport[0]), panel_height.min(viewport[1])],
            [0.01, 0.02, 0.04, 0.88],
        );
        for (row, line) in lines.iter().enumerate() {
            let row = u32::try_from(row).unwrap_or(u32::MAX);
            for (column, character) in line.chars().enumerate() {
                let column = u32::try_from(column).unwrap_or(u32::MAX);
                push_glyph(
                    &mut vertices,
                    viewport,
                    [
                        PANEL_MARGIN + column * GLYPH_ADVANCE * scale,
                        PANEL_MARGIN + row * LINE_ADVANCE * scale,
                    ],
                    scale,
                    character,
                );
            }
        }
        if vertices.is_empty() {
            return;
        }
        let vertex_count = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("punctra status overlay vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("punctra status overlay pass"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}

fn push_glyph(
    vertices: &mut Vec<OverlayVertex>,
    viewport: [u32; 2],
    origin: [u32; 2],
    scale: u32,
    character: char,
) {
    let rows = glyph(character);
    for (y, bits) in rows.into_iter().enumerate() {
        let y = u32::try_from(y).unwrap_or(FONT_HEIGHT);
        for x in 0..FONT_WIDTH {
            if bits & (1 << (FONT_WIDTH - 1 - x)) != 0 {
                push_rect(
                    vertices,
                    viewport,
                    [origin[0] + x * scale, origin[1] + y * scale],
                    [scale, scale],
                    [0.88, 0.94, 1.0, 1.0],
                );
            }
        }
    }
}

fn push_rect(
    vertices: &mut Vec<OverlayVertex>,
    viewport: [u32; 2],
    origin: [u32; 2],
    size: [u32; 2],
    color: [f32; 4],
) {
    let ndc = |x: u32, y: u32| {
        [
            f32::from(u16::try_from(x.min(viewport[0])).unwrap_or(u16::MAX))
                / f32::from(
                    u16::try_from(viewport[0].min(u32::from(u16::MAX))).unwrap_or(u16::MAX),
                )
                * 2.0
                - 1.0,
            1.0 - f32::from(u16::try_from(y.min(viewport[1])).unwrap_or(u16::MAX))
                / f32::from(
                    u16::try_from(viewport[1].min(u32::from(u16::MAX))).unwrap_or(u16::MAX),
                )
                * 2.0,
        ]
    };
    let x1 = origin[0].saturating_add(size[0]);
    let y1 = origin[1].saturating_add(size[1]);
    let corners = [
        ndc(origin[0], origin[1]),
        ndc(x1, origin[1]),
        ndc(origin[0], y1),
        ndc(origin[0], y1),
        ndc(x1, origin[1]),
        ndc(x1, y1),
    ];
    vertices.extend(corners.map(|position| OverlayVertex { position, color }));
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 14],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        '>' => [16, 8, 4, 2, 4, 8, 16],
        '#' => [10, 31, 10, 10, 31, 10, 0],
        '=' => [0, 31, 0, 31, 0, 0, 0],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use renderer_demo::display::DisplayMode;

    use crate::{
        orbit_camera::ProjectionMode,
        scene::SceneMetrics,
        status::{MAX_STATUS_COLUMNS, MAX_STATUS_LINES, StatusSnapshot, StreamStatus},
    };

    use super::*;

    #[test]
    fn generated_status_characters_have_visible_glyphs() {
        for display in [
            DisplayMode::Neutral,
            DisplayMode::Elevation,
            DisplayMode::Rgb,
            DisplayMode::Intensity,
            DisplayMode::Classification,
        ] {
            let lines = StatusSnapshot {
                display,
                projection: ProjectionMode::Orthographic,
                stream: StreamStatus::Steady,
                scene: SceneMetrics {
                    logical_points: 12_345,
                    resident_points: 6_789,
                    sampled_resident_batches: 7,
                    complete_resident_batches: 3,
                    ..SceneMetrics::default()
                },
                drawn_points: 6_000,
                selected: None,
                clear_selection_available: false,
                resident_highlights: 0,
                orientation: "UP",
                scale_world_units: 125.25,
                cursor_world: Some([-6_378_137.25, 13_756_432.5, 120.0]),
            }
            .lines();

            for character in lines
                .iter()
                .flat_map(|line| line.chars())
                .filter(|character| !character.is_whitespace())
            {
                assert_ne!(
                    glyph(character),
                    [0; 7],
                    "status output has no glyph for {character:?}"
                );
            }
        }
    }

    #[test]
    fn two_hundred_percent_panel_fits_the_minimum_physical_window() {
        let width =
            u32::try_from(MAX_STATUS_COLUMNS).unwrap() * GLYPH_ADVANCE * 4 + PANEL_MARGIN * 2;
        let height = u32::try_from(MAX_STATUS_LINES).unwrap() * LINE_ADVANCE * 4 + PANEL_MARGIN * 2;
        assert!(width <= 1_280);
        assert!(height <= 960);
    }
}
