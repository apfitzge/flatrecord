#![deny(unsafe_op_in_unsafe_fn)]

pub mod dynamic;
pub mod error;
pub mod schema;

pub use dynamic::{DynamicRecord, FieldRef, PreparedSchema, ValueRef};
pub use error::{Error, Result};
pub use flatrecord_derive::FlatRecord;
pub use schema::{FieldDef, FieldIndex, FieldType, PrimitiveType, RecordDef, RootDef, Schema};

pub trait FlatRecord: Sized {
    const RECORD_NAME: &'static str;
    const PAYLOAD_SIZE: usize;

    fn record_def() -> RecordDef;
    fn encode_payload(&self, dst: &mut [u8]) -> Result<usize>;
    fn decode_payload(src: &[u8]) -> Result<Self>;

    fn payload_len(&self) -> usize {
        Self::PAYLOAD_SIZE
    }

    fn from_record_bytes(src: &[u8]) -> Result<Self> {
        Self::decode_payload(src)
    }
}

pub trait RecordRoot: Sized {
    fn root_def() -> RootDef;
    fn record_defs() -> Vec<RecordDef>;
    fn record_len(&self) -> usize;
    fn encode_record(&self, dst: &mut [u8]) -> Result<usize>;
    fn decode_record(src: &[u8]) -> Result<Self>;
}

impl<T: FlatRecord> RecordRoot for T {
    fn root_def() -> RootDef {
        RootDef::Struct
    }

    fn record_defs() -> Vec<RecordDef> {
        vec![T::record_def()]
    }

    fn record_len(&self) -> usize {
        self.payload_len()
    }

    fn encode_record(&self, dst: &mut [u8]) -> Result<usize> {
        self.encode_payload(dst)
    }

    fn decode_record(src: &[u8]) -> Result<Self> {
        T::decode_payload(src)
    }
}
