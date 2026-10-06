use crate::ProtocolError;

/// One side of a complementary, disposable raster-coverage transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterTransitionSide {
    /// Replacement coverage grows as progress advances.
    Incoming,
    /// Retiring coverage uses the exact complement of the incoming mask.
    Outgoing,
}

/// Bounded presentation of one resident batch during a LOD transition.
///
/// Color and visibility depth share the mask. Nominal picking, Point identity,
/// Source data, and exact Query completion remain independent. Matching sides
/// use the same seed and progress; the host owns group membership and timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterTransition {
    seed: u32,
    step: u8,
    side: Option<RasterTransitionSide>,
}

impl RasterTransition {
    /// The endpoint of the fixed eight-step coverage interval.
    pub const MAX_STEP: u8 = 8;

    /// Full raster coverage without a transition selector.
    pub const FULL: Self = Self {
        seed: 0,
        step: Self::MAX_STEP,
        side: None,
    };

    /// No color or visibility-depth coverage; nominal picks remain available.
    pub const HIDDEN: Self = Self {
        seed: 0,
        step: 0,
        side: Some(RasterTransitionSide::Incoming),
    };

    /// Creates one side of a deterministic complementary transition.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidRasterTransitionStep`] for progress
    /// greater than [`Self::MAX_STEP`].
    pub const fn new(
        seed: u32,
        side: RasterTransitionSide,
        step: u8,
    ) -> Result<Self, ProtocolError> {
        if step > Self::MAX_STEP {
            return Err(ProtocolError::InvalidRasterTransitionStep { step });
        }
        Ok(Self {
            seed,
            step,
            side: Some(side),
        })
    }

    /// Returns the caller's stable group seed.
    #[must_use]
    pub const fn seed(self) -> u32 {
        self.seed
    }

    /// Returns the validated zero-through-eight progress step.
    #[must_use]
    pub const fn step(self) -> u8 {
        self.step
    }

    /// Returns the controlled side, or `None` for full unmasked coverage.
    #[must_use]
    pub const fn side(self) -> Option<RasterTransitionSide> {
        self.side
    }

    /// Returns the declared coverage fraction's numerator over eight.
    ///
    /// This is a display-density policy input, not a count of visible Points.
    #[must_use]
    pub const fn coverage_eighths(self) -> u8 {
        match self.side {
            None => Self::MAX_STEP,
            Some(RasterTransitionSide::Incoming) => self.step,
            Some(RasterTransitionSide::Outgoing) => Self::MAX_STEP - self.step,
        }
    }
}

impl Default for RasterTransition {
    fn default() -> Self {
        Self::FULL
    }
}
