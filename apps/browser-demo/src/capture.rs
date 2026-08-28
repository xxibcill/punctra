//! Bounded, target-neutral browser frame-capture layout and canonicalization.

use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::diagnostics::PointFootprintFacts;
use crate::host::{MAX_CANVAS_DIMENSION, MAX_CANVAS_PIXELS};
use crate::streaming::VisualBatchFacts;

const CAPTURE_BYTES_PER_PIXEL: u32 = 4;

/// Monotonic callback observations for one completed private frame capture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct CaptureCompletionFacts {
    schema: &'static str,
    origin: &'static str,
    submitted_work_done_callback_milliseconds: f64,
    readback_mapping_callback_milliseconds: f64,
}

/// Target-neutral ownership for one pending GPU capture and its last completion.
#[derive(Debug)]
pub(crate) struct CaptureSlot<T> {
    pending: Option<T>,
    completion: Option<CaptureCompletionFacts>,
}

impl<T> CaptureSlot<T> {
    pub(crate) const fn idle() -> Self {
        Self {
            pending: None,
            completion: None,
        }
    }

    pub(crate) const fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn begin(&mut self, ticket: T) -> Result<(), T> {
        if self.pending.is_some() {
            return Err(ticket);
        }
        self.completion = None;
        self.pending = Some(ticket);
        Ok(())
    }

    pub(crate) fn pending_mut(&mut self) -> Option<&mut T> {
        self.pending.as_mut()
    }

    pub(crate) fn complete(&mut self, completion: CaptureCompletionFacts) -> bool {
        if self.pending.take().is_none() {
            return false;
        }
        self.completion = Some(completion);
        true
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.completion = None;
    }

    pub(crate) const fn completion(&self) -> Option<CaptureCompletionFacts> {
        self.completion
    }
}

impl CaptureCompletionFacts {
    pub(crate) fn new(submitted_work_done: Duration, readback_mapping: Duration) -> Self {
        Self {
            schema: "punctra-browser-frame-capture-completion-v1",
            origin: "begin_frame_capture_monotonic_clock",
            submitted_work_done_callback_milliseconds: submitted_work_done.as_secs_f64() * 1_000.0,
            readback_mapping_callback_milliseconds: readback_mapping.as_secs_f64() * 1_000.0,
        }
    }

    pub(crate) fn to_json(self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self)
    }
}

/// Immutable facts about the renderer work represented by one capture.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CaptureFrameFacts {
    view_generation: u64,
    drawn_points: u64,
    draw_calls: u64,
    resident_bytes: u64,
    renderer_transient_texture_bytes: u64,
    point_footprint: PointFootprintFacts,
    batches: Vec<VisualBatchFacts>,
}

impl CaptureFrameFacts {
    pub(crate) fn new(
        view_generation: u64,
        drawn_points: u64,
        draw_calls: u64,
        resident_bytes: u64,
        renderer_transient_texture_bytes: u64,
        point_footprint: PointFootprintFacts,
        batches: Vec<VisualBatchFacts>,
    ) -> Self {
        Self {
            view_generation,
            drawn_points,
            draw_calls,
            resident_bytes,
            renderer_transient_texture_bytes,
            point_footprint,
            batches,
        }
    }
}

/// A validated four-byte color-texture copy layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CaptureLayout {
    dimensions: [u32; 2],
    source: SourcePixels,
    tight_bytes_per_row: u32,
    padded_bytes_per_row: u32,
    output_bytes: u64,
    staging_bytes: u64,
}

