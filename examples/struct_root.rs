use flatrecord::{
    DynamicRecord, FlatEnum, FlatRecord, PreparedSchema, RecordRoot, Schema, ValueRef,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq, FlatEnum)]
enum State {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, PartialEq, FlatRecord)]
struct Message {
    timestamp: u64,
    id: u64,
    state: State,
}

fn main() -> flatrecord::Result<()> {
    let message = Message {
        timestamp: 1_722_849_600,
        id: 42,
        state: State::Ready,
    };

    let mut record_bytes = [0; Message::PAYLOAD_SIZE];
    let written = message.encode_record(&mut record_bytes)?;
    let schema_bytes = wincode::serialize(&Message::schema())
        .map_err(|error| flatrecord::Error::Schema(format!("failed to encode schema: {error}")))?;

    dynamic_consumer(&schema_bytes, &record_bytes[..written])
}

// This function receives only schema and record bytes. It never names or
// deserializes `Message` or `State`.
fn dynamic_consumer(schema_bytes: &[u8], record_bytes: &[u8]) -> flatrecord::Result<()> {
    let schema: Schema = wincode::deserialize_exact(schema_bytes)
        .map_err(|error| flatrecord::Error::Schema(format!("failed to decode schema: {error}")))?;
    let prepared = PreparedSchema::new(schema)?;
    let record = DynamicRecord::read(&prepared, record_bytes)?;

    println!("{}", record.record_name());
    for field in record.fields() {
        match field.value()? {
            ValueRef::Enum(state) => println!(
                "{} = {}::{} ({})",
                field.name(),
                state.enum_name(),
                state.variant_name(),
                state.index(),
            ),
            value => println!("{} = {value:?}", field.name()),
        }
    }

    Ok(())
}
