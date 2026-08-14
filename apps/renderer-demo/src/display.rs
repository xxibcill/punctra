use std::{error::Error, ffi::OsStr, fmt};

use point_contracts::{AttributeDataType, AttributeId, SourceMetadata, WorldBounds};
use point_index::{DisplayAttributes, IndexRecipe, InspectionAttributeIds};

/// Stable neutral display color used when no semantic mode is selected.
pub const NEUTRAL_COLOR: [u8; 4] = [190, 205, 220, 255];

/// Stable `source-las` Attribute identity for raw pulse-return intensity.
pub const LAS_INTENSITY_ATTRIBUTE: AttributeId = fixed_attribute_id(1);

/// Stable `source-las` Attribute identity for raw classification.
pub const LAS_CLASSIFICATION_ATTRIBUTE: AttributeId = fixed_attribute_id(6);

/// Stable `source-las` Attribute identities for raw red, green, and blue channels.
pub const LAS_RGB_ATTRIBUTES: [AttributeId; 3] = [
    fixed_attribute_id(16),
    fixed_attribute_id(17),
    fixed_attribute_id(18),
];

const fn fixed_attribute_id(value: u32) -> AttributeId {
    match AttributeId::new(value) {
        Ok(id) => id,
        Err(_) => panic!("fixed LAS Attribute identity must be nonzero"),
    }
}

/// Returns the single fixed inspection Attribute profile used by this host.
///
/// # Panics
///
/// Panics if the checked-in LAS Attribute identities stop being distinct.
#[must_use]
pub fn inspection_attribute_ids() -> InspectionAttributeIds {
    InspectionAttributeIds::new(
        LAS_INTENSITY_ATTRIBUTE,
        LAS_CLASSIFICATION_ATTRIBUTE,
        LAS_RGB_ATTRIBUTES,
    )
    .expect("fixed LAS inspection Attribute identities are distinct")
}

const ELEVATION_COLORS: [[u8; 4]; 5] = [
    [68, 1, 84, 255],
    [59, 82, 139, 255],
    [33, 145, 140, 255],
    [94, 201, 98, 255],
    [253, 231, 37, 255],
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Private-host display policy selected by the renderer demonstration.
pub enum DisplayMode {
    /// One application-owned neutral color.
    #[default]
    Neutral,
    /// Source-wide elevation palette.
    Elevation,
    /// Raw unsigned 16-bit red, green, and blue channels.
    Rgb,
    /// Raw unsigned 16-bit pulse-return intensity.
    Intensity,
    /// Raw unsigned 8-bit LAS classification.
    Classification,
}

/// Index recipe, artifact naming, and version policy for one display family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayIndexPolicy {
    /// Position-only samples used by neutral and elevation display.
    PositionOnly,
    /// Attributed inspection samples used by RGB, intensity, and classification.
    Inspection,
}

impl DisplayIndexPolicy {
    /// Builds the exact index recipe for this display family.
    #[must_use]
    pub fn recipe(self) -> IndexRecipe {
        match self {
            Self::PositionOnly => IndexRecipe::PositionOnlyV1,
            Self::Inspection => IndexRecipe::InspectionV1(inspection_attribute_ids()),
        }
    }

    /// Returns the default suffix appended to the complete Source path.
    #[must_use]
    pub const fn target_suffix(self) -> &'static str {
        match self {
            Self::PositionOnly => ".pidx",
            Self::Inspection => ".inspection-v2.pidx",
        }
    }

    /// Returns the expected recipe and disk versions.
    #[must_use]
    pub const fn versions(self) -> (u32, u32) {
        match self {
            Self::PositionOnly => (1, 1),
            Self::Inspection => (2, 2),
        }
    }
}