impl CaptureLayout {
    /// Validates one bounded capture layout for the supported browser formats.
    pub(crate) fn new(
        dimensions: [u32; 2],
        format: wgpu::TextureFormat,
    ) -> Result<Self, CaptureError> {
        let [width, height] = dimensions;
        if width == 0 || height == 0 {
            return Err(CaptureError::ZeroDimensions);
        }
        if width > MAX_CANVAS_DIMENSION || height > MAX_CANVAS_DIMENSION {
            return Err(CaptureError::DimensionLimit { dimensions });
        }
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(CaptureError::LengthOverflow)?;
        if pixels > MAX_CANVAS_PIXELS {
            return Err(CaptureError::PixelLimit { pixels });
        }

        let source = SourcePixels::from_texture_format(format)?;
        let tight_bytes_per_row = width
            .checked_mul(CAPTURE_BYTES_PER_PIXEL)
            .ok_or(CaptureError::LengthOverflow)?;
        let padded_bytes_per_row = tight_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .checked_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .ok_or(CaptureError::LengthOverflow)?;
        let output_bytes = u64::from(tight_bytes_per_row)
            .checked_mul(u64::from(height))
            .ok_or(CaptureError::LengthOverflow)?;
        let staging_bytes = u64::from(padded_bytes_per_row)
            .checked_mul(u64::from(height))
            .ok_or(CaptureError::LengthOverflow)?;
        usize::try_from(output_bytes).map_err(|_| CaptureError::LengthOverflow)?;
        usize::try_from(staging_bytes).map_err(|_| CaptureError::LengthOverflow)?;

        Ok(Self {
            dimensions,
            source,
            tight_bytes_per_row,
            padded_bytes_per_row,
            output_bytes,
            staging_bytes,
        })
    }

    pub(crate) const fn dimensions(self) -> [u32; 2] {
        self.dimensions
    }

    pub(crate) const fn texture_format(self) -> wgpu::TextureFormat {
        self.source.format
    }

    pub(crate) const fn padded_bytes_per_row(self) -> u32 {
        self.padded_bytes_per_row
    }

    pub(crate) const fn staging_bytes(self) -> u64 {
        self.staging_bytes
    }

    /// Serializes explicit pending, format, resource, and completion facts.
    pub(crate) fn pending_facts_json(
        self,
        frame: CaptureFrameFacts,
    ) -> Result<String, serde_json::Error> {
        serde_json::to_string(&PendingCaptureFacts::new(self, frame))
    }

