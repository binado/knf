//! Python bindings for `knf`, published as `pyknf`. The wheel's `knf`
//! command runs the same `main_from` as `knf-cli`.

use std::io;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[path = "../../knf/src/main.rs"]
mod cli_bin;

use knf::glob::{GlobError, GlobPattern, KeyGlobError, KeyGlobPattern};
use knf::{InterpError, InterpolationOptions, MergeOptions, Seg, Value, merge_layers, render_path};
use knf_convert::{InputFormat, Layers, LoadError, ReadError, load_layers};
use pyo3::PyTypeInfo;
use pyo3::create_exception;
use pyo3::exceptions::{
    PyFileNotFoundError, PyIsADirectoryError, PyOSError, PyPermissionError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString};

mod from_py;
mod writer;

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

/// Filter candidates without filesystem access, preserving their order and duplicates.
/// Matching precedes conversion to pathlib.Path, which can normalize `./`.
#[pyfunction]
#[pyo3(signature = (files, pattern, *, filename_only = false))]
fn filter_paths(files: Vec<PathBuf>, pattern: &str, filename_only: bool) -> PyResult<Vec<PathBuf>> {
    let pattern: GlobPattern = pattern
        .parse()
        .map_err(|err: GlobError| PyValueError::new_err(err.to_string()))?;
    Ok(knf::fs::filter_paths(&files, &pattern, filename_only))
}

/// Load and merge homogeneous JSON or TOML files into one `dict`.
///
/// `files` merge left to right. `shallow` is a key-path glob whose matches
/// replace wholesale. With `interpolate`, references bind to the final
/// document and a whole-string reference merges as the value it names.
/// `context` is one filepath used only for interpolation, with output-first
/// lookup; it requires `interpolate=True`.
#[pyfunction]
#[pyo3(signature = (files, *, interpolate = false, shallow = None, context = None))]
fn load<'py>(
    py: Python<'py>,
    files: Vec<PathBuf>,
    interpolate: bool,
    shallow: Option<&str>,
    context: Option<PathBuf>,
) -> PyResult<Bound<'py, PyAny>> {
    if context.is_some() && !interpolate {
        return Err(PyValueError::new_err("context requires interpolate=True"));
    }
    let shallow: Option<KeyGlobPattern> = shallow
        .map(str::parse)
        .transpose()
        .map_err(|err: KeyGlobError| PyValueError::new_err(err.to_string()))?;
    let opts = MergeOptions { shallow };
    let merged = py
        .detach(move || {
            let Layers {
                format,
                values,
                context,
            } = load_layers(&files, context.as_deref(), Some(InputFormat::Json))
                .map_err(Failure::Load)?;
            let parse_inline = |text| format.parse_inline(text);
            let interpolation = interpolate
                .then(|| InterpolationOptions::new(&parse_inline).with_context(context.as_ref()));
            merge_layers(values, &opts, interpolation).map_err(Failure::Interpolate)
        })
        .map_err(|failure| match failure {
            Failure::Load(err) => load_error(py, err),
            Failure::Interpolate(err) => InterpolationError::new_err(err.to_string()),
        })?;
    value_to_py(py, merged, &mut Vec::new())
}

enum Failure {
    Load(LoadError),
    Interpolate(InterpError),
}

/// Maps a load failure to the exception Python itself would raise: an
/// `OSError` subclass, `ValueError`, or [`ParseError`].
fn load_error(py: Python<'_>, err: LoadError) -> PyErr {
    match err {
        LoadError::Directory { ref path } => {
            os_error::<PyIsADirectoryError>(py, "EISDIR", path, &err)
        }
        LoadError::NoInputs | LoadError::MixedFormats | LoadError::UnknownExtension { .. } => {
            PyValueError::new_err(err.to_string())
        }
        LoadError::File { path, source } => file_error(py, &path, &source),
    }
}

/// Maps a read or parse failure of `path`.
fn file_error(py: Python<'_>, path: &Path, err: &ReadError) -> PyErr {
    if let ReadError::Io { source, .. } = err {
        match source.kind() {
            io::ErrorKind::NotFound => {
                return os_error::<PyFileNotFoundError>(py, "ENOENT", path, err);
            }
            io::ErrorKind::PermissionDenied => {
                return os_error::<PyPermissionError>(py, "EACCES", path, err);
            }
            // `read_to_string` on bytes that are not UTF-8: the content's fault.
            io::ErrorKind::InvalidData => {}
            _ => return PyOSError::new_err(one_line(err)),
        }
    }
    ParseError::new_err(one_line(err))
}

