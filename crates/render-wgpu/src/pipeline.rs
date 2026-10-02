use crate::footprint::MULTISAMPLE_COUNT;
use crate::gpu::{BatchUniform, CameraUniform, EdlUniform, GpuPoint};

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub(crate) const PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

pub(crate) struct PointPipelines {
    pub(crate) camera_layout: wgpu::BindGroupLayout,
    pub(crate) batch_layout: wgpu::BindGroupLayout,
    pub(crate) single_sample: PointPipelinePair,
    pub(crate) multisample: Option<PointPipelinePair>,
    pub(crate) eye_dome_depth: Option<wgpu::RenderPipeline>,
    pub(crate) pick: wgpu::RenderPipeline,
}

pub(crate) struct PointPipelinePair {
    pub(crate) opaque: wgpu::RenderPipeline,
    pub(crate) translucent: wgpu::RenderPipeline,
}

pub(crate) struct EdlPipeline {
    pub(crate) layout: wgpu::BindGroupLayout,
    pub(crate) pipeline: wgpu::RenderPipeline,
}

impl PointPipelines {
    pub(crate) fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        enable_edl: bool,
        enable_multisample: bool,
    ) -> Self {
        let (camera_layout, batch_layout) = point_bind_group_layouts(device, enable_edl);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("punctra point pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&batch_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("punctra point shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("point.wgsl").into()),
        });
        let vertex_buffers = [Some(GpuPoint::layout())];
        let color_targets = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pick_targets = [Some(wgpu::ColorTargetState {
            format: PICK_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let draw_fragment = if enable_edl {
            "eye_dome_point_fragment"
        } else {
            "point_fragment"
        };
        let single_sample = create_point_pipeline_pair(
            device,
            &pipeline_layout,
            &shader,
            &vertex_buffers,
            PointPipelinePairDescriptor {
                targets: &color_targets,
                vertex_entry_point: "point_vertex",
                fragment_entry_point: draw_fragment,
                sample_count: 1,
                opaque_label: "punctra point pipeline",
                translucent_label: "punctra translucent point pipeline",
            },
        );
        let multisample_fragment = if enable_edl {
            "multisample_eye_dome_point_fragment"
        } else {
            "multisample_point_fragment"
        };
        let multisample = enable_multisample.then(|| {
            create_point_pipeline_pair(
                device,
                &pipeline_layout,
                &shader,
                &vertex_buffers,
                PointPipelinePairDescriptor {
                    targets: &color_targets,
                    vertex_entry_point: "multisample_point_vertex",
                    fragment_entry_point: multisample_fragment,
                    sample_count: MULTISAMPLE_COUNT,
                    opaque_label: "punctra four-sample point pipeline",
                    translucent_label: "punctra four-sample translucent point pipeline",
                },
            )
        });
        let eye_dome_depth = enable_edl.then(|| {
            create_pipeline(
                device,
                &pipeline_layout,
                &shader,
                &vertex_buffers,
                PointPipelineDescriptor {
                    targets: &[],
                    vertex_entry_point: "point_vertex",
                    fragment_entry_point: "eye_dome_depth_fragment",
                    depth_write_enabled: true,
                    sample_count: 1,
                    label: "punctra eye-dome visibility depth pipeline",
                },
            )
        });
        let pick = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            &vertex_buffers,
            PointPipelineDescriptor {
                targets: &pick_targets,
                vertex_entry_point: "point_vertex",
                fragment_entry_point: "pick_fragment",
                depth_write_enabled: true,
                sample_count: 1,
                label: "punctra pick pipeline",
            },
        );
        Self {
            camera_layout,
            batch_layout,
            single_sample,
            multisample,
            eye_dome_depth,
            pick,
        }
    }
}

impl EdlPipeline {
    pub(crate) fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("punctra eye-dome bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<EdlUniform>() as u64),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("punctra eye-dome pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("punctra eye-dome shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("eye_dome.wgsl").into()),
        });
        let targets = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("punctra eye-dome pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("fullscreen_vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("eye_dome_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }
}

fn uniform_layout<T>(
    device: &wgpu::Device,
    label: &'static str,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<T>() as u64),
            },
            count: None,
        }],
    })
}

fn point_bind_group_layouts(
    device: &wgpu::Device,
    enable_edl: bool,
) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    let camera = uniform_layout::<CameraUniform>(
        device,
        "punctra camera layout",
        wgpu::ShaderStages::VERTEX,
    );
    let batch_visibility = if enable_edl {
        wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT
    } else {
        wgpu::ShaderStages::VERTEX
    };
    let batch = uniform_layout::<BatchUniform>(device, "punctra batch layout", batch_visibility);
    (camera, batch)
}

#[derive(Clone, Copy)]
struct PointPipelineDescriptor<'targets> {
    targets: &'targets [Option<wgpu::ColorTargetState>],
    vertex_entry_point: &'static str,
    fragment_entry_point: &'static str,
    depth_write_enabled: bool,
    sample_count: u32,
    label: &'static str,
}

#[derive(Clone, Copy)]
struct PointPipelinePairDescriptor<'targets> {
    targets: &'targets [Option<wgpu::ColorTargetState>],
    vertex_entry_point: &'static str,
    fragment_entry_point: &'static str,
    sample_count: u32,
    opaque_label: &'static str,
    translucent_label: &'static str,
}

fn create_point_pipeline_pair(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    descriptor: PointPipelinePairDescriptor<'_>,
) -> PointPipelinePair {
    let create = |depth_write_enabled, label| {
        create_pipeline(
            device,
            layout,
            shader,
            vertex_buffers,
            PointPipelineDescriptor {
                targets: descriptor.targets,
                vertex_entry_point: descriptor.vertex_entry_point,
                fragment_entry_point: descriptor.fragment_entry_point,
                depth_write_enabled,
                sample_count: descriptor.sample_count,
                label,
            },
        )
    };
    PointPipelinePair {
        opaque: create(true, descriptor.opaque_label),
        translucent: create(false, descriptor.translucent_label),
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    descriptor: PointPipelineDescriptor<'_>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(descriptor.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(descriptor.vertex_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(descriptor.depth_write_enabled),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: descriptor.sample_count,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(descriptor.fragment_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: descriptor.targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}
