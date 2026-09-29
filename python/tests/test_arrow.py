from __future__ import annotations

import bz2
import gzip
import json
import math
import struct
import subprocess
import sys
import zipfile
from pathlib import Path

import pyarrow as pa
import pytest
import stdf_convert

HEADER_COLUMNS = ["sequence_number", "byte_offset", "rec_len", "rec_typ", "rec_sub"]


def record(typ: int, sub: int, payload: bytes) -> bytes:
    return struct.pack("<HBB", len(payload), typ, sub) + payload


def cn(text: str) -> bytes:
    raw = text.encode("latin-1")
    return bytes([len(raw)]) + raw


def ptr(site: int, result: float, *, limits: bool) -> bytes:
    payload = struct.pack("<IBBBBf", 1001, 1, site, 0x00, 0xC0, result) + cn("Vdd") + cn("")
    if limits:
        payload += struct.pack("<Bbbbff", 0x0E, 0, 0, 0, 0.018, 1.25) + cn("V")
    return record(15, 10, payload)


def mpr(site: int) -> bytes:
    # RTN_ICNT=3, RSLT_CNT=2, RTN_STAT nibbles [1, 2, 3], RTN_RSLT [0.5, NaN]; the rest omitted.
    payload = struct.pack("<IBBBBHH", 2002, 1, site, 0, 0, 3, 2) + bytes([0x21, 0x03])
    payload += struct.pack("<ff", 0.5, math.nan)
    return record(15, 15, payload)


def gdr() -> bytes:
    # B0 pad, U1 5, Cn "x", Bn [00 80]
    payload = struct.pack("<H", 4) + bytes([0, 1, 5, 10]) + cn("x") + bytes([11, 2, 0x00, 0x80])
    return record(50, 10, payload)


def ftr() -> bytes:
    # RTN_INDX [5, 6], RTN_STAT nibbles [1, 2], FAIL_PIN 5 bits [0x15]; the rest omitted.
    payload = struct.pack("<IBBBBIIIIiihHH", 3003, 1, 1, 0, 0, 10, 0, 1, 1, 0, 0, 0, 2, 0)
    payload += struct.pack("<HH", 5, 6) + bytes([0x21]) + struct.pack("<H", 5) + bytes([0x15])
    return record(15, 20, payload)


def sample_file(path: Path) -> Path:
    """Two sites whose results interleave, with records cut short, NaN, arrays, bit fields,
    GDR values, UTF-8 and Latin-1 text, and reserved and unknown records."""
    path.write_bytes(
        b"".join(
            [
                record(0, 10, bytes([2, 4])),  # FAR
                record(5, 10, bytes([1, 1])),  # PIR site 1
                record(5, 10, bytes([1, 2])),  # PIR site 2
                ptr(1, 0.017999999225139618, limits=True),
                ptr(2, math.nan, limits=False),
                mpr(2),
                record(5, 20, struct.pack("<BBBHH", 1, 1, 0, 1, 1)),  # PRR site 1 (cut short)
                gdr(),
                ftr(),
                record(50, 30, cn("hello")),  # DTR
                record(50, 30, bytes([2, 0xC3, 0xA9])),  # DTR, UTF-8 é
                record(50, 30, bytes([1, 0xE9])),  # DTR, Latin-1 é
                record(180, 7, b"\x00\x80\xff"),  # reserved
                record(99, 99, b"\x01\x02"),  # unknown
            ]
        )
    )
    return path


def f32(x: float) -> float:
    return struct.unpack("<f", struct.pack("<f", x))[0]


def from_json(t: pa.DataType, v):
    """A value from the command's JSON output, in the form `pyarrow.Table.to_pylist` gives it."""
    if v is None:
        return None
    if pa.types.is_binary(t):
        return bytes.fromhex(v)
    if pa.types.is_floating(t):
        v = {"NaN": math.nan, "Infinity": math.inf, "-Infinity": -math.inf}.get(v, v)
        return f32(v) if t == pa.float32() else float(v)
    if pa.types.is_list(t):
        if isinstance(v, dict):  # KxUf: {"F2": [...]}
            (v,) = v.values()
        item = t.value_type
        if pa.types.is_struct(item) and "TYPE" in item.names:  # GDR values: "B0" or {"U1": 5}
            values = []
            for x in v:
                name, value = (x, None) if isinstance(x, str) else next(iter(x.items()))
                row = {f.name: None for f in item}
                row["TYPE"] = name
                if name in item.names:
                    row[name] = from_json(item.field(name).type, value)
                values.append(row)
            return values
        return [from_json(item, x) for x in v]
    if pa.types.is_struct(t):
        return {f.name: from_json(f.type, v[f.name]) for f in t}
    return v


def same(a, b) -> bool:
    if isinstance(a, float) and isinstance(b, float):
        return a == b or (math.isnan(a) and math.isnan(b))
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    return type(a) is type(b) and a == b


