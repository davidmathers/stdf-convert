//! JSON Lines output: one JSON object per record.
//!
//! ```json
//! {"sequence_number":2,"byte_offset":14,"rec_typ":15,"rec_sub":10,"record_type":"PTR","data":{"TEST_NUM":1,"HEAD_NUM":1,"SITE_NUM":2,"TEST_FLG":"81","PARM_FLG":"42","RESULT":0.018,"TEST_TXT":"","ALARM_ID":"","OPT_FLAG":null,...}}
//! ```
//!
//! Fields are in STDF specification order. Byte and bit fields (flags, `PART_FIX`, pin
//! maps, `RAW_DATA`, ...) are lowercase hex strings. Floats are written with the fewest
//! digits that read back as the same value; NaN and infinities, which JSON cannot represent
//! as numbers, are the strings `"NaN"`, `"Infinity"` and `"-Infinity"`. Absent optional
//! fields are `null`.

use std::fmt::Display;
use std::io::Write;

use rust_stdf::StdfRecord;
use serde::Serialize;
use serde::ser::{self, SerializeMap, SerializeSeq, SerializeStruct};

use crate::{BYTE_FIELDS, Record, Result};

/// A record's fields, serializable with any serde serializer.
pub struct RecordData<'a>(pub &'a StdfRecord);

#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
struct InvalidRecord {
    len: u16,
    typ: u8,
    sub: u8,
}

impl Serialize for RecordData<'_> {
    fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            StdfRecord::FAR(d) => d.serialize(s),
            StdfRecord::ATR(d) => d.serialize(s),
            StdfRecord::VUR(d) => d.serialize(s),
            StdfRecord::MIR(d) => d.serialize(s),
            StdfRecord::MRR(d) => d.serialize(s),
            StdfRecord::PCR(d) => d.serialize(s),
            StdfRecord::HBR(d) => d.serialize(s),
            StdfRecord::SBR(d) => d.serialize(s),
            StdfRecord::PMR(d) => d.serialize(s),
            StdfRecord::PGR(d) => d.serialize(s),
            StdfRecord::PLR(d) => d.serialize(s),
            StdfRecord::RDR(d) => d.serialize(s),
            StdfRecord::SDR(d) => d.serialize(s),
            StdfRecord::PSR(d) => d.serialize(s),
            StdfRecord::NMR(d) => d.serialize(s),
            StdfRecord::CNR(d) => d.serialize(s),
            StdfRecord::SSR(d) => d.serialize(s),
            StdfRecord::CDR(d) => d.serialize(s),
            StdfRecord::WIR(d) => d.serialize(s),
            StdfRecord::WRR(d) => d.serialize(s),
            StdfRecord::WCR(d) => d.serialize(s),
            StdfRecord::PIR(d) => d.serialize(s),
            StdfRecord::PRR(d) => d.serialize(s),
            StdfRecord::TSR(d) => d.serialize(s),
            StdfRecord::PTR(d) => d.serialize(s),
            StdfRecord::MPR(d) => d.serialize(s),
            StdfRecord::FTR(d) => d.serialize(s),
            StdfRecord::STR(d) => d.serialize(s),
            StdfRecord::BPS(d) => d.serialize(s),
            StdfRecord::EPS(d) => d.serialize(s),
            StdfRecord::GDR(d) => d.serialize(s),
            StdfRecord::DTR(d) => d.serialize(s),
            StdfRecord::ReservedRec(d) => d.serialize(s),
            StdfRecord::InvalidRec(d) => InvalidRecord { len: d.len, typ: d.typ, sub: d.sub }.serialize(s),
        }
    }
}

/// Append `record` as one JSON object (without a newline) to `out`.
pub fn write_record(out: &mut Vec<u8>, record: &Record) {
    let _ = write!(
        out,
        r#"{{"sequence_number":{},"byte_offset":{},"rec_typ":{},"rec_sub":{},"record_type":"{}","data":"#,
        record.sequence_number, record.byte_offset, record.rec_typ, record.rec_sub, record.record_type
    );
    RecordData(&record.data)
        .serialize(&mut Json { out })
        .expect("STDF records contain only JSON-representable values");
    out.push(b'}');
}

