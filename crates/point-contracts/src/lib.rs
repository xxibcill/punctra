//! Canonical, validated Source and Point values.
//!
//! This crate defines lossless values shared by Source adapters and their
//! callers. It performs no input/output, decoding, scheduling, Workspace, View,
//! or GPU work.
//!
//! # Example
//!
//! ```
//! use point_contracts::{
//!     AttributeColumns, ContractError, PointBatch, PositionTransform,
//!     QuantizedPositions, SourceId,
//! };
//!
//! # fn main() -> Result<(), ContractError> {
//! let transform = PositionTransform::new([100.0, 200.0, 0.0], [0.01; 3])?;
//! let positions = QuantizedPositions::new(transform, vec![[0, 0, 0], [10, 20, 30]])?;
//! let batch = PointBatch::new(
//!     SourceId::new([7; 32]),
//!     0,
//!     positions,
//!     AttributeColumns::empty(2),
//! )?;
//!
//! assert_eq!(batch.len(), 2);
//! assert_eq!(batch.positions().world_f64(1), Some([100.1, 200.2, 0.3]));
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{fmt, iter::FusedIterator, marker::PhantomData, num::NonZeroU32, ops::Range};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeSeed, EnumAccess, IgnoredAny, MapAccess, SeqAccess, VariantAccess, Visitor},
    ser::{SerializeSeq, SerializeStructVariant},
};
use thiserror::Error;

/// Maximum UTF-8 bytes in one Attribute name.
///
/// One KiB accommodates descriptive adapter-owned names while keeping hostile
/// schemas from embedding arbitrarily large strings in each definition.
pub const MAX_ATTRIBUTE_NAME_BYTES: usize = 1_024;

/// Maximum number of definitions in one Attribute schema.
///
/// This is intentionally far above the dimension counts of ordinary point
/// formats while bounding schema allocation and validation work.
pub const MAX_ATTRIBUTE_DEFINITIONS: usize = 4_096;

/// Maximum UTF-8 bytes in a declared Coordinate Reference WKT value.
///
/// One MiB leaves ample room for compound reference-system definitions without
/// allowing one metadata string to grow without limit.
pub const MAX_COORDINATE_REFERENCE_WKT_BYTES: usize = 1024 * 1024;

/// Maximum UTF-8 bytes in one metadata namespace.
pub const MAX_METADATA_NAMESPACE_BYTES: usize = 256;

/// Maximum UTF-8 bytes in one metadata record name.
pub const MAX_METADATA_NAME_BYTES: usize = 1_024;

/// Maximum bytes in one opaque metadata record payload.
///
/// Sixty-four MiB supports substantial format extension records while forcing
/// larger data to use a purpose-built bounded stream rather than metadata.
pub const MAX_METADATA_RECORD_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Maximum number of ordered metadata records attached to one Source.
pub const MAX_METADATA_RECORDS: usize = 16_384;

/// Maximum combined opaque metadata payload bytes attached to one Source.
pub const MAX_SOURCE_METADATA_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

/// Maximum UTF-8 bytes in a Source format name.
pub const MAX_SOURCE_FORMAT_NAME_BYTES: usize = 256;

/// Maximum UTF-8 bytes in a Source logical-order rule.
pub const MAX_LOGICAL_ORDER_BYTES: usize = 1_024;

/// A string decoded through a byte limit before it can enter a canonical value.
struct BoundedText<const MAX_BYTES: usize>(String);

impl<const MAX_BYTES: usize> BoundedText<MAX_BYTES> {
    fn into_string(self) -> String {
        self.0
    }
}

impl<'de, const MAX_BYTES: usize> Deserialize<'de> for BoundedText<MAX_BYTES> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(BoundedTextVisitor::<MAX_BYTES>)
    }
}

struct BoundedTextVisitor<const MAX_BYTES: usize>;

impl<const MAX_BYTES: usize> Visitor<'_> for BoundedTextVisitor<MAX_BYTES> {
    type Value = BoundedText<MAX_BYTES>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a UTF-8 string of at most {MAX_BYTES} bytes")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Self::check(value.len())?;
        Ok(BoundedText(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Self::check(value.len())?;
        Ok(BoundedText(value))
    }
}

impl<const MAX_BYTES: usize> BoundedTextVisitor<MAX_BYTES> {
    fn check<E>(actual_bytes: usize) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        if actual_bytes > MAX_BYTES {
            return Err(E::custom(format_args!(
                "string is {actual_bytes} UTF-8 bytes; maximum is {MAX_BYTES}"
            )));
        }
        Ok(())
    }
}

/// Stable 256-bit identity of one immutable Source.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SourceId([u8; 32]);

impl SourceId {
    /// Creates a Source Identity from its opaque bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the opaque identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the opaque identity bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

/// Collision-resistant 256-bit hash of canonical content.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Creates a content hash from its opaque bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the opaque hash bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the opaque hash bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

fn write_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

/// Stable identity of one Point within one immutable Source.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PointId {
    source: SourceId,
    ordinal: u64,
}

impl PointId {
    /// Creates a Point Identity from its Source Identity and logical ordinal.
    #[must_use]
    pub const fn new(source: SourceId, ordinal: u64) -> Self {
        Self { source, ordinal }
    }

    /// Returns the Source Identity.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Returns the zero-based logical Source ordinal.
    #[must_use]
    pub const fn ordinal(self) -> u64 {
        self.ordinal
    }
}

/// Finite offset and positive finite scale used to decode position ticks.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PositionTransformUnchecked")]
pub struct PositionTransform {
    offset: [f64; 3],
    scale: [f64; 3],
}

#[derive(Deserialize)]
struct PositionTransformUnchecked {
    offset: [f64; 3],
    scale: [f64; 3],
}

impl PositionTransform {
    /// Creates a validated transform.
    ///
    /// # Errors
    ///
    /// Returns an error for the first non-finite offset or non-finite,
    /// non-positive scale axis.
    pub fn new(offset: [f64; 3], scale: [f64; 3]) -> Result<Self, ContractError> {
        for axis in 0..3 {
            if !offset[axis].is_finite() {
                return Err(ContractError::NonFinitePositionOffset { axis });
            }
            if !scale[axis].is_finite() || scale[axis] <= 0.0 {
                return Err(ContractError::InvalidPositionScale { axis });
            }
        }
        Ok(Self { offset, scale })
    }

    /// Returns the finite Source offset.
    #[must_use]
    pub const fn offset(self) -> [f64; 3] {
        self.offset
    }

    /// Returns the positive finite Source scale.
    #[must_use]
    pub const fn scale(self) -> [f64; 3] {
        self.scale
    }

    /// Converts exact ticks to world coordinates using Source scale and offset.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn world_f64(self, ticks: [i64; 3]) -> [f64; 3] {
        [
            ticks[0] as f64 * self.scale[0] + self.offset[0],
            ticks[1] as f64 * self.scale[1] + self.offset[1],
            ticks[2] as f64 * self.scale[2] + self.offset[2],
        ]
    }
}

impl TryFrom<PositionTransformUnchecked> for PositionTransform {
    type Error = ContractError;

    fn try_from(value: PositionTransformUnchecked) -> Result<Self, Self::Error> {
        Self::new(value.offset, value.scale)
    }
}

/// A non-empty column of exact signed position ticks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuantizedPositions {
    transform: PositionTransform,
    ticks: Box<[[i64; 3]]>,
    #[serde(skip)]
    payload_bytes: u64,
}

impl QuantizedPositions {
    /// Creates a non-empty position column without changing any ticks.
    ///
    /// # Errors
    ///
    /// Returns an error when `ticks` is empty or its payload size cannot be
    /// represented.
    pub fn new(transform: PositionTransform, ticks: Vec<[i64; 3]>) -> Result<Self, ContractError> {
        if ticks.is_empty() {
            return Err(ContractError::EmptyQuantizedPositions);
        }
        let payload_bytes = position_payload_bytes(ticks.len())?;
        Ok(Self {
            transform,
            ticks: ticks.into_boxed_slice(),
            payload_bytes,
        })
    }

    /// Returns the Source position transform.
    #[must_use]
    pub const fn transform(&self) -> PositionTransform {
        self.transform
    }

    /// Returns the exact signed ticks.
    #[must_use]
    pub fn ticks(&self) -> &[[i64; 3]] {
        &self.ticks
    }

    /// Returns the number of positions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    /// Reports whether the column is empty.
    ///
    /// A constructed value always returns `false`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }

    /// Returns one world position, or `None` when `row` is out of range.
    #[must_use]
    pub fn world_f64(&self, row: usize) -> Option<[f64; 3]> {
        self.ticks
            .get(row)
            .copied()
            .map(|ticks| self.transform.world_f64(ticks))
    }

    /// Copies a non-empty row range into another validated position column.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid or empty range.
    pub fn slice_rows(&self, rows: Range<usize>) -> Result<Self, ContractError> {
        validate_row_range(&rows, self.len())?;
        Self::new(self.transform, self.ticks[rows].to_vec())
    }

    /// Returns the exact tick payload size in bytes.
    #[must_use]
    pub const fn estimated_payload_bytes(&self) -> u64 {
        self.payload_bytes
    }
}

fn position_payload_bytes(row_count: usize) -> Result<u64, ContractError> {
    let rows = u64::try_from(row_count).map_err(|_| ContractError::PayloadSizeOverflow)?;
    rows.checked_mul(24)
        .ok_or(ContractError::PayloadSizeOverflow)
}

