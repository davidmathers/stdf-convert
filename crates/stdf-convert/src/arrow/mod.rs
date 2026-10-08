//! Arrow output: one table per record type, with a schema taken from rust-stdf's record
//! definitions.
//!
//! Every table starts with the record's position and header, then has one column per record
//! field, named as in [`crate::json`]:
//!
//! | column | type | |
//! |---|---|---|
//! | `sequence_number` | `uint64` | position of the record in the file (unaffected by filtering) |
//! | `byte_offset` | `uint64` | offset of the record header in the decompressed stream |
//! | `rec_len`, `rec_typ`, `rec_sub` | `uint16`, `uint8`, `uint8` | the record header |
//! | `TEST_NUM`, ... | see below | the record's fields, in specification order |
//!
//! Each field's column type follows from the STDF type rust-stdf declares it with, kept in the
//! field's `stdf_type` metadata: `U1`..`U8` are unsigned integers, `I1`..`I4` signed, `R4`
//! `float32`, text is `utf8`, `B1` and `Bn` are `binary`, `Dn` bit fields are
//! `struct<bit_count, bit_data>`, and `Kx..` arrays are lists.
//! Optional fields are nullable, and null only where the record left them out; NaN stays NaN.
//! `KxUf` arrays are widened to `list<uint64>` (the record's `*_SIZE` fields give the width
//! in the file). `GDR.GEN_DATA` is a list of structs: the value's type (`U1`, `Cn`, `B0`, ...)
//! and a column per type, set only for values of that type.
//! `VUR` has `UPD_CNT` and `UPD_NAM` (`list<utf8>`) from [`crate::Vur`] rather than rust-stdf's
//! single name.
//!
//! ```no_run
//! for batch in stdf_convert::arrow::BatchReader::open("results.stdf.gz", None, 65_536)? {
//!     let (record_type, batch) = batch?;
//!     println!("{record_type}: {} rows", batch.num_rows());
//! }
//! # Ok::<(), stdf_convert::Error>(())
//! ```

mod col;
#[rustfmt::skip]
mod generated;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::builder::UInt64Builder;
use arrow_schema::{DataType, Field, Schema, SchemaRef};

pub use col::STDF_TYPE;
pub use generated::RUST_STDF_VERSION;
pub(crate) use generated::{BYTE_FIELDS, RECORD_TYPES, record_index};

use crate::{Error, Record, RecordReader, Result};
use col::Column;
use generated::Columns;

/// Rows per batch used by the Python package.
pub const DEFAULT_BATCH_SIZE: usize = 65_536;

/// The schema of `record_type`'s table (case-insensitive, as in [`crate::RECORD_TYPES`]).
///
/// The schema metadata holds `record_type`, `stdf_convert_version` and `rust_stdf_version`.
pub fn schema(record_type: &str) -> Result<SchemaRef> {
    let upper = record_type.to_ascii_uppercase();
    let name =
        RECORD_TYPES.iter().copied().find(|&known| known == upper).ok_or(Error::UnknownRecordType(upper))?;
    Ok(schema_of(name))
}

