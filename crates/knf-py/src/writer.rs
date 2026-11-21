use knf_convert::{InputFormat, OutputFormat, Policy};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::from_py::dict_to_value;

/// Serialize a dict as JSON, TOML, or YAML with a trailing newline.
#[pyfunction]
#[pyo3(signature = (data, *, format, pretty = true))]
pub(super) fn dumps(data: &Bound<'_, PyDict>, format: &str, pretty: bool) -> PyResult<String> {
    let (output, input) = match format {
        "json" => (OutputFormat::Json, InputFormat::Json),
        "toml" => (OutputFormat::Toml, InputFormat::Toml),
        #[cfg(feature = "yaml")]
        "yaml" => {
            if !pretty {
                return Err(PyValueError::new_err("format='yaml' requires pretty=True"));
            }
            (OutputFormat::Yaml, InputFormat::Json)
        }
        _ => {
            let message = if cfg!(feature = "yaml") {
                "format must be 'json', 'toml', or 'yaml'"
            } else {
                "format must be 'json' or 'toml'"
            };
            return Err(PyValueError::new_err(message));
        }
    };
    output
        .emit(&dict_to_value(data, input)?, Policy::Strict, pretty)
        .map(|result| result.value)
        .map_err(|err| PyValueError::new_err(format!("{err:#}")))
}

/// Serialize a dict as TOML; see `__init__.pyi` for supported types and errors.
#[pyfunction]
#[pyo3(signature = (data, *, pretty = true))]
pub(super) fn dumps_toml(data: &Bound<'_, PyDict>, pretty: bool) -> PyResult<String> {
    dumps(data, "toml", pretty)
}
