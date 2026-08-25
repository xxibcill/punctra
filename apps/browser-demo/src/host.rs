use serde::Serialize;
use thiserror::Error;

pub(crate) const MAX_CANVAS_DIMENSION: u32 = 4_096;
pub(crate) const MAX_CANVAS_PIXELS: u64 = 8_388_608;
pub(crate) const MAX_DEVICE_PIXEL_RATIO: f64 = 4.0;
pub(crate) const SURFACE_BYTES_PER_PIXEL: u64 = 4;
pub(crate) const RENDERER_DEPTH_AND_PICK_BYTES_PER_PIXEL: u64 = 8;
pub(crate) const MAX_RENDER_TRANSIENT_BYTES: u64 =
    MAX_CANVAS_PIXELS * RENDERER_DEPTH_AND_PICK_BYTES_PER_PIXEL;
pub(crate) const PRESENTATION_LATENCY_FRAMES: u32 = 2;
pub(crate) const RESIZE_VIEWPORT_ACTION: &str = "Keep the current surface configuration, choose finite positive CSS dimensions and a device-pixel ratio at most four so the physical canvas remains within 4,096 pixels per dimension and 8,388,608 pixels total, then resize again.";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CssViewportRequest {
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
}

impl CssViewportRequest {
    pub(crate) const fn new(width: f64, height: f64, device_pixel_ratio: f64) -> Self {
        Self {
            width,
            height,
            device_pixel_ratio,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct PhysicalViewport {
    css_width: f64,
    css_height: f64,
    device_pixel_ratio: f64,
    physical_width: u32,
    physical_height: u32,
    surface_bytes: u64,
}

impl PhysicalViewport {
    pub(crate) fn from_css(request: CssViewportRequest) -> Result<Self, HostModelError> {
        validate_css_size(request)?;
        let physical_width = physical_dimension(request.width, request.device_pixel_ratio)?;
        let physical_height = physical_dimension(request.height, request.device_pixel_ratio)?;
        validate_physical_size(physical_width, physical_height)?;
        let surface_bytes = pixel_count(physical_width, physical_height)?
            .checked_mul(SURFACE_BYTES_PER_PIXEL)
            .ok_or(HostModelError::SizeOverflow)?;
        Ok(Self {
            css_width: request.width,
            css_height: request.height,
            device_pixel_ratio: request.device_pixel_ratio,
            physical_width,
            physical_height,
            surface_bytes,
        })
    }

    pub(crate) const fn dimensions(self) -> [u32; 2] {
        [self.physical_width, self.physical_height]
    }

    pub(crate) fn renderer_transient_bytes_with_pick(self) -> Result<u64, HostModelError> {
        pixel_count(self.physical_width, self.physical_height)?
            .checked_mul(RENDERER_DEPTH_AND_PICK_BYTES_PER_PIXEL)
            .ok_or(HostModelError::SizeOverflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewerPhase {
    Ready,
    Hidden,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RenderDisposition {
    Record,
    SkipHidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Lifecycle {
    phase: ViewerPhase,
    rendered_frames: u64,
    hidden_frame_skips: u64,
}

impl Lifecycle {
    pub(crate) const fn ready() -> Self {
        Self {
            phase: ViewerPhase::Ready,
            rendered_frames: 0,
            hidden_frame_skips: 0,
        }
    }

    pub(crate) const fn phase(self) -> ViewerPhase {
        self.phase
    }

    pub(crate) const fn rendered_frames(self) -> u64 {
        self.rendered_frames
    }

    pub(crate) const fn hidden_frame_skips(self) -> u64 {
        self.hidden_frame_skips
    }

    pub(crate) fn set_visible(&mut self, visible: bool) -> Result<(), HostModelError> {
        self.ensure_active()?;
        self.phase = if visible {
            ViewerPhase::Ready
        } else {
            ViewerPhase::Hidden
        };
        Ok(())
    }

    pub(crate) fn begin_render(&mut self) -> Result<RenderDisposition, HostModelError> {
        match self.phase {
            ViewerPhase::Ready => Ok(RenderDisposition::Record),
            ViewerPhase::Hidden => {
                self.hidden_frame_skips = self
                    .hidden_frame_skips
                    .checked_add(1)
                    .ok_or(HostModelError::SizeOverflow)?;
                Ok(RenderDisposition::SkipHidden)
            }
            ViewerPhase::Shutdown => Err(HostModelError::ViewerShutdown),
        }
    }

    pub(crate) fn record_frame(&mut self) -> Result<(), HostModelError> {
        self.ensure_active()?;
        self.rendered_frames = self
            .rendered_frames
            .checked_add(1)
            .ok_or(HostModelError::SizeOverflow)?;
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), HostModelError> {
        self.ensure_active()?;
        self.phase = ViewerPhase::Shutdown;
        Ok(())
    }

    pub(crate) fn ensure_active(self) -> Result<(), HostModelError> {
        if self.phase == ViewerPhase::Shutdown {
            Err(HostModelError::ViewerShutdown)
        } else {
            Ok(())
        }
    }

    pub(crate) fn ensure_source_publication(self) -> Result<(), HostModelError> {
        self.ensure_active()
    }

    pub(crate) fn ensure_ready(self) -> Result<(), HostModelError> {
        self.ensure_active()?;
        if self.phase == ViewerPhase::Hidden {
            Err(HostModelError::ViewerHidden)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub(crate) enum HostModelError {
    #[error("CSS width, height, and device-pixel ratio must be positive and finite")]
    InvalidCssSize,
    #[error("device-pixel ratio exceeds the accepted maximum of {MAX_DEVICE_PIXEL_RATIO}")]
    DevicePixelRatioLimit,
    #[error("physical canvas dimensions must be nonzero and at most {MAX_CANVAS_DIMENSION}")]
    CanvasDimensionLimit,
    #[error("physical canvas area exceeds the accepted {MAX_CANVAS_PIXELS}-pixel ceiling")]
    CanvasPixelLimit,
    #[error("viewer is shut down; create a new viewer before performing more work")]
    ViewerShutdown,
    #[error("the host declared the canvas hidden")]
    ViewerHidden,
    #[error("browser host resource accounting overflowed")]
    SizeOverflow,
}

fn validate_css_size(request: CssViewportRequest) -> Result<(), HostModelError> {
    if !request.width.is_finite()
        || request.width <= 0.0
        || !request.height.is_finite()
        || request.height <= 0.0
        || !request.device_pixel_ratio.is_finite()
        || request.device_pixel_ratio <= 0.0
    {
        return Err(HostModelError::InvalidCssSize);
    }
    if request.device_pixel_ratio > MAX_DEVICE_PIXEL_RATIO {
        return Err(HostModelError::DevicePixelRatioLimit);
    }
    Ok(())
}

fn physical_dimension(css_size: f64, device_pixel_ratio: f64) -> Result<u32, HostModelError> {
    let physical = (css_size * device_pixel_ratio).round();
    if !physical.is_finite() || physical < 1.0 || physical > f64::from(u32::MAX) {
        return Err(HostModelError::CanvasDimensionLimit);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(physical as u32)
}

fn validate_physical_size(width: u32, height: u32) -> Result<(), HostModelError> {
    if width > MAX_CANVAS_DIMENSION || height > MAX_CANVAS_DIMENSION {
        return Err(HostModelError::CanvasDimensionLimit);
    }
    if pixel_count(width, height)? > MAX_CANVAS_PIXELS {
        return Err(HostModelError::CanvasPixelLimit);
    }
    Ok(())
}

fn pixel_count(width: u32, height: u32) -> Result<u64, HostModelError> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(HostModelError::SizeOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_preserves_separate_css_physical_and_surface_facts() {
        let viewport =
            PhysicalViewport::from_css(CssViewportRequest::new(800.0, 500.0, 2.0)).unwrap();

        assert_eq!(viewport.dimensions(), [1_600, 1_000]);
        assert_eq!(viewport.surface_bytes, 6_400_000);
        assert_eq!(
            viewport.renderer_transient_bytes_with_pick().unwrap(),
            12_800_000
        );
    }

    #[test]
    fn viewport_limits_fail_before_rounding_into_an_accepted_size() {
        assert_eq!(
            PhysicalViewport::from_css(CssViewportRequest::new(1_025.0, 600.0, 4.0)),
            Err(HostModelError::CanvasDimensionLimit)
        );
        assert_eq!(
            PhysicalViewport::from_css(CssViewportRequest::new(800.0, 600.0, 4.01)),
            Err(HostModelError::DevicePixelRatioLimit)
        );
        assert_eq!(
            PhysicalViewport::from_css(CssViewportRequest::new(4_000.0, 4_000.0, 1.0)),
            Err(HostModelError::CanvasPixelLimit)
        );
    }

    #[test]
    fn lifecycle_suspends_hidden_work_and_fuses_shutdown() {
        let mut lifecycle = Lifecycle::ready();
        assert_eq!(lifecycle.phase(), ViewerPhase::Ready);
        assert_eq!(MAX_RENDER_TRANSIENT_BYTES, 67_108_864);
        lifecycle.set_visible(false).unwrap();
        assert_eq!(lifecycle.phase(), ViewerPhase::Hidden);
        assert_eq!(
            lifecycle.begin_render().unwrap(),
            RenderDisposition::SkipHidden
        );
        assert_eq!(lifecycle.ensure_ready(), Err(HostModelError::ViewerHidden));
        lifecycle.ensure_source_publication().unwrap();
        assert_eq!(lifecycle.hidden_frame_skips(), 1);

        lifecycle.set_visible(true).unwrap();
        lifecycle.ensure_ready().unwrap();
        assert_eq!(lifecycle.begin_render().unwrap(), RenderDisposition::Record);
        lifecycle.record_frame().unwrap();
        lifecycle.shutdown().unwrap();

        assert_eq!(lifecycle.phase(), ViewerPhase::Shutdown);
        assert_eq!(lifecycle.rendered_frames(), 1);
        assert_eq!(
            lifecycle.begin_render(),
            Err(HostModelError::ViewerShutdown)
        );
        assert_eq!(
            lifecycle.set_visible(true),
            Err(HostModelError::ViewerShutdown)
        );
        assert_eq!(lifecycle.shutdown(), Err(HostModelError::ViewerShutdown));
        assert_eq!(
            lifecycle.ensure_source_publication(),
            Err(HostModelError::ViewerShutdown)
        );
    }
}
