//! The exception hierarchy, and where a library error picks up this interface's
//! vocabulary.
//!
//! This file is `knf-py`'s [`explain.rs`]. The library crates raise errors that
//! carry key paths, reference spellings and file paths and never name the thing
//! the caller typed — `knf-core` and `knf-interp` have never heard of a flag,
//! and neither has heard of a keyword argument. `knf-cli` closes that gap with
//! `help:` lines naming `--append` and `-f`; this closes the same gap with
//! `rules=`, `input_format=`, `overlays=`, `paths=` and `env=`, and it is the
//! only place in this crate where those spellings appear in a message.
//!
//! [`explain.rs`]: https://github.com/binado/knf/blob/main/crates/knf/src/explain.rs
//!
//! # What is not here
//!
//! The whole emission-rejection family — `TomlError`, `NullInToml`,
//! `IntegerOutOfRange`, `BadDatetime`, `NonFiniteFloat` — has no exception,
//! because it has no way to be raised: `deep_merge` returns a `dict` and never
//! calls `format::emit`. Python spells every value those errors exist to refuse.
//! See [`crate::convert`].

use knf::{
    InterpError, LoadError as ConfigLoadError, MergeError as ConfigMergeError,
    PathError as ConfigPathError, Problem, RuleError as ConfigRuleError, RuleErrors, Seg,
    render_path,
};
use pyo3::exceptions::{PyException, PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use pyo3::{PyErr, create_exception};

create_exception!(
    _knf,
    KnfError,
    PyException,
    "Base class for everything knf raises from its own pipeline.\n\n\
     Failures in the *arguments* — a bad type, an int with no Rust spelling — \
     are the builtins a Python caller expects (`TypeError`, `ValueError`, \
     `OverflowError`) rather than subclasses of this."
);
create_exception!(
    _knf,
    LoadError,
    KnfError,
    "A path could not be turned into a layer.\n\n\
     `.path` is the offending file as a `str`, or `None` for stdin."
);
create_exception!(
    _knf,
    ParseError,
    KnfError,
    "A layer is not valid JSON or TOML."
);
create_exception!(
    _knf,
    MergeError,
    KnfError,
    "The fold rejected a layer.\n\n`.path` is the key path, as a tuple of keys."
);
create_exception!(
    _knf,
    RuleError,
    KnfError,
    "The rule set is not legal.\n\n\
     `.paths` is one key-path tuple per offending rule."
);
create_exception!(
    _knf,
    InterpolationError,
    KnfError,
    "A `${...}` reference could not be resolved.\n\n\
     `.problems` is a tuple of `(kind, path, detail)` triples, empty for a cycle."
);
create_exception!(
    _knf,
    PathError,
    KnfError,
    "A dotted key in `rules=` is not a legal path."
);

/// Registers the hierarchy on the extension module.
pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("KnfError", py.get_type::<KnfError>())?;
    module.add("LoadError", py.get_type::<LoadError>())?;
    module.add("ParseError", py.get_type::<ParseError>())?;
    module.add("MergeError", py.get_type::<MergeError>())?;
    module.add("RuleError", py.get_type::<RuleError>())?;
    module.add("InterpolationError", py.get_type::<InterpolationError>())?;
    module.add("PathError", py.get_type::<PathError>())?;
    Ok(())
}

// --- the pipeline's errors ------------------------------------------------

/// Turns an error out of `knf::merge_with_env` into the exception for it.
///
/// Downcasts rather than matching on a wrapper enum, because the pipeline's
/// errors arrive inside `anyhow` and each typed error becomes a different class.
/// Note the hazard, which `knf-cli` carries too: if `knf-config` ever wraps
/// these in one error type of its own, every downcast below starts missing and
/// nothing here fails to compile — the tests that pin these exception classes
/// are what would catch it.
pub fn pipeline_error(py: Python<'_>, err: anyhow::Error) -> PyErr {
    let err = match err.downcast::<ConfigLoadError>() {
        Ok(err) => return load_error(py, err),
        Err(err) => err,
    };
    let err = match err.downcast::<ConfigMergeError>() {
        Ok(err) => return merge_error(py, err),
        Err(err) => err,
    };
    let err = match err.downcast::<InterpError>() {
        Ok(err) => return interpolation_error(py, err),
        Err(err) => err,
    };
    // A file that is missing or unreadable is an `io::Error` under a context
    // line, not a typed pipeline error. It reaches Python as the builtin its
    // kind maps to — `FileNotFoundError`, `PermissionError` — because that is
    // what a caller writing `except` around a path argument already handles.
    // The message is anyhow's whole chain, so the context line survives.
    if let Some(io) = err.chain().find_map(|c| c.downcast_ref::<std::io::Error>()) {
        return PyErr::from(std::io::Error::new(io.kind(), format!("{err:#}")));
    }
    // Everything left is a parse failure: `format::parse` is anyhow-shaped and
    // reports the source name and the format crate's own message.
    //
    // The emission family cannot arrive here at all — `deep_merge` never emits —
    // so unlike `knf-cli`'s `explain_pipeline` there is nothing below this line.
    ParseError::new_err(format!("{err:#}"))
}

