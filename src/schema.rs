use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
pub struct Schema {
    schema_version: u32,
    root: RootDef,
    records: Vec<RecordDef>,
}

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
#[wincode(tag_encoding = "u8")]
pub enum RootDef {
    Struct,
    TaggedUnion { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
pub struct RecordDef {
    name: String,
    size: Option<u32>,
    fields: Box<[FieldDef]>,
}

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
pub struct FieldDef {
    name: String,
    ty: FieldType,
    offset: u32,
    size: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
pub struct EnumDef {
    name: String,
    variants: Box<[EnumVariantDef]>,
}

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
pub struct EnumVariantDef {
    name: String,
    index: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldIndex(usize);

// A pure function of `size` + `fields`; see `RecordLayout::from_fields`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecordLayout {
    pub(crate) header_size: u32,
    pub(crate) has_dynamic_fields: bool,
    pub(crate) fixed_payload_size: Option<u32>,
    valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
#[wincode(tag_encoding = "u8")]
pub enum PrimitiveType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

#[derive(Clone, Debug, PartialEq, Eq, wincode::SchemaWrite, wincode::SchemaRead)]
#[wincode(tag_encoding = "u8")]
pub enum FieldType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    FixedArray { elem: PrimitiveType, len: u32 },
    String,
    Vec { elem: PrimitiveType },
    Enum(EnumDef),
}

impl Schema {
    #[inline]
    pub fn from_parts(schema_version: u32, root: RootDef, records: Vec<RecordDef>) -> Result<Self> {
        let schema = Self {
            schema_version,
            root,
            records,
        };
        schema.validate()?;
        Ok(schema)
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[inline]
    pub fn root(&self) -> &RootDef {
        &self.root
    }

    #[inline]
    pub fn records(&self) -> &[RecordDef] {
        &self.records
    }

    #[inline]
    pub fn record_for_variant(&self, tag: u16) -> Option<&RecordDef> {
        match &self.root {
            RootDef::Struct => (tag == 0).then(|| self.records.first()).flatten(),
            RootDef::TaggedUnion { .. } => self.records.get(tag as usize),
        }
    }

    pub fn validate(&self) -> Result<()> {
        for record in &self.records {
            // Errors if the field set is malformed (unsized field, offset/size
            // overflow, or a fixed record whose declared size disagrees with its fields).
            let header_size = record.header_size()?;
            record.validate_fields()?;
            if let Some(size) = record.size {
                if record.has_dynamic_fields() {
                    if size < header_size {
                        return Err(Error::Schema(format!(
                            "record `{}` max size {} is smaller than header size {}",
                            record.name, size, header_size
                        )));
                    }
                } else if size != header_size {
                    return Err(Error::Schema(format!(
                        "record `{}` fixed size {} does not match header size {}",
                        record.name, size, header_size
                    )));
                }
            }
        }

        match &self.root {
            RootDef::Struct => {
                if self.records.len() != 1 {
                    return Err(Error::Schema(format!(
                        "root struct schema must contain exactly one record, found {}",
                        self.records.len()
                    )));
                }
            }
            RootDef::TaggedUnion { .. } => {}
        }

        Ok(())
    }
}

impl RecordDef {
    #[inline]
    pub fn new(name: String, size: Option<u32>, fields: Vec<FieldDef>) -> Self {
        Self {
            name,
            size,
            fields: fields.into_boxed_slice(),
        }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn size(&self) -> Option<u32> {
        self.size
    }

    #[inline]
    pub fn fields(&self) -> &[FieldDef] {
        &self.fields
    }

    #[inline]
    pub(crate) fn layout(&self) -> Result<RecordLayout> {
        let layout = RecordLayout::from_fields(self.size, &self.fields);
        if layout.valid {
            Ok(layout)
        } else {
            Err(Error::Schema(format!(
                "record `{}` layout is invalid",
                self.name
            )))
        }
    }

    #[inline]
    pub fn header_size(&self) -> Result<u32> {
        Ok(self.layout()?.header_size)
    }

    #[inline]
    pub fn has_dynamic_fields(&self) -> bool {
        RecordLayout::from_fields(self.size, &self.fields).has_dynamic_fields
    }

    #[inline]
    pub fn field_index(&self, name: &str) -> Option<FieldIndex> {
        self.fields
            .iter()
            .position(|field| field.name == name)
            .map(FieldIndex)
    }

    #[inline]
    pub fn field(&self, index: FieldIndex) -> Option<&FieldDef> {
        self.fields.get(index.0)
    }

    fn validate_fields(&self) -> Result<()> {
        for field in &self.fields {
            if let FieldType::Enum(enum_def) = &field.ty {
                enum_def.validate().map_err(|error| {
                    Error::Schema(format!(
                        "field `{}` has invalid enum definition: {}",
                        field.name, error
                    ))
                })?;
            }
        }
        Ok(())
    }
}

impl RecordLayout {
    #[inline]
    fn from_fields(size: Option<u32>, fields: &[FieldDef]) -> Self {
        let mut header_size = 0u32;
        let mut has_dynamic_fields = false;
        for field in fields {
            let Some(size) = field.ty.checked_fixed_size() else {
                return Self {
                    header_size: 0,
                    has_dynamic_fields,
                    fixed_payload_size: None,
                    valid: false,
                };
            };
            let Some(end) = field.offset.checked_add(size) else {
                return Self {
                    header_size: 0,
                    has_dynamic_fields,
                    fixed_payload_size: None,
                    valid: false,
                };
            };
            header_size = header_size.max(end);
            has_dynamic_fields |= field.ty.is_dynamic();
        }

        let fixed_payload_size = if has_dynamic_fields {
            None
        } else {
            match size {
                Some(size) if size == header_size => Some(size),
                Some(_) => {
                    return Self {
                        header_size,
                        has_dynamic_fields,
                        fixed_payload_size: None,
                        valid: false,
                    };
                }
                None => None,
            }
        };

        if has_dynamic_fields
            && let Some(size) = size
            && size < header_size
        {
            return Self {
                header_size,
                has_dynamic_fields,
                fixed_payload_size: None,
                valid: false,
            };
        }

        Self {
            header_size,
            has_dynamic_fields,
            fixed_payload_size,
            valid: true,
        }
    }
}

impl FieldIndex {
    #[inline]
    pub fn get(self) -> usize {
        self.0
    }
}

impl FieldDef {
    #[inline]
    pub fn new(name: String, ty: FieldType, offset: u32, size: Option<u32>) -> Self {
        Self {
            name,
            ty,
            offset,
            size,
        }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn ty(&self) -> &FieldType {
        &self.ty
    }

    #[inline]
    pub fn offset(&self) -> u32 {
        self.offset
    }

    #[inline]
    pub fn size(&self) -> Option<u32> {
        self.size
    }
}

impl EnumDef {
    #[inline]
    pub fn new(name: String, variants: Vec<EnumVariantDef>) -> Self {
        Self {
            name,
            variants: variants.into_boxed_slice(),
        }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn variants(&self) -> &[EnumVariantDef] {
        &self.variants
    }

    #[inline]
    pub fn variant_for_index(&self, index: u8) -> Option<&EnumVariantDef> {
        self.variants.iter().find(|variant| variant.index == index)
    }

    fn validate(&self) -> std::result::Result<(), String> {
        for (position, variant) in self.variants.iter().enumerate() {
            if self.variants[..position]
                .iter()
                .any(|previous| previous.name == variant.name)
            {
                return Err(format!("duplicate variant name `{}`", variant.name));
            }
            if self.variants[..position]
                .iter()
                .any(|previous| previous.index == variant.index)
            {
                return Err(format!("duplicate variant index {}", variant.index));
            }
        }
        Ok(())
    }
}

impl EnumVariantDef {
    #[inline]
    pub fn new(name: String, index: u8) -> Self {
        Self { name, index }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn index(&self) -> u8 {
        self.index
    }
}

impl PrimitiveType {
    #[inline]
    pub fn size(self) -> u32 {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
        }
    }
}

impl FieldType {
    #[inline]
    pub fn checked_fixed_size(&self) -> Option<u32> {
        Some(match self {
            Self::U8 | Self::I8 | Self::Bool => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
            Self::FixedArray { elem, len } => elem.size().checked_mul(*len)?,
            Self::String | Self::Vec { .. } => 8,
            Self::Enum(_) => 1,
        })
    }

    #[inline]
    pub fn fixed_size(&self) -> u32 {
        self.checked_fixed_size().unwrap_or(u32::MAX)
    }

    #[inline]
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Self::String | Self::Vec { .. })
    }
}
