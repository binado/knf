//! Table-driven merge tests. Adding a case is one line in `CASES`.

use knf::{MergeError, MergeOptions, merge};
use serde_json::Value;

use Expect::{Doc, Error};

struct Case {
    name: &'static str,
    /// JSON literals, merged left to right.
    layers: &'static [&'static str],
    strict: bool,
    /// A full key-path glob selecting replacement values.
    shallow: Option<&'static str>,
    expect: Expect,
}

enum Expect {
    /// A JSON literal the merge must equal.
    Doc(&'static str),
    /// A type conflict at this dotted key path.
    Error(&'static str),
}

const fn ok(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        strict: false,
        shallow: None,
        expect: Doc(doc),
    }
}

const fn strict(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        strict: true,
        shallow: None,
        expect: Doc(doc),
    }
}

const fn conflict(name: &'static str, layers: &'static [&'static str], path: &'static str) -> Case {
    Case {
        name,
        layers,
        strict: true,
        shallow: None,
        expect: Error(path),
    }
}

const fn shallow(name: &'static str, layers: &'static [&'static str], expect: Expect) -> Case {
    Case {
        name,
        layers,
        strict: false,
        shallow: Some("*"),
        expect,
    }
}

const fn shallow_at(
    name: &'static str,
    at: &'static str,
    layers: &'static [&'static str],
    expect: Expect,
) -> Case {
    Case {
        name,
        layers,
        strict: false,
        shallow: Some(at),
        expect,
    }
}

const fn strict_shallow(
    name: &'static str,
    layers: &'static [&'static str],
    expect: Expect,
) -> Case {
    Case {
        name,
        layers,
        strict: true,
        shallow: Some("*"),
        expect,
    }
}

const fn strict_shallow_at(
    name: &'static str,
    at: &'static str,
    layers: &'static [&'static str],
    expect: Expect,
) -> Case {
    Case {
        name,
        layers,
        strict: true,
        shallow: Some(at),
        expect,
    }
}