/// `knf-config` states the problem — stdin has no extension, this file's says
/// nothing, this path is a directory — and stops. Naming the keyword that would
/// settle it is ours.
fn load_error(py: Python<'_>, err: ConfigLoadError) -> PyErr {
    let hint = match &err {
        ConfigLoadError::StdinNeedsFormat | ConfigLoadError::UnknownExtension { .. } => {
            "; pass input_format=\"json\" or input_format=\"toml\""
        }
        ConfigLoadError::Directory { .. } => "; pass the files inside it as separate layers",
    };
    let path = err
        .path()
        .map(|p| p.display().to_string())
        .into_pyobject(py)
        .map(Bound::into_any);
    let exception = LoadError::new_err(format!("{err}{hint}"));
    with_attr(py, exception, "path", path)
}

/// The same division of labour the CLI keeps: `knf-core` renders the key path,
/// the interface names the knob that carried the rule.
fn merge_error(py: Python<'_>, err: ConfigMergeError) -> PyErr {
    let hint = match &err {
        ConfigMergeError::Locked { .. } => {
            "\nhint: rules={\"…\": \"fail\"} pins a path to the first layer that sets it"
        }
        ConfigMergeError::AppendKind { .. } => {
            "\nhint: rules={\"…\": \"append\"} needs an array on both sides"
        }
        ConfigMergeError::TypeConflict { .. } => "",
    };
    let path = keys_tuple(py, err.path());
    let exception = MergeError::new_err(format!("{err}{hint}"));
    with_attr(py, exception, "path", path)
}

/// A rule set rejected before any file is read.
///
/// Only [`ConfigRuleError::Unreachable`] can be spelled from here:
/// [`Conflict`](ConfigRuleError::Conflict) needs one path named twice with two
/// strategies, and `rules=` is a mapping, whose keys are unique. The conflict
/// arm is kept so that a future `rules=` spelling that *can* express one does
/// not silently lose its message.
pub fn rule_errors(py: Python<'_>, errors: RuleErrors) -> PyErr {
    let mut hint = String::new();
    if errors
        .errors()
        .iter()
        .any(|e| matches!(e, ConfigRuleError::Unreachable { .. }))
    {
        hint.push_str(
            "\nhint: every strategy takes the whole value at its path, \
             so a rule in rules= below another can never fire",
        );
    }
    if errors
        .errors()
        .iter()
        .any(|e| matches!(e, ConfigRuleError::Conflict { .. }))
    {
        hint.push_str("\nhint: a path may be given only one strategy in rules=");
    }
    let paths = errors
        .errors()
        .iter()
        .map(|e| keys_tuple(py, e.path()))
        .collect::<PyResult<Vec<_>>>()
        .and_then(|paths| PyTuple::new(py, paths))
        .map(Bound::into_any);
    let exception = RuleError::new_err(format!("{errors}{hint}"));
    with_attr(py, exception, "paths", paths)
}

/// A dotted key in `rules=` that is not a path.
pub fn rule_path_error(err: ConfigPathError) -> PyErr {
    match err {
        ConfigPathError::IndexInKeyPath { .. } => PathError::new_err(format!(
            "{err}\nhint: a key in rules= is a dotted key path; \
             an index like servers[0] can be read by a ${{...}} reference but never written"
        )),
        other => PathError::new_err(other.to_string()),
    }
}

/// `knf-interp` names key paths and reference spellings and has never heard of
/// `interpolate=`.
fn interpolation_error(py: Python<'_>, err: InterpError) -> PyErr {
    let problems = match &err {
        // A cycle stops resolution, so it arrives alone and there is no list to
        // hand over. `.problems` is an empty tuple rather than absent, so a
        // caller reading the attribute never has to test for it.
        InterpError::Cycle(_) => PyTuple::empty(py).into_any(),
        InterpError::Problems(problems) => {
            match PyTuple::new(py, problems.iter().map(|p| problem_triple(py, p))) {
                Ok(tuple) => tuple.into_any(),
                Err(e) => return e,
            }
        }
    };
    let exception = InterpolationError::new_err(format!(
        "{err}\nhint: references resolve only under interpolate=True, \
         and `${{env:NAME}}` reads env= (or the process environment)"
    ));
    with_attr(py, exception, "problems", PyResult::Ok(problems))
}

/// One problem as `(kind, path, detail)`.
///
/// Three strings rather than a class: the kinds are `knf-interp`'s to name and
/// adding one must not require a new Python type on this side.
fn problem_triple<'py>(py: Python<'py>, problem: &Problem) -> Bound<'py, PyAny> {
    let (kind, detail) = match problem {
        Problem::Syntax { error, .. } => ("syntax", error.to_string()),
        Problem::Unresolved { reference, .. } => ("unresolved", reference.clone()),
        Problem::NotStringifiable {
            reference, kind, ..
        } => ("not-stringifiable", format!("{reference} is {kind}")),
    };
    let triple = (kind, render_path(problem.path()), detail);
    // A tuple of three `str`s; the conversion has nothing that can fail.
    match triple.into_pyobject(py) {
        Ok(obj) => obj.into_any(),
        Err(_) => py.None().into_bound(py),
    }
}

