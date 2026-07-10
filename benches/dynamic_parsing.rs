//! Dynamic-decoding comparison between flatrecord's packed wire format and
//! wincode-dynamic's native wincode wire format.
//!
//! These parsers cannot read each other's bytes. The benchmark intentionally
//! serializes the same logical record with each library, then measures each
//! library parsing its own encoded representation.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use flatrecord::{DynamicRecord, FlatRecord, PreparedSchema};
use wincode::{SchemaRead, SchemaReadContext, SchemaWrite, config::DefaultConfig, io::Reader};
use wincode_dynamic::{
    PrimitiveTy as WincodePrimitiveTy, SchemaDynamic, SchemaRuntime, Value as WincodeValue,
};

const ITERATIONS: usize = 1_000;

const FIXED_FIELD_TYPES: [WincodePrimitiveTy; 7] = [
    WincodePrimitiveTy::U64,
    WincodePrimitiveTy::U64,
    WincodePrimitiveTy::Bool,
    WincodePrimitiveTy::U32,
    WincodePrimitiveTy::F32,
    WincodePrimitiveTy::I64,
    WincodePrimitiveTy::U16,
];

#[derive(FlatRecord)]
#[record(max_size = 384)]
struct FlatRecordMessage {
    timestamp: u64,
    active: bool,
    count: u32,
    label: String,
    values: Vec<u64>,
}

#[derive(SchemaDynamic, SchemaRead, SchemaWrite)]
struct WincodeMessage {
    timestamp: u64,
    active: bool,
    count: u32,
    label: String,
    values: Vec<u64>,
}

#[derive(FlatRecord)]
struct FlatRecordFixedMessage {
    timestamp: u64,
    sequence: u64,
    active: bool,
    count: u32,
    rating: f32,
    delta: i64,
    shard: u16,
}

#[derive(SchemaDynamic, SchemaRead, SchemaWrite)]
struct WincodeFixedMessage {
    timestamp: u64,
    sequence: u64,
    active: bool,
    count: u32,
    rating: f32,
    delta: i64,
    shard: u16,
}

fn flatrecord_bytes() -> Vec<u8> {
    let message = FlatRecordMessage {
        timestamp: 1_722_849_600,
        active: true,
        count: 42,
        label: "a dynamic record with an owned string".to_owned(),
        values: (0..32).collect(),
    };
    let mut bytes = vec![0; message.payload_len()];
    let written = message.encode_payload(&mut bytes).unwrap();
    bytes.truncate(written);
    bytes
}

fn wincode_bytes() -> Vec<u8> {
    wincode::serialize(&WincodeMessage {
        timestamp: 1_722_849_600,
        active: true,
        count: 42,
        label: "a dynamic record with an owned string".to_owned(),
        values: (0..32).collect(),
    })
    .unwrap()
}

fn flatrecord_fixed_bytes() -> Vec<u8> {
    let message = FlatRecordFixedMessage {
        timestamp: 1_722_849_600,
        sequence: 987_654_321,
        active: true,
        count: 42,
        rating: 4.5,
        delta: -123_456,
        shard: 12,
    };
    let mut bytes = vec![0; message.payload_len()];
    let written = message.encode_payload(&mut bytes).unwrap();
    bytes.truncate(written);
    bytes
}

fn wincode_fixed_bytes() -> Vec<u8> {
    wincode::serialize(&WincodeFixedMessage {
        timestamp: 1_722_849_600,
        sequence: 987_654_321,
        active: true,
        count: 42,
        rating: 4.5,
        delta: -123_456,
        shard: 12,
    })
    .unwrap()
}

fn parse_flatrecord(prepared: &PreparedSchema, bytes: &[u8]) {
    let record = DynamicRecord::read(prepared, black_box(bytes)).unwrap();
    for field in record.fields() {
        black_box(field.value().unwrap());
    }
}

fn parse_wincode(runtime: &SchemaRuntime, bytes: &[u8]) {
    black_box(runtime.parse(black_box(bytes)).unwrap());
}

