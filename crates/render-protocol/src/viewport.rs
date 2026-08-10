use std::num::NonZeroU32;

use thiserror::Error;

/// Validated physical dimensions shared by planning and rendering.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Viewport {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl Viewport {
    /// Creates a viewport with nonzero physical extents.
    ///
    /// # Errors
    ///
    /// Returns [`ViewportError::Empty`] when either extent is zero.
    pub const fn new(width: u32, height: u32) -> Result<Self, ViewportError> {
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return Err(ViewportError::Empty {
                dimensions: [width, height],
            });
        };
        Ok(Self { width, height })
    }

    /// Returns the physical width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    /// Returns the physical height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height.get()
    }

    /// Returns the physical width and height in pixels.
    #[must_use]
    pub const fn dimensions(self) -> [u32; 2] {
        [self.width(), self.height()]
    }

    /// Returns the width-to-height ratio used by camera projection.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn aspect_ratio(self) -> f32 {
        self.width() as f32 / self.height() as f32
    }
}

/// An invalid physical viewport.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ViewportError {
    /// At least one physical extent was zero.
    #[error("viewport dimensions must be nonzero, got {dimensions:?}")]
    Empty {
        /// Rejected width and height.
        dimensions: [u32; 2],
    },
}
