use render_protocol::RenderLimits;
#[cfg(target_arch = "wasm32")]
use render_wgpu::{FrameReport, PickHit};
use serde::Serialize;

use crate::{
    host::{
        MAX_CANVAS_DIMENSION, MAX_CANVAS_PIXELS, MAX_DEVICE_PIXEL_RATIO,
        MAX_RENDER_TRANSIENT_BYTES, PRESENTATION_LATENCY_FRAMES, PhysicalViewport,
        SURFACE_BYTES_PER_PIXEL, ViewerPhase,
    },
    scene::SceneFacts,
};

#[derive(Serialize)]
pub(crate) struct Diagnostics<'a> {
    pub(crate) schema: &'static str,
    pub(crate) package_version: &'static str,
    pub(crate) phase: ViewerPhase,
    pub(crate) rendered_frames: u64,
    pub(crate) hidden_frame_skips: u64,
    pub(crate) capabilities: &'a CapabilityFacts,
    pub(crate) limits: LimitFacts,
    pub(crate) viewport: PhysicalViewport,
    pub(crate) scene: SceneFacts,
    pub(crate) frame: Option<FrameFacts>,
    pub(crate) pick: &'a PickFacts,
    pub(crate) display_authority: &'static str,
    pub(crate) safe_shutdown_action: &'static str,
}

impl Diagnostics<'_> {
    pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[derive(Serialize)]
pub(crate) struct CapabilityFacts {
    secure_context: bool,
    webgpu: bool,
    browser_user_agent: String,
    browser_platform: String,
    adapter_name: String,
    backend: String,
    device_type: String,
    surface_format: String,
    present_mode: &'static str,
    required_feature_count: u64,
    adapter_max_buffer_size: u64,
    adapter_max_texture_dimension_2d: u32,
}

