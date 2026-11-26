use knf_convert::{Emit, EmitError, Layout, OutputFormat, Policy, Unrepresentable};
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
        "yaml" => OutputFormat::Yaml,
        _ => {
            let message = if cfg!(feature = "yaml") {
                "format must be 'json', 'toml', or 'yaml'"
            } else {
                "format must be 'json' or 'toml'"
            };
            return Err(PyValueError::new_err(message));
        }
    };
    // Only YAML lacks a layout.
    let emit = Emit::new(output, Layout::pretty(pretty))
        .map_err(|_| PyValueError::new_err("format='yaml' requires pretty=True"))?;
    emit.emit(&dict_to_value(data, None)?, Policy::Strict)
        .map(|result| result.value)
        .map_err(|err| match err {
            EmitError::Conversion(error) if error.category() == Unrepresentable::Type => {
                PyTypeError::new_err(error.to_string())
            }
            err => PyValueError::new_err(crate::one_line(&err)),
        })
}

/// Serialize a dict as TOML; see `__init__.pyi` for supported types and errors.
#[pyfunction]
#[pyo3(signature = (data, *, pretty = true))]
pub(super) fn dumps_toml(data: &Bound<'_, PyDict>, pretty: bool) -> PyResult<String> {
    dumps(data, "toml", pretty)
}