impl DisplayMode {
    /// Parses one exact CLI display name.
    #[must_use]
    pub fn parse(value: &OsStr) -> Option<Self> {
        if value == OsStr::new("neutral") {
            Some(Self::Neutral)
        } else if value == OsStr::new("elevation") {
            Some(Self::Elevation)
        } else if value == OsStr::new("rgb") {
            Some(Self::Rgb)
        } else if value == OsStr::new("intensity") {
            Some(Self::Intensity)
        } else if value == OsStr::new("classification") {
            Some(Self::Classification)
        } else {
            None
        }
    }

    /// Reports whether the original synthetic fixture is inapplicable.
    #[must_use]
    pub const fn requires_source(self) -> bool {
        !matches!(self, Self::Neutral)
    }

    /// Reports whether the v2 attributed inspection recipe is required.
    #[must_use]
    pub const fn requires_inspection_index(self) -> bool {
        matches!(self.index_policy(), DisplayIndexPolicy::Inspection)
    }

    /// Returns the complete index policy for this display mode.
    #[must_use]
    pub const fn index_policy(self) -> DisplayIndexPolicy {
        match self {
            Self::Neutral | Self::Elevation => DisplayIndexPolicy::PositionOnly,
            Self::Rgb | Self::Intensity | Self::Classification => DisplayIndexPolicy::Inspection,
        }
    }

    /// Validates the fixed Source Attribute inputs required by this host mode.
    ///
    /// # Errors
    ///
    /// Returns one bounded request diagnostic when an attributed mode cannot
    /// be represented by the fixed v0.10 inspection recipe.
    pub fn validate_source(self, metadata: &SourceMetadata) -> Result<(), &'static str> {
        if !self.requires_inspection_index() {
            return Ok(());
        }
        if !has_attribute(metadata, LAS_INTENSITY_ATTRIBUTE, AttributeDataType::U16)
            || !has_attribute(
                metadata,
                LAS_CLASSIFICATION_ATTRIBUTE,
                AttributeDataType::U8,
            )
        {
            return Err(
                "attributed display requires LAS intensity Attribute 1 as U16 and classification Attribute 6 as U8",
            );
        }
        if self == Self::Rgb
            && !LAS_RGB_ATTRIBUTES
                .into_iter()
                .all(|id| has_attribute(metadata, id, AttributeDataType::U16))
        {
            return Err("RGB display requires all three LAS RGB Attributes 16, 17, and 18 as U16");
        }
        Ok(())
    }
}

fn has_attribute(metadata: &SourceMetadata, id: AttributeId, data_type: AttributeDataType) -> bool {
    metadata
        .attributes()
        .get(id)
        .is_some_and(|definition| definition.data_type() == data_type)
}

impl fmt::Display for DisplayMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Neutral => formatter.write_str("neutral"),
            Self::Elevation => formatter.write_str("elevation"),
            Self::Rgb => formatter.write_str("rgb"),
            Self::Intensity => formatter.write_str("intensity"),
            Self::Classification => formatter.write_str("classification"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// Deterministic CPU color conversion for one selected display mode.
pub enum PointColorizer {
    /// One application-owned neutral color.
    Neutral,
    /// Palette normalized by complete Source Z bounds.
    Elevation {
        /// Inclusive Source-wide Z minimum and maximum, absent for an empty Source.
        source_z_range: Option<[f64; 2]>,
    },
    /// Raw unsigned 16-bit red, green, and blue channels.
    Rgb,
    /// Raw unsigned 16-bit pulse-return intensity.
    Intensity,
    /// Raw unsigned 8-bit LAS classification.
    Classification,
}

/// Failure to provide the explicit sampled input required by a display mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayMappingError {
    /// An attributed display received no inspection Attribute row.
    MissingAttributes(DisplayMode),
}

impl fmt::Display for DisplayMappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAttributes(mode) => {
                write!(formatter, "{mode} display requires inspection Attributes")
            }
        }
    }
}

impl Error for DisplayMappingError {}

