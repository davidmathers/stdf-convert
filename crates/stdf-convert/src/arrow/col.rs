//! One Arrow column type per STDF type code: the only hand-written part of the Arrow schema.
//! The generated columns name these by the codes that rust-stdf declares each field with, and
//! the compiler checks that each field's Rust type is the column's [`Column::Value`].

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, Float32Builder, Float64Builder, Int8Builder, Int16Builder, Int32Builder, ListBuilder,
    PrimitiveBuilder, StringBuilder, UInt8Builder, UInt16Builder, UInt32Builder, UInt64Builder,
};
use arrow_array::types::ArrowPrimitiveType;
use arrow_array::{ArrayRef, ListArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType, Field, Fields};
use rust_stdf::V1;

use super::generated::{DnColumns, V1Columns};

/// Field metadata key holding the STDF type code a column was declared with.
pub const STDF_TYPE: &str = "stdf_type";

/// A column being built from one field of every record of a type.
pub(crate) trait Column: Default {
    /// The field's Rust type in rust-stdf.
    type Value: ?Sized;
    /// The STDF type code, kept in the field's metadata.
    const STDF_TYPE: &'static str;
    const NULLABLE: bool = false;

    fn data_type() -> DataType;
    fn append(&mut self, value: &Self::Value);
    /// Take the column built so far, leaving it empty.
    fn finish(&mut self) -> ArrayRef;

    fn field(name: &str) -> Field {
        Field::new(name, Self::data_type(), Self::NULLABLE)
            .with_metadata(HashMap::from([(STDF_TYPE.to_string(), Self::STDF_TYPE.to_string())]))
    }
}

/// A column that can hold nulls, for optional fields.
pub(crate) trait Nullable: Column {
    fn append_null(&mut self);
}

/// An optional field: null where the record left the field out.
#[derive(Default)]
pub(crate) struct Opt<C>(C);

impl<C: Nullable> Opt<C> {
    pub(crate) fn append_option(&mut self, value: Option<&C::Value>) {
        match value {
            Some(v) => self.0.append(v),
            None => self.0.append_null(),
        }
    }
}

impl<C: Nullable> Column for Opt<C>
where
    C::Value: Sized,
{
    type Value = Option<C::Value>;
    const STDF_TYPE: &'static str = C::STDF_TYPE;
    const NULLABLE: bool = true;

    fn data_type() -> DataType {
        C::data_type()
    }
    fn append(&mut self, value: &Option<C::Value>) {
        self.append_option(value.as_ref());
    }
    fn finish(&mut self) -> ArrayRef {
        self.0.finish()
    }
}

macro_rules! primitive {
    ($($code:ident: $rust:ty => $builder:ty, $data_type:expr;)*) => {$(
        #[derive(Default)]
        pub(crate) struct $code($builder);

        impl Column for $code {
            type Value = $rust;
            const STDF_TYPE: &'static str = stringify!($code);
            fn data_type() -> DataType {
                $data_type
            }
            fn append(&mut self, value: &$rust) {
                self.0.append_value(*value);
            }
            fn finish(&mut self) -> ArrayRef {
                Arc::new(self.0.finish())
            }
        }

        impl Nullable for $code {
            fn append_null(&mut self) {
                self.0.append_null();
            }
        }
    )*};
}

primitive! {
    U1: u8 => UInt8Builder, DataType::UInt8;
    U2: u16 => UInt16Builder, DataType::UInt16;
    U4: u32 => UInt32Builder, DataType::UInt32;
    U8: u64 => UInt64Builder, DataType::UInt64;
    I1: i8 => Int8Builder, DataType::Int8;
    I2: i16 => Int16Builder, DataType::Int16;
    I4: i32 => Int32Builder, DataType::Int32;
    R4: f32 => Float32Builder, DataType::Float32;
    R8: f64 => Float64Builder, DataType::Float64;
}

macro_rules! text {
    ($($code:ident: $rust:ty => |$v:ident| $as_str:expr;)*) => {$(
        #[derive(Default)]
        pub(crate) struct $code(StringBuilder);

        impl Column for $code {
            type Value = $rust;
            const STDF_TYPE: &'static str = stringify!($code);
            fn data_type() -> DataType {
                DataType::Utf8
            }
            fn append(&mut self, $v: &$rust) {
                self.0.append_value($as_str);
            }
            fn finish(&mut self) -> ArrayRef {
                Arc::new(self.0.finish())
            }
        }

        impl Nullable for $code {
            fn append_null(&mut self) {
                self.0.append_null();
            }
        }
    )*};
}

