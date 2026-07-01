use crate::schema::{
    FieldDef, FieldIndex, FieldType, PrimitiveType, RecordDef, RecordLayout, RootDef,
};
use crate::{Error, Result, Schema};

/// A `Schema` with its per-record layouts computed and validated up front, so that
/// decoding is a plain lookup rather than a recompute. Build one per schema with
/// [`PreparedSchema::new`] and reuse it across every record you decode.
#[derive(Debug)]
pub struct PreparedSchema {
    schema: Schema,
    // Invariant: exactly one entry per `schema.records()`, in the same order. Both
    // fields are immutable after construction, so `read` indexes `layouts` unchecked
    // using an index already bounds-checked against `records`.
    layouts: Box<[RecordLayout]>,
}

impl PreparedSchema {
    /// Validates the schema and precomputes each record's layout, taking ownership so
    /// the result can be stored and passed around freely. Returns an error if the
    /// schema is malformed (see [`Schema::validate`]).
    #[inline]
    pub fn new(schema: Schema) -> Result<Self> {
        schema.validate()?;
        let layouts = schema
            .records()
            .iter()
            .map(RecordDef::layout)
            .collect::<Result<Vec<_>>>()?
            .into_boxed_slice();
        Ok(Self { schema, layouts })
    }

    #[inline(always)]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }
}

#[derive(Debug)]
pub struct DynamicRecord<'schema, 'data> {
    record_type: u16,
    record: &'schema RecordDef,
    payload: &'data [u8],
    payload_limit: usize,
}

#[derive(Debug)]
pub struct FieldRef<'schema, 'data> {
    field: &'schema FieldDef,
    payload: &'data [u8],
    payload_limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValueRef<'a> {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    Bytes(&'a [u8]),
    Str(&'a str),
    ArrayBytes(&'a [u8]),
}

pub struct FieldIter<'schema, 'data> {
    inner: std::slice::Iter<'schema, FieldDef>,
    payload: &'data [u8],
    payload_limit: usize,
}

impl<'schema, 'data> DynamicRecord<'schema, 'data> {
    #[inline(always)]
    pub fn read(prepared: &'schema PreparedSchema, bytes: &'data [u8]) -> Result<Self> {
        let schema = &prepared.schema;
        match schema.root() {
            RootDef::Struct => {
                let record_def = schema.records().first().ok_or_else(|| {
                    Error::Schema("root struct schema does not contain a record".to_owned())
                })?;
                let layout = unsafe {
                    // SAFETY: `layouts` has exactly one entry per record (see
                    // PreparedSchema::new) and both are immutable, so index 0 is in
                    // bounds because `records[0]` was.
                    prepared.layouts.get_unchecked(0)
                };
                let payload_limit = validate_payload_len(layout, record_def.size(), bytes)?;
                Ok(Self {
                    record_type: 0,
                    record: record_def,
                    payload: bytes,
                    payload_limit,
                })
            }
            RootDef::TaggedUnion { .. } => {
                let (tag_bytes, payload) =
                    bytes.split_at_checked(2).ok_or(Error::PayloadTooShort {
                        minimum: 2,
                        actual: bytes.len(),
                    })?;

                let tag = unsafe {
                    // SAFETY: tag_bytes.len() == 2 was checked above, and read_unaligned is
                    // correct for the packed little-endian wire tag.
                    u16::from_le(tag_bytes.as_ptr().cast::<u16>().read_unaligned())
                };
                let record_def = schema
                    .records()
                    .get(tag as usize)
                    .ok_or(Error::UnknownRecordTag(tag))?;
                let layout = unsafe {
                    // SAFETY: `layouts` has exactly one entry per record (see
                    // PreparedSchema::new) and both are immutable, so this index is in
                    // bounds because `records[tag]` was.
                    prepared.layouts.get_unchecked(tag as usize)
                };
                let payload_limit = validate_payload_len(layout, record_def.size(), payload)?;
                Ok(Self {
                    record_type: tag,
                    record: record_def,
                    payload,
                    payload_limit,
                })
            }
        }
    }

    #[inline(always)]
    pub fn record_type(&self) -> u16 {
        self.record_type
    }

    #[inline(always)]
    pub fn record_name(&self) -> &str {
        self.record.name()
    }

    #[inline]
    pub fn record_def(&self) -> &RecordDef {
        self.record
    }

    #[inline]
    pub fn payload(&self) -> &'data [u8] {
        self.payload
    }

    #[inline(always)]
    pub fn fields(&self) -> FieldIter<'schema, 'data> {
        FieldIter {
            inner: self.record.fields().iter(),
            payload: self.payload,
            payload_limit: self.payload_limit,
        }
    }

    #[inline]
    pub fn field(&self, index: FieldIndex) -> Result<FieldRef<'schema, 'data>> {
        let field = self
            .record
            .field(index)
            .ok_or_else(|| Error::UnknownField(format!("#{}", index.get())))?;
        Ok(FieldRef {
            field,
            payload: self.payload,
            payload_limit: self.payload_limit,
        })
    }
}

