from __future__ import annotations

import bz2
import gzip
import math
import struct
import zipfile
from pathlib import Path

import pytest
import stdf_convert


# Little-endian FAR (CPU_TYPE=2, STDF_VER=4) followed by PIR (head=1, site=2).
STDF_BYTES = bytes([2, 0, 0, 10, 2, 4, 2, 0, 5, 10, 1, 2])


def envelope(
    sequence_number: int,
    byte_offset: int,
    rec_typ: int,
    rec_sub: int,
    record_type: str,
    data: dict,
) -> dict:
    return {
        "sequence_number": sequence_number,
        "byte_offset": byte_offset,
        "rec_typ": rec_typ,
        "rec_sub": rec_sub,
        "record_type": record_type,
        "data": data,
    }


def write_stdf(path: Path) -> Path:
    path.write_bytes(STDF_BYTES)
    return path


def test_records_are_streamed_as_native_dicts(tmp_path: Path) -> None:
    path = write_stdf(tmp_path / "sample.stdf")

    records = list(stdf_convert.records(path))

    assert records == [
        envelope(0, 0, 0, 10, "FAR", {"CPU_TYPE": 2, "STDF_VER": 4}),
        envelope(1, 6, 5, 10, "PIR", {"HEAD_NUM": 1, "SITE_NUM": 2}),
    ]


def test_record_filter_is_case_insensitive(tmp_path: Path) -> None:
    path = write_stdf(tmp_path / "sample.stdf")

    assert list(stdf_convert.records(path, ["pir"])) == [
        envelope(1, 6, 5, 10, "PIR", {"HEAD_NUM": 1, "SITE_NUM": 2})
    ]


def test_gzip_input(tmp_path: Path) -> None:
    path = tmp_path / "sample.stdf.gz"
    with gzip.open(path, "wb") as output:
        output.write(STDF_BYTES)

    assert [record["record_type"] for record in stdf_convert.records(path)] == ["FAR", "PIR"]


def test_bzip2_input(tmp_path: Path) -> None:
    path = tmp_path / "sample.stdf.bz2"
    path.write_bytes(bz2.compress(STDF_BYTES))

    assert [record["record_type"] for record in stdf_convert.records(path)] == ["FAR", "PIR"]


def test_zip_input(tmp_path: Path) -> None:
    path = tmp_path / "sample.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("sample.stdf", STDF_BYTES)

    assert [record["record_type"] for record in stdf_convert.records(path)] == ["FAR", "PIR"]


def test_unknown_record_filter_is_rejected(tmp_path: Path) -> None:
    path = write_stdf(tmp_path / "sample.stdf")

    with pytest.raises(ValueError, match="unknown STDF record type"):
        stdf_convert.Reader(path, ["not-a-record"])


def test_invalid_records_can_be_filtered(tmp_path: Path) -> None:
    path = tmp_path / "invalid-record.stdf"
    path.write_bytes(STDF_BYTES[:6] + bytes([0, 0, 99, 99]))

    assert list(stdf_convert.records(path, ["invalid"])) == [
        envelope(1, 6, 99, 99, "INVALID", {"LEN": 0, "TYP": 99, "SUB": 99})
    ]


def test_invalid_file_raises_stdf_error(tmp_path: Path) -> None:
    path = tmp_path / "invalid.stdf"
    path.write_bytes(b"not an STDF file")

    with pytest.raises(stdf_convert.StdfError):
        stdf_convert.Reader(path)


def test_reserved_record_retains_original_payload_bytes(tmp_path: Path) -> None:
    payload = b"\x00\x80\xff"
    path = tmp_path / "reserved.stdf"
    path.write_bytes(STDF_BYTES[:6] + bytes([len(payload), 0, 180, 7]) + payload)

    assert list(stdf_convert.records(path, ["reserved"])) == [
        envelope(1, 6, 180, 7, "RESERVED", {"RAW_DATA": payload})
    ]


def test_big_endian_input(tmp_path: Path) -> None:
    path = tmp_path / "big-endian.stdf"
    path.write_bytes(bytes([0, 2, 0, 10, 1, 4, 0, 2, 5, 10, 1, 2]))

    assert list(stdf_convert.records(path)) == [
        envelope(0, 0, 0, 10, "FAR", {"CPU_TYPE": 1, "STDF_VER": 4}),
        envelope(1, 6, 5, 10, "PIR", {"HEAD_NUM": 1, "SITE_NUM": 2}),
    ]


def test_v4_2007_record(tmp_path: Path) -> None:
    upgrade_name = b"V4-2007"
    payload = bytes([len(upgrade_name)]) + upgrade_name
    path = tmp_path / "v4-2007.stdf"
    path.write_bytes(STDF_BYTES[:6] + bytes([len(payload), 0, 0, 30]) + payload)

    assert list(stdf_convert.records(path, ["vur"])) == [
        envelope(1, 6, 0, 30, "VUR", {"UPD_NAM": "V4-2007"})
    ]


def test_truncated_record_raises_stdf_error(tmp_path: Path) -> None:
    path = tmp_path / "truncated.stdf"
    path.write_bytes(STDF_BYTES[:6] + bytes([2, 0, 5, 10, 1]))

    with pytest.raises(stdf_convert.StdfError, match="failed to fill whole buffer"):
        list(stdf_convert.records(path))


@pytest.mark.parametrize("result", [math.nan, math.inf, -math.inf])
def test_non_finite_floats_bit_fields_and_missing_values(
    tmp_path: Path, result: float
) -> None:
    payload = struct.pack("<IBBBBfBB", 1, 1, 2, 0x81, 0x42, result, 0, 0)
    path = tmp_path / "ptr.stdf"
    path.write_bytes(
        STDF_BYTES[:6]
        + struct.pack("<HBB", len(payload), 15, 10)
        + payload
    )

    record = list(stdf_convert.records(path, ["ptr"]))[0]
    parsed = record["data"]
    assert math.isnan(parsed["RESULT"]) if math.isnan(result) else parsed["RESULT"] == result
    assert parsed["TEST_FLG"] == b"\x81"
    assert parsed["PARM_FLG"] == b"\x42"
    assert parsed["OPT_FLAG"] is None
    assert parsed["LO_LIMIT"] is None


def test_nested_gdr_byte_arrays_and_bit_fields_use_bytes(tmp_path: Path) -> None:
    payload = struct.pack("<HBB3sBH", 2, 11, 3, b"\x00\x80\xff", 12, 8) + b"\xa5"
    path = tmp_path / "gdr.stdf"
    path.write_bytes(
        STDF_BYTES[:6]
        + struct.pack("<HBB", len(payload), 50, 10)
        + payload
    )

    parsed = list(stdf_convert.records(path, ["gdr"]))[0]["data"]
    assert parsed["GEN_DATA"] == [{"Bn": b"\x00\x80\xff"}, {"Dn": b"\xa5"}]
