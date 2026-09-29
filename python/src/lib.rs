//! Python bindings: the Arrow functions `stdf_convert.schema`, `batches` and `tables`, and the
//! `stdf-convert` command.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::JoinHandle;

use arrow_array::RecordBatch;

use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_arrow::{PyRecordBatch, PySchema, PyTable};
use stdf_convert::RecordReader;
use stdf_convert::arrow::{BatchReader, DEFAULT_BATCH_SIZE};

pyo3::create_exception!(stdf_convert, StdfError, PyException, "An STDF file could not be read.");

fn py_err(e: stdf_convert::Error) -> PyErr {
    match e {
        stdf_convert::Error::UnknownRecordType(_) => PyValueError::new_err(e.to_string()),
        stdf_convert::Error::Io(e) => e.into(),
        e => StdfError::new_err(e.to_string()),
    }
}

/// The Arrow schema of a record type's table.
#[pyfunction]
fn schema(record_type: &str) -> PyResult<PySchema> {
    stdf_convert::arrow::schema(record_type).map(PySchema::new).map_err(py_err)
}

type BatchResult = stdf_convert::Result<(&'static str, RecordBatch)>;

/// Arrow record batches `(record_type, batch)` from an STDF file, parsed on a background thread
/// while Python handles the previous batches.
#[pyclass(module = "stdf_convert", unsendable)]
struct Batches {
    batches: Option<Receiver<BatchResult>>,
    thread: Option<JoinHandle<()>>,
    #[pyo3(get)]
    path: PathBuf,
}

#[pymethods]
impl Batches {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<(&'static str, PyRecordBatch)>> {
        let Some(batches) = &mut self.batches else { return Ok(None) };
        match py.detach(move || batches.recv()) {
            Ok(Ok((record_type, batch))) => Ok(Some((record_type, PyRecordBatch::new(batch)))),
            Ok(Err(e)) => {
                self.batches = None;
                Err(py_err(e))
            }
            // The thread has finished: at the end of the file, or because it panicked.
            Err(_) => {
                self.batches = None;
                match self.thread.take().map(|t| py.detach(|| t.join())) {
                    Some(Err(_)) => Err(StdfError::new_err("reading the STDF file failed unexpectedly")),
                    _ => Ok(None),
                }
            }
        }
    }

    fn __repr__(&self) -> String {
        format!("Batches(path={:?})", self.path)
    }
}

/// Return an iterator of `(record_type, batch)` Arrow record batches from `path`.
#[pyfunction]
#[pyo3(signature = (path, record_types = None, *, batch_size = DEFAULT_BATCH_SIZE))]
fn batches(
    py: Python<'_>,
    path: PathBuf,
    record_types: Option<Vec<String>>,
    batch_size: usize,
) -> PyResult<Batches> {
    if batch_size == 0 {
        return Err(PyValueError::new_err("batch_size must be at least 1"));
    }
    // The reader can't move between threads, so it is opened on the thread that uses it, and
    // an error opening it is raised here.
    let (opened_send, opened) = sync_channel(1);
    // A small buffer: parsing runs at most two batches ahead of Python.
    let (send, receive) = sync_channel(2);
    let file = path.clone();
    let thread = std::thread::spawn(move || {
        let filter: Option<Vec<&str>> = record_types.as_ref().map(|t| t.iter().map(String::as_str).collect());
        let reader = match RecordReader::open(&file, filter.as_deref()) {
            Ok(records) => BatchReader::new(records, batch_size),
            Err(e) => {
                let _ = opened_send.send(Err(e));
                return;
            }
        };
        let _ = opened_send.send(Ok(()));
        for batch in reader {
            if send.send(batch).is_err() {
                break; // the iterator was dropped
            }
        }
    });
    match py.detach(move || opened.recv()) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(py_err(e)),
        Err(_) => return Err(StdfError::new_err("opening the STDF file failed unexpectedly")),
    }
    Ok(Batches { batches: Some(receive), thread: Some(thread), path })
}

/// Read `path` into one Arrow table per record type present.
#[pyfunction]
#[pyo3(signature = (path, record_types = None))]
fn tables<'py>(
    py: Python<'py>,
    path: PathBuf,
    record_types: Option<Vec<String>>,
) -> PyResult<Bound<'py, PyDict>> {
    let filter: Option<Vec<&str>> = record_types.as_ref().map(|t| t.iter().map(String::as_str).collect());
    let batches = py.detach(|| stdf_convert::arrow::read_tables(&path, filter.as_deref())).map_err(py_err)?;
    let tables = PyDict::new(py);
    for (record_type, batch) in batches {
        let schema = batch.schema();
        tables.set_item(record_type, PyTable::try_new(vec![batch], schema)?)?;
    }
    Ok(tables)
}

/// Run the `stdf-convert` command with `argv` and return its exit code.
#[pyfunction]
fn run_cli(py: Python<'_>, argv: Vec<OsString>) -> i32 {
    py.detach(|| stdf_convert::run(argv))
}

#[pymodule]
fn _stdf_convert(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Batches>()?;
    m.add_function(wrap_pyfunction!(schema, m)?)?;
    m.add_function(wrap_pyfunction!(batches, m)?)?;
    m.add_function(wrap_pyfunction!(tables, m)?)?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add("StdfError", m.py().get_type::<StdfError>())?;
    m.add("RECORD_TYPES", stdf_convert::RECORD_TYPES)?;
    m.add("DEFAULT_BATCH_SIZE", DEFAULT_BATCH_SIZE)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
