# stdf-convert

Convert STDF V4 and V4-2007 semiconductor test data files to JSON Lines, from the command
line or Python. Parsing is done in Rust by [`rust-stdf`](https://github.com/noonchen/rust-stdf).

```bash
stdf-convert results.stdf.gz                 # writes results.jsonl
stdf-convert -o out/ lots/                   # every STDF file under lots/, recursively
stdf-convert --records PIR,PTR,PRR lot.stdf  # only these record types
uvx stdf-convert lot.stdf                    # run without installing
```

| Option | Default | |
|---|---|---|
| `-f, --format json` | `json` | Output format (JSON Lines) |
| `-o, --output-dir <DIR>` | next to each input | Write outputs under `DIR`, mirroring the input folders |
| `--records <TYPES>` | all | Only write these record types, comma-separated |
| `--overwrite` | off | Replace existing outputs (otherwise they are skipped) |
| `-q, --quiet` | off | Only print warnings and errors |

Inputs may be gzip (`.gz`), bzip2 (`.bz2`) or zip (`.zip`, first member) compressed.
Folders are searched for `*.stdf` and `*.std` files and their compressed forms. The exit
status is non-zero if any file fails; the other files are still converted.

## Output

One JSON object per line, per record, fields in STDF specification order:

```json
{"sequence_number":2,"byte_offset":14,"rec_typ":15,"rec_sub":10,"record_type":"PTR","data":{"TEST_NUM":1001,"HEAD_NUM":1,"SITE_NUM":0,"TEST_FLG":"00","PARM_FLG":"00","RESULT":0.018,"TEST_TXT":"Vdd","ALARM_ID":"","OPT_FLAG":null,...}}
```

- `sequence_number` is the record's position in the file and `byte_offset` the offset of
  its header in the decompressed stream; both are unaffected by `--records`.
- Byte fields (flags, `PART_FIX`, ...) are lowercase hex strings. Bit fields (pin maps,
  ...) are `{"bit_count":5,"bit_data":"15"}`.
- Text is UTF-8 when the file's bytes are valid UTF-8, otherwise Latin-1.
- Floats have the fewest digits that read back as the same value. NaN and infinities,
  which JSON numbers cannot represent, are the strings `"NaN"`, `"Infinity"` and
  `"-Infinity"`.
- Absent optional fields are `null`. Records outside the specification are kept:
  `RESERVED` (types 180 and 181) and `UNKNOWN` (unknown types), each with `TYP`, `SUB`,
  `BYTE_ORDER` and its payload as `RAW_DATA`.
- `VUR` is `{"UPD_CNT":2,"UPD_NAM":["V4-2007","Scan:2007.1"]}`, or with `UPD_CNT` null for
  the V4-2007 specification's single name. A VUR that is neither is an error.

## Python

```bash
pip install stdf-convert
```

```python
import stdf_convert

tables = stdf_convert.tables("results.stdf.gz")   # {"FAR": pyarrow.Table, "PIR": ..., "PTR": ...}
```

The Python package reads files as Arrow tables, one per record type, with a fixed schema
generated from rust-stdf's record definitions. See [`python/README.md`](python/README.md).

To build just the command from source, with a recent stable Rust (tested with 1.98):

```bash
cargo install --path crates/stdf-convert
```

## Layout

| Path | |
|---|---|
| `crates/stdf-convert` | The `stdf-convert` command and the library behind it (record reader, JSON Lines writer, Arrow reader) |
| `crates/codegen` | Generates `crates/stdf-convert/src/arrow/generated.rs` from rust-stdf's record definitions: `cargo run -p stdf-convert-codegen` after changing the rust-stdf version (a test fails if it is out of date) |
| `python` | The Python package (PyO3, built with maturin) |

## Development

```bash
cargo test
(cd python && uv run maturin develop && uv run pytest)
```

## License

MIT; see [`LICENSE`](LICENSE). `rust-stdf` is also MIT-licensed; see [`NOTICE`](NOTICE).