    /// Removes GPU row padding and normalizes the source channels to tight RGBA8.
    pub(crate) fn canonical_rgba(self, mapped: &[u8]) -> Result<Vec<u8>, CaptureError> {
        let actual = u64::try_from(mapped.len()).map_err(|_| CaptureError::LengthOverflow)?;
        if actual != self.staging_bytes {
            return Err(CaptureError::ReadbackLength {
                expected: self.staging_bytes,
                actual,
            });
        }

        let tight_row =
            usize::try_from(self.tight_bytes_per_row).map_err(|_| CaptureError::LengthOverflow)?;
        let padded_row =
            usize::try_from(self.padded_bytes_per_row).map_err(|_| CaptureError::LengthOverflow)?;
        let output_length =
            usize::try_from(self.output_bytes).map_err(|_| CaptureError::LengthOverflow)?;
        let height =
            usize::try_from(self.dimensions[1]).map_err(|_| CaptureError::LengthOverflow)?;
        let mut output = Vec::with_capacity(output_length);
        for row in mapped.chunks_exact(padded_row).take(height) {
            output.extend_from_slice(&row[..tight_row]);
        }
        if output.len() != output_length {
            return Err(CaptureError::LengthOverflow);
        }
        if self.source.channel_order == ChannelOrder::Bgra {
            for pixel in output.chunks_exact_mut(usize::try_from(CAPTURE_BYTES_PER_PIXEL).unwrap())
            {
                pixel.swap(0, 2);
            }
        }
        Ok(output)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourcePixels {
    format: wgpu::TextureFormat,
    format_name: &'static str,
    channel_order: ChannelOrder,
    encoding: &'static str,
}

impl SourcePixels {
    fn from_texture_format(format: wgpu::TextureFormat) -> Result<Self, CaptureError> {
        let (format_name, channel_order, encoding) = match format {
            wgpu::TextureFormat::Rgba8Unorm => ("rgba8_unorm", ChannelOrder::Rgba, "linear"),
            wgpu::TextureFormat::Rgba8UnormSrgb => ("rgba8_unorm_srgb", ChannelOrder::Rgba, "srgb"),
            wgpu::TextureFormat::Bgra8Unorm => ("bgra8_unorm", ChannelOrder::Bgra, "linear"),
            wgpu::TextureFormat::Bgra8UnormSrgb => ("bgra8_unorm_srgb", ChannelOrder::Bgra, "srgb"),
            _ => return Err(CaptureError::UnsupportedFormat { format }),
        };
        Ok(Self {
            format,
            format_name,
            channel_order,
            encoding,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChannelOrder {
    Rgba,
    Bgra,
}

impl ChannelOrder {
    const fn name(self) -> &'static str {
        match self {
            Self::Rgba => "rgba",
            Self::Bgra => "bgra",
        }
    }
}

#[derive(Serialize)]
struct PendingCaptureFacts {
    schema: &'static str,
    status: &'static str,
    completion: &'static str,
    presentation: &'static str,
    width: u32,
    height: u32,
    view_generation: u64,
    drawn_points: u64,
    draw_calls: u64,
    resident_bytes: u64,
    renderer_transient_texture_bytes: u64,
    point_footprint: PointFootprintFacts,
    batch_state_authority: &'static str,
    batches: Vec<VisualBatchFacts>,
    source_format: &'static str,
    source_channel_order: &'static str,
    source_encoding: &'static str,
    configured_surface_color_space: &'static str,
    canonical_format: &'static str,
    canonical_channel_order: &'static str,
    canonical_encoding: &'static str,
    origin: &'static str,
    bytes_per_pixel: u32,
    row_alignment_bytes: u32,
    tight_bytes_per_row: u32,
    padded_bytes_per_row: u32,
    output_bytes: u64,
    color_texture_bytes: u64,
    staging_buffer_bytes: u64,
}

impl PendingCaptureFacts {
    fn new(layout: CaptureLayout, frame: CaptureFrameFacts) -> Self {
        Self {
            schema: "punctra-browser-frame-capture-v1",
            status: "pending",
            completion: "map_callback_pending",
            presentation: "offscreen_not_presented",
            width: layout.dimensions[0],
            height: layout.dimensions[1],
            view_generation: frame.view_generation,
            drawn_points: frame.drawn_points,
            draw_calls: frame.draw_calls,
            resident_bytes: frame.resident_bytes,
            renderer_transient_texture_bytes: frame.renderer_transient_texture_bytes,
            point_footprint: frame.point_footprint,
            batch_state_authority: "renderer_accepted_updates",
            batches: frame.batches,
            source_format: layout.source.format_name,
            source_channel_order: layout.source.channel_order.name(),
            source_encoding: layout.source.encoding,
            configured_surface_color_space: "srgb",
            canonical_format: "rgba8",
            canonical_channel_order: "rgba",
            canonical_encoding: layout.source.encoding,
            origin: "top_left",
            bytes_per_pixel: CAPTURE_BYTES_PER_PIXEL,
            row_alignment_bytes: wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            tight_bytes_per_row: layout.tight_bytes_per_row,
            padded_bytes_per_row: layout.padded_bytes_per_row,
            output_bytes: layout.output_bytes,
            color_texture_bytes: layout.output_bytes,
            staging_buffer_bytes: layout.staging_bytes,
        }
    }
}

/// A rejected capture format, layout, or mapped byte range.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(crate) enum CaptureError {
    #[error("capture dimensions must be nonzero")]
    ZeroDimensions,
    #[error(
        "capture dimensions {dimensions:?} exceed the {MAX_CANVAS_DIMENSION}-pixel dimension ceiling"
    )]
    DimensionLimit { dimensions: [u32; 2] },
    #[error("capture area contains {pixels} pixels above the {MAX_CANVAS_PIXELS}-pixel ceiling")]
    PixelLimit { pixels: u64 },
    #[error("capture texture format {format:?} is not a supported four-byte RGBA/BGRA format")]
    UnsupportedFormat { format: wgpu::TextureFormat },
    #[error("capture byte-length arithmetic overflowed the bounded host address space")]
    LengthOverflow,
    #[error("capture readback contains {actual} bytes instead of the required {expected} bytes")]
    ReadbackLength { expected: u64, actual: u64 },
}

#[cfg(test)]
mod tests {
    use render_wgpu::{PointFootprint, PointFootprintStatus};
    use serde_json::json;
    use std::time::Duration;

    use super::*;

    #[test]
    fn completion_facts_separate_the_two_observed_callbacks() {
        let facts = CaptureCompletionFacts::new(
            Duration::from_micros(12_500),
            Duration::from_micros(9_250),
        );
        let value: serde_json::Value = serde_json::from_str(&facts.to_json().unwrap()).unwrap();
        assert_eq!(
            value,
            json!({
                "schema": "punctra-browser-frame-capture-completion-v1",
                "origin": "begin_frame_capture_monotonic_clock",
                "submitted_work_done_callback_milliseconds": 12.5,
                "readback_mapping_callback_milliseconds": 9.25
            })
        );
    }

    #[test]
    fn capture_slot_owns_exactly_one_pending_ticket_and_cleans_every_terminal_path() {
        let mut slot = CaptureSlot::idle();
        assert!(!slot.is_pending());
        assert_eq!(slot.completion(), None);

        slot.begin(7_u8).unwrap();
        assert!(slot.is_pending());
        assert_eq!(slot.pending_mut().map(|ticket| *ticket), Some(7));
        assert_eq!(slot.begin(9_u8), Err(9));
        assert_eq!(slot.pending_mut().map(|ticket| *ticket), Some(7));

        let completion =
            CaptureCompletionFacts::new(Duration::from_millis(4), Duration::from_millis(3));
        assert!(slot.complete(completion));
        assert!(!slot.is_pending());
        assert_eq!(slot.completion(), Some(completion));
        assert!(!slot.complete(completion));

        slot.begin(11_u8).unwrap();
        assert!(slot.is_pending());
        assert_eq!(slot.completion(), None);
        slot.cancel();
        assert!(!slot.is_pending());
        assert_eq!(slot.completion(), None);
    }

    #[test]
    fn rgba_formats_strip_each_padded_row_without_changing_channels() {
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            let layout = CaptureLayout::new([2, 2], format).unwrap();
            let mut mapped = vec![0xEE; usize::try_from(layout.staging_bytes()).unwrap()];
            mapped[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
            let second_row = usize::try_from(layout.padded_bytes_per_row()).unwrap();
            mapped[second_row..second_row + 8].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);

            assert_eq!(
                layout.canonical_rgba(&mapped).unwrap(),
                [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
            );
        }
    }

    #[test]
    fn bgra_formats_strip_padding_and_swizzle_red_and_blue() {
        for format in [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ] {
            let layout = CaptureLayout::new([2, 1], format).unwrap();
            let mut mapped = vec![0xEE; usize::try_from(layout.staging_bytes()).unwrap()];
            mapped[..8].copy_from_slice(&[3, 2, 1, 4, 7, 6, 5, 8]);

            assert_eq!(
                layout.canonical_rgba(&mapped).unwrap(),
                [1, 2, 3, 4, 5, 6, 7, 8]
            );
        }
    }

    #[test]
    fn layout_rejects_unbounded_or_unsupported_inputs() {
        assert_eq!(
            CaptureLayout::new([0, 1], wgpu::TextureFormat::Rgba8Unorm),
            Err(CaptureError::ZeroDimensions)
        );
        assert_eq!(
            CaptureLayout::new(
                [MAX_CANVAS_DIMENSION + 1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
            ),
            Err(CaptureError::DimensionLimit {
                dimensions: [MAX_CANVAS_DIMENSION + 1, 1]
            })
        );
        assert!(matches!(
            CaptureLayout::new(
                [MAX_CANVAS_DIMENSION, MAX_CANVAS_DIMENSION],
                wgpu::TextureFormat::Rgba8Unorm,
            ),
            Err(CaptureError::PixelLimit { .. })
        ));
        assert_eq!(
            CaptureLayout::new([1, 1], wgpu::TextureFormat::Rgba16Float),
            Err(CaptureError::UnsupportedFormat {
                format: wgpu::TextureFormat::Rgba16Float
            })
        );
    }

    #[test]
    fn canonicalization_requires_the_exact_staging_length() {
        let layout = CaptureLayout::new([1, 2], wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let expected = layout.staging_bytes();
        assert_eq!(
            layout.canonical_rgba(&vec![0; usize::try_from(expected - 1).unwrap()]),
            Err(CaptureError::ReadbackLength {
                expected,
                actual: expected - 1
            })
        );
        assert_eq!(
            layout.canonical_rgba(&vec![0; usize::try_from(expected + 1).unwrap()]),
            Err(CaptureError::ReadbackLength {
                expected,
                actual: expected + 1
            })
        );
    }

    #[test]
    fn pending_facts_distinguish_format_layout_completion_and_presentation() {
        let layout = CaptureLayout::new([65, 2], wgpu::TextureFormat::Bgra8UnormSrgb).unwrap();
        let frame = CaptureFrameFacts::new(
            7,
            42,
            3,
            4_096,
            33_280,
            PointFootprintFacts::new(
                PointFootprint::Antialiased,
                PointFootprintStatus::Multisample4x,
                7.0,
                4.25,
            ),
            vec![VisualBatchFacts::resident(0, 1, 2, 42, 96)],
        );
        assert_eq!(layout.dimensions(), [65, 2]);
        assert_eq!(layout.texture_format(), wgpu::TextureFormat::Bgra8UnormSrgb);
        let value: serde_json::Value =
            serde_json::from_str(&layout.pending_facts_json(frame).unwrap()).unwrap();

        assert_eq!(value["schema"], "punctra-browser-frame-capture-v1");
        assert_eq!(value["status"], "pending");
        assert_eq!(value["completion"], "map_callback_pending");
        assert_eq!(value["presentation"], "offscreen_not_presented");
        assert_eq!(value["view_generation"], 7);
        assert_eq!(value["source_format"], "bgra8_unorm_srgb");
        assert_eq!(value["source_channel_order"], "bgra");
        assert_eq!(value["source_encoding"], "srgb");
        assert_eq!(value["configured_surface_color_space"], "srgb");
        assert_eq!(value["canonical_format"], "rgba8");
        assert_eq!(value["canonical_channel_order"], "rgba");
        assert_eq!(value["canonical_encoding"], "srgb");
        assert_eq!(value["origin"], "top_left");
        assert_eq!(value["tight_bytes_per_row"], 260);
        assert_eq!(value["padded_bytes_per_row"], 512);
        assert_eq!(value["output_bytes"], 520);
        assert_eq!(value["staging_buffer_bytes"], 1_024);
        assert_eq!(
            value,
            json!({
                "schema": "punctra-browser-frame-capture-v1",
                "status": "pending",
                "completion": "map_callback_pending",
                "presentation": "offscreen_not_presented",
                "width": 65,
                "height": 2,
                "view_generation": 7,
                "drawn_points": 42,
                "draw_calls": 3,
                "resident_bytes": 4_096,
                "renderer_transient_texture_bytes": 33_280,
                "point_footprint": {
                    "requested": "antialiased",
                    "selected": "multisample4x",
                    "nominal_pick_size_physical_pixels": 7.0,
                    "display_size_physical_pixels": 4.25
                },
                "batch_state_authority": "renderer_accepted_updates",
                "batches": [{
                    "batch_index": 0,
                    "key": 1,
                    "version": 2,
                    "point_count": 42,
                    "state": "resident",
                    "presentation_weight_u8": 96
                }],
                "source_format": "bgra8_unorm_srgb",
                "source_channel_order": "bgra",
                "source_encoding": "srgb",
                "configured_surface_color_space": "srgb",
                "canonical_format": "rgba8",
                "canonical_channel_order": "rgba",
                "canonical_encoding": "srgb",
                "origin": "top_left",
                "bytes_per_pixel": 4,
                "row_alignment_bytes": 256,
                "tight_bytes_per_row": 260,
                "padded_bytes_per_row": 512,
                "output_bytes": 520,
                "color_texture_bytes": 520,
                "staging_buffer_bytes": 1_024
            })
        );
    }
}