fn options(case: &Case) -> MergeOptions {
    MergeOptions {
        strict: case.strict,
        shallow: case.shallow.map(|pattern| pattern.parse().unwrap()),
    }
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    // --- §2.1: the semantics table ------------------------------------------
    ok("no layers is an empty object", &[], "{}"),
    ok("one layer is a no-op", &[r#"{"a":1,"b":{"c":[1,2]}}"#], r#"{"a":1,"b":{"c":[1,2]}}"#),
    ok("object + object recurses per key", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"x":1,"y":2}}"#),
    ok("disjoint keys union", &[r#"{"a":1}"#, r#"{"b":2}"#], r#"{"a":1,"b":2}"#),
    ok("scalar + scalar is last-wins", &[r#"{"a":1}"#, r#"{"a":2}"#], r#"{"a":2}"#),
    ok("scalar shadows an object", &[r#"{"a":{"b":1}}"#, r#"{"a":5}"#], r#"{"a":5}"#),
    ok("object shadows a scalar", &[r#"{"a":5}"#, r#"{"a":{"b":1}}"#], r#"{"a":{"b":1}}"#),

    // Arrays replace wholesale. Lodash-style index-merging would produce
    // ["a","y","z"] here — a value nobody wrote.
    ok("array replaces, never index-merges", &[r#"{"a":["x","y","z"]}"#, r#"{"a":["a"]}"#], r#"{"a":["a"]}"#),
    ok("array replaces with the empty array", &[r#"{"a":[1,2]}"#, r#"{"a":[]}"#], r#"{"a":[]}"#),
    ok("array is not merged element-wise", &[r#"{"a":[{"x":1}]}"#, r#"{"a":[{"y":2}]}"#], r#"{"a":[{"y":2}]}"#),
    ok("array replaces a scalar", &[r#"{"a":1}"#, r#"{"a":[1]}"#], r#"{"a":[1]}"#),

    // Null is a value, not a delete (RFC 7386 merge-patch was rejected).
    ok("null overwrites a scalar", &[r#"{"a":1}"#, r#"{"a":null}"#], r#"{"a":null}"#),
    ok("null overwrites an object", &[r#"{"a":{"b":1}}"#, r#"{"a":null}"#], r#"{"a":null}"#),
    ok("a value overwrites null", &[r#"{"a":null}"#, r#"{"a":1}"#], r#"{"a":1}"#),
    ok("null survives a single layer", &[r#"{"a":null}"#], r#"{"a":null}"#),

    // --- §2.1: merge is not associative -------------------------------------
    // The worked example. merge folds strictly left over the flat list, so
    // {a:5} erases {a:{b:1}} and {a:{c:2}} then merges into a fresh object.
    ok("left fold, not right", &[r#"{"a":{"b":1}}"#, r#"{"a":5}"#, r#"{"a":{"c":2}}"#], r#"{"a":{"c":2}}"#),
    // Grouping the last two first would give {"a":{"b":1,"c":2}} — the bug this
    // ordering rule exists to prevent.
    ok("three-layer deep merge", &[r#"{"a":{"b":1}}"#, r#"{"a":{"c":2}}"#, r#"{"a":{"b":9}}"#], r#"{"a":{"b":9,"c":2}}"#),

    // --- nesting depth ------------------------------------------------------
    ok("deep recursion", &[r#"{"a":{"b":{"c":{"d":1}}}}"#, r#"{"a":{"b":{"c":{"e":2}}}}"#], r#"{"a":{"b":{"c":{"d":1,"e":2}}}}"#),
    ok("deep insert into a missing branch", &[r#"{"a":{"b":1}}"#, r#"{"x":{"y":{"z":2}}}"#], r#"{"a":{"b":1},"x":{"y":{"z":2}}}"#),

    // --- §2.2: strict mode --------------------------------------------------
    strict("strict allows new keys", &[r#"{"a":1}"#, r#"{"b":2}"#], r#"{"a":1,"b":2}"#),
    strict("strict allows same-kind replacement", &[r#"{"a":1}"#, r#"{"a":2}"#], r#"{"a":2}"#),
    strict("strict treats int and float as one kind", &[r#"{"a":1}"#, r#"{"a":1.5}"#], r#"{"a":1.5}"#),
    strict("strict allows array replacement", &[r#"{"a":[1]}"#, r#"{"a":["x","y"]}"#], r#"{"a":["x","y"]}"#),
    strict("strict allows null over null", &[r#"{"a":null}"#, r#"{"a":null}"#], r#"{"a":null}"#),
    strict("strict recurses without conflict", &[r#"{"a":{"b":1}}"#, r#"{"a":{"b":2,"c":3}}"#], r#"{"a":{"b":2,"c":3}}"#),

    conflict("scalar shadowing an object", &[r#"{"a":{"b":1}}"#, r#"{"a":5}"#], "a"),
    conflict("object shadowing a scalar", &[r#"{"a":5}"#, r#"{"a":{"b":1}}"#], "a"),
    conflict("array shadowing a scalar", &[r#"{"a":1}"#, r#"{"a":[1]}"#], "a"),
    conflict("null shadowing a value", &[r#"{"a":1}"#, r#"{"a":null}"#], "a"),
    conflict("value shadowing null", &[r#"{"a":null}"#, r#"{"a":1}"#], "a"),
    conflict("string shadowing a number", &[r#"{"a":1}"#, r#"{"a":"1"}"#], "a"),
    conflict("bool shadowing a number", &[r#"{"a":1}"#, r#"{"a":true}"#], "a"),
    conflict("conflict reports a nested path", &[r#"{"a":{"b":{"c":1}}}"#, r#"{"a":{"b":{"c":[]}}}"#], "a.b.c"),
    conflict("conflict from the third layer", &[r#"{"a":1}"#, r#"{"a":2}"#, r#"{"a":"three"}"#], "a"),

    // --- shallow: jq's `+` rather than `*` ---------------------------------
    // Top-level keys only: a colliding object is taken whole, so keys the
    // overlay omits are gone.
    shallow("shallow takes a nested object whole", &[r#"{"a":{"x":1,"y":2}}"#, r#"{"a":{"y":9}}"#], Doc(r#"{"a":{"y":9}}"#)),
    shallow("shallow keeps untouched top-level keys", &[r#"{"a":{"x":1},"b":1}"#, r#"{"a":{"y":2}}"#], Doc(r#"{"a":{"y":2},"b":1}"#)),
    shallow("shallow still inserts new keys", &[r#"{"a":1}"#, r#"{"b":{"c":2}}"#], Doc(r#"{"a":1,"b":{"c":2}}"#)),
    shallow("shallow replaces arrays too", &[r#"{"a":[1,2]}"#, r#"{"a":[3]}"#], Doc(r#"{"a":[3]}"#)),
    shallow("shallow one layer is a no-op", &[r#"{"a":{"b":[1]}}"#], Doc(r#"{"a":{"b":[1]}}"#)),
    shallow("shallow across three layers", &[r#"{"a":{"x":1}}"#, r#"{"a":5}"#, r#"{"a":{"y":2}}"#], Doc(r#"{"a":{"y":2}}"#)),

    // --strict is orthogonal: it kind-checks wherever a replacement happens,
    // and under shallow that is every colliding top-level key.
    strict_shallow("strict kind-checks a shallow replace", &[r#"{"a":{"x":1}}"#, r#"{"a":5}"#], Error("a")),
    strict_shallow("strict allows a same-kind shallow replace", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":"s"}}"#], Doc(r#"{"a":{"y":"s"}}"#)),
    // Nothing below the top level is compared: `x` changes kind unseen.
    strict_shallow("strict shallow never looks below the top level", &[r#"{"a":{"x":1}}"#, r#"{"a":{"x":"s"}}"#], Doc(r#"{"a":{"x":"s"}}"#)),

    // --- selectors replace matching values; `foo.*` is shallow inside foo --

    shallow_at("a selected object replaces entirely", "db", &[r#"{"db":{"pool":{"min":1},"host":"a"},"app":{"x":1}}"#, r#"{"db":{"pool":{"max":9}},"app":{"y":2}}"#], Doc(r#"{"db":{"pool":{"max":9}},"app":{"x":1,"y":2}}"#)),
    shallow_at("globstar selects cache at every depth", "**.cache", &[r#"{"cache":{"old":1},"a":{"cache":{"old":1},"keep":{"x":1}}}"#, r#"{"cache":{"new":2},"a":{"cache":{"new":2},"keep":{"y":2}}}"#], Doc(r#"{"cache":{"new":2},"a":{"cache":{"new":2},"keep":{"x":1,"y":2}}}"#)),
    shallow_at("selected ancestor stops descendant traversal", "{a,a.b}", &[r#"{"a":{"b":{"old":1},"keep":1}}"#, r#"{"a":{"b":{"new":2}}}"#], Doc(r#"{"a":{"b":{"new":2}}}"#)),
    shallow_at("negation can select a parent before its children", "!a.*", &[r#"{"a":{"old":1}}"#, r#"{"a":{"new":2}}"#], Doc(r#"{"a":{"new":2}}"#)),
    shallow_at("quoted dot and slash keys differ from nested paths", "{'a.b','a/b'}", &[r#"{"a.b":{"old":1},"a/b":{"old":1},"a":{"b":{"old":1}}}"#, r#"{"a.b":{"new":2},"a/b":{"new":2},"a":{"b":{"new":2}}}"#], Doc(r#"{"a.b":{"new":2},"a/b":{"new":2},"a":{"b":{"old":1,"new":2}}}"#)),
    shallow_at("brackets name a literal key when quoted", "'servers[0]'", &[r#"{"servers[0]":{"old":1},"servers":[{"old":1}]}"#, r#"{"servers[0]":{"new":2},"servers":[{"new":2}]}"#], Doc(r#"{"servers[0]":{"new":2},"servers":[{"new":2}]}"#)),
    shallow_at("selected null is an ordinary overwrite", "a", &[r#"{"a":{"old":1}}"#, r#"{"a":null}"#], Doc(r#"{"a":null}"#)),
    shallow_at("selected paths still fold left across scalar shadows", "a.b", &[r#"{"a":{"b":{"old":1},"keep":1}}"#, r#"{"a":5}"#, r#"{"a":{"b":{"new":2}}}"#], Doc(r#"{"a":{"b":{"new":2}}}"#)),
    strict_shallow_at("strict checks a selected object boundary", "a", &[r#"{"a":{"old":1}}"#, r#"{"a":5}"#], Error("a")),
    strict_shallow_at("strict never visits selected descendants", "a", &[r#"{"a":{"b":1}}"#, r#"{"a":{"b":"s"}}"#], Doc(r#"{"a":{"b":"s"}}"#)),
    // `db`'s children are replaced whole; `db` itself and its siblings merge deep.
    shallow_at("shallow at a path takes its children whole", "db.*", &[r#"{"db":{"pool":{"min":1,"max":5},"host":"a"}}"#, r#"{"db":{"pool":{"max":9}}}"#], Doc(r#"{"db":{"pool":{"max":9},"host":"a"}}"#)),
    shallow_at("shallow at a path leaves siblings deep", "db.*", &[r#"{"db":{"x":{"a":1}},"app":{"x":{"a":1}}}"#, r#"{"db":{"x":{"b":2}},"app":{"x":{"b":2}}}"#], Doc(r#"{"db":{"x":{"b":2}},"app":{"x":{"a":1,"b":2}}}"#)),
    shallow_at("shallow at a nested path", "a.b.*", &[r#"{"a":{"b":{"c":{"x":1}},"d":{"x":1}}}"#, r#"{"a":{"b":{"c":{"y":2}},"d":{"y":2}}}"#], Doc(r#"{"a":{"b":{"c":{"y":2}},"d":{"x":1,"y":2}}}"#)),
    shallow_at("several shallow paths", "{a.*,b.*}", &[r#"{"a":{"x":{"k":1}},"b":{"x":{"k":1}},"c":{"x":{"k":1}}}"#, r#"{"a":{"x":{"j":2}},"b":{"x":{"j":2}},"c":{"x":{"j":2}}}"#], Doc(r#"{"a":{"x":{"j":2}},"b":{"x":{"j":2}},"c":{"x":{"k":1,"j":2}}}"#)),
    // The outer path replaces `a`'s children whole, so `a.b` is never reached.
    shallow_at("an outer shallow path makes an inner one moot", "{a.*,a.b.*}", &[r#"{"a":{"b":{"c":{"x":1}}}}"#, r#"{"a":{"b":{"d":2}}}"#], Doc(r#"{"a":{"b":{"d":2}}}"#)),
    shallow_at("root and a path together are just root", "{*,a.*}", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], Doc(r#"{"a":{"y":2}}"#)),
    // A path decided by argv may not exist, or name a non-object, in the layers.
    shallow_at("a missing shallow path is a deep merge", "nope.*", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], Doc(r#"{"a":{"x":1,"y":2}}"#)),
    shallow_at("a shallow path at a scalar just replaces it", "a.*", &[r#"{"a":1}"#, r#"{"a":2}"#], Doc(r#"{"a":2}"#)),
    shallow_at("a shallow path at an array just replaces it", "a.*", &[r#"{"a":[{"x":1}]}"#, r#"{"a":[{"y":2}]}"#], Doc(r#"{"a":[{"y":2}]}"#)),
    shallow_at("a shallow path shadowed by a scalar", "a.*", &[r#"{"a":{"x":1}}"#, r#"{"a":5}"#, r#"{"a":{"y":2}}"#], Doc(r#"{"a":{"y":2}}"#)),

    strict_shallow_at("strict kind-checks a pathed shallow replace", "a.*", &[r#"{"a":{"b":{"x":1}}}"#, r#"{"a":{"b":5}}"#], Error("a.b")),
    strict_shallow_at("strict pathed shallow never looks below the path's children", "a.*", &[r#"{"a":{"b":{"x":1}}}"#, r#"{"a":{"b":{"x":"s"}}}"#], Doc(r#"{"a":{"b":{"x":"s"}}}"#)),
];

#[test]
fn table() {
    for case in CASES {
        let layers = case.layers.iter().map(|s| ir(s));
        let got = merge(layers, &options(case));

        match (&case.expect, got) {
            (Doc(want), Ok(got)) => {
                assert_eq!(got, ir(want), "case `{}`", case.name);
            }
            (Doc(want), Err(e)) => {
                panic!("case `{}`: expected {want}, got error: {e}", case.name);
            }
            (Error(want), Err(e)) => {
                assert_eq!(
                    e.path().join("."),
                    *want,
                    "case `{}`: wrong path",
                    case.name
                );
            }
            (Error(want), Ok(got)) => {
                panic!(
                    "case `{}`: expected a type conflict at `{want}`, merged to {got:?}",
                    case.name
                );
            }
        }
    }
}

/// `merge_into` and `merge` must agree — the former is what callers reach for
/// when they already hold an accumulator.
#[test]
fn merge_into_matches_merge() {
    for case in CASES {
        let Doc(want) = case.expect else {
            continue;
        };
        let opts = options(case);
        let mut acc = Value::Object(Default::default());
        for layer in case.layers {
            knf::merge_into(&mut acc, ir(layer), &opts).expect(case.name);
        }
        assert_eq!(acc, ir(want), "case `{}`", case.name);
    }
}

/// The conflict path is relative to the document root, so an error at the root
/// itself has an empty path rather than a bogus key.
#[test]
fn root_level_conflict_has_empty_path() {
    let mut base = Value::Object(Default::default());
    let err = knf::merge_into(&mut base, Value::Bool(true), &MergeOptions::STRICT).unwrap_err();
    assert_eq!(err.path(), &[] as &[String]);
    assert!(err.to_string().contains("<root>"), "{err}");
}

/// Error text carries both kinds, which is what makes the message actionable
/// without the user re-running with more verbosity.
#[test]
fn conflict_message_names_both_kinds() {
    let err = merge(
        [ir(r#"{"a":{"b":1}}"#), ir(r#"{"a":5}"#)],
        &MergeOptions::STRICT,
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "type conflict at `a`: object would be replaced by number"
    );
}

#[test]
fn datetime_conflicts_with_string_under_strict() {
    let mut base: toml::Value = toml::from_str("a = 1979-05-27T07:32:00Z").unwrap();
    let over: toml::Value = toml::from_str("a = '1979-05-27T07:32:00Z'").unwrap();
    let err = knf::merge_into(&mut base, over, &MergeOptions::STRICT).unwrap_err();
    let MergeError::TypeConflict {
        path,
        expected,
        found,
    } = err;
    assert_eq!(path, ["a"]);
    assert_eq!(expected, "datetime");
    assert_eq!(found, "string");
}

/// Parses a native JSON fixture. Panics on a malformed literal — every
/// caller passes a `&'static str` written in this file.
fn ir(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("bad JSON literal `{s}`: {e}"))
}

/// Exercise the same structural cases as JSON, excluding JSON-only null values.
#[test]
fn toml_table_and_merge_into() {
    for case in CASES {
        let fixtures: Vec<Value> = case.layers.iter().map(|text| ir(text)).collect();
        if fixtures.iter().any(has_null) {
            continue;
        }
        let layers: Vec<toml::Value> = fixtures
            .into_iter()
            .map(|value| toml::Value::try_from(value).unwrap())
            .collect();
        let opts = options(case);
        let got = merge(layers.clone(), &opts);
        match (&case.expect, got) {
            (Doc(want), Ok(got)) => {
                let expected = toml::Value::try_from(ir(want)).unwrap();
                assert_eq!(got, expected, "{}", case.name);
                let mut acc = toml::Value::Table(Default::default());
                for layer in layers {
                    knf::merge_into(&mut acc, layer, &opts).unwrap();
                }
                assert_eq!(acc, expected, "{}", case.name);
            }
            (Error(want), Err(error)) => assert_eq!(error.path().join("."), *want, "{}", case.name),
            (_, got) => panic!("{}: unexpected result {got:?}", case.name),
        }
    }
}

fn has_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().any(has_null),
        Value::Object(map) => map.values().any(has_null),
        _ => false,
    }
}
