# stdf-convert

Read STDF V4 and V4-2007 semiconductor test data files, and convert them to JSON Lines.
Parsing is done in Rust by [`rust-stdf`](https://github.com/noonchen/rust-stdf).

```python
import stdf_convert

for record in stdf_convert.records("results.stdf.gz", ["PIR", "PTR", "PRR"]):
    print(record["record_type"], record["data"])

stdf_convert.convert("results.stdf.gz")   # writes results.jsonl
```

Installing the package also installs the `stdf-convert` command (`stdf-convert --help`), or
run it without installing: `uvx stdf-convert results.stdf`.

Each record is a dictionary:

```python
{
    "sequence_number": 42,     # position in the file (unaffected by filtering)
    "byte_offset": 1234,       # offset of the record header in the decompressed stream
    "rec_typ": 15,
    "rec_sub": 10,
    "record_type": "PTR",
    "data": {"TEST_NUM": 1001, "HEAD_NUM": 1, "SITE_NUM": 0, "TEST_FLG": b"\x00", "RESULT": 0.018, ...},
}
```

Numbers are `int` or `float` (including NaN and infinities), byte and bit fields are
`bytes`, and absent optional fields are `None`. Gzip (`.gz`), bzip2 (`.bz2`) and zip
(`.zip`, first member) files are read directly. Errors raise `stdf_convert.StdfError`;
an unknown record type in a filter raises `ValueError`.

In the JSON Lines output, byte and bit fields are lowercase hex strings (`"TEST_FLG":"80"`)
and NaN and infinities are the strings `"NaN"`, `"Infinity"` and `"-Infinity"`, so every
JSON parser can read the file.

## Development

```bash
cd python
uv venv && uv pip install maturin pytest
uv run maturin develop --release
uv run pytest
```