/// Stable nonzero identity of one Attribute in a Source schema.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttributeId(NonZeroU32);

impl AttributeId {
    /// Creates an Attribute identity.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ZeroAttributeId`] when `value` is zero.
    pub const fn new(value: u32) -> Result<Self, ContractError> {
        match NonZeroU32::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(ContractError::ZeroAttributeId),
        }
    }

    /// Returns the nonzero integer identity.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Exact storage type of one Attribute column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum AttributeDataType {
    /// Signed 8-bit integers.
    I8,
    /// Unsigned 8-bit integers.
    U8,
    /// Signed 16-bit integers.
    I16,
    /// Unsigned 16-bit integers.
    U16,
    /// Signed 32-bit integers.
    I32,
    /// Unsigned 32-bit integers.
    U32,
    /// Signed 64-bit integers.
    I64,
    /// Unsigned 64-bit integers.
    U64,
    /// IEEE 754 32-bit floating-point values.
    F32,
    /// IEEE 754 64-bit floating-point values.
    F64,
    /// Fixed-width opaque values with the given nonzero byte width.
    FixedBytes(NonZeroU32),
}

impl AttributeDataType {
    /// Creates a fixed-width opaque Attribute type.
    ///
    /// # Errors
    ///
    /// Returns an error when `width` is zero.
    pub const fn fixed_bytes(width: u32) -> Result<Self, ContractError> {
        match NonZeroU32::new(width) {
            Some(width) => Ok(Self::FixedBytes(width)),
            None => Err(ContractError::ZeroFixedBytesWidth),
        }
    }

    /// Returns the bytes occupied by one Attribute value.
    #[must_use]
    pub const fn element_bytes(self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
            Self::FixedBytes(width) => width.get(),
        }
    }
}

/// One named Attribute in a Source schema.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AttributeDefinitionUnchecked")]
pub struct AttributeDefinition {
    id: AttributeId,
    name: String,
    data_type: AttributeDataType,
}

#[derive(Deserialize)]
struct AttributeDefinitionUnchecked {
    id: AttributeId,
    name: BoundedText<MAX_ATTRIBUTE_NAME_BYTES>,
    data_type: AttributeDataType,
}

impl AttributeDefinition {
    /// Creates a named Attribute definition.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty, whitespace-only, or longer than
    /// [`MAX_ATTRIBUTE_NAME_BYTES`] UTF-8 bytes.
    pub fn new(
        id: AttributeId,
        name: impl Into<String>,
        data_type: AttributeDataType,
    ) -> Result<Self, ContractError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ContractError::EmptyAttributeName { id });
        }
        if name.len() > MAX_ATTRIBUTE_NAME_BYTES {
            return Err(ContractError::AttributeNameTooLong {
                id,
                actual_bytes: name.len(),
                max_bytes: MAX_ATTRIBUTE_NAME_BYTES,
            });
        }
        Ok(Self {
            id,
            name,
            data_type,
        })
    }

    /// Returns the stable Attribute identity.
    #[must_use]
    pub const fn id(&self) -> AttributeId {
        self.id
    }

    /// Returns the Attribute name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the exact Attribute storage type.
    #[must_use]
    pub const fn data_type(&self) -> AttributeDataType {
        self.data_type
    }
}

impl TryFrom<AttributeDefinitionUnchecked> for AttributeDefinition {
    type Error = ContractError;

    fn try_from(value: AttributeDefinitionUnchecked) -> Result<Self, Self::Error> {
        Self::new(value.id, value.name.into_string(), value.data_type)
    }
}

/// Canonically ordered, duplicate-free Attribute definitions.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AttributeSchemaUnchecked")]
pub struct AttributeSchema {
    definitions: Box<[AttributeDefinition]>,
}

#[derive(Deserialize)]
struct AttributeSchemaUnchecked {
    #[serde(deserialize_with = "deserialize_attribute_definitions")]
    definitions: Vec<AttributeDefinition>,
}

impl AttributeSchema {
    /// Sorts definitions by identity and rejects duplicates.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema exceeds
    /// [`MAX_ATTRIBUTE_DEFINITIONS`] or two definitions have the same
    /// Attribute identity.
    pub fn new(mut definitions: Vec<AttributeDefinition>) -> Result<Self, ContractError> {
        if definitions.len() > MAX_ATTRIBUTE_DEFINITIONS {
            return Err(ContractError::TooManyAttributeDefinitions {
                actual: definitions.len(),
                max: MAX_ATTRIBUTE_DEFINITIONS,
            });
        }
        definitions.sort_by_key(AttributeDefinition::id);
        reject_duplicate_definitions(&definitions)?;
        Ok(Self {
            definitions: definitions.into_boxed_slice(),
        })
    }

    /// Creates an empty Attribute schema.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            definitions: Box::new([]),
        }
    }

    /// Returns definitions in ascending Attribute identity order.
    #[must_use]
    pub fn definitions(&self) -> &[AttributeDefinition] {
        &self.definitions
    }

    /// Finds one definition by Attribute identity.
    #[must_use]
    pub fn get(&self, id: AttributeId) -> Option<&AttributeDefinition> {
        self.definitions
            .binary_search_by_key(&id, AttributeDefinition::id)
            .ok()
            .map(|index| &self.definitions[index])
    }

    /// Returns the number of Attribute definitions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// Reports whether no Attributes are defined.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}

fn deserialize_attribute_definitions<'de, D>(
    deserializer: D,
) -> Result<Vec<AttributeDefinition>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_sequence(deserializer, MAX_ATTRIBUTE_DEFINITIONS, |actual, max| {
        ContractError::TooManyAttributeDefinitions { actual, max }
    })
}

impl TryFrom<AttributeSchemaUnchecked> for AttributeSchema {
    type Error = ContractError;

    fn try_from(value: AttributeSchemaUnchecked) -> Result<Self, Self::Error> {
        Self::new(value.definitions)
    }
}

fn reject_duplicate_definitions(definitions: &[AttributeDefinition]) -> Result<(), ContractError> {
    for pair in definitions.windows(2) {
        if pair[0].id() == pair[1].id() {
            return Err(ContractError::DuplicateAttributeId { id: pair[0].id() });
        }
    }
    Ok(())
}

/// Owned, exactly typed values for one Attribute column.
#[derive(Clone, Debug, Serialize)]
pub struct AttributeValues {
    values: AttributeValuesKind,
}

#[derive(Clone, Debug)]
enum AttributeValuesKind {
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    FixedBytes { width: NonZeroU32, payload: Vec<u8> },
}

impl PartialEq for AttributeValues {
    fn eq(&self, other: &Self) -> bool {
        match (&self.values, &other.values) {
            (AttributeValuesKind::I8(left), AttributeValuesKind::I8(right)) => left == right,
            (AttributeValuesKind::U8(left), AttributeValuesKind::U8(right)) => left == right,
            (AttributeValuesKind::I16(left), AttributeValuesKind::I16(right)) => left == right,
            (AttributeValuesKind::U16(left), AttributeValuesKind::U16(right)) => left == right,
            (AttributeValuesKind::I32(left), AttributeValuesKind::I32(right)) => left == right,
            (AttributeValuesKind::U32(left), AttributeValuesKind::U32(right)) => left == right,
            (AttributeValuesKind::I64(left), AttributeValuesKind::I64(right)) => left == right,
            (AttributeValuesKind::U64(left), AttributeValuesKind::U64(right)) => left == right,
            (AttributeValuesKind::F32(left), AttributeValuesKind::F32(right)) => {
                float_bits_equal(left, right, f32::to_bits)
            }
            (AttributeValuesKind::F64(left), AttributeValuesKind::F64(right)) => {
                float_bits_equal(left, right, f64::to_bits)
            }
            (
                AttributeValuesKind::FixedBytes {
                    width: left_width,
                    payload: left_payload,
                },
                AttributeValuesKind::FixedBytes {
                    width: right_width,
                    payload: right_payload,
                },
            ) => left_width == right_width && left_payload == right_payload,
            _ => false,
        }
    }
}

impl Eq for AttributeValues {}

fn float_bits_equal<T, B>(left: &[T], right: &[T], to_bits: impl Fn(T) -> B) -> bool
where
    T: Copy,
    B: Eq,
{
    left.len() == right.len()
        && left
            .iter()
            .copied()
            .zip(right.iter().copied())
            .all(|(left, right)| to_bits(left) == to_bits(right))
}

impl Serialize for AttributeValuesKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::I8(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 0, "i8", values)
            }
            Self::U8(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 1, "u8", values)
            }
            Self::I16(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 2, "i16", values)
            }
            Self::U16(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 3, "u16", values)
            }
            Self::I32(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 4, "i32", values)
            }
            Self::U32(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 5, "u32", values)
            }
            Self::I64(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 6, "i64", values)
            }
            Self::U64(values) => {
                serializer.serialize_newtype_variant("AttributeValuesKind", 7, "u64", values)
            }
            Self::F32(values) => serializer.serialize_newtype_variant(
                "AttributeValuesKind",
                8,
                "f32_bits",
                &FloatBits32(values),
            ),
            Self::F64(values) => serializer.serialize_newtype_variant(
                "AttributeValuesKind",
                9,
                "f64_bits",
                &FloatBits64(values),
            ),
            Self::FixedBytes { width, payload } => {
                let mut state = serializer.serialize_struct_variant(
                    "AttributeValuesKind",
                    10,
                    "fixed_bytes",
                    2,
                )?;
                state.serialize_field("width", width)?;
                state.serialize_field("payload", payload)?;
                state.end()
            }
        }
    }
}

