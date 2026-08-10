use glam::Mat4;
use render_protocol::{Camera, ViewGenerationKey, Viewport};
use thiserror::Error;

/// Appearance shared by every point in one rendered frame.
///
/// The default style uses a 3.0-physical-pixel point diameter, the linear RGB
/// highlight color `[1.0, 0.8, 0.1]`, and the linear RGBA clear color
/// `[0.015, 0.02, 0.03, 1.0]`. Highlighting replaces only source RGB and
/// preserves each point's source alpha for drawing and picking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointStyle {
    default_size_pixels: f32,
    highlight_color: [f32; 3],
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
        highlight_color: [f32; 3],
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

    /// Returns the diameter used for every point in the frame.
    #[must_use]
    pub const fn default_size_pixels(self) -> f32 {
        self.default_size_pixels
    }

    /// Returns the linear RGB highlight color.
    #[must_use]
    pub const fn highlight_color(self) -> [f32; 3] {
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
            highlight_color: [1.0, 0.8, 0.1],
            clear_color: [0.015, 0.02, 0.03, 1.0],
        }
    }
}

/// All caller-controlled values used to record one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    view_generation: ViewGenerationKey,
    camera: Camera,
    viewport: Viewport,
    style: PointStyle,
    view_projection: Mat4,
}

impl Frame {
    /// Creates a frame using [`PointStyle::default`].
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::NonFiniteCameraProjection`] when the viewport's
    /// aspect ratio would make the camera projection non-finite.
    pub fn new(
        view_generation: ViewGenerationKey,
        camera: Camera,
        viewport: Viewport,
    ) -> Result<Self, FrameError> {
        let view_projection = Mat4::from_cols_array(
            &camera
                .view_projection_matrix(viewport.aspect_ratio())
                .map_err(|_| FrameError::NonFiniteCameraProjection {
                    viewport: viewport.dimensions(),
                })?,
        );

        Ok(Self {
            view_generation,
            camera,
            viewport,
            style: PointStyle::default(),
            view_projection,
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
    pub const fn viewport(self) -> Viewport {
        self.viewport
    }

    /// Returns this frame's point style.
    #[must_use]
    pub const fn style(self) -> PointStyle {
        self.style
    }

    pub(crate) const fn view_projection(self) -> Mat4 {
        self.view_projection
    }
}

/// A frame or style construction error.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum FrameError {
    /// The camera cannot produce a finite projection for this viewport.
    #[error("camera projection for viewport {viewport:?} must remain finite")]
    NonFiniteCameraProjection {
        /// The viewport whose aspect ratio made the projection non-finite.
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

    #[test]
    fn style_rejects_invalid_size_and_color() {
        assert_eq!(
            PointStyle::new(0.0, [1.0; 3], [0.0; 4]),
            Err(FrameError::InvalidPointSize(0.0))
        );
        assert_eq!(
            PointStyle::new(2.0, [1.0, f32::NAN, 0.0], [0.0; 4]),
            Err(FrameError::InvalidColor {
                name: "highlight",
                channel: 1,
            })
        );
    }

    #[test]
    fn frame_rejects_a_projection_that_overflows_for_its_viewport() {
        let view_generation = ViewGenerationKey::new(ViewId::new(1), 1);
        let narrow_field_of_view_camera = Camera::perspective(
            [0.0, -5.0, 0.0],
            [0.0; 3],
            [0.0, 0.0, 1.0],
            1.0e-30,
            0.1,
            100.0,
        )
        .expect("the projection should remain finite at a square aspect ratio");

        assert_eq!(
            Frame::new(
                view_generation,
                narrow_field_of_view_camera,
                Viewport::new(1, u32::MAX).unwrap(),
            ),
            Err(FrameError::NonFiniteCameraProjection {
                viewport: [1, u32::MAX],
            })
        );
    }
}
