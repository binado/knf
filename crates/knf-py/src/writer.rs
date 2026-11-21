use knf_convert::{ConversionError, Layout, OutputFormat, Policy};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::from_py::dict_to_value;

/// Serialize a dict as JSON, TOML, or YAML with a trailing newline.
#[pyfunction]
#[pyo3(signature = (data, *, format, pretty = true))]
pub(super) fn dumps(data: &Bound<'_, PyDict>, format: &str, pretty: bool) -> PyResult<String> {
    let output = match format {
        "json" => OutputFormat::Json,
        "toml" => OutputFormat::Toml,
        #[cfg(feature = "yaml")]
        "yaml" => {
            if !pretty {
                return Err(PyValueError::new_err("format='yaml' requires pretty=True"));
            }
            OutputFormat::Yaml
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
    let layout = if pretty {
        Layout::Pretty
    } else {
        Layout::Compact
    };
    output
        .emit(&dict_to_value(data)?, Policy::Strict, layout)
        .map(|result| result.value)
        .map_err(|err| match err.downcast_ref::<ConversionError>() {
            // The format has no such type, as opposed to a number it cannot hold.
            Some(error) if error.kind() != "number" => PyTypeError::new_err(error.to_string()),
            _ => PyValueError::new_err(format!("{err:#}")),
        })
}

/// Serialize a dict as TOML; see `__init__.pyi` for supported types and errors.
#[pyfunction]
#[pyo3(signature = (data, *, pretty = true))]
pub(super) fn dumps_toml(data: &Bound<'_, PyDict>, pretty: bool) -> PyResult<String> {
    dumps(data, "toml", pretty)
}