// rust-stdf reads each text byte as one character (U+0000 to U+00FF), so the original bytes
// can be recovered by encoding the text as Latin-1.
text! {
    C1: char => |v| v.encode_utf8(&mut [0; 4]);
    Cn: String => |v| v;
    Sn: String => |v| v;
}

macro_rules! bytes {
    ($($code:ident: $rust:ty;)*) => {$(
        #[derive(Default)]
        pub(crate) struct $code(BinaryBuilder);

        impl Column for $code {
            type Value = $rust;
            const STDF_TYPE: &'static str = stringify!($code);
            fn data_type() -> DataType {
                DataType::Binary
            }
            fn append(&mut self, value: &$rust) {
                self.0.append_value(value);
            }
            fn finish(&mut self) -> ArrayRef {
                Arc::new(self.0.finish())
            }
        }

        impl Nullable for $code {
            fn append_null(&mut self) {
                self.0.append_null();
            }
        }
    )*};
}

bytes! {
    B1: [u8; 1];
    Bn: Vec<u8>;
}

/// `Dn` (bit fields): a struct of the bit count and the packed bits (see [`DnColumns`]).
#[derive(Default)]
pub(crate) struct Dn {
    values: DnColumns,
    valid: Vec<bool>,
}

impl Column for Dn {
    type Value = rust_stdf::Dn;
    const STDF_TYPE: &'static str = "Dn";
    fn data_type() -> DataType {
        DataType::Struct(Fields::from(DnColumns::fields()))
    }
    fn append(&mut self, value: &rust_stdf::Dn) {
        self.values.append(value);
        self.valid.push(true);
    }
    fn finish(&mut self) -> ArrayRef {
        let valid = std::mem::take(&mut self.valid);
        let nulls = valid.contains(&false).then(|| NullBuffer::from(valid));
        Arc::new(StructArray::new(Fields::from(DnColumns::fields()), self.values.finish(), nulls))
    }
}

impl Nullable for Dn {
    fn append_null(&mut self) {
        self.values.append(&rust_stdf::Dn::default());
        self.valid.push(false);
    }
}

/// The byte order of a reserved or unknown record's data: `LittleEndian` or `BigEndian`.
#[derive(Default)]
pub(crate) struct ByteOrder(StringBuilder);

impl Column for ByteOrder {
    type Value = rust_stdf::ByteOrder;
    const STDF_TYPE: &'static str = "";
    fn data_type() -> DataType {
        DataType::Utf8
    }
    fn append(&mut self, value: &rust_stdf::ByteOrder) {
        self.0.append_value(match value {
            rust_stdf::ByteOrder::LittleEndian => "LittleEndian",
            rust_stdf::ByteOrder::BigEndian => "BigEndian",
        });
    }
    fn finish(&mut self) -> ArrayRef {
        Arc::new(self.0.finish())
    }
    fn field(name: &str) -> Field {
        Field::new(name, DataType::Utf8, false)
    }
}

/// The list item field: list elements are never null.
fn item(data_type: DataType) -> Arc<Field> {
    Arc::new(Field::new_list_field(data_type, false))
}

/// A list column of primitive values.
pub(crate) struct PrimitiveList<T: ArrowPrimitiveType>(ListBuilder<PrimitiveBuilder<T>>);

impl<T: ArrowPrimitiveType> Default for PrimitiveList<T> {
    fn default() -> Self {
        Self(ListBuilder::new(PrimitiveBuilder::new()).with_field(item(T::DATA_TYPE)))
    }
}

impl<T: ArrowPrimitiveType> PrimitiveList<T> {
    fn append_values(&mut self, values: impl IntoIterator<Item = T::Native>) {
        self.0.values().extend(values.into_iter().map(Some));
        self.0.append(true);
    }
}

macro_rules! primitive_list {
    ($($code:ident: $rust:ty => $arrow:ty;)*) => {$(
        #[derive(Default)]
        pub(crate) struct $code(PrimitiveList<$arrow>);

        impl Column for $code {
            type Value = Vec<$rust>;
            const STDF_TYPE: &'static str = stringify!($code);
            fn data_type() -> DataType {
                DataType::List(item(<$arrow>::DATA_TYPE))
            }
            fn append(&mut self, value: &Vec<$rust>) {
                self.0.append_values(value.iter().copied());
            }
            fn finish(&mut self) -> ArrayRef {
                Arc::new(self.0.0.finish())
            }
        }

        impl Nullable for $code {
            fn append_null(&mut self) {
                self.0.0.append_null();
            }
        }
    )*};
}

