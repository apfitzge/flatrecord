use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DataEnum, DataStruct, DeriveInput, Expr, ExprLit, Fields, GenericArgument,
    Lit, LitInt, PathArguments, Type, TypeArray, TypePath, parse_macro_input,
};

#[proc_macro_derive(FlatRecord, attributes(record, schema))]
pub fn derive_flat_record(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_flat_record(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(FlatEnum)]
pub fn derive_flat_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_flat_enum(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_flat_record(input: DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "FlatRecord does not support generics in v1",
        ));
    }

    let ident = input.ident;
    let attrs = input.attrs;

    match input.data {
        Data::Struct(data) => expand_struct(ident, attrs, data),
        Data::Enum(data) => expand_enum(ident, attrs, data),
        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            "FlatRecord does not support unions",
        )),
    }
}

fn expand_flat_enum(input: DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "FlatEnum does not support generics in v1",
        ));
    }

    let ident = input.ident;
    let data = match input.data {
        Data::Enum(data) => data,
        Data::Struct(data) => {
            return Err(syn::Error::new_spanned(
                data.struct_token,
                "FlatEnum can only be derived for enums",
            ));
        }
        Data::Union(data) => {
            return Err(syn::Error::new_spanned(
                data.union_token,
                "FlatEnum can only be derived for enums",
            ));
        }
    };

    if data.variants.len() > 256 {
        return Err(syn::Error::new_spanned(
            ident,
            "FlatEnum supports at most 256 variants",
        ));
    }

    let mut variant_defs = Vec::new();
    let mut to_index_arms = Vec::new();
    let mut try_from_index_arms = Vec::new();

    for (index, variant) in data.variants.into_iter().enumerate() {
        let variant_ident = variant.ident;
        match variant.fields {
            Fields::Unit => {}
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "FlatEnum variants must be fieldless unit variants",
                ));
            }
        }
        if let Some((_, expr)) = variant.discriminant {
            return Err(syn::Error::new_spanned(
                expr,
                "FlatEnum variants must not use explicit discriminants; declaration index is the wire value",
            ));
        }

        let variant_name = variant_ident.to_string();
        let index = index as u8;
        variant_defs.push(quote! {
            ::flatrecord::schema::EnumVariantDef::new(#variant_name.to_owned(), #index)
        });
        to_index_arms.push(quote! {
            Self::#variant_ident => #index,
        });
        try_from_index_arms.push(quote! {
            #index => Some(Self::#variant_ident),
        });
    }

    let enum_name = ident.to_string();

    Ok(quote! {
        impl ::flatrecord::FlatEnum for #ident {
            const ENUM_NAME: &'static str = #enum_name;

            fn enum_def() -> ::flatrecord::schema::EnumDef {
                ::flatrecord::schema::EnumDef::new(
                    Self::ENUM_NAME.to_owned(),
                    vec![#(#variant_defs),*],
                )
            }

            fn to_index(self) -> u8 {
                match self {
                    #(#to_index_arms)*
                }
            }

            fn try_from_index(index: u8) -> Option<Self> {
                match index {
                    #(#try_from_index_arms)*
                    _ => None,
                }
            }
        }
    })
}

