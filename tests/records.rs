use flatrecord::{
    DynamicRecord, EnumDef, EnumVariantDef, Error, FieldDef, FieldIndex, FieldType, FlatEnum,
    FlatRecord, PreparedSchema, PrimitiveType, RecordDef, RecordRoot, RootDef, Schema, ValueRef,
};

fn u64_field(record: &DynamicRecord, index: FieldIndex) -> u64 {
    match record.field(index).unwrap().value().unwrap() {
        ValueRef::U64(value) => value,
        other => panic!("expected u64 field, got {other:?}"),
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, FlatEnum)]
pub enum EventKind {
    Created,
    Updated,
}

#[derive(Debug, PartialEq, FlatRecord)]
pub struct RecordType1 {
    pub timestamp: u64,
    pub value: u64,
    pub previous_value: u64,
    pub bytes: [u8; 32],
    pub kind: EventKind,
}

#[derive(Debug, PartialEq, FlatRecord)]
pub struct RecordType2 {
    pub timestamp: u64,
    pub value: u64,
    pub bytes: [u8; 32],
}

#[derive(Debug, PartialEq, FlatRecord)]
#[record(max_size = 128)]
pub struct RecordType3 {
    pub timestamp: u64,
    pub label: String,
    pub values: Vec<u64>,
    pub raw: Vec<u8>,
}

#[derive(Debug, PartialEq, FlatRecord)]
#[schema(version = 7)]
pub enum Record {
    Type1(RecordType1),
    Type2(RecordType2),
    Type3(RecordType3),
}

fn record_type1() -> RecordType1 {
    RecordType1 {
        timestamp: 123,
        value: 456,
        previous_value: 455,
        bytes: [9u8; 32],
        kind: EventKind::Created,
    }
}

fn record_type2() -> RecordType2 {
    RecordType2 {
        timestamp: 124,
        value: 456,
        bytes: [7u8; 32],
    }
}

fn record_type3() -> RecordType3 {
    RecordType3 {
        timestamp: 125,
        label: "example-record".to_owned(),
        values: vec![456, 457],
        raw: vec![1, 2, 3, 4],
    }
}

fn encode_type1(record: &RecordType1) -> [u8; 57] {
    let mut bytes = [0u8; 57];
    assert_eq!(record.encode_payload(&mut bytes).unwrap(), 57);
    bytes
}

fn encode_type2(record: &RecordType2) -> [u8; 48] {
    let mut bytes = [0u8; 48];
    assert_eq!(record.encode_payload(&mut bytes).unwrap(), 48);
    bytes
}

fn encode_type1_root(record: RecordType1) -> [u8; 59] {
    let mut bytes = [0u8; 59];
    assert_eq!(Record::Type1(record).encode_record(&mut bytes).unwrap(), 59);
    bytes
}

fn encode_type2_root(record: RecordType2) -> [u8; 50] {
    let mut bytes = [0u8; 50];
    assert_eq!(Record::Type2(record).encode_record(&mut bytes).unwrap(), 50);
    bytes
}

fn encode_type3(record: &RecordType3) -> ([u8; 128], usize) {
    let mut bytes = [0u8; 128];
    let written = record.encode_payload(&mut bytes).unwrap();
    (bytes, written)
}

