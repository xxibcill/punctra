use bytemuck::{Pod, Zeroable};

pub(crate) const HIGHLIGHTED_FLAG: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct CameraUniform {
    pub(crate) view_projection: [[f32; 4]; 4],
    pub(crate) viewport_size: [f32; 2],
    pub(crate) default_point_size: f32,
    pub(crate) _padding: f32,
    pub(crate) highlight_color: [f32; 3],
    pub(crate) _highlight_padding: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct BatchUniform {
    pub(crate) origin_from_camera: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuPoint {
    pub(crate) position: [f32; 3],
    pub(crate) color: [u8; 4],
    pub(crate) flags: u32,
    pub(crate) pick_token: u32,
}

impl GpuPoint {
    pub(crate) const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Unorm8x4,
        2 => Uint32,
        3 => Uint32
    ];

    pub(crate) fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }

    pub(crate) fn set_highlighted(&mut self, highlighted: bool) {
        if highlighted {
            self.flags |= HIGHLIGHTED_FLAG;
        } else {
            self.flags &= !HIGHLIGHTED_FLAG;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_point_layout_matches_the_protocol_residency_model() {
        assert_eq!(size_of::<GpuPoint>(), 24);
        assert_eq!(align_of::<GpuPoint>(), 4);
        assert_eq!(GpuPoint::layout().array_stride, 24);
    }

    #[test]
    fn pick_token_remains_available_to_the_shader() {
        let point = GpuPoint {
            position: [0.0; 3],
            color: [0; 4],
            flags: 0,
            pick_token: 42,
        };

        assert_eq!(point.pick_token, 42);
    }

    #[test]
    fn highlighting_preserves_unrelated_flags() {
        let mut point = GpuPoint {
            position: [0.0; 3],
            color: [0; 4],
            flags: 0b100,
            pick_token: 0,
        };

        point.set_highlighted(true);
        assert_eq!(point.flags, 0b101);

        point.set_highlighted(false);
        assert_eq!(point.flags, 0b100);
    }
}
