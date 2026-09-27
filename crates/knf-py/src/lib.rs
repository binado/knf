//! Native Python loading, file discovery and path filtering from `knf-core`.
//!
//! A frontend, sibling to `knf-cli`: that one is argv and stderr, this one is
//! arguments and exceptions. Published to PyPI as `pyknf`. The `knf` command
//! installed by that wheel calls `cli_bin::main_from`, the same source
//! `knf-cli` compiles, so there is one command line and no second package.

use anyhow::Context;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[path = "../../knf/src/main.rs"]
mod cli_bin;

use knf::fs::{AccumulateError, AccumulateTarget};
use knf::glob::{GlobError, GlobPattern};
use knf::{
    ConfigFormat, Format, InterpError, LoadError, MergeError, MergeOptions, ProcessEnv, Seg,
    interpolate as interpolate_value, merge, render_path, resolve_format,
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

/// Discover ordered JSON or TOML paths without reading their contents.
/// An explicit base produces absolute paths; the default produces relative paths.
#[pyfunction]
#[pyo3(signature = (target, *, base_dir = None))]
fn accumulate(
    py: Python<'_>,
    target: PathBuf,
    base_dir: Option<PathBuf>,
) -> PyResult<Vec<PathBuf>> {
    let target =
        AccumulateTarget::try_from(target).map_err(|err| PyValueError::new_err(err.to_string()))?;
    py.detach(move || knf::fs::accumulate(&target, base_dir.as_deref()))
        .map_err(|err| match err {
            AccumulateError::Inspect { path, source } | AccumulateError::List { path, source } => {
                discovery_os_error(py, Some(&path), source)
            }
            AccumulateError::CurrentDirectory(source) => discovery_os_error(py, None, source),
            AccumulateError::Directory { path } => discovery_os_error(
                py,
                Some(&path),
                io::Error::from(io::ErrorKind::IsADirectory),
            ),
            err @ AccumulateError::NonRegular { .. } => PyValueError::new_err(err.to_string()),
        })
}

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

/// Build OSError with native filenames and errno, letting Python select its subclass.
fn discovery_os_error(py: Python<'_>, path: Option<&Path>, source: io::Error) -> PyErr {
    let build = || -> PyResult<PyErr> {
        let errno_name = match source.kind() {
            io::ErrorKind::NotFound => "ENOENT",
            io::ErrorKind::PermissionDenied => "EACCES",
            io::ErrorKind::IsADirectory => "EISDIR",
            io::ErrorKind::NotADirectory => "ENOTDIR",
            io::ErrorKind::AlreadyExists => "EEXIST",
            io::ErrorKind::Interrupted => "EINTR",
            io::ErrorKind::InvalidInput => "EINVAL",
            _ => "EIO",
        };
        let fallback: i32 = py.import("errno")?.getattr(errno_name)?.extract()?;
        #[cfg(unix)]
        let errno = source.raw_os_error().unwrap_or(fallback);
        #[cfg(not(unix))]
        let errno = fallback;
        let strerror = py.import("os")?.call_method1("strerror", (errno,))?;
        let filename = match path {
            Some(path) => path.as_os_str().into_pyobject(py)?.into_any().unbind(),
            None => py.None(),
        };
        #[cfg(windows)]
        let args = (
            errno,
            strerror.unbind(),
            filename,
            py.None(),
            source.raw_os_error(),
        );
        #[cfg(not(windows))]
        let args = (errno, strerror.unbind(), filename);
        Ok(PyOSError::new_err(args))
    };
    build().unwrap_or_else(|_| PyOSError::new_err(source.to_string()))
}

/// Load and merge homogeneous JSON or TOML files into one `dict`.
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
        .detach(move || {
            let format = resolve_format(&files, None).map_err(|err| {
                Failure::File(
                    err.path().unwrap_or(Path::new("")).to_path_buf(),
                    err.into(),
                )
            })?;
            match format {
                Format::Json => load_native(&files, interpolate).map(Document::Json),
                Format::Toml => load_native(&files, interpolate).map(Document::Toml),
            }
        })
        .map_err(|failure| match failure {
            Failure::File(path, err) => file_error(py, &path, err),
            Failure::Merge(err) => PyValueError::new_err(err.to_string()),
            Failure::Interpolate(err) => InterpolationError::new_err(err.to_string()),
        })?;
    match merged {
        Document::Json(value) => json_to_py(py, value),
        Document::Toml(value) => toml_to_py(py, value, &mut Vec::new()),
    }
}

// Dispatch only at the boundary; each pipeline remains entirely native.
enum Document {
    Json(serde_json::Value),
    Toml(toml::Value),
}

fn load_native<V: ConfigFormat>(files: &[PathBuf], interpolate: bool) -> Result<V, Failure> {
    let mut layers = Vec::with_capacity(files.len());
    for path in files {
        let read = || -> anyhow::Result<V> {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading `{}`", path.display()))?;
            knf::format::parse(&text, &knf::format::SourceName::File(path.clone()))
        };
        layers.push(read().map_err(|err| Failure::File(path.clone(), err))?);
    }
    let merged = merge(layers, &MergeOptions::default()).map_err(Failure::Merge)?;
    if interpolate {
        interpolate_value(merged, &ProcessEnv).map_err(Failure::Interpolate)
    } else {
        Ok(merged)
    }
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
    m.add_function(wrap_pyfunction!(accumulate, m)?)?;
    m.add_function(wrap_pyfunction!(filter_paths, m)?)?;
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add(
        "InterpolationError",
        m.py().get_type::<InterpolationError>(),
    )?;
    Ok(())
}

// --- Native values → Python -----------------------------------------------

fn json_to_py(py: Python<'_>, value: serde_json::Value) -> PyResult<Bound<'_, PyAny>> {
    use serde_json::Value;
    Ok(match value {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, b).to_owned().into_any(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any()
            } else if let Some(u) = n.as_u64() {
                u.into_pyobject(py)?.into_any()
            } else {
                n.as_f64()
                    .expect("a native JSON number is i64/u64/f64")
                    .into_pyobject(py)?
                    .into_any()
            }
        }
        Value::String(s) => PyString::new(py, &s).into_any(),
        Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any()
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (key, value) in map {
                dict.set_item(key, json_to_py(py, value)?)?;
            }
            dict.into_any()
        }
    })
}

fn toml_to_py<'py>(
    py: Python<'py>,
    value: toml::Value,
    path: &mut Vec<Seg>,
) -> PyResult<Bound<'py, PyAny>> {
    use toml::Value;
    Ok(match value {
        Value::Boolean(b) => PyBool::new(py, b).to_owned().into_any(),
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
                list.append(toml_to_py(py, item, path)?)?;
                path.pop();
            }
            list.into_any()
        }
        Value::Table(map) => {
            let dict = PyDict::new(py);
            for (key, value) in map {
                path.push(Seg::Key(key.clone()));
                dict.set_item(key, toml_to_py(py, value, path)?)?;
                path.pop();
            }
            dict.into_any()
        }
    })
}

fn datetime_to_py(py: Python<'_>, value: toml::value::Datetime) -> PyResult<Bound<'_, PyAny>> {
    // Construct through the standard library for abi3/Python 3.9 compatibility.
    // Pass native components directly; never serialize and reparse a document.
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
