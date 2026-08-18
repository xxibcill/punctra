//! Public-interface tests for canonical Source and Point values.

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeSchema, AttributeValues, ContentHash, ContractError, CoordinateReference, LinearUnit,
    MAX_ATTRIBUTE_DEFINITIONS, MAX_ATTRIBUTE_NAME_BYTES, MAX_COORDINATE_REFERENCE_WKT_BYTES,
    MAX_LOGICAL_ORDER_BYTES, MAX_METADATA_NAME_BYTES, MAX_METADATA_NAMESPACE_BYTES,
    MAX_METADATA_RECORD_PAYLOAD_BYTES, MAX_METADATA_RECORDS, MAX_SOURCE_FORMAT_NAME_BYTES,
    MetadataRecord, PointBatch, PointId, PositionTransform, QuantizedPositions, SourceId,
    SourceMetadata, SourceProvenance, SpatialAxes, SpatialReferenceProfile,
    SpatialReferenceProvenance, WorldBounds,
};
use serde::Serialize;
use serde_json::json;

#[test]
fn identities_are_opaque_stable_values() {
    let source = SourceId::new([0xab; 32]);
    let hash = ContentHash::new([0x0f; 32]);
    let point = PointId::new(source, 42);

    assert_eq!(source.as_bytes(), &[0xab; 32]);
    assert_eq!(source.into_bytes(), [0xab; 32]);
    assert_eq!(source.to_string(), "ab".repeat(32));
    assert_eq!(hash.as_bytes(), &[0x0f; 32]);
    assert_eq!(hash.to_string(), "0f".repeat(32));
    assert_eq!(point.source(), source);
    assert_eq!(point.ordinal(), 42);
}

#[test]
fn positions_preserve_exact_ticks_and_use_a_finite_positive_transform() {
    assert_eq!(
        PositionTransform::new([f64::NAN, 0.0, 0.0], [1.0; 3]),
        Err(ContractError::NonFinitePositionOffset { axis: 0 })
    );
    assert_eq!(
        PositionTransform::new([0.0; 3], [1.0, 0.0, 1.0]),
        Err(ContractError::InvalidPositionScale { axis: 1 })
    );

    let transform = PositionTransform::new([100.0, -20.0, 5.0], [0.5, 2.0, 0.25]).unwrap();
    assert_eq!(
        transform.offset().map(f64::to_bits),
        [100.0, -20.0, 5.0].map(f64::to_bits)
    );
    let transform_wire = serde_json::to_value(transform).unwrap();
    assert_eq!(transform_wire["offset"], json!([100.0, -20.0, 5.0]));
    assert!(transform_wire.get("origin").is_none());
    let ticks = vec![[2, -3, 4], [i64::MIN, 0, i64::MAX]];
    let positions = QuantizedPositions::new(transform, ticks.clone()).unwrap();

    assert_eq!(positions.ticks(), ticks);
    assert_eq!(positions.world_f64(0), Some([101.0, -26.0, 6.0]));
    assert_eq!(positions.world_f64(2), None);
    assert_eq!(positions.estimated_payload_bytes(), 48);
    assert_eq!(
        QuantizedPositions::new(transform, Vec::new()),
        Err(ContractError::EmptyQuantizedPositions)
    );

    let invalid: Result<PositionTransform, _> = serde_json::from_value(json!({
        "offset": [0.0, 0.0, 0.0],
        "scale": [1.0, -1.0, 1.0]
    }));
    assert!(invalid.is_err(), "deserialization must preserve invariants");
}

