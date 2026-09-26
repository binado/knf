//! `knf.load`: the `knf-core` pipeline as one Python function.
//!
//! A frontend, sibling to `knf-cli`: that one is argv and stderr, this one is
//! arguments and exceptions. Published to PyPI as `pyknf`. The `knf` command
//! installed by that wheel calls `cli_bin::main_from`, the same source
//! `knf-cli` compiles, so there is one command line and no second package.

use std::io;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[path = "../../knf/src/main.rs"]
mod cli_bin;

use knf::{
    InterpError, LoadError, MergeError, MergeOptions, Number, ProcessEnv, Value,
    interpolate as interpolate_value, load_layers, merge,
};
use pyo3::PyTypeInfo;
use pyo3::create_exception;
use pyo3::exceptions::{
    PyFileNotFoundError, PyIsADirectoryError, PyOSError, PyPermissionError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString};

create_exception!(
    knf,
    ParseError,
    PyValueError,
    "A file is not valid JSON or TOML, or is not an object at the top level."
);

create_exception!(
    knf,
    InterpolationError,
    PyValueError,
    "A configuration reference cannot be resolved."
);

/// Load and merge layered JSON and TOML files into one `dict`.
///
/// `files` are merged left to right. Objects merge key by key; arrays, scalars
/// and `None` replace wholesale. Interpolation, when requested, runs once
/// after all layers have been merged.
#[pyfunction]
#[pyo3(signature = (files, *, interpolate = false))]
fn load<'py>(
    py: Python<'py>,
    files: Vec<PathBuf>,
    interpolate: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let merged = py
        .detach(move || -> Result<Value, Failure> {
            // One file at a time, so a failure knows its path without anyone
            // parsing it back out of an error message.
            let mut layers = Vec::with_capacity(files.len());
            for path in files {
                match load_layers(std::slice::from_ref(&path), None) {
                    Ok((loaded, _formats)) => layers.extend(loaded),
                    Err(err) => return Err(Failure::File(path, err)),
                }
            }
            let merged = merge(layers, &MergeOptions::default()).map_err(Failure::Merge)?;
            if interpolate {
                interpolate_value(merged, &ProcessEnv).map_err(Failure::Interpolate)
            } else {
                Ok(merged)
            }
        })
        .map_err(|failure| match failure {
            Failure::File(path, err) => file_error(py, &path, err),
            // Only strict mode reports a conflict, and it is not exposed.
            Failure::Merge(err) => PyValueError::new_err(err.to_string()),
            Failure::Interpolate(err) => InterpolationError::new_err(err.to_string()),
        })?;

    value_to_py(py, merged)
}

enum Failure {
    File(PathBuf, anyhow::Error),
    Merge(MergeError),
    Interpolate(InterpError),
}

/// The exception Python's own I/O and parsers would raise for the same
/// failure: `OSError` subclasses the way `open()` raises them, `ValueError`
/// for a path knf cannot read as a layer, and [`ParseError`] — a `ValueError`,
/// like `json.JSONDecodeError` — for a file that is not a valid document.
fn file_error(py: Python<'_>, path: &Path, err: anyhow::Error) -> PyErr {
    if let Some(load) = err.downcast_ref::<LoadError>() {
        return match load {
            LoadError::Directory { .. } => {
                os_error::<PyIsADirectoryError>(py, "EISDIR", path, &err)
            }
            _ => PyValueError::new_err(load.to_string()),
        };
    }
    if let Some(io) = err.chain().find_map(|c| c.downcast_ref::<io::Error>()) {
        match io.kind() {
            io::ErrorKind::NotFound => {
                return os_error::<PyFileNotFoundError>(py, "ENOENT", path, &err);
            }
            io::ErrorKind::PermissionDenied => {
                return os_error::<PyPermissionError>(py, "EACCES", path, &err);
            }
            // `read_to_string` on bytes that are not UTF-8: the content's fault.
            io::ErrorKind::InvalidData => {}
            _ => return PyOSError::new_err(format!("{err:#}")),
        }
    }
    // `{:#}` is the whole chain on one line: "a.json: invalid JSON: …".
    ParseError::new_err(format!("{err:#}"))
}

/// `E(errno, strerror, filename)`, which is how `open()` raises, so `.errno`,
/// `.strerror` and `.filename` are all set.
///
/// By [`io::ErrorKind`] rather than [`io::Error::raw_os_error`]: on Windows the
/// raw code is not an errno, and passing it through would pick the wrong
/// `OSError` subclass. The errno comes from Python's own `errno` module.
fn os_error<E: PyTypeInfo>(
    py: Python<'_>,
    errno_name: &str,
    path: &Path,
    err: &anyhow::Error,
) -> PyErr {
    let build = || -> PyResult<PyErr> {
        let errno = py.import("errno")?.getattr(errno_name)?;
        let strerror = py.import("os")?.call_method1("strerror", (&errno,))?;
        let filename = path.to_string_lossy().into_owned();
        Ok(PyErr::new::<E, _>((
            errno.unbind(),
            strerror.unbind(),
            filename,
        )))
    };
    build().unwrap_or_else(|_| PyOSError::new_err(format!("{err:#}")))
}

/// Run the `knf` command line and exit. The installed script is a thin wrapper
/// around this. Its process was started by Python, so the arguments are
/// `sys.argv`, not `std::env::args`.
#[pyfunction]
fn cli(py: Python<'_>) -> PyResult<()> {
    let sys_argv = py.import("sys")?.getattr("argv")?;
    // Python decodes Unix argv with surrogateescape. Extracting it as Rust
    // Strings rejects filenames containing bytes that are not valid UTF-8.
    #[cfg(unix)]
    let argv = {
        let fsencode = py.import("os")?.getattr("fsencode")?;
        sys_argv
            .try_iter()?
            .map(|arg| {
                let bytes: Vec<u8> = fsencode.call1((arg?,))?.extract()?;
                Ok(OsString::from_vec(bytes))
            })
            .collect::<PyResult<Vec<_>>>()?
    };
    #[cfg(not(unix))]
    let argv: Vec<String> = sys_argv.extract()?;
    cli_bin::main_from(argv);
    Ok(())
}

#[pymodule]
fn _knf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add(
        "InterpolationError",
        m.py().get_type::<InterpolationError>(),
    )?;
    Ok(())
}

// --- IR → Python ----------------------------------------------------------

/// Total: Python spells everything the IR holds. A TOML datetime comes back as
/// its TOML spelling, the same string `knf -f json` prints.
fn value_to_py(py: Python<'_>, value: Value) -> PyResult<Bound<'_, PyAny>> {
    Ok(match value {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, b).to_owned().into_any(),
        Value::Number(Number::I64(i)) => i.into_pyobject(py)?.into_any(),
        Value::Number(Number::U64(u)) => u.into_pyobject(py)?.into_any(),
        Value::Number(Number::F64(f)) => f.into_pyobject(py)?.into_any(),
        Value::String(s) | Value::Datetime(s) => PyString::new(py, &s).into_any(),
        Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(value_to_py(py, item)?)?;
            }
            list.into_any()
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (key, value) in map {
                dict.set_item(key, value_to_py(py, value)?)?;
            }
            dict.into_any()
        }
    })
}