impl PointColorizer {
    /// Binds Source-wide facts needed by one display mode.
    #[must_use]
    pub fn for_source(mode: DisplayMode, bounds: Option<WorldBounds>) -> Self {
        match mode {
            DisplayMode::Neutral => Self::Neutral,
            DisplayMode::Elevation => Self::Elevation {
                source_z_range: bounds.map(|bounds| [bounds.min()[2], bounds.max()[2]]),
            },
            DisplayMode::Rgb => Self::Rgb,
            DisplayMode::Intensity => Self::Intensity,
            DisplayMode::Classification => Self::Classification,
        }
    }

    /// Reports whether every input row must carry v2 inspection Attributes.
    #[must_use]
    pub const fn requires_attributes(self) -> bool {
        matches!(self, Self::Rgb | Self::Intensity | Self::Classification)
    }

    /// Maps one sample to the exact RGBA8 bytes uploaded by the renderer.
    ///
    /// # Errors
    ///
    /// Returns [`DisplayMappingError::MissingAttributes`] when an attributed
    /// display receives no explicit inspection Attribute row.
    pub fn color(
        self,
        world_z: f64,
        attributes: Option<DisplayAttributes>,
    ) -> Result<[u8; 4], DisplayMappingError> {
        match self {
            Self::Neutral => Ok(NEUTRAL_COLOR),
            Self::Elevation {
                source_z_range: Some([minimum, maximum]),
            } => Ok(elevation_color(world_z, minimum, maximum)),
            Self::Elevation {
                source_z_range: None,
            } => Ok(ELEVATION_COLORS[2]),
            Self::Rgb => attributes
                .map(|attributes| rgb_color(attributes.rgb()))
                .ok_or(DisplayMappingError::MissingAttributes(DisplayMode::Rgb)),
            Self::Intensity => attributes
                .map(|attributes| intensity_color(attributes.intensity()))
                .ok_or(DisplayMappingError::MissingAttributes(
                    DisplayMode::Intensity,
                )),
            Self::Classification => attributes
                .map(|attributes| classification_color(attributes.classification()))
                .ok_or(DisplayMappingError::MissingAttributes(
                    DisplayMode::Classification,
                )),
        }
    }
}

/// Scales raw unsigned 16-bit RGB with round-to-nearest conversion.
#[must_use]
pub fn rgb_color([red, green, blue]: [u16; 3]) -> [u8; 4] {
    [u16_to_u8(red), u16_to_u8(green), u16_to_u8(blue), 255]
}

/// Scales raw unsigned 16-bit intensity to opaque grayscale.
#[must_use]
pub fn intensity_color(intensity: u16) -> [u8; 4] {
    let intensity = u16_to_u8(intensity);
    [intensity, intensity, intensity, 255]
}

fn u16_to_u8(value: u16) -> u8 {
    let rounded = (u32::from(value) * 255 + 32_767) / 65_535;
    u8::try_from(rounded).expect("scaled U16 is in the inclusive U8 range")
}

/// Maps every raw unsigned 8-bit classification to one fixed opaque color.
#[must_use]
pub fn classification_color(classification: u8) -> [u8; 4] {
    const STANDARD: [[u8; 4]; 19] = [
        [120, 120, 120, 255],
        [155, 155, 155, 255],
        [139, 95, 57, 255],
        [80, 180, 80, 255],
        [45, 150, 45, 255],
        [20, 110, 20, 255],
        [220, 70, 70, 255],
        [200, 200, 200, 255],
        [170, 120, 220, 255],
        [80, 150, 230, 255],
        [60, 100, 210, 255],
        [40, 180, 210, 255],
        [230, 170, 60, 255],
        [220, 120, 40, 255],
        [235, 80, 150, 255],
        [170, 70, 170, 255],
        [255, 220, 80, 255],
        [100, 220, 190, 255],
        [245, 245, 245, 255],
    ];
    STANDARD
        .get(usize::from(classification))
        .copied()
        .unwrap_or_else(|| {
            let value = classification;
            [
                value.wrapping_mul(73).wrapping_add(41),
                value.wrapping_mul(151).wrapping_add(97),
                value.wrapping_mul(199).wrapping_add(17),
                255,
            ]
        })
}