fn encode_type3_root(record: RecordType3) -> ([u8; 130], usize) {
    let mut bytes = [0u8; 130];
    let written = Record::Type3(record).encode_record(&mut bytes).unwrap();
    (bytes, written)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn fixed_payload_sizes_are_exact() {
    assert_eq!(RecordType1::PAYLOAD_SIZE, 8 + 8 + 8 + 32 + 1);
    assert_eq!(RecordType2::PAYLOAD_SIZE, 8 + 8 + 32);

    let type1_bytes = encode_type1(&record_type1());
    let type2_bytes = encode_type2(&record_type2());

    assert_eq!(type1_bytes.len(), 57);
    assert_eq!(type2_bytes.len(), 48);
    assert_eq!(type1_bytes[56], 0);

    let type1_def = RecordType1::record_def();
    assert_eq!(type1_def.size(), Some(57));
    assert_eq!(
        type1_def
            .fields()
            .iter()
            .map(FieldDef::offset)
            .collect::<Vec<_>>(),
        vec![0, 8, 16, 24, 56]
    );
    match type1_def.fields()[4].ty() {
        FieldType::Enum(enum_def) => {
            assert_eq!(enum_def.name(), "EventKind");
            assert_eq!(enum_def.variants()[0].name(), "Created");
            assert_eq!(enum_def.variants()[0].index(), 0);
            assert_eq!(enum_def.variants()[1].name(), "Updated");
            assert_eq!(enum_def.variants()[1].index(), 1);
        }
        other => panic!("expected enum field, got {other:?}"),
    }
}

#[test]
fn enum_root_records_use_two_byte_tag_and_payload_only() {
    let bytes = encode_type1_root(record_type1());

    assert_eq!(bytes.len(), 2 + 57);
    assert_eq!(&bytes[..2], &0u16.to_le_bytes());
    assert_eq!(&bytes[2..10], &123u64.to_le_bytes());
    assert_eq!(&bytes[10..18], &456u64.to_le_bytes());
    assert_eq!(bytes[58], EventKind::Created.to_index());

    for forbidden in [
        b"timestamp".as_slice(),
        b"value".as_slice(),
        b"RecordType1".as_slice(),
        b"Record".as_slice(),
        b"EventKind".as_slice(),
        b"Created".as_slice(),
    ] {
        assert!(
            !contains_subslice(&bytes, forbidden),
            "record unexpectedly contained schema text {:?}",
            std::str::from_utf8(forbidden).unwrap()
        );
    }
}

#[test]
fn flat_enum_indices_follow_declaration_order() {
    assert_eq!(EventKind::Created.to_index(), 0);
    assert_eq!(EventKind::Updated.to_index(), 1);
    assert_eq!(EventKind::try_from_index(0), Some(EventKind::Created));
    assert_eq!(EventKind::try_from_index(1), Some(EventKind::Updated));
    assert_eq!(EventKind::try_from_index(2), None);
}

#[test]
fn typed_struct_roundtrip_works() {
    let record = record_type1();
    let bytes = encode_type1(&record);
    let decoded = RecordType1::from_record_bytes(&bytes).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn typed_enum_roundtrip_works() {
    let record = Record::Type2(record_type2());
    let bytes = encode_type2_root(record_type2());
    let decoded = Record::from_record_bytes(&bytes).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn typed_dynamic_roundtrip_uses_trailing_data() {
    let record = record_type3();
    let (bytes, written) = encode_type3(&record);

    assert_eq!(RecordType3::PAYLOAD_SIZE, 128);
    assert_eq!(written, 8 + 8 + 8 + 8 + 14 + 16 + 4);

    let decoded = RecordType3::from_record_bytes(&bytes[..written]).unwrap();
    assert_eq!(decoded, record);

    let def = RecordType3::record_def();
    assert_eq!(def.size(), Some(128));
    assert_eq!(def.fields()[1].offset(), 8);
    assert_eq!(def.fields()[1].size(), Some(8));
    assert_eq!(def.fields()[1].ty(), &FieldType::String);
    assert_eq!(
        def.fields()[2].ty(),
        &FieldType::Vec {
            elem: PrimitiveType::U64
        }
    );
    assert_eq!(
        def.fields()[3].ty(),
        &FieldType::Vec {
            elem: PrimitiveType::U8
        }
    );
}

#[test]
fn dynamic_payloads_reject_max_size_overflow() {
    let record = RecordType3 {
        timestamp: 125,
        label: "x".repeat(200),
        values: vec![],
        raw: vec![],
    };
    let mut bytes = [0u8; 256];

    assert!(matches!(
        record.encode_payload(&mut bytes),
        Err(Error::PayloadTooLarge {
            max: 128,
            actual: 232
        })
    ));
}

#[test]
fn decode_ignores_trailing_payload_bytes() {
    let record = record_type3();
    let (mut bytes, written) = encode_type3(&record);
    bytes[written] = 99;

    let decoded = RecordType3::from_record_bytes(&bytes[..written + 1]).unwrap();
    assert_eq!(decoded, record);

    let decoded = RecordType3::from_record_bytes(&bytes).unwrap();
    assert_eq!(decoded, record);

    let schema = Record::schema();
    let prepared = PreparedSchema::new(schema).unwrap();
    let timestamp = prepared.schema().records()[2]
        .field_index("timestamp")
        .unwrap();
    let (mut root, root_written) = encode_type3_root(record_type3());
    root[root_written] = 99;
    let decoded = DynamicRecord::read(&prepared, &root[..root_written + 1]).unwrap();
    assert_eq!(u64_field(&decoded, timestamp), 125);
}

#[test]
fn dynamic_enum_root_decode_works_from_exported_schema() {
    let schema = Record::schema();
    let record_def = &schema.records()[0];
    let value = record_def.field_index("value").unwrap();
    let bytes = encode_type1_root(record_type1());

    let prepared = PreparedSchema::new(schema).unwrap();
    let decoded = DynamicRecord::read(&prepared, &bytes).unwrap();

    assert_eq!(decoded.record_type(), 0);
    assert_eq!(decoded.record_name(), "RecordType1");
    assert_eq!(u64_field(&decoded, value), 456);

    let fields = decoded
        .fields()
        .map(|field| (field.name().to_owned(), field.value().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(fields[0], ("timestamp".to_owned(), ValueRef::U64(123)));
    assert_eq!(fields[1], ("value".to_owned(), ValueRef::U64(456)));
    assert_eq!(fields[2], ("previous_value".to_owned(), ValueRef::U64(455)));
    assert_eq!(fields[3], ("bytes".to_owned(), ValueRef::Bytes(&[9u8; 32])));
    assert_eq!(fields[4].0, "kind");
    match fields[4].1 {
        ValueRef::Enum(value) => {
            assert_eq!(value.enum_name(), "EventKind");
            assert_eq!(value.variant_name(), "Created");
            assert_eq!(value.index(), 0);
        }
        other => panic!("expected enum value, got {other:?}"),
    }
}

#[test]
fn dynamic_reflection_reads_trailing_string_and_vectors() {
    let schema = Record::schema();
    let (bytes, written) = encode_type3_root(record_type3());

    let prepared = PreparedSchema::new(schema).unwrap();
    let decoded = DynamicRecord::read(&prepared, &bytes[..written]).unwrap();

    assert_eq!(decoded.record_type(), 2);
    assert_eq!(decoded.record_name(), "RecordType3");

    let fields = decoded
        .fields()
        .map(|field| (field.name().to_owned(), field.value().unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(fields[0], ("timestamp".to_owned(), ValueRef::U64(125)));
    assert_eq!(
        fields[1],
        ("label".to_owned(), ValueRef::Str("example-record"))
    );
    assert_eq!(
        fields[2],
        (
            "values".to_owned(),
            ValueRef::ArrayBytes(&[200, 1, 0, 0, 0, 0, 0, 0, 201, 1, 0, 0, 0, 0, 0, 0])
        )
    );
    assert_eq!(
        fields[3],
        ("raw".to_owned(), ValueRef::Bytes(&[1, 2, 3, 4]))
    );
}

#[test]
fn dynamic_reflection_validates_string_utf8_on_access() {
    let schema = Record::schema();
    let prepared = PreparedSchema::new(schema).unwrap();
    let record_def = &prepared.schema().records()[2];
    let timestamp = record_def.field_index("timestamp").unwrap();
    let label = record_def.field_index("label").unwrap();
    let values = record_def.field_index("values").unwrap();

    let (mut bytes, written) = encode_type3(&record_type3());
    bytes[32] = 0xff;
    assert!(matches!(
        RecordType3::from_record_bytes(&bytes[..written]),
        Err(Error::InvalidUtf8 { field }) if field == "label"
    ));

    // Same corruption in a tag-prefixed record, read through the dynamic path.
    let (mut root, root_written) = encode_type3_root(record_type3());
    root[2 + 32] = 0xff;
    let decoded = DynamicRecord::read(&prepared, &root[..root_written]).unwrap();
    assert_eq!(u64_field(&decoded, timestamp), 125);
    assert!(matches!(
        decoded.field(values).unwrap().value().unwrap(),
        ValueRef::ArrayBytes(_)
    ));
    assert!(matches!(
        decoded.field(label).unwrap().value(),
        Err(Error::InvalidUtf8 { field }) if field == "label"
    ));
}

#[test]
fn dynamic_field_access_rejects_ranges_in_trailing_cell_bytes() {
    let schema = Record::schema();
    let prepared = PreparedSchema::new(schema).unwrap();
    let values = prepared.schema().records()[2]
        .field_index("values")
        .unwrap();
    let (bytes, written) = encode_type3(&record_type3());
    let mut cell = vec![0u8; 256];
    cell[..written].copy_from_slice(&bytes[..written]);
    cell[16..20].copy_from_slice(&128u32.to_le_bytes());
    cell[20..24].copy_from_slice(&1u32.to_le_bytes());

    assert!(matches!(
        RecordType3::from_record_bytes(&cell),
        Err(Error::FieldOutOfBounds {
            field,
            payload_len: 128,
            ..
        }) if field == "values"
    ));

    // The same tampered payload, tag-prefixed and read through the dynamic path.
    let mut tagged = vec![0u8; 2 + cell.len()];
    tagged[..2].copy_from_slice(&2u16.to_le_bytes());
    tagged[2..].copy_from_slice(&cell);
    let decoded = DynamicRecord::read(&prepared, &tagged).unwrap();
    assert!(matches!(
        decoded.field(values).unwrap().value(),
        Err(Error::FieldOutOfBounds {
            field,
            payload_len: 128,
            ..
        }) if field == "values"
    ));
}

#[test]
fn dynamic_struct_decode_rejects_fixed_field_outside_payload() {
    // A struct-root schema whose only field ends at offset 16, decoded against an
    // 8-byte payload: the recomputed header size must reject it before any read.
    let schema = Schema::from_parts(
        1,
        RootDef::Struct,
        vec![RecordDef::new(
            "BadRecord".to_owned(),
            None,
            vec![FieldDef::new(
                "value".to_owned(),
                FieldType::U64,
                8,
                Some(8),
            )],
        )],
    )
    .unwrap();
    let prepared = PreparedSchema::new(schema).unwrap();
    let payload = [0u8; 8];

    assert!(matches!(
        DynamicRecord::read(&prepared, &payload),
        Err(Error::PayloadTooShort {
            minimum: 16,
            actual: 8
        })
    ));
}

#[test]
fn invalid_enum_indexes_are_rejected_on_access() {
    let mut payload = encode_type1(&record_type1());
    payload[56] = 99;
    assert!(matches!(
        RecordType1::from_record_bytes(&payload),
        Err(Error::InvalidEnum {
            field,
            enum_name,
            value: 99,
        }) if field == "kind" && enum_name == "EventKind"
    ));

    let schema = Record::schema();
    let prepared = PreparedSchema::new(schema).unwrap();
    let kind = prepared.schema().records()[0].field_index("kind").unwrap();
    let mut record = encode_type1_root(record_type1());
    record[58] = 99;
    assert!(matches!(
        Record::from_record_bytes(&record),
        Err(Error::InvalidEnum {
            field,
            enum_name,
            value: 99,
        }) if field == "kind" && enum_name == "EventKind"
    ));
    let decoded = DynamicRecord::read(&prepared, &record).unwrap();
    assert!(matches!(
        decoded.field(kind).unwrap().value(),
        Err(Error::InvalidEnum {
            field,
            enum_name,
            value: 99,
        }) if field == "kind" && enum_name == "EventKind"
    ));
}

#[test]
fn schema_rejects_duplicate_enum_indexes() {
    assert!(matches!(
        Schema::from_parts(
            1,
            RootDef::Struct,
            vec![RecordDef::new(
                "BadRecord".to_owned(),
                Some(1),
                vec![FieldDef::new(
                    "kind".to_owned(),
                    FieldType::Enum(EnumDef::new(
                        "EventKind".to_owned(),
                        vec![
                            EnumVariantDef::new("Created".to_owned(), 0),
                            EnumVariantDef::new("Updated".to_owned(), 0),
                        ],
                    )),
                    0,
                    Some(1),
                )],
            )],
        ),
        Err(Error::Schema(message)) if message.contains("duplicate variant index 0")
    ));
}

#[test]
fn undersized_payload_lengths_are_rejected() {
    let payload = encode_type1(&record_type1());

    assert!(matches!(
        RecordType1::from_record_bytes(&payload[..56]),
        Err(Error::UnexpectedLength {
            expected: 57,
            actual: 56
        })
    ));

    let mut record_bytes = [0u8; 60];
    let record = Record::Type1(record_type1());
    let written = Record::Type1(record_type1())
        .encode_record(&mut record_bytes[..59])
        .unwrap();
    assert_eq!(written, 59);

    let decoded = Record::from_record_bytes(&record_bytes).unwrap();
    assert_eq!(decoded, record);

    let schema = Record::schema();
    let prepared = PreparedSchema::new(schema).unwrap();
    let decoded = DynamicRecord::read(&prepared, &record_bytes).unwrap();
    assert_eq!(decoded.record_type(), 0);
}

#[test]
fn incorrect_record_tags_are_rejected() {
    let mut record = encode_type1_root(record_type1());
    record[..2].copy_from_slice(&99u16.to_le_bytes());

    assert!(matches!(
        Record::from_record_bytes(&record),
        Err(Error::UnknownRecordTag(99))
    ));

    let schema = Record::schema();
    let prepared = PreparedSchema::new(schema).unwrap();
    assert!(matches!(
        DynamicRecord::read(&prepared, &record),
        Err(Error::UnknownRecordTag(99))
    ));
}

#[test]
fn undersized_write_buffers_are_rejected() {
    let mut payload = [0u8; 56];
    assert!(matches!(
        record_type1().encode_payload(&mut payload),
        Err(Error::BufferTooSmall {
            required: 57,
            actual: 56
        })
    ));

    let mut record = [0u8; 58];
    assert!(matches!(
        Record::Type1(record_type1()).encode_record(&mut record),
        Err(Error::BufferTooSmall {
            required: 59,
            actual: 58
        })
    ));
}

#[test]
fn schema_binary_roundtrips() {
    let schema = Record::schema();
    assert_eq!(Record::SCHEMA_VERSION, 7);
    assert_eq!(schema.schema_version(), 7);

    let bytes = wincode::serialize(&schema).unwrap();
    let decoded: Schema = wincode::deserialize_exact(&bytes).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, schema);
}

#[test]
fn deserialized_schema_decodes_and_bounds_reads() {
    // Preparing a deserialized schema derives every layout from its fields, so a schema
    // round-tripped through wincode decodes correctly and rejects a too-short payload
    // rather than reading out of bounds.
    let schema = Record::schema();
    let bytes = wincode::serialize(&schema).unwrap();
    let decoded: Schema = wincode::deserialize_exact(&bytes).unwrap();
    let prepared = PreparedSchema::new(decoded).unwrap();

    let (record_bytes, written) = encode_type3_root(record_type3());
    let record = DynamicRecord::read(&prepared, &record_bytes[..written]).unwrap();
    assert_eq!(record.record_name(), "RecordType3");

    // A payload shorter than the recomputed header is rejected, not read out of bounds.
    assert!(matches!(
        DynamicRecord::read(&prepared, &record_bytes[..4]),
        Err(Error::PayloadTooShort { .. })
    ));
}