struct FloatBits32<'a>(&'a [f32]);

impl Serialize for FloatBits32<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            sequence.serialize_element(&value.to_bits())?;
        }
        sequence.end()
    }
}

struct FloatBits64<'a>(&'a [f64]);

impl Serialize for FloatBits64<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            sequence.serialize_element(&value.to_bits())?;
        }
        sequence.end()
    }
}

impl AttributeValues {
    /// Owns signed 8-bit values.
    #[must_use]
    pub fn i8(values: Vec<i8>) -> Self {
        Self::from_kind(AttributeValuesKind::I8(values))
    }

    /// Owns unsigned 8-bit values.
    #[must_use]
    pub fn u8(values: Vec<u8>) -> Self {
        Self::from_kind(AttributeValuesKind::U8(values))
    }

    /// Owns signed 16-bit values.
    #[must_use]
    pub fn i16(values: Vec<i16>) -> Self {
        Self::from_kind(AttributeValuesKind::I16(values))
    }

    /// Owns unsigned 16-bit values.
    #[must_use]
    pub fn u16(values: Vec<u16>) -> Self {
        Self::from_kind(AttributeValuesKind::U16(values))
    }

    /// Owns signed 32-bit values.
    #[must_use]
    pub fn i32(values: Vec<i32>) -> Self {
        Self::from_kind(AttributeValuesKind::I32(values))
    }

    /// Owns unsigned 32-bit values.
    #[must_use]
    pub fn u32(values: Vec<u32>) -> Self {
        Self::from_kind(AttributeValuesKind::U32(values))
    }

    /// Owns signed 64-bit values.
    #[must_use]
    pub fn i64(values: Vec<i64>) -> Self {
        Self::from_kind(AttributeValuesKind::I64(values))
    }

    /// Owns unsigned 64-bit values.
    #[must_use]
    pub fn u64(values: Vec<u64>) -> Self {
        Self::from_kind(AttributeValuesKind::U64(values))
    }

    /// Owns IEEE 754 32-bit values without coercion.
    #[must_use]
    pub fn f32(values: Vec<f32>) -> Self {
        Self::from_kind(AttributeValuesKind::F32(values))
    }

    /// Owns IEEE 754 64-bit values without coercion.
    #[must_use]
    pub fn f64(values: Vec<f64>) -> Self {
        Self::from_kind(AttributeValuesKind::F64(values))
    }

    /// Owns fixed-width opaque values.
    ///
    /// # Errors
    ///
    /// Returns an error when `width` is zero or the payload does not contain a
    /// whole number of values.
    pub fn fixed_bytes(width: u32, payload: Vec<u8>) -> Result<Self, ContractError> {
        let width = NonZeroU32::new(width).ok_or(ContractError::ZeroFixedBytesWidth)?;
        Self::from_fixed_bytes(width, payload)
    }

    fn from_fixed_bytes(width: NonZeroU32, payload: Vec<u8>) -> Result<Self, ContractError> {
        let width_usize =
            usize::try_from(width.get()).map_err(|_| ContractError::PayloadSizeOverflow)?;
        if !payload.len().is_multiple_of(width_usize) {
            return Err(ContractError::InvalidFixedBytesLength {
                width: width.get(),
                payload_bytes: payload.len(),
            });
        }
        Ok(Self::from_kind(AttributeValuesKind::FixedBytes {
            width,
            payload,
        }))
    }

    /// Decodes the exact serde wire representation under a caller-owned
    /// payload budget.
    ///
    /// Floating-point payloads are represented as `f32_bits` arrays of `u32`
    /// and `f64_bits` arrays of `u64`, preserving every IEEE 754 bit. Bulk
    /// canonical values deliberately do not implement [`Deserialize`], because
    /// their allocation budget belongs to the read boundary.
    ///
    /// # Errors
    ///
    /// Returns a deserializer error when the wire value is malformed, a
    /// fixed-width payload is invalid, or the decoded payload would exceed
    /// `max_payload_bytes`.
    pub fn deserialize_bounded<'de, D>(
        deserializer: D,
        max_payload_bytes: u64,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_struct(
            "AttributeValues",
            &["values"],
            AttributeValuesVisitor { max_payload_bytes },
        )
    }

    const fn from_kind(values: AttributeValuesKind) -> Self {
        Self { values }
    }

    /// Returns the exact Attribute storage type.
    #[must_use]
    pub const fn data_type(&self) -> AttributeDataType {
        match &self.values {
            AttributeValuesKind::I8(_) => AttributeDataType::I8,
            AttributeValuesKind::U8(_) => AttributeDataType::U8,
            AttributeValuesKind::I16(_) => AttributeDataType::I16,
            AttributeValuesKind::U16(_) => AttributeDataType::U16,
            AttributeValuesKind::I32(_) => AttributeDataType::I32,
            AttributeValuesKind::U32(_) => AttributeDataType::U32,
            AttributeValuesKind::I64(_) => AttributeDataType::I64,
            AttributeValuesKind::U64(_) => AttributeDataType::U64,
            AttributeValuesKind::F32(_) => AttributeDataType::F32,
            AttributeValuesKind::F64(_) => AttributeDataType::F64,
            AttributeValuesKind::FixedBytes { width, .. } => AttributeDataType::FixedBytes(*width),
        }
    }

    /// Returns the number of Attribute rows.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.values {
            AttributeValuesKind::I8(values) => values.len(),
            AttributeValuesKind::U8(values) => values.len(),
            AttributeValuesKind::I16(values) => values.len(),
            AttributeValuesKind::U16(values) => values.len(),
            AttributeValuesKind::I32(values) => values.len(),
            AttributeValuesKind::U32(values) => values.len(),
            AttributeValuesKind::I64(values) => values.len(),
            AttributeValuesKind::U64(values) => values.len(),
            AttributeValuesKind::F32(values) => values.len(),
            AttributeValuesKind::F64(values) => values.len(),
            AttributeValuesKind::FixedBytes { width, payload } => {
                let Ok(width) = usize::try_from(width.get()) else {
                    return 0;
                };
                payload.len() / width
            }
        }
    }

    /// Reports whether the column has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the encoded payload size without container overhead.
    #[must_use]
    pub fn payload_bytes(&self) -> u64 {
        match &self.values {
            AttributeValuesKind::I8(values) => vector_payload_bytes::<i8>(values.len()),
            AttributeValuesKind::U8(values) => vector_payload_bytes::<u8>(values.len()),
            AttributeValuesKind::I16(values) => vector_payload_bytes::<i16>(values.len()),
            AttributeValuesKind::U16(values) => vector_payload_bytes::<u16>(values.len()),
            AttributeValuesKind::I32(values) => vector_payload_bytes::<i32>(values.len()),
            AttributeValuesKind::U32(values) => vector_payload_bytes::<u32>(values.len()),
            AttributeValuesKind::I64(values) => vector_payload_bytes::<i64>(values.len()),
            AttributeValuesKind::U64(values) => vector_payload_bytes::<u64>(values.len()),
            AttributeValuesKind::F32(values) => vector_payload_bytes::<f32>(values.len()),
            AttributeValuesKind::F64(values) => vector_payload_bytes::<f64>(values.len()),
            AttributeValuesKind::FixedBytes { payload, .. } => {
                u64::try_from(payload.len()).unwrap_or(u64::MAX)
            }
        }
    }

    /// Copies a row range while preserving the exact Attribute type.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid row range.
    pub fn slice_rows(&self, rows: Range<usize>) -> Result<Self, ContractError> {
        validate_row_range_allow_empty(&rows, self.len())?;
        match &self.values {
            AttributeValuesKind::I8(values) => Ok(Self::i8(values[rows].to_vec())),
            AttributeValuesKind::U8(values) => Ok(Self::u8(values[rows].to_vec())),
            AttributeValuesKind::I16(values) => Ok(Self::i16(values[rows].to_vec())),
            AttributeValuesKind::U16(values) => Ok(Self::u16(values[rows].to_vec())),
            AttributeValuesKind::I32(values) => Ok(Self::i32(values[rows].to_vec())),
            AttributeValuesKind::U32(values) => Ok(Self::u32(values[rows].to_vec())),
            AttributeValuesKind::I64(values) => Ok(Self::i64(values[rows].to_vec())),
            AttributeValuesKind::U64(values) => Ok(Self::u64(values[rows].to_vec())),
            AttributeValuesKind::F32(values) => Ok(Self::f32(values[rows].to_vec())),
            AttributeValuesKind::F64(values) => Ok(Self::f64(values[rows].to_vec())),
            AttributeValuesKind::FixedBytes { width, payload } => {
                let width_usize =
                    usize::try_from(width.get()).map_err(|_| ContractError::PayloadSizeOverflow)?;
                let start = rows
                    .start
                    .checked_mul(width_usize)
                    .ok_or(ContractError::PayloadSizeOverflow)?;
                let end = rows
                    .end
                    .checked_mul(width_usize)
                    .ok_or(ContractError::PayloadSizeOverflow)?;
                Self::from_fixed_bytes(*width, payload[start..end].to_vec())
            }
        }
    }

    /// Returns signed 8-bit values when this column has that type.
    #[must_use]
    pub fn as_i8(&self) -> Option<&[i8]> {
        match &self.values {
            AttributeValuesKind::I8(values) => Some(values),
            _ => None,
        }
    }

    /// Returns unsigned 8-bit values when this column has that type.
    #[must_use]
    pub fn as_u8(&self) -> Option<&[u8]> {
        match &self.values {
            AttributeValuesKind::U8(values) => Some(values),
            _ => None,
        }
    }

    /// Returns signed 16-bit values when this column has that type.
    #[must_use]
    pub fn as_i16(&self) -> Option<&[i16]> {
        match &self.values {
            AttributeValuesKind::I16(values) => Some(values),
            _ => None,
        }
    }

    /// Returns unsigned 16-bit values when this column has that type.
    #[must_use]
    pub fn as_u16(&self) -> Option<&[u16]> {
        match &self.values {
            AttributeValuesKind::U16(values) => Some(values),
            _ => None,
        }
    }

    /// Returns signed 32-bit values when this column has that type.
    #[must_use]
    pub fn as_i32(&self) -> Option<&[i32]> {
        match &self.values {
            AttributeValuesKind::I32(values) => Some(values),
            _ => None,
        }
    }

    /// Returns unsigned 32-bit values when this column has that type.
    #[must_use]
    pub fn as_u32(&self) -> Option<&[u32]> {
        match &self.values {
            AttributeValuesKind::U32(values) => Some(values),
            _ => None,
        }
    }

    /// Returns signed 64-bit values when this column has that type.
    #[must_use]
    pub fn as_i64(&self) -> Option<&[i64]> {
        match &self.values {
            AttributeValuesKind::I64(values) => Some(values),
            _ => None,
        }
    }

    /// Returns unsigned 64-bit values when this column has that type.
    #[must_use]
    pub fn as_u64(&self) -> Option<&[u64]> {
        match &self.values {
            AttributeValuesKind::U64(values) => Some(values),
            _ => None,
        }
    }

    /// Returns 32-bit floating values when this column has that type.
    #[must_use]
    pub fn as_f32(&self) -> Option<&[f32]> {
        match &self.values {
            AttributeValuesKind::F32(values) => Some(values),
            _ => None,
        }
    }

    /// Returns 64-bit floating values when this column has that type.
    #[must_use]
    pub fn as_f64(&self) -> Option<&[f64]> {
        match &self.values {
            AttributeValuesKind::F64(values) => Some(values),
            _ => None,
        }
    }

    /// Returns the width and payload when this column contains fixed bytes.
    #[must_use]
    pub fn as_fixed_bytes(&self) -> Option<(u32, &[u8])> {
        match &self.values {
            AttributeValuesKind::FixedBytes { width, payload } => Some((width.get(), payload)),
            _ => None,
        }
    }
}

