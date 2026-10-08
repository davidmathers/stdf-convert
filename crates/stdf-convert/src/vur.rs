//! The Version Update Record, which rust-stdf reads in only one of the two layouts in use.
//!
//! The STDF V4-2007 specification defines VUR as a single `UPD_NAM` (`C*n`), e.g. `"V4-2007"`.
//! Later extensions (the scan and memory fail datalog updates) redefine it as a count and an
//! array, so a file can name every update it uses:
//!
//! | field | type | |
//! |---|---|---|
//! | `UPD_CNT` | `U*1` | count (k) of update names |
//! | `UPD_NAM` | `k*C*n` | update names, e.g. `"V4-2007"`, `"Scan:2007.1"` |
//!
//! rust-stdf's `VUR` has the single `C*n`, so it reads `UPD_CNT` as the length of the first
//! name and drops the rest ([rust-stdf#33](https://github.com/noonchen/rust-stdf/issues/33)). [`Vur`] reads both
//! layouts: `UPD_CNT` is null for the single-name layout, so the record can be written back as it
//! was.

use std::borrow::Cow;

use rust_stdf::{KxCn, KxCnRef, StdfRecord, U1};
use serde::Serialize;

use crate::Record;

/// A VUR record's fields.
///
/// The field types are rust-stdf's STDF type aliases, which the Arrow column generator reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub struct Vur {
    /// The count of update names, or `None` for the specification's single-name layout.
    pub upd_cnt: Option<U1>,
    /// The update names: in the single-name layout, one name, or none if the record is empty.
    pub upd_nam: KxCn,
}

impl Vur {
    /// Decode a VUR record's payload (the bytes after the header).
    ///
    /// A payload that is exactly one `C*n` is the single-name layout; any other is `UPD_CNT`
    /// followed by that many `C*n`, which must fill the payload exactly. (Both readings fit
    /// only when every name is empty, and then they encode the same bytes.) Text is decoded
    /// as rust-stdf decodes `C*n`: UTF-8 if valid, otherwise Latin-1.
    pub fn decode(payload: &[u8]) -> Result<Vur, String> {
        let Some(&first) = payload.first() else { return Ok(Vur::default()) };
        if 1 + usize::from(first) == payload.len() {
            return Ok(Vur { upd_cnt: None, upd_nam: KxCnRef::new(payload, 0, 1).to_owned() });
        }
        let k = usize::from(first);
        let mut pos = 1;
        for i in 0..k {
            let Some(&len) = payload.get(pos) else {
                return Err(format!("UPD_CNT is {k}, but the record holds only {i}"));
            };
            pos += 1 + usize::from(len);
            if pos > payload.len() {
                let have = len as usize - (pos - payload.len());
                return Err(format!(
                    "UPD_CNT is {k}, but update name {} is cut short ({have} of {len} bytes)",
                    i + 1
                ));
            }
        }
        if pos < payload.len() {
            let extra = payload.len() - pos;
            return Err(format!("UPD_CNT is {k}, but {extra} byte(s) follow the update names"));
        }
        Ok(Vur { upd_cnt: Some(first), upd_nam: KxCnRef::new(payload, 1, k).to_owned() })
    }

    /// The VUR fields of `record`, which must be a VUR record: those [`crate::RecordReader`]
    /// decoded, or for a record built without them, rust-stdf's single name.
    pub(crate) fn of(record: &Record) -> Cow<'_, Vur> {
        match (&record.vur, &record.data) {
            (Some(vur), _) => Cow::Borrowed(vur),
            (None, StdfRecord::VUR(r)) => Cow::Owned(Vur { upd_cnt: None, upd_nam: vec![r.upd_nam.clone()] }),
            (None, _) => panic!("VUR fields requested for a {} record", record.record_type),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vur(upd_cnt: Option<u8>, names: &[&str]) -> Vur {
        Vur { upd_cnt, upd_nam: names.iter().map(|n| n.to_string()).collect() }
    }

    #[test]
    fn counted_layout_keeps_every_name() {
        // UPD_CNT=2, "V4-2007", "Scan:2007.1"
        let payload = b"\x02\x07V4-2007\x0bScan:2007.1";
        assert_eq!(Vur::decode(payload), Ok(vur(Some(2), &["V4-2007", "Scan:2007.1"])));
        assert_eq!(Vur::decode(b"\x01\x07V4-2007"), Ok(vur(Some(1), &["V4-2007"])));
        assert_eq!(Vur::decode(b"\x02\x00\x01x"), Ok(vur(Some(2), &["", "x"])));
    }

    #[test]
    fn single_name_layout() {
        assert_eq!(Vur::decode(b"\x07V4-2007"), Ok(vur(None, &["V4-2007"])));
        assert_eq!(Vur::decode(b"\x00"), Ok(vur(None, &[""])));
        assert_eq!(Vur::decode(b""), Ok(vur(None, &[])));
        // Latin-1, as rust-stdf decodes C*n that is not UTF-8
        assert_eq!(Vur::decode(b"\x01\xe9"), Ok(vur(None, &["é"])));
    }

    #[test]
    fn malformed_payloads_are_errors() {
        let cases: [(&[u8], &str); 5] = [
            (b"\x02\x07V4-2007", "UPD_CNT is 2, but the record holds only 1"),
            (b"\x02\x07V4-2007\x0bScan:20", "UPD_CNT is 2, but update name 2 is cut short (7 of 11 bytes)"),
            (b"\x01\x07V4-2007xy", "UPD_CNT is 1, but 2 byte(s) follow the update names"),
            (b"\x00\x00", "UPD_CNT is 0, but 1 byte(s) follow the update names"),
            // a single-name record cut short reads as a count, and fails
            (b"\x07V4-20", "UPD_CNT is 7, but update name 1 is cut short (4 of 86 bytes)"),
        ];
        for (payload, want) in cases {
            assert_eq!(Vur::decode(payload), Err(want.to_string()), "{payload:?}");
        }
    }
}
