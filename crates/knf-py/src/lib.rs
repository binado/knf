//! `knf.deep_merge` — the `knf-config` pipeline as a Python function.
//!
//! A frontend, sibling to `knf-cli`: the pipeline is `knf-config`, and this
//! crate is keyword arguments and exceptions the way that one is argv and
//! stderr. Neither has a library target anyone depends on, and neither knows
//! anything the other does — no clap here, no pyo3 there.
//!
//! Everything a caller typed is validated before a single file is opened, in
//! the order [`knf-cli`'s `run`] uses and for the same reason: a mistake in
//! `rules=` must not queue behind a missing file, or the caller fixes the path,
//! re-runs, and only then learns about the rule.
//!
//! [`knf-cli`'s `run`]: https://github.com/binado/knf/blob/main/crates/knf/src/main.rs

mod convert;
mod errors;

use std::collections::HashMap;
use std::path::PathBuf;

use knf::{
    Env, EnvValue, Format, Map, MergeOpts, ProcessEnv, RefPath, Rules, Strategy, json_or_string,
    merge_with_env, value,
};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyString};

/// Merge layered JSON and TOML configuration files into one `dict`.
///
/// Files are layers, merged left to right in argument order; JSON and TOML mix
/// freely. Objects merge key by key, while arrays, scalars and `None` all
/// replace wholesale — `None` is an ordinary value that overwrites, not a
/// delete instruction. With one path and no options this is the identity.
///
/// Arguments:
///     paths: Iterable of `str` or `os.PathLike`. `"-"` reads stdin, and needs
///         an explicit `input_format` because stdin has no extension. A bare
///         `str` is rejected: it is iterable, and would become one layer per
///         character.
///     input_format: `"json"` or `"toml"`, treating every input as that format
///         instead of inferring from extensions.
///     strict: Error when a layer changes the kind of an existing key.
///     rules: `{"key.path": "append" | "replace" | "fail"}`. Every strategy
///         takes the whole value at its path and does not recurse, so no rule
///         may sit beneath another; the set is validated before any file is
///         opened, and its order never affects the result.
///     overlays: Dicts appended after every file, in order, and merged as
///         ordinary terminal layers.
///     interpolate: Resolve `${key.path}` and `${env:VAR}` once over the merged
///         document. Off by default, since knf sits upstream of tools whose own
///         syntax is `${...}`.
///     env: What `${env:VAR}` reads. `None` is the process environment; a
///         mapping is snapshotted, and then the result is a function of the
///         arguments alone.
///
/// Returns:
///     The merged document. `null` is `None`, integers are arbitrary precision,
///     `inf` and `nan` survive, and TOML datetimes arrive as `datetime.datetime`,
///     `date` or `time` — Python spells everything knf can hold, so nothing is
///     refused on the way out.
///
/// Raises:
///     KnfError: Or one of its subclasses, for anything wrong with the
///         documents: LoadError, ParseError, MergeError, RuleError,
///         InterpolationError, PathError.
///     TypeError, ValueError, OverflowError: For anything wrong with the
///         arguments, including a value in `overlays` that has no place in a
///         document.
///     OSError: For a path that cannot be read, as FileNotFoundError and friends.
#[pyfunction]
#[pyo3(
    signature = (
        paths,
        *,
        input_format = None,
        strict = false,
        rules = None,
        overlays = None,
        interpolate = false,
        env = None,
    ),
    text_signature = "(paths, *, input_format=None, strict=False, rules=None, \
                      overlays=(), interpolate=False, env=None)"
)]
#[allow(clippy::too_many_arguments)]
fn deep_merge<'py>(
    py: Python<'py>,
    paths: &Bound<'py, PyAny>,
    input_format: Option<&str>,
    strict: bool,
    rules: Option<&Bound<'py, PyAny>>,
    overlays: Option<&Bound<'py, PyAny>>,
    interpolate: bool,
    env: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    // Before anything is read. The whole rule set is validated at once, every
    // overlay is converted, and the environment is snapshotted — all of it from
    // the arguments alone, so nothing about the files can change whether it is
    // legal.
    let opts = MergeOpts {
        input_format: input_format.map(parse_format).transpose()?,
        strict,
        rules: parse_rules(py, rules)?,
        overlays: parse_overlays(overlays)?,
        interpolate,
    };
    let paths = parse_paths(paths)?;
    let environment = parse_env(env)?;

    // Everything past here is Rust and file I/O, so the interpreter is free for
    // the duration. (`detach` is 0.29's name for what was `allow_threads`.)
    let merged = py
        .detach(move || merge_with_env(&paths, opts, &environment))
        .map_err(|err| errors::pipeline_error(py, err))?;

    // Total, and that is the headline: Python is the widest target in this
    // workspace, so nothing in the merged document has to be refused on the way
    // out and `format::emit` is never called. See [`convert`].
    convert::value_to_py(py, merged)
}