struct AttributeValuesVisitor {
    max_payload_bytes: u64,
}

impl<'de> Visitor<'de> for AttributeValuesVisitor {
    type Value = AttributeValues;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an AttributeValues wire object")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let values = sequence
            .next_element_seed(AttributeValuesKindSeed {
                max_payload_bytes: self.max_payload_bytes,
            })?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(serde::de::Error::invalid_length(2, &self));
        }
        Ok(values)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = None;
        while let Some(field) = map.next_key::<AttributeValuesField>()? {
            match field {
                AttributeValuesField::Values => {
                    if values.is_some() {
                        return Err(serde::de::Error::duplicate_field("values"));
                    }
                    values = Some(map.next_value_seed(AttributeValuesKindSeed {
                        max_payload_bytes: self.max_payload_bytes,
                    })?);
                }
                AttributeValuesField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        values.ok_or_else(|| serde::de::Error::missing_field("values"))
    }
}

enum AttributeValuesField {
    Values,
    Other,
}

impl<'de> Deserialize<'de> for AttributeValuesField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(AttributeValuesFieldVisitor)
    }
}

struct AttributeValuesFieldVisitor;

impl Visitor<'_> for AttributeValuesFieldVisitor {
    type Value = AttributeValuesField;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("`values`")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(if value == "values" {
            AttributeValuesField::Values
        } else {
            AttributeValuesField::Other
        })
    }
}

struct AttributeValuesKindSeed {
    max_payload_bytes: u64,
}

impl<'de> DeserializeSeed<'de> for AttributeValuesKindSeed {
    type Value = AttributeValues;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_enum(
            "AttributeValuesKind",
            ATTRIBUTE_VALUE_VARIANTS,
            AttributeValuesKindVisitor {
                max_payload_bytes: self.max_payload_bytes,
            },
        )
    }
}

const ATTRIBUTE_VALUE_VARIANTS: &[&str] = &[
    "i8",
    "u8",
    "i16",
    "u16",
    "i32",
    "u32",
    "i64",
    "u64",
    "f32_bits",
    "f64_bits",
    "fixed_bytes",
];

struct AttributeValuesKindVisitor {
    max_payload_bytes: u64,
}

impl<'de> Visitor<'de> for AttributeValuesKindVisitor {
    type Value = AttributeValues;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an exact Attribute value variant")
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (kind, variant) = data.variant::<AttributeValuesVariant>()?;
        let max = self.max_payload_bytes;
        match kind {
            AttributeValuesVariant::I8 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<i8>::new(max))
                .map(AttributeValues::i8),
            AttributeValuesVariant::U8 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<u8>::new(max))
                .map(AttributeValues::u8),
            AttributeValuesVariant::I16 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<i16>::new(max))
                .map(AttributeValues::i16),
            AttributeValuesVariant::U16 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<u16>::new(max))
                .map(AttributeValues::u16),
            AttributeValuesVariant::I32 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<i32>::new(max))
                .map(AttributeValues::i32),
            AttributeValuesVariant::U32 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<u32>::new(max))
                .map(AttributeValues::u32),
            AttributeValuesVariant::I64 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<i64>::new(max))
                .map(AttributeValues::i64),
            AttributeValuesVariant::U64 => variant
                .newtype_variant_seed(BoundedValueVecSeed::<u64>::new(max))
                .map(AttributeValues::u64),
            AttributeValuesVariant::F32Bits => variant
                .newtype_variant_seed(BoundedValueVecSeed::<u32>::new(max))
                .map(|bits| AttributeValues::f32(bits.into_iter().map(f32::from_bits).collect())),
            AttributeValuesVariant::F64Bits => variant
                .newtype_variant_seed(BoundedValueVecSeed::<u64>::new(max))
                .map(|bits| AttributeValues::f64(bits.into_iter().map(f64::from_bits).collect())),
            AttributeValuesVariant::FixedBytes => variant.struct_variant(
                &["width", "payload"],
                FixedBytesValuesVisitor {
                    max_payload_bytes: max,
                },
            ),
        }
    }
}

enum AttributeValuesVariant {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32Bits,
    F64Bits,
    FixedBytes,
}

impl<'de> Deserialize<'de> for AttributeValuesVariant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(AttributeValuesVariantVisitor)
    }
}

struct AttributeValuesVariantVisitor;

impl Visitor<'_> for AttributeValuesVariantVisitor {
    type Value = AttributeValuesVariant;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an Attribute value variant")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match value {
            "i8" => Ok(AttributeValuesVariant::I8),
            "u8" => Ok(AttributeValuesVariant::U8),
            "i16" => Ok(AttributeValuesVariant::I16),
            "u16" => Ok(AttributeValuesVariant::U16),
            "i32" => Ok(AttributeValuesVariant::I32),
            "u32" => Ok(AttributeValuesVariant::U32),
            "i64" => Ok(AttributeValuesVariant::I64),
            "u64" => Ok(AttributeValuesVariant::U64),
            "f32_bits" => Ok(AttributeValuesVariant::F32Bits),
            "f64_bits" => Ok(AttributeValuesVariant::F64Bits),
            "fixed_bytes" => Ok(AttributeValuesVariant::FixedBytes),
            _ => Err(serde::de::Error::unknown_variant(
                value,
                ATTRIBUTE_VALUE_VARIANTS,
            )),
        }
    }
}

struct BoundedValueVecSeed<T> {
    max_payload_bytes: u64,
    item: PhantomData<T>,
}

impl<T> BoundedValueVecSeed<T> {
    const fn new(max_payload_bytes: u64) -> Self {
        Self {
            max_payload_bytes,
            item: PhantomData,
        }
    }
}

impl<'de, T> DeserializeSeed<'de> for BoundedValueVecSeed<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let element_bytes =
            u64::try_from(std::mem::size_of::<T>()).expect("Attribute primitive width fits u64");
        let max_items_u64 = self.max_payload_bytes / element_bytes;
        let max_items = usize::try_from(max_items_u64).unwrap_or(usize::MAX);
        deserializer.deserialize_seq(BoundedValueVecVisitor::<T> {
            max_payload_bytes: self.max_payload_bytes,
            max_items,
            element_bytes,
            item: PhantomData,
        })
    }
}

struct BoundedValueVecVisitor<T> {
    max_payload_bytes: u64,
    max_items: usize,
    element_bytes: u64,
    item: PhantomData<T>,
}

impl<'de, T> Visitor<'de> for BoundedValueVecVisitor<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an Attribute payload of at most {} bytes",
            self.max_payload_bytes
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let size_hint = sequence.size_hint();
        if let Some(actual) = size_hint.filter(|actual| *actual > self.max_items) {
            return Err(attribute_payload_limit_error(
                actual,
                self.element_bytes,
                self.max_payload_bytes,
            ));
        }
        let capacity = size_hint.unwrap_or_default().min(self.max_items);
        let mut values = Vec::with_capacity(capacity);
        while values.len() < self.max_items {
            let Some(value) = sequence.next_element()? else {
                return Ok(values);
            };
            values.push(value);
        }
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(attribute_payload_limit_error(
                self.max_items.saturating_add(1),
                self.element_bytes,
                self.max_payload_bytes,
            ));
        }
        Ok(values)
    }
}

