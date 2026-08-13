//! Smoke coverage for the TOML walk: table recurse, number kind, datetime kind.

use knf_merge::{MergeError, MergeOptions, merge_all_toml, merge_toml};

fn parse(s: &str) -> toml::Value {
    toml::from_str(s).unwrap_or_else(|e| panic!("bad TOML: {e}\n{s}"))
}

#[test]
fn tables_recurse_per_key() {
    let got = merge_all_toml(
        [parse("[a]\nx = 1\n"), parse("[a]\ny = 2\n")],
        &MergeOptions::LAST_WINS,
    )
    .expect("merge");
    assert_eq!(got, parse("[a]\nx = 1\ny = 2\n"));
}

#[test]
fn int_and_float_are_one_kind_under_strict() {
    let got = merge_all_toml(
        [parse("a = 1\n"), parse("a = 1.5\n")],
        &MergeOptions::STRICT,
    )
    .expect("int and float are both number");
    assert_eq!(got, parse("a = 1.5\n"));
}

#[test]
fn datetime_conflicts_with_string_under_strict() {
    let mut base = parse("a = 1979-05-27T07:32:00Z\n");
    let err = merge_toml(
        &mut base,
        parse("a = \"1979-05-27T07:32:00Z\"\n"),
        &MergeOptions::STRICT,
    )
    .unwrap_err();
    match err {
        MergeError::TypeConflict {
            path,
            expected,
            found,
        } => {
            assert_eq!(path, ["a"]);
            assert_eq!(expected, "datetime");
            assert_eq!(found, "string");
        }
    }
}