/// The two formats there are.
///
/// A local match rather than a `FromStr` on [`Format`], which is `knf-config`'s
/// type and has no reason to learn this crate's spelling — the same shape, and
/// the same orphan-rule reason, as `FormatArg` in `crates/knf/src/cli.rs`.
fn parse_format(given: &str) -> PyResult<Format> {
    match given {
        "json" => Ok(Format::Json),
        "toml" => Ok(Format::Toml),
        other => Err(errors::bad_input_format(other)),
    }
}

/// `{"plugins": "append"}` as a validated rule set.
///
/// A mapping rather than three separate arguments, and that choice removes a
/// whole error: keys are unique, so one path can never carry two strategies and
/// `RuleError::Conflict` is not expressible. Only `Unreachable` survives.
fn parse_rules(py: Python<'_>, rules: Option<&Bound<'_, PyAny>>) -> PyResult<Rules> {
    let Some(rules) = rules else {
        return Ok(Rules::EMPTY);
    };
    let rules = as_dict(rules, "rules", "takes a mapping of key path to strategy")?;

    let mut parsed: Vec<(Vec<String>, Strategy)> = Vec::with_capacity(rules.len());
    for (key, strategy) in rules.iter() {
        let key: String = key
            .extract()
            .map_err(|_| errors::bad_mapping("rules", "keys must be str key paths"))?;
        let strategy: String = strategy.extract().map_err(|_| {
            errors::bad_mapping(
                "rules",
                "values must be \"append\", \"replace\" or \"fail\"",
            )
        })?;

        // The one write-side predicate, at the boundary and before any I/O,
        // exactly where `merge_opts` runs it per flag: a rule names keys, never
        // an array element.
        let path: RefPath = key.parse().map_err(errors::rule_path_error)?;
        let keys = path.try_into_keys().map_err(errors::rule_path_error)?;

        // `Strategy` has a `Display` but no `FromStr`; the core names strategies
        // and leaves every interface to choose how they are spelled.
        let strategy = match strategy.as_str() {
            "append" => Strategy::Append,
            "replace" => Strategy::Replace,
            "fail" => Strategy::Fail,
            other => return Err(errors::bad_strategy(&key, other)),
        };
        parsed.push((keys, strategy));
    }

    Rules::build(parsed).map_err(|errs| errors::rule_errors(py, errs))
}

