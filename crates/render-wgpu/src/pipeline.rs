use crate::gpu::{BatchUniform, CameraUniform, EdlUniform, GpuPoint};

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub(crate) const PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

pub(crate) struct PointPipelines {
    pub(crate) camera_layout: wgpu::BindGroupLayout,
    pub(crate) batch_layout: wgpu::BindGroupLayout,
    pub(crate) draw: wgpu::RenderPipeline,
    pub(crate) pick: wgpu::RenderPipeline,
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
    ) -> Self {
        let camera_layout = uniform_layout::<CameraUniform>(
            device,
            "punctra camera layout",
            wgpu::ShaderStages::VERTEX,
        );
        let batch_visibility = if enable_edl {
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT
        } else {
            wgpu::ShaderStages::VERTEX
        };
        let batch_layout =
            uniform_layout::<BatchUniform>(device, "punctra batch layout", batch_visibility);
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
        let draw = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            &vertex_buffers,
            &color_targets,
            draw_fragment,
            "punctra point pipeline",
        );
        let pick = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            &vertex_buffers,
            &pick_targets,
            "pick_fragment",
            "punctra pick pipeline",
        );
        Self {
            camera_layout,
            batch_layout,
            draw,
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

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    targets: &[Option<wgpu::ColorTargetState>],
    fragment_entry_point: &'static str,
    label: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("point_vertex"),
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
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}
