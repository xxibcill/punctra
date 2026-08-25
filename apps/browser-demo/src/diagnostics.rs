use render_protocol::{Camera, CameraProjection, RenderLimits, SourceId, ViewGenerationKey};
#[cfg(target_arch = "wasm32")]
use render_wgpu::{FrameReport, PickHit};
use serde::Serialize;

#[cfg(test)]
use crate::host::CssViewportRequest;
use crate::{
    display::DisplayMode,
    host::{
        MAX_CANVAS_DIMENSION, MAX_CANVAS_PIXELS, MAX_DEVICE_PIXEL_RATIO,
        MAX_RENDER_TRANSIENT_BYTES, PRESENTATION_LATENCY_FRAMES, PhysicalViewport,
        SURFACE_BYTES_PER_PIXEL, ViewerPhase,
    },
    scene::SceneFacts,
    streaming::{StreamFacts, StreamingLimitFacts},
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
    pub(crate) streaming: StreamFacts,
    pub(crate) streaming_limits: StreamingLimitFacts,
    pub(crate) frame: Option<FrameFacts>,
    pub(crate) pick: &'a PickFacts,
    pub(crate) camera: CameraFacts,
    pub(crate) display_mode: DisplayMode,
    pub(crate) highlights: HighlightFacts,
    pub(crate) display_authority: &'static str,
    pub(crate) safe_shutdown_action: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct CameraFacts {
    eye: [f64; 3],
    target: [f64; 3],
    up: [f64; 3],
    projection: &'static str,
    vertical_field_of_view_radians: Option<f32>,
    vertical_world_height: Option<f64>,
    near_distance: f32,
    far_distance: f32,
}

impl CameraFacts {
    pub(crate) const fn from_camera(camera: Camera) -> Self {
        let (projection, vertical_field_of_view_radians, vertical_world_height) =
            match camera.projection() {
                CameraProjection::Perspective {
                    vertical_field_of_view_radians,
                } => ("perspective", Some(vertical_field_of_view_radians), None),
                CameraProjection::Orthographic {
                    vertical_world_height,
                } => ("orthographic", None, Some(vertical_world_height)),
            };
        Self {
            eye: camera.eye(),
            target: camera.target(),
            up: camera.up(),
            projection,
            vertical_field_of_view_radians,
            vertical_world_height,
            near_distance: camera.near_distance(),
            far_distance: camera.far_distance(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct HighlightFacts {
    generation: Option<u64>,
    #[serde(serialize_with = "serialize_source_identity")]
    source_identity: Option<SourceId>,
    point_count: u64,
    authority: &'static str,
}

impl HighlightFacts {
    pub(crate) const fn empty() -> Self {
        Self {
            generation: None,
            source_identity: None,
            point_count: 0,
            authority: "presentation_only",
        }
    }

    pub(crate) fn complete(
        view_generation: ViewGenerationKey,
        source_identity: SourceId,
        point_count: usize,
    ) -> Result<Self, std::num::TryFromIntError> {
        Ok(Self {
            generation: Some(view_generation.generation()),
            source_identity: Some(source_identity),
            point_count: u64::try_from(point_count)?,
            authority: "presentation_only",
        })
    }
}

#[allow(clippy::ref_option)] // serde's serialize_with contract passes a field by reference.
pub(crate) fn serialize_source_identity<S>(
    source: &Option<SourceId>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match source {
        Some(source) => serializer.serialize_some(&source.to_string()),
        None => serializer.serialize_none(),
    }
}

pub(crate) fn serialize_required_source_identity<S>(
    source: &SourceId,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&source.to_string())
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
    composite_alpha_mode: String,
    present_mode: &'static str,
    surface_format_support: SurfaceFormatSupport,
    required_feature_count: u64,
    adapter_max_buffer_size: u64,
    adapter_max_texture_dimension_2d: u32,
    adapter_max_bind_groups: u32,
    adapter_max_vertex_buffers: u32,
    adapter_max_color_attachments: u32,
}

#[derive(Serialize)]
struct SurfaceFormatSupport {
    render_attachment: bool,
    blendable: bool,
}

impl CapabilityFacts {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(
        adapter: &wgpu::AdapterInfo,
        limits: &wgpu::Limits,
        surface_capabilities: &wgpu::SurfaceCapabilities,
        surface: &wgpu::SurfaceConfiguration,
        browser_user_agent: String,
        browser_platform: String,
    ) -> Self {
        let adapter_name = if adapter.name.is_empty() {
            "browser WebGPU adapter".to_owned()
        } else {
            adapter.name.clone()
        };
        let format_features = surface
            .format
            .guaranteed_format_features(wgpu::Features::empty());
        Self {
            secure_context: true,
            webgpu: true,
            browser_user_agent,
            browser_platform,
            adapter_name,
            backend: format!("{:?}", adapter.backend),
            device_type: format!("{:?}", adapter.device_type),
            surface_format: format!("{:?}", surface.format),
            composite_alpha_mode: format!("{:?}", surface.alpha_mode),
            present_mode: "fifo",
            surface_format_support: SurfaceFormatSupport {
                render_attachment: surface_capabilities
                    .usages
                    .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                    && format_features
                        .allowed_usages
                        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
                blendable: format_features
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE),
            },
            required_feature_count: 0,
            adapter_max_buffer_size: limits.max_buffer_size,
            adapter_max_texture_dimension_2d: limits.max_texture_dimension_2d,
            adapter_max_bind_groups: limits.max_bind_groups,
            adapter_max_vertex_buffers: limits.max_vertex_buffers,
            adapter_max_color_attachments: limits.max_color_attachments,
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

    pub(crate) const fn record_pick_transient_bytes(&mut self, transient_texture_bytes: u64) {
        self.transient_texture_bytes = transient_texture_bytes;
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
    source_identity: Option<String>,
    point_ordinal: Option<String>,
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
            source_identity: None,
            point_ordinal: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn hit(hit: PickHit) -> Self {
        Self {
            status: PickStatus::Hit,
            authority: "provisional_gpu_hint",
            generation: Some(hit.view_generation().generation()),
            batch_key: Some(hit.batch().get()),
            batch_version: Some(hit.version().get()),
            source_identity: Some(hit.point().source().to_string()),
            point_ordinal: Some(hit.point().ordinal().to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureCode {
    CapabilityInspection,
    CanvasSurface,
    DeviceLost,
    DevicePoll,
    DiagnosticSerialization,
    FrameRecording,
    FrameValidation,
    CameraValidation,
    HostModel,
    InitialViewport,
    InsecureContext,
    MissingRecordedFrame,
    MissingWindow,
    PickInvariant,
    PickNotRequested,
    PickOutsideViewport,
    PickPending,
    PickReadback,
    PickRecording,
    HighlightValidation,
    PresentationMode,
    RendererCapability,
    ResizeViewport,
    ScenePlanning,
    ScenePublication,
    SceneValidation,
    StreamPublication,
    StreamValidation,
    StaleGeneration,
    DisplayMode,
    SurfaceAlphaMode,
    SurfaceConfiguration,
    SurfaceFormat,
    SurfaceLost,
    SurfaceOccluded,
    SurfaceOutdated,
    SurfaceReconfiguration,
    SurfaceTimeout,
    SurfaceValidation,
    TransientTextureLimit,
    ViewerHidden,
    ViewportValidation,
    #[serde(rename = "webgpu_adapter")]
    WebGpuAdapter,
    #[serde(rename = "webgpu_device")]
    WebGpuDevice,
    #[serde(rename = "webgpu_unavailable")]
    WebGpuUnavailable,
}

#[derive(Serialize)]
pub(crate) struct Failure {
    schema: &'static str,
    code: FailureCode,
    message: String,
    safe_action: &'static str,
}

impl Failure {
    pub(crate) fn new(
        code: FailureCode,
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
                "{{\"schema\":\"punctra-browser-failure-v1\",\"code\":\"diagnostic_serialization\",\"message\":\"browser failure\",\"safe_action\":\"{}\"}}",
                self.safe_action
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        host::{HostModelError, Lifecycle, RESIZE_VIEWPORT_ACTION},
        scene,
        streaming::{StreamingLimitFacts, StreamingScene},
    };

    #[test]
    fn diagnostics_preserve_schema_authority_and_bounded_resource_facts() {
        let scene = scene::PreparedScene::new().unwrap();
        let viewport =
            PhysicalViewport::from_css(CssViewportRequest::new(800.0, 500.0, 2.0)).unwrap();
        let capabilities = capability_fixture();
        let pick = PickFacts::not_requested();
        let lifecycle = Lifecycle::ready();
        let diagnostics = Diagnostics {
            schema: "punctra-browser-streaming-v1",
            package_version: env!("CARGO_PKG_VERSION"),
            phase: lifecycle.phase(),
            rendered_frames: lifecycle.rendered_frames(),
            hidden_frame_skips: lifecycle.hidden_frame_skips(),
            capabilities: &capabilities,
            limits: LimitFacts::new(scene::render_limits()),
            viewport,
            scene: scene.facts(),
            streaming: StreamingScene::idle().facts(),
            streaming_limits: StreamingLimitFacts::fixed(),
            frame: None,
            pick: &pick,
            camera: CameraFacts::from_camera(scene.camera()),
            display_mode: DisplayMode::Rgb,
            highlights: HighlightFacts::empty(),
            display_authority: "progressive_gpu_non_authoritative",
            safe_shutdown_action: "recreate",
        };

        let value: serde_json::Value =
            serde_json::from_str(&diagnostics.to_json().unwrap()).unwrap();
        assert_eq!(value["schema"], "punctra-browser-streaming-v1");
        assert_eq!(
            value["display_authority"],
            "progressive_gpu_non_authoritative"
        );
        assert_eq!(value["scene"]["point_count"], 1_089);
        assert_eq!(value["scene"]["source_identity"], "15".repeat(32));
        assert_eq!(value["limits"]["points"], 8_192);
        assert_eq!(value["limits"]["estimated_gpu_bytes"], 196_608);
        assert_eq!(value["limits"]["surface_bytes_per_pixel"], 4);
        assert_eq!(value["limits"]["presentation_latency_frames"], 2);
        assert_eq!(value["viewport"]["surface_bytes"], 6_400_000);
        assert_eq!(value["pick"]["status"], "not_requested");
        assert_eq!(value["streaming"]["phase"], "idle");
        assert_eq!(value["streaming_limits"]["range_bytes"], 262_144);
        assert_eq!(value["capabilities"]["composite_alpha_mode"], "Opaque");
        assert_eq!(
            value["capabilities"]["surface_format_support"],
            json!({ "render_attachment": true, "blendable": true })
        );
        assert_eq!(value["capabilities"]["adapter_max_bind_groups"], 4);
        assert_eq!(value["capabilities"]["adapter_max_vertex_buffers"], 8);
        assert_eq!(value["capabilities"]["adapter_max_color_attachments"], 8);
    }

    #[test]
    fn failure_preserves_schema_code_message_and_single_safe_action() {
        let failure = Failure::new(
            FailureCode::DeviceLost,
            "adapter reset",
            "recreate the viewer",
        );
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
    fn resize_failure_preserves_retry_without_recreation_contract() {
        let failure = Failure::new(
            FailureCode::ResizeViewport,
            HostModelError::DevicePixelRatioLimit,
            RESIZE_VIEWPORT_ACTION,
        );
        let value: serde_json::Value = serde_json::from_str(&failure.to_json()).unwrap();

        assert_eq!(
            value,
            json!({
                "schema": "punctra-browser-failure-v1",
                "code": "resize_viewport",
                "message": "device-pixel ratio exceeds the accepted maximum of 4",
                "safe_action": "Keep the current surface configuration, choose finite positive CSS dimensions and a device-pixel ratio at most four so the physical canvas remains within 4,096 pixels per dimension and 8,388,608 pixels total, then resize again.",
            })
        );
    }

    #[test]
    fn failure_codes_preserve_the_existing_json_vocabulary() {
        assert_eq!(
            serde_json::to_value(
                &[
                    FailureCode::CapabilityInspection,
                    FailureCode::CanvasSurface,
                    FailureCode::DeviceLost,
                    FailureCode::DevicePoll,
                    FailureCode::DiagnosticSerialization,
                    FailureCode::FrameRecording,
                    FailureCode::FrameValidation,
                    FailureCode::CameraValidation,
                    FailureCode::HostModel,
                    FailureCode::InitialViewport,
                    FailureCode::InsecureContext,
                    FailureCode::MissingRecordedFrame,
                    FailureCode::MissingWindow,
                    FailureCode::PickInvariant,
                    FailureCode::PickNotRequested,
                    FailureCode::PickOutsideViewport,
                    FailureCode::PickPending,
                    FailureCode::PickReadback,
                    FailureCode::PickRecording,
                    FailureCode::HighlightValidation,
                    FailureCode::PresentationMode,
                    FailureCode::RendererCapability,
                    FailureCode::ResizeViewport,
                    FailureCode::ScenePlanning,
                    FailureCode::ScenePublication,
                    FailureCode::SceneValidation,
                    FailureCode::StreamPublication,
                    FailureCode::StreamValidation,
                    FailureCode::StaleGeneration,
                    FailureCode::DisplayMode,
                    FailureCode::SurfaceAlphaMode,
                    FailureCode::SurfaceConfiguration,
                    FailureCode::SurfaceFormat,
                    FailureCode::SurfaceLost,
                    FailureCode::SurfaceOccluded,
                    FailureCode::SurfaceOutdated,
                    FailureCode::SurfaceReconfiguration,
                    FailureCode::SurfaceTimeout,
                    FailureCode::SurfaceValidation,
                    FailureCode::TransientTextureLimit,
                    FailureCode::ViewerHidden,
                    FailureCode::ViewportValidation,
                    FailureCode::WebGpuAdapter,
                    FailureCode::WebGpuDevice,
                    FailureCode::WebGpuUnavailable,
                ][..]
            )
            .unwrap(),
            json!([
                "capability_inspection",
                "canvas_surface",
                "device_lost",
                "device_poll",
                "diagnostic_serialization",
                "frame_recording",
                "frame_validation",
                "camera_validation",
                "host_model",
                "initial_viewport",
                "insecure_context",
                "missing_recorded_frame",
                "missing_window",
                "pick_invariant",
                "pick_not_requested",
                "pick_outside_viewport",
                "pick_pending",
                "pick_readback",
                "pick_recording",
                "highlight_validation",
                "presentation_mode",
                "renderer_capability",
                "resize_viewport",
                "scene_planning",
                "scene_publication",
                "scene_validation",
                "stream_publication",
                "stream_validation",
                "stale_generation",
                "display_mode",
                "surface_alpha_mode",
                "surface_configuration",
                "surface_format",
                "surface_lost",
                "surface_occluded",
                "surface_outdated",
                "surface_reconfiguration",
                "surface_timeout",
                "surface_validation",
                "transient_texture_limit",
                "viewer_hidden",
                "viewport_validation",
                "webgpu_adapter",
                "webgpu_device",
                "webgpu_unavailable"
            ])
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

    #[test]
    fn frame_facts_refresh_exact_transient_bytes_after_pick_allocation() {
        let mut frame = FrameFacts {
            view_generation: 1,
            drawn_points: 1_089,
            draw_calls: 1,
            resident_bytes: 26_136,
            transient_texture_bytes: 7_646_628,
            surface_suboptimal: false,
        };

        frame.record_pick_transient_bytes(15_293_256);

        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["transient_texture_bytes"], 15_293_256);
    }

    #[test]
    fn highlight_facts_bind_one_generation_and_source() {
        let source = SourceId::new([0x17; 32]);
        let generation = ViewGenerationKey::new(render_protocol::ViewId::new(17), 3);
        let value =
            serde_json::to_value(HighlightFacts::complete(generation, source, 2).unwrap()).unwrap();

        assert_eq!(value["generation"], 3);
        assert_eq!(value["source_identity"], "17".repeat(32));
        assert_eq!(value["point_count"], 2);
        assert_eq!(value["authority"], "presentation_only");
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
            composite_alpha_mode: "Opaque".to_owned(),
            present_mode: "fifo",
            surface_format_support: SurfaceFormatSupport {
                render_attachment: true,
                blendable: true,
            },
            required_feature_count: 0,
            adapter_max_buffer_size: 268_435_456,
            adapter_max_texture_dimension_2d: 8_192,
            adapter_max_bind_groups: 4,
            adapter_max_vertex_buffers: 8,
            adapter_max_color_attachments: 8,
        }
    }
}
