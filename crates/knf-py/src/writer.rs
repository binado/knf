use knf::format::emit;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::from_py::{dict_to_json, dict_to_toml};

/// Serialize a dict as JSON, TOML, or YAML with a trailing newline.
#[pyfunction]
#[pyo3(signature = (data, *, format, pretty = true))]
pub(super) fn dumps(data: &Bound<'_, PyDict>, format: &str, pretty: bool) -> PyResult<String> {
    match format {
        "json" => emit(dict_to_json(data)?, pretty),
        "toml" => emit(dict_to_toml(data)?, pretty),
        #[cfg(feature = "yaml")]
        "yaml" => {
            if !pretty {
                return Err(PyValueError::new_err("format='yaml' requires pretty=True"));
            }
            knf_convert::emit_yaml(&dict_to_json(data)?)
        }
        _ => {
            let message = if cfg!(feature = "yaml") {
                "format must be 'json', 'toml', or 'yaml'"
            } else {
                "format must be 'json' or 'toml'"
            };
            return Err(PyValueError::new_err(message));
        }
    }
    .map_err(|err| PyValueError::new_err(format!("{err:#}")))
}

/// Serialize a dict as TOML; see `__init__.pyi` for supported types and errors.
#[pyfunction]
#[pyo3(signature = (data, *, pretty = true))]
pub(super) fn dumps_toml(data: &Bound<'_, PyDict>, pretty: bool) -> PyResult<String> {
    dumps(data, "toml", pretty)
}
