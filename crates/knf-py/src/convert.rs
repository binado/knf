//! [`Value`] ↔ Python objects.
//!
//! The two directions are not mirror images, and the asymmetry is the point.
//! **Python is the widest target in this workspace**: every emission path knf
//! has narrows — `to_toml` rejects nulls, integers past `i64::MAX` and
//! unparseable datetimes, `to_json` rejects non-finite floats — while Python
//! spells all four (`None`, arbitrary-precision `int`, `float('inf')`,
//! `datetime`). So [`value_to_py`] is *total*: nothing in a merged document has
//! to be refused on the way out, and the whole `TomlError`/`NonFiniteFloat`
//! family is unreachable from this crate.
//!
//! The rejection moves to the way *in*. [`map_from_py`] is where a Python object
//! meets the merge IR, and three things have no place in one: an `int` outside
//! `[i64::MIN, u64::MAX]`, a non-`str` mapping key, and anything the IR has no
//! variant for.

use knf::{Map, Number, Seg, Value, parse_datetime};
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyDate, PyDateTime, PyDelta, PyDict, PyFloat, PyInt, PyList, PyString, PyTime, PyTuple,
    PyTzInfo,
};

/// How deep a layer handed in from Python may nest.
///
/// A `dict` may contain itself; a recursive descent over one does not terminate.
/// Python's own recursion limit defaults to 1000 and CPython's `json` encoder
/// caps at the same place, so a config far below either is not a config anyone
/// wrote by hand — the cap turns a stack overflow (which aborts the interpreter)
/// into an exception the caller can catch.
const MAX_DEPTH: usize = 128;

// --- IR → Python ----------------------------------------------------------

/// Every [`Value`] has a Python spelling, so this cannot fail on the value; the
/// [`PyResult`] is the interpreter's, not the document's.
pub fn value_to_py<'py>(py: Python<'py>, value: Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match value {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, b).to_owned().into_any(),
        // `int` is arbitrary-precision, so `U64` needs no rejection here — the
        // one place in the workspace where that is true.
        Value::Number(Number::I64(i)) => i.into_pyobject(py)?.into_any(),
        Value::Number(Number::U64(u)) => u.into_pyobject(py)?.into_any(),
        // `inf` and `nan` are ordinary floats in Python, so the value `-f json`
        // has to refuse passes straight through.
        Value::Number(Number::F64(f)) => f.into_pyobject(py)?.into_any(),
        Value::String(s) => PyString::new(py, &s).into_any(),
        Value::Datetime(s) => datetime_to_py(py, &s)?,
        Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(value_to_py(py, item)?)?;
            }
            list.into_any()
        }
        Value::Object(map) => {
            // `Map` is an `IndexMap` and a `dict` preserves insertion order, so
            // the key order of the merged document survives into Python.
            let dict = PyDict::new(py);
            for (key, value) in map {
                dict.set_item(key, value_to_py(py, value)?)?;
            }
            dict.into_any()
        }
    })
}

/// A TOML datetime spelling as a real `datetime.datetime`, `date` or `time`,
/// matching what `tomllib.load` produces for the same document.
///
/// The decomposition is `knf-config`'s [`parse_datetime`] — the workspace's one
/// call into the TOML datetime grammar. Re-implementing the grammar here would
/// give this crate a second one, and two parsers agree only until one is edited.
///
/// A spelling that does not parse falls back to the string, which is exactly
/// what `to_json` does with a datetime. It is unreachable in practice: a
/// `Value::Datetime` only ever originates in the TOML parser, and this crate
/// cannot make one at all — [`value_from_py`] rejects a Python `datetime` rather
/// than synthesising a spelling from it.
fn datetime_to_py<'py>(py: Python<'py>, spelling: &str) -> PyResult<Bound<'py, PyAny>> {
    let Some(parts) = parse_datetime(spelling) else {
        return Ok(PyString::new(py, spelling).into_any());
    };

    // `timezone(timedelta(0))` is `timezone.utc` — CPython returns the singleton
    // — so `Z` lands on the same tzinfo `tomllib` gives it.
    let tz = parts
        .offset_minutes
        .map(|minutes| {
            let delta = PyDelta::new(py, 0, i32::from(minutes) * 60, 0, true)?;
            PyTzInfo::fixed_offset(py, delta)
        })
        .transpose()?;

    Ok(match (parts.date, parts.time) {
        (Some((year, month, day)), Some((hour, minute, second, nanosecond))) => PyDateTime::new(
            py,
            i32::from(year),
            month,
            day,
            hour,
            minute,
            second,
            // Truncating rather than rounding, as `tomllib` does: it keeps the
            // first six fractional digits and discards the rest.
            nanosecond / 1_000,
            tz.as_ref(),
        )?
        .into_any(),
        (Some((year, month, day)), None) => {
            PyDate::new(py, i32::from(year), month, day)?.into_any()
        }
        (None, Some((hour, minute, second, nanosecond))) => {
            PyTime::new(py, hour, minute, second, nanosecond / 1_000, tz.as_ref())?.into_any()
        }
        // Neither half: not one of TOML's four forms, so not something the
        // grammar above can produce. Falls back with the unparseable case.
        (None, None) => PyString::new(py, spelling).into_any(),
    })
}

