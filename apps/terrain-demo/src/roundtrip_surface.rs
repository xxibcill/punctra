//! Semantic TIN construction and exact face validation for `LandXML` readers.

use std::collections::BTreeMap;

use num_bigint::BigInt;
use point_contracts::SpatialReferenceProfile;
use robust::{Coord, orient2d};

use crate::roundtrip::{InputSide, RoundTripFailure, RoundTripLimits, RoundTripReason};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Position {
    pub(crate) easting: f64,
    pub(crate) northing: f64,
    pub(crate) elevation: f64,
}

impl Position {
    pub(crate) fn from_landxml(
        side: InputSide,
        northing: f64,
        easting: f64,
        elevation: f64,
    ) -> Result<Self, RoundTripFailure> {
        if [northing, easting, elevation]
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} P coordinates must be finite"),
            ));
        }
        Ok(Self {
            easting: canonical_zero(easting),
            northing: canonical_zero(northing),
            elevation: canonical_zero(elevation),
        })
    }

    pub(crate) fn key(self) -> [u64; 3] {
        [
            self.easting.to_bits(),
            self.northing.to_bits(),
            self.elevation.to_bits(),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Triangle {
    first: usize,
    second: usize,
    third: usize,
}

impl Triangle {
    const fn new(first: usize, second: usize, third: usize) -> Self {
        Self {
            first,
            second,
            third,
        }
    }

    const fn has_repeated_point(self) -> bool {
        self.first == self.second || self.second == self.third || self.first == self.third
    }

    fn positions(self, points: &[Position]) -> [Position; 3] {
        [points[self.first], points[self.second], points[self.third]]
    }

    pub(crate) fn canonical_point_indices(self) -> [usize; 3] {
        let mut indices = [self.first, self.second, self.third];
        indices.sort_unstable();
        indices
    }

    pub(crate) fn remap(self, point_mapping: &[usize]) -> Self {
        Self::new(
            point_mapping[self.first],
            point_mapping[self.second],
            point_mapping[self.third],
        )
    }
}

#[derive(Debug)]
pub(crate) struct ParsedSurface {
    pub(crate) points: Vec<Position>,
    pub(crate) faces: Vec<Triangle>,
    pub(crate) surface_name: Option<Box<str>>,
    pub(crate) ignored_top_level_sections: Box<[Box<str>]>,
    pub(crate) spatial_reference_profile: Option<SpatialReferenceProfile>,
}

pub(crate) struct SemanticSurfaceBuilder {
    limits: RoundTripLimits,
    points: Vec<Position>,
    point_ids: BTreeMap<u64, usize>,
    faces: Vec<Triangle>,
}

impl SemanticSurfaceBuilder {
    pub(crate) const fn new(limits: RoundTripLimits) -> Self {
        Self {
            limits,
            points: Vec::new(),
            point_ids: BTreeMap::new(),
            faces: Vec::new(),
        }
    }

    pub(crate) fn reserve_points(
        &mut self,
        side: InputSide,
        point_count: usize,
    ) -> Result<(), RoundTripFailure> {
        check_item_limit(side, "points", point_count, self.limits.points())?;
        self.points.try_reserve_exact(point_count).map_err(|_| {
            RoundTripFailure::resource(format_args!(
                "{side} point storage cannot reserve {point_count} entries"
            ))
        })
    }

    pub(crate) fn reserve_faces(
        &mut self,
        side: InputSide,
        face_count: usize,
    ) -> Result<(), RoundTripFailure> {
        check_item_limit(side, "faces", face_count, self.limits.faces())?;
        self.faces.try_reserve_exact(face_count).map_err(|_| {
            RoundTripFailure::resource(format_args!(
                "{side} face storage cannot reserve {face_count} entries"
            ))
        })
    }

    pub(crate) fn add_point(
        &mut self,
        side: InputSide,
        id: u64,
        position: Position,
    ) -> Result<(), RoundTripFailure> {
        if self.points.len() as u64 >= self.limits.points() {
            return Err(RoundTripFailure::resource(format_args!(
                "{side} points exceed the {} point limit",
                self.limits.points()
            )));
        }
        if id == 0 {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} point ID must be positive"),
            ));
        }
        let index = self.points.len();
        if self.point_ids.insert(id, index).is_some() {
            return Err(RoundTripFailure::semantic(
                RoundTripReason::XmlInvalid,
                format_args!("{side} contains duplicate point ID {id}"),
            ));
        }
        self.points.push(position);
        Ok(())
    }

    pub(crate) fn add_face(
        &mut self,
        side: InputSide,
        point_ids: [u64; 3],
    ) -> Result<(), RoundTripFailure> {
        if self.faces.len() as u64 >= self.limits.faces() {
            return Err(RoundTripFailure::resource(format_args!(
                "{side} faces exceed the {} face limit",
                self.limits.faces()
            )));
        }
        let resolve = |id| {
            self.point_ids.get(&id).copied().ok_or_else(|| {
                RoundTripFailure::semantic(
                    RoundTripReason::XmlInvalid,
                    format_args!("{side} face has dangling point reference {id}"),
                )
            })
        };
        let [first, second, third] = point_ids;
        let face = Triangle::new(resolve(first)?, resolve(second)?, resolve(third)?);
        validate_face(side, face, &self.points)?;
        self.faces.push(face);
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        side: InputSide,
    ) -> Result<(Vec<Position>, Vec<Triangle>), RoundTripFailure> {
        if self.points.len() < 3 || self.faces.is_empty() {
            return Err(subset_error(
                side,
                "TIN requires at least three points and one face",
            ));
        }
        reject_duplicate_faces(side, &mut self.faces)?;
        Ok((self.points, self.faces))
    }
}