#[test]
fn attribute_schema_and_columns_are_sorted_typed_and_row_aligned() {
    let classification = definition(1, "classification", AttributeDataType::U8);
    let intensity = definition(2, "intensity", AttributeDataType::U16);
    let schema = AttributeSchema::new(vec![intensity.clone(), classification.clone()]).unwrap();

    assert_eq!(
        schema
            .definitions()
            .iter()
            .map(AttributeDefinition::id)
            .collect::<Vec<_>>(),
        [attribute_id(1), attribute_id(2)]
    );
    assert_eq!(schema.get(attribute_id(2)), Some(&intensity));
    assert_eq!(
        AttributeSchema::new(vec![classification.clone(), classification.clone()]),
        Err(ContractError::DuplicateAttributeId {
            id: attribute_id(1)
        })
    );

    assert_eq!(
        AttributeColumn::new(classification.clone(), AttributeValues::u16(vec![1, 2])),
        Err(ContractError::AttributeTypeMismatch {
            id: attribute_id(1),
            expected: AttributeDataType::U8,
            actual: AttributeDataType::U16,
        })
    );

    let columns = AttributeColumns::new(
        vec![
            AttributeColumn::new(intensity, AttributeValues::u16(vec![100, 200])).unwrap(),
            AttributeColumn::new(classification, AttributeValues::u8(vec![2, 5])).unwrap(),
        ],
        2,
    )
    .unwrap();
    assert_eq!(columns.columns()[0].id(), attribute_id(1));
    assert_eq!(columns.columns()[1].id(), attribute_id(2));
    assert_eq!(columns.estimated_payload_bytes(), 6);
    assert_eq!(
        columns.get(attribute_id(1)).unwrap().values().as_u8(),
        Some([2, 5].as_slice())
    );

    let wrong_rows = AttributeColumn::new(
        definition(3, "return", AttributeDataType::U8),
        AttributeValues::u8(vec![1]),
    )
    .unwrap();
    assert_eq!(
        AttributeColumns::new(vec![wrong_rows], 2),
        Err(ContractError::AttributeRowCountMismatch {
            id: attribute_id(3),
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn typed_values_slice_without_coercion() {
    let opaque = AttributeValues::fixed_bytes(3, vec![1, 2, 3, 4, 5, 6]).unwrap();
    assert_eq!(
        opaque.data_type(),
        AttributeDataType::fixed_bytes(3).unwrap()
    );
    assert_eq!(opaque.len(), 2);
    assert_eq!(opaque.payload_bytes(), 6);
    assert_eq!(
        opaque.as_fixed_bytes(),
        Some((3, [1, 2, 3, 4, 5, 6].as_slice()))
    );
    assert_eq!(
        opaque.slice_rows(1..2).unwrap().as_fixed_bytes(),
        Some((3, [4, 5, 6].as_slice()))
    );
    assert_eq!(
        AttributeValues::fixed_bytes(0, vec![]),
        Err(ContractError::ZeroFixedBytesWidth)
    );
    assert_eq!(
        AttributeValues::fixed_bytes(3, vec![1, 2]),
        Err(ContractError::InvalidFixedBytesLength {
            width: 3,
            payload_bytes: 2,
        })
    );

    let floats = AttributeValues::f64(vec![-0.0, 1.25]);
    assert_eq!(floats.as_f64().unwrap()[0].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(
        floats.slice_rows(0..1).unwrap().as_f64().unwrap()[0].to_bits(),
        (-0.0_f64).to_bits()
    );
}

#[test]
fn floating_attribute_values_use_bitwise_equality_and_exact_bounded_wire() {
    let f32_bits = [
        0x7fc0_0001,
        0xffc1_2345,
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        0.0_f32.to_bits(),
        (-0.0_f32).to_bits(),
    ];
    let f32_values = AttributeValues::f32(f32_bits.map(f32::from_bits).to_vec());
    assert_eq!(
        f32_values,
        f32_values.clone(),
        "NaNs must be reflexive by bits"
    );
    assert_ne!(
        AttributeValues::f32(vec![f32::from_bits(0x7fc0_0001)]),
        AttributeValues::f32(vec![f32::from_bits(0x7fc0_0002)]),
        "distinct NaN payloads must remain distinct"
    );
    assert_ne!(
        AttributeValues::f32(vec![0.0]),
        AttributeValues::f32(vec![-0.0]),
        "signed zero must remain distinct"
    );

    let encoded_f32 = serde_json::to_string(&f32_values).unwrap();
    let f32_wire: serde_json::Value = serde_json::from_str(&encoded_f32).unwrap();
    assert_eq!(f32_wire["values"]["f32_bits"], json!(f32_bits));
    assert_eq!(
        deserialize_attribute_values(&encoded_f32, f32_values.payload_bytes()).unwrap(),
        f32_values
    );
    assert!(
        deserialize_attribute_values(&encoded_f32, f32_values.payload_bytes() - 1).is_err(),
        "the explicit wire decoder must enforce its payload budget"
    );

    let f64_bits = [
        0x7ff8_0000_0000_0001,
        0xfff8_0000_1234_5678,
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        0.0_f64.to_bits(),
        (-0.0_f64).to_bits(),
    ];
    let f64_values = AttributeValues::f64(f64_bits.map(f64::from_bits).to_vec());
    assert_eq!(
        f64_values,
        f64_values.clone(),
        "NaNs must be reflexive by bits"
    );
    assert_ne!(
        AttributeValues::f64(vec![f64::from_bits(0x7ff8_0000_0000_0001)]),
        AttributeValues::f64(vec![f64::from_bits(0x7ff8_0000_0000_0002)]),
        "distinct NaN payloads must remain distinct"
    );
    assert_ne!(
        AttributeValues::f64(vec![0.0]),
        AttributeValues::f64(vec![-0.0]),
        "signed zero must remain distinct"
    );

    let encoded_f64 = serde_json::to_string(&f64_values).unwrap();
    let f64_wire: serde_json::Value = serde_json::from_str(&encoded_f64).unwrap();
    assert_eq!(f64_wire["values"]["f64_bits"], json!(f64_bits));
    assert_eq!(
        deserialize_attribute_values(&encoded_f64, f64_values.payload_bytes()).unwrap(),
        f64_values
    );
}

#[test]
fn metadata_and_provenance_round_trip_without_guessing() {
    let transform = PositionTransform::new([1.0, 2.0, 3.0], [0.01; 3]).unwrap();
    let bounds = WorldBounds::new([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap();
    let record = MetadataRecord::new("las.vlr", "34735", vec![0, 1, 255]).unwrap();
    let metadata = SourceMetadata::new(
        7,
        transform,
        CoordinateReference::Unknown,
        AttributeSchema::empty(),
        Some(bounds),
        "LAS 1.4",
        vec![record.clone()],
    )
    .unwrap();
    let provenance = SourceProvenance::new(
        SourceId::new([7; 32]),
        ContentHash::new([8; 32]),
        "point-record-order-v1",
        1,
    )
    .unwrap();

    assert!(metadata.coordinate_reference().is_unknown());
    assert_eq!(metadata.metadata_records(), [record]);
    assert_eq!(metadata.world_bounds(), Some(bounds));
    assert_eq!(provenance.source(), SourceId::new([7; 32]));
    assert_eq!(provenance.content_hash(), ContentHash::new([8; 32]));
    let metadata_json = serde_json::to_string(&metadata).unwrap();
    let provenance_json = serde_json::to_string(&provenance).unwrap();
    assert_eq!(
        serde_json::from_str::<SourceMetadata>(&metadata_json).unwrap(),
        metadata
    );
    assert_eq!(
        serde_json::from_str::<SourceProvenance>(&provenance_json).unwrap(),
        provenance
    );
}

#[test]
fn structured_spatial_profile_is_explicit_bounded_and_canonical() {
    let profile = SpatialReferenceProfile::new(
        32_647,
        5_703,
        SpatialAxes::EastingNorthingElevation,
        LinearUnit::Metre,
        LinearUnit::Metre,
        SpatialReferenceProvenance::SourceMetadata,
    )
    .unwrap();
    let reference = CoordinateReference::profile(profile);

    assert_eq!(profile.horizontal_epsg(), 32_647);
    assert_eq!(profile.vertical_epsg(), 5_703);
    assert_eq!(profile.axes(), SpatialAxes::EastingNorthingElevation);
    assert_eq!(profile.horizontal_unit(), LinearUnit::Metre);
    assert_eq!(profile.vertical_unit(), LinearUnit::Metre);
    assert_eq!(
        profile.provenance(),
        SpatialReferenceProvenance::SourceMetadata
    );
    assert!(profile.is_supported_metric_survey());
    assert_eq!(
        profile.canonical_bytes(),
        [1, 0, 0, 0, 0x87, 0x7f, 0, 0, 0x47, 0x16, 0, 0, 1, 1, 1, 1,]
    );
    assert_eq!(reference.spatial_profile(), Some(profile));
    assert_eq!(reference.as_wkt(), None);
    assert!(!reference.is_unknown());

    let encoded = serde_json::to_value(&reference).unwrap();
    assert_eq!(encoded["Profile"]["horizontal_epsg"], 32_647);
    assert_eq!(encoded["Profile"]["vertical_epsg"], 5_703);
    assert_eq!(
        serde_json::from_value::<CoordinateReference>(encoded).unwrap(),
        reference
    );

    assert_eq!(
        SpatialReferenceProfile::new(
            0,
            5_703,
            SpatialAxes::EastingNorthingElevation,
            LinearUnit::Metre,
            LinearUnit::Metre,
            SpatialReferenceProvenance::CallerDeclaration,
        ),
        Err(ContractError::InvalidHorizontalEpsg { value: 0 })
    );
    let caller_profile = SpatialReferenceProfile::new(
        32_767,
        32_767,
        SpatialAxes::EastingNorthingElevation,
        LinearUnit::Metre,
        LinearUnit::Metre,
        SpatialReferenceProvenance::CallerDeclaration,
    )
    .expect("caller declarations accept every nonzero EPSG identity");
    assert_eq!(caller_profile.horizontal_epsg(), 32_767);
    assert_eq!(caller_profile.vertical_epsg(), 32_767);
    assert_eq!(
        serde_json::from_value::<CoordinateReference>(json!({
            "Profile": {
                "horizontal_epsg": 32767,
                "vertical_epsg": 32767,
                "axes": "EastingNorthingElevation",
                "horizontal_unit": "Metre",
                "vertical_unit": "Metre",
                "provenance": "CallerDeclaration"
            }
        }))
        .unwrap()
        .spatial_profile(),
        Some(caller_profile)
    );
    assert!(
        serde_json::from_value::<CoordinateReference>(json!({
            "Profile": {
                "horizontal_epsg": 32647,
                "vertical_epsg": 0,
                "axes": "EastingNorthingElevation",
                "horizontal_unit": "Metre",
                "vertical_unit": "Metre",
                "provenance": "CallerDeclaration"
            }
        }))
        .is_err()
    );

    let feet = SpatialReferenceProfile::new(
        2_230,
        5_703,
        SpatialAxes::EastingNorthingElevation,
        LinearUnit::UsSurveyFoot,
        LinearUnit::UsSurveyFoot,
        SpatialReferenceProvenance::CallerDeclaration,
    )
    .unwrap();
    assert!(!feet.is_supported_metric_survey());
    assert_eq!(LinearUnit::InternationalFoot.epsg_code(), 9_002);
    assert_eq!(LinearUnit::UsSurveyFoot.epsg_code(), 9_003);
}

#[test]
fn every_structured_profile_value_has_exact_wire_and_canonical_code() {
    for (unit, unit_wire, unit_canonical) in [
        (LinearUnit::Metre, "Metre", 1),
        (LinearUnit::InternationalFoot, "InternationalFoot", 2),
        (LinearUnit::UsSurveyFoot, "UsSurveyFoot", 3),
    ] {
        for (provenance, provenance_wire, provenance_canonical) in [
            (
                SpatialReferenceProvenance::SourceMetadata,
                "SourceMetadata",
                1,
            ),
            (
                SpatialReferenceProvenance::CallerDeclaration,
                "CallerDeclaration",
                2,
            ),
        ] {
            let profile = SpatialReferenceProfile::new(
                32_647,
                5_703,
                SpatialAxes::EastingNorthingElevation,
                unit,
                unit,
                provenance,
            )
            .unwrap();
            let canonical = profile.canonical_bytes();
            assert_eq!(LinearUnit::from_epsg_code(unit.epsg_code()), Some(unit));
            assert_eq!(canonical[12], 1, "axis canonical code");
            assert_eq!(canonical[13], unit_canonical, "horizontal unit code");
            assert_eq!(canonical[14], unit_canonical, "vertical unit code");
            assert_eq!(canonical[15], provenance_canonical, "provenance code");

            let encoded = serde_json::to_value(CoordinateReference::profile(profile)).unwrap();
            assert_eq!(encoded["Profile"]["axes"], "EastingNorthingElevation");
            assert_eq!(encoded["Profile"]["horizontal_unit"], unit_wire);
            assert_eq!(encoded["Profile"]["vertical_unit"], unit_wire);
            assert_eq!(encoded["Profile"]["provenance"], provenance_wire);
            assert_eq!(
                serde_json::from_value::<CoordinateReference>(encoded).unwrap(),
                CoordinateReference::profile(profile)
            );
        }
    }
    assert_eq!(LinearUnit::from_epsg_code(9_012), None);
}

#[test]
fn source_bounds_exist_exactly_when_points_exist() {
    let transform = PositionTransform::new([0.0; 3], [1.0; 3]).unwrap();
    let bounds = WorldBounds::new([0.0; 3], [1.0; 3]).unwrap();

    let empty = SourceMetadata::new(
        0,
        transform,
        CoordinateReference::Unknown,
        AttributeSchema::empty(),
        None,
        "memory",
        Vec::new(),
    )
    .unwrap();
    assert_eq!(empty.world_bounds(), None);

    let non_empty = SourceMetadata::new(
        1,
        transform,
        CoordinateReference::Unknown,
        AttributeSchema::empty(),
        Some(bounds),
        "memory",
        Vec::new(),
    )
    .unwrap();
    assert_eq!(non_empty.world_bounds(), Some(bounds));

    assert_eq!(
        SourceMetadata::new(
            0,
            transform,
            CoordinateReference::Unknown,
            AttributeSchema::empty(),
            Some(bounds),
            "memory",
            Vec::new(),
        ),
        Err(ContractError::BoundsForEmptySource)
    );
    assert_eq!(
        SourceMetadata::new(
            1,
            transform,
            CoordinateReference::Unknown,
            AttributeSchema::empty(),
            None,
            "memory",
            Vec::new(),
        ),
        Err(ContractError::MissingBoundsForNonEmptySource)
    );

    let invalid_empty = RawSourceMetadata {
        point_count: 0,
        position_transform: transform,
        coordinate_reference: &CoordinateReference::Unknown,
        attributes: &AttributeSchema::empty(),
        world_bounds: Some(bounds),
        format_name: "memory",
        metadata_records: &[],
    };
    assert!(deserialize_raw_source_metadata(&invalid_empty).is_err());

    let invalid_non_empty = RawSourceMetadata {
        point_count: 1,
        position_transform: transform,
        coordinate_reference: &CoordinateReference::Unknown,
        attributes: &AttributeSchema::empty(),
        world_bounds: None,
        format_name: "memory",
        metadata_records: &[],
    };
    assert!(deserialize_raw_source_metadata(&invalid_non_empty).is_err());
}

#[test]
fn source_metadata_text_and_collection_limits_are_checked() {
    let attribute_id = attribute_id(1);
    let longest_attribute_name = "a".repeat(MAX_ATTRIBUTE_NAME_BYTES);
    assert!(
        AttributeDefinition::new(attribute_id, longest_attribute_name, AttributeDataType::U8)
            .is_ok()
    );
    let oversized_attribute_name = "a".repeat(MAX_ATTRIBUTE_NAME_BYTES + 1);
    assert_eq!(
        AttributeDefinition::new(
            attribute_id,
            oversized_attribute_name.clone(),
            AttributeDataType::U8,
        ),
        Err(ContractError::AttributeNameTooLong {
            id: attribute_id,
            actual_bytes: MAX_ATTRIBUTE_NAME_BYTES + 1,
            max_bytes: MAX_ATTRIBUTE_NAME_BYTES,
        })
    );
    let encoded_definition = json!({
        "id": attribute_id,
        "name": oversized_attribute_name,
        "data_type": "U8",
    });
    assert!(serde_json::from_value::<AttributeDefinition>(encoded_definition).is_err());

    let definitions = (1..=MAX_ATTRIBUTE_DEFINITIONS + 1)
        .map(|id| {
            AttributeDefinition::new(
                AttributeId::new(u32::try_from(id).unwrap()).unwrap(),
                "attribute",
                AttributeDataType::U8,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        AttributeSchema::new(definitions.clone()),
        Err(ContractError::TooManyAttributeDefinitions {
            actual: MAX_ATTRIBUTE_DEFINITIONS + 1,
            max: MAX_ATTRIBUTE_DEFINITIONS,
        })
    );
    let encoded_schema = serde_json::to_value(RawAttributeSchema {
        definitions: &definitions,
    })
    .unwrap();
    assert!(serde_json::from_value::<AttributeSchema>(encoded_schema).is_err());

    let longest_wkt = "w".repeat(MAX_COORDINATE_REFERENCE_WKT_BYTES);
    assert_eq!(
        CoordinateReference::wkt(longest_wkt.clone())
            .unwrap()
            .as_wkt(),
        Some(longest_wkt.as_str())
    );
    let oversized_wkt = "w".repeat(MAX_COORDINATE_REFERENCE_WKT_BYTES + 1);
    assert_eq!(
        CoordinateReference::wkt(oversized_wkt.clone()),
        Err(ContractError::CoordinateReferenceWktTooLong {
            actual_bytes: MAX_COORDINATE_REFERENCE_WKT_BYTES + 1,
            max_bytes: MAX_COORDINATE_REFERENCE_WKT_BYTES,
        })
    );
    assert!(
        serde_json::from_value::<CoordinateReference>(json!({ "Wkt": oversized_wkt })).is_err()
    );
    assert_eq!(
        CoordinateReference::wkt("   "),
        Err(ContractError::EmptyCoordinateReferenceWkt)
    );
    assert!(serde_json::from_value::<CoordinateReference>(json!({ "Wkt": "" })).is_err());
    let wkt = CoordinateReference::wkt("LOCAL_CS[\"fixture\"]").unwrap();
    let encoded_wkt = serde_json::to_string(&wkt).unwrap();
    assert_eq!(
        serde_json::from_str::<CoordinateReference>(&encoded_wkt).unwrap(),
        wkt
    );
}

#[test]
fn metadata_record_limits_reject_oversized_input() {
    let oversized_namespace = "n".repeat(MAX_METADATA_NAMESPACE_BYTES + 1);
    assert_eq!(
        MetadataRecord::new(oversized_namespace.clone(), "name", Vec::new()),
        Err(ContractError::MetadataNamespaceTooLong {
            actual_bytes: MAX_METADATA_NAMESPACE_BYTES + 1,
            max_bytes: MAX_METADATA_NAMESPACE_BYTES,
        })
    );
    assert!(
        serde_json::from_value::<MetadataRecord>(json!({
            "namespace": oversized_namespace,
            "name": "name",
            "payload": [],
        }))
        .is_err()
    );

    let oversized_name = "n".repeat(MAX_METADATA_NAME_BYTES + 1);
    assert_eq!(
        MetadataRecord::new("namespace", oversized_name.clone(), Vec::new()),
        Err(ContractError::MetadataNameTooLong {
            actual_bytes: MAX_METADATA_NAME_BYTES + 1,
            max_bytes: MAX_METADATA_NAME_BYTES,
        })
    );
    assert!(
        serde_json::from_value::<MetadataRecord>(json!({
            "namespace": "namespace",
            "name": oversized_name,
            "payload": [],
        }))
        .is_err()
    );
    assert_eq!(
        MetadataRecord::new(
            "namespace",
            "name",
            vec![0; MAX_METADATA_RECORD_PAYLOAD_BYTES + 1],
        ),
        Err(ContractError::MetadataPayloadTooLong {
            actual_bytes: MAX_METADATA_RECORD_PAYLOAD_BYTES + 1,
            max_bytes: MAX_METADATA_RECORD_PAYLOAD_BYTES,
        })
    );
}

#[test]
fn source_metadata_and_provenance_limits_reject_oversized_input() {
    let transform = PositionTransform::new([0.0; 3], [1.0; 3]).unwrap();
    let oversized_format_name = "f".repeat(MAX_SOURCE_FORMAT_NAME_BYTES + 1);
    assert_eq!(
        SourceMetadata::new(
            0,
            transform,
            CoordinateReference::Unknown,
            AttributeSchema::empty(),
            None,
            oversized_format_name.clone(),
            Vec::new(),
        ),
        Err(ContractError::FormatNameTooLong {
            actual_bytes: MAX_SOURCE_FORMAT_NAME_BYTES + 1,
            max_bytes: MAX_SOURCE_FORMAT_NAME_BYTES,
        })
    );
    let oversized_format_metadata = RawSourceMetadata {
        point_count: 0,
        position_transform: transform,
        coordinate_reference: &CoordinateReference::Unknown,
        attributes: &AttributeSchema::empty(),
        world_bounds: None,
        format_name: &oversized_format_name,
        metadata_records: &[],
    };
    assert!(deserialize_raw_source_metadata(&oversized_format_metadata).is_err());

    let record = MetadataRecord::new("namespace", "name", Vec::new()).unwrap();
    let records = vec![record; MAX_METADATA_RECORDS + 1];
    assert_eq!(
        SourceMetadata::new(
            0,
            transform,
            CoordinateReference::Unknown,
            AttributeSchema::empty(),
            None,
            "memory",
            records.clone(),
        ),
        Err(ContractError::TooManyMetadataRecords {
            actual: MAX_METADATA_RECORDS + 1,
            max: MAX_METADATA_RECORDS,
        })
    );
    let encoded_metadata = serde_json::to_value(RawSourceMetadata {
        point_count: 0,
        position_transform: transform,
        coordinate_reference: &CoordinateReference::Unknown,
        attributes: &AttributeSchema::empty(),
        world_bounds: None,
        format_name: "memory",
        metadata_records: &records,
    })
    .unwrap();
    assert!(serde_json::from_value::<SourceMetadata>(encoded_metadata).is_err());

    let oversized_logical_order = "o".repeat(MAX_LOGICAL_ORDER_BYTES + 1);
    assert_eq!(
        SourceProvenance::new(
            SourceId::new([1; 32]),
            ContentHash::new([2; 32]),
            oversized_logical_order.clone(),
            1,
        ),
        Err(ContractError::LogicalOrderTooLong {
            actual_bytes: MAX_LOGICAL_ORDER_BYTES + 1,
            max_bytes: MAX_LOGICAL_ORDER_BYTES,
        })
    );
    let encoded_provenance = json!({
        "source": SourceId::new([1; 32]),
        "content_hash": ContentHash::new([2; 32]),
        "logical_order": oversized_logical_order,
        "contract_version": 1,
    });
    assert!(serde_json::from_value::<SourceProvenance>(encoded_provenance).is_err());
}

#[test]
fn point_batch_identity_is_stable_across_partitioning() {
    let source = SourceId::new([9; 32]);
    let transform = PositionTransform::new([0.0; 3], [0.01; 3]).unwrap();
    let positions =
        QuantizedPositions::new(transform, vec![[10, 20, 30], [11, 21, 31], [12, 22, 32]]).unwrap();
    let classification = AttributeColumn::new(
        definition(1, "classification", AttributeDataType::U8),
        AttributeValues::u8(vec![2, 2, 5]),
    )
    .unwrap();
    let attributes = AttributeColumns::new(vec![classification], 3).unwrap();
    let batch = PointBatch::new(source, 40, positions, attributes).unwrap();

    let identities = batch.point_ids().collect::<Vec<_>>();
    assert_eq!(
        identities,
        [
            PointId::new(source, 40),
            PointId::new(source, 41),
            PointId::new(source, 42),
        ]
    );
    assert_eq!(batch.first_ordinal(), 40);
    assert_eq!(batch.last_ordinal(), 42);
    assert_eq!(batch.estimated_payload_bytes(), 75);
    assert_eq!(batch.point_id(3), None);

    let left = batch.slice_rows(0..1).unwrap();
    let right = batch.slice_rows(1..3).unwrap();
    let partitioned = left
        .point_ids()
        .chain(right.point_ids())
        .collect::<Vec<_>>();
    assert_eq!(partitioned, identities);
}

#[test]
fn point_batch_rejects_misaligned_rows_and_ordinal_overflow() {
    let transform = PositionTransform::new([0.0; 3], [1.0; 3]).unwrap();
    let positions = QuantizedPositions::new(transform, vec![[0; 3], [1; 3]]).unwrap();
    assert_eq!(
        PointBatch::new(
            SourceId::new([1; 32]),
            0,
            positions.clone(),
            AttributeColumns::empty(1),
        ),
        Err(ContractError::PointBatchRowCountMismatch {
            positions: 2,
            attributes: 1,
        })
    );
    assert_eq!(
        PointBatch::new(
            SourceId::new([1; 32]),
            u64::MAX,
            positions,
            AttributeColumns::empty(2),
        ),
        Err(ContractError::PointOrdinalOverflow {
            first_ordinal: u64::MAX,
            point_count: 2,
        })
    );
}

fn attribute_id(value: u32) -> AttributeId {
    AttributeId::new(value).unwrap()
}

fn definition(id: u32, name: &str, data_type: AttributeDataType) -> AttributeDefinition {
    AttributeDefinition::new(attribute_id(id), name, data_type).unwrap()
}

#[derive(Serialize)]
struct RawAttributeSchema<'a> {
    definitions: &'a [AttributeDefinition],
}

#[derive(Serialize)]
struct RawSourceMetadata<'a> {
    point_count: u64,
    position_transform: PositionTransform,
    coordinate_reference: &'a CoordinateReference,
    attributes: &'a AttributeSchema,
    world_bounds: Option<WorldBounds>,
    format_name: &'a str,
    metadata_records: &'a [MetadataRecord],
}

fn deserialize_raw_source_metadata(
    metadata: &RawSourceMetadata<'_>,
) -> Result<SourceMetadata, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(metadata)?)
}

fn deserialize_attribute_values(
    encoded: &str,
    max_payload_bytes: u64,
) -> Result<AttributeValues, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(encoded);
    let values = AttributeValues::deserialize_bounded(&mut deserializer, max_payload_bytes)?;
    deserializer.end()?;
    Ok(values)
}