impl<'schema, 'data> Iterator for FieldIter<'schema, 'data> {
    type Item = FieldRef<'schema, 'data>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|field| FieldRef {
            field,
            payload: self.payload,
            payload_limit: self.payload_limit,
        })
    }
}

impl<'schema, 'data> FieldRef<'schema, 'data> {
    #[inline(always)]
    pub fn name(&self) -> &str {
        self.field.name()
    }

    #[inline]
    pub fn def(&self) -> &FieldDef {
        self.field
    }

    #[inline(always)]
    pub fn value(&self) -> Result<ValueRef<'data>> {
        read_value(self.field, self.payload, self.payload_limit)
    }
}

impl ValueRef<'_> {
    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::U8(_) => "u8",
            Self::U16(_) => "u16",
            Self::U32(_) => "u32",
            Self::U64(_) => "u64",
            Self::I8(_) => "i8",
            Self::I16(_) => "i16",
            Self::I32(_) => "i32",
            Self::I64(_) => "i64",
            Self::F32(_) => "f32",
            Self::F64(_) => "f64",
            Self::Bool(_) => "bool",
            Self::Bytes(_) => "bytes",
            Self::Str(_) => "str",
            Self::ArrayBytes(_) => "array_bytes",
        }
    }
}

#[inline(always)]
fn validate_payload_len(layout: &RecordLayout, size: Option<u32>, payload: &[u8]) -> Result<usize> {
    if let Some(fixed) = layout.fixed_payload_size {
        let expected = fixed as usize;
        if payload.len() < expected {
            return Err(Error::UnexpectedLength {
                expected,
                actual: payload.len(),
            });
        }
        return Ok(expected);
    }

    let header_size = layout.header_size as usize;

    if payload.len() < header_size {
        return Err(Error::PayloadTooShort {
            minimum: header_size,
            actual: payload.len(),
        });
    }

    Ok(size.map_or(payload.len(), |size| payload.len().min(size as usize)))
}

#[inline(always)]
fn read_value<'a>(
    field: &FieldDef,
    payload: &'a [u8],
    payload_limit: usize,
) -> Result<ValueRef<'a>> {
    let ptr = unsafe {
        // SAFETY: DynamicRecord construction calls validate_payload_len, which checks
        // payload.len() >= header_size. header_size is the maximum of every fixed
        // field's offset + size (RecordLayout::from_fields), so this field's
        // [offset, offset + size) range is contained in payload before any FieldRef
        // can exist.
        payload.as_ptr().add(field.offset() as usize)
    };
    match field.ty() {
        FieldType::U8 => Ok(ValueRef::U8(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            ptr.read_unaligned()
        })),
        FieldType::U16 => Ok(ValueRef::U16(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            u16::from_le(ptr.cast::<u16>().read_unaligned())
        })),
        FieldType::U32 => Ok(ValueRef::U32(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            u32::from_le(ptr.cast::<u32>().read_unaligned())
        })),
        FieldType::U64 => Ok(ValueRef::U64(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            u64::from_le(ptr.cast::<u64>().read_unaligned())
        })),
        FieldType::I8 => Ok(ValueRef::I8(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            ptr.read_unaligned() as i8
        })),
        FieldType::I16 => Ok(ValueRef::I16(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            i16::from_le(ptr.cast::<i16>().read_unaligned())
        })),
        FieldType::I32 => Ok(ValueRef::I32(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            i32::from_le(ptr.cast::<i32>().read_unaligned())
        })),
        FieldType::I64 => Ok(ValueRef::I64(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            i64::from_le(ptr.cast::<i64>().read_unaligned())
        })),
        FieldType::F32 => Ok(ValueRef::F32(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            f32::from_bits(u32::from_le(ptr.cast::<u32>().read_unaligned()))
        })),
        FieldType::F64 => Ok(ValueRef::F64(unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            f64::from_bits(u64::from_le(ptr.cast::<u64>().read_unaligned()))
        })),
        FieldType::Bool => match unsafe {
            // SAFETY: DynamicRecord validated this field's fixed-width range.
            ptr.read_unaligned()
        } {
            0 => Ok(ValueRef::Bool(false)),
            1 => Ok(ValueRef::Bool(true)),
            value => Err(Error::InvalidBool {
                field: field.name().to_owned(),
                value,
            }),
        },
        FieldType::FixedArray {
            elem: PrimitiveType::U8,
            len,
        } => {
            let bytes = unsafe {
                // SAFETY: DynamicRecord validated this field's fixed-width range.
                std::slice::from_raw_parts(ptr, *len as usize)
            };
            Ok(ValueRef::Bytes(bytes))
        }
        FieldType::FixedArray { elem, len } => {
            let byte_len = *len as usize * elem.size() as usize;
            let bytes = unsafe {
                // SAFETY: DynamicRecord validated this field's fixed-width range.
                std::slice::from_raw_parts(ptr, byte_len)
            };
            Ok(ValueRef::ArrayBytes(bytes))
        }
        FieldType::String => {
            let bytes = dynamic_field_bytes(field, payload, payload_limit)?;
            let value = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8 {
                field: field.name().to_owned(),
            })?;
            Ok(ValueRef::Str(value))
        }
        FieldType::Vec { elem } => {
            let bytes = dynamic_field_bytes(field, payload, payload_limit)?;
            read_array_value(*elem, bytes)
        }
    }
}