fn validate_face(
    side: InputSide,
    face: Triangle,
    points: &[Position],
) -> Result<(), RoundTripFailure> {
    if face.has_repeated_point() {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} contains a face with repeated point references"),
        ));
    }
    let [a, b, c] = face.positions(points);
    let robust_orientation = normalized_orientation_xy(a, b, c);
    let is_collinear = match robust_orientation {
        Some(orientation) if orientation != 0.0 => false,
        Some(_) | None => exact_orientation_is_zero(a, b, c),
    };
    if is_collinear {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::XmlInvalid,
            format_args!("{side} contains a geometrically degenerate face"),
        ));
    }
    Ok(())
}

fn reject_duplicate_faces(side: InputSide, faces: &mut [Triangle]) -> Result<(), RoundTripFailure> {
    faces.sort_unstable_by_key(|face| face.canonical_point_indices());
    if faces
        .windows(2)
        .any(|pair| pair[0].canonical_point_indices() == pair[1].canonical_point_indices())
    {
        return Err(RoundTripFailure::semantic(
            RoundTripReason::TopologyDrift,
            format_args!("{side} contains duplicate faces"),
        ));
    }
    Ok(())
}

fn normalized_orientation_xy(a: Position, b: Position, c: Position) -> Option<f64> {
    let [ax, bx, cx] = scale_axis_exact([a.easting, b.easting, c.easting])?;
    let [ay, by, cy] = scale_axis_exact([a.northing, b.northing, c.northing])?;
    let orientation = orient2d(
        Coord { x: ax, y: ay },
        Coord { x: bx, y: by },
        Coord { x: cx, y: cy },
    );
    orientation.is_finite().then_some(orientation)
}

fn scale_axis_exact(values: [f64; 3]) -> Option<[f64; 3]> {
    let maximum = values
        .into_iter()
        .fold(0.0_f64, |current, value| current.max(value.abs()));
    if maximum == 0.0 {
        return Some(values);
    }
    let shift = (-binary_exponent(maximum)).clamp(-1_022, 1_023);
    let factor = normal_power_of_two(shift);
    let mut scaled = [0.0; 3];
    for (target, value) in scaled.iter_mut().zip(values) {
        *target = value * factor;
        if !target.is_finite()
            || (value != 0.0 && *target == 0.0)
            || (*target / factor).to_bits() != value.to_bits()
        {
            return None;
        }
    }
    Some(scaled)
}

