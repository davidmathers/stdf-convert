<!-- Filed as https://github.com/noonchen/rust-stdf/issues/33 on 2026-10-08. Background and stdf-convert's workaround:
     rust-stdf-vur-update-count.md. -->

**Title:** VUR written as `UPD_CNT` + `UPD_NAM` array is misread: count byte taken as name length

---

`VUR` is declared as a single `C*n`:

```rust
pub struct VUR {
    pub upd_nam: Cn,
}
```

That matches the V4-2007 specification, but testers also write VUR as a count followed by an array
of names, so a file can list every update it uses:

| Field | Type |
|---|---|
| `UPD_CNT` | `U*1` |
| `UPD_NAM` | `k*C*n` |

rust-stdf reads `UPD_CNT` as the length of the first name, returns the wrong text, and drops the
rest of the record without an error.

### Reproduction (rust-stdf 1.1.0)

```rust
use rust_stdf::{ByteOrder, RawDataElement, RecordHeader, StdfRecord};

fn main() {
    // UPD_CNT=2, UPD_NAM=["V4-2007", "Scan:2007.1"]
    let payload = b"\x02\x07V4-2007\x0bScan:2007.1".to_vec();
    let raw = RawDataElement {
        offset: 0,
        header: RecordHeader { len: payload.len() as u16, typ: 0, sub: 30 },
        raw_data: payload,
        byte_order: ByteOrder::LittleEndian,
    };
    if let StdfRecord::VUR(vur) = StdfRecord::from(&raw) {
        println!("{:?}", vur.upd_nam);
    }
}
```

Expected: `["V4-2007", "Scan:2007.1"]`. Actual: `"\u{7}V"`.

This record comes from a production V4-2007 file. Spry Software's Java STDF reader models VUR the
same way (`getUpd_cnt()`, `String[] getUpd_nam()`;
[Javadoc](https://sprysoftware.net/javadoc/javadoc/spry/reader/stdf/VUR.html)).

### Suggested fix

Both layouts occur, and the record length tells them apart:

- empty payload: no names
- `1 + payload[0] == len`: one `C*n` (the specification's layout)
- otherwise: `UPD_CNT` followed by that many `C*n`, which should fill the record

Both readings fit only when every name is empty, and then they encode the same bytes. Keeping the
layout lets `StdfWriter` reproduce the original record:

```rust
pub struct VUR {
    pub upd_cnt: Option<U1>, // None: single-name layout
    pub upd_nam: KxCn,
}
```

This changes a public struct and the serde output. `KxCnRef` / `read_kx_cn` already decode the
names.