fn attribute_payload_limit_error<E>(actual_items: usize, element_bytes: u64, max_bytes: u64) -> E
where
    E: serde::de::Error,
{
    let actual_bytes = u64::try_from(actual_items)
        .unwrap_or(u64::MAX)
        .saturating_mul(element_bytes);
    E::custom(ContractError::AttributeValuesPayloadTooLong {
        actual_bytes,
        max_bytes,
    })
}

struct FixedBytesValuesVisitor {
    max_payload_bytes: u64,
}

impl<'de> Visitor<'de> for FixedBytesValuesVisitor {
    type Value = AttributeValues;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a fixed-width Attribute payload")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let width = sequence
            .next_element::<u32>()?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        let payload = sequence
            .next_element_seed(BoundedValueVecSeed::<u8>::new(self.max_payload_bytes))?
            .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
        AttributeValues::fixed_bytes(width, payload).map_err(serde::de::Error::custom)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut width = None;
        let mut payload = None;
        while let Some(field) = map.next_key::<FixedBytesField>()? {
            match field {
                FixedBytesField::Width => {
                    if width.is_some() {
                        return Err(serde::de::Error::duplicate_field("width"));
                    }
                    width = Some(map.next_value::<u32>()?);
                }
                FixedBytesField::Payload => {
                    if payload.is_some() {
                        return Err(serde::de::Error::duplicate_field("payload"));
                    }
                    payload =
                        Some(map.next_value_seed(BoundedValueVecSeed::<u8>::new(
                            self.max_payload_bytes,
                        ))?);
                }
                FixedBytesField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let width = width.ok_or_else(|| serde::de::Error::missing_field("width"))?;
        let payload = payload.ok_or_else(|| serde::de::Error::missing_field("payload"))?;
        AttributeValues::fixed_bytes(width, payload).map_err(serde::de::Error::custom)
    }
}

enum FixedBytesField {
    Width,
    Payload,
    Other,
}

impl<'de> Deserialize<'de> for FixedBytesField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(FixedBytesFieldVisitor)
    }
}

struct FixedBytesFieldVisitor;

impl Visitor<'_> for FixedBytesFieldVisitor {
    type Value = FixedBytesField;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("`width` or `payload`")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(match value {
            "width" => FixedBytesField::Width,
            "payload" => FixedBytesField::Payload,
            _ => FixedBytesField::Other,
        })
    }
}

fn vector_payload_bytes<T>(len: usize) -> u64 {
    let len = u64::try_from(len).expect("allocated vector length fits u64");
    let width = u64::try_from(std::mem::size_of::<T>()).expect("type size fits u64");
    len.checked_mul(width)
        .expect("allocated vector payload size fits u64")
}

/// One validated Attribute definition and its exactly typed values.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributeColumn {
    definition: AttributeDefinition,
    values: AttributeValues,
}

impl AttributeColumn {
    /// Creates a column whose values match its declared type.
    ///
    /// # Errors
    ///
    /// Returns an error when the value type differs from the definition.
    pub fn new(
        definition: AttributeDefinition,
        values: AttributeValues,
    ) -> Result<Self, ContractError> {
        let expected = definition.data_type();
        let actual = values.data_type();
        if expected != actual {
            return Err(ContractError::AttributeTypeMismatch {
                id: definition.id(),
                expected,
                actual,
            });
        }
        Ok(Self { definition, values })
    }

    /// Returns the Attribute identity.
    #[must_use]
    pub const fn id(&self) -> AttributeId {
        self.definition.id()
    }

    /// Returns the Attribute definition.
    #[must_use]
    pub const fn definition(&self) -> &AttributeDefinition {
        &self.definition
    }

    /// Returns the exactly typed values.
    #[must_use]
    pub const fn values(&self) -> &AttributeValues {
        &self.values
    }

    /// Returns the number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Reports whether the column has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Canonically ordered Attribute columns with one shared row count.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributeColumns {
    columns: Box<[AttributeColumn]>,
    row_count: usize,
    #[serde(skip)]
    estimated_payload_bytes: u64,
}

impl AttributeColumns {
    /// Sorts columns by Attribute identity and validates identity and row count.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate Attribute identities, a mismatched row
    /// count, or payload accounting overflow.
    pub fn new(mut columns: Vec<AttributeColumn>, row_count: usize) -> Result<Self, ContractError> {
        columns.sort_by_key(AttributeColumn::id);
        reject_duplicate_columns(&columns)?;
        validate_column_rows(&columns, row_count)?;
        let estimated_payload_bytes = attribute_payload_bytes(&columns)?;
        Ok(Self {
            columns: columns.into_boxed_slice(),
            row_count,
            estimated_payload_bytes,
        })
    }

    /// Creates zero Attribute columns for a known row count.
    #[must_use]
    pub fn empty(row_count: usize) -> Self {
        Self {
            columns: Box::new([]),
            row_count,
            estimated_payload_bytes: 0,
        }
    }

    /// Returns columns in ascending Attribute identity order.
    #[must_use]
    pub fn columns(&self) -> &[AttributeColumn] {
        &self.columns
    }

    /// Finds one column by Attribute identity.
    #[must_use]
    pub fn get(&self, id: AttributeId) -> Option<&AttributeColumn> {
        self.columns
            .binary_search_by_key(&id, AttributeColumn::id)
            .ok()
            .map(|index| &self.columns[index])
    }

    /// Returns the common row count, including when no Attributes were read.
    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Returns the number of included Attribute columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    /// Reports whether no Attribute columns are included.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Returns the sum of exact Attribute payload bytes.
    #[must_use]
    pub const fn estimated_payload_bytes(&self) -> u64 {
        self.estimated_payload_bytes
    }

    /// Copies one row range from every Attribute column.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid range or payload accounting failure.
    pub fn slice_rows(&self, rows: Range<usize>) -> Result<Self, ContractError> {
        validate_row_range_allow_empty(&rows, self.row_count)?;
        let row_count = rows.end - rows.start;
        let columns = self
            .columns
            .iter()
            .map(|column| {
                AttributeColumn::new(
                    column.definition.clone(),
                    column.values.slice_rows(rows.clone())?,
                )
            })
            .collect::<Result<Vec<_>, ContractError>>()?;
        Self::new(columns, row_count)
    }
}

fn reject_duplicate_columns(columns: &[AttributeColumn]) -> Result<(), ContractError> {
    for pair in columns.windows(2) {
        if pair[0].id() == pair[1].id() {
            return Err(ContractError::DuplicateAttributeId { id: pair[0].id() });
        }
    }
    Ok(())
}

fn validate_column_rows(columns: &[AttributeColumn], expected: usize) -> Result<(), ContractError> {
    for column in columns {
        let actual = column.len();
        if actual != expected {
            return Err(ContractError::AttributeRowCountMismatch {
                id: column.id(),
                expected,
                actual,
            });
        }
    }
    Ok(())
}

fn attribute_payload_bytes(columns: &[AttributeColumn]) -> Result<u64, ContractError> {
    columns.iter().try_fold(0_u64, |total, column| {
        total
            .checked_add(column.values().payload_bytes())
            .ok_or(ContractError::PayloadSizeOverflow)
    })
}

/// Declared Coordinate Reference of Source positions.
///
/// The representation is opaque so every WKT value passes through the same
/// length validation during construction and deserialization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CoordinateReference(CoordinateReferenceValue);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
enum CoordinateReferenceValue {
    Unknown,
    Wkt(String),
}

#[derive(Deserialize)]
enum CoordinateReferenceUnchecked {
    Unknown,
    Wkt(BoundedText<MAX_COORDINATE_REFERENCE_WKT_BYTES>),
}

impl CoordinateReference {
    /// No Coordinate Reference is declared; callers must not guess one.
    #[allow(non_upper_case_globals)]
    pub const Unknown: Self = Self(CoordinateReferenceValue::Unknown);

    /// Creates a declared well-known-text Coordinate Reference.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is empty or exceeds
    /// [`MAX_COORDINATE_REFERENCE_WKT_BYTES`] UTF-8 bytes.
    pub fn wkt(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ContractError::EmptyCoordinateReferenceWkt);
        }
        if value.len() > MAX_COORDINATE_REFERENCE_WKT_BYTES {
            return Err(ContractError::CoordinateReferenceWktTooLong {
                actual_bytes: value.len(),
                max_bytes: MAX_COORDINATE_REFERENCE_WKT_BYTES,
            });
        }
        Ok(Self(CoordinateReferenceValue::Wkt(value)))
    }

    /// Returns the well-known text, or `None` when explicitly unknown.
    #[must_use]
    pub fn as_wkt(&self) -> Option<&str> {
        match &self.0 {
            CoordinateReferenceValue::Unknown => None,
            CoordinateReferenceValue::Wkt(value) => Some(value),
        }
    }

    /// Reports whether the Coordinate Reference is explicitly unknown.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self.0, CoordinateReferenceValue::Unknown)
    }
}

impl<'de> Deserialize<'de> for CoordinateReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match CoordinateReferenceUnchecked::deserialize(deserializer)? {
            CoordinateReferenceUnchecked::Unknown => Ok(Self::Unknown),
            CoordinateReferenceUnchecked::Wkt(value) => {
                Self::wkt(value.into_string()).map_err(serde::de::Error::custom)
            }
        }
    }
}

