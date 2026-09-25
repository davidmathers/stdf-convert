//! Python bindings: `stdf_convert.records`, `stdf_convert.Reader`, `stdf_convert.convert` and
//! the `stdf-convert` command.

use std::ffi::OsString;
use std::path::PathBuf;

use pyo3::exceptions::{PyException, PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pythonize::pythonize;
use serde::Serialize;
use stdf_convert::json::RecordData;
use stdf_convert::{BYTE_FIELDS, Record, RecordReader, convert_file};

pyo3::create_exception!(stdf_convert, StdfError, PyException, "An STDF file could not be read.");

fn py_err(e: stdf_convert::Error) -> PyErr {
    match e {
        stdf_convert::Error::UnknownRecordType(_) => PyValueError::new_err(e.to_string()),
        stdf_convert::Error::Io(e) => e.into(),
        e => StdfError::new_err(e.to_string()),
    }
}

/// Like [`py_err`], but file errors become Python's own `OSError(errno, strerror, filename)`,
/// e.g. `FileNotFoundError`.
fn py_err_with_files(py: Python<'_>, e: stdf_convert::Error) -> PyErr {
    let stdf_convert::Error::File { path, source } = e else { return py_err(e) };
    let Some(errno) = source.raw_os_error() else { return source.into() };
    let text = source.to_string();
    let strerror = text.strip_suffix(&format!(" (os error {errno})")).unwrap_or(&text).to_string();
    match py.get_type::<PyOSError>().call1((errno, strerror, path.to_string_lossy().into_owned())) {
        Ok(err) => PyErr::from_value(err),
        Err(e) => e,
    }
}

fn normalize_byte_fields(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Ok(dict) = value.cast::<PyDict>() {
        for (key, item) in dict.iter() {
            let is_bytes = key.extract::<String>().is_ok_and(|k| BYTE_FIELDS.contains(&k.as_str()));
            if is_bytes && !item.is_none() {
                let bytes = item.extract::<Vec<u8>>()?;
                dict.set_item(key, PyBytes::new(py, &bytes))?;
            } else {
                normalize_byte_fields(py, &item)?;
            }
        }
    } else if let Ok(list) = value.cast::<PyList>() {
        for item in list.iter() {
            normalize_byte_fields(py, &item)?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Envelope<'a> {
    sequence_number: u64,
    byte_offset: u64,
    rec_typ: u8,
    rec_sub: u8,
    record_type: &'static str,
    data: RecordData<'a>,
}

fn to_python(py: Python<'_>, r: &Record) -> PyResult<Py<PyAny>> {
    let envelope = Envelope {
        sequence_number: r.sequence_number,
        byte_offset: r.byte_offset,
        rec_typ: r.rec_typ,
        rec_sub: r.rec_sub,
        record_type: r.record_type,
        data: RecordData(&r.data),
    };
    let value = pythonize(py, &envelope)
        .map_err(|e| StdfError::new_err(format!("could not convert {}: {e}", r.record_type)))?;
    normalize_byte_fields(py, &value)?;
    Ok(value.unbind())
}

fn open(path: &PathBuf, record_types: Option<&[String]>) -> PyResult<RecordReader> {
    let filter: Option<Vec<&str>> = record_types.map(|t| t.iter().map(String::as_str).collect());
    RecordReader::open(path, filter.as_deref()).map_err(py_err)
}

/// A streaming iterator over the records in an STDF file.
#[pyclass(module = "stdf_convert", unsendable)]
struct Reader {
    inner: RecordReader,
    #[pyo3(get)]
    path: PathBuf,
}

#[pymethods]
impl Reader {
    #[new]
    #[pyo3(signature = (path, record_types = None))]
    fn new(path: PathBuf, record_types: Option<Vec<String>>) -> PyResult<Self> {
        Ok(Reader { inner: open(&path, record_types.as_deref())?, path })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match self.inner.next() {
            None => Ok(None),
            Some(r) => to_python(py, &r.map_err(py_err)?).map(Some),
        }
    }

    fn __repr__(&self) -> String {
        format!("Reader(path={:?})", self.path)
    }
}

/// Return a streaming iterator over the records in `path`.
#[pyfunction]
#[pyo3(signature = (path, record_types = None))]
fn records(path: PathBuf, record_types: Option<Vec<String>>) -> PyResult<Reader> {
    Reader::new(path, record_types)
}

/// Convert an STDF file to JSON Lines and return the output path.
#[pyfunction]
#[pyo3(signature = (path, output = None, *, record_types = None))]
fn convert(
    py: Python<'_>,
    path: PathBuf,
    output: Option<PathBuf>,
    record_types: Option<Vec<String>>,
) -> PyResult<PathBuf> {
    let output = output.unwrap_or_else(|| stdf_convert::output_path(&path));
    let filter: Option<Vec<&str>> = record_types.as_ref().map(|t| t.iter().map(String::as_str).collect());
    py.detach(|| convert_file(&path, &output, filter.as_deref())).map_err(|e| py_err_with_files(py, e))?;
    Ok(output)
}

/// Run the `stdf-convert` command with `argv` and return its exit code.
#[pyfunction]
fn run_cli(py: Python<'_>, argv: Vec<OsString>) -> i32 {
    py.detach(|| stdf_convert::run(argv))
}

#[pymodule]
fn _stdf_convert(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Reader>()?;
    m.add_function(wrap_pyfunction!(records, m)?)?;
    m.add_function(wrap_pyfunction!(convert, m)?)?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add("StdfError", m.py().get_type::<StdfError>())?;
    m.add("RECORD_TYPES", stdf_convert::RECORD_TYPES)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