primitive_list! {
    KxU1: u8 => arrow_array::types::UInt8Type;
    KxN1: u8 => arrow_array::types::UInt8Type;
    KxU2: u16 => arrow_array::types::UInt16Type;
    KxU4: u32 => arrow_array::types::UInt32Type;
    KxU8: u64 => arrow_array::types::UInt64Type;
    KxR4: f32 => arrow_array::types::Float32Type;
}

/// A list column of text.
pub(crate) struct TextList(ListBuilder<StringBuilder>);

impl Default for TextList {
    fn default() -> Self {
        Self(ListBuilder::new(StringBuilder::new()).with_field(item(DataType::Utf8)))
    }
}

macro_rules! text_list {
    ($($code:ident;)*) => {$(
        #[derive(Default)]
        pub(crate) struct $code(TextList);

        impl Column for $code {
            type Value = Vec<String>;
            const STDF_TYPE: &'static str = stringify!($code);
            fn data_type() -> DataType {
                DataType::List(item(DataType::Utf8))
            }
            fn append(&mut self, value: &Vec<String>) {
                for v in value {
                    self.0.0.values().append_value(v);
                }
                self.0.0.append(true);
            }
            fn finish(&mut self) -> ArrayRef {
                Arc::new(self.0.0.finish())
            }
        }

        impl Nullable for $code {
            fn append_null(&mut self) {
                self.0.0.append_null();
            }
        }
    )*};
}

text_list! {
    KxCn;
    KxSn;
    KxCf;
}

/// `KxUf` (STR arrays of 1, 2, 4 or 8 byte numbers): widened to `u64`. The record's
/// `*_SIZE` field says which width the file used.
#[derive(Default)]
pub(crate) struct KxUf(PrimitiveList<arrow_array::types::UInt64Type>);

impl Column for KxUf {
    type Value = rust_stdf::KxUf;
    const STDF_TYPE: &'static str = "KxUf";
    fn data_type() -> DataType {
        DataType::List(item(DataType::UInt64))
    }
    fn append(&mut self, value: &rust_stdf::KxUf) {
        use rust_stdf::KxUf::*;
        match value {
            F1(v) => self.0.append_values(v.iter().map(|&x| u64::from(x))),
            F2(v) => self.0.append_values(v.iter().map(|&x| u64::from(x))),
            F4(v) => self.0.append_values(v.iter().map(|&x| u64::from(x))),
            F8(v) => self.0.append_values(v.iter().copied()),
        }
    }
    fn finish(&mut self) -> ArrayRef {
        Arc::new(self.0.0.finish())
    }
}

/// `Vn` (`GDR.GEN_DATA`): a list of typed values, each a struct of its type name and one
/// column per type (see [`V1Columns`]).
pub(crate) struct Vn {
    offsets: Vec<i32>,
    values: V1Columns,
}

impl Default for Vn {
    fn default() -> Self {
        Vn { offsets: vec![0], values: V1Columns::default() }
    }
}

impl Vn {
    fn item() -> Arc<Field> {
        item(DataType::Struct(Fields::from(V1Columns::fields())))
    }
}

impl Column for Vn {
    type Value = Vec<V1>;
    const STDF_TYPE: &'static str = "Vn";
    fn data_type() -> DataType {
        DataType::List(Vn::item())
    }
    fn append(&mut self, value: &Vec<V1>) {
        for v in value {
            self.values.append(v);
        }
        let end = self.offsets.last().copied().unwrap_or(0) + value.len() as i32;
        self.offsets.push(end);
    }
    fn finish(&mut self) -> ArrayRef {
        let values = StructArray::new(Fields::from(V1Columns::fields()), self.values.finish(), None);
        let offsets = OffsetBuffer::new(std::mem::replace(&mut self.offsets, vec![0]).into());
        Arc::new(ListArray::new(Vn::item(), offsets, Arc::new(values), None))
    }
}

/// The type name of a GDR value (`U1`, `Cn`, `B0`, ...).
#[derive(Default)]
pub(crate) struct Name(StringBuilder);

impl Column for Name {
    type Value = str;
    const STDF_TYPE: &'static str = "";
    fn data_type() -> DataType {
        DataType::Utf8
    }
    fn append(&mut self, value: &str) {
        self.0.append_value(value);
    }
    fn finish(&mut self) -> ArrayRef {
        Arc::new(self.0.finish())
    }
    fn field(name: &str) -> Field {
        Field::new(name, DataType::Utf8, false)
    }
}
