use knf::{Seg, Value, render_path};
use knf_convert::InputFormat;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

/// Maximum container nesting accepted when converting Python values.
const MAX_DEPTH: usize = 128;

/// Traversal state: the current key path and the containers being visited (for cycle detection).
#[derive(Default)]
struct Walk {
    path: Vec<Seg>,
    ancestors: Vec<usize>,
}

pub(super) fn dict_to_value(data: &Bound<'_, PyDict>, format: InputFormat) -> PyResult<Value> {
    convert(data.as_any(), &mut Walk::default(), format)
}

fn convert(value: &Bound<'_, PyAny>, walk: &mut Walk, format: InputFormat) -> PyResult<Value> {
    let (label, noun) = match format {
        InputFormat::Json => ("JSON", "object"),
        InputFormat::Toml => ("TOML", "table"),
    };
    let location = || format!("at `{}`", render_path(&walk.path));
    let container = value.is_instance_of::<PyDict>()
        || value.is_instance_of::<PyList>()
        || value.is_instance_of::<PyTuple>();
    if container {
        let identity = value.as_ptr() as usize;
        if walk.ancestors.contains(&identity) {
            return Err(PyValueError::new_err(format!(
                "circular reference {}",
                location()
            )));
        }
        if walk.ancestors.len() >= MAX_DEPTH {
            return Err(PyValueError::new_err(format!(
                "{} nesting exceeds {MAX_DEPTH} containers {}",
                label,
                location()
            )));
        }
        walk.ancestors.push(identity);
        let result = if let Ok(dict) = value.cast::<PyDict>() {
            let mut object = indexmap::IndexMap::new();
            for (key, item) in dict.iter() {
                let key: String = key.extract().map_err(|_| {
                    PyTypeError::new_err(format!(
                        "{} {} keys must be strings at `{}`",
                        label,
                        noun,
                        render_path(&walk.path)
                    ))
                })?;
                walk.path.push(Seg::Key(key.clone()));
                object.insert(key, convert(&item, walk, format)?);
                walk.path.pop();
            }
            Value::Object(object)
        } else {
            let mut items = Vec::new();
            for (index, item) in value.try_iter()?.enumerate() {
                walk.path.push(Seg::Index(index));
                items.push(convert(&item?, walk, format)?);
                walk.path.pop();
            }
            Value::Array(items)
        };
        walk.ancestors.pop();
        return Ok(result);
    }
    match format {
        InputFormat::Json => json_scalar(value, &walk.path),
        InputFormat::Toml => toml_scalar(value, &walk.path),
    }
}

fn json_scalar(value: &Bound<'_, PyAny>, path: &[Seg]) -> PyResult<Value> {
    let location = || format!("at `{}`", render_path(path));
    if value.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(v) = value.cast::<PyBool>() {
        return Ok(Value::Bool(v.extract()?));
    }
    if let Ok(v) = value.cast::<PyInt>() {
        return v
            .extract::<i64>()
            .map(Value::Integer)
            .or_else(|_| v.extract::<u64>().map(Value::Unsigned))
            .map_err(|_| {
                PyValueError::new_err(format!(
                    "integer outside JSON's supported 64-bit range {}",
                    location()
                ))
            });
    }
    if let Ok(v) = value.cast::<PyFloat>() {
        let f: f64 = v.extract()?;
        return f.is_finite().then_some(Value::Float(f)).ok_or_else(|| {
            PyValueError::new_err(format!("JSON requires finite floats {}", location()))
        });
    }
    if let Ok(v) = value.cast::<PyString>() {
        return Ok(Value::String(v.extract()?));
    }
    Err(PyTypeError::new_err(format!(
        "unsupported JSON type `{}` {}",
        value.get_type().name()?,
        location()
    )))
}

fn toml_scalar(value: &Bound<'_, PyAny>, path: &[Seg]) -> PyResult<Value> {
    // Non-finite floats (inf, nan) are intentionally allowed: TOML supports them.
    let location = || format!("at `{}`", render_path(path));
    if let Ok(v) = value.cast::<PyBool>() {
        return Ok(Value::Bool(v.extract()?));
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
    // Go through the standard library, as `datetime_to_py` does, for abi3.
    let datetime = value.py().import("datetime")?;
    let is_datetime = value.is_instance(&datetime.getattr("datetime")?)?;
    let is_time = value.is_instance(&datetime.getattr("time")?)?;
    if is_datetime || is_time || value.is_instance(&datetime.getattr("date")?)? {
        // Any tzinfo on a time is rejected: `utcoffset()` is `None` for
        // DST-aware zones, which would silently drop the zone.
        if is_time && !value.getattr("tzinfo")?.is_none() {
            return Err(PyValueError::new_err(format!(
                "TOML does not support times with a tzinfo {}",
                location()
            )));
        }
        if is_datetime {
            let offset = value.call_method0("utcoffset")?;
            if offset.is_none() {
                if !value.getattr("tzinfo")?.is_none() {
                    return Err(PyValueError::new_err(format!(
                        "TOML datetime has a tzinfo whose utcoffset() is None {}",
                        location()
                    )));
                }
            } else {
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
