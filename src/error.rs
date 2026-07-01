use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unexpected payload length: expected {expected}, got {actual}")]
    UnexpectedLength { expected: usize, actual: usize },
    #[error("buffer too small: need {required} bytes, got {actual}")]
    BufferTooSmall { required: usize, actual: usize },
    #[error("payload too large: max {max} bytes, got {actual}")]
    PayloadTooLarge { max: usize, actual: usize },
    #[error("payload too short: need at least {minimum} bytes, got {actual}")]
    PayloadTooShort { minimum: usize, actual: usize },
    #[error("unknown record tag {0}")]
    UnknownRecordTag(u16),
    #[error("unknown field `{0}`")]
    UnknownField(String),
    #[error(
        "field `{field}` is out of bounds: offset {offset}, size {size}, payload length {payload_len}"
    )]
    FieldOutOfBounds {
        field: String,
        offset: u32,
        size: u32,
        payload_len: usize,
    },
    #[error("field `{field}` has type {actual}, expected {expected}")]
    TypeMismatch {
        field: String,
        expected: &'static str,
        actual: &'static str,
    },
    #[error("field `{field}` has invalid bool byte {value}")]
    InvalidBool { field: String, value: u8 },
    #[error("field `{field}` contains invalid UTF-8")]
    InvalidUtf8 { field: String },
    #[error("{0}")]
    Schema(String),
}