fn binary_exponent(value: f64) -> i32 {
    const FRACTION_BITS: u64 = (1_u64 << 52) - 1;
    let bits = value.to_bits() & i64::MAX as u64;
    let encoded = ((bits >> 52) & 0x7ff) as i32;
    if encoded != 0 {
        encoded - 1_023
    } else {
        let Ok(highest_fraction_bit) =
            i32::try_from(63_u32.saturating_sub((bits & FRACTION_BITS).leading_zeros()))
        else {
            unreachable!("a binary64 fraction bit index fits i32");
        };
        highest_fraction_bit - 1_074
    }
}

fn normal_power_of_two(exponent: i32) -> f64 {
    debug_assert!((-1_022..=1_023).contains(&exponent));
    let Ok(encoded) = u64::try_from(exponent + 1_023) else {
        unreachable!("a validated binary64 exponent is nonnegative");
    };
    f64::from_bits(encoded << 52)
}

fn exact_orientation_is_zero(a: Position, b: Position, c: Position) -> bool {
    let (a_easting_delta, a_easting_exponent) = exact_difference(a.easting, c.easting);
    let (a_northing_delta, a_northing_exponent) = exact_difference(a.northing, c.northing);
    let (b_easting_delta, b_easting_exponent) = exact_difference(b.easting, c.easting);
    let (b_northing_delta, b_northing_exponent) = exact_difference(b.northing, c.northing);
    let left = a_easting_delta * b_northing_delta;
    let right = a_northing_delta * b_easting_delta;
    exact_scaled_integers_equal(
        left,
        a_easting_exponent + b_northing_exponent,
        right,
        a_northing_exponent + b_easting_exponent,
    )
}

fn exact_difference(left: f64, right: f64) -> (BigInt, i32) {
    let (left_significand, left_exponent) = exact_dyadic(left);
    let (right_significand, right_exponent) = exact_dyadic(right);
    let exponent = left_exponent.min(right_exponent);
    let left_shift = nonnegative_shift(left_exponent - exponent);
    let right_shift = nonnegative_shift(right_exponent - exponent);
    (
        (left_significand << left_shift) - (right_significand << right_shift),
        exponent,
    )
}

fn exact_dyadic(value: f64) -> (BigInt, i32) {
    const FRACTION_BITS: u64 = (1_u64 << 52) - 1;
    const SIGN_BIT: u64 = 1_u64 << 63;
    let bits = value.to_bits();
    let Ok(encoded_exponent) = i32::try_from((bits >> 52) & 0x7ff) else {
        unreachable!("a binary64 encoded exponent fits i32");
    };
    let fraction = bits & FRACTION_BITS;
    let (significand, exponent) = if encoded_exponent == 0 {
        (fraction, -1_074)
    } else {
        ((1_u64 << 52) | fraction, encoded_exponent - 1_023 - 52)
    };
    let significand = BigInt::from(significand);
    if bits & SIGN_BIT == 0 {
        (significand, exponent)
    } else {
        (-significand, exponent)
    }
}

fn exact_scaled_integers_equal(
    left: BigInt,
    left_exponent: i32,
    right: BigInt,
    right_exponent: i32,
) -> bool {
    if left_exponent == right_exponent {
        return left == right;
    }
    if left_exponent < right_exponent {
        let shift = nonnegative_shift(right_exponent - left_exponent);
        left == right << shift
    } else {
        let shift = nonnegative_shift(left_exponent - right_exponent);
        left << shift == right
    }
}

fn nonnegative_shift(value: i32) -> usize {
    let Ok(value) = usize::try_from(value) else {
        unreachable!("an exponent difference is nonnegative");
    };
    value
}

fn check_item_limit(
    side: InputSide,
    item: &str,
    actual: usize,
    allowed: u64,
) -> Result<(), RoundTripFailure> {
    if actual as u64 > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} {item} required {actual}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn subset_error(side: InputSide, message: &'static str) -> RoundTripFailure {
    RoundTripFailure::semantic(
        RoundTripReason::SubsetUnsupported,
        format_args!("{side} schema is unsupported: {message}"),
    )
}

pub(crate) const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}
