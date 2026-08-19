//! Rendering a referent into surrounding text.
//!
//! Only embedded references need this — a whole-string reference takes the
//! referent's value *and type*, so nothing is rendered at all.
//!
//! Numbers are hand-rolled because this crate deliberately has no serializer:
//! Rust's [`Display`](std::fmt::Display) prints `1.0f64` as `1`, while both of
//! `knf`'s emitters write `1.0`. Borrowing `Display` would make
//! `"v${version}"` and `version = ${version}` disagree about the same value.

use knf_core::{Number, Value};

/// The text an embedded reference splices in, or `None` when the value has no
/// format-independent spelling.
///
/// `Null`, `Array` and `Object` are the `None` cases. For the containers there
/// is genuinely no answer: `${db}` inside a string is `{host = "x"}` under
/// `-f toml` and `{"host":"x"}` under `-f json`, and picking one would mean this
/// crate knowing the output format and pulling in a format crate to do it.
/// Rejecting is the direction that can be loosened later.
pub fn stringify(value: &Value) -> Option<String> {
    match value {
        Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
        // A datetime is its stored source spelling, which is what the TOML
        // parser printed and what a TOML emitter would print again.
        Value::String(s) | Value::Datetime(s) => Some(s.clone()),
        Value::Number(n) => Some(number(*n)),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn number(n: Number) -> String {
    match n {
        Number::I64(i) => i.to_string(),
        Number::U64(u) => u.to_string(),
        Number::F64(f) => float(f),
    }
}

/// A float as JSON and TOML both write it.
///
/// [`Debug`](std::fmt::Debug), not [`Display`](std::fmt::Display), and that is
/// the whole subtlety: `Display` prints `1.0f64` as `1`, while `Debug` keeps the
/// point and switches to exponent form at the same extremes the emitters' own
/// shortest-round-trip formatting does.
///
/// Non-finite values are reachable rather than theoretical — `from_toml` accepts
/// TOML's `inf` and `nan` literals — so they take their TOML spellings. The sign
/// of a NaN is not meaningful, so every NaN renders `nan`.
fn float(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f.is_sign_positive() { "inf" } else { "-inf" }.to_string();
    }
    format!("{f:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use knf_core::Map;

    #[test]
    fn scalars_render() {
        assert_eq!(stringify(&Value::Bool(true)).unwrap(), "true");
        assert_eq!(stringify(&Value::Bool(false)).unwrap(), "false");
        assert_eq!(stringify(&Value::String("hi".into())).unwrap(), "hi");
        assert_eq!(
            stringify(&Value::Datetime("1979-05-27T07:32:00Z".into())).unwrap(),
            "1979-05-27T07:32:00Z"
        );
    }

    /// The subtle one: `1.0` must not render as `1`, or an embedded reference
    /// would disagree with the emitter about the same value.
    #[test]
    fn integral_floats_keep_their_point() {
        assert_eq!(number(Number::F64(1.0)), "1.0");
        assert_eq!(number(Number::F64(-0.0)), "-0.0");
        assert_eq!(number(Number::F64(1.5)), "1.5");
        assert_eq!(number(Number::I64(1)), "1");
        assert_eq!(number(Number::U64(u64::MAX)), "18446744073709551615");
    }

    /// At the extremes the spelling goes exponential, as both emitters' own
    /// shortest-round-trip formatting does, and an exponent needs no `.0`.
    #[test]
    fn extreme_magnitudes_go_exponential() {
        assert_eq!(number(Number::F64(1e300)), "1e300");
        assert_eq!(number(Number::F64(1e-7)), "1e-7");
    }

    #[test]
    fn non_finite_floats_take_their_toml_spellings() {
        assert_eq!(number(Number::F64(f64::INFINITY)), "inf");
        assert_eq!(number(Number::F64(f64::NEG_INFINITY)), "-inf");
        assert_eq!(number(Number::F64(f64::NAN)), "nan");
        assert_eq!(number(Number::F64(-f64::NAN)), "nan");
    }

    #[test]
    fn containers_and_null_have_no_spelling() {
        assert_eq!(stringify(&Value::Null), None);
        assert_eq!(stringify(&Value::Array(vec![])), None);
        assert_eq!(stringify(&Value::Object(Map::new())), None);
    }
}
