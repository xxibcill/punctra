use render_protocol::Viewport;

use crate::pipeline::{DEPTH_FORMAT, PICK_FORMAT};

pub(crate) const MULTISAMPLE_COUNT: u32 = 4;
pub(crate) const MAX_ANTIALIASED_PIXELS: u64 = 1_310_720;
pub(crate) const MAX_TRANSIENT_TEXTURE_BYTES: u64 = 67_108_864;

/// Requested color-edge treatment for rendered Points.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointFootprint {
    /// Uses the inherited one-sample hard circular footprint.
    #[default]
    SingleSample,
    /// Requests deterministic four-sample circular edge coverage.
    Antialiased,
}

/// Selected Point-footprint path for one physical viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointFootprintStatus {
    /// The caller requested the inherited one-sample path.
    SingleSample,
    /// Four-sample color and depth coverage is active.
    Multisample4x,
    /// Required four-sample target capabilities are unavailable.
    UnsupportedFallback,
    /// The viewport exceeds the bounded multisample resource envelope, so the
    /// frame uses the unenhanced one-sample path.
    ResourceFallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PointFootprintPlan {
    request: PointFootprint,
    multisample_supported: bool,
    antialiased_bytes_per_pixel: u64,
    single_sample_edl_and_pick_bytes_per_pixel: u64,
}

