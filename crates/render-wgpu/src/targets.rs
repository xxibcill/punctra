use render_protocol::Viewport;

use crate::{
    footprint::MULTISAMPLE_COUNT,
    pipeline::{DEPTH_FORMAT, PICK_FORMAT},
};

pub(crate) struct RenderTargets {
    color_format: wgpu::TextureFormat,
    edl_color_format: Option<wgpu::TextureFormat>,
    current: Option<ViewportTargets>,
}

impl RenderTargets {
    pub(crate) const fn new(
        color_format: wgpu::TextureFormat,
        edl_color_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        Self {
            color_format,
            edl_color_format,
            current: None,
        }
    }

    pub(crate) fn single_sample_depth(
        &mut self,
        device: &wgpu::Device,
        viewport: Viewport,
    ) -> &DepthTarget {
        self.for_viewport(viewport)
            .single_sample_depth
            .get_or_insert_with(|| DepthTarget::single_sample(device, viewport, false))
    }

    pub(crate) fn multisample(
        &mut self,
        device: &wgpu::Device,
        viewport: Viewport,
    ) -> (&ColorTarget, &DepthTarget) {
        let color_format = self.color_format;
        let targets = self.for_viewport(viewport);
        let multisample = targets
            .multisample
            .get_or_insert_with(|| MultisampleTargets::new(device, viewport, color_format));
        (&multisample.color, &multisample.depth)
    }

    pub(crate) fn depth_and_pick(
        &mut self,
        device: &wgpu::Device,
        viewport: Viewport,
        separate_pick_depth: bool,
    ) -> (&DepthTarget, &PickTarget) {
        let targets = self.for_viewport(viewport);
        let pick = targets
            .pick
            .get_or_insert_with(|| PickTarget::new(device, viewport));
        let depth = if separate_pick_depth {
            targets
                .pick_depth
                .get_or_insert_with(|| DepthTarget::single_sample(device, viewport, false))
        } else {
            targets
                .single_sample_depth
                .get_or_insert_with(|| DepthTarget::single_sample(device, viewport, false))
        };
        (depth, pick)
    }

    pub(crate) fn eye_dome(
        &mut self,
        device: &wgpu::Device,
        viewport: Viewport,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
    ) -> (&DepthTarget, &ColorTarget, &wgpu::BindGroup) {
        let color_format = self
            .edl_color_format
            .expect("EDL targets are configured when the renderer enables EDL");
        let targets = self.for_viewport(viewport);
        let depth = targets
            .single_sample_depth
            .get_or_insert_with(|| DepthTarget::single_sample(device, viewport, true));
        let color = targets
            .edl_color
            .get_or_insert_with(|| ColorTarget::eye_dome(device, viewport, color_format));
        let bind_group = targets
            .edl_bind_group
            .get_or_insert_with(|| eye_dome_bind_group(device, layout, color, depth, uniform));
        (depth, color, bind_group)
    }

    pub(crate) fn multisample_eye_dome(
        &mut self,
        device: &wgpu::Device,
        viewport: Viewport,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
    ) -> (
        &ColorTarget,
        &DepthTarget,
        &DepthTarget,
        &ColorTarget,
        &wgpu::BindGroup,
    ) {
        let color_format = self
            .edl_color_format
            .expect("EDL targets are configured when the renderer enables EDL");
        let targets = self.for_viewport(viewport);
        let multisample = targets
            .multisample
            .get_or_insert_with(|| MultisampleTargets::new(device, viewport, color_format));
        let visibility_depth = targets
            .single_sample_depth
            .get_or_insert_with(|| DepthTarget::single_sample(device, viewport, true));
        let resolved_color = targets
            .edl_color
            .get_or_insert_with(|| ColorTarget::eye_dome(device, viewport, color_format));
        let bind_group = targets.edl_bind_group.get_or_insert_with(|| {
            eye_dome_bind_group(device, layout, resolved_color, visibility_depth, uniform)
        });
        (
            &multisample.color,
            &multisample.depth,
            visibility_depth,
            resolved_color,
            bind_group,
        )
    }

    pub(crate) fn transient_texture_bytes(&self) -> u64 {
        self.current
            .as_ref()
            .map_or(0, ViewportTargets::transient_texture_bytes)
    }

    fn for_viewport(&mut self, viewport: Viewport) -> &mut ViewportTargets {
        let matches = self
            .current
            .as_ref()
            .is_some_and(|targets| targets.viewport == viewport);
        if !matches {
            self.current = Some(ViewportTargets::new(viewport));
        }
        self.current
            .as_mut()
            .expect("viewport targets were initialized above")
    }
}

struct ViewportTargets {
    viewport: Viewport,
    single_sample_depth: Option<DepthTarget>,
    pick: Option<PickTarget>,
    pick_depth: Option<DepthTarget>,
    edl_color: Option<ColorTarget>,
    edl_bind_group: Option<wgpu::BindGroup>,
    multisample: Option<MultisampleTargets>,
}

