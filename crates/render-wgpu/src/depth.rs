use crate::pipeline::DEPTH_FORMAT;

pub(crate) struct DepthTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    viewport: [u32; 2],
}

impl DepthTarget {
    pub(crate) fn new(device: &wgpu::Device, viewport: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("punctra depth texture"),
            size: wgpu::Extent3d {
                width: viewport[0],
                height: viewport[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            _texture: texture,
            view,
            viewport,
        }
    }

    pub(crate) const fn viewport(&self) -> [u32; 2] {
        self.viewport
    }

    pub(crate) const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}
