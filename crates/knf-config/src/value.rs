//! Conversion between the merge IR and the native JSON/TOML value types.
//!
//! Merge runs on [`knf_core::Value`], so these fire exactly twice per run: once
//! per layer on the way in, once on the whole document on the way out. Free
//! functions rather than `From`/`TryFrom` impls because both sides are foreign
//! types — `impl From<toml::Value> for knf_core::Value` names nothing local and
//! does not compile.

use std::fmt;

use knf_core::{Map, Number, Seg, Value, render_path};

/// JSON → IR. Total: every JSON value has an IR counterpart.
pub fn from_json(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(number_from_json(&n)),
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(items) => Value::Array(items.into_iter().map(from_json).collect()),
        serde_json::Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, from_json(v))).collect())
        }
    }
}

/// JSON object → IR object.
///
/// A layer is a map, not a value — [`crate::MergeOpts::overlays`] says so in its
/// type — so a caller holding a `serde_json` object needs this rather than
/// [`from_json`] to build one.
pub fn object_from_json(map: serde_json::Map<String, serde_json::Value>) -> Map {
    map.into_iter().map(|(k, v)| (k, from_json(v))).collect()
}

/// IR → JSON, rejecting up front the one thing JSON cannot hold.
///
/// One impossibility against [`to_toml`]'s three, and the same pre-walk shape for
/// the same reasons: a non-finite float. TOML's number grammar has `inf`, `-inf`
/// and `nan` literals, so an ordinary `.toml` input hands this function an
/// infinity, and `serde_json::Number::from_f64` refuses it. That refusal used to
/// be swallowed — `timeout = inf` emitted `{"timeout":0}` — which is the same
/// silent substitution the null walk exists to prevent, one format over.
///
/// A datetime is not an impossibility here: JSON has no such type, so it renders
/// as the string JSON would have to use anyway.
pub fn to_json(value: Value) -> Result<serde_json::Value, NonFiniteFloat> {
    let mut nonfinite = Vec::new();
    collect_unjsonable(&value, &mut Vec::new(), &mut nonfinite);
    if !nonfinite.is_empty() {
        return Err(NonFiniteFloat { entries: nonfinite });
    }
    Ok(to_json_unchecked(value))
}

fn to_json_unchecked(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::Number(n) => serde_json::Value::Number(number_to_json(n)),
        Value::String(s) | Value::Datetime(s) => serde_json::Value::String(s),
        Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(to_json_unchecked).collect())
        }
        Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, to_json_unchecked(v)))
                .collect(),
        ),
    }
}

/// TOML → IR. Total: datetimes keep their source spelling rather than becoming
/// strings, which is what lets a TOML datetime survive a mixed-format merge.
pub fn from_toml(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(Number::I64(i)),
        toml::Value::Float(f) => Value::Number(Number::F64(f)),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(dt) => Value::Datetime(dt.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(from_toml).collect()),
        toml::Value::Table(table) => {
            Value::Object(table.into_iter().map(|(k, v)| (k, from_toml(v))).collect())
        }
    }
}

/// IR → TOML, rejecting up front what TOML cannot hold.
///
/// Three impossibilities, one walk. A null is the one users meet most, and its
/// check is separate because serde's own message ("unsupported None value")
/// carries no key path, and `toml`'s map serializer *skips* a `None` entry rather
/// than failing — so walking up front is the only way to surface it at all, let
/// alone with paths.
///
/// An integer above `i64::MAX` is the second. TOML integers are signed 64-bit, so
/// a JSON snowflake ID has no TOML spelling at all; it used to be rounded through
/// `f64`, which is the very loss [`Number::U64`] exists to prevent.
///
/// A [`Value::Datetime`] whose spelling does not re-parse is the third. All three
/// ride along here rather than failing inside the conversion for the same two
/// reasons: the messages want the key path this walk already carries, and checking
/// here keeps `to_toml_unchecked` infallible, so no `Result` has to be threaded
/// through its array and table arms.
///
/// Reported in the order a *document* can reach them. Nulls and out-of-range
/// integers both arrive from a real input file; only a caller that hand-built a
/// `Value` can produce a malformed datetime, so it goes last.
pub fn to_toml(value: Value) -> Result<toml::Value, TomlError> {
    let mut nulls = Vec::new();
    let mut integers = Vec::new();
    let mut datetimes = Vec::new();
    collect_untomlable(
        &value,
        &mut Vec::new(),
        &mut nulls,
        &mut integers,
        &mut datetimes,
    );
    if !nulls.is_empty() {
        return Err(TomlError::Null(NullInToml { entries: nulls }));
    }
    if !integers.is_empty() {
        return Err(TomlError::Integer(IntegerOutOfRange { entries: integers }));
    }
    if !datetimes.is_empty() {
        return Err(TomlError::Datetime(BadDatetime { entries: datetimes }));
    }
    Ok(to_toml_unchecked(value))
}