/// Write `records` as JSON Lines and return how many were written.
pub fn write_json_lines<W: Write>(records: impl IntoIterator<Item = Result<Record>>, out: W) -> Result<u64> {
    let mut out = std::io::BufWriter::with_capacity(1 << 16, out);
    let mut line = Vec::with_capacity(1024);
    let mut n = 0;
    for record in records {
        line.clear();
        write_record(&mut line, &record?);
        line.push(b'\n');
        out.write_all(&line)?;
        n += 1;
    }
    out.flush()?;
    Ok(n)
}

#[derive(Debug)]
struct Error(String);

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Error(msg.to_string())
    }
}

/// A compact JSON serializer with the conventions described in the module docs.
struct Json<'a> {
    out: &'a mut Vec<u8>,
}

impl Json<'_> {
    fn string(&mut self, s: &str) {
        self.out.push(b'"');
        for c in s.chars() {
            match c {
                '"' => self.out.extend_from_slice(b"\\\""),
                '\\' => self.out.extend_from_slice(b"\\\\"),
                '\n' => self.out.extend_from_slice(b"\\n"),
                '\r' => self.out.extend_from_slice(b"\\r"),
                '\t' => self.out.extend_from_slice(b"\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(self.out, "\\u{:04x}", c as u32);
                }
                c => {
                    let mut buf = [0; 4];
                    self.out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            }
        }
        self.out.push(b'"');
    }

    fn hex(&mut self, bytes: &[u8]) {
        self.out.push(b'"');
        for b in bytes {
            let _ = write!(self.out, "{b:02x}");
        }
        self.out.push(b'"');
    }

    fn non_finite(&mut self, nan: bool, positive: bool) {
        let s: &[u8] = match (nan, positive) {
            (true, _) => b"\"NaN\"",
            (false, true) => b"\"Infinity\"",
            (false, false) => b"\"-Infinity\"",
        };
        self.out.extend_from_slice(s);
    }

    /// Write a byte field as hex, or as ordinary JSON if it turns out not to be bytes.
    fn byte_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let mut bytes = Bytes::default();
        match value.serialize(&mut bytes) {
            Ok(()) if bytes.none => self.out.extend_from_slice(b"null"),
            Ok(()) => self.hex(&bytes.data),
            Err(_) => value.serialize(&mut *self)?,
        }
        Ok(())
    }
}

/// Serializer state for a JSON array or object being written.
struct Compound<'a, 'b> {
    json: &'b mut Json<'a>,
    first: bool,
    /// Closing text: `]`, `}`, or `]}` / `}}` for enum variants.
    close: &'static [u8],
}

impl Compound<'_, '_> {
    fn comma(&mut self) {
        if !self.first {
            self.json.out.push(b',');
        }
        self.first = false;
    }
}

