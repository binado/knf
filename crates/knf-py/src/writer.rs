use knf::format::emit;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::from_py::{dict_to_json, dict_to_toml};

/// Serialize a dict as JSON or TOML with a trailing newline.
#[pyfunction]
#[pyo3(signature = (data, *, format, pretty = true))]
pub(super) fn dumps(data: &Bound<'_, PyDict>, format: &str, pretty: bool) -> PyResult<String> {
    match format {
        "json" => emit(dict_to_json(data)?, pretty),
        "toml" => emit(dict_to_toml(data)?, pretty),
        _ => return Err(PyValueError::new_err("format must be 'json' or 'toml'")),
    }
    .map_err(|err| PyValueError::new_err(format!("{err:#}")))
}

/// Serialize a dict as TOML; see `__init__.pyi` for supported types and errors.
#[pyfunction]
#[pyo3(signature = (data, *, pretty = true))]
pub(super) fn dumps_toml(data: &Bound<'_, PyDict>, pretty: bool) -> PyResult<String> {
    dumps(data, "toml", pretty)
}
