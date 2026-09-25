//! End-to-end tests on small synthetic STDF files.

use std::path::PathBuf;

use stdf_convert::{Error, RecordReader, json};

/// Little-endian FAR (CPU_TYPE=2, STDF_VER=4) followed by PIR (head=1, site=2).
const STDF: [u8; 12] = [2, 0, 0, 10, 2, 4, 2, 0, 5, 10, 1, 2];

fn file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stdf-convert-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn with_record(typ: u8, sub: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = STDF[..6].to_vec();
    b.extend((payload.len() as u16).to_le_bytes());
    b.extend([typ, sub]);
    b.extend(payload);
    b
}

fn jsonl(path: &PathBuf, filter: Option<&[&str]>) -> String {
    let mut out = Vec::new();
    json::write_json_lines(RecordReader::open(path, filter).unwrap(), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn json_lines() {
    let path = file("basic.stdf", &STDF);
    assert_eq!(
        jsonl(&path, None),
        concat!(
            r#"{"sequence_number":0,"byte_offset":0,"rec_typ":0,"rec_sub":10,"record_type":"FAR","data":{"CPU_TYPE":2,"STDF_VER":4}}"#,
            "\n",
            r#"{"sequence_number":1,"byte_offset":6,"rec_typ":5,"rec_sub":10,"record_type":"PIR","data":{"HEAD_NUM":1,"SITE_NUM":2}}"#,
            "\n"
        )
    );
}

#[test]
fn filter_is_case_insensitive_and_keeps_positions() {
    let path = file("filter.stdf", &STDF);
    let out = jsonl(&path, Some(&["pir"]));
    assert!(out.starts_with(r#"{"sequence_number":1,"byte_offset":6,"#));
    assert_eq!(out.lines().count(), 1);
    assert!(
        matches!(RecordReader::open(&path, Some(&["nope"])), Err(Error::UnknownRecordType(t)) if t == "NOPE")
    );
}

#[test]
fn big_endian() {
    let path = file("be.stdf", &[0, 2, 0, 10, 1, 4, 0, 2, 5, 10, 1, 2]);
    assert!(jsonl(&path, None).contains(r#""data":{"CPU_TYPE":1,"STDF_VER":4}"#));
}

#[test]
fn reserved_records_keep_their_bytes_as_hex() {
    let path = file("reserved.stdf", &with_record(180, 7, b"\x00\x80\xff"));
    assert!(
        jsonl(&path, Some(&["reserved"]))
            .contains(r#""record_type":"RESERVED","data":{"RAW_DATA":"0080ff"}"#)
    );
}

#[test]
fn invalid_records() {
    let path = file("invalid.stdf", &with_record(99, 99, b""));
    assert!(
        jsonl(&path, Some(&["invalid"]))
            .contains(r#""record_type":"INVALID","data":{"LEN":0,"TYP":99,"SUB":99}"#)
    );
}

#[test]
fn v4_2007_record() {
    let path = file("vur.stdf", &with_record(0, 30, b"\x07V4-2007"));
    assert!(jsonl(&path, Some(&["vur"])).contains(r#""data":{"UPD_NAM":"V4-2007"}"#));
}

#[test]
fn gdr_nested_bytes() {
    // GDR with two fields: B*n [00 80 ff] and D*n with 8 bits [a5]
    let mut payload = vec![2, 0, 11, 3, 0x00, 0x80, 0xff, 12, 8, 0];
    payload.push(0xa5);
    let path = file("gdr.stdf", &with_record(50, 10, &payload));
    assert!(jsonl(&path, Some(&["gdr"])).contains(r#""GEN_DATA":[{"Bn":"0080ff"},{"Dn":"a5"}]"#));
}

#[test]
fn truncated_file_is_an_error() {
    let mut bytes = STDF[..6].to_vec();
    bytes.extend([2, 0, 5, 10, 1]);
    let path = file("truncated.stdf", &bytes);
    let mut out = Vec::new();
    let err = json::write_json_lines(RecordReader::open(&path, None).unwrap(), &mut out).unwrap_err();
    assert!(err.to_string().contains("failed to fill whole buffer"), "{err}");
}

#[test]
fn not_stdf_is_an_error() {
    let path = file("not.stdf", b"not an STDF file");
    assert!(matches!(RecordReader::open(&path, None), Err(Error::Open { .. })));
}