impl<'a, 'b> ser::Serializer for &'b mut Json<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Compound<'a, 'b>;
    type SerializeTuple = Compound<'a, 'b>;
    type SerializeTupleStruct = Compound<'a, 'b>;
    type SerializeTupleVariant = Compound<'a, 'b>;
    type SerializeMap = Compound<'a, 'b>;
    type SerializeStruct = Compound<'a, 'b>;
    type SerializeStructVariant = Compound<'a, 'b>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        self.out.extend_from_slice(if v { b"true" } else { b"false" });
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        self.serialize_i64(v.into())
    }
    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        self.serialize_i64(v.into())
    }
    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        self.serialize_i64(v.into())
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        let _ = write!(self.out, "{v}");
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.serialize_u64(v.into())
    }
    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        self.serialize_u64(v.into())
    }
    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        self.serialize_u64(v.into())
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        let _ = write!(self.out, "{v}");
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Result<(), Error> {
        if v.is_finite() {
            self.out.extend_from_slice(ryu::Buffer::new().format_finite(v).as_bytes());
        } else {
            self.non_finite(v.is_nan(), v > 0.0);
        }
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        if v.is_finite() {
            self.out.extend_from_slice(ryu::Buffer::new().format_finite(v).as_bytes());
        } else {
            self.non_finite(v.is_nan(), v > 0.0);
        }
        Ok(())
    }
    fn serialize_char(self, v: char) -> Result<(), Error> {
        self.string(v.encode_utf8(&mut [0; 4]));
        Ok(())
    }
    fn serialize_str(self, v: &str) -> Result<(), Error> {
        self.string(v);
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        self.hex(v);
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Error> {
        self.out.extend_from_slice(b"null");
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        self.serialize_none()
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        self.serialize_none()
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<(), Error> {
        self.string(variant);
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.out.push(b'{');
        self.string(variant);
        self.out.push(b':');
        if BYTE_FIELDS.contains(&variant) {
            self.byte_field(value)?;
        } else {
            value.serialize(&mut *self)?;
        }
        self.out.push(b'}');
        Ok(())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'a, 'b>, Error> {
        self.out.push(b'[');
        Ok(Compound { json: self, first: true, close: b"]" })
    }
    fn serialize_tuple(self, len: usize) -> Result<Compound<'a, 'b>, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Compound<'a, 'b>, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'a, 'b>, Error> {
        self.out.push(b'{');
        self.string(variant);
        self.out.extend_from_slice(b":[");
        Ok(Compound { json: self, first: true, close: b"]}" })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Compound<'a, 'b>, Error> {
        self.out.push(b'{');
        Ok(Compound { json: self, first: true, close: b"}" })
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> Result<Compound<'a, 'b>, Error> {
        self.serialize_map(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'a, 'b>, Error> {
        self.out.push(b'{');
        self.string(variant);
        self.out.extend_from_slice(b":{");
        Ok(Compound { json: self, first: true, close: b"}}" })
    }
}

impl SerializeSeq for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.comma();
        value.serialize(&mut *self.json)
    }
    fn end(self) -> Result<(), Error> {
        self.json.out.extend_from_slice(self.close);
        Ok(())
    }
}

impl ser::SerializeTuple for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeMap for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        self.comma();
        let mut k = Key::default();
        key.serialize(&mut k)?;
        self.json.string(&k.0);
        self.json.out.push(b':');
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(&mut *self.json)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        self.comma();
        self.json.string(key);
        self.json.out.push(b':');
        if BYTE_FIELDS.contains(&key) {
            self.json.byte_field(value)
        } else {
            value.serialize(&mut *self.json)
        }
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl ser::SerializeStructVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        SerializeStruct::serialize_field(self, key, value)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

/// Map keys: strings, or numbers written as strings.
#[derive(Default)]
struct Key(String);

fn not_a_key() -> Error {
    Error("map key must be a string".into())
}

impl ser::Serializer for &mut Key {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = ser::Impossible<(), Error>;
    type SerializeTuple = ser::Impossible<(), Error>;
    type SerializeTupleStruct = ser::Impossible<(), Error>;
    type SerializeTupleVariant = ser::Impossible<(), Error>;
    type SerializeMap = ser::Impossible<(), Error>;
    type SerializeStruct = ser::Impossible<(), Error>;
    type SerializeStructVariant = ser::Impossible<(), Error>;

    fn serialize_str(self, v: &str) -> Result<(), Error> {
        self.0 = v.to_string();
        Ok(())
    }
    fn serialize_char(self, v: char) -> Result<(), Error> {
        self.0 = v.to_string();
        Ok(())
    }
    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        self.0 = v.to_string();
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        self.serialize_i64(v.into())
    }
    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        self.serialize_i64(v.into())
    }
    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        self.serialize_i64(v.into())
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        self.0 = v.to_string();
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.serialize_u64(v.into())
    }
    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        self.serialize_u64(v.into())
    }
    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        self.serialize_u64(v.into())
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        self.0 = v.to_string();
        Ok(())
    }
    fn serialize_f32(self, _: f32) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_f64(self, _: f64) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_none(self) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<(), Error> {
        self.0 = variant.to_string();
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), Error> {
        Err(not_a_key())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(not_a_key())
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(not_a_key())
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Error> {
        Err(not_a_key())
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(not_a_key())
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(not_a_key())
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(not_a_key())
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(not_a_key())
    }
}