fn schema_of(record_type: &'static str) -> SchemaRef {
    let mut fields = vec![
        Field::new("sequence_number", DataType::UInt64, false),
        Field::new("byte_offset", DataType::UInt64, false),
        col::U2::field("rec_len"),
        col::U1::field("rec_typ"),
        col::U1::field("rec_sub"),
    ];
    fields.extend(Columns::fields(record_type).expect("a known record type"));
    let metadata = HashMap::from([
        ("record_type".to_string(), record_type.to_string()),
        ("stdf_convert_version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
        ("rust_stdf_version".to_string(), RUST_STDF_VERSION.to_string()),
    ]);
    Arc::new(Schema::new_with_metadata(fields, metadata))
}

/// The rows of one record type collected so far.
struct Table {
    schema: SchemaRef,
    sequence_number: UInt64Builder,
    byte_offset: UInt64Builder,
    rec_len: col::U2,
    rec_typ: col::U1,
    rec_sub: col::U1,
    columns: Columns,
    rows: usize,
}

impl Table {
    fn new(record_type: &'static str) -> Self {
        Table {
            schema: schema_of(record_type),
            sequence_number: UInt64Builder::new(),
            byte_offset: UInt64Builder::new(),
            rec_len: Default::default(),
            rec_typ: Default::default(),
            rec_sub: Default::default(),
            columns: Columns::new(record_type).expect("a known record type"),
            rows: 0,
        }
    }

    fn append(&mut self, record: &Record) {
        self.sequence_number.append_value(record.sequence_number);
        self.byte_offset.append_value(record.byte_offset);
        self.rec_len.append(&record.rec_len);
        self.rec_typ.append(&record.rec_typ);
        self.rec_sub.append(&record.rec_sub);
        self.columns.append(record);
        self.rows += 1;
    }

    fn finish(&mut self) -> Result<RecordBatch> {
        let mut columns = vec![
            Arc::new(self.sequence_number.finish()) as _,
            Arc::new(self.byte_offset.finish()) as _,
            self.rec_len.finish(),
            self.rec_typ.finish(),
            self.rec_sub.finish(),
        ];
        columns.extend(self.columns.finish());
        self.rows = 0;
        Ok(RecordBatch::try_new(self.schema.clone(), columns)?)
    }
}

/// Reads an STDF file as Arrow record batches, one record type per batch.
///
/// A batch of a record type is produced each time `batch_size` of its records have been read,
/// and the remaining rows of every type at the end of the file, in [`crate::RECORD_TYPES`]
/// order. Batches of one type come in file order. After an error, the iterator ends.
pub struct BatchReader {
    records: RecordReader,
    tables: Vec<Option<Table>>,
    batch_size: usize,
    /// At the end of the file: the next table to flush.
    flushing: Option<usize>,
    failed: bool,
}

impl BatchReader {
    /// Open `path`, optionally keeping only the given record types (case-insensitive).
    pub fn open(path: impl AsRef<Path>, record_types: Option<&[&str]>, batch_size: usize) -> Result<Self> {
        Ok(Self::new(RecordReader::open(path, record_types)?, batch_size))
    }

    pub fn new(records: RecordReader, batch_size: usize) -> Self {
        BatchReader {
            records,
            tables: RECORD_TYPES.iter().map(|_| None).collect(),
            batch_size: batch_size.max(1),
            flushing: None,
            failed: false,
        }
    }
}

impl Iterator for BatchReader {
    type Item = Result<(&'static str, RecordBatch)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(start) = self.flushing {
                let i = (start..self.tables.len())
                    .find(|&i| self.tables[i].as_ref().is_some_and(|t| t.rows > 0))?;
                self.flushing = Some(i + 1);
                return Some(self.tables[i].as_mut().unwrap().finish().map(|b| (RECORD_TYPES[i], b)));
            }
            let record = match self.records.next() {
                None => {
                    self.flushing = Some(0);
                    continue;
                }
                Some(Err(e)) => {
                    self.failed = true;
                    return Some(Err(e));
                }
                Some(Ok(record)) => record,
            };
            let i = record_index(&record.data);
            let table = self.tables[i].get_or_insert_with(|| Table::new(RECORD_TYPES[i]));
            table.append(&record);
            if table.rows >= self.batch_size {
                return Some(table.finish().map(|b| (RECORD_TYPES[i], b)));
            }
        }
    }
}