// --- Python → IR ----------------------------------------------------------

/// Everything about one Python layer that has no place in the merge IR, each
/// with the path it sits at.
///
/// Grouped rather than listed in encounter order, and reported one group at a
/// time, because each group is a different Python exception: an oversized `int`
/// is an `OverflowError`, a bad key or a bad value a `TypeError`. The shape is
/// [`knf::value::to_toml`](knf::value::to_toml)'s — collect every
/// offender with its path, so a caller does not learn of them one run at a time.
#[derive(Debug, Default)]
pub struct ConversionErrors {
    /// Nesting past [`MAX_DEPTH`], which a self-referential container reaches.
    pub too_deep: Vec<Vec<Seg>>,
    /// A value whose type the IR has no variant for, with that type's name.
    pub types: Vec<(Vec<Seg>, String)>,
    /// A mapping key that is not a `str`, with its type's name. The path is the
    /// mapping's, since a key that is not a string cannot be a path segment.
    pub keys: Vec<(Vec<Seg>, String)>,
    /// An `int` outside `[i64::MIN, u64::MAX]`, with its decimal spelling.
    pub integers: Vec<(Vec<Seg>, String)>,
}

impl ConversionErrors {
    fn is_empty(&self) -> bool {
        self.too_deep.is_empty()
            && self.types.is_empty()
            && self.keys.is_empty()
            && self.integers.is_empty()
    }
}

/// One Python mapping as one merge layer.
///
/// A layer is an object, not a value — [`knf::MergeOpts::overlays`](knf::MergeOpts::overlays)
/// says so in its type, for the reason `format::parse` requires an object at the
/// top of every file.
pub fn map_from_py(obj: &Bound<'_, PyDict>) -> Result<Map, ConversionErrors> {
    let mut errors = ConversionErrors::default();
    let map = object_from_py(obj, &mut Vec::new(), 0, &mut errors);
    if errors.is_empty() {
        Ok(map)
    } else {
        Err(errors)
    }
}

/// Walks one value, collecting offenders and building the IR in the same pass.
///
/// Deliberately *not* the pre-walk-then-infallible-conversion split
/// `collect_untomlable` uses, and the reason that split exists is the reason it
/// does not carry over: there it keeps `to_toml_unchecked` free of a `Result`
/// threaded through its array and table arms. Here every step is a fallible
/// `extract` against a live interpreter no matter which pass it runs in, so a
/// second walk would buy no infallibility — only a second set of extractions,
/// and a set of `expect`s standing on the claim that the two walks agree.
///
/// What the split is actually *for* survives untouched: every offender is
/// collected with its path rather than the first one aborting the walk. Where
/// one is recorded the IR gets a `Null` placeholder, which is never observed —
/// the caller discards the whole layer as soon as the report is non-empty.
fn value_from_py(
    obj: &Bound<'_, PyAny>,
    path: &mut Vec<Seg>,
    depth: usize,
    errors: &mut ConversionErrors,
) -> Value {
    if obj.is_none() {
        return Value::Null;
    }
    // Before `int`, always: Python's `bool` is a subclass of `int`, so `True`
    // extracts as `1` and this test is the only thing keeping it a bool.
    if let Ok(b) = obj.cast::<PyBool>() {
        return Value::Bool(b.is_true());
    }
    if obj.is_instance_of::<PyInt>() {
        return match (obj.extract::<i64>(), obj.extract::<u64>()) {
            (Ok(i), _) => Value::Number(Number::I64(i)),
            // `from_u64` demotes, so this arm only ever yields a canonical
            // `U64`: a value past `i64::MAX`, which is exactly what the variant
            // is for.
            (_, Ok(u)) => Value::Number(Number::from_u64(u)),
            // Rejected, never truncated — the same rule `IntegerOutOfRange`
            // enforces one format over, and for the same reason: the digits are
            // the value.
            _ => {
                errors.integers.push((path.clone(), decimal_spelling(obj)));
                Value::Null
            }
        };
    }
    if obj.is_instance_of::<PyFloat>() {
        return match obj.extract::<f64>() {
            Ok(f) => Value::Number(Number::F64(f)),
            // A `PyFloat` is a C double, so this is unreachable — but a
            // substituted `nan` would be the silent corruption this whole file
            // is written to avoid, so it reports instead.
            Err(_) => {
                errors.types.push((path.clone(), type_name(obj)));
                Value::Null
            }
        };
    }
    if obj.cast::<PyString>().is_ok() {
        return match obj.extract::<String>() {
            Ok(s) => Value::String(s),
            // A `str` of unpaired surrogates has no UTF-8 encoding, so it has no
            // place in a document either. Named for what it is rather than by
            // its type, which would read as `str` and say nothing.
            Err(_) => {
                errors
                    .types
                    .push((path.clone(), "str with unpaired surrogates".to_string()));
                Value::Null
            }
        };
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        if depth >= MAX_DEPTH {
            errors.too_deep.push(path.clone());
            return Value::Null;
        }
        return Value::Object(object_from_py(dict, path, depth + 1, errors));
    }
    // A tuple as well as a list, which is what `json.dumps` accepts: both are
    // ordered sequences and an array is what either means.
    if let Ok(items) = obj.cast::<PyList>() {
        return sequence_from_py(items.iter(), path, depth, errors);
    }
    if let Ok(items) = obj.cast::<PyTuple>() {
        return sequence_from_py(items.iter(), path, depth, errors);
    }
    // Everything else: a `set`, a `Decimal`, an arbitrary object — and a
    // `datetime`, which is refused rather than converted. `Value::Datetime` may
    // only ever *originate* in the TOML parser: it carries a source spelling,
    // and this crate turning a Python `datetime` into one would be synthesising
    // that spelling from outside the grammar.
    errors.types.push((path.clone(), type_name(obj)));
    Value::Null
}