def json_lines(path: Path, tmp_path: Path) -> list:
    subprocess.run(
        [sys.executable, "-m", "stdf_convert", "-q", "-o", str(tmp_path / "json"), str(path)], check=True
    )
    return [json.loads(line) for line in (tmp_path / "json" / f"{path.stem}.jsonl").read_text().splitlines()]


def test_tables_hold_exactly_what_the_json_output_holds(tmp_path: Path) -> None:
    path = sample_file(tmp_path / "sample.stdf")
    records = json_lines(path, tmp_path)
    tables = stdf_convert.tables(path)

    assert list(tables) == [t for t in stdf_convert.RECORD_TYPES if t in {r["record_type"] for r in records}]
    rows = {}
    for record_type, table in tables.items():
        assert table.schema == stdf_convert.schema(record_type)
        for row in table.to_pylist():
            rows[row["sequence_number"]] = (record_type, table.schema, row)

    assert sorted(rows) == [r["sequence_number"] for r in records]
    for i, r in enumerate(records):
        record_type, schema, row = rows[r["sequence_number"]]
        assert record_type == r["record_type"]
        for key in ["byte_offset", "rec_typ", "rec_sub"]:
            assert row[key] == r[key]
        end = records[i + 1]["byte_offset"] if i + 1 < len(records) else path.stat().st_size
        assert row["rec_len"] == end - r["byte_offset"] - 4
        fields = [f for f in schema if f.name not in HEADER_COLUMNS]
        assert [f.name for f in fields] == list(r["data"]), record_type
        for f in fields:
            want = from_json(f.type, r["data"][f.name])
            assert same(row[f.name], want), (record_type, f.name, row[f.name], want)


def test_text_is_utf8_when_valid_otherwise_latin1(tmp_path: Path) -> None:
    dtr = stdf_convert.tables(sample_file(tmp_path / "sample.stdf"), ["DTR"])["DTR"]
    assert dtr.column("TEXT_DAT").to_pylist() == ["hello", "é", "é"]


def test_types_nan_and_nulls(tmp_path: Path) -> None:
    ptr_table = stdf_convert.tables(sample_file(tmp_path / "sample.stdf"), ["PTR"])["PTR"]

    assert ptr_table.schema.field("TEST_NUM").type == pa.uint32()
    assert ptr_table.schema.field("RESULT").type == pa.float32()
    assert ptr_table.schema.field("TEST_FLG").type == pa.binary()
    assert ptr_table.schema.field("LO_LIMIT").nullable
    assert ptr_table.schema.field("LO_LIMIT").metadata == {b"stdf_type": b"R4"}

    result = ptr_table.column("RESULT")
    assert result.null_count == 0
    assert math.isnan(result[1].as_py())
    assert result[0].as_py() == 0.017999999225139618
    assert ptr_table.column("LO_LIMIT").to_pylist()[1] is None
    assert ptr_table.column("SITE_NUM").to_pylist() == [1, 2]


def test_schema_is_the_same_whatever_the_file_holds(tmp_path: Path) -> None:
    full = stdf_convert.tables(sample_file(tmp_path / "full.stdf"), ["PTR"])["PTR"]
    (tmp_path / "bare.stdf").write_bytes(record(0, 10, bytes([2, 4])) + ptr(1, 1.0, limits=False))
    bare = stdf_convert.tables(tmp_path / "bare.stdf", ["PTR"])["PTR"]

    assert bare.column("LO_LIMIT").null_count == 1
    assert bare.schema == full.schema == stdf_convert.schema("ptr")
    assert pa.concat_tables([full, bare]).num_rows == 3


def test_schema_metadata() -> None:
    schema = stdf_convert.schema("MPR")
    assert schema.names[:5] == HEADER_COLUMNS
    assert schema.metadata[b"record_type"] == b"MPR"
    assert schema.metadata[b"stdf_convert_version"] == stdf_convert.__version__.encode()
    assert schema.field("RTN_STAT").type == pa.list_(pa.field("item", pa.uint8(), nullable=False))
    assert schema.field("RTN_STAT").metadata == {b"stdf_type": b"KxN1"}
    with pytest.raises(ValueError):
        stdf_convert.schema("nope")


def test_batches_stream_in_file_order(tmp_path: Path) -> None:
    path = sample_file(tmp_path / "sample.stdf")

    batches = list(stdf_convert.batches(path, batch_size=1))

    assert all(isinstance(b, pa.RecordBatch) and b.num_rows == 1 for _, b in batches)
    total = sum(t.num_rows for t in stdf_convert.tables(path).values())
    assert [b.column("sequence_number")[0].as_py() for _, b in batches] == list(range(total))
    ptr_rows = [b for t, b in batches if t == "PTR"]
    assert same(pa.Table.from_batches(ptr_rows).to_pylist(), stdf_convert.tables(path)["PTR"].to_pylist())