fn to_toml_unchecked(value: Value) -> toml::Value {
    match value {
        Value::Null => {
            unreachable!("nulls are rejected by to_toml before conversion");
        }
        Value::Bool(b) => toml::Value::Boolean(b),
        Value::Number(n) => number_to_toml(n),
        Value::String(s) => toml::Value::String(s),
        // Guarded by `to_toml`, exactly as the `Null` arm above is. In a document
        // that came from a parser it could not fail at all: every `Datetime`
        // *originates* in the TOML parser, from a string `toml` itself printed,
        // and while interpolation may copy one (`d2 = "${d}"` takes the referent's
        // type), nothing anywhere synthesizes one from text — `${env:...}` types
        // through JSON, which has no datetime and so structurally cannot. A caller
        // hand-building a `Value` can still spell one wrongly, and that is what the
        // pre-walk catches.
        Value::Datetime(s) => toml::Value::Datetime(
            s.parse()
                .expect("unparseable datetimes are rejected by to_toml before conversion"),
        ),
        Value::Array(items) => {
            toml::Value::Array(items.into_iter().map(to_toml_unchecked).collect())
        }
        Value::Object(map) => {
            let mut table = toml::Table::new();
            for (k, v) in map {
                table.insert(k, to_toml_unchecked(v));
            }
            toml::Value::Table(table)
        }
    }
}

fn number_from_json(n: &serde_json::Number) -> Number {
    if let Some(i) = n.as_i64() {
        Number::I64(i)
    } else if let Some(u) = n.as_u64() {
        Number::from_u64(u)
    } else if let Some(f) = n.as_f64() {
        Number::F64(f)
    } else {
        // Unreachable as this workspace is built: without serde_json's
        // `arbitrary_precision` feature a `Number` is exactly one of i64/u64/f64,
        // so one of the three arms above always takes it. Nothing here enables
        // that feature, but cargo unifies features across a whole graph, so a
        // downstream crate could switch it on from outside — hence a fallback
        // rather than a panic in a library.
        Number::F64(0.0)
    }
}

fn number_to_json(n: Number) -> serde_json::Number {
    match n {
        Number::I64(i) => i.into(),
        Number::U64(u) => u.into(),
        // Guarded by `to_json`, exactly as the arms in `to_toml_unchecked` are.
        // `from_f64` returns `None` only for inf and NaN, and both are collected
        // by the pre-walk before this runs.
        Number::F64(f) => serde_json::Number::from_f64(f)
            .expect("non-finite floats are rejected by to_json before conversion"),
    }
}

fn number_to_toml(n: Number) -> toml::Value {
    match n {
        Number::I64(i) => toml::Value::Integer(i),
        // TOML integers are signed, so anything past i64::MAX has no TOML
        // spelling. It used to become a float, which silently discarded the exact
        // digits `Number::U64` exists to preserve; `to_toml`'s pre-walk rejects it
        // instead, leaving this conversion for the values that do fit. Only a
        // hand-built `Number::U64` gets here at all — `Number::from_u64` demotes
        // anything representable to `I64` — and the pre-walk lets exactly those
        // through.
        Number::U64(u) => toml::Value::Integer(
            i64::try_from(u)
                .expect("out-of-range integers are rejected by to_toml before conversion"),
        ),
        Number::F64(f) => toml::Value::Float(f),
    }
}

// --- what TOML cannot hold ------------------------------------------------

