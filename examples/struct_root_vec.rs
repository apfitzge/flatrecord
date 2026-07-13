use flatrecord::{DynamicRecord, FlatRecord, PreparedSchema, RecordRoot, Schema, ValueRef};

#[derive(Debug, PartialEq, FlatRecord)]
#[record(max_size = 128)]
struct Message {
    timestamp: u64,
    id: u64,
    values: Vec<u64>,
}

fn main() -> flatrecord::Result<()> {
    let message = Message {
        timestamp: 1_722_849_600,
        id: 42,
        values: vec![100, 200, 300],
    };

    let mut record_bytes = [0; Message::PAYLOAD_SIZE];
    let written = message.encode_record(&mut record_bytes)?;
    let schema_bytes = wincode::serialize(&Message::schema())
        .map_err(|error| flatrecord::Error::Schema(format!("failed to encode schema: {error}")))?;

    dynamic_consumer(&schema_bytes, &record_bytes[..written])
}

// This function receives only schema and record bytes. The values field is
// exposed as a borrowed byte slice, so the consumer chooses when to decode it.
fn dynamic_consumer(schema_bytes: &[u8], record_bytes: &[u8]) -> flatrecord::Result<()> {
    let schema: Schema = wincode::deserialize_exact(schema_bytes)
        .map_err(|error| flatrecord::Error::Schema(format!("failed to decode schema: {error}")))?;
    let prepared = PreparedSchema::new(schema)?;
    let record = DynamicRecord::read(&prepared, record_bytes)?;

    println!("{}", record.record_name());
    for field in record.fields() {
        match field.value()? {
            ValueRef::ArrayBytes(bytes) if field.name() == "values" => {
                let values = bytes
                    .chunks_exact(size_of::<u64>())
                    .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("u64-sized chunk")))
                    .collect::<Vec<_>>();
                println!("{} = {values:?}", field.name());
            }
            value => println!("{} = {value:?}", field.name()),
        }
    }

    Ok(())
}