/// In-memory layers, appended after every file in order.
fn parse_overlays(overlays: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Map>> {
    let Some(overlays) = overlays else {
        return Ok(Vec::new());
    };
    let items = overlays
        .try_iter()
        .map_err(|_| errors::overlays_not_iterable(&type_name(overlays)))?;

    let mut parsed = Vec::new();
    for (index, overlay) in items.enumerate() {
        let overlay = overlay?;
        let dict = overlay
            .cast::<PyDict>()
            .map_err(|_| errors::bad_overlay(index, &type_name(&overlay)))?;
        parsed
            .push(convert::map_from_py(dict).map_err(|errs| errors::overlay_errors(index, errs))?);
    }
    Ok(parsed)
}

/// The positional layers.
///
/// Any iterable of anything `os.fspath` accepts, which is what pyo3's `PathBuf`
/// extractor already honours. `"-"` still reads stdin, and still needs
/// `input_format=` — stdin has no extension whatever the interface is.
fn parse_paths(paths: &Bound<'_, PyAny>) -> PyResult<Vec<PathBuf>> {
    if paths.cast::<PyString>().is_ok() || paths.cast::<PyBytes>().is_ok() {
        return Err(errors::paths_is_a_string(&type_name(paths)));
    }
    let items = paths
        .try_iter()
        .map_err(|_| errors::paths_not_iterable(&type_name(paths)))?;

    let mut parsed = Vec::new();
    for (index, item) in items.enumerate() {
        let item = item?;
        parsed.push(
            item.extract::<PathBuf>()
                .map_err(|_| errors::bad_path_item(index, &type_name(&item)))?,
        );
    }
    Ok(parsed)
}

/// Where `${env:NAME}` reads from.
///
/// `None` is the process environment. A mapping is **snapshotted** — copied
/// into an owned map before the merge starts — which is what lets the merge run
/// with the interpreter released, and what makes the output a function of the
/// arguments alone.
fn parse_env(env: Option<&Bound<'_, PyAny>>) -> PyResult<Environment> {
    let Some(env) = env else {
        return Ok(Environment::Process(ProcessEnv));
    };
    let env = as_dict(env, "env", "takes a mapping of variable name to value")?;

    let mut snapshot = HashMap::with_capacity(env.len());
    for (name, raw) in env.iter() {
        let name: String = name
            .extract()
            .map_err(|_| errors::bad_mapping("env", "keys must be str"))?;
        let raw: String = raw
            .extract()
            .map_err(|_| errors::bad_mapping("env", "values must be str"))?;
        // The same typing rule `ProcessEnv` uses, from the same function, so
        // `${env:PORT}` is a number whether it came from a dict or from the
        // process. Going through JSON is also why this can never fabricate a
        // `Datetime`: JSON has no such type.
        let typed = value::from_json(json_or_string(raw.clone()));
        snapshot.insert(name, EnvValue { raw, typed });
    }
    Ok(Environment::Snapshot(snapshot))
}

/// The environment, in the one shape [`merge_with_env`] takes.
///
/// An enum rather than a boxed trait object because the closure handed to
/// `Python::detach` must be `Send`, and `Box<dyn Env>` is not.
enum Environment {
    Process(ProcessEnv),
    Snapshot(HashMap<String, EnvValue>),
}

impl Env for Environment {
    fn lookup(&self, name: &str) -> Option<EnvValue> {
        match self {
            Self::Process(process) => process.lookup(name),
            Self::Snapshot(snapshot) => snapshot.get(name).cloned(),
        }
    }
}

/// A mapping-shaped argument as a `dict`.
///
/// `dict(x)` rather than a downcast, so an `OrderedDict`, a `TypedDict`, a
/// `MappingProxyType` or anything else `Mapping` all work. Only the two *option*
/// arguments go through this; an overlay is data and must be an actual `dict`,
/// since coercing there would quietly accept a list of pairs as a layer.
fn as_dict<'py>(
    obj: &Bound<'py, PyAny>,
    keyword: &str,
    detail: &str,
) -> PyResult<Bound<'py, PyDict>> {
    if let Ok(dict) = obj.cast::<PyDict>() {
        return Ok(dict.clone());
    }
    obj.py()
        .get_type::<PyDict>()
        .call1((obj,))
        .and_then(|d| Ok(d.cast_into::<PyDict>()?))
        .map_err(|_| errors::bad_mapping(keyword, detail))
}

fn type_name(obj: &Bound<'_, PyAny>) -> String {
    obj.get_type()
        .name()
        .map(|name| name.to_string())
        .unwrap_or_else(|_| "?".to_string())
}

/// The extension module. `_knf` rather than `knf`: the importable package is the
/// `python/knf` wrapper, which is what gives the distribution a `py.typed` and a
/// place for the stubs.
#[pymodule]
fn _knf(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(deep_merge, module)?)?;
    errors::register(module)?;
    Ok(())
}