#[inline]
fn read_array_value<'a>(elem: PrimitiveType, bytes: &'a [u8]) -> Result<ValueRef<'a>> {
    match elem {
        PrimitiveType::U8 => Ok(ValueRef::Bytes(bytes)),
        PrimitiveType::U16
        | PrimitiveType::U32
        | PrimitiveType::U64
        | PrimitiveType::I8
        | PrimitiveType::I16
        | PrimitiveType::I32
        | PrimitiveType::I64
        | PrimitiveType::F32
        | PrimitiveType::F64 => Ok(ValueRef::ArrayBytes(bytes)),
    }
}

#[inline]
fn dynamic_field_bytes<'a>(
    field: &FieldDef,
    payload: &'a [u8],
    payload_limit: usize,
) -> Result<&'a [u8]> {
    Ok(dynamic_field_range(field, payload, payload_limit)?.0)
}

#[inline]
fn dynamic_field_range<'a>(
    field: &FieldDef,
    payload: &'a [u8],
    payload_limit: usize,
) -> Result<(&'a [u8], usize)> {
    let elem_size = match field.ty() {
        FieldType::String => 1usize,
        FieldType::Vec { elem } => elem.size() as usize,
        _ => {
            return Err(Error::Schema(format!(
                "field `{}` is not dynamically sized",
                field.name()
            )));
        }
    };
    let start = field.offset() as usize;
    let end = start.checked_add(8).ok_or_else(|| {
        Error::Schema(format!(
            "field `{}` offset plus size overflows",
            field.name()
        ))
    })?;
    let descriptor = payload
        .get(start..end)
        .ok_or_else(|| Error::FieldOutOfBounds {
            field: field.name().to_owned(),
            offset: field.offset(),
            size: 8,
            payload_len: payload.len(),
        })?;

    let offset = unsafe {
        // SAFETY: descriptor.len() == 8 was checked above.
        u32::from_le(descriptor.as_ptr().cast::<u32>().read_unaligned())
    };
    let len = unsafe {
        // SAFETY: descriptor.len() == 8 was checked above.
        u32::from_le(descriptor.as_ptr().add(4).cast::<u32>().read_unaligned())
    };
    let byte_len = (len as usize).checked_mul(elem_size).ok_or_else(|| {
        Error::Schema(format!("field `{}` dynamic length overflows", field.name()))
    })?;
    let start = offset as usize;
    let end = start.checked_add(byte_len).ok_or_else(|| {
        Error::Schema(format!("field `{}` dynamic offset overflows", field.name()))
    })?;
    if end > payload_limit {
        return Err(Error::FieldOutOfBounds {
            field: field.name().to_owned(),
            offset,
            size: u32::try_from(byte_len).unwrap_or(u32::MAX),
            payload_len: payload_limit,
        });
    }

    payload
        .get(start..end)
        .map(|bytes| (bytes, end))
        .ok_or_else(|| Error::FieldOutOfBounds {
            field: field.name().to_owned(),
            offset,
            size: u32::try_from(byte_len).unwrap_or(u32::MAX),
            payload_len: payload.len(),
        })
}