fn expand_struct(
    ident: syn::Ident,
    attrs: Vec<Attribute>,
    data: DataStruct,
) -> syn::Result<TokenStream2> {
    let record_attr = record_attrs(&attrs)?;
    let schema_attr = schema_attrs(&attrs)?;
    let schema_version = schema_attr.version;
    let fields = match data.fields {
        Fields::Named(fields) => fields.named,
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "FlatRecord structs must have named fields",
            ));
        }
    };

    let mut offset = 0usize;
    let mut field_defs = Vec::new();
    let mut fixed_encoders = Vec::new();
    let mut dynamic_encoders = Vec::new();
    let mut dynamic_validators = Vec::new();
    let mut decoders = Vec::new();
    let mut field_names = Vec::new();
    let mut payload_len_terms = Vec::new();
    let mut has_dynamic_fields = false;

    for field in fields.iter() {
        let field_ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(field, "FlatRecord fields must be named"))?;
        let supported = SupportedType::parse(&field.ty)?;
        let size = supported.header_size();
        let field_ty = supported.field_type_tokens();
        let field_offset = offset;
        let field_size = size;
        let field_name = field_ident.to_string();

        field_defs.push(quote! {
            ::flatrecord::schema::FieldDef::new(
                #field_name.to_owned(),
                #field_ty,
                #field_offset as u32,
                Some(#field_size as u32),
            )
        });
        if supported.is_dynamic() {
            has_dynamic_fields = true;
            payload_len_terms.push(supported.payload_len_term(&field_ident));
            dynamic_encoders.push(supported.encode_dynamic_tokens(&field_ident, field_offset));
            dynamic_validators
                .push(supported.decode_dynamic_validator_tokens(&field_ident, field_offset));
        } else {
            fixed_encoders.push(supported.encode_fixed_tokens(&field_ident, field_offset));
        }
        decoders.push(supported.decode_tokens(&field_ident, field_offset));
        field_names.push(field_ident);
        offset += size;
    }

    let header_size = offset;
    let payload_size = if has_dynamic_fields {
        let max_size = record_attr.max_size.ok_or_else(|| {
            syn::Error::new_spanned(
                ident.clone(),
                "FlatRecord structs with String or Vec<T> fields require #[record(max_size = N)]",
            )
        })?;
        if max_size < header_size {
            return Err(syn::Error::new_spanned(
                ident.clone(),
                format!("record max_size {max_size} is smaller than header size {header_size}"),
            ));
        }
        max_size
    } else {
        header_size
    };
    let record_name = ident.to_string();
    let exact_decode_check = if has_dynamic_fields {
        quote! {
            if src.len() < #header_size {
                return Err(::flatrecord::Error::PayloadTooShort {
                    minimum: #header_size,
                    actual: src.len(),
                });
            }
            let __flatrecord_payload_limit = ::core::cmp::min(src.len(), #payload_size);
            #(#dynamic_validators)*
        }
    } else {
        quote! {
            if src.len() < #payload_size {
                return Err(::flatrecord::Error::UnexpectedLength {
                    expected: #payload_size,
                    actual: src.len(),
                });
            }
        }
    };
    let payload_len_body = if has_dynamic_fields {
        quote! {
            {
                let mut __flatrecord_len = #header_size;
                #(
                    __flatrecord_len = __flatrecord_len.saturating_add(#payload_len_terms);
                )*
                __flatrecord_len
            }
        }
    } else {
        quote!(#payload_size)
    };
    let max_size_check = if has_dynamic_fields {
        quote! {
            if payload_len > #payload_size {
                return Err(::flatrecord::Error::PayloadTooLarge {
                    max: #payload_size,
                    actual: payload_len,
                });
            }
        }
    } else {
        quote!()
    };
    let tail_cursor = if has_dynamic_fields {
        quote!(let mut __flatrecord_tail = #header_size;)
    } else {
        quote!()
    };
    let record_size = if has_dynamic_fields {
        quote!(Some(#payload_size as u32))
    } else {
        quote!(Some(#header_size as u32))
    };

    Ok(quote! {
        impl ::flatrecord::FlatRecord for #ident {
            const RECORD_NAME: &'static str = #record_name;
            const PAYLOAD_SIZE: usize = #payload_size;

            fn record_def() -> ::flatrecord::schema::RecordDef {
                ::flatrecord::schema::RecordDef::new(
                    Self::RECORD_NAME.to_owned(),
                    #record_size,
                    vec![#(#field_defs),*],
                )
            }

            fn payload_len(&self) -> usize {
                #payload_len_body
            }

            fn encode_payload(&self, dst: &mut [u8]) -> ::flatrecord::Result<usize> {
                let payload_len = self.payload_len();
                #max_size_check
                if dst.len() < payload_len {
                    return Err(::flatrecord::Error::BufferTooSmall {
                        required: payload_len,
                        actual: dst.len(),
                    });
                }
                let __flatrecord_dst = dst.as_mut_ptr();
                #(#fixed_encoders)*
                #tail_cursor
                #(#dynamic_encoders)*
                Ok(payload_len)
            }

            fn decode_payload(src: &[u8]) -> ::flatrecord::Result<Self> {
                #exact_decode_check
                #(#decoders)*
                Ok(Self {
                    #(#field_names),*
                })
            }
        }

        impl #ident {
            pub const SCHEMA_VERSION: u32 = #schema_version;

            pub fn schema() -> ::flatrecord::schema::Schema {
                ::flatrecord::schema::Schema::from_parts(
                    Self::SCHEMA_VERSION,
                    <Self as ::flatrecord::RecordRoot>::root_def(),
                    <Self as ::flatrecord::RecordRoot>::record_defs(),
                )
                .expect("generated flatrecord schema should be valid")
            }

        }
    })
}

fn expand_enum(
    ident: syn::Ident,
    attrs: Vec<Attribute>,
    data: DataEnum,
) -> syn::Result<TokenStream2> {
    let schema_attr = schema_attrs(&attrs)?;
    let schema_version = schema_attr.version;
    let mut encode_arms = Vec::new();
    let mut record_len_arms = Vec::new();
    let mut decode_arms = Vec::new();
    let mut records = Vec::new();

    for (variant_index, variant) in data.variants.into_iter().enumerate() {
        let variant_ident = variant.ident;
        if let Some(attr) = variant
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("record"))
        {
            return Err(syn::Error::new_spanned(
                attr,
                "FlatRecord enum variants do not support #[record(...)]; the wire tag is the variant index",
            ));
        }
        if variant_index > u16::MAX as usize {
            return Err(syn::Error::new_spanned(
                variant_ident,
                "FlatRecord enum roots support at most 65536 variants",
            ));
        }
        let variant_tag = variant_index as u16;

        let inner_ty = match variant.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let mut fields = fields.unnamed.into_iter();
                let Some(field) = fields.next() else {
                    return Err(syn::Error::new_spanned(
                        variant_ident,
                        "FlatRecord enum variants must be tuple variants with one record payload",
                    ));
                };
                field.ty
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "FlatRecord enum variants must be tuple variants with one record payload",
                ));
            }
        };
        // Safety model for the generated unsafe writes in this arm:
        // - `required` includes the tag and the exact payload length.
        // - `dst.len() < required` returns before any pointer writes.
        // - the tag write is a two-byte unaligned write at the start of `dst`.
        // - the payload slice is rebuilt from the already-checked tail range.
        encode_arms.push(quote! {
            Self::#variant_ident(value) => {
                let max_payload_len = <#inner_ty as ::flatrecord::FlatRecord>::PAYLOAD_SIZE;
                let payload_len = <#inner_ty as ::flatrecord::FlatRecord>::payload_len(value);
                if payload_len > max_payload_len {
                    return Err(::flatrecord::Error::PayloadTooLarge {
                        max: max_payload_len,
                        actual: payload_len,
                    });
                }
                let required = Self::TAG_SIZE + payload_len;
                if dst.len() < required {
                    return Err(::flatrecord::Error::BufferTooSmall {
                        required,
                        actual: dst.len(),
                    });
                }
                unsafe {
                    // SAFETY: required includes the two-byte tag and dst.len() >= required
                    // was checked above. write_unaligned handles the packed wire layout.
                    dst.as_mut_ptr()
                        .cast::<u16>()
                        .write_unaligned((#variant_tag as u16).to_le());
                }
                let written = <#inner_ty as ::flatrecord::FlatRecord>::encode_payload(
                    value,
                    unsafe {
                        // SAFETY: required was checked above. The generated payload encoder
                        // receives exactly the payload portion of the output buffer.
                        ::core::slice::from_raw_parts_mut(
                            dst.as_mut_ptr().add(Self::TAG_SIZE),
                            payload_len,
                        )
                    },
                )?;
                Ok(Self::TAG_SIZE + written)
            }
        });
        record_len_arms.push(quote! {
            Self::#variant_ident(value) => {
                Self::TAG_SIZE + <#inner_ty as ::flatrecord::FlatRecord>::payload_len(value)
            }
        });
        decode_arms.push(quote! {
            #variant_tag => Ok(Self::#variant_ident(<#inner_ty as ::flatrecord::FlatRecord>::decode_payload(payload)?)),
        });
        records.push(quote! {
            <#inner_ty as ::flatrecord::FlatRecord>::record_def()
        });
    }

    let root_name = ident.to_string();

    Ok(quote! {
        impl #ident {
            pub const TAG_SIZE: usize = 2;
            pub const SCHEMA_VERSION: u32 = #schema_version;

            pub fn schema() -> ::flatrecord::schema::Schema {
                ::flatrecord::schema::Schema::from_parts(
                    Self::SCHEMA_VERSION,
                    <Self as ::flatrecord::RecordRoot>::root_def(),
                    <Self as ::flatrecord::RecordRoot>::record_defs(),
                )
                .expect("generated flatrecord schema should be valid")
            }

            pub fn root_def() -> ::flatrecord::schema::RootDef {
                <Self as ::flatrecord::RecordRoot>::root_def()
            }

            pub fn record_defs() -> Vec<::flatrecord::schema::RecordDef> {
                <Self as ::flatrecord::RecordRoot>::record_defs()
            }

            pub fn record_len(&self) -> usize {
                <Self as ::flatrecord::RecordRoot>::record_len(self)
            }

            pub fn write_record(&self, dst: &mut [u8]) -> ::flatrecord::Result<usize> {
                <Self as ::flatrecord::RecordRoot>::encode_record(self, dst)
            }

            pub fn from_record_bytes(src: &[u8]) -> ::flatrecord::Result<Self> {
                <Self as ::flatrecord::RecordRoot>::decode_record(src)
            }
        }

        impl ::flatrecord::RecordRoot for #ident {
            fn root_def() -> ::flatrecord::schema::RootDef {
                ::flatrecord::schema::RootDef::TaggedUnion {
                    name: #root_name.to_owned(),
                }
            }

            fn record_defs() -> Vec<::flatrecord::schema::RecordDef> {
                vec![#(#records),*]
            }

            fn record_len(&self) -> usize {
                match self {
                    #(#record_len_arms)*
                }
            }

            fn encode_record(&self, dst: &mut [u8]) -> ::flatrecord::Result<usize> {
                match self {
                    #(#encode_arms)*
                }
            }

            fn decode_record(src: &[u8]) -> ::flatrecord::Result<Self> {
                if src.len() < 2 {
                    return Err(::flatrecord::Error::PayloadTooShort {
                        minimum: 2,
                        actual: src.len(),
                    });
                }
                let tag = u16::from_le(unsafe {
                    // SAFETY: src.len() >= 2 was checked above. read_unaligned handles
                    // the packed wire layout.
                    src.as_ptr().cast::<u16>().read_unaligned()
                });
                let payload = &src[2..];
                match tag {
                    #(#decode_arms)*
                    other => Err(::flatrecord::Error::UnknownRecordTag(other)),
                }
            }
        }
    })
}