/// One walk for all three impossibilities, each collected with the path it sits at.
fn collect_untomlable(
    value: &Value,
    cur: &mut Vec<Seg>,
    nulls: &mut Vec<Vec<Seg>>,
    integers: &mut Vec<(Vec<Seg>, u64)>,
    datetimes: &mut Vec<(Vec<Seg>, String)>,
) {
    match value {
        Value::Null => nulls.push(cur.clone()),
        // A guard rather than an arm, for the same reason as the datetime below:
        // a `U64` small enough for an `i64` converts perfectly well, and only a
        // hand-built value is ever spelled that way, since `Number::from_u64`
        // demotes. Every `U64` a *parser* produces fails this `try_from`.
        Value::Number(Number::U64(u)) if i64::try_from(*u).is_err() => {
            integers.push((cur.clone(), *u));
        }
        // A guard rather than an arm, so a datetime that parses — every datetime
        // a parsed document can contain — falls through to the `_` below.
        Value::Datetime(s) if s.parse::<toml::value::Datetime>().is_err() => {
            datetimes.push((cur.clone(), s.clone()));
        }
        Value::Object(obj) => {
            for (k, v) in obj {
                cur.push(Seg::Key(k.clone()));
                collect_untomlable(v, cur, nulls, integers, datetimes);
                cur.pop();
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                cur.push(Seg::Index(i));
                collect_untomlable(v, cur, nulls, integers, datetimes);
                cur.pop();
            }
        }
        _ => {}
    }
}

// --- what JSON cannot hold ------------------------------------------------

/// The mirror of [`collect_untomlable`], with one kind to find rather than three.
fn collect_unjsonable(value: &Value, cur: &mut Vec<Seg>, nonfinite: &mut Vec<(Vec<Seg>, f64)>) {
    match value {
        Value::Number(Number::F64(f)) if !f.is_finite() => nonfinite.push((cur.clone(), *f)),
        Value::Object(obj) => {
            for (k, v) in obj {
                cur.push(Seg::Key(k.clone()));
                collect_unjsonable(v, cur, nonfinite);
                cur.pop();
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                cur.push(Seg::Index(i));
                collect_unjsonable(v, cur, nonfinite);
                cur.pop();
            }
        }
        _ => {}
    }
}

/// The TOML spellings, which are the only text a non-finite float has anywhere in
/// this workspace — and the spelling such a value arrived as, since TOML is the
/// only format that can carry one in.
///
/// Three lines duplicated from `knf-interp`'s private `render::float` rather than
/// shared: reaching into that crate would mean widening its public surface for a
/// display detail, and `knf-config` already depends on it only through the
/// pipeline. Keep the two in step.
///
/// Only non-finite values reach here, so the sign test after the NaN test is
/// exhaustive.
fn nonfinite_spelling(f: f64) -> &'static str {
    if f.is_nan() {
        "nan"
    } else if f.is_sign_positive() {
        "inf"
    } else {
        "-inf"
    }
}

/// Substitutes `placeholder` for every null in the document.
///
/// The alternative to [`to_toml`]'s rejection, and so only ever called on the
/// way to TOML — JSON holds a null fine and has nothing to be rescued from. A
/// null cannot be encoded as TOML, leaving only two honest options: fail, or
/// write a value that was in none of the inputs. *Which* value that is has to
/// be the user's choice rather than the tool's — `yq` and `tomlq` both drop
/// null keys silently, and both invent a string inside arrays (`""` and
/// `"None"` respectively, for the same input), which is the behaviour this
/// exists to avoid.
pub fn replace_nulls(value: &mut Value, placeholder: &str) {
    match value {
        Value::Null => *value = Value::String(placeholder.to_string()),
        Value::Object(obj) => {
            for v in obj.values_mut() {
                replace_nulls(v, placeholder);
            }
        }
        Value::Array(items) => {
            for v in items.iter_mut() {
                replace_nulls(v, placeholder);
            }
        }
        _ => {}
    }
}

/// Nulls survived into a document being converted to TOML.
///
/// A genuine impossibility in user data, so it is an error rather than a silent
/// drop: the `toml` crate's map serializer *skips* a `None` entry, so emitting
/// without this check would quietly lose keys.
///
/// Carries paths and nothing else — no filenames and no flag names. Naming the
/// layer each null came from would mean retaining every parsed layer past the
/// merge purely for an error path, and the paths alone locate the value in the
/// merged document. The remedies are all interface-shaped — emit JSON instead,
/// substitute a string, or drop the null — so which of them a caller can offer
/// is the caller's to say, the same division of labour [`crate::LoadError`]
/// keeps.
#[derive(Debug)]
pub struct NullInToml {
    entries: Vec<Vec<Seg>>,
}

impl fmt::Display for NullInToml {
    /// Ends without a trailing newline, so a caller can append a `help:` line
    /// of its own — which is what `knf-cli` does, and why the line is not here.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot serialize null to TOML")?;
        for path in &self.entries {
            write!(f, "\n  --> {}", render_path(path))?;
        }
        Ok(())
    }
}

impl std::error::Error for NullInToml {}