/// Finite inclusive world-coordinate bounds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "WorldBoundsUnchecked")]
pub struct WorldBounds {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Deserialize)]
struct WorldBoundsUnchecked {
    min: [f64; 3],
    max: [f64; 3],
}

impl WorldBounds {
    /// Creates finite bounds whose minimum does not exceed their maximum.
    ///
    /// # Errors
    ///
    /// Returns an error for the first non-finite or reversed axis.
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Result<Self, ContractError> {
        for axis in 0..3 {
            if !min[axis].is_finite() || !max[axis].is_finite() {
                return Err(ContractError::NonFiniteWorldBounds { axis });
            }
            if min[axis] > max[axis] {
                return Err(ContractError::ReversedWorldBounds { axis });
            }
        }
        Ok(Self { min, max })
    }

    /// Returns the inclusive minimum corner.
    #[must_use]
    pub const fn min(self) -> [f64; 3] {
        self.min
    }

    /// Returns the inclusive maximum corner.
    #[must_use]
    pub const fn max(self) -> [f64; 3] {
        self.max
    }
}

impl TryFrom<WorldBoundsUnchecked> for WorldBounds {
    type Error = ContractError;

    fn try_from(value: WorldBoundsUnchecked) -> Result<Self, Self::Error> {
        Self::new(value.min, value.max)
    }
}

/// Ordered namespaced metadata retained from a Source format.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MetadataRecordUnchecked")]
pub struct MetadataRecord {
    namespace: String,
    name: String,
    payload: Vec<u8>,
}

#[derive(Deserialize)]
struct MetadataRecordUnchecked {
    namespace: BoundedText<MAX_METADATA_NAMESPACE_BYTES>,
    name: BoundedText<MAX_METADATA_NAME_BYTES>,
    #[serde(deserialize_with = "deserialize_metadata_payload")]
    payload: Vec<u8>,
}

impl MetadataRecord {
    /// Creates one namespaced metadata record while preserving payload bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace or name is empty, when either string
    /// exceeds its documented UTF-8 byte limit, or when `payload` exceeds
    /// [`MAX_METADATA_RECORD_PAYLOAD_BYTES`].
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<Self, ContractError> {
        let namespace = namespace.into();
        let name = name.into();
        if namespace.trim().is_empty() {
            return Err(ContractError::EmptyMetadataNamespace);
        }
        if namespace.len() > MAX_METADATA_NAMESPACE_BYTES {
            return Err(ContractError::MetadataNamespaceTooLong {
                actual_bytes: namespace.len(),
                max_bytes: MAX_METADATA_NAMESPACE_BYTES,
            });
        }
        if name.trim().is_empty() {
            return Err(ContractError::EmptyMetadataName);
        }
        if name.len() > MAX_METADATA_NAME_BYTES {
            return Err(ContractError::MetadataNameTooLong {
                actual_bytes: name.len(),
                max_bytes: MAX_METADATA_NAME_BYTES,
            });
        }
        if payload.len() > MAX_METADATA_RECORD_PAYLOAD_BYTES {
            return Err(ContractError::MetadataPayloadTooLong {
                actual_bytes: payload.len(),
                max_bytes: MAX_METADATA_RECORD_PAYLOAD_BYTES,
            });
        }
        Ok(Self {
            namespace,
            name,
            payload,
        })
    }

    /// Returns the format-owned namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the name within the namespace.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the exact metadata payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

fn deserialize_metadata_payload<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_sequence(
        deserializer,
        MAX_METADATA_RECORD_PAYLOAD_BYTES,
        |actual, max| ContractError::MetadataPayloadTooLong {
            actual_bytes: actual,
            max_bytes: max,
        },
    )
}

impl TryFrom<MetadataRecordUnchecked> for MetadataRecord {
    type Error = ContractError;

    fn try_from(value: MetadataRecordUnchecked) -> Result<Self, Self::Error> {
        Self::new(
            value.namespace.into_string(),
            value.name.into_string(),
            value.payload,
        )
    }
}

/// Immutable canonical metadata describing one Source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SourceMetadataUnchecked")]
pub struct SourceMetadata {
    point_count: u64,
    position_transform: PositionTransform,
    coordinate_reference: CoordinateReference,
    attributes: AttributeSchema,
    world_bounds: Option<WorldBounds>,
    format_name: String,
    metadata_records: Box<[MetadataRecord]>,
}

#[derive(Deserialize)]
struct SourceMetadataUnchecked {
    point_count: u64,
    position_transform: PositionTransform,
    coordinate_reference: CoordinateReference,
    attributes: AttributeSchema,
    world_bounds: Option<WorldBounds>,
    format_name: BoundedText<MAX_SOURCE_FORMAT_NAME_BYTES>,
    #[serde(deserialize_with = "deserialize_metadata_records")]
    metadata_records: Vec<MetadataRecord>,
}

impl SourceMetadata {
    /// Creates canonical Source metadata.
    ///
    /// # Errors
    ///
    /// A non-empty Source must have bounds and an empty Source must not. Returns
    /// an error when `format_name` is empty or exceeds
    /// [`MAX_SOURCE_FORMAT_NAME_BYTES`], when metadata record count exceeds
    /// [`MAX_METADATA_RECORDS`], or when their combined payload exceeds
    /// [`MAX_SOURCE_METADATA_PAYLOAD_BYTES`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        point_count: u64,
        position_transform: PositionTransform,
        coordinate_reference: CoordinateReference,
        attributes: AttributeSchema,
        world_bounds: Option<WorldBounds>,
        format_name: impl Into<String>,
        metadata_records: Vec<MetadataRecord>,
    ) -> Result<Self, ContractError> {
        let format_name = format_name.into();
        if format_name.trim().is_empty() {
            return Err(ContractError::EmptyFormatName);
        }
        if format_name.len() > MAX_SOURCE_FORMAT_NAME_BYTES {
            return Err(ContractError::FormatNameTooLong {
                actual_bytes: format_name.len(),
                max_bytes: MAX_SOURCE_FORMAT_NAME_BYTES,
            });
        }
        match (point_count, world_bounds) {
            (0, Some(_)) => return Err(ContractError::BoundsForEmptySource),
            (1.., None) => return Err(ContractError::MissingBoundsForNonEmptySource),
            _ => {}
        }
        validate_metadata_records(&metadata_records)?;
        Ok(Self {
            point_count,
            position_transform,
            coordinate_reference,
            attributes,
            world_bounds,
            format_name,
            metadata_records: metadata_records.into_boxed_slice(),
        })
    }

    /// Returns the total Point count.
    #[must_use]
    pub const fn point_count(&self) -> u64 {
        self.point_count
    }

    /// Returns the Source position transform.
    #[must_use]
    pub const fn position_transform(&self) -> PositionTransform {
        self.position_transform
    }

    /// Returns the declared Coordinate Reference.
    #[must_use]
    pub const fn coordinate_reference(&self) -> &CoordinateReference {
        &self.coordinate_reference
    }

    /// Returns the complete Attribute schema.
    #[must_use]
    pub const fn attributes(&self) -> &AttributeSchema {
        &self.attributes
    }

    /// Returns finite Source bounds when supplied.
    #[must_use]
    pub const fn world_bounds(&self) -> Option<WorldBounds> {
        self.world_bounds
    }

    /// Returns the canonical format name.
    #[must_use]
    pub fn format_name(&self) -> &str {
        &self.format_name
    }

    /// Returns format metadata in Source order.
    #[must_use]
    pub fn metadata_records(&self) -> &[MetadataRecord] {
        &self.metadata_records
    }
}

fn validate_metadata_records(metadata_records: &[MetadataRecord]) -> Result<(), ContractError> {
    if metadata_records.len() > MAX_METADATA_RECORDS {
        return Err(ContractError::TooManyMetadataRecords {
            actual: metadata_records.len(),
            max: MAX_METADATA_RECORDS,
        });
    }

    let mut payload_bytes = 0_usize;
    for record in metadata_records {
        payload_bytes = payload_bytes.checked_add(record.payload().len()).ok_or(
            ContractError::SourceMetadataPayloadTooLong {
                max_bytes: MAX_SOURCE_METADATA_PAYLOAD_BYTES,
            },
        )?;
        if payload_bytes > MAX_SOURCE_METADATA_PAYLOAD_BYTES {
            return Err(ContractError::SourceMetadataPayloadTooLong {
                max_bytes: MAX_SOURCE_METADATA_PAYLOAD_BYTES,
            });
        }
    }
    Ok(())
}

fn deserialize_metadata_records<'de, D>(deserializer: D) -> Result<Vec<MetadataRecord>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(MetadataRecordsVisitor)
}

struct MetadataRecordsVisitor;

impl<'de> Visitor<'de> for MetadataRecordsVisitor {
    type Value = Vec<MetadataRecord>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "at most {MAX_METADATA_RECORDS} metadata records totaling at most \
             {MAX_SOURCE_METADATA_PAYLOAD_BYTES} payload bytes"
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let size_hint = sequence.size_hint();
        if let Some(actual) = size_hint.filter(|actual| *actual > MAX_METADATA_RECORDS) {
            return Err(serde::de::Error::custom(
                ContractError::TooManyMetadataRecords {
                    actual,
                    max: MAX_METADATA_RECORDS,
                },
            ));
        }
        let capacity = size_hint.unwrap_or_default();
        let mut records = Vec::with_capacity(capacity);
        let mut payload_bytes = 0_usize;