fn elevation_color(world_z: f64, minimum: f64, maximum: f64) -> [u8; 4] {
    let normalized = normalize_elevation(world_z, minimum, maximum);
    let palette_position = normalized * 4.0;
    if palette_position >= 4.0 {
        return ELEVATION_COLORS[4];
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lower_index = palette_position.floor() as usize;
    let local_position = palette_position.fract();
    interpolate_color(
        ELEVATION_COLORS[lower_index],
        ELEVATION_COLORS[lower_index + 1],
        local_position,
    )
}

fn normalize_elevation(world_z: f64, minimum: f64, maximum: f64) -> f64 {
    if minimum >= maximum {
        return 0.5;
    }
    if world_z <= minimum {
        return 0.0;
    }
    if world_z >= maximum {
        return 1.0;
    }

    let midpoint = minimum / 2.0 + maximum / 2.0;
    if world_z <= midpoint {
        0.5 * ((world_z - minimum) / (midpoint - minimum))
    } else {
        0.5 + 0.5 * ((world_z - midpoint) / (maximum - midpoint))
    }
}

fn interpolate_color(lower: [u8; 4], upper: [u8; 4], position: f64) -> [u8; 4] {
    std::array::from_fn(|channel| {
        let lower = f64::from(lower[channel]);
        let upper = f64::from(upper[channel]);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let value = (lower + (upper - lower) * position).round() as u8;
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_attributed_modes_require_inspection_rows() {
        for mode in [DisplayMode::Neutral, DisplayMode::Elevation] {
            assert!(!PointColorizer::for_source(mode, None).requires_attributes());
        }
        for mode in [
            DisplayMode::Rgb,
            DisplayMode::Intensity,
            DisplayMode::Classification,
        ] {
            assert!(PointColorizer::for_source(mode, None).requires_attributes());
        }
    }

    #[test]
    fn display_index_policy_keeps_recipe_suffix_and_versions_together() {
        let position_only = DisplayMode::Elevation.index_policy();
        assert_eq!(position_only.target_suffix(), ".pidx");
        assert_eq!(position_only.versions(), (1, 1));
        assert!(matches!(
            position_only.recipe(),
            IndexRecipe::PositionOnlyV1
        ));

        let inspection = DisplayMode::Classification.index_policy();
        assert_eq!(inspection.target_suffix(), ".inspection-v2.pidx");
        assert_eq!(inspection.versions(), (2, 2));
        assert!(matches!(inspection.recipe(), IndexRecipe::InspectionV1(_)));
    }

    #[test]
    fn neutral_color_does_not_depend_on_source_elevation() {
        let bounds = WorldBounds::new([-10.0; 3], [10.0; 3]).unwrap();
        let colorizer = PointColorizer::for_source(DisplayMode::Neutral, Some(bounds));

        assert_eq!(colorizer.color(-1_000.0, None).unwrap(), NEUTRAL_COLOR);
        assert_eq!(colorizer.color(1_000.0, None).unwrap(), NEUTRAL_COLOR);
    }

    #[test]
    fn elevation_color_uses_clamped_source_world_z_bounds() {
        let bounds = WorldBounds::new([0.0, 0.0, 100.0], [1.0, 1.0, 200.0]).unwrap();
        let colorizer = PointColorizer::for_source(DisplayMode::Elevation, Some(bounds));

        assert_eq!(colorizer.color(0.0, None).unwrap(), ELEVATION_COLORS[0]);
        assert_eq!(colorizer.color(100.0, None).unwrap(), ELEVATION_COLORS[0]);
        assert_eq!(colorizer.color(125.0, None).unwrap(), ELEVATION_COLORS[1]);
        assert_eq!(colorizer.color(150.0, None).unwrap(), ELEVATION_COLORS[2]);
        assert_eq!(colorizer.color(175.0, None).unwrap(), ELEVATION_COLORS[3]);
        assert_eq!(colorizer.color(200.0, None).unwrap(), ELEVATION_COLORS[4]);
        assert_eq!(colorizer.color(300.0, None).unwrap(), ELEVATION_COLORS[4]);
    }

    #[test]
    fn elevation_color_interpolates_rgba8_channels_deterministically() {
        assert_eq!(elevation_color(12.5, 0.0, 100.0), [64, 42, 112, 255]);
    }

    #[test]
    fn elevation_normalization_stays_finite_across_the_full_f64_range() {
        assert_eq!(
            elevation_color(-f64::MAX / 2.0, -f64::MAX, f64::MAX),
            ELEVATION_COLORS[1]
        );
        assert_eq!(
            elevation_color(0.0, -f64::MAX, f64::MAX),
            ELEVATION_COLORS[2]
        );
        assert_eq!(
            elevation_color(f64::MAX / 2.0, -f64::MAX, f64::MAX),
            ELEVATION_COLORS[3]
        );
    }

    #[test]
    fn flat_and_empty_sources_have_a_stable_midpoint_color() {
        let flat_bounds = WorldBounds::new([0.0, 0.0, 42.0], [1.0, 1.0, 42.0]).unwrap();
        let flat = PointColorizer::for_source(DisplayMode::Elevation, Some(flat_bounds));
        let empty = PointColorizer::for_source(DisplayMode::Elevation, None);

        assert_eq!(flat.color(42.0, None).unwrap(), ELEVATION_COLORS[2]);
        assert_eq!(empty.color(42.0, None).unwrap(), ELEVATION_COLORS[2]);
    }

    #[test]
    fn attributed_modes_reject_missing_rows_instead_of_using_neutral_color() {
        for (mode, expected) in [
            (
                DisplayMode::Rgb,
                DisplayMappingError::MissingAttributes(DisplayMode::Rgb),
            ),
            (
                DisplayMode::Intensity,
                DisplayMappingError::MissingAttributes(DisplayMode::Intensity),
            ),
            (
                DisplayMode::Classification,
                DisplayMappingError::MissingAttributes(DisplayMode::Classification),
            ),
        ] {
            assert_eq!(
                PointColorizer::for_source(mode, None).color(0.0, None),
                Err(expected)
            );
        }
    }

    #[test]
    fn u16_channels_scale_to_rgba8_with_exact_rounding() {
        assert_eq!(u16_to_u8(0), 0);
        assert_eq!(u16_to_u8(32_767), 127);
        assert_eq!(u16_to_u8(32_768), 128);
        assert_eq!(u16_to_u8(u16::MAX), u8::MAX);
        assert_eq!(rgb_color([0, 32_768, u16::MAX]), [0, 128, 255, 255]);
        assert_eq!(intensity_color(32_768), [128, 128, 128, 255]);
    }

    #[test]
    fn every_u8_classification_matches_the_exact_mapping_oracle() {
        let standard = [
            [120, 120, 120, 255],
            [155, 155, 155, 255],
            [139, 95, 57, 255],
            [80, 180, 80, 255],
            [45, 150, 45, 255],
            [20, 110, 20, 255],
            [220, 70, 70, 255],
            [200, 200, 200, 255],
            [170, 120, 220, 255],
            [80, 150, 230, 255],
            [60, 100, 210, 255],
            [40, 180, 210, 255],
            [230, 170, 60, 255],
            [220, 120, 40, 255],
            [235, 80, 150, 255],
            [170, 70, 170, 255],
            [255, 220, 80, 255],
            [100, 220, 190, 255],
            [245, 245, 245, 255],
        ];
        for (classification, expected) in (u8::MIN..=18).zip(standard) {
            assert_eq!(classification_color(classification), expected);
        }

        for classification in 19..=u8::MAX {
            let channel = |factor: u16, offset: u16| {
                u8::try_from((u16::from(classification) * factor + offset) % 256).unwrap()
            };
            assert_eq!(
                classification_color(classification),
                [channel(73, 41), channel(151, 97), channel(199, 17), 255]
            );
        }
    }
}