/// A [`Value::Datetime`] carrying text that is not a TOML datetime.
///
/// Unreachable from a parsed document, and unreachable from this crate's own
/// pipeline: a datetime only ever *originates* in the TOML parser, and neither
/// `${env:...}` nor the CLI's `--set` can make one, since both type their values
/// through JSON, which has no datetime. It is reachable from a caller that
/// builds a [`Value`] by hand, though — the variant is an ordinary public one
/// holding an ordinary `String` — and this is what that caller gets instead of a
/// panic. The rule it reports on is unchanged: nothing may synthesize a datetime
/// from text.
///
/// Carries paths and the offending spelling, and like [`NullInToml`] stops short
/// of a trailing newline so an interface can append a line of its own. There is no
/// flag that would help here, so no interface has one to append.
#[derive(Debug)]
pub struct BadDatetime {
    entries: Vec<(Vec<Seg>, String)>,
}

impl fmt::Display for BadDatetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot serialize datetime to TOML")?;
        for (path, text) in &self.entries {
            write!(f, "\n  --> {}: `{text}`", render_path(path))?;
        }
        Ok(())
    }
}

impl std::error::Error for BadDatetime {}

/// An integer too large for TOML's signed 64-bit integers.
///
/// Reachable from an ordinary JSON input — a snowflake ID or a hash above
/// `i64::MAX` is exactly what [`Number::U64`] exists to carry losslessly — so
/// unlike [`BadDatetime`] this is a failure a user meets rather than one only a
/// caller can build. It used to be rounded through `f64` on the way out, which
/// discarded the very digits the variant preserves.
///
/// Carries paths and the offending value, and like its siblings stops short of a
/// trailing newline so an interface can append a line of its own. There is one
/// worth appending here: `-f json` emits the integer exactly.
#[derive(Debug)]
pub struct IntegerOutOfRange {
    entries: Vec<(Vec<Seg>, u64)>,
}

impl fmt::Display for IntegerOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot serialize integer to TOML")?;
        for (path, n) in &self.entries {
            write!(f, "\n  --> {}: `{n}`", render_path(path))?;
        }
        Ok(())
    }
}

impl std::error::Error for IntegerOutOfRange {}

/// A float JSON cannot represent: an infinity or a NaN.
///
/// JSON's grammar has no spelling for either, and `serde_json` refuses to build a
/// `Number` from one. TOML's grammar *does* — `inf`, `-inf`, `nan` are literals —
/// so a `.toml` input can carry one straight into a `-f json` emission, where it
/// used to be silently replaced by `0`.
///
/// The lone JSON impossibility, which is why [`to_json`] returns this type
/// directly where [`to_toml`] returns the [`TomlError`] enum. A second one would
/// be the moment to introduce the wrapper, not before: an enum with one variant
/// buys a consumer nothing and costs it a `match`.
///
/// Carries paths and the value's TOML spelling, and stops short of a trailing
/// newline like its TOML-side siblings.
#[derive(Debug)]
pub struct NonFiniteFloat {
    entries: Vec<(Vec<Seg>, f64)>,
}

