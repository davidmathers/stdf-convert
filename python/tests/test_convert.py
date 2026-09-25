import json
import subprocess
import sys
from pathlib import Path

import pytest

import stdf_convert

# Little-endian FAR (CPU_TYPE=2, STDF_VER=4) followed by PIR (head=1, site=2).
STDF_BYTES = bytes([2, 0, 0, 10, 2, 4, 2, 0, 5, 10, 1, 2])


def test_convert_writes_json_lines(tmp_path: Path) -> None:
    src = tmp_path / "lot.stdf"
    src.write_bytes(STDF_BYTES)
    out = stdf_convert.convert(src)
    assert out == tmp_path / "lot.jsonl"
    lines = [json.loads(line) for line in out.read_text().splitlines()]
    assert lines == [
        {"sequence_number": 0, "byte_offset": 0, "rec_typ": 0, "rec_sub": 10, "record_type": "FAR",
         "data": {"CPU_TYPE": 2, "STDF_VER": 4}},
        {"sequence_number": 1, "byte_offset": 6, "rec_typ": 5, "rec_sub": 10, "record_type": "PIR",
         "data": {"HEAD_NUM": 1, "SITE_NUM": 2}},
    ]
    assert lines == list(stdf_convert.records(src))


def test_convert_filter_and_output(tmp_path: Path) -> None:
    src = tmp_path / "lot.stdf"
    src.write_bytes(STDF_BYTES)
    out = stdf_convert.convert(src, tmp_path / "pir.jsonl", record_types=["pir"])
    assert [json.loads(l)["record_type"] for l in out.read_text().splitlines()] == ["PIR"]
    with pytest.raises(ValueError, match="unknown STDF record type"):
        stdf_convert.convert(src, record_types=["nope"])


def test_command_line(tmp_path: Path) -> None:
    (tmp_path / "in").mkdir()
    (tmp_path / "in" / "a.stdf").write_bytes(STDF_BYTES)
    result = subprocess.run(
        [sys.executable, "-m", "stdf_convert", "-q", "-o", str(tmp_path / "out"), str(tmp_path / "in")],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    assert (tmp_path / "out" / "a.jsonl").exists()
    result = subprocess.run([sys.executable, "-m", "stdf_convert", "--version"], capture_output=True, text=True)
    assert result.stdout.strip() == f"stdf-convert {stdf_convert.__version__}"


def test_unwritable_output_names_the_folder(tmp_path: Path) -> None:
    src = tmp_path / "lot.stdf"
    src.write_bytes(STDF_BYTES)
    blocker = tmp_path / "file"
    blocker.write_text("")
    folder = blocker / "sub"  # can't be created: its parent is a file
    with pytest.raises(OSError) as e:
        stdf_convert.convert(src, folder / "lot.jsonl")
    assert e.value.filename == str(folder)
