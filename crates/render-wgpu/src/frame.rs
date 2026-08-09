use render_protocol::ViewGenerationKey;
use thiserror::Error;

use crate::Camera;

/// Appearance shared by every point in one rendered frame.
///
/// The default style uses a 3.0-physical-pixel point diameter, the linear RGBA
/// highlight color `[1.0, 0.8, 0.1, 1.0]`, and the linear RGBA clear color
/// `[0.015, 0.02, 0.03, 1.0]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointStyle {
    default_size_pixels: f32,
    highlight_color: [f32; 4],
    clear_color: [f64; 4],
}

impl PointStyle {
    /// Creates a validated point style.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] when the point size is not positive and finite
    /// or a color channel lies outside the finite inclusive unit interval.
    pub fn new(
        default_size_pixels: f32,
        highlight_color: [f32; 4],
        clear_color: [f64; 4],
    ) -> Result<Self, FrameError> {
        if !default_size_pixels.is_finite() || default_size_pixels <= 0.0 {
            return Err(FrameError::InvalidPointSize(default_size_pixels));
        }
        validate_color("highlight", &highlight_color)?;
        validate_color("clear", &clear_color)?;

        Ok(Self {
            default_size_pixels,
            highlight_color,
            clear_color,
        })
    }

    /// Returns the diameter used when a point has no size override.
    #[must_use]
    pub const fn default_size_pixels(self) -> f32 {
        self.default_size_pixels
    }

    /// Returns the linear RGBA highlight color.
    #[must_use]
    pub const fn highlight_color(self) -> [f32; 4] {
        self.highlight_color
    }

    /// Returns the linear RGBA clear color.
    #[must_use]
    pub const fn clear_color(self) -> [f64; 4] {
        self.clear_color
    }
}

impl Default for PointStyle {
    fn default() -> Self {
        Self {
            default_size_pixels: 3.0,
            highlight_color: [1.0, 0.8, 0.1, 1.0],
            clear_color: [0.015, 0.02, 0.03, 1.0],
        }
    }
}

/// All caller-controlled values used to record one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    view_generation: ViewGenerationKey,
    camera: Camera,
    viewport: [u32; 2],
    style: PointStyle,
}

impl Frame {
    /// Creates a frame using [`PointStyle::default`].
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::EmptyViewport`] when either physical extent is zero.
    pub fn new(
        view_generation: ViewGenerationKey,
        camera: Camera,
        viewport: [u32; 2],
    ) -> Result<Self, FrameError> {
        if viewport.into_iter().any(|extent| extent == 0) {
            return Err(FrameError::EmptyViewport { viewport });
        }

        Ok(Self {
            view_generation,
            camera,
            viewport,
            style: PointStyle::default(),
        })
    }

    /// Replaces the point style.
    #[must_use]
    pub const fn with_style(mut self, style: PointStyle) -> Self {
        self.style = style;
        self
    }

    /// Returns the View generation to draw.
    #[must_use]
    pub const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns the camera.
    #[must_use]
    pub const fn camera(self) -> Camera {
        self.camera
    }

    /// Returns the physical viewport width and height.
    #[must_use]
    pub const fn viewport(self) -> [u32; 2] {
        self.viewport
    }

    /// Returns this frame's point style.
    #[must_use]
    pub const fn style(self) -> PointStyle {
        self.style
    }
}

/// A frame or style construction error.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum FrameError {
    /// The requested physical viewport has no drawable area.
    #[error("frame viewport must be non-zero, got {viewport:?}")]
    EmptyViewport {
        /// The rejected viewport.
        viewport: [u32; 2],
    },
    /// The default point diameter is not positive and finite.
    #[error("default point size must be positive and finite, got {0}")]
    InvalidPointSize(f32),
    /// A color channel is outside the finite inclusive unit interval.
    #[error("{name} color channel {channel} must be finite and inside [0, 1]")]
    InvalidColor {
        /// The color's role.
        name: &'static str,
        /// Zero-based RGBA channel.
        channel: usize,
    },
}

fn validate_color<T, const N: usize>(name: &'static str, color: &[T; N]) -> Result<(), FrameError>
where
    T: UnitChannel,
{
    if let Some(channel) = color.iter().position(|value| !value.is_unit_channel()) {
        Err(FrameError::InvalidColor { name, channel })
    } else {
        Ok(())
    }
}

trait UnitChannel {
    fn is_unit_channel(&self) -> bool;
}

impl UnitChannel for f32 {
    fn is_unit_channel(&self) -> bool {
        self.is_finite() && (0.0..=1.0).contains(self)
    }
}

impl UnitChannel for f64 {
    fn is_unit_channel(&self) -> bool {
        self.is_finite() && (0.0..=1.0).contains(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_protocol::ViewId;

    fn camera() -> Camera {
        Camera::perspective(
            [0.0, -5.0, 2.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            1.0,
            0.1,
            100.0,
        )
        .unwrap()
    }

    #[test]
    fn frame_rejects_an_empty_viewport() {
        let view_generation = ViewGenerationKey::new(ViewId::new(1), 1);

        assert_eq!(
            Frame::new(view_generation, camera(), [1920, 0]),
            Err(FrameError::EmptyViewport {
                viewport: [1920, 0]
            })
        );
    }

    #[test]
    fn style_rejects_invalid_size_and_color() {
        assert_eq!(
            PointStyle::new(0.0, [1.0; 4], [0.0; 4]),
            Err(FrameError::InvalidPointSize(0.0))
        );
        assert_eq!(
            PointStyle::new(2.0, [1.0, f32::NAN, 0.0, 1.0], [0.0; 4]),
            Err(FrameError::InvalidColor {
                name: "highlight",
                channel: 1,
            })
        );
    }
}
