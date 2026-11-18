use knf::{ConfigObject, ConfigValue, Seg, render_path};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

pub(super) fn dict_to_toml(data: &Bound<'_, PyDict>) -> PyResult<toml::Value> {
    convert(
        data.as_any(),
        "TOML",
        toml_scalar,
        &mut Vec::new(),
        &mut Vec::new(),
    )
}

pub(super) fn dict_to_json(data: &Bound<'_, PyDict>) -> PyResult<serde_json::Value> {
    convert(
        data.as_any(),
        "JSON",
        json_scalar,
        &mut Vec::new(),
        &mut Vec::new(),
    )
}

fn convert<V: ConfigValue>(
    value: &Bound<'_, PyAny>,
    label: &str,
    scalar: fn(&Bound<'_, PyAny>, &[Seg]) -> PyResult<V>,
    path: &mut Vec<Seg>,
    ancestors: &mut Vec<usize>,
) -> PyResult<V> {
    let location = || format!("at `{}`", render_path(path));
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
                "{label} nesting exceeds 128 containers {}",
                location()
            )));
        }
        ancestors.push(identity);
        let result = if let Ok(dict) = value.cast::<PyDict>() {
            let mut object = V::Object::new();
            for (key, item) in dict.iter() {
                let noun = if label == "TOML" { "table" } else { "object" };
                let key: String = key.extract().map_err(|_| {
                    PyTypeError::new_err(format!(
                        "{label} {noun} keys must be strings at `{}`",
                        render_path(path)
                    ))
                })?;
                path.push(Seg::Key(key.clone()));
                object.insert(key, convert(&item, label, scalar, path, ancestors)?);
                path.pop();
            }
            V::object(object)
        } else {
            let mut items = Vec::new();
            for (index, item) in value.try_iter()?.enumerate() {
                path.push(Seg::Index(index));
                items.push(convert(&item?, label, scalar, path, ancestors)?);
                path.pop();
            }
            V::array(items)
        };
        ancestors.pop();
        return Ok(result);
    }
    scalar(value, path)
}

fn json_scalar(value: &Bound<'_, PyAny>, path: &[Seg]) -> PyResult<serde_json::Value> {
    use serde_json::{Number, Value};
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
            .map(Number::from)
            .or_else(|_| v.extract::<u64>().map(Number::from))
            .map(Value::Number)
            .map_err(|_| {
                PyValueError::new_err(format!(
                    "integer outside JSON's supported 64-bit range {}",
                    location()
                ))
            });
    }
    if let Ok(v) = value.cast::<PyFloat>() {
        return Number::from_f64(v.extract()?)
            .map(Value::Number)
            .ok_or_else(|| {
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

fn toml_scalar(value: &Bound<'_, PyAny>, path: &[Seg]) -> PyResult<toml::Value> {
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
            if !offset.is_none() {
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