impl ViewportTargets {
    const fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            single_sample_depth: None,
            pick: None,
            pick_depth: None,
            edl_color: None,
            edl_bind_group: None,
            multisample: None,
        }
    }

    fn transient_texture_bytes(&self) -> u64 {
        [
            self.single_sample_depth
                .as_ref()
                .map_or(0, DepthTarget::byte_size),
            self.pick.as_ref().map_or(0, PickTarget::byte_size),
            self.pick_depth.as_ref().map_or(0, DepthTarget::byte_size),
            self.edl_color.as_ref().map_or(0, ColorTarget::byte_size),
            self.multisample
                .as_ref()
                .map_or(0, MultisampleTargets::byte_size),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .expect("validated GPU target extents fit exact u64 accounting")
    }
}

struct MultisampleTargets {
    color: ColorTarget,
    depth: DepthTarget,
}

impl MultisampleTargets {
    fn new(device: &wgpu::Device, viewport: Viewport, color_format: wgpu::TextureFormat) -> Self {
        Self {
            color: ColorTarget::multisample(device, viewport, color_format),
            depth: DepthTarget::multisample(device, viewport),
        }
    }

    fn byte_size(&self) -> u64 {
        self.color
            .byte_size()
            .checked_add(self.depth.byte_size())
            .expect("validated GPU target extents fit exact u64 accounting")
    }
}

pub(crate) struct DepthTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    byte_size: u64,
}

impl DepthTarget {
    fn single_sample(device: &wgpu::Device, viewport: Viewport, sampleable: bool) -> Self {
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if sampleable {
            usage |= wgpu::TextureUsages::TEXTURE_BINDING;
        }
        Self::new(device, viewport, "punctra depth texture", usage, 1)
    }

    fn multisample(device: &wgpu::Device, viewport: Viewport) -> Self {
        Self::new(
            device,
            viewport,
            "punctra four-sample depth texture",
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            MULTISAMPLE_COUNT,
        )
    }

    fn new(
        device: &wgpu::Device,
        viewport: Viewport,
        label: &'static str,
        usage: wgpu::TextureUsages,
        sample_count: u32,
    ) -> Self {
        let (texture, view) =
            create_target(device, viewport, label, DEPTH_FORMAT, usage, sample_count);
        Self {
            _texture: texture,
            view,
            byte_size: texture_byte_size(viewport, DEPTH_FORMAT, sample_count),
        }
    }

    pub(crate) const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    const fn byte_size(&self) -> u64 {
        self.byte_size
    }
}

pub(crate) struct ColorTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    byte_size: u64,
}

impl ColorTarget {
    fn eye_dome(device: &wgpu::Device, viewport: Viewport, format: wgpu::TextureFormat) -> Self {
        Self::new(
            device,
            viewport,
            "punctra eye-dome color texture",
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            1,
        )
    }

    fn multisample(device: &wgpu::Device, viewport: Viewport, format: wgpu::TextureFormat) -> Self {
        Self::new(
            device,
            viewport,
            "punctra four-sample color texture",
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            MULTISAMPLE_COUNT,
        )
    }

    fn new(
        device: &wgpu::Device,
        viewport: Viewport,
        label: &'static str,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        sample_count: u32,
    ) -> Self {
        let (texture, view) = create_target(device, viewport, label, format, usage, sample_count);
        Self {
            _texture: texture,
            view,
            byte_size: texture_byte_size(viewport, format, sample_count),
        }
    }

    pub(crate) const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    const fn byte_size(&self) -> u64 {
        self.byte_size
    }
}

pub(crate) struct PickTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    byte_size: u64,
}

impl PickTarget {
    fn new(device: &wgpu::Device, viewport: Viewport) -> Self {
        let (texture, view) = create_target(
            device,
            viewport,
            "punctra pick texture",
            PICK_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            1,
        );
        Self {
            texture,
            view,
            byte_size: texture_byte_size(viewport, PICK_FORMAT, 1),
        }
    }

    pub(crate) const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    const fn byte_size(&self) -> u64 {
        self.byte_size
    }
}

fn eye_dome_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color: &ColorTarget,
    depth: &DepthTarget,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("punctra eye-dome bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(color.view()),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(depth.view()),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

fn create_target(
    device: &wgpu::Device,
    viewport: Viewport,
    label: &'static str,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    sample_count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: texture_extent(viewport),
        mip_level_count: 1,
        sample_count,
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

fn texture_byte_size(viewport: Viewport, format: wgpu::TextureFormat, sample_count: u32) -> u64 {
    let (block_width, block_height) = format.block_dimensions();
    let block_bytes = format
        .block_copy_size(None)
        .expect("renderer-owned target formats have exact texel-block sizes");
    u64::from(viewport.width().div_ceil(block_width))
        .checked_mul(u64::from(viewport.height().div_ceil(block_height)))
        .and_then(|blocks| blocks.checked_mul(u64::from(block_bytes)))
        .and_then(|single_sample_bytes| single_sample_bytes.checked_mul(u64::from(sample_count)))
        .expect("validated GPU target extents fit exact u64 accounting")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multisample_texture_accounting_includes_all_four_samples() {
        let viewport = Viewport::new(640, 480).unwrap();

        assert_eq!(
            texture_byte_size(viewport, wgpu::TextureFormat::Rgba8Unorm, 4),
            4_915_200
        );
        assert_eq!(
            texture_byte_size(viewport, wgpu::TextureFormat::Depth32Float, 4),
            4_915_200
        );
    }
}
