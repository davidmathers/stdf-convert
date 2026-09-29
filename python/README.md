# stdf-convert

Read STDF V4 and V4-2007 semiconductor test data files as Arrow tables, one per record type.
Parsing is done in Rust by [`rust-stdf`](https://github.com/noonchen/rust-stdf).

```python
import stdf_convert

tables = stdf_convert.tables("results.stdf.gz")    # {"FAR": pyarrow.Table, "PIR": ..., "PTR": ...}
ptr = stdf_convert.tables("results.stdf.gz", ["PIR", "PTR", "PRR"])["PTR"]
for record_type, batch in stdf_convert.batches("results.stdf.gz", batch_size=65_536):
    ...                                            # for files too large to hold at once
stdf_convert.schema("PTR")                         # the schema, without reading a file
```

Both `tables` and `batches` parse the file in Rust without holding the GIL. Gzip (`.gz`),
bzip2 (`.bz2`) and zip (`.zip`, first member) files are read directly. Errors raise
`stdf_convert.StdfError`; an unknown record type in a filter raises `ValueError`.

Installing the package also installs the `stdf-convert` command, which converts files to
JSON Lines (`stdf-convert --help`), or run it without installing: `uvx stdf-convert results.stdf`.

## Tables

Every table starts with the record's position and header, then has one column per record
field, in specification order:

| Column | Type | |
|---|---|---|
| `sequence_number` | `uint64` | position of the record in the file (unaffected by filtering) |
| `byte_offset` | `uint64` | offset of the record header in the decompressed stream |
| `rec_len`, `rec_typ`, `rec_sub` | `uint16`, `uint8`, `uint8` | the record header |
| `TEST_NUM`, ... | see below | the record's fields |

The schema comes from the STDF type rust-stdf declares each field with (kept in the field
metadata as `stdf_type`), so it is the same for every file, whatever values the file holds:

| STDF | Arrow |
|---|---|
| `U1`, `U2`, `U4`, `U8` / `I1`, `I2`, `I4` / `R4` | `uint8` ... `uint64` / `int8` ... `int32` / `float32` |
| `C1`, `Cn`, `Sn` | `string`: UTF-8 when the bytes are valid UTF-8, otherwise Latin-1 |
| `B1`, `Bn` | `binary` |
| `Dn` (bit fields) | `struct<bit_count: uint16, bit_data: binary>` |
| `KxU1`, `KxN1`, `KxU2`, `KxR4`, `KxCn`, ... | `list` of the item type |
| `KxUf` (STR) | `list<uint64>`; the `*_SIZE` fields give the width in the file |
| `GDR.GEN_DATA` | `list<struct>`: `TYPE` (`U1`, `Cn`, `B0`, ...) and a column per type |

Optional fields are nullable and null only where the record left them out; NaN stays NaN.
Records outside the specification are kept: `RESERVED` (types 180 and 181) and `UNKNOWN`
(unknown types), each with `TYP`, `SUB`, `BYTE_ORDER` and its payload as `RAW_DATA`.

Record order across tables is given by `sequence_number`, e.g. to find each result's part:

```python
import duckdb

con = duckdb.connect()
for record_type, t in stdf_convert.tables("results.stdf.gz").items():
    con.execute(f"CREATE TABLE {record_type.lower()} AS SELECT * FROM t")
con.sql("""
    SELECT pir.sequence_number AS part_start, ptr.*
    FROM ptr ASOF JOIN pir
      ON ptr.HEAD_NUM = pir.HEAD_NUM AND ptr.SITE_NUM = pir.SITE_NUM
     AND ptr.sequence_number > pir.sequence_number
""")
```

## Development

```bash
cd python
uv venv && uv pip install maturin pytest duckdb
uv run maturin develop --release
uv run pytest
```