struct RecordAttr {
    max_size: Option<usize>,
}

struct SchemaAttr {
    version: u32,
}

fn schema_attrs(attrs: &[Attribute]) -> syn::Result<SchemaAttr> {
    let mut version = None;
    for attr in attrs {
        if !attr.path().is_ident("schema") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("version") {
                let value = meta.value()?;
                let lit: LitInt = value.parse()?;
                let parsed = lit.base10_parse::<u32>()?;
                if version.replace(parsed).is_some() {
                    return Err(syn::Error::new_spanned(
                        lit,
                        "duplicate schema version attribute",
                    ));
                }
                Ok(())
            } else {
                Err(meta.error("unsupported schema attribute; expected version = N"))
            }
        })?;
    }
    Ok(SchemaAttr {
        version: version.unwrap_or(1),
    })
}

fn record_attrs(attrs: &[Attribute]) -> syn::Result<RecordAttr> {
    let mut max_size = None;
    for attr in attrs {
        if !attr.path().is_ident("record") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("max_size") {
                let value = meta.value()?;
                let lit: LitInt = value.parse()?;
                let parsed = lit.base10_parse::<usize>()?;
                if parsed > u32::MAX as usize {
                    return Err(syn::Error::new_spanned(
                        lit,
                        "record max_size must fit in u32 for v1 schema metadata",
                    ));
                }
                max_size = Some(parsed);
                Ok(())
            } else {
                Err(meta.error("unsupported record attribute; expected max_size = N"))
            }
        })?;
    }
    Ok(RecordAttr { max_size })
}

#[derive(Clone)]
enum SupportedType {
    Primitive(Primitive),
    Bool,
    FixedArray { elem: Primitive, len: usize },
    String,
    Vec { elem: Primitive },
    FlatEnum(Box<Type>),
}

