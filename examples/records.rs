use flatrecord::{DynamicRecord, FlatEnum, FlatRecord, PreparedSchema, RecordRoot, Schema};

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
pub enum Record {
    Type1(RecordType1),
    Type2(RecordType2),
}

fn main() -> flatrecord::Result<()> {
    let record = Record::Type1(RecordType1 {
        timestamp: 123,
        value: 456,
        previous_value: 455,
        bytes: [0; 32],
        kind: EventKind::Created,
    });

    let mut bytes = [0u8; 59];
    let written = record.encode_record(&mut bytes)?;
    let schema = Record::schema();
    let schema_bytes = wincode::serialize(&schema)
        .map_err(|error| flatrecord::Error::Schema(format!("failed to encode schema: {error}")))?;

    dynamic_consumer(&schema_bytes, &bytes[..written])?;
    Ok(())
}

fn dynamic_consumer(schema_bytes: &[u8], record_bytes: &[u8]) -> flatrecord::Result<()> {
    let runtime_schema: Schema = wincode::deserialize_exact(schema_bytes)
        .map_err(|error| flatrecord::Error::Schema(format!("failed to decode schema: {error}")))?;
    let prepared = PreparedSchema::new(runtime_schema)?;
    let decoded = DynamicRecord::read(&prepared, record_bytes)?;

    println!("{}", decoded.record_name());
    for field in decoded.fields() {
        println!("{} = {:?}", field.name(), field.value()?);
    }

    Ok(())
}