        while records.len() < MAX_METADATA_RECORDS {
            let Some(record) = sequence.next_element::<MetadataRecord>()? else {
                return Ok(records);
            };
            payload_bytes = payload_bytes
                .checked_add(record.payload().len())
                .ok_or_else(|| {
                    serde::de::Error::custom(ContractError::SourceMetadataPayloadTooLong {
                        max_bytes: MAX_SOURCE_METADATA_PAYLOAD_BYTES,
                    })
                })?;
            if payload_bytes > MAX_SOURCE_METADATA_PAYLOAD_BYTES {
                return Err(serde::de::Error::custom(
                    ContractError::SourceMetadataPayloadTooLong {
                        max_bytes: MAX_SOURCE_METADATA_PAYLOAD_BYTES,
                    },
                ));
            }
            records.push(record);
        }

        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(serde::de::Error::custom(
                ContractError::TooManyMetadataRecords {
                    actual: MAX_METADATA_RECORDS.saturating_add(1),
                    max: MAX_METADATA_RECORDS,
                },
            ));
        }
        Ok(records)
    }
}

impl TryFrom<SourceMetadataUnchecked> for SourceMetadata {
    type Error = ContractError;

    fn try_from(value: SourceMetadataUnchecked) -> Result<Self, Self::Error> {
        Self::new(
            value.point_count,
            value.position_transform,
            value.coordinate_reference,
            value.attributes,
            value.world_bounds,
            value.format_name.into_string(),
            value.metadata_records,
        )
    }
}

/// Immutable provenance shared by a Source and every successful read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SourceProvenanceUnchecked")]
pub struct SourceProvenance {
    source: SourceId,
    content_hash: ContentHash,
    logical_order: String,
    contract_version: u32,
}

#[derive(Deserialize)]
struct SourceProvenanceUnchecked {
    source: SourceId,
    content_hash: ContentHash,
    logical_order: BoundedText<MAX_LOGICAL_ORDER_BYTES>,
    contract_version: u32,
}

impl SourceProvenance {
    /// Creates Source provenance with an explicit logical-order rule.
    ///
    /// # Errors
    ///
    /// Returns an error when `logical_order` is empty or exceeds
    /// [`MAX_LOGICAL_ORDER_BYTES`] UTF-8 bytes.
    pub fn new(
        source: SourceId,
        content_hash: ContentHash,
        logical_order: impl Into<String>,
        contract_version: u32,
    ) -> Result<Self, ContractError> {
        let logical_order = logical_order.into();
        if logical_order.trim().is_empty() {
            return Err(ContractError::EmptyLogicalOrder);
        }
        if logical_order.len() > MAX_LOGICAL_ORDER_BYTES {
            return Err(ContractError::LogicalOrderTooLong {
                actual_bytes: logical_order.len(),
                max_bytes: MAX_LOGICAL_ORDER_BYTES,
            });
        }
        Ok(Self {
            source,
            content_hash,
            logical_order,
            contract_version,
        })
    }

    /// Returns the Source Identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Returns the full canonical content hash.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Returns the versioned logical-order rule.
    #[must_use]
    pub fn logical_order(&self) -> &str {
        &self.logical_order
    }

    /// Returns the canonical contract version.
    #[must_use]
    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }
}

fn deserialize_bounded_sequence<'de, D, T>(
    deserializer: D,
    max_items: usize,
    too_many: fn(usize, usize) -> ContractError,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_seq(BoundedSequenceVisitor {
        max_items,
        too_many,
        item: PhantomData,
    })
}

struct BoundedSequenceVisitor<T> {
    max_items: usize,
    too_many: fn(usize, usize) -> ContractError,
    item: PhantomData<T>,
}

impl<'de, T> Visitor<'de> for BoundedSequenceVisitor<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a sequence of at most {} items", self.max_items)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let size_hint = sequence.size_hint();
        if let Some(actual) = size_hint.filter(|actual| *actual > self.max_items) {
            return Err(serde::de::Error::custom((self.too_many)(
                actual,
                self.max_items,
            )));
        }
        let capacity = size_hint.unwrap_or_default();
        let mut items = Vec::with_capacity(capacity);
        while items.len() < self.max_items {
            let Some(item) = sequence.next_element()? else {
                return Ok(items);
            };
            items.push(item);
        }

        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(serde::de::Error::custom((self.too_many)(
                self.max_items.saturating_add(1),
                self.max_items,
            )));
        }
        Ok(items)
    }
}

#[cfg(test)]
mod bounded_deserialization_tests {
    use serde::de::value::{Error as ValueError, SeqDeserializer};

    use super::*;

    #[test]
    fn oversized_metadata_payload_size_hint_is_rejected_before_allocation() {
        let bytes = std::iter::repeat_n(0_u8, MAX_METADATA_RECORD_PAYLOAD_BYTES + 1);
        let deserializer = SeqDeserializer::<_, ValueError>::new(bytes);

        let error = deserialize_metadata_payload(deserializer).unwrap_err();

        assert!(error.to_string().contains("metadata payload"));
    }
}

impl TryFrom<SourceProvenanceUnchecked> for SourceProvenance {
    type Error = ContractError;

    fn try_from(value: SourceProvenanceUnchecked) -> Result<Self, Self::Error> {
        Self::new(
            value.source,
            value.content_hash,
            value.logical_order.into_string(),
            value.contract_version,
        )
    }
}

/// One non-empty contiguous Point Batch in logical Source order.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PointBatch {
    source: SourceId,
    first_ordinal: u64,
    positions: QuantizedPositions,
    attributes: AttributeColumns,
    #[serde(skip)]
    point_count: u64,
    #[serde(skip)]
    last_ordinal: u64,
    #[serde(skip)]
    estimated_payload_bytes: u64,
}

impl PointBatch {
    /// Creates a contiguous Point Batch with row-aligned positions and Attributes.
    ///
    /// # Errors
    ///
    /// Returns an error when row counts differ, the ordinal range overflows, or
    /// payload byte accounting overflows.
    pub fn new(
        source: SourceId,
        first_ordinal: u64,
        positions: QuantizedPositions,
        attributes: AttributeColumns,
    ) -> Result<Self, ContractError> {
        let row_count = positions.len();
        if attributes.row_count() != row_count {
            return Err(ContractError::PointBatchRowCountMismatch {
                positions: row_count,
                attributes: attributes.row_count(),
            });
        }
        let (point_count, last_ordinal) = ordinal_facts(first_ordinal, row_count)?;
        let estimated_payload_bytes = positions
            .estimated_payload_bytes()
            .checked_add(attributes.estimated_payload_bytes())
            .ok_or(ContractError::PayloadSizeOverflow)?;
        Ok(Self {
            source,
            first_ordinal,
            positions,
            attributes,
            point_count,
            last_ordinal,
            estimated_payload_bytes,
        })
    }

    /// Returns the Source Identity shared by every Point.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Returns the first logical Source ordinal.
    #[must_use]
    pub const fn first_ordinal(&self) -> u64 {
        self.first_ordinal
    }

    /// Returns the final logical Source ordinal.
    #[must_use]
    pub const fn last_ordinal(&self) -> u64 {
        self.last_ordinal
    }

    /// Returns the exact quantized positions.
    #[must_use]
    pub const fn positions(&self) -> &QuantizedPositions {
        &self.positions
    }

    /// Returns the row-aligned Attribute columns.
    #[must_use]
    pub const fn attributes(&self) -> &AttributeColumns {
        &self.attributes
    }

    /// Returns the Point count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// Reports whether the Point Batch is empty.
    ///
    /// A constructed value always returns `false`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Returns the Point count in Source-scale accounting units.
    #[must_use]
    pub const fn point_count(&self) -> u64 {
        self.point_count
    }

    /// Returns one stable Point Identity, or `None` for an invalid row.
    #[must_use]
    pub fn point_id(&self, row: usize) -> Option<PointId> {
        if row >= self.len() {
            return None;
        }
        let row = u64::try_from(row).ok()?;
        self.first_ordinal
            .checked_add(row)
            .map(|ordinal| PointId::new(self.source, ordinal))
    }

    /// Iterates stable Point Identities in ascending logical ordinal order.
    #[must_use]
    pub fn point_ids(&self) -> PointIds {
        PointIds {
            source: self.source,
            next_ordinal: self.first_ordinal,
            remaining: self.len(),
        }
    }

    /// Returns position and Attribute payload bytes, excluding container overhead.
    #[must_use]
    pub const fn estimated_payload_bytes(&self) -> u64 {
        self.estimated_payload_bytes
    }

    /// Copies a non-empty row range into another contiguous Point Batch.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid or empty row range.
    pub fn slice_rows(&self, rows: Range<usize>) -> Result<Self, ContractError> {
        validate_row_range(&rows, self.len())?;
        let first_offset =
            u64::try_from(rows.start).map_err(|_| ContractError::PointOrdinalOverflow {
                first_ordinal: self.first_ordinal,
                point_count: self.point_count(),
            })?;
        Self::new(
            self.source,
            self.first_ordinal + first_offset,
            self.positions.slice_rows(rows.clone())?,
            self.attributes.slice_rows(rows)?,
        )
    }
}

fn ordinal_facts(first_ordinal: u64, row_count: usize) -> Result<(u64, u64), ContractError> {
    let point_count =
        u64::try_from(row_count).map_err(|_| ContractError::PointOrdinalOverflow {
            first_ordinal,
            point_count: u64::MAX,
        })?;
    let last_offset = point_count - 1;
    let last_ordinal =
        first_ordinal
            .checked_add(last_offset)
            .ok_or(ContractError::PointOrdinalOverflow {
                first_ordinal,
                point_count,
            })?;
    Ok((point_count, last_ordinal))
}

