//! Table-driven merge tests. Adding a case is one line in `CASES`.

use knf_merge::{MergeError, MergeOptions, merge_all};
use serde_json::Value;

struct Case {
    name: &'static str,
    /// JSON literals, merged left to right.
    layers: &'static [&'static str],
    opts: MergeOptions,
    expect: Expect,
}

enum Expect {
    /// A JSON literal the merge must equal.
    Doc(&'static str),
    /// A type conflict at this dotted key path.
    Conflict(&'static str),
}

const fn ok(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        opts: MergeOptions::LAST_WINS,
        expect: Expect::Doc(doc),
    }
}

const fn strict(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        opts: MergeOptions::STRICT,
        expect: Expect::Doc(doc),
    }
}

const fn conflict(name: &'static str, layers: &'static [&'static str], path: &'static str) -> Case {
    Case {
        name,
        layers,
        opts: MergeOptions::STRICT,
        expect: Expect::Conflict(path),
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
    // The worked example. merge_all folds strictly left over the flat list, so
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
];

#[test]
fn table() {
    for case in CASES {
        let layers = case.layers.iter().map(|s| parse(s, case.name));
        let got = merge_all(layers, &case.opts);

        match (&case.expect, got) {
            (Expect::Doc(want), Ok(got)) => {
                assert_eq!(got, parse(want, case.name), "case `{}`", case.name);
            }
            (Expect::Doc(want), Err(e)) => {
                panic!("case `{}`: expected {want}, got error: {e}", case.name);
            }
            (Expect::Conflict(want), Err(MergeError::TypeConflict { path, .. })) => {
                assert_eq!(path.join("."), *want, "case `{}`: wrong path", case.name);
            }
            (Expect::Conflict(want), Ok(got)) => {
                panic!(
                    "case `{}`: expected conflict at `{want}`, merged to {got}",
                    case.name
                );
            }
        }
    }
}

fn parse(s: &str, case: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("case `{case}`: bad JSON literal {s}: {e}"))
}

/// `merge` and `merge_all` must agree — the former is what callers reach for
/// when they already hold an accumulator.
#[test]
fn merge_matches_merge_all() {
    for case in CASES {
        let Expect::Doc(want) = case.expect else {
            continue;
        };
        let mut acc = Value::Object(Default::default());
        for layer in case.layers {
            knf_merge::merge(&mut acc, parse(layer, case.name), &case.opts).expect(case.name);
        }
        assert_eq!(acc, parse(want, case.name), "case `{}`", case.name);
    }
}

/// The conflict path is relative to the document root, so an error at the root
/// itself has an empty path rather than a bogus key.
#[test]
fn root_level_conflict_has_empty_path() {
    let mut base = Value::Object(Default::default());
    let err = knf_merge::merge(&mut base, Value::Bool(true), &MergeOptions::STRICT).unwrap_err();
    assert_eq!(err.path(), &[] as &[String]);
    assert!(err.to_string().contains("<root>"), "{err}");
}

/// Error text carries both kinds, which is what makes the message actionable
/// without the user re-running with more verbosity.
#[test]
fn conflict_message_names_both_kinds() {
    let err = merge_all(
        [parse(r#"{"a":{"b":1}}"#, "msg"), parse(r#"{"a":5}"#, "msg")],
        &MergeOptions::STRICT,
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "type conflict at `a`: object would be replaced by number"
    );
}
