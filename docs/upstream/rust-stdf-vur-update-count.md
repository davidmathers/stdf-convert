# rust-stdf: VUR with an update count is misread (`UPD_CNT` taken as the name length)

Background for [noonchen/rust-stdf#33](https://github.com/noonchen/rust-stdf/issues/33), filed 2026-10-08 with the text in
[rust-stdf-vur-issue.md](rust-stdf-vur-issue.md).
stdf-convert works around it in [`crates/stdf-convert/src/vur.rs`](../../crates/stdf-convert/src/vur.rs);
when rust-stdf is fixed, remove the workaround (see [After an upstream fix](#after-an-upstream-fix)).

---

## Summary

`VUR` (Version Update Record, type 0 / subtype 30) is defined in rust-stdf 1.1.0 as a single
`C*n`:

```rust
pub struct VUR {
    pub upd_nam: Cn, //Update Version Name
}
```

This matches the layout in the published STDF V4-2007 specification. But testers also write VUR
in a later layout, a count followed by an array of names, so that a file can list every update it
uses (e.g. the base standard and the scan extension):

| Field | Type | Description |
|---|---|---|
| `UPD_CNT` | `U*1` | Count (k) of update names |
| `UPD_NAM` | `k*C*n` | Update names |

rust-stdf reads such a record's `UPD_CNT` byte as the length of `UPD_NAM`, so it returns the wrong
text and silently drops the rest of the record. No error is reported.

## Versions

- rust-stdf 1.1.0 (and earlier: the struct is unchanged)
- Found through stdf-convert 0.2.0, reading a production V4-2007 file whose VUR is written as below

## Reproduction

Payload (21 bytes, after the 4-byte header `15 00 00 1e`, little-endian):

```text
02 07 56 34 2d 32 30 30 37 0b 53 63 61 6e 3a 32 30 30 37 2e 31
```

i.e. `UPD_CNT = 2`, `UPD_NAM = ["V4-2007", "Scan:2007.1"]`.

```toml
[dependencies]
rust-stdf = "=1.1.0"
```

```rust
use rust_stdf::{ByteOrder, RawDataElement, RecordHeader, StdfRecord};

fn main() {
    // VUR payload: UPD_CNT=2, UPD_NAM=["V4-2007", "Scan:2007.1"]
    let payload = b"\x02\x07V4-2007\x0bScan:2007.1".to_vec();
    let raw = RawDataElement {
        offset: 0,
        header: RecordHeader { len: payload.len() as u16, typ: 0, sub: 30 },
        raw_data: payload,
        byte_order: ByteOrder::LittleEndian,
    };
    match StdfRecord::from(&raw) {
        StdfRecord::VUR(vur) => println!("{:?}", vur.upd_nam),
        other => panic!("not a VUR: {other:?}"),
    }
}
```

**Expected:** both names, `["V4-2007", "Scan:2007.1"]` (and the count, 2).

**Actual:** `"\u{7}V"`. `read_cn` takes `0x02` as the string length and reads `07 56`; the remaining
18 bytes are ignored.

Verified against rust-stdf 1.1.0 on 2026-10-07.

## Impact

- The update names, which tell readers which extensions (e.g. scan fail data) a file uses, are lost
  or garbled, with no error.
- Writing the record back (`StdfWriter`) produces a different record (a single 2-byte `C*n`), so
  read-then-write is not safe for such files.
- The serde output (`serialize` feature) and the zero-copy `VURView`, both derived from the same
  struct, show the same garbled string.

## Specification background

- The STDF V4-2007 specification defines only `UPD_NAM C*n  Update Version Name`, with
  `"V4-2007"` as the example value. The two copies found online are both Word exports by the
  editor rather than a dated final edition, and both say this: one from July 2008
  ("Draft_07_09_08", [hosted by Roos](https://www.roos.com/roos/documentation.nsf/3d6a93a7e05462cf85256a9c007dcaf3/92102f712ce51df48825783800832332/$FILE/STDF%20Spec%20V4%202007.pdf)),
  and one from January 2009 ("ScanFailDatalog_2009_1_9", [on GitHub](https://github.com/lantianjialiang/stdf_spec/blob/master/stdf_V4-2007_spec.pdf)).
  rust-stdf's other V4-2007 records (e.g. STR) match the January 2009 copy, which differs from the
  July 2008 one.
- The count-and-array layout (`UPD_CNT U*1`, `UPD_NAM k*C*n`) comes from SEMI's STDF memory fail
  datalog work. According to search summaries (the document itself was not checked), SEMI Draft
  Document 4782 defines VUR as a count of entries and an array of version update names, with values
  such as `"Memory:2"`. `"Scan:2007.1"` follows the same naming pattern.
- That draft was published as [SEMI G91](https://store-us.semi.org/products/g09100-semi-g91-standard-test-data-format-stdf-memory-fail-datalog),
  *Standard Test Data Format (STDF) Memory Fail Datalog* (first revision G91-0513). SEMI lists it as
  inactive, which it says leaves it valid for use. G91 is a paid document; before filing, confirm
  that its VUR definition matches the draft's, and cite the clause.
- Other readers implement the count layout. Spry Software's Java STDF reader
  ([`spry.reader.stdf.VUR`](https://sprysoftware.net/javadoc/javadoc/spry/reader/stdf/VUR.html))
  has `getUpd_cnt()` (`short`) and `getUpd_nam()` (`String[]`), and a separate
  `readDraftFormatRecord`, apparently for the other layout. Its documentation cites no
  specification, so this shows how files are written in practice, not what a standard says.
- Both layouts occur in real files, so a reader has to accept both.

References (bibliographic details from Crossref):

- A. Khoche, J. Katz, S. Landini, K. Liao, N. Agrawal, G. Plowman, S. Zuo, L. Lai, J. Rowe,
  T. Zanon, "STDF Memory Fail Datalog Standard", *2009 27th IEEE VLSI Test Symposium*,
  pp. 209–214, May 2009. [doi:10.1109/VTS.2009.29](https://doi.org/10.1109/VTS.2009.29)
  ([Semantic Scholar](https://www.semanticscholar.org/paper/STDF-Memory-Fail-Datalog-Standard-Khoche-Katz/18a756b95d0f8951d569410bebd58ffd54d3ea8a)).
  The memory fail datalog proposal that became SEMI G91. Checked 2026-10-07: it defines the new
  memory records (MSR, MCR, IDR, MMR, ASR, FSR, BSR, MTR) but does not mention VUR, `UPD_CNT` or
  update names, so it is background only, not a source for the VUR layout.
- A. Khoche, P. Burlison, J. Rowe, G. Plowman, "A Tutorial on STDF Fail Datalog Standard",
  *2008 IEEE International Test Conference*, pp. 1–10, Oct. 2008.
  [doi:10.1109/TEST.2008.4700654](https://doi.org/10.1109/TEST.2008.4700654).
  The scan fail datalog that became STDF V4-2007. Not checked.
- SEMI G91, *Standard Test Data Format (STDF) Memory Fail Datalog*.
- *Standard Test Data Format (STDF) V4-2007 Specification*, VUR section (January 2009 copy).

## Suggested fix

Distinguish the two layouts by the record length; they can only both fit when every name is empty,
and then they encode the same bytes:

1. Empty payload: no names.
2. Payload is exactly one `C*n` (`1 + payload[0] == rec_len`): the specification's single name.
3. Otherwise: `UPD_CNT` then `UPD_CNT` × `C*n`, which should fill the record exactly; report an
   error (or at least don't silently truncate) when it doesn't.

A shape that keeps the layout, so that writing reproduces the original bytes:

```rust
pub struct VUR {
    pub upd_cnt: Option<U1>, // None: the V4-2007 single-name layout
    pub upd_nam: KxCn,
}
```

This is a breaking change to the public `VUR` struct (and its serde output), so it likely needs a
major version. rust-stdf's existing `KxCnRef` / `read_kx_cn` already decode the names.

---

## After an upstream fix

stdf-convert's workaround, to remove once rust-stdf reads both layouts:

- `crates/stdf-convert/src/vur.rs`: `Vur` and its decoder (`Vur::decode`, `Vur::of`), with unit
  tests for both layouts and malformed records.
- `Record.vur` and the VUR decoding in `RecordReader` (`crates/stdf-convert/src/lib.rs`), and
  `Error::Malformed`.
- The `VUR` entry in `OVERRIDES` in `crates/codegen/src/lib.rs`, which generates VUR's Arrow columns
  from `Vur` rather than `rust_stdf::VUR`.
- The `StdfRecord::VUR` arm in `json::RecordData`.

Keep the Arrow schema and JSON output (`UPD_CNT` uint8, nullable; `UPD_NAM` list of strings), and
keep the regression tests (`crates/stdf-convert/tests/records.rs`, `python/tests/test_arrow.py`),
which use only the synthetic payloads above.