// This models a collection-free `SchemaRuntime::parse_each` API for the
// fixed-width schema below. `SchemaRuntime` does not currently expose its
// fields, so the benchmark uses the same public runtime type descriptors and
// dispatch implementation directly. It intentionally does not allocate the
// `Vec<Value>` produced by `SchemaRuntime::parse`.
fn parse_wincode_without_collection(bytes: &[u8]) {
    let mut reader = bytes;
    for ty in FIXED_FIELD_TYPES {
        let value = <WincodeValue as SchemaReadContext<DefaultConfig, _>>::get_with_context(
            ty,
            reader.by_ref(),
        )
        .unwrap();
        black_box(value);
    }
}

fn dynamic_parsing(c: &mut Criterion) {
    let flatrecord_bytes = flatrecord_bytes();
    let flatrecord_schema = PreparedSchema::new(FlatRecordMessage::schema()).unwrap();
    let wincode_bytes = wincode_bytes();
    let wincode_runtime = SchemaRuntime::new(WincodeMessage::schema());

    eprintln!(
        "dynamic parsing payload sizes: flatrecord = {} B; wincode-dynamic = {} B",
        flatrecord_bytes.len(),
        wincode_bytes.len(),
    );

    let mut group = c.benchmark_group("dynamic_parse_all_fields");
    group.throughput(Throughput::Bytes(
        (flatrecord_bytes.len() * ITERATIONS) as u64,
    ));
    group.bench_with_input(
        BenchmarkId::new("flatrecord", "parse_all_fields"),
        &(&flatrecord_schema, &flatrecord_bytes),
        |b, (prepared, bytes)| {
            b.iter(|| {
                for _ in 0..ITERATIONS {
                    parse_flatrecord(prepared, bytes);
                }
            })
        },
    );
    group.throughput(Throughput::Bytes((wincode_bytes.len() * ITERATIONS) as u64));
    group.bench_with_input(
        BenchmarkId::new("wincode_dynamic", "parse_all_fields"),
        &(&wincode_runtime, &wincode_bytes),
        |b, (runtime, bytes)| {
            b.iter(|| {
                for _ in 0..ITERATIONS {
                    parse_wincode(runtime, bytes);
                }
            })
        },
    );
    group.finish();
}

fn fixed_width_parsing(c: &mut Criterion) {
    let flatrecord_bytes = flatrecord_fixed_bytes();
    let flatrecord_schema = PreparedSchema::new(FlatRecordFixedMessage::schema()).unwrap();
    let wincode_bytes = wincode_fixed_bytes();
    let wincode_runtime = SchemaRuntime::new(WincodeFixedMessage::schema());

    eprintln!(
        "fixed-width parsing payload sizes: flatrecord = {} B; wincode-dynamic = {} B",
        flatrecord_bytes.len(),
        wincode_bytes.len(),
    );

    let mut group = c.benchmark_group("fixed_width_parse_all_fields");
    group.throughput(Throughput::Bytes(
        (flatrecord_bytes.len() * ITERATIONS) as u64,
    ));
    group.bench_with_input(
        BenchmarkId::new("flatrecord", "parse_all_fields"),
        &(&flatrecord_schema, &flatrecord_bytes),
        |b, (prepared, bytes)| {
            b.iter(|| {
                for _ in 0..ITERATIONS {
                    parse_flatrecord(prepared, bytes);
                }
            })
        },
    );
    group.throughput(Throughput::Bytes((wincode_bytes.len() * ITERATIONS) as u64));
    group.bench_with_input(
        BenchmarkId::new("wincode_dynamic", "parse_all_fields"),
        &(&wincode_runtime, &wincode_bytes),
        |b, (runtime, bytes)| {
            b.iter(|| {
                for _ in 0..ITERATIONS {
                    parse_wincode(runtime, bytes);
                }
            })
        },
    );
    group.throughput(Throughput::Bytes((wincode_bytes.len() * ITERATIONS) as u64));
    group.bench_with_input(
        BenchmarkId::new("wincode_dynamic", "parse_without_collection"),
        &wincode_bytes,
        |b, bytes| {
            b.iter(|| {
                for _ in 0..ITERATIONS {
                    parse_wincode_without_collection(bytes);
                }
            })
        },
    );
    group.finish();
}

criterion_group!(benches, dynamic_parsing, fixed_width_parsing);
criterion_main!(benches);