fn object_from_py(
    dict: &Bound<'_, PyDict>,
    path: &mut Vec<Seg>,
    depth: usize,
    errors: &mut ConversionErrors,
) -> Map {
    let mut map = Map::with_capacity(dict.len());
    for (key, value) in dict.iter() {
        // A key becomes a path segment, so it has to be text. `Seg::Key` is a
        // `String` and there is no honest coercion from an arbitrary object:
        // `{1: "a"}` would have to invent the key `"1"`, which is a different
        // document from the one the caller wrote.
        let Some(name) = string_key(&key) else {
            errors.keys.push((path.clone(), key_kind(&key)));
            continue;
        };
        let key = name;
        path.push(Seg::Key(key.clone()));
        let value = value_from_py(&value, path, depth, errors);
        path.pop();
        map.insert(key, value);
    }
    map
}

fn sequence_from_py<'py>(
    items: impl Iterator<Item = Bound<'py, PyAny>>,
    path: &mut Vec<Seg>,
    depth: usize,
    errors: &mut ConversionErrors,
) -> Value {
    if depth >= MAX_DEPTH {
        errors.too_deep.push(path.clone());
        return Value::Null;
    }
    let mut out = Vec::new();
    for (index, item) in items.enumerate() {
        path.push(Seg::Index(index));
        out.push(value_from_py(&item, path, depth + 1, errors));
        path.pop();
    }
    Value::Array(out)
}

/// The type of an object, for a message that has to say what arrived.
fn type_name(obj: &Bound<'_, PyAny>) -> String {
    obj.get_type()
        .name()
        .map(|name| name.to_string())
        .unwrap_or_else(|_| "?".to_string())
}

/// What was wrong with a key, for the message.
///
/// A `str` that downcast but did not extract is not "a `str`", which is what its
/// type would say and would leave a caller reading "keys must be str" about a
/// key that already is one.
fn key_kind(key: &Bound<'_, PyAny>) -> String {
    if key.cast::<PyString>().is_ok() {
        "str with unpaired surrogates".to_string()
    } else {
        type_name(key)
    }
}

/// A key as a `String`, or `None` if it is not one.
///
/// The extraction is part of the test rather than an assumption after it: a
/// `str` of unpaired surrogates downcasts fine and has no UTF-8 encoding, and a
/// key that cannot be encoded is as unusable as a key that is not a string.
fn string_key(key: &Bound<'_, PyAny>) -> Option<String> {
    key.cast::<PyString>().ok()?;
    key.extract::<String>().ok()
}

/// An integer's decimal spelling. Arbitrary-precision, so `str` rather than any
/// Rust integer type — the whole reason the value was rejected is that no Rust
/// integer holds it.
fn decimal_spelling(obj: &Bound<'_, PyAny>) -> String {
    obj.str()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "?".to_string())
}
