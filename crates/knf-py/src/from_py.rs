use knf::{Seg, Value, render_path};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

/// Maximum container nesting accepted when converting Python values.
const MAX_DEPTH: usize = 128;

/// Traversal state: the input's name, the current key path and the containers
/// being visited (for cycle detection).
#[derive(Default)]
struct Walk<'a> {
    source: Option<&'a str>,
    path: Vec<Seg>,
    ancestors: Vec<usize>,
}

impl Walk<'_> {
    fn location(&self) -> String {
        let path = render_path(&self.path);
        match self.source {
            Some(source) => format!("at `{path}` in {source}"),
            None => format!("at `{path}`"),
        }
    }
}

/// Convert a dict to a [`Value`], keeping every scalar some output format can
/// carry; the output format rejects what it cannot represent. Errors name
/// `source`, when given, after the key path.
pub(super) fn dict_to_value(data: &Bound<'_, PyDict>, source: Option<&str>) -> PyResult<Value> {
    let mut walk = Walk {
        source,
        ..Walk::default()
    };
    convert(data.as_any(), &mut walk)
}

fn convert(value: &Bound<'_, PyAny>, walk: &mut Walk) -> PyResult<Value> {
    let container = value.is_instance_of::<PyDict>()
        || value.is_instance_of::<PyList>()
        || value.is_instance_of::<PyTuple>();
    if !container {
        return scalar(value, walk);
    }
    let identity = value.as_ptr() as usize;
    if walk.ancestors.contains(&identity) {
        return Err(PyValueError::new_err(format!(
            "circular reference {}",
            walk.location()
        )));
    }
    if walk.ancestors.len() >= MAX_DEPTH {
        return Err(PyValueError::new_err(format!(
            "nesting exceeds {MAX_DEPTH} containers {}",
            walk.location()
        )));
    }
    walk.ancestors.push(identity);
    let result = if let Ok(dict) = value.cast::<PyDict>() {
        let mut object = indexmap::IndexMap::new();
        for (key, item) in dict.iter() {
            let key: String = key.extract().map_err(|_| {
                PyTypeError::new_err(format!("keys must be strings {}", walk.location()))
            })?;
            walk.path.push(Seg::Key(key.clone()));
            object.insert(key, convert(&item, walk)?);
            walk.path.pop();
        }
        Value::Object(object)
    } else {
        let mut items = Vec::new();
        for (index, item) in value.try_iter()?.enumerate() {
            walk.path.push(Seg::Index(index));
            items.push(convert(&item?, walk)?);
            walk.path.pop();
        }
        Value::Array(items)
    };
    walk.ancestors.pop();
    Ok(result)
}

fn scalar(value: &Bound<'_, PyAny>, walk: &Walk) -> PyResult<Value> {
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
            .or_else(|_| v.extract::<u64>().map(Value::unsigned))
            .map_err(|_| {
                PyValueError::new_err(format!(
                    "integer outside the 64-bit range -2**63..2**64-1 {}",
                    walk.location()
                ))
            });
    }
    if let Ok(v) = value.cast::<PyFloat>() {
        return Ok(Value::Float(v.extract()?));
    }
    if let Ok(v) = value.cast::<PyString>() {
        return Ok(Value::String(v.extract()?));
    }
    if let Some(datetime) = datetime(value, walk)? {
        return Ok(Value::Datetime(datetime));
    }
    Err(PyTypeError::new_err(format!(
        "unsupported type `{}` {}",
        value.get_type().name()?,
        walk.location()
    )))
}

/// A `datetime`, `date` or `time` as a TOML datetime, or `None` for other types.
fn datetime(value: &Bound<'_, PyAny>, walk: &Walk) -> PyResult<Option<toml::value::Datetime>> {
    // Go through the standard library, as `datetime_to_py` does, for abi3.
    let module = value.py().import("datetime")?;
    let is_datetime = value.is_instance(&module.getattr("datetime")?)?;
    let is_time = value.is_instance(&module.getattr("time")?)?;
    if !(is_datetime || is_time || value.is_instance(&module.getattr("date")?)?) {
        return Ok(None);
    }
    // Any tzinfo on a time is rejected: `utcoffset()` is `None` for
    // DST-aware zones, which would silently drop the zone.
    if is_time && !value.getattr("tzinfo")?.is_none() {
        return Err(PyValueError::new_err(format!(
            "times with a tzinfo are unsupported {}",
            walk.location()
        )));
    }
    if is_datetime {
        let offset = value.call_method0("utcoffset")?;
        if offset.is_none() {
            if !value.getattr("tzinfo")?.is_none() {
                return Err(PyValueError::new_err(format!(
                    "datetime has a tzinfo whose utcoffset() is None {}",
                    walk.location()
                )));
            }
        } else {
            let seconds: f64 = offset.call_method0("total_seconds")?.extract()?;
            if seconds % 60.0 != 0.0 {
                return Err(PyValueError::new_err(format!(
                    "datetime offsets must be whole minutes {}",
                    walk.location()
                )));
            }
        }
    }
    let text: String = value.call_method0("isoformat")?.extract()?;
    text.parse().map(Some).map_err(|err| {
        PyValueError::new_err(format!("invalid datetime {}: {err}", walk.location()))
    })
}