// --- this interface's own arguments ---------------------------------------

/// `input_format=` took something other than the two formats there are.
pub fn bad_input_format(given: &str) -> PyErr {
    PyValueError::new_err(format!(
        "input_format= must be \"json\" or \"toml\", not {given:?}"
    ))
}

/// `paths=` was handed a lone string.
///
/// Called out by name because a `str` *is* iterable: without this it would
/// silently become one layer per character, and the first failure the caller
/// saw would be about a file named `b`.
pub fn paths_is_a_string(kind: &str) -> PyErr {
    PyTypeError::new_err(format!(
        "paths= takes an iterable of paths, and a bare {kind} would be read one \
         character at a time; pass a list even for one file"
    ))
}

/// `paths=` was not iterable at all.
pub fn paths_not_iterable(kind: &str) -> PyErr {
    PyTypeError::new_err(format!("paths= takes an iterable of paths, not {kind}"))
}

/// One element of `paths=` was not a path.
pub fn bad_path_item(index: usize, kind: &str) -> PyErr {
    PyTypeError::new_err(format!(
        "paths[{index}] must be a str or os.PathLike, not {kind}"
    ))
}

/// `rules=` named a strategy that does not exist.
pub fn bad_strategy(key: &str, given: &str) -> PyErr {
    PyValueError::new_err(format!(
        "rules[{key:?}] must be \"append\", \"replace\" or \"fail\", not {given:?}"
    ))
}

/// `rules=` or `env=` was not a mapping of strings to strings.
pub fn bad_mapping(keyword: &str, detail: &str) -> PyErr {
    PyTypeError::new_err(format!("{keyword}= {detail}"))
}

/// One overlay's report, as the exception for its first non-empty group.
///
/// One group at a time because each is a different builtin, and in this order
/// because it is how much the rest of the report is worth reading: past
/// [`MAX_DEPTH`](crate::convert) the walk stopped early and everything below is
/// missing, an unsupported type is the mistake a caller actually makes, and an
/// oversized `int` is the one that needs a genuinely enormous number.
pub fn overlay_errors(index: usize, errors: crate::convert::ConversionErrors) -> PyErr {
    let at = |path: &[Seg]| {
        let rendered = render_path(path);
        if rendered.is_empty() {
            format!("overlays[{index}]")
        } else {
            format!("overlays[{index}].{rendered}")
        }
    };
    let report = |summary: &str, entries: Vec<String>| {
        format!("{summary}\n  --> {}", entries.join("\n  --> "))
    };

    if !errors.too_deep.is_empty() {
        return PyValueError::new_err(report(
            "nesting is too deep to convert (a container may contain itself)",
            errors.too_deep.iter().map(|p| at(p)).collect(),
        ));
    }
    if !errors.types.is_empty() {
        return PyTypeError::new_err(report(
            "a layer takes None, bool, int, float, str, list and dict only",
            errors
                .types
                .iter()
                .map(|(p, kind)| format!("{}: {kind}", at(p)))
                .collect(),
        ));
    }
    if !errors.keys.is_empty() {
        return PyTypeError::new_err(report(
            "a layer's keys must be str",
            errors
                .keys
                .iter()
                .map(|(p, kind)| format!("{}: {kind}", at(p)))
                .collect(),
        ));
    }
    PyOverflowError::new_err(report(
        "an integer outside [-2**63, 2**64) has no place in a document",
        errors
            .integers
            .iter()
            .map(|(p, value)| format!("{}: {value}", at(p)))
            .collect(),
    ))
}

/// `overlays=` was not iterable at all.
pub fn overlays_not_iterable(kind: &str) -> PyErr {
    PyTypeError::new_err(format!("overlays= takes an iterable of dicts, not {kind}"))
}

/// An overlay that is not a mapping at all.
pub fn bad_overlay(index: usize, kind: &str) -> PyErr {
    PyTypeError::new_err(format!(
        "overlays[{index}] must be a dict, not {kind}; a layer is an object, \
         the same rule every input file follows"
    ))
}

// --- attaching the structured half ----------------------------------------

/// Hangs one attribute off an exception instance.
///
/// The message says what went wrong in prose; the attribute is the same fact in
/// a shape a caller can branch on, which is what keeps `except LoadError` from
/// having to parse a string.
fn with_attr<'py>(
    py: Python<'py>,
    err: PyErr,
    name: &str,
    value: Result<Bound<'py, PyAny>, impl Into<PyErr>>,
) -> PyErr {
    let value = match value {
        Ok(value) => value,
        Err(e) => return e.into(),
    };
    if let Err(e) = err.value(py).setattr(name, value) {
        return e;
    }
    err
}

/// A key path as a tuple of keys.
///
/// Structured rather than the dotted rendering, because a key may contain a dot
/// and joining would make such a path ambiguous — the same reason `--set` cannot
/// address one. The dotted spelling is already in the message.
fn keys_tuple<'py>(py: Python<'py>, keys: &[String]) -> PyResult<Bound<'py, PyAny>> {
    PyTuple::new(py, keys).map(Bound::into_any)
}