/// Read all of `path` into one record batch per record type present, in
/// [`crate::RECORD_TYPES`] order.
pub fn read_tables(
    path: impl AsRef<Path>,
    record_types: Option<&[&str]>,
) -> Result<Vec<(&'static str, RecordBatch)>> {
    BatchReader::open(path, record_types, usize::MAX)?.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Float32Type, UInt8Type, UInt64Type};
    use arrow_array::{Array, ListArray};
    use rust_stdf::stdf_record_type::get_rec_name_from_code;
    use rust_stdf::{Dn, KxUf, ReservedRec, StdfRecord, V1};

    fn batch_of(records: Vec<StdfRecord>) -> RecordBatch {
        let name = crate::record_type_name(&records[0]);
        let mut table = Table::new(RECORD_TYPES.iter().copied().find(|&t| t == name).unwrap());
        for (i, data) in records.into_iter().enumerate() {
            let record_type = crate::record_type_name(&data);
            let i = i as u64;
            table.append(&Record {
                sequence_number: i,
                byte_offset: 10 * i,
                rec_len: 6,
                rec_typ: 15,
                rec_sub: 10,
                record_type,
                data,
                vur: None,
            });
        }
        table.finish().unwrap()
    }

    #[test]
    fn every_record_type_builds_batches_matching_its_schema() {
        let mut records: Vec<StdfRecord> = (0..32).map(|i| StdfRecord::new(1 << i)).collect();
        for r in &records {
            assert_eq!(crate::record_type_name(r), get_rec_name_from_code(r.get_type()));
        }
        records.push(StdfRecord::ReservedRec(ReservedRec::new()));
        records.push(StdfRecord::UnknownRec(ReservedRec::new()));
        let names: Vec<_> = records.iter().map(crate::record_type_name).collect();
        assert_eq!(names, RECORD_TYPES);
        for r in records {
            let name = crate::record_type_name(&r);
            let batch = batch_of(vec![r]);
            assert_eq!(batch.schema(), schema(name).unwrap(), "{name}");
            assert_eq!(batch.num_rows(), 1);
        }
    }

    #[test]
    fn schema_types_nullability_and_metadata() {
        let s = schema("ptr").unwrap();
        assert_eq!(s.metadata()["record_type"], "PTR");
        assert_eq!(s.metadata()["rust_stdf_version"], RUST_STDF_VERSION);
        let names: Vec<_> = s.fields().iter().take(6).map(|f| f.name().as_str()).collect();
        assert_eq!(names, ["sequence_number", "byte_offset", "rec_len", "rec_typ", "rec_sub", "TEST_NUM"]);
        let field = |n: &str| s.field_with_name(n).unwrap().clone();
        assert_eq!(
            (field("TEST_NUM").data_type(), field("TEST_NUM").is_nullable()),
            (&DataType::UInt32, false)
        );
        assert_eq!((field("RESULT").data_type(), field("RESULT").is_nullable()), (&DataType::Float32, false));
        assert_eq!(
            (field("LO_LIMIT").data_type(), field("LO_LIMIT").is_nullable()),
            (&DataType::Float32, true)
        );
        assert_eq!(field("TEST_FLG").data_type(), &DataType::Binary);
        assert_eq!(field("RES_SCAL").data_type(), &DataType::Int8);
        assert_eq!(field("LO_LIMIT").metadata()[STDF_TYPE], "R4");
        assert_eq!(field("TEST_FLG").metadata()[STDF_TYPE], "B1");
        let mpr = schema("MPR").unwrap();
        assert_eq!(mpr.field_with_name("RTN_STAT").unwrap().metadata()[STDF_TYPE], "KxN1");
        assert!(matches!(schema("nope"), Err(Error::UnknownRecordType(t)) if t == "NOPE"));
    }

    #[test]
    fn ptr_values_keep_nan_nulls_and_exact_floats() {
        let mut ptr = rust_stdf::PTR::new();
        ptr.test_num = 4_000_000_000;
        ptr.test_flg = [0x81];
        ptr.result = f32::NAN;
        ptr.lo_limit = Some(0.018);
        ptr.units = Some("V".into());
        let batch = batch_of(vec![StdfRecord::PTR(ptr.clone()), StdfRecord::PTR(rust_stdf::PTR::new())]);
        let col = |n: &str| batch.column_by_name(n).unwrap().clone();
        assert_eq!(col("TEST_NUM").as_primitive::<arrow_array::types::UInt32Type>().value(0), 4_000_000_000);
        let result = col("RESULT");
        assert!(result.as_primitive::<Float32Type>().value(0).is_nan());
        assert_eq!(result.null_count(), 0);
        let lo = col("LO_LIMIT");
        assert_eq!(lo.as_primitive::<Float32Type>().value(0), 0.018_f32);
        assert!(lo.is_null(1));
        assert!(col("HI_LIMIT").is_null(0));
        assert_eq!(col("TEST_FLG").as_binary::<i32>().value(0), [0x81]);
        assert_eq!(col("UNITS").as_string::<i32>().value(0), "V");
        assert_eq!(col("byte_offset").as_primitive::<UInt64Type>().values().to_vec(), [0, 10]);
    }

    #[test]
    fn gdr_values_are_typed_structs() {
        let mut gdr = rust_stdf::GDR::new();
        gdr.gen_data = vec![V1::B0, V1::U1(5), V1::Cn("x".into()), V1::Bn(vec![0, 0x80]), V1::N1(3)];
        gdr.fld_cnt = 5;
        let batch = batch_of(vec![StdfRecord::GDR(gdr)]);
        let list: &ListArray = batch.column_by_name("GEN_DATA").unwrap().as_list();
        let values = list.value(0);
        let values = values.as_struct();
        let types: Vec<_> =
            values.column_by_name("TYPE").unwrap().as_string::<i32>().iter().flatten().collect();
        assert_eq!(types, ["B0", "U1", "Cn", "Bn", "N1"]);
        let u1 = values.column_by_name("U1").unwrap().as_primitive::<UInt8Type>();
        assert_eq!((u1.is_null(0), u1.value(1), u1.is_null(2)), (true, 5, true));
        assert_eq!(values.column_by_name("Cn").unwrap().as_string::<i32>().value(2), "x");
        assert_eq!(values.column_by_name("Bn").unwrap().as_binary::<i32>().value(3), [0, 0x80]);
        assert_eq!(values.column_by_name("N1").unwrap().as_primitive::<UInt8Type>().value(4), 3);
    }

    #[test]
    fn dn_bit_fields_keep_their_bit_count() {
        let mut ftr = rust_stdf::FTR::new();
        ftr.fail_pin = Dn { bit_count: 5, bit_data: vec![0x15] };
        let mut gdr = rust_stdf::GDR::new();
        gdr.gen_data = vec![V1::U1(1), V1::Dn(Dn { bit_count: 12, bit_data: vec![0xff, 0x0a] })];
        let fail_pin = batch_of(vec![StdfRecord::FTR(ftr)]).column_by_name("FAIL_PIN").unwrap().clone();
        let fail_pin = fail_pin.as_struct();
        let bit_count =
            fail_pin.column_by_name("bit_count").unwrap().as_primitive::<arrow_array::types::UInt16Type>();
        assert_eq!(bit_count.value(0), 5);
        assert_eq!(fail_pin.column_by_name("bit_data").unwrap().as_binary::<i32>().value(0), [0x15]);

        let batch = batch_of(vec![StdfRecord::GDR(gdr)]);
        let list: &ListArray = batch.column_by_name("GEN_DATA").unwrap().as_list();
        let values = list.value(0);
        let dn = values.as_struct().column_by_name("Dn").unwrap().as_struct().clone();
        assert!(dn.is_null(0));
        assert_eq!(dn.column_by_name("bit_data").unwrap().as_binary::<i32>().value(1), [0xff, 0x0a]);
    }

    #[test]
    fn str_kxuf_arrays_widen_to_u64() {
        let mut s = rust_stdf::STR::new();
        s.cyc_ofst = KxUf::F2(vec![1, 65535]);
        s.pmr_indx = KxUf::F8(vec![u64::MAX]);
        let batch = batch_of(vec![StdfRecord::STR(s)]);
        let values = |n: &str| {
            let list: &ListArray = batch.column_by_name(n).unwrap().as_list();
            list.value(0).as_primitive::<UInt64Type>().values().to_vec()
        };
        assert_eq!(values("CYC_OFST"), [1, 65535]);
        assert_eq!(values("PMR_INDX"), [u64::MAX]);
    }
}