def test_batches_group_rows_by_record_type(tmp_path: Path) -> None:
    batches = list(stdf_convert.batches(sample_file(tmp_path / "sample.stdf"), ["pir", "ptr", "prr"]))
    assert [(t, b.num_rows) for t, b in batches] == [("PIR", 2), ("PRR", 1), ("PTR", 2)]


def test_abandoned_batches_do_not_hang(tmp_path: Path) -> None:
    it = stdf_convert.batches(sample_file(tmp_path / "sample.stdf"), batch_size=1)
    next(it)
    del it


def test_errors(tmp_path: Path) -> None:
    path = tmp_path / "truncated.stdf"
    path.write_bytes(record(0, 10, bytes([2, 4])) + ptr(1, 1.0, limits=True)[:-3])
    with pytest.raises(stdf_convert.StdfError):
        stdf_convert.tables(path)
    with pytest.raises(stdf_convert.StdfError):
        list(stdf_convert.batches(path))
    with pytest.raises(ValueError):
        stdf_convert.batches(path, ["nope"])
    with pytest.raises(ValueError):
        stdf_convert.batches(path, batch_size=0)
    with pytest.raises(stdf_convert.StdfError):
        stdf_convert.tables(tmp_path / "missing.stdf")


def test_duckdb_raw_tables(tmp_path: Path) -> None:
    duckdb = pytest.importorskip("duckdb")
    path = sample_file(tmp_path / "sample.stdf")
    con = duckdb.connect()
    for record_type in ["PIR", "PTR", "PRR"]:
        empty = stdf_convert.schema(record_type).empty_table()
        con.execute(f"CREATE TABLE {record_type.lower()} AS SELECT * FROM empty")
    for record_type, arrow_table in stdf_convert.tables(path, ["PIR", "PTR", "PRR"]).items():
        con.execute(f"INSERT INTO {record_type.lower()} SELECT * FROM arrow_table")

    types = dict(con.execute("SELECT column_name, data_type FROM duckdb_columns() WHERE table_name = 'ptr'").fetchall())
    assert types["TEST_NUM"] == "UINTEGER"
    assert types["RESULT"] == "FLOAT"
    assert types["TEST_FLG"] == "BLOB"
    assert types["sequence_number"] == "UBIGINT"
    assert con.execute(
        "SELECT count(*) FILTER (isnan(RESULT)), count(*) FILTER (RESULT IS NULL), "
        "count(*) FILTER (LO_LIMIT IS NULL), any_value(LO_LIMIT) FILTER (LO_LIMIT IS NOT NULL) = 0.018::FLOAT "
        "FROM ptr"
    ).fetchone() == (1, 0, 1, True)
    # Results belong to the latest PIR of their head and site.
    assert con.execute(
        "SELECT t.SITE_NUM, p.sequence_number FROM ptr t ASOF JOIN pir p "
        "ON t.HEAD_NUM = p.HEAD_NUM AND t.SITE_NUM = p.SITE_NUM AND t.sequence_number > p.sequence_number "
        "ORDER BY t.sequence_number"
    ).fetchall() == [(1, 1), (2, 2)]


STDF_BYTES = record(0, 10, bytes([2, 4])) + record(5, 10, bytes([1, 2]))


@pytest.mark.parametrize("suffix", ["stdf.gz", "stdf.bz2", "zip"])
def test_compressed_input(tmp_path: Path, suffix: str) -> None:
    path = tmp_path / f"sample.{suffix}"
    if suffix == "stdf.gz":
        with gzip.open(path, "wb") as out:
            out.write(STDF_BYTES)
    elif suffix == "stdf.bz2":
        path.write_bytes(bz2.compress(STDF_BYTES))
    else:
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("sample.stdf", STDF_BYTES)
    assert list(stdf_convert.tables(path)) == ["FAR", "PIR"]


def test_big_endian_input(tmp_path: Path) -> None:
    path = tmp_path / "be.stdf"
    path.write_bytes(bytes([0, 2, 0, 10, 1, 4, 0, 2, 5, 10, 1, 2]))
    far = stdf_convert.tables(path)["FAR"]
    assert far.column("CPU_TYPE").to_pylist() == [1]


def test_record_filter_is_case_insensitive_and_keeps_positions(tmp_path: Path) -> None:
    path = tmp_path / "sample.stdf"
    path.write_bytes(STDF_BYTES)
    tables = stdf_convert.tables(path, ["pir"])
    assert list(tables) == ["PIR"]
    assert tables["PIR"].column("sequence_number").to_pylist() == [1]
    assert tables["PIR"].column("byte_offset").to_pylist() == [6]


def test_not_stdf_raises_stdf_error(tmp_path: Path) -> None:
    path = tmp_path / "not.stdf"
    path.write_bytes(b"not an STDF file")
    with pytest.raises(stdf_convert.StdfError):
        stdf_convert.tables(path)
