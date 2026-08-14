//! Table-driven merge tests. Adding a case is one line in `CASES`.

mod common;

use common::ir;
use knf_core::{MergeError, MergeOptions, Value, merge_with};

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
    // The worked example. merge_with folds strictly left over the flat list, so
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
        let layers = case.layers.iter().map(|s| ir(s));
        let got = merge_with(layers, &case.opts);

        match (&case.expect, got) {
            (Expect::Doc(want), Ok(got)) => {
                assert_eq!(got, ir(want), "case `{}`", case.name);
            }
            (Expect::Doc(want), Err(e)) => {
                panic!("case `{}`: expected {want}, got error: {e}", case.name);
            }
            (Expect::Conflict(want), Err(MergeError::TypeConflict { path, .. })) => {
                assert_eq!(path.join("."), *want, "case `{}`: wrong path", case.name);
            }
            (Expect::Conflict(want), Ok(got)) => {
                panic!(
                    "case `{}`: expected conflict at `{want}`, merged to {got:?}",
                    case.name
                );
            }
        }
    }
}

/// `merge_into` and `merge_with` must agree — the former is what callers reach for
/// when they already hold an accumulator.
#[test]
fn merge_into_matches_merge_with() {
    for case in CASES {
        let Expect::Doc(want) = case.expect else {
            continue;
        };
        let mut acc = Value::Object(Default::default());
        for layer in case.layers {
            knf_core::merge_into(&mut acc, ir(layer), &case.opts).expect(case.name);
        }
        assert_eq!(acc, ir(want), "case `{}`", case.name);
    }
}

/// [`knf_core::merge`] is last-wins [`merge_with`].
#[test]
fn merge_is_last_wins() {
    for case in CASES {
        if case.opts != MergeOptions::LAST_WINS {
            continue;
        }
        let Expect::Doc(want) = case.expect else {
            continue;
        };
        let got = knf_core::merge(case.layers.iter().map(|s| ir(s))).expect(case.name);
        assert_eq!(got, ir(want), "case `{}`", case.name);
    }
}

/// The conflict path is relative to the document root, so an error at the root
/// itself has an empty path rather than a bogus key.
#[test]
fn root_level_conflict_has_empty_path() {
    let mut base = Value::Object(Default::default());
    let err =
        knf_core::merge_into(&mut base, Value::Bool(true), &MergeOptions::STRICT).unwrap_err();
    assert_eq!(err.path(), &[] as &[String]);
    assert!(err.to_string().contains("<root>"), "{err}");
}

/// Error text carries both kinds, which is what makes the message actionable
/// without the user re-running with more verbosity.
#[test]
fn conflict_message_names_both_kinds() {
    let err = merge_with(
        [ir(r#"{"a":{"b":1}}"#), ir(r#"{"a":5}"#)],
        &MergeOptions::STRICT,
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "type conflict at `a`: object would be replaced by number"
    );
}

/// A datetime is not a string. This is the one kind a JSON literal cannot
/// express, and the reason `--strict` still catches a JSON string landing on
/// top of a TOML datetime now that both share one walk.
#[test]
fn datetime_conflicts_with_string_under_strict() {
    let mut base = Value::Object(
        [(
            "a".to_string(),
            Value::Datetime("1979-05-27T07:32:00Z".to_string()),
        )]
        .into_iter()
        .collect(),
    );
    let err = knf_core::merge_into(
        &mut base,
        ir(r#"{"a":"1979-05-27T07:32:00Z"}"#),
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
