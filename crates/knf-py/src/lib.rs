//! `knf.deep_merge`: the `knf-core` pipeline as one Python function.
//!
//! A frontend, sibling to `knf-cli`: that one is argv and stderr, this one is
//! arguments and exceptions. Published to PyPI as `pyknf`, which depends on the
//! `knf-cli` wheel, so `pip install pyknf` gives both.

use std::path::PathBuf;

use knf::{Map, MergeOptions, Number, Seg, Value, load_layers, merge, render_path};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

create_exception!(
    knf,
    KnfError,
    PyException,
    "A file could not be read, parsed or merged."
);

/// Merge layered JSON and TOML files into one `dict`.
///
/// `files` are merged left to right; `override`, if given, is merged last, as
/// one more layer — the Python spelling of `knf --set`. Objects merge key by
/// key; arrays, scalars and `None` replace wholesale.
#[pyfunction]
#[pyo3(signature = (files, *, r#override = None))]
fn deep_merge<'py>(
    py: Python<'py>,
    files: Vec<PathBuf>,
    r#override: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    // Before any file is read: a bad override is a mistake in the arguments
    // alone, and saying so must not wait on the files existing or parsing.
    let overlay = r#override
        .map(|dict| object_from_py(dict, &mut Vec::new()))
        .transpose()?;

    let merged = py
        .detach(move || -> anyhow::Result<Value> {
            let (mut layers, _formats) = load_layers(&files, None)?;
            layers.extend(overlay.map(Value::Object));
            Ok(merge(layers, &MergeOptions::default())?)
        })
        // `{:#}` is the whole cause chain on one line: "reading `x`: No such file".
        .map_err(|err| KnfError::new_err(format!("{err:#}")))?;

    value_to_py(py, merged)
}

#[pymodule]
fn _knf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(deep_merge, m)?)?;
    m.add("KnfError", m.py().get_type::<KnfError>())?;
    Ok(())
}

// --- Python → IR ----------------------------------------------------------

/// A JSON-like Python value as a layer. Anything else is rejected by path —
/// including `datetime`, since a `Value::Datetime` only ever comes from TOML.
fn value_from_py(obj: &Bound<'_, PyAny>, path: &mut Vec<Seg>) -> PyResult<Value> {
    // `bool` before `int`: `True` is an `int` to Python.
    if obj.is_none() {
        Ok(Value::Null)
    } else if let Ok(b) = obj.cast::<PyBool>() {
        Ok(Value::Bool(b.is_true()))
    } else if obj.is_instance_of::<PyInt>() {
        let number = if let Ok(i) = obj.extract::<i64>() {
            Number::I64(i)
        } else if let Ok(u) = obj.extract::<u64>() {
            Number::from_u64(u)
        } else {
            return Err(PyValueError::new_err(format!(
                "override: integer at `{}` does not fit in 64 bits",
                render_path(path)
            )));
        };
        Ok(Value::Number(number))
    } else if let Ok(f) = obj.cast::<PyFloat>() {
        Ok(Value::Number(Number::F64(f.value())))
    } else if let Ok(s) = obj.cast::<PyString>() {
        Ok(Value::String(s.to_cow()?.into_owned()))
    } else if let Ok(dict) = obj.cast::<PyDict>() {
        Ok(Value::Object(object_from_py(dict, path)?))
    } else if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>() {
        let mut items = Vec::new();
        for (i, item) in obj.try_iter()?.enumerate() {
            path.push(Seg::Index(i));
            items.push(value_from_py(&item?, path)?);
            path.pop();
        }
        Ok(Value::Array(items))
    } else {
        Err(PyTypeError::new_err(format!(
            "override: `{}` is a {}, which has no place in a config",
            render_path(path),
            obj.get_type().name()?
        )))
    }
}

fn object_from_py(dict: &Bound<'_, PyDict>, path: &mut Vec<Seg>) -> PyResult<Map> {
    let mut map = Map::with_capacity(dict.len());
    for (key, value) in dict.iter() {
        let Ok(key) = key.extract::<String>() else {
            return Err(PyTypeError::new_err(format!(
                "override: keys must be str, got {} under `{}`",
                key.get_type().name()?,
                render_path(path)
            )));
        };
        path.push(Seg::Key(key.clone()));
        let value = value_from_py(&value, path)?;
        path.pop();
        map.insert(key, value);
    }
    Ok(map)
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