impl PointFootprintPlan {
    pub(crate) fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        request: PointFootprint,
        eye_dome_active: bool,
    ) -> Self {
        let color_features = color_format.guaranteed_format_features(device.features());
        let depth_features = DEPTH_FORMAT.guaranteed_format_features(device.features());
        Self {
            request,
            multisample_supported: supports_multisampling(color_features, depth_features),
            antialiased_bytes_per_pixel: antialiased_bytes_per_pixel(color_format, eye_dome_active),
            single_sample_edl_and_pick_bytes_per_pixel: single_sample_edl_and_pick_bytes_per_pixel(
                color_format,
            ),
        }
    }

    pub(crate) fn status(self, viewport: Viewport) -> PointFootprintStatus {
        if self.request == PointFootprint::SingleSample {
            return PointFootprintStatus::SingleSample;
        }
        if !self.multisample_supported {
            return PointFootprintStatus::UnsupportedFallback;
        }

        let pixels = viewport_pixels(viewport);
        let exceeds_pixel_limit = pixels > MAX_ANTIALIASED_PIXELS;
        let exceeds_byte_limit =
            !fits_transient_ceiling(viewport, self.antialiased_bytes_per_pixel);
        if exceeds_pixel_limit || exceeds_byte_limit {
            PointFootprintStatus::ResourceFallback
        } else {
            PointFootprintStatus::Multisample4x
        }
    }

    pub(crate) const fn creates_multisample_pipelines(self) -> bool {
        matches!(self.request, PointFootprint::Antialiased) && self.multisample_supported
    }

    pub(crate) fn allows_eye_dome(self, viewport: Viewport) -> bool {
        match self.status(viewport) {
            PointFootprintStatus::ResourceFallback => false,
            PointFootprintStatus::Multisample4x => true,
            PointFootprintStatus::SingleSample | PointFootprintStatus::UnsupportedFallback => {
                fits_transient_ceiling(viewport, self.single_sample_edl_and_pick_bytes_per_pixel)
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn forced_for_test(
        request: PointFootprint,
        multisample_supported: bool,
        antialiased_bytes_per_pixel: u64,
        single_sample_edl_and_pick_bytes_per_pixel: u64,
    ) -> Self {
        Self {
            request,
            multisample_supported,
            antialiased_bytes_per_pixel,
            single_sample_edl_and_pick_bytes_per_pixel,
        }
    }
}

fn viewport_pixels(viewport: Viewport) -> u64 {
    u64::from(viewport.width()) * u64::from(viewport.height())
}

fn fits_transient_ceiling(viewport: Viewport, bytes_per_pixel: u64) -> bool {
    viewport_pixels(viewport)
        .checked_mul(bytes_per_pixel)
        .is_some_and(|bytes| bytes <= MAX_TRANSIENT_TEXTURE_BYTES)
}

fn supports_multisampling(
    color: wgpu::TextureFormatFeatures,
    depth: wgpu::TextureFormatFeatures,
) -> bool {
    color
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        && color.flags.contains(
            wgpu::TextureFormatFeatureFlags::BLENDABLE
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        )
        && depth
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        && depth
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
}

fn antialiased_bytes_per_pixel(color_format: wgpu::TextureFormat, eye_dome_active: bool) -> u64 {
    let color_bytes = format_bytes_per_pixel(color_format);
    let depth_bytes = format_bytes_per_pixel(DEPTH_FORMAT);
    let pick_bytes = format_bytes_per_pixel(PICK_FORMAT);
    let multisample_bytes = (color_bytes + depth_bytes) * u64::from(MULTISAMPLE_COUNT);
    let pick_pair_bytes = pick_bytes + depth_bytes;
    let eye_dome_bytes = if eye_dome_active {
        color_bytes + depth_bytes
    } else {
        0
    };
    multisample_bytes + pick_pair_bytes + eye_dome_bytes
}

fn single_sample_edl_and_pick_bytes_per_pixel(color_format: wgpu::TextureFormat) -> u64 {
    format_bytes_per_pixel(color_format)
        + format_bytes_per_pixel(DEPTH_FORMAT)
        + format_bytes_per_pixel(PICK_FORMAT)
}

fn format_bytes_per_pixel(format: wgpu::TextureFormat) -> u64 {
    let (block_width, block_height) = format.block_dimensions();
    assert_eq!(block_width, 1, "render target formats are uncompressed");
    assert_eq!(block_height, 1, "render target formats are uncompressed");
    u64::from(
        format
            .block_copy_size(None)
            .expect("render target formats have exact texel sizes"),
    )
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf};

    use super::*;
    use crate::renderer::point_footprint_test_support::{TestFootprintPath, measure};

    const SMALL_VIEWPORT: Viewport = match Viewport::new(64, 64) {
        Ok(viewport) => viewport,
        Err(_) => panic!("the fixed viewport is valid"),
    };

    #[test]
    fn single_sample_request_never_becomes_a_fallback() {
        let plan = PointFootprintPlan {
            request: PointFootprint::SingleSample,
            multisample_supported: false,
            antialiased_bytes_per_pixel: 48,
            single_sample_edl_and_pick_bytes_per_pixel: 12,
        };

        let status = plan.status(SMALL_VIEWPORT);
        let creates_multisample_pipelines = plan.creates_multisample_pipelines();
        assert_eq!(status, PointFootprintStatus::SingleSample);
        assert!(!creates_multisample_pipelines);
        emit_selection_evidence_if_requested(
            selection_evidence_facts(
                plan.request,
                status,
                creates_multisample_pipelines,
                SMALL_VIEWPORT,
            ),
            Some(TestFootprintPath::SingleSample),
        );
    }

    #[test]
    fn capability_fallback_precedes_the_viewport_resource_check() {
        let plan = PointFootprintPlan {
            request: PointFootprint::Antialiased,
            multisample_supported: false,
            antialiased_bytes_per_pixel: 48,
            single_sample_edl_and_pick_bytes_per_pixel: 12,
        };
        let oversized = Viewport::new(4_096, 2_048).unwrap();

        let status = plan.status(oversized);
        let creates_multisample_pipelines = plan.creates_multisample_pipelines();
        assert_eq!(status, PointFootprintStatus::UnsupportedFallback);
        assert!(!creates_multisample_pipelines);
        emit_selection_evidence_if_requested(
            selection_evidence_facts(
                plan.request,
                status,
                creates_multisample_pipelines,
                oversized,
            ),
            Some(TestFootprintPath::UnsupportedFallback),
        );
    }

    #[test]
    fn preferred_path_accepts_the_exact_pixel_ceiling() {
        let plan = PointFootprintPlan {
            request: PointFootprint::Antialiased,
            multisample_supported: true,
            antialiased_bytes_per_pixel: 48,
            single_sample_edl_and_pick_bytes_per_pixel: 12,
        };

        assert_eq!(
            plan.status(Viewport::new(1_280, 1_024).unwrap()),
            PointFootprintStatus::Multisample4x
        );
        assert_eq!(
            plan.status(Viewport::new(1_281, 1_024).unwrap()),
            PointFootprintStatus::ResourceFallback
        );
        assert!(plan.creates_multisample_pipelines());
    }

    #[test]
    fn single_sample_eye_dome_stays_within_renderer_ceiling() {
        let plan = PointFootprintPlan {
            request: PointFootprint::SingleSample,
            multisample_supported: true,
            antialiased_bytes_per_pixel: 48,
            single_sample_edl_and_pick_bytes_per_pixel: 12,
        };
        let largest_bounded_viewport = Viewport::new(4_096, 1_365).unwrap();
        let first_unbounded_viewport = Viewport::new(4_096, 1_366).unwrap();
        let maximum_viewport = Viewport::new(4_096, 2_048).unwrap();

        for viewport in [
            largest_bounded_viewport,
            first_unbounded_viewport,
            maximum_viewport,
        ] {
            assert_eq!(plan.status(viewport), PointFootprintStatus::SingleSample);
        }
        assert!(plan.allows_eye_dome(largest_bounded_viewport));
        assert!(!plan.allows_eye_dome(first_unbounded_viewport));
        assert!(!plan.allows_eye_dome(maximum_viewport));
        assert_eq!(
            u64::from(maximum_viewport.width()) * u64::from(maximum_viewport.height()) * 8,
            MAX_TRANSIENT_TEXTURE_BYTES
        );
    }

    #[test]
    fn exact_high_water_accounts_for_pick_and_eye_dome_targets() {
        let preferred_non_edl_bytes_per_pixel =
            antialiased_bytes_per_pixel(wgpu::TextureFormat::Rgba8Unorm, false);
        let preferred_edl_bytes_per_pixel =
            antialiased_bytes_per_pixel(wgpu::TextureFormat::Rgba8Unorm, true);
        let fallback_bytes_per_pixel =
            format_bytes_per_pixel(DEPTH_FORMAT) + format_bytes_per_pixel(PICK_FORMAT);
        let maximum_preferred_transient_bytes =
            MAX_ANTIALIASED_PIXELS * preferred_edl_bytes_per_pixel;

        assert_eq!(preferred_non_edl_bytes_per_pixel, 40);
        assert_eq!(preferred_edl_bytes_per_pixel, 48);
        assert_eq!(fallback_bytes_per_pixel, 8);
        assert_eq!(maximum_preferred_transient_bytes, 62_914_560);
        assert_eq!(MAX_TRANSIENT_TEXTURE_BYTES, 67_108_864);
        emit_selection_evidence_if_requested(
            serde_json::json!({
                "transient_bounds": {
                    "preferred_non_edl_bytes_per_pixel": preferred_non_edl_bytes_per_pixel,
                    "preferred_edl_bytes_per_pixel": preferred_edl_bytes_per_pixel,
                    "fallback_bytes_per_pixel": fallback_bytes_per_pixel,
                    "maximum_preferred_physical_pixels": MAX_ANTIALIASED_PIXELS,
                    "maximum_preferred_transient_bytes": maximum_preferred_transient_bytes,
                    "renderer_transient_byte_ceiling": MAX_TRANSIENT_TEXTURE_BYTES,
                },
            }),
            None,
        );
    }

    fn selection_evidence_facts(
        request: PointFootprint,
        status: PointFootprintStatus,
        creates_multisample_pipelines: bool,
        viewport: Viewport,
    ) -> serde_json::Value {
        serde_json::json!({
            "selection": {
                "requested": point_footprint_name(request),
                "selected": point_footprint_status_name(status),
                "sample_count": if status == PointFootprintStatus::Multisample4x { 4 } else { 1 },
                "multisample_pipeline_created": creates_multisample_pipelines,
            },
            "physical_width": viewport.width(),
            "physical_height": viewport.height(),
            "resources": null,
            "pick_probes": null,
        })
    }

    fn emit_selection_evidence_if_requested(
        mut facts: serde_json::Value,
        gpu_path: Option<TestFootprintPath>,
    ) {
        let Some(path) = env::var_os("PUNCTRA_PRIVATE_POINT_FOOTPRINT_FACTS_PATH") else {
            return;
        };
        let mut environment = None;
        if let Some(gpu_path) = gpu_path {
            let proof = measure(gpu_path);
            facts
                .as_object_mut()
                .expect("private Point-footprint facts are an object")
                .extend(
                    proof
                        .facts
                        .as_object()
                        .expect("private Point-footprint proof is an object")
                        .clone(),
                );
            environment = Some(proof.environment);
        }
        let path = PathBuf::from(path);
        let output = environment.map_or(facts.clone(), |environment| {
            serde_json::json!({
                "environment": environment,
                "facts": facts,
            })
        });
        let mut bytes = serde_json::to_vec_pretty(&output)
            .expect("the bounded private Point-footprint facts should serialize");
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap_or_else(|error| {
            panic!(
                "failed to write private Point-footprint facts to {}: {error}",
                path.display()
            )
        });
    }

    const fn point_footprint_name(request: PointFootprint) -> &'static str {
        match request {
            PointFootprint::SingleSample => "single_sample",
            PointFootprint::Antialiased => "antialiased",
        }
    }

    const fn point_footprint_status_name(status: PointFootprintStatus) -> &'static str {
        match status {
            PointFootprintStatus::SingleSample => "single_sample",
            PointFootprintStatus::Multisample4x => "multisample4x",
            PointFootprintStatus::UnsupportedFallback => "unsupported_fallback",
            PointFootprintStatus::ResourceFallback => "resource_fallback",
        }
    }

    #[test]
    fn capability_selection_requires_blending_resolve_and_depth_multisampling() {
        let usages = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let supported_color = wgpu::TextureFormatFeatures {
            allowed_usages: usages,
            flags: wgpu::TextureFormatFeatureFlags::BLENDABLE
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        };
        let supported_depth = wgpu::TextureFormatFeatures {
            allowed_usages: usages,
            flags: wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4,
        };

        assert!(supports_multisampling(supported_color, supported_depth));
        assert!(!supports_multisampling(
            wgpu::TextureFormatFeatures {
                flags: wgpu::TextureFormatFeatureFlags::BLENDABLE
                    | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4,
                ..supported_color
            },
            supported_depth
        ));
        assert!(!supports_multisampling(
            wgpu::TextureFormatFeatures {
                flags: wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                    | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
                ..supported_color
            },
            supported_depth
        ));
        assert!(!supports_multisampling(
            supported_color,
            wgpu::TextureFormatFeatures {
                flags: wgpu::TextureFormatFeatureFlags::empty(),
                ..supported_depth
            }
        ));
    }
}