/// Collects a value made only of bytes (`u8`s, byte slices, sequences of them, optional);
/// any other shape is an error, so the caller can write the value as ordinary JSON.
#[derive(Default)]
struct Bytes {
    data: Vec<u8>,
    none: bool,
}

fn not_bytes() -> Error {
    Error("not bytes".into())
}

impl<'b> ser::Serializer for &'b mut Bytes {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = &'b mut Bytes;
    type SerializeTuple = &'b mut Bytes;
    type SerializeTupleStruct = &'b mut Bytes;
    type SerializeTupleVariant = ser::Impossible<(), Error>;
    type SerializeMap = ser::Impossible<(), Error>;
    type SerializeStruct = ser::Impossible<(), Error>;
    type SerializeStructVariant = ser::Impossible<(), Error>;

    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.data.push(v);
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        self.data.extend_from_slice(v);
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Error> {
        self.none = true;
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Error> {
        Ok(self)
    }

    fn serialize_bool(self, _: bool) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_i8(self, _: i8) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_i16(self, _: i16) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_i32(self, _: i32) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_i64(self, _: i64) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_u16(self, _: u16) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_u32(self, _: u32) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_u64(self, _: u64) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_f32(self, _: f32) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_f64(self, _: f64) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_char(self, _: char) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_str(self, _: &str) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_unit(self) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), Error> {
        Err(not_bytes())
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(not_bytes())
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(not_bytes())
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(not_bytes())
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(not_bytes())
    }
}

impl SerializeSeq for &mut Bytes {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}

impl ser::SerializeTuple for &mut Bytes {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for &mut Bytes {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_stdf::StdfRecord;

    fn json_of(record: StdfRecord) -> String {
        let record_type = crate::record_type_name(&record);
        let r =
            Record { sequence_number: 1, byte_offset: 6, rec_typ: 0, rec_sub: 0, record_type, data: record };
        let mut out = Vec::new();
        write_record(&mut out, &r);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn floats_bytes_and_missing_fields() {
        let mut ptr = rust_stdf::PTR::new();
        ptr.test_num = 7;
        ptr.test_flg = [0x81];
        ptr.parm_flg = [0x42];
        ptr.result = 0.018;
        ptr.test_txt = "Vdd \"core\"\n".into();
        let json = json_of(StdfRecord::PTR(ptr.clone()));
        assert!(json.starts_with(
            r#"{"sequence_number":1,"byte_offset":6,"rec_typ":0,"rec_sub":0,"record_type":"PTR","data":{"TEST_NUM":7,"#
        ));
        assert!(
            json.contains(r#""TEST_FLG":"81","PARM_FLG":"42","RESULT":0.018,"TEST_TXT":"Vdd \"core\"\n","#)
        );
        assert!(json.contains(r#""OPT_FLAG":null"#));
        assert!(json.contains(r#""LO_LIMIT":null"#));

        for (v, want) in
            [(f32::NAN, "\"NaN\""), (f32::INFINITY, "\"Infinity\""), (f32::NEG_INFINITY, "\"-Infinity\"")]
        {
            ptr.result = v;
            assert!(json_of(StdfRecord::PTR(ptr.clone())).contains(&format!(r#""RESULT":{want}"#)));
        }
    }
}
