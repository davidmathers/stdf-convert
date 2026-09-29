"""Read STDF V4 and V4-2007 semiconductor test data files as Arrow tables, one per record type."""

from __future__ import annotations

import os
from typing import Dict, Iterable, Iterator, Optional, Tuple, Union

import pyarrow

from . import _stdf_convert
from ._stdf_convert import DEFAULT_BATCH_SIZE, RECORD_TYPES, StdfError, __version__

__all__ = ["tables", "batches", "schema", "StdfError", "RECORD_TYPES", "DEFAULT_BATCH_SIZE", "__version__"]

PathLike = Union[str, "os.PathLike[str]"]


def _types(record_types: Optional[Iterable[str]]):
    return None if record_types is None else list(record_types)


def tables(path: PathLike, record_types: Optional[Iterable[str]] = None) -> Dict[str, pyarrow.Table]:
    """Read an STDF file into one ``pyarrow.Table`` per record type present.

    Pass ``record_types`` (e.g. ``["PIR", "PTR", "PRR"]``, case-insensitive) to skip other
    records while parsing. The tables have the schemas given by :func:`schema`, and their rows
    are in file order. Gzip (.gz), bzip2 (.bz2) and zip (.zip) files are read directly. The
    file is parsed without holding the GIL.
    """
    native = _stdf_convert.tables(os.fspath(path), _types(record_types))
    return {record_type: pyarrow.table(table) for record_type, table in native.items()}


def batches(
    path: PathLike,
    record_types: Optional[Iterable[str]] = None,
    *,
    batch_size: int = DEFAULT_BATCH_SIZE,
) -> Iterator[Tuple[str, pyarrow.RecordBatch]]:
    """Stream the records in an STDF file as ``(record_type, pyarrow.RecordBatch)`` pairs.

    A record type's batch is produced each time ``batch_size`` of its records have been read,
    and the rest of every type at the end of the file. Batches of one type come in file order;
    ``sequence_number`` gives the order across types. The file is parsed on a background
    thread, without holding the GIL.
    """
    native = _stdf_convert.batches(os.fspath(path), _types(record_types), batch_size=batch_size)
    return ((record_type, pyarrow.record_batch(batch)) for record_type, batch in native)


def schema(record_type: str) -> pyarrow.Schema:
    """The Arrow schema of a record type's table (case-insensitive, e.g. ``"PTR"``).

    Every table starts with ``sequence_number``, ``byte_offset``, ``rec_len``, ``rec_typ``
    and ``rec_sub``, followed by the record's fields in specification order. Column types come
    from the STDF type each field is declared with, which is kept in the field metadata as
    ``stdf_type``. The schema is the same for every file, so it can be used to create tables
    before loading any data.
    """
    return pyarrow.schema(_stdf_convert.schema(record_type))
