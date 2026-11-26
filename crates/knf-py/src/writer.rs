use knf_convert::{Emit, EmitError, Layout, OutputFormat, Policy, Unrepresentable};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::from_py::dict_to_value;

/// Serialize a dict as JSON, TOML, or YAML with a trailing newline.
///
/// TOML supports strings, booleans, signed 64-bit integers, floats, lists,
/// tuples, dicts with string keys, and datetime, date, and time objects.
/// Datetime offsets must be whole minutes and times must have no tzinfo.
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
