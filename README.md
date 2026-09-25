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
- Byte and bit fields (flags, `PART_FIX`, pin maps, ...) are lowercase hex strings.
- Floats have the fewest digits that read back as the same value. NaN and infinities,
  which JSON numbers cannot represent, are the strings `"NaN"`, `"Infinity"` and
  `"-Infinity"`.
- Absent optional fields are `null`. Records outside the specification are kept:
  `RESERVED` with its payload as `RAW_DATA`, `INVALID` with its header.

## Python

```bash
pip install stdf-convert
```

```python
import stdf_convert

for record in stdf_convert.records("results.stdf.gz", ["PIR", "PTR", "PRR"]):
    print(record["record_type"], record["data"])

stdf_convert.convert("results.stdf.gz")
```

Records are dictionaries in the same shape as the JSON, with byte fields as `bytes` and
NaN as `float("nan")`. See [`python/README.md`](python/README.md).

To build just the command from source, with a recent stable Rust (tested with 1.98):

```bash
cargo install --path crates/stdf-convert
```

## Layout

| Path | |
|---|---|
| `crates/stdf-convert` | The `stdf-convert` command and the library behind it (record reader, JSON Lines writer) |
| `python` | The Python package (PyO3, built with maturin) |

## Development

```bash
cargo test
(cd python && uv run maturin develop && uv run pytest)
```

## License

MIT; see [`LICENSE`](LICENSE). `rust-stdf` is also MIT-licensed; see [`NOTICE`](NOTICE).