/// The error and its sources on one line: "a.json: invalid JSON: …".
fn one_line(err: &dyn std::error::Error) -> String {
    let mut line = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        line.push_str(": ");
        line.push_str(&cause.to_string());
        source = cause.source();
    }
    line
}

/// Builds `E(errno, strerror, filename)` as `open()` does.
///
/// Maps by [`io::ErrorKind`] because Windows raw OS codes are not errnos.
fn os_error<E: PyTypeInfo>(
    py: Python<'_>,
    errno_name: &str,
    path: &Path,
    err: &dyn std::error::Error,
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
    build().unwrap_or_else(|_| PyOSError::new_err(one_line(err)))
}

/// Runs the `knf` command line with `sys.argv` and exits.
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
    m.add_function(wrap_pyfunction!(writer::dumps, m)?)?;
    m.add_function(wrap_pyfunction!(writer::dumps_toml, m)?)?;
    m.add_function(wrap_pyfunction!(filter_paths, m)?)?;
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add(
        "InterpolationError",
        m.py().get_type::<InterpolationError>(),
    )?;
    Ok(())
}

fn value_to_py<'py>(
    py: Python<'py>,
    value: Value,
    path: &mut Vec<Seg>,
) -> PyResult<Bound<'py, PyAny>> {
    Ok(match value {
        Value::Null => py.None().into_bound(py),
        Value::Unsigned(u) => u.get().into_pyobject(py)?.into_any(),
        Value::Bool(b) => PyBool::new(py, b).to_owned().into_any(),
        Value::Integer(i) => i.into_pyobject(py)?.into_any(),
        Value::Float(f) => f.into_pyobject(py)?.into_any(),
        Value::String(s) => PyString::new(py, &s).into_any(),
        Value::Datetime(dt) => datetime_to_py(py, dt).map_err(|err| {
            // Python cannot represent e.g. year zero or leap seconds.
            if err.is_instance_of::<PyValueError>(py) {
                PyValueError::new_err(format!(
                    "cannot represent TOML datetime at `{}` in Python: {err}",
                    render_path(path)
                ))
            } else {
                err
            }
        })?,
        Value::Array(items) => {
            let list = PyList::empty(py);
            for (index, item) in items.into_iter().enumerate() {
                path.push(Seg::Index(index));
                list.append(value_to_py(py, item, path)?)?;
                path.pop();
            }
            list.into_any()
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (key, value) in map {
                path.push(Seg::Key(key.clone()));
                dict.set_item(key, value_to_py(py, value, path)?)?;
                path.pop();
            }
            dict.into_any()
        }
    })
}

fn datetime_to_py(py: Python<'_>, value: toml::value::Datetime) -> PyResult<Bound<'_, PyAny>> {
    // Construct through the standard library for abi3 compatibility.
    let module = py.import("datetime")?;
    match (value.date, value.time) {
        (Some(date), Some(time)) => {
            let tz = match value.offset {
                None => py.None().into_bound(py),
                Some(toml::value::Offset::Z) => module.getattr("timezone")?.getattr("utc")?,
                Some(toml::value::Offset::Custom { minutes }) => {
                    let delta = module
                        .getattr("timedelta")?
                        .call1((0, i32::from(minutes) * 60))?;
                    module.getattr("timezone")?.call1((delta,))?
                }
            };
            module.getattr("datetime")?.call1((
                date.year,
                date.month,
                date.day,
                time.hour,
                time.minute,
                time.second.unwrap_or(0),
                time.nanosecond.unwrap_or(0) / 1000,
                tz,
            ))
        }
        (Some(date), None) => module
            .getattr("date")?
            .call1((date.year, date.month, date.day)),
        (None, Some(time)) => module.getattr("time")?.call1((
            time.hour,
            time.minute,
            time.second.unwrap_or(0),
            time.nanosecond.unwrap_or(0) / 1000,
        )),
        (None, None) => Err(PyValueError::new_err(
            "datetime has neither a date nor a time",
        )),
    }
}
