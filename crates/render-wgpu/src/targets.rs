use render_protocol::Viewport;

use crate::pipeline::{DEPTH_FORMAT, PICK_FORMAT};

#[derive(Default)]
pub(crate) struct RenderTargets {
    current: Option<ViewportTargets>,
}

impl RenderTargets {
    pub(crate) fn depth(&mut self, device: &wgpu::Device, viewport: Viewport) -> &DepthTarget {
        &self.for_viewport(device, viewport).depth
    }

    pub(crate) fn depth_and_pick(
        &mut self,
        device: &wgpu::Device,
        viewport: Viewport,
    ) -> (&DepthTarget, &PickTarget) {
        let targets = self.for_viewport(device, viewport);
        let pick = targets
            .pick
            .get_or_insert_with(|| PickTarget::new(device, viewport));
        (&targets.depth, pick)
    }

    fn for_viewport(&mut self, device: &wgpu::Device, viewport: Viewport) -> &mut ViewportTargets {
        let matches = self
            .current
            .as_ref()
            .is_some_and(|targets| targets.viewport == viewport);
        if !matches {
            self.current = Some(ViewportTargets {
                viewport,
                depth: DepthTarget::new(device, viewport),
                pick: None,
            });
        }
        self.current
            .as_mut()
            .expect("viewport targets were initialized above")
    }
}

struct ViewportTargets {
    viewport: Viewport,
    depth: DepthTarget,
    pick: Option<PickTarget>,
}

pub(crate) struct DepthTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl DepthTarget {
    fn new(device: &wgpu::Device, viewport: Viewport) -> Self {
        let (texture, view) = create_target(
            device,
            viewport,
            "punctra depth texture",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        Self {
            _texture: texture,
            view,
        }
    }

    pub(crate) const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

pub(crate) struct PickTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl PickTarget {
    fn new(device: &wgpu::Device, viewport: Viewport) -> Self {
        let (texture, view) = create_target(
            device,
            viewport,
            "punctra pick texture",
            PICK_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        Self { texture, view }
    }

    pub(crate) const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

fn create_target(
    device: &wgpu::Device,
    viewport: Viewport,
    label: &'static str,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: texture_extent(viewport),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

const fn texture_extent(viewport: Viewport) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: viewport.width(),
        height: viewport.height(),
        depth_or_array_layers: 1,
    }
}
