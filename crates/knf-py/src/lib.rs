//! Python bindings for `knf-core`, published as `pyknf`. The wheel's `knf`
//! command runs the same `main_from` as `knf-cli`.

use anyhow::Context;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[path = "../../knf/src/main.rs"]
mod cli_bin;

use knf::fs::{AccumulateError, AccumulateTarget};
use knf::glob::{GlobError, GlobPattern, KeyGlobError, KeyGlobPattern};
use knf::{
    ConfigFormat, Format, InterpError, InterpOptions, LoadError, MergeError, MergeOptions,
    ProcessEnv, Seg, merge, merge_interpolate, render_path, resolve_format,
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
/// `files` merge left to right. `shallow` is a key-path glob whose matches
/// replace wholesale. With `interpolate`, references bind to the final
/// document and a whole-string reference merges as the value it names.
/// `context` is one filepath used only for interpolation, with output-first
/// lookup; it requires `interpolate=True`.
/// `merge_key` selects a literal inheritance key containing one whole-string
/// object reference; requires `interpolate=True` and honors `shallow`.
#[pyfunction]
#[pyo3(signature = (files, *, interpolate = false, shallow = None, context = None, merge_key = None))]
fn load<'py>(
    py: Python<'py>,
    files: Vec<PathBuf>,
    interpolate: bool,
    shallow: Option<&str>,
    context: Option<PathBuf>,
    merge_key: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    if merge_key.is_some() && !interpolate {
        return Err(PyValueError::new_err("merge_key requires interpolate=True"));
    }
    if let Some(path) = &context {
        if !interpolate {
            return Err(PyValueError::new_err("context requires interpolate=True"));
        }
        if path.as_os_str() == knf::STDIN {
            return Err(PyValueError::new_err("context does not accept stdin"));
        }
    }
    let shallow: Option<KeyGlobPattern> = shallow
        .map(str::parse)
        .transpose()
        .map_err(|err: KeyGlobError| PyValueError::new_err(err.to_string()))?;
    let opts = MergeOptions {
        shallow,
        ..Default::default()
    };
    let interp_options = InterpOptions {
        merge_key,
        shallow: opts.shallow.clone(),
    };
    let merged = py
        .detach(move || {
            let mut inputs = files.clone();
            inputs.extend(context.iter().cloned());
            let format = resolve_format(&inputs, None).map_err(|err| {
                Failure::File(
                    err.path().unwrap_or(Path::new("")).to_path_buf(),
                    err.into(),
                )
            })?;
            match format {
                Format::Json => load_native(
                    &files,
                    context.as_deref(),
                    interpolate,
                    &opts,
                    &interp_options,
                )
                .map(Document::Json),
                Format::Toml => load_native(
                    &files,
                    context.as_deref(),
                    interpolate,
                    &opts,
                    &interp_options,
                )
                .map(Document::Toml),
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

fn load_native<V: ConfigFormat>(
    files: &[PathBuf],
    context_path: Option<&Path>,
    interpolate: bool,
    opts: &MergeOptions,
    interp_options: &InterpOptions,
) -> Result<V, Failure> {
    let mut layers = Vec::with_capacity(files.len() + usize::from(context_path.is_some()));
    for path in files.iter().map(PathBuf::as_path).chain(context_path) {
        let read = || -> anyhow::Result<V> {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading `{}`", path.display()))?;
            knf::format::parse(&text, &knf::format::SourceName::File(path.to_path_buf()))
        };
        layers.push(read().map_err(|err| Failure::File(path.to_path_buf(), err))?);
    }
    let context = context_path.map(|_| layers.pop().expect("context was loaded last"));
    if interpolate {
        merge_interpolate(layers, context.as_ref(), &ProcessEnv, interp_options)
            .map_err(Failure::Interpolate)
    } else {
        merge(layers, opts).map_err(Failure::Merge)
    }
}

enum Failure {
    File(PathBuf, anyhow::Error),
    Merge(MergeError),
    Interpolate(InterpError),
}

/// Maps a load failure to the exception Python itself would raise: an
/// `OSError` subclass, `ValueError`, or [`ParseError`].
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

/// Builds `E(errno, strerror, filename)` as `open()` does.
///
/// Maps by [`io::ErrorKind`] because Windows raw OS codes are not errnos.
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
