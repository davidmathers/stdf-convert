//! Read STDF V4 and V4-2007 semiconductor test data files, and write them as JSON Lines:
//! the library behind the `stdf-convert` command and Python package.
//!
//! Parsing is done by [`rust_stdf`]. This crate adds a record iterator that tracks each
//! record's position in the file and filters by record type, and a JSON Lines writer.
//!
//! ```no_run
//! let records = stdf_convert::RecordReader::open("results.stdf.gz", None)?;
//! let out = std::fs::File::create("results.jsonl")?;
//! stdf_convert::json::write_json_lines(records, out)?;
//! # Ok::<(), stdf_convert::Error>(())
//! ```

mod cli;
pub mod json;

pub use cli::{output_path, run};

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub use rust_stdf;
use rust_stdf::StdfRecord;
use rust_stdf::stdf_file::StdfReader;
use rust_stdf::stdf_record_type::get_rec_name_from_code;

/// Every record type name, as used by [`RecordReader::open`]'s filter and
/// [`Record::record_type`]. `RESERVED` and `INVALID` cover records that are not part of
/// the STDF specification.
pub const RECORD_TYPES: &[&str] = &[
    "FAR", "ATR", "VUR", "MIR", "MRR", "PCR", "HBR", "SBR", "PMR", "PGR", "PLR", "RDR", "SDR", "PSR", "NMR",
    "CNR", "SSR", "CDR", "WIR", "WRR", "WCR", "PIR", "PRR", "TSR", "PTR", "MPR", "FTR", "STR", "BPS", "EPS",
    "GDR", "DTR", "RESERVED", "INVALID",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {msg}")]
    Open { path: PathBuf, msg: String },
    #[error("{0}")]
    Read(String),
    #[error("unknown STDF record type {0:?}")]
    UnknownRecordType(String),
    /// Reading or writing a file failed.
    #[error("{path}: {source}")]
    File { path: PathBuf, source: std::io::Error },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Fields that hold bytes or packed bits rather than numbers: `bytes` in Python, hex in JSON.
/// `Bn` and `Dn` are the byte and bit values inside `GDR.GEN_DATA`.
pub const BYTE_FIELDS: &[&str] = &[
    "RAW_DATA", "PART_FIX", "CONT_FLG", "OPT_FLG", "PART_FLG", "OPT_FLAG", "TEST_FLG", "PARM_FLG",
    "FAIL_PIN", "SPIN_MAP", "FMU_FLG", "MASK_MAP", "FAL_MAP", "Bn", "Dn",
];

/// Convert `input` to JSON Lines at `output`, keeping only `record_types` if given, and return
/// how many records were written. The folder is created if needed, and the file is written
/// under a temporary name first, so a failed conversion never leaves a partial output.
pub fn convert_file(input: &Path, output: &Path, record_types: Option<&[&str]>) -> Result<u64> {
    let file_err = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::File { path, source }
    };
    let records = RecordReader::open(input, record_types)?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(file_err(parent))?;
    }
    let tmp = output.with_extension("jsonl.tmp");
    let result = File::create(&tmp)
        .map_err(file_err(&tmp))
        .and_then(|f| json::write_json_lines(records, f))
        .and_then(|n| std::fs::rename(&tmp, output).map(|()| n).map_err(file_err(output)));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One record and where it came from.
pub struct Record {
    /// Zero-based position of the record in the file (unaffected by filtering).
    pub sequence_number: u64,
    /// Zero-based offset of the record header in the decompressed stream.
    pub byte_offset: u64,
    /// The header's record type and subtype codes, kept for reserved and invalid records.
    pub rec_typ: u8,
    pub rec_sub: u8,
    /// `"PTR"`, `"PIR"`, ..., `"RESERVED"` or `"INVALID"`.
    pub record_type: &'static str,
    pub data: StdfRecord,
}

/// Streams the records of an STDF file, which may be gzip (`.gz`), bzip2 (`.bz2`) or zip
/// (`.zip`, first member) compressed.
pub struct RecordReader {
    inner: StdfReader<BufReader<File>>,
    filter: Option<HashSet<&'static str>>,
    next_sequence_number: u64,
    next_byte_offset: u64,
}

impl RecordReader {
    /// Open `path`, optionally keeping only the given record types (case-insensitive).
    pub fn open(path: impl AsRef<Path>, record_types: Option<&[&str]>) -> Result<Self> {
        let filter = record_types
            .map(|types| {
                types
                    .iter()
                    .map(|t| {
                        let upper = t.to_ascii_uppercase();
                        RECORD_TYPES
                            .iter()
                            .copied()
                            .find(|&known| known == upper)
                            .ok_or(Error::UnknownRecordType(upper))
                    })
                    .collect::<Result<HashSet<_>>>()
            })
            .transpose()?;
        let path = path.as_ref();
        let inner = StdfReader::new(path)
            .map_err(|e| Error::Open { path: path.to_path_buf(), msg: e.to_string() })?;
        Ok(RecordReader { inner, filter, next_sequence_number: 0, next_byte_offset: 0 })
    }
}

impl Iterator for RecordReader {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let raw = match self.inner.get_rawdata_iter().next()? {
                Ok(raw) => raw,
                Err(e) => return Some(Err(Error::Read(e.to_string()))),
            };
            let sequence_number = self.next_sequence_number;
            self.next_sequence_number += 1;
            let byte_offset = self.next_byte_offset;
            self.next_byte_offset += u64::from(raw.header.len) + 4;
            let data = StdfRecord::from(&raw);
            let record_type = record_type_name(&data);
            if self.filter.as_ref().is_some_and(|f| !f.contains(record_type)) {
                continue;
            }
            return Some(Ok(Record {
                sequence_number,
                byte_offset,
                rec_typ: raw.header.typ,
                rec_sub: raw.header.sub,
                record_type,
                data,
            }));
        }
    }
}

/// The record type name of `record`, as in [`RECORD_TYPES`].
pub fn record_type_name(record: &StdfRecord) -> &'static str {
    match record {
        StdfRecord::ReservedRec(_) => "RESERVED",
        StdfRecord::InvalidRec(_) => "INVALID",
        _ => get_rec_name_from_code(record.get_type()),
    }
}