impl fmt::Display for NonFiniteFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot serialize non-finite number to JSON")?;
        for (path, value) in &self.entries {
            write!(
                f,
                "\n  --> {}: `{}`",
                render_path(path),
                nonfinite_spelling(*value)
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for NonFiniteFloat {}

/// Why a document could not be converted to TOML.
///
/// Deliberately no `#[from]` and no `#[source]` on either variant: a generated
/// `source()` would be a second copy of a message the variant's own `Display`
/// already prints in full, and `knf-cli` walks the cause chain onto stderr.
#[derive(Debug, thiserror::Error)]
pub enum TomlError {
    /// The document contains a null, which TOML cannot represent.
    #[error("{0}")]
    Null(NullInToml),
    /// The document contains an integer above `i64::MAX`.
    #[error("{0}")]
    Integer(IntegerOutOfRange),
    /// The document contains a datetime whose spelling does not parse.
    #[error("{0}")]
    Datetime(BadDatetime),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ir(v: serde_json::Value) -> Value {
        from_json(v)
    }

    fn json(v: Value) -> serde_json::Value {
        to_json(v).expect("no non-finite floats")
    }

    #[test]
    fn a_toml_datetime_stays_a_datetime_in_the_ir() {
        let parsed: toml::Value =
            toml::from_str("date = 1979-05-27T07:32:00Z\n").expect("valid toml");
        let v = from_toml(parsed);
        let Value::Object(map) = &v else {
            panic!("expected an object, got {v:?}");
        };
        assert_eq!(
            map["date"],
            Value::Datetime("1979-05-27T07:32:00Z".to_string())
        );
        // Only the sentinel-free `Display` spelling, never toml's internal map.
        assert_eq!(json(v), json!({"date": "1979-05-27T07:32:00Z"}));
    }

    /// All four TOML datetime forms round-trip through `Display`/`FromStr`,
    /// which is the whole basis for storing a datetime as a `String`.
    #[test]
    fn every_toml_datetime_form_round_trips() {
        let src = "\
offset = 1979-05-27T07:32:00Z
offset_frac = 1979-05-27T00:32:00.999999-07:00
local = 1979-05-27T07:32:00
date = 1979-05-27
time = 07:32:00.5
";
        let parsed: toml::Value = toml::from_str(src).expect("valid toml");
        let back = to_toml(from_toml(parsed.clone())).expect("every spelling re-parses");
        assert_eq!(back, parsed);
    }

    #[test]
    fn to_toml_rejects_nulls_with_array_indices() {
        let err = to_toml(ir(json!({"a": {"b": null}, "c": [1, null], "d": 2}))).unwrap_err();
        let TomlError::Null(report) = err else {
            panic!("expected a null report, got {err}");
        };
        let rendered: Vec<_> = report.entries.iter().map(|p| render_path(p)).collect();
        assert_eq!(rendered, vec!["a.b", "c[1]"]);
    }

    /// Reachable only from a hand-built `Value` — the variant is public and holds
    /// a plain `String` — and it used to abort the process instead. Every
    /// offender is named, so a caller does not learn of them one run at a time,
    /// and the report ends without a newline like its null-shaped sibling.
    #[test]
    fn to_toml_reports_every_datetime_that_does_not_reparse() {
        let value = Value::Object(Map::from_iter([
            ("created".to_string(), Value::Datetime("nope".to_string())),
            (
                "events".to_string(),
                Value::Array(vec![
                    // The valid one is untouched, so only the second is reported.
                    Value::Datetime("1979-05-27T07:32:00Z".to_string()),
                    Value::Datetime("yesterday".to_string()),
                ]),
            ),
        ]));

        let err = to_toml(value).unwrap_err();

        assert!(matches!(err, TomlError::Datetime(_)), "{err}");
        assert_eq!(
            err.to_string(),
            "cannot serialize datetime to TOML\n  --> created: `nope`\n  --> events[1]: `yesterday`"
        );
    }

    /// A document with all three reports the nulls, then the integers. The order
    /// is how close each is to something a real input file can contain: a null and
    /// an oversized integer both arrive from a document, a malformed datetime only
    /// from a caller that built one by hand.
    #[test]
    fn a_null_is_reported_before_a_malformed_datetime() {
        let all_three = || {
            Value::Object(Map::from_iter([
                ("a".to_string(), Value::Null),
                ("n".to_string(), Value::Number(Number::U64(u64::MAX))),
                ("d".to_string(), Value::Datetime("nope".to_string())),
            ]))
        };
        assert!(matches!(
            to_toml(all_three()).unwrap_err(),
            TomlError::Null(_)
        ));

        // Drop the null and the integer surfaces; drop that too and the datetime does.
        let Value::Object(mut map) = all_three() else {
            unreachable!()
        };
        map.shift_remove("a");
        assert!(matches!(
            to_toml(Value::Object(map.clone())).unwrap_err(),
            TomlError::Integer(_)
        ));
        map.shift_remove("n");
        assert!(matches!(
            to_toml(Value::Object(map)).unwrap_err(),
            TomlError::Datetime(_)
        ));
    }

    /// TOML integers are signed 64-bit, so a snowflake ID has no spelling there at
    /// all. It used to round through `f64` — discarding the exact digits
    /// `Number::U64` exists to keep — and now names every offender instead.
    #[test]
    fn to_toml_reports_every_integer_above_i64_max() {
        let value = ir(json!({
            "id": 10_000_000_000_000_000_001_u64,
            "ok": 42,
            "ids": [1, 18_446_744_073_709_551_615_u64],
        }));

        let err = to_toml(value).unwrap_err();

        assert!(matches!(err, TomlError::Integer(_)), "{err}");
        assert_eq!(
            err.to_string(),
            "cannot serialize integer to TOML\n  --> id: `10000000000000000001`\n  \
             --> ids[1]: `18446744073709551615`"
        );
    }

    /// The pre-walk guards on the range, not on the variant, so a `U64` small
    /// enough for an `i64` still converts. Only a hand-built value is spelled that
    /// way — `Number::from_u64` demotes — but nothing should panic when one is.
    #[test]
    fn a_u64_small_enough_for_an_i64_still_converts() {
        let value = Value::Object(Map::from_iter([(
            "n".to_string(),
            Value::Number(Number::U64(1)),
        )]));
        let toml = to_toml(value).expect("1 fits an i64");
        assert_eq!(toml["n"], toml::Value::Integer(1));
    }

    /// TOML's grammar has `inf`, `-inf` and `nan`; JSON's has none of them, and
    /// `serde_json` refuses to build a number from one. This used to be swallowed,
    /// emitting `0` for a value the user wrote as `inf`.
    #[test]
    fn to_json_reports_every_non_finite_float() {
        let value = Value::Object(Map::from_iter([
            (
                "timeout".to_string(),
                Value::Number(Number::F64(f64::INFINITY)),
            ),
            ("ok".to_string(), Value::Number(Number::F64(1.5))),
            (
                "backoff".to_string(),
                Value::Array(vec![
                    Value::Number(Number::F64(f64::NEG_INFINITY)),
                    Value::Number(Number::F64(f64::NAN)),
                ]),
            ),
        ]));

        let err = to_json(value).unwrap_err();

        assert_eq!(
            err.to_string(),
            "cannot serialize non-finite number to JSON\n  --> timeout: `inf`\n  \
             --> backoff[0]: `-inf`\n  --> backoff[1]: `nan`"
        );
    }

    /// Both reports end mid-line so `knf-cli` can append a `help:` line flush
    /// against the last `-->`, the same seam the null report keeps.
    #[test]
    fn the_new_reports_end_without_a_newline() {
        let big = ir(json!({"id": 10_000_000_000_000_000_001_u64}));
        assert!(!to_toml(big).unwrap_err().to_string().ends_with('\n'));

        let inf = Value::Object(Map::from_iter([(
            "t".to_string(),
            Value::Number(Number::F64(f64::INFINITY)),
        )]));
        assert!(!to_json(inf).unwrap_err().to_string().ends_with('\n'));
    }

    /// A non-finite float is a JSON problem alone: TOML spells all three, so the
    /// same document emits fine that way and `-f toml` is a real escape.
    #[test]
    fn non_finite_floats_are_fine_in_toml() {
        let value = Value::Object(Map::from_iter([(
            "timeout".to_string(),
            Value::Number(Number::F64(f64::INFINITY)),
        )]));
        assert_eq!(
            toml::to_string(&to_toml(value).expect("toml has inf")).expect("serializes"),
            "timeout = inf\n"
        );
    }

    #[test]
    fn numbers_bools_and_tables_round_trip() {
        let src = json!({
            "n": 1,
            "f": 1.5,
            "ok": true,
            "name": "svc",
            "xs": [1, 2],
            "nested": {"k": 3}
        });
        let toml = to_toml(ir(src.clone())).expect("no nulls");
        assert_eq!(json(from_toml(toml)), src);
    }

    /// A JSON integer above `i64::MAX` must not be rounded through `f64`.
    #[test]
    fn large_unsigned_integers_survive_json_round_trip() {
        let src = json!({"id": 10_000_000_000_000_000_001_u64});
        assert_eq!(json(ir(src.clone())), src);
    }

    /// The array case is the one both `yq` and `tomlq` fabricate a value for,
    /// since a null cannot be dropped from an array without shifting every
    /// index after it. Substituting preserves the length and lets the user name
    /// the value that lands there.
    #[test]
    fn replace_nulls_substitutes_everywhere_and_unblocks_toml() {
        let mut v = ir(json!({"a": {"b": null}, "c": [1, null, 3], "d": 2}));
        replace_nulls(&mut v, "none");
        assert_eq!(
            json(v.clone()),
            json!({"a": {"b": "none"}, "c": [1, "none", 3], "d": 2})
        );
        to_toml(v).expect("the substitution left no nulls");
    }

    /// A document without nulls is untouched, so the flag cannot perturb a
    /// merge that never needed it.
    #[test]
    fn replace_nulls_is_the_identity_without_nulls() {
        let src = json!({"a": 1, "xs": [1, 2], "nested": {"k": "v"}});
        let mut v = ir(src.clone());
        replace_nulls(&mut v, "none");
        assert_eq!(json(v), src);
    }
}
