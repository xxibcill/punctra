use std::{fmt, str::FromStr};

use serde::Serialize;
use thiserror::Error;

const NEUTRAL_COLOR: [u8; 4] = [190, 205, 220, 255];
const ELEVATION_COLORS: [[u8; 4]; 5] = [
    [68, 1, 84, 255],
    [59, 82, 139, 255],
    [33, 145, 140, 255],
    [94, 201, 98, 255],
    [253, 231, 37, 255],
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DisplayMode {
    Neutral,
    Elevation,
    #[default]
    Rgb,
    Intensity,
    Classification,
}

impl DisplayMode {
    pub(crate) const ALL: [Self; 5] = [
        Self::Neutral,
        Self::Elevation,
        Self::Rgb,
        Self::Intensity,
        Self::Classification,
    ];

    pub(crate) fn color(
        self,
        world_z: f64,
        source_z_range: [f64; 2],
        intensity: u16,
        classification: u8,
        rgb: [u16; 3],
    ) -> [u8; 4] {
        match self {
            Self::Neutral => NEUTRAL_COLOR,
            Self::Elevation => elevation_color(world_z, source_z_range),
            Self::Rgb => rgb_color(rgb),
            Self::Intensity => intensity_color(intensity),
            Self::Classification => classification_color(classification),
        }
    }
}

impl fmt::Display for DisplayMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Neutral => "neutral",
            Self::Elevation => "elevation",
            Self::Rgb => "rgb",
            Self::Intensity => "intensity",
            Self::Classification => "classification",
        })
    }
}

impl FromStr for DisplayMode {
    type Err = DisplayModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.to_string() == value)
            .ok_or_else(|| DisplayModeError(value.to_owned()))
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
#[error("unsupported display mode {0:?}")]
pub(crate) struct DisplayModeError(String);

fn rgb_color([red, green, blue]: [u16; 3]) -> [u8; 4] {
    [u16_to_u8(red), u16_to_u8(green), u16_to_u8(blue), 255]
}

fn intensity_color(intensity: u16) -> [u8; 4] {
    let intensity = u16_to_u8(intensity);
    [intensity, intensity, intensity, 255]
}

fn u16_to_u8(value: u16) -> u8 {
    let rounded = (u32::from(value) * 255 + 32_767) / 65_535;
    u8::try_from(rounded).expect("scaled U16 is in the inclusive U8 range")
}

fn classification_color(classification: u8) -> [u8; 4] {
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
    if classification < 19 {
        STANDARD[classification as usize]
    } else {
        [
            classification.wrapping_mul(73).wrapping_add(41),
            classification.wrapping_mul(151).wrapping_add(97),
            classification.wrapping_mul(199).wrapping_add(17),
            255,
        ]
    }
}

fn elevation_color(world_z: f64, [minimum, maximum]: [f64; 2]) -> [u8; 4] {
    let normalized = normalize_elevation(world_z, minimum, maximum);
    let palette_position = normalized * 4.0;
    if palette_position >= 4.0 {
        return ELEVATION_COLORS[4];
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lower_index = palette_position.floor() as usize;
    interpolate_color(
        ELEVATION_COLORS[lower_index],
        ELEVATION_COLORS[lower_index + 1],
        palette_position.fract(),
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
        let value = lower + (upper - lower) * position;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let value = value.round() as u8;
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_display_names_round_trip() {
        for mode in DisplayMode::ALL {
            assert_eq!(mode.to_string().parse(), Ok(mode));
        }
        assert!("height".parse::<DisplayMode>().is_err());
    }

    #[test]
    fn inherited_mappings_remain_exact() {
        let source_z_range = [0.0, 100.0];
        assert_eq!(
            DisplayMode::Neutral.color(50.0, source_z_range, 0, 0, [0; 3]),
            [190, 205, 220, 255]
        );
        assert_eq!(
            DisplayMode::Elevation.color(0.0, source_z_range, 0, 0, [0; 3]),
            [68, 1, 84, 255]
        );
        assert_eq!(
            DisplayMode::Elevation.color(50.0, source_z_range, 0, 0, [0; 3]),
            [33, 145, 140, 255]
        );
        assert_eq!(
            DisplayMode::Rgb.color(0.0, source_z_range, 0, 0, [0, 32_768, 65_535]),
            [0, 128, 255, 255]
        );
        assert_eq!(
            DisplayMode::Intensity.color(0.0, source_z_range, 65_535, 0, [0; 3]),
            [255; 4]
        );
        assert_eq!(
            DisplayMode::Classification.color(0.0, source_z_range, 0, 2, [0; 3]),
            [139, 95, 57, 255]
        );
    }
}