#[derive(Clone, Copy)]
enum Primitive {
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

impl SupportedType {
    fn parse(ty: &Type) -> syn::Result<Self> {
        if let Some(primitive) = parse_primitive(ty) {
            return Ok(Self::Primitive(primitive));
        }
        if type_ident(ty).as_deref() == Some("bool") {
            return Ok(Self::Bool);
        }
        if type_ident(ty).as_deref() == Some("String") {
            return Ok(Self::String);
        }
        if let Some(elem) = parse_vec(ty)? {
            return Ok(Self::Vec { elem });
        }
        if let Type::Array(array) = ty {
            return parse_array(array);
        }
        if let Some(enum_ty) = parse_flat_enum(ty) {
            return Ok(Self::FlatEnum(Box::new(enum_ty)));
        }

        Err(syn::Error::new_spanned(
            ty,
            "unsupported FlatRecord field type in v1; supported types are fixed-width primitives, bool, [T; N], String, Vec<T> for primitive T, and FlatEnum field types",
        ))
    }

    fn header_size(&self) -> usize {
        match self {
            Self::Primitive(primitive) => primitive.size(),
            Self::Bool => 1,
            Self::FixedArray { elem, len } => elem.size() * *len,
            Self::String | Self::Vec { .. } => 8,
            Self::FlatEnum(_) => 1,
        }
    }

    fn is_dynamic(&self) -> bool {
        matches!(self, Self::String | Self::Vec { .. })
    }