/// Exact-size iterator over the stable Point Identities of one Point Batch.
#[derive(Clone, Debug)]
pub struct PointIds {
    source: SourceId,
    next_ordinal: u64,
    remaining: usize,
}

impl Iterator for PointIds {
    type Item = PointId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let point = PointId::new(self.source, self.next_ordinal);
        self.remaining -= 1;
        if self.remaining > 0 {
            self.next_ordinal += 1;
        }
        Some(point)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for PointIds {}
impl FusedIterator for PointIds {}

fn validate_row_range(rows: &Range<usize>, len: usize) -> Result<(), ContractError> {
    validate_row_range_allow_empty(rows, len)?;
    if rows.is_empty() {
        return Err(ContractError::EmptyRowRange);
    }
    Ok(())
}

fn validate_row_range_allow_empty(rows: &Range<usize>, len: usize) -> Result<(), ContractError> {
    if rows.start > rows.end || rows.end > len {
        return Err(ContractError::InvalidRowRange {
            start: rows.start,
            end: rows.end,
            len,
        });
    }
    Ok(())
}

/// Invalid canonical contract input.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ContractError {
    /// One position offset axis was NaN or infinite.
    #[error("position offset axis {axis} must be finite")]
    NonFinitePositionOffset {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// One position scale axis was non-positive, NaN, or infinite.
    #[error("position scale axis {axis} must be finite and positive")]
    InvalidPositionScale {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// Quantized positions contained no rows.
    #[error("quantized positions must not be empty")]
    EmptyQuantizedPositions,
    /// Attribute identity zero is reserved as invalid.
    #[error("Attribute identities must be nonzero")]
    ZeroAttributeId,
    /// An Attribute name was empty or whitespace-only.
    #[error("Attribute {id:?} name must not be empty")]
    EmptyAttributeName {
        /// Affected Attribute identity.
        id: AttributeId,
    },
    /// An Attribute name exceeded its documented UTF-8 byte limit.
    #[error("Attribute {id:?} name is {actual_bytes} bytes; maximum is {max_bytes}")]
    AttributeNameTooLong {
        /// Affected Attribute identity.
        id: AttributeId,
        /// Supplied UTF-8 byte count.
        actual_bytes: usize,
        /// Maximum accepted UTF-8 byte count.
        max_bytes: usize,
    },
    /// An Attribute schema contained too many definitions.
    #[error("Attribute schema has {actual} definitions; maximum is {max}")]
    TooManyAttributeDefinitions {
        /// Supplied definition count.
        actual: usize,
        /// Maximum accepted definition count.
        max: usize,
    },
    /// A fixed-byte Attribute declared zero bytes per value.
    #[error("fixed-byte Attribute width must be nonzero")]
    ZeroFixedBytesWidth,
    /// A fixed-byte payload ended partway through one value.
    #[error("fixed-byte payload of {payload_bytes} bytes is not divisible by width {width}")]
    InvalidFixedBytesLength {
        /// Declared bytes per row.
        width: u32,
        /// Supplied payload bytes.
        payload_bytes: usize,
    },
    /// A bounded Attribute-value wire payload exceeded its caller budget.
    #[error("Attribute values payload is {actual_bytes} bytes; maximum is {max_bytes}")]
    AttributeValuesPayloadTooLong {
        /// Minimum supplied payload byte count known at rejection.
        actual_bytes: u64,
        /// Caller-provided maximum payload byte count.
        max_bytes: u64,
    },
    /// Attribute values did not match their definition.
    #[error("Attribute {id:?} declares {expected:?} but contains {actual:?}")]
    AttributeTypeMismatch {
        /// Affected Attribute identity.
        id: AttributeId,
        /// Declared storage type.
        expected: AttributeDataType,
        /// Actual value type.
        actual: AttributeDataType,
    },
    /// Two definitions or columns had the same Attribute identity.
    #[error("Attribute identity {id:?} appears more than once")]
    DuplicateAttributeId {
        /// Duplicate Attribute identity.
        id: AttributeId,
    },
    /// One Attribute column had the wrong row count.
    #[error("Attribute {id:?} has {actual} rows; expected {expected}")]
    AttributeRowCountMismatch {
        /// Affected Attribute identity.
        id: AttributeId,
        /// Required row count.
        expected: usize,
        /// Actual row count.
        actual: usize,
    },
    /// A row range was reversed or out of bounds.
    #[error("row range {start}..{end} is invalid for length {len}")]
    InvalidRowRange {
        /// Inclusive start row.
        start: usize,
        /// Exclusive end row.
        end: usize,
        /// Available row count.
        len: usize,
    },
    /// A required non-empty slice selected no rows.
    #[error("row range must not be empty")]
    EmptyRowRange,
    /// One world-bounds axis contained NaN or infinity.
    #[error("world bounds axis {axis} must be finite")]
    NonFiniteWorldBounds {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// One world-bounds minimum exceeded its maximum.
    #[error("world bounds axis {axis} is reversed")]
    ReversedWorldBounds {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// A Coordinate Reference WKT value was empty or whitespace-only.
    #[error("Coordinate Reference WKT must not be empty")]
    EmptyCoordinateReferenceWkt,
    /// A Coordinate Reference WKT value exceeded its documented byte limit.
    #[error("Coordinate Reference WKT is {actual_bytes} bytes; maximum is {max_bytes}")]
    CoordinateReferenceWktTooLong {
        /// Supplied UTF-8 byte count.
        actual_bytes: usize,
        /// Maximum accepted UTF-8 byte count.
        max_bytes: usize,
    },
    /// A metadata namespace was empty or whitespace-only.
    #[error("metadata namespace must not be empty")]
    EmptyMetadataNamespace,
    /// A metadata namespace exceeded its documented UTF-8 byte limit.
    #[error("metadata namespace is {actual_bytes} bytes; maximum is {max_bytes}")]
    MetadataNamespaceTooLong {
        /// Supplied UTF-8 byte count.
        actual_bytes: usize,
        /// Maximum accepted UTF-8 byte count.
        max_bytes: usize,
    },
    /// A metadata name was empty or whitespace-only.
    #[error("metadata name must not be empty")]
    EmptyMetadataName,
    /// A metadata name exceeded its documented UTF-8 byte limit.
    #[error("metadata name is {actual_bytes} bytes; maximum is {max_bytes}")]
    MetadataNameTooLong {
        /// Supplied UTF-8 byte count.
        actual_bytes: usize,
        /// Maximum accepted UTF-8 byte count.
        max_bytes: usize,
    },
    /// A metadata payload exceeded its per-record byte limit.
    #[error("metadata payload is {actual_bytes} bytes; maximum is {max_bytes}")]
    MetadataPayloadTooLong {
        /// Supplied payload byte count.
        actual_bytes: usize,
        /// Maximum accepted payload byte count.
        max_bytes: usize,
    },
    /// Source metadata contained too many ordered records.
    #[error("Source metadata has {actual} records; maximum is {max}")]
    TooManyMetadataRecords {
        /// Supplied record count.
        actual: usize,
        /// Maximum accepted record count.
        max: usize,
    },
    /// Source metadata payload exceeded its combined byte limit.
    #[error("combined Source metadata payload exceeds {max_bytes} bytes")]
    SourceMetadataPayloadTooLong {
        /// Maximum accepted combined payload byte count.
        max_bytes: usize,
    },
    /// A Source format name was empty or whitespace-only.
    #[error("Source format name must not be empty")]
    EmptyFormatName,
    /// A Source format name exceeded its documented UTF-8 byte limit.
    #[error("Source format name is {actual_bytes} bytes; maximum is {max_bytes}")]
    FormatNameTooLong {
        /// Supplied UTF-8 byte count.
        actual_bytes: usize,
        /// Maximum accepted UTF-8 byte count.
        max_bytes: usize,
    },
    /// An empty Source supplied bounds it cannot substantiate.
    #[error("an empty Source must not declare world bounds")]
    BoundsForEmptySource,
    /// A non-empty Source omitted required finite world bounds.
    #[error("a non-empty Source must declare world bounds")]
    MissingBoundsForNonEmptySource,
    /// A provenance logical-order rule was empty or whitespace-only.
    #[error("Source logical-order rule must not be empty")]
    EmptyLogicalOrder,
    /// A logical-order rule exceeded its documented UTF-8 byte limit.
    #[error("Source logical-order rule is {actual_bytes} bytes; maximum is {max_bytes}")]
    LogicalOrderTooLong {
        /// Supplied UTF-8 byte count.
        actual_bytes: usize,
        /// Maximum accepted UTF-8 byte count.
        max_bytes: usize,
    },
    /// Point Batch position and Attribute row counts differed.
    #[error("Point Batch has {positions} positions but {attributes} Attribute rows")]
    PointBatchRowCountMismatch {
        /// Position row count.
        positions: usize,
        /// Attribute row count.
        attributes: usize,
    },
    /// A contiguous Point ordinal range exceeded `u64`.
    #[error("Point range beginning at {first_ordinal} with {point_count} Points overflows")]
    PointOrdinalOverflow {
        /// First Point ordinal.
        first_ordinal: u64,
        /// Requested Point count.
        point_count: u64,
    },
    /// Payload byte accounting exceeded its integer representation.
    #[error("canonical payload size exceeds the supported integer range")]
    PayloadSizeOverflow,
}
