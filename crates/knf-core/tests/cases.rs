//! Table-driven merge tests. Adding a case is one line in `CASES`.

use knf::{MergeOptions, merge};
use serde_json::Value;

struct Case {
    name: &'static str,
    /// JSON literals, merged left to right.
    layers: &'static [&'static str],
    /// A full key-path glob selecting replacement values.
    shallow: Option<&'static str>,
    /// A JSON literal the merge must equal.
    expect: &'static str,
}

const fn ok(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        shallow: None,
        expect: doc,
    }
}

const fn shallow(
    name: &'static str,
    layers: &'static [&'static str],
    expect: &'static str,
) -> Case {
    Case {
        name,
        layers,
        shallow: Some("*"),
        expect,
    }
}

const fn shallow_at(
    name: &'static str,
    at: &'static str,
    layers: &'static [&'static str],
    expect: &'static str,
) -> Case {
    Case {
        name,
        layers,
        shallow: Some(at),
        expect,
    }
}

fn options(case: &Case) -> MergeOptions {
    MergeOptions {
        shallow: case.shallow.map(|pattern| pattern.parse().unwrap()),
    }
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    // --- the semantics table ------------------------------------------------
    ok("no layers is an empty object", &[], "{}"),
    ok("one layer is a no-op", &[r#"{"a":1,"b":{"c":[1,2]}}"#], r#"{"a":1,"b":{"c":[1,2]}}"#),
    ok("object + object recurses per key", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"x":1,"y":2}}"#),
    ok("disjoint keys union", &[r#"{"a":1}"#, r#"{"b":2}"#], r#"{"a":1,"b":2}"#),
    ok("scalar + scalar is last-wins", &[r#"{"a":1}"#, r#"{"a":2}"#], r#"{"a":2}"#),
    ok("scalar shadows an object", &[r#"{"a":{"b":1}}"#, r#"{"a":5}"#], r#"{"a":5}"#),
    ok("object shadows a scalar", &[r#"{"a":5}"#, r#"{"a":{"b":1}}"#], r#"{"a":{"b":1}}"#),

    // Arrays replace wholesale; never index-merged.
    ok("array replaces, never index-merges", &[r#"{"a":["x","y","z"]}"#, r#"{"a":["a"]}"#], r#"{"a":["a"]}"#),
    ok("array replaces with the empty array", &[r#"{"a":[1,2]}"#, r#"{"a":[]}"#], r#"{"a":[]}"#),
    ok("array is not merged element-wise", &[r#"{"a":[{"x":1}]}"#, r#"{"a":[{"y":2}]}"#], r#"{"a":[{"y":2}]}"#),
    ok("array replaces a scalar", &[r#"{"a":1}"#, r#"{"a":[1]}"#], r#"{"a":[1]}"#),

    // Null is a value, not a delete.
    ok("null overwrites a scalar", &[r#"{"a":1}"#, r#"{"a":null}"#], r#"{"a":null}"#),
    ok("null overwrites an object", &[r#"{"a":{"b":1}}"#, r#"{"a":null}"#], r#"{"a":null}"#),
    ok("a value overwrites null", &[r#"{"a":null}"#, r#"{"a":1}"#], r#"{"a":1}"#),
    ok("null survives a single layer", &[r#"{"a":null}"#], r#"{"a":null}"#),

    // --- merge is not associative -------------------------------------------
    ok("left fold, not right", &[r#"{"a":{"b":1}}"#, r#"{"a":5}"#, r#"{"a":{"c":2}}"#], r#"{"a":{"c":2}}"#),
    // Right-grouping would give {"a":{"b":1,"c":2}}.
    ok("three-layer deep merge", &[r#"{"a":{"b":1}}"#, r#"{"a":{"c":2}}"#, r#"{"a":{"b":9}}"#], r#"{"a":{"b":9,"c":2}}"#),

    // --- nesting depth ------------------------------------------------------
    ok("deep recursion", &[r#"{"a":{"b":{"c":{"d":1}}}}"#, r#"{"a":{"b":{"c":{"e":2}}}}"#], r#"{"a":{"b":{"c":{"d":1,"e":2}}}}"#),
    ok("deep insert into a missing branch", &[r#"{"a":{"b":1}}"#, r#"{"x":{"y":{"z":2}}}"#], r#"{"a":{"b":1},"x":{"y":{"z":2}}}"#),



    // --- shallow: jq's `+` rather than `*` ---------------------------------
    shallow("shallow takes a nested object whole", &[r#"{"a":{"x":1,"y":2}}"#, r#"{"a":{"y":9}}"#], r#"{"a":{"y":9}}"#),
    shallow("shallow keeps untouched top-level keys", &[r#"{"a":{"x":1},"b":1}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"y":2},"b":1}"#),
    shallow("shallow still inserts new keys", &[r#"{"a":1}"#, r#"{"b":{"c":2}}"#], r#"{"a":1,"b":{"c":2}}"#),
    shallow("shallow replaces arrays too", &[r#"{"a":[1,2]}"#, r#"{"a":[3]}"#], r#"{"a":[3]}"#),
    shallow("shallow one layer is a no-op", &[r#"{"a":{"b":[1]}}"#], r#"{"a":{"b":[1]}}"#),
    shallow("shallow across three layers", &[r#"{"a":{"x":1}}"#, r#"{"a":5}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"y":2}}"#),


    // --- selectors replace matching values; `foo.*` is shallow inside foo --

    shallow_at("a selected object replaces entirely", "db", &[r#"{"db":{"pool":{"min":1},"host":"a"},"app":{"x":1}}"#, r#"{"db":{"pool":{"max":9}},"app":{"y":2}}"#], r#"{"db":{"pool":{"max":9}},"app":{"x":1,"y":2}}"#),
    shallow_at("globstar selects cache at every depth", "**.cache", &[r#"{"cache":{"old":1},"a":{"cache":{"old":1},"keep":{"x":1}}}"#, r#"{"cache":{"new":2},"a":{"cache":{"new":2},"keep":{"y":2}}}"#], r#"{"cache":{"new":2},"a":{"cache":{"new":2},"keep":{"x":1,"y":2}}}"#),
    shallow_at("selected ancestor stops descendant traversal", "{a,a.b}", &[r#"{"a":{"b":{"old":1},"keep":1}}"#, r#"{"a":{"b":{"new":2}}}"#], r#"{"a":{"b":{"new":2}}}"#),
    shallow_at("negation can select a parent before its children", "!a.*", &[r#"{"a":{"old":1}}"#, r#"{"a":{"new":2}}"#], r#"{"a":{"new":2}}"#),
    shallow_at("quoted dot and slash keys differ from nested paths", "{'a.b','a/b'}", &[r#"{"a.b":{"old":1},"a/b":{"old":1},"a":{"b":{"old":1}}}"#, r#"{"a.b":{"new":2},"a/b":{"new":2},"a":{"b":{"new":2}}}"#], r#"{"a.b":{"new":2},"a/b":{"new":2},"a":{"b":{"old":1,"new":2}}}"#),
    shallow_at("brackets name a literal key when quoted", "'servers[0]'", &[r#"{"servers[0]":{"old":1},"servers":[{"old":1}]}"#, r#"{"servers[0]":{"new":2},"servers":[{"new":2}]}"#], r#"{"servers[0]":{"new":2},"servers":[{"new":2}]}"#),
    shallow_at("selected null is an ordinary overwrite", "a", &[r#"{"a":{"old":1}}"#, r#"{"a":null}"#], r#"{"a":null}"#),
    shallow_at("selected paths still fold left across scalar shadows", "a.b", &[r#"{"a":{"b":{"old":1},"keep":1}}"#, r#"{"a":5}"#, r#"{"a":{"b":{"new":2}}}"#], r#"{"a":{"b":{"new":2}}}"#),
    // `db`'s children are replaced whole; `db` itself and its siblings merge deep.
    shallow_at("shallow at a path takes its children whole", "db.*", &[r#"{"db":{"pool":{"min":1,"max":5},"host":"a"}}"#, r#"{"db":{"pool":{"max":9}}}"#], r#"{"db":{"pool":{"max":9},"host":"a"}}"#),
    shallow_at("shallow at a path leaves siblings deep", "db.*", &[r#"{"db":{"x":{"a":1}},"app":{"x":{"a":1}}}"#, r#"{"db":{"x":{"b":2}},"app":{"x":{"b":2}}}"#], r#"{"db":{"x":{"b":2}},"app":{"x":{"a":1,"b":2}}}"#),
    shallow_at("shallow at a nested path", "a.b.*", &[r#"{"a":{"b":{"c":{"x":1}},"d":{"x":1}}}"#, r#"{"a":{"b":{"c":{"y":2}},"d":{"y":2}}}"#], r#"{"a":{"b":{"c":{"y":2}},"d":{"x":1,"y":2}}}"#),
    shallow_at("several shallow paths", "{a.*,b.*}", &[r#"{"a":{"x":{"k":1}},"b":{"x":{"k":1}},"c":{"x":{"k":1}}}"#, r#"{"a":{"x":{"j":2}},"b":{"x":{"j":2}},"c":{"x":{"j":2}}}"#], r#"{"a":{"x":{"j":2}},"b":{"x":{"j":2}},"c":{"x":{"k":1,"j":2}}}"#),
    // The outer path replaces `a`'s children whole, so `a.b` is never reached.
    shallow_at("an outer shallow path makes an inner one moot", "{a.*,a.b.*}", &[r#"{"a":{"b":{"c":{"x":1}}}}"#, r#"{"a":{"b":{"d":2}}}"#], r#"{"a":{"b":{"d":2}}}"#),
    shallow_at("root and a path together are just root", "{*,a.*}", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"y":2}}"#),
    // A selector may name a missing path.
    shallow_at("a missing shallow path is a deep merge", "nope.*", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"x":1,"y":2}}"#),
    shallow_at("a shallow path at a scalar just replaces it", "a.*", &[r#"{"a":1}"#, r#"{"a":2}"#], r#"{"a":2}"#),
    shallow_at("a shallow path at an array just replaces it", "a.*", &[r#"{"a":[{"x":1}]}"#, r#"{"a":[{"y":2}]}"#], r#"{"a":[{"y":2}]}"#),
    shallow_at("a shallow path shadowed by a scalar", "a.*", &[r#"{"a":{"x":1}}"#, r#"{"a":5}"#, r#"{"a":{"y":2}}"#], r#"{"a":{"y":2}}"#),

];

#[test]
fn table() {
    for case in CASES {
        let layers = case.layers.iter().map(|s| ir(s));
        let got = merge(layers, &options(case));
        assert_eq!(got, ir(case.expect), "case `{}`", case.name);
    }
}

/// `merge_into` and `merge` agree.
#[test]
fn merge_into_matches_merge() {
    for case in CASES {
        let opts = options(case);
        let mut acc = Value::Object(Default::default());
        for layer in case.layers {
            knf::merge_into(&mut acc, ir(layer), &opts);
        }
        assert_eq!(acc, ir(case.expect), "case `{}`", case.name);
    }
}

/// Parses a JSON fixture.
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
        let expected = toml::Value::try_from(ir(case.expect)).unwrap();
        assert_eq!(merge(layers.clone(), &opts), expected, "{}", case.name);
        let mut acc = toml::Value::Table(Default::default());
        for layer in layers {
            knf::merge_into(&mut acc, layer, &opts);
        }
        assert_eq!(acc, expected, "{}", case.name);
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