    fn field_type_tokens(&self) -> TokenStream2 {
        match self {
            Self::Primitive(primitive) => primitive.field_type_tokens(),
            Self::Bool => quote!(::flatrecord::schema::FieldType::Bool),
            Self::FixedArray { elem, len } => {
                let elem = elem.primitive_type_tokens();
                quote!(::flatrecord::schema::FieldType::FixedArray {
                    elem: #elem,
                    len: #len as u32
                })
            }
            Self::String => quote!(::flatrecord::schema::FieldType::String),
            Self::Vec { elem } => {
                let elem = elem.primitive_type_tokens();
                quote!(::flatrecord::schema::FieldType::Vec { elem: #elem })
            }
            Self::FlatEnum(ty) => {
                let ty = ty.as_ref();
                quote!(::flatrecord::schema::FieldType::Enum(<#ty as ::flatrecord::FlatEnum>::enum_def()))
            }
        }
    }

    fn payload_len_term(&self, field: &syn::Ident) -> TokenStream2 {
        match self {
            Self::String => quote!(self.#field.as_bytes().len()),
            Self::Vec { elem } => {
                let elem_size = elem.size();
                quote!(self.#field.len().saturating_mul(#elem_size))
            }
            _ => quote!(0usize),
        }
    }

    fn encode_fixed_tokens(&self, field: &syn::Ident, offset: usize) -> TokenStream2 {
        match self {
            Self::Primitive(primitive) => {
                // Safety model for generated fixed primitive writes:
                // `encode_payload` validates that the destination is at least the full
                // payload size before any field writes run. Field offsets and sizes are
                // compile-time constants computed by the derive macro, so every generated
                // pointer write is inside that validated payload range. `write_unaligned`
                // is required because the wire format is tightly packed and does not
                // promise alignment.
                let write = primitive.encode_to_ptr_tokens(
                    quote!(__flatrecord_dst.add(#offset)),
                    quote!(self.#field),
                );
                quote! {
                    #write
                }
            }
            Self::Bool => quote! {
                unsafe {
                    // SAFETY: encode_payload checked that dst is large enough for the
                    // full payload before fixed-offset writes.
                    __flatrecord_dst
                        .add(#offset)
                        .write_unaligned(u8::from(self.#field));
                }
            },
            Self::FixedArray {
                elem: Primitive::U8,
                len,
            } => quote! {
                unsafe {
                    // SAFETY: encode_payload checked that dst is large enough for the
                    // full payload. The source field and output buffer do not overlap.
                    ::core::ptr::copy_nonoverlapping(
                        self.#field.as_ptr(),
                        __flatrecord_dst.add(#offset),
                        #len,
                    );
                }
            },
            Self::FixedArray { elem, .. } => {
                let elem_size = elem.size();
                // Safety model for generated fixed numeric-array writes:
                // the array field's byte range is fully contained in the compile-time
                // payload size checked by `encode_payload`; the pointer advances by the
                // primitive element size for exactly the statically-known array length.
                let write = elem.encode_to_ptr_tokens(quote!(__flatrecord_ptr), quote!(*item));
                quote! {
                    let mut __flatrecord_ptr = unsafe {
                        // SAFETY: encode_payload checked that dst is large enough for the
                        // full payload before fixed-offset writes.
                        __flatrecord_dst.add(#offset)
                    };
                    for item in &self.#field {
                        #write
                        __flatrecord_ptr = unsafe {
                            // SAFETY: the loop advances within the validated fixed array field.
                            __flatrecord_ptr.add(#elem_size)
                        };
                    }
                }
            }
            Self::FlatEnum(_) => quote! {
                unsafe {
                    // SAFETY: encode_payload checked that dst is large enough for the
                    // full payload before fixed-offset writes.
                    __flatrecord_dst
                        .add(#offset)
                        .write_unaligned(::flatrecord::FlatEnum::to_index(self.#field));
                }
            },
            Self::String | Self::Vec { .. } => quote!(),
        }
    }

    fn encode_dynamic_tokens(&self, field: &syn::Ident, offset: usize) -> TokenStream2 {
        match self {
            // Safety model for generated dynamic writes:
            // `payload_len()` computes the exact header + trailing-data length for this
            // instance, and `encode_payload` checks both max size and destination capacity
            // before any pointer writes. Header descriptor offsets are compile-time fixed;
            // trailing pointers are advanced only within the computed payload length.
            Self::String => quote! {
                let __flatrecord_bytes = self.#field.as_bytes();
                let __flatrecord_end = __flatrecord_tail + __flatrecord_bytes.len();
                unsafe {
                    // SAFETY: encode_payload checked payload_len and dst capacity before
                    // writing descriptors and trailing data.
                    __flatrecord_dst
                        .add(#offset)
                        .cast::<u32>()
                        .write_unaligned((__flatrecord_tail as u32).to_le());
                    __flatrecord_dst
                        .add(#offset + 4)
                        .cast::<u32>()
                        .write_unaligned((__flatrecord_bytes.len() as u32).to_le());
                    ::core::ptr::copy_nonoverlapping(
                        __flatrecord_bytes.as_ptr(),
                        __flatrecord_dst.add(__flatrecord_tail),
                        __flatrecord_bytes.len(),
                    );
                }
                __flatrecord_tail = __flatrecord_end;
            },
            Self::Vec {
                elem: Primitive::U8,
            } => quote! {
                let __flatrecord_items = self.#field.as_slice();
                let __flatrecord_end = __flatrecord_tail + __flatrecord_items.len();
                unsafe {
                    // SAFETY: encode_payload checked payload_len and dst capacity before
                    // writing descriptors and trailing data.
                    __flatrecord_dst
                        .add(#offset)
                        .cast::<u32>()
                        .write_unaligned((__flatrecord_tail as u32).to_le());
                    __flatrecord_dst
                        .add(#offset + 4)
                        .cast::<u32>()
                        .write_unaligned((__flatrecord_items.len() as u32).to_le());
                    ::core::ptr::copy_nonoverlapping(
                        __flatrecord_items.as_ptr(),
                        __flatrecord_dst.add(__flatrecord_tail),
                        __flatrecord_items.len(),
                    );
                }
                __flatrecord_tail = __flatrecord_end;
            },
            Self::Vec { elem } => {
                let elem_size = elem.size();
                let write = elem.encode_to_ptr_tokens(
                    quote!(__flatrecord_tail_ptr),
                    quote!(*__flatrecord_item),
                );
                quote! {
                    let __flatrecord_items = self.#field.as_slice();
                    unsafe {
                        // SAFETY: encode_payload checked payload_len and dst capacity before
                        // writing descriptors and trailing data.
                        __flatrecord_dst
                            .add(#offset)
                            .cast::<u32>()
                            .write_unaligned((__flatrecord_tail as u32).to_le());
                        __flatrecord_dst
                            .add(#offset + 4)
                            .cast::<u32>()
                            .write_unaligned((__flatrecord_items.len() as u32).to_le());
                    }
                    let mut __flatrecord_tail_ptr = unsafe {
                        // SAFETY: encode_payload checked payload_len and dst capacity.
                        __flatrecord_dst.add(__flatrecord_tail)
                    };
                    for __flatrecord_item in __flatrecord_items {
                        #write
                        __flatrecord_tail += #elem_size;
                        __flatrecord_tail_ptr = unsafe {
                            // SAFETY: the loop advances within the validated trailing data.
                            __flatrecord_tail_ptr.add(#elem_size)
                        };
                    }
                }
            }
            _ => quote!(),
        }
    }

    fn decode_dynamic_validator_tokens(&self, field: &syn::Ident, offset: usize) -> TokenStream2 {
        match self {
            Self::String => dynamic_validator_tokens(field, offset, 1),
            Self::Vec { elem } => dynamic_validator_tokens(field, offset, elem.size()),
            _ => quote!(),
        }
    }

    fn decode_tokens(&self, field: &syn::Ident, offset: usize) -> TokenStream2 {
        match self {
            Self::Primitive(primitive) => {
                // Safety model for generated fixed primitive reads:
                // `decode_payload` validates the minimum fixed payload length before any
                // field reads run. Field offsets and sizes are compile-time constants
                // computed by the derive macro, so every generated pointer read is inside
                // that validated payload range. `read_unaligned` is required because the
                // wire format is tightly packed and does not promise alignment.
                let value = primitive.decode_from_ptr_tokens(quote!(__flatrecord_ptr));
                quote! {
                    let #field = {
                        let __flatrecord_ptr = unsafe {
                            // SAFETY: decode_payload validated the full fixed field range
                            // before decoding fixed-offset fields.
                            src.as_ptr().add(#offset)
                        };
                        #value
                    };
                }
            }
            Self::Bool => {
                let name = field.to_string();
                quote! {
                    let __flatrecord_value = unsafe {
                        // SAFETY: decode_payload validated the full fixed field range
                        // before decoding fixed-offset fields.
                        src.as_ptr().add(#offset).read_unaligned()
                    };
                    let #field = match __flatrecord_value {
                        0 => false,
                        1 => true,
                        value => {
                            return Err(::flatrecord::Error::InvalidBool {
                                field: #name.to_owned(),
                                value,
                            });
                        }
                    };
                }
            }
            Self::FixedArray {
                elem: Primitive::U8,
                len,
            } => quote! {
                let mut #field = [0u8; #len];
                unsafe {
                    // SAFETY: decode_payload validated the full fixed field range
                    // before decoding fixed-offset fields. The destination is a local
                    // array of exactly len bytes and does not overlap src.
                    ::core::ptr::copy_nonoverlapping(
                        src.as_ptr().add(#offset),
                        #field.as_mut_ptr(),
                        #len,
                    );
                }
            },
            Self::FixedArray { elem, len } => {
                let elem_ty = elem.type_ident();
                let elem_size = elem.size();
                // Safety model for generated fixed numeric-array reads:
                // the full array byte range is included in the previously validated
                // fixed field range; the pointer advances by one primitive element for
                // exactly the statically-known array length.
                let value = elem.decode_from_ptr_tokens(quote!(__flatrecord_ptr));
                quote! {
                    let mut #field = [<#elem_ty as ::core::default::Default>::default(); #len];
                    let mut __flatrecord_ptr = unsafe {
                        // SAFETY: decode_payload validated the full fixed field range
                        // before decoding fixed-offset fields.
                        src.as_ptr().add(#offset)
                    };
                    for __flatrecord_item in &mut #field {
                        *__flatrecord_item = #value;
                        __flatrecord_ptr = unsafe {
                            // SAFETY: the loop advances within the validated fixed array field.
                            __flatrecord_ptr.add(#elem_size)
                        };
                    }
                }
            }
            Self::FlatEnum(ty) => {
                let ty = ty.as_ref();
                let name = field.to_string();
                quote! {
                    let __flatrecord_value = unsafe {
                        // SAFETY: decode_payload validated the full fixed field range
                        // before decoding fixed-offset fields.
                        src.as_ptr().add(#offset).read_unaligned()
                    };
                    let #field = <#ty as ::flatrecord::FlatEnum>::try_from_index(__flatrecord_value)
                        .ok_or_else(|| ::flatrecord::Error::InvalidEnum {
                            field: #name.to_owned(),
                            enum_name: <#ty as ::flatrecord::FlatEnum>::ENUM_NAME.to_owned(),
                            value: __flatrecord_value,
                        })?;
                }
            }
            Self::String => {
                let name = field.to_string();
                quote! {
                    let #field = {
                        let __flatrecord_descriptor = src.get(#offset..#offset + 8)
                            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                                field: #name.to_owned(),
                                offset: #offset as u32,
                                size: 8,
                                payload_len: src.len(),
                            })?;
                        let __flatrecord_descriptor_ptr = __flatrecord_descriptor.as_ptr();
                        let __flatrecord_offset = unsafe {
                            // SAFETY: the descriptor slice was checked to contain exactly
                            // the 8 descriptor bytes before this read.
                            u32::from_le(__flatrecord_descriptor_ptr.cast::<u32>().read_unaligned())
                        } as usize;
                        let __flatrecord_len = unsafe {
                            // SAFETY: the descriptor slice was checked to contain exactly
                            // the 8 descriptor bytes before this read.
                            u32::from_le(__flatrecord_descriptor_ptr.add(4).cast::<u32>().read_unaligned())
                        } as usize;
                        let __flatrecord_end = __flatrecord_offset
                            .checked_add(__flatrecord_len)
                            .ok_or_else(|| ::flatrecord::Error::Schema(
                                format!("field `{}` dynamic offset overflows", #name)
                            ))?;
                        let __flatrecord_bytes = src
                            .get(__flatrecord_offset..__flatrecord_end)
                            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                                field: #name.to_owned(),
                                offset: __flatrecord_offset as u32,
                                size: __flatrecord_len as u32,
                                payload_len: src.len(),
                            })?;
                        let __flatrecord_value = ::core::str::from_utf8(__flatrecord_bytes)
                            .map_err(|_| ::flatrecord::Error::InvalidUtf8 {
                                field: #name.to_owned(),
                            })?;
                        __flatrecord_value.to_owned()
                    };
                }
            }
            Self::Vec {
                elem: Primitive::U8,
            } => {
                let name = field.to_string();
                quote! {
                    let #field = {
                        let __flatrecord_descriptor = src.get(#offset..#offset + 8)
                            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                                field: #name.to_owned(),
                                offset: #offset as u32,
                                size: 8,
                                payload_len: src.len(),
                            })?;
                        let __flatrecord_descriptor_ptr = __flatrecord_descriptor.as_ptr();
                        let __flatrecord_offset = unsafe {
                            // SAFETY: the descriptor slice was checked to contain exactly
                            // the 8 descriptor bytes before this read.
                            u32::from_le(__flatrecord_descriptor_ptr.cast::<u32>().read_unaligned())
                        } as usize;
                        let __flatrecord_len = unsafe {
                            // SAFETY: the descriptor slice was checked to contain exactly
                            // the 8 descriptor bytes before this read.
                            u32::from_le(__flatrecord_descriptor_ptr.add(4).cast::<u32>().read_unaligned())
                        } as usize;
                        let __flatrecord_end = __flatrecord_offset
                            .checked_add(__flatrecord_len)
                            .ok_or_else(|| ::flatrecord::Error::Schema(
                                format!("field `{}` dynamic offset overflows", #name)
                            ))?;
                        let __flatrecord_bytes = src
                            .get(__flatrecord_offset..__flatrecord_end)
                            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                                field: #name.to_owned(),
                                offset: __flatrecord_offset as u32,
                                size: __flatrecord_len as u32,
                                payload_len: src.len(),
                            })?;
                        __flatrecord_bytes.to_vec()
                    };
                }
            }
            Self::Vec { elem } => {
                let elem_size = elem.size();
                let name = field.to_string();
                let value = elem.decode_from_ptr_tokens(quote!(__flatrecord_ptr));
                quote! {
                    let #field = {
                        let __flatrecord_descriptor = src.get(#offset..#offset + 8)
                            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                                field: #name.to_owned(),
                                offset: #offset as u32,
                                size: 8,
                                payload_len: src.len(),
                            })?;
                        let __flatrecord_descriptor_ptr = __flatrecord_descriptor.as_ptr();
                        let __flatrecord_offset = unsafe {
                            // SAFETY: the descriptor slice was checked to contain exactly
                            // the 8 descriptor bytes before this read.
                            u32::from_le(__flatrecord_descriptor_ptr.cast::<u32>().read_unaligned())
                        } as usize;
                        let __flatrecord_len = unsafe {
                            // SAFETY: the descriptor slice was checked to contain exactly
                            // the 8 descriptor bytes before this read.
                            u32::from_le(__flatrecord_descriptor_ptr.add(4).cast::<u32>().read_unaligned())
                        } as usize;
                        let __flatrecord_byte_len = __flatrecord_len
                            .checked_mul(#elem_size)
                            .ok_or_else(|| ::flatrecord::Error::Schema(
                                format!("field `{}` dynamic length overflows", #name)
                            ))?;
                        let __flatrecord_end = __flatrecord_offset
                            .checked_add(__flatrecord_byte_len)
                            .ok_or_else(|| ::flatrecord::Error::Schema(
                                format!("field `{}` dynamic offset overflows", #name)
                            ))?;
                        let __flatrecord_bytes = src
                            .get(__flatrecord_offset..__flatrecord_end)
                            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                                field: #name.to_owned(),
                                offset: __flatrecord_offset as u32,
                                size: __flatrecord_byte_len as u32,
                                payload_len: src.len(),
                            })?;
                        let mut __flatrecord_vec = Vec::with_capacity(__flatrecord_len);
                        let mut __flatrecord_ptr = __flatrecord_bytes.as_ptr();
                        for _ in 0..__flatrecord_len {
                            __flatrecord_vec.push(#value);
                            __flatrecord_ptr = unsafe {
                                // SAFETY: __flatrecord_bytes covers len * elem_size bytes,
                                // and this loop advances by one element each iteration.
                                __flatrecord_ptr.add(#elem_size)
                            };
                        }
                        __flatrecord_vec
                    };
                }
            }
        }
    }
}

fn dynamic_validator_tokens(field: &syn::Ident, offset: usize, elem_size: usize) -> TokenStream2 {
    let name = field.to_string();
    quote! {
        let __flatrecord_descriptor = src.get(#offset..#offset + 8)
            .ok_or_else(|| ::flatrecord::Error::FieldOutOfBounds {
                field: #name.to_owned(),
                offset: #offset as u32,
                size: 8,
                payload_len: src.len(),
            })?;
        let __flatrecord_descriptor_ptr = __flatrecord_descriptor.as_ptr();
        let __flatrecord_offset = unsafe {
            // SAFETY: the descriptor slice was checked to contain exactly
            // the 8 descriptor bytes before this read.
            u32::from_le(__flatrecord_descriptor_ptr.cast::<u32>().read_unaligned())
        } as usize;
        let __flatrecord_len = unsafe {
            // SAFETY: the descriptor slice was checked to contain exactly
            // the 8 descriptor bytes before this read.
            u32::from_le(__flatrecord_descriptor_ptr.add(4).cast::<u32>().read_unaligned())
        } as usize;
        let __flatrecord_byte_len = __flatrecord_len
            .checked_mul(#elem_size)
            .ok_or_else(|| ::flatrecord::Error::Schema(
                format!("field `{}` dynamic length overflows", #name)
            ))?;
        let __flatrecord_end = __flatrecord_offset
            .checked_add(__flatrecord_byte_len)
            .ok_or_else(|| ::flatrecord::Error::Schema(
                format!("field `{}` dynamic offset overflows", #name)
            ))?;
        if __flatrecord_end > __flatrecord_payload_limit {
            return Err(::flatrecord::Error::FieldOutOfBounds {
                field: #name.to_owned(),
                offset: __flatrecord_offset as u32,
                size: u32::try_from(__flatrecord_byte_len).unwrap_or(u32::MAX),
                payload_len: __flatrecord_payload_limit,
            });
        }
    }
}

impl Primitive {
    fn encode_to_ptr_tokens(self, ptr: TokenStream2, value: TokenStream2) -> TokenStream2 {
        match self {
            Self::U8 => quote! {
                unsafe {
                    // SAFETY: callers only pass pointers into output ranges already
                    // validated by the generated encoder.
                    #ptr.write_unaligned(#value);
                }
            },
            Self::I8 => quote! {
                unsafe {
                    // SAFETY: callers only pass pointers into output ranges already
                    // validated by the generated encoder.
                    #ptr.write_unaligned(#value as u8);
                }
            },
            Self::U16 | Self::U32 | Self::U64 | Self::I16 | Self::I32 | Self::I64 => {
                let ty = self.type_ident();
                quote! {
                    unsafe {
                        // SAFETY: callers only pass pointers into output ranges already
                        // validated by the generated encoder. write_unaligned handles the
                        // packed wire layout.
                        #ptr.cast::<#ty>().write_unaligned((#value).to_le());
                    }
                }
            }
            Self::F32 => quote! {
                unsafe {
                    // SAFETY: callers only pass pointers into output ranges already
                    // validated by the generated encoder. write_unaligned handles the
                    // packed wire layout.
                    #ptr.cast::<u32>().write_unaligned((#value).to_bits().to_le());
                }
            },
            Self::F64 => quote! {
                unsafe {
                    // SAFETY: callers only pass pointers into output ranges already
                    // validated by the generated encoder. write_unaligned handles the
                    // packed wire layout.
                    #ptr.cast::<u64>().write_unaligned((#value).to_bits().to_le());
                }
            },
        }
    }

    fn decode_from_ptr_tokens(self, ptr: TokenStream2) -> TokenStream2 {
        match self {
            Self::U8 => quote! {
                unsafe {
                    // SAFETY: callers only pass pointers into payload ranges already
                    // validated by the generated decoder.
                    #ptr.read_unaligned()
                }
            },
            Self::I8 => quote! {
                unsafe {
                    // SAFETY: callers only pass pointers into payload ranges already
                    // validated by the generated decoder.
                    #ptr.read_unaligned() as i8
                }
            },
            Self::U16 | Self::U32 | Self::U64 | Self::I16 | Self::I32 | Self::I64 => {
                let ty = self.type_ident();
                quote! {
                    #ty::from_le(unsafe {
                        // SAFETY: callers only pass pointers into payload ranges already
                        // validated by the generated decoder. read_unaligned handles the
                        // packed wire layout.
                        #ptr.cast::<#ty>().read_unaligned()
                    })
                }
            }
            Self::F32 => quote! {
                f32::from_bits(u32::from_le(unsafe {
                    // SAFETY: callers only pass pointers into payload ranges already
                    // validated by the generated decoder. read_unaligned handles the
                    // packed wire layout.
                    #ptr.cast::<u32>().read_unaligned()
                }))
            },
            Self::F64 => quote! {
                f64::from_bits(u64::from_le(unsafe {
                    // SAFETY: callers only pass pointers into payload ranges already
                    // validated by the generated decoder. read_unaligned handles the
                    // packed wire layout.
                    #ptr.cast::<u64>().read_unaligned()
                }))
            },
        }
    }

    fn size(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
        }
    }

    fn type_ident(self) -> syn::Ident {
        match self {
            Self::U8 => format_ident!("u8"),
            Self::U16 => format_ident!("u16"),
            Self::U32 => format_ident!("u32"),
            Self::U64 => format_ident!("u64"),
            Self::I8 => format_ident!("i8"),
            Self::I16 => format_ident!("i16"),
            Self::I32 => format_ident!("i32"),
            Self::I64 => format_ident!("i64"),
            Self::F32 => format_ident!("f32"),
            Self::F64 => format_ident!("f64"),
        }
    }

    fn field_type_tokens(self) -> TokenStream2 {
        match self {
            Self::U8 => quote!(::flatrecord::schema::FieldType::U8),
            Self::U16 => quote!(::flatrecord::schema::FieldType::U16),
            Self::U32 => quote!(::flatrecord::schema::FieldType::U32),
            Self::U64 => quote!(::flatrecord::schema::FieldType::U64),
            Self::I8 => quote!(::flatrecord::schema::FieldType::I8),
            Self::I16 => quote!(::flatrecord::schema::FieldType::I16),
            Self::I32 => quote!(::flatrecord::schema::FieldType::I32),
            Self::I64 => quote!(::flatrecord::schema::FieldType::I64),
            Self::F32 => quote!(::flatrecord::schema::FieldType::F32),
            Self::F64 => quote!(::flatrecord::schema::FieldType::F64),
        }
    }

    fn primitive_type_tokens(self) -> TokenStream2 {
        match self {
            Self::U8 => quote!(::flatrecord::schema::PrimitiveType::U8),
            Self::U16 => quote!(::flatrecord::schema::PrimitiveType::U16),
            Self::U32 => quote!(::flatrecord::schema::PrimitiveType::U32),
            Self::U64 => quote!(::flatrecord::schema::PrimitiveType::U64),
            Self::I8 => quote!(::flatrecord::schema::PrimitiveType::I8),
            Self::I16 => quote!(::flatrecord::schema::PrimitiveType::I16),
            Self::I32 => quote!(::flatrecord::schema::PrimitiveType::I32),
            Self::I64 => quote!(::flatrecord::schema::PrimitiveType::I64),
            Self::F32 => quote!(::flatrecord::schema::PrimitiveType::F32),
            Self::F64 => quote!(::flatrecord::schema::PrimitiveType::F64),
        }
    }
}

fn parse_array(array: &TypeArray) -> syn::Result<SupportedType> {
    let len = array_len(&array.len)?;
    let elem = parse_primitive(&array.elem).ok_or_else(|| {
        syn::Error::new_spanned(
            &array.elem,
            "FlatRecord arrays must contain numeric primitive elements in v1",
        )
    })?;

    Ok(SupportedType::FixedArray { elem, len })
}

fn parse_vec(ty: &Type) -> syn::Result<Option<Primitive>> {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return Ok(None);
    };
    if path.segments.len() != 1 || path.segments[0].ident != "Vec" {
        return Ok(None);
    }

    let PathArguments::AngleBracketed(args) = &path.segments[0].arguments else {
        return Err(syn::Error::new_spanned(
            ty,
            "Vec fields must specify a primitive element type",
        ));
    };
    if args.args.len() != 1 {
        return Err(syn::Error::new_spanned(
            ty,
            "Vec fields must specify exactly one primitive element type",
        ));
    }

    let Some(GenericArgument::Type(elem_ty)) = args.args.first() else {
        return Err(syn::Error::new_spanned(
            ty,
            "Vec fields must use a primitive type argument",
        ));
    };
    let elem = parse_primitive(elem_ty).ok_or_else(|| {
        syn::Error::new_spanned(
            elem_ty,
            "FlatRecord Vec fields must contain primitive numeric elements in v1",
        )
    })?;
    Ok(Some(elem))
}

fn parse_flat_enum(ty: &Type) -> Option<Type> {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return None;
    };
    if path.segments.is_empty() {
        return None;
    }
    if !path
        .segments
        .iter()
        .all(|segment| matches!(segment.arguments, PathArguments::None))
    {
        return None;
    }
    Some(ty.clone())
}

fn array_len(expr: &Expr) -> syn::Result<usize> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(lit), ..
        }) => lit.base10_parse::<usize>(),
        other => Err(syn::Error::new_spanned(
            other,
            "FlatRecord array lengths must be integer literals in v1",
        )),
    }
}

fn parse_primitive(ty: &Type) -> Option<Primitive> {
    match type_ident(ty)?.as_str() {
        "u8" => Some(Primitive::U8),
        "u16" => Some(Primitive::U16),
        "u32" => Some(Primitive::U32),
        "u64" => Some(Primitive::U64),
        "i8" => Some(Primitive::I8),
        "i16" => Some(Primitive::I16),
        "i32" => Some(Primitive::I32),
        "i64" => Some(Primitive::I64),
        "f32" => Some(Primitive::F32),
        "f64" => Some(Primitive::F64),
        _ => None,
    }
}

fn type_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => {
            if path.segments.len() == 1 {
                Some(path.segments[0].ident.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}