impl CapabilityFacts {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(
        adapter: &wgpu::AdapterInfo,
        limits: &wgpu::Limits,
        surface: &wgpu::SurfaceConfiguration,
        browser_user_agent: String,
        browser_platform: String,
    ) -> Self {
        let adapter_name = if adapter.name.is_empty() {
            "browser WebGPU adapter".to_owned()
        } else {
            adapter.name.clone()
        };
        Self {
            secure_context: true,
            webgpu: true,
            browser_user_agent,
            browser_platform,
            adapter_name,
            backend: format!("{:?}", adapter.backend),
            device_type: format!("{:?}", adapter.device_type),
            surface_format: format!("{:?}", surface.format),
            present_mode: "fifo",
            required_feature_count: 0,
            adapter_max_buffer_size: limits.max_buffer_size,
            adapter_max_texture_dimension_2d: limits.max_texture_dimension_2d,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct LimitFacts {
    estimated_gpu_bytes: u64,
    points: u64,
    batches: u64,
    highlight_points: u64,
    canvas_dimension: u32,
    canvas_pixels: u64,
    device_pixel_ratio: f64,
    surface_bytes_per_pixel: u64,
    renderer_transient_bytes: u64,
    presentation_latency_frames: u32,
}

impl LimitFacts {
    pub(crate) const fn new(render: RenderLimits) -> Self {
        Self {
            estimated_gpu_bytes: render.max_estimated_gpu_bytes(),
            points: render.max_points(),
            batches: render.max_batches(),
            highlight_points: render.max_highlight_points(),
            canvas_dimension: MAX_CANVAS_DIMENSION,
            canvas_pixels: MAX_CANVAS_PIXELS,
            device_pixel_ratio: MAX_DEVICE_PIXEL_RATIO,
            surface_bytes_per_pixel: SURFACE_BYTES_PER_PIXEL,
            renderer_transient_bytes: MAX_RENDER_TRANSIENT_BYTES,
            presentation_latency_frames: PRESENTATION_LATENCY_FRAMES,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct FrameFacts {
    view_generation: u64,
    drawn_points: u64,
    draw_calls: u64,
    resident_bytes: u64,
    transient_texture_bytes: u64,
    surface_suboptimal: bool,
}

impl FrameFacts {
    #[cfg(target_arch = "wasm32")]
    pub(crate) const fn from_report(report: FrameReport, surface_suboptimal: bool) -> Self {
        Self {
            view_generation: report.view_generation().generation(),
            drawn_points: report.drawn_points(),
            draw_calls: report.draw_calls(),
            resident_bytes: report.resident_bytes(),
            transient_texture_bytes: report.transient_texture_bytes(),
            surface_suboptimal,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PickStatus {
    NotRequested,
    Pending,
    Miss,
    Hit,
}

#[derive(Serialize)]
pub(crate) struct PickFacts {
    status: PickStatus,
    authority: &'static str,
    generation: Option<u64>,
    batch_key: Option<u64>,
    batch_version: Option<u64>,
    point_ordinal: Option<u64>,
}

impl PickFacts {
    pub(crate) const fn not_requested() -> Self {
        Self::empty(PickStatus::NotRequested)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) const fn pending() -> Self {
        Self::empty(PickStatus::Pending)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) const fn miss() -> Self {
        Self::empty(PickStatus::Miss)
    }

    const fn empty(status: PickStatus) -> Self {
        Self {
            status,
            authority: "provisional_gpu_hint",
            generation: None,
            batch_key: None,
            batch_version: None,
            point_ordinal: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) const fn hit(hit: PickHit) -> Self {
        Self {
            status: PickStatus::Hit,
            authority: "provisional_gpu_hint",
            generation: Some(hit.view_generation().generation()),
            batch_key: Some(hit.batch().get()),
            batch_version: Some(hit.version().get()),
            point_ordinal: Some(hit.point().ordinal()),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct Failure {
    schema: &'static str,
    code: &'static str,
    message: String,
    safe_action: &'static str,
}

impl Failure {
    pub(crate) fn new(
        code: &'static str,
        message: impl std::fmt::Display,
        safe_action: &'static str,
    ) -> Self {
        Self {
            schema: "punctra-browser-failure-v1",
            code,
            message: message.to_string(),
            safe_action,
        }
    }

    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                "{{\"schema\":\"punctra-browser-failure-v1\",\"code\":\"{}\",\"message\":\"browser failure\",\"safe_action\":\"{}\"}}",
                self.code, self.safe_action
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{host::Lifecycle, scene};

    #[test]
    fn diagnostics_preserve_schema_authority_and_bounded_resource_facts() {
        let scene = scene::PreparedScene::new().unwrap();
        let viewport = PhysicalViewport::from_css(800.0, 500.0, 2.0).unwrap();
        let capabilities = capability_fixture();
        let pick = PickFacts::not_requested();
        let lifecycle = Lifecycle::ready();
        let diagnostics = Diagnostics {
            schema: "punctra-browser-foundation-v1",
            package_version: env!("CARGO_PKG_VERSION"),
            phase: lifecycle.phase(),
            rendered_frames: lifecycle.rendered_frames(),
            hidden_frame_skips: lifecycle.hidden_frame_skips(),
            capabilities: &capabilities,
            limits: LimitFacts::new(scene::render_limits()),
            viewport,
            scene: scene.facts(),
            frame: None,
            pick: &pick,
            display_authority: "progressive_gpu_non_authoritative",
            safe_shutdown_action: "recreate",
        };

        let value: serde_json::Value =
            serde_json::from_str(&diagnostics.to_json().unwrap()).unwrap();
        assert_eq!(value["schema"], "punctra-browser-foundation-v1");
        assert_eq!(
            value["display_authority"],
            "progressive_gpu_non_authoritative"
        );
        assert_eq!(value["scene"]["point_count"], 1_089);
        assert_eq!(value["limits"]["points"], 2_048);
        assert_eq!(value["limits"]["estimated_gpu_bytes"], 49_152);
        assert_eq!(value["limits"]["surface_bytes_per_pixel"], 4);
        assert_eq!(value["limits"]["presentation_latency_frames"], 2);
        assert_eq!(value["viewport"]["surface_bytes"], 6_400_000);
        assert_eq!(value["pick"]["status"], "not_requested");
    }

    #[test]
    fn failure_preserves_schema_code_message_and_single_safe_action() {
        let failure = Failure::new("device_lost", "adapter reset", "recreate the viewer");
        let value: serde_json::Value = serde_json::from_str(&failure.to_json()).unwrap();

        assert_eq!(
            value,
            json!({
                "schema": "punctra-browser-failure-v1",
                "code": "device_lost",
                "message": "adapter reset",
                "safe_action": "recreate the viewer",
            })
        );
    }

    #[test]
    fn pick_status_serialization_is_closed_and_stable() {
        assert_eq!(
            serde_json::to_value([
                PickStatus::NotRequested,
                PickStatus::Pending,
                PickStatus::Miss,
                PickStatus::Hit,
            ])
            .unwrap(),
            json!(["not_requested", "pending", "miss", "hit"])
        );
    }

    fn capability_fixture() -> CapabilityFacts {
        CapabilityFacts {
            secure_context: true,
            webgpu: true,
            browser_user_agent: "test browser".to_owned(),
            browser_platform: "test platform".to_owned(),
            adapter_name: "test adapter".to_owned(),
            backend: "BrowserWebGpu".to_owned(),
            device_type: "Other".to_owned(),
            surface_format: "Bgra8Unorm".to_owned(),
            present_mode: "fifo",
            required_feature_count: 0,
            adapter_max_buffer_size: 268_435_456,
            adapter_max_texture_dimension_2d: 8_192,
        }
    }
}
