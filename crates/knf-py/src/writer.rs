use knf::{Seg, render_path};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

/// Serialize a dict as TOML, preserving insertion order and ending with a newline.
///
/// Values may be strings, bools, signed 64-bit integers, floats, lists, tuples,
/// dicts with string keys, and datetime/date/time objects. None is unsupported.
/// Datetime offsets must be whole minutes; time objects must be naive.
/// Raises TypeError for unsupported types and ValueError for invalid values,
/// cycles, or nesting deeper than 128 containers. Errors include the key path.
#[pyfunction]
#[pyo3(signature = (data, *, pretty = true))]
pub(super) fn dumps_toml(data: &Bound<'_, PyDict>, pretty: bool) -> PyResult<String> {
    let value = convert(data.as_any(), &mut Vec::new(), &mut Vec::new())?;
    knf::format::emit(value, pretty).map_err(|err| PyValueError::new_err(err.to_string()))
}

fn convert(
    value: &Bound<'_, PyAny>,
    path: &mut Vec<Seg>,
    ancestors: &mut Vec<usize>,
) -> PyResult<toml::Value> {
    use toml::Value;
    let location = || format!("at `{}`", render_path(path));
    if let Ok(v) = value.cast::<PyBool>() {
        return Ok(Value::Boolean(v.extract()?));
    }
    if let Ok(v) = value.cast::<PyInt>() {
        return v.extract().map(Value::Integer).map_err(|_| {
            PyValueError::new_err(format!(
                "integer outside TOML's signed 64-bit range {}",
                location()
            ))
        });
    }
    if let Ok(v) = value.cast::<PyFloat>() {
        return Ok(Value::Float(v.extract()?));
    }
    if let Ok(v) = value.cast::<PyString>() {
        return Ok(Value::String(v.extract()?));
    }
    let container = value.is_instance_of::<PyDict>()
        || value.is_instance_of::<PyList>()
        || value.is_instance_of::<PyTuple>();
    if container {
        let identity = value.as_ptr() as usize;
        if ancestors.contains(&identity) {
            return Err(PyValueError::new_err(format!(
                "circular reference {}",
                location()
            )));
        }
        if ancestors.len() >= 128 {
            return Err(PyValueError::new_err(format!(
                "TOML nesting exceeds 128 containers {}",
                location()
            )));
        }
        ancestors.push(identity);
        let result = if let Ok(dict) = value.cast::<PyDict>() {
            let mut table = toml::Table::new();
            for (key, item) in dict.iter() {
                let key: String = key.extract().map_err(|_| {
                    PyTypeError::new_err(format!(
                        "TOML table keys must be strings at `{}`",
                        render_path(path)
                    ))
                })?;
                path.push(Seg::Key(key.clone()));
                table.insert(key, convert(&item, path, ancestors)?);
                path.pop();
            }
            Value::Table(table)
        } else {
            let mut items = Vec::new();
            for (index, item) in value.try_iter()?.enumerate() {
                path.push(Seg::Index(index));
                items.push(convert(&item?, path, ancestors)?);
                path.pop();
            }
            Value::Array(items)
        };
        ancestors.pop();
        return Ok(result);
    }
    // Use the standard library to keep abi3/Python 3.9 compatibility.
    let datetime = value.py().import("datetime")?;
    let is_datetime = value.is_instance(&datetime.getattr("datetime")?)?;
    let is_time = value.is_instance(&datetime.getattr("time")?)?;
    if is_datetime || is_time || value.is_instance(&datetime.getattr("date")?)? {
        if is_datetime || is_time {
            let offset = value.call_method0("utcoffset")?;
            if !offset.is_none() {
                if is_time {
                    return Err(PyValueError::new_err(format!(
                        "TOML does not support offset times {}",
                        location()
                    )));
                }
                let seconds: f64 = offset.call_method0("total_seconds")?.extract()?;
                if seconds % 60.0 != 0.0 {
                    return Err(PyValueError::new_err(format!(
                        "TOML datetime offsets must be whole minutes {}",
                        location()
                    )));
                }
            }
        }
        let text: String = value.call_method0("isoformat")?.extract()?;
        return text
            .parse::<toml::value::Datetime>()
            .map(Value::Datetime)
            .map_err(|err| {
                PyValueError::new_err(format!("invalid TOML datetime {}: {err}", location()))
            });
    }
    Err(PyTypeError::new_err(format!(
        "unsupported TOML type `{}` {}",
        value.get_type().name()?,
        location()
    )))
}
