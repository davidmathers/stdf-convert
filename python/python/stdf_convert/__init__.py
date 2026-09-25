"""Read STDF V4 and V4-2007 semiconductor test data files, and convert them to JSON Lines."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Iterable, Optional, Union

from . import _stdf_convert
from ._stdf_convert import RECORD_TYPES, Reader, StdfError, __version__

__all__ = ["records", "Reader", "convert", "StdfError", "RECORD_TYPES", "__version__"]

PathLike = Union[str, "os.PathLike[str]"]


def records(path: PathLike, record_types: Optional[Iterable[str]] = None) -> Reader:
    """Stream the records in an STDF file as dictionaries.

    Each record is ``{"sequence_number", "byte_offset", "rec_typ", "rec_sub",
    "record_type", "data"}``, with the record's fields in ``data``. Pass ``record_types``
    (e.g. ``["PIR", "PTR", "PRR"]``, case-insensitive) to skip other records while parsing.
    Gzip (.gz), bzip2 (.bz2) and zip (.zip) files are read directly.
    """
    return _stdf_convert.records(os.fspath(path), None if record_types is None else list(record_types))


def convert(
    path: PathLike,
    output: Optional[PathLike] = None,
    *,
    record_types: Optional[Iterable[str]] = None,
) -> Path:
    """Convert an STDF file to JSON Lines and return the output path.

    The output defaults to ``path`` with its STDF and compression extensions replaced by
    ``.jsonl`` and is replaced if it exists. Byte and bit fields are hex strings, and NaN
    and infinities are the strings ``"NaN"``, ``"Infinity"`` and ``"-Infinity"``.
    """
    out = _stdf_convert.convert(
        os.fspath(path),
        None if output is None else os.fspath(output),
        record_types=None if record_types is None else list(record_types),
    )
    return Path(out)

