//! Table-driven merge tests. Adding a case is one line in `CASES`.

mod common;

use common::ir;
use knf_core::{MergeError, MergeOptions, Rules, Strategy, Value, merge_with};

use ErrKind::{AppendKind, Locked, TypeConflict};
use Expect::{Doc, Error};
use Strategy::{Append, Fail, Replace};

struct Case {
    name: &'static str,
    /// JSON literals, merged left to right.
    layers: &'static [&'static str],
    strict: bool,
    /// Dotted paths and their strategies, in no meaningful order.
    rules: &'static [(&'static str, Strategy)],
    expect: Expect,
}

enum Expect {
    /// A JSON literal the merge must equal.
    Doc(&'static str),
    /// This kind of error, at this dotted key path.
    Error(ErrKind, &'static str),
}

/// Which [`MergeError`] a case expects, without repeating its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrKind {
    TypeConflict,
    Locked,
    AppendKind,
}

const fn ok(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        strict: false,
        rules: &[],
        expect: Doc(doc),
    }
}

const fn strict(name: &'static str, layers: &'static [&'static str], doc: &'static str) -> Case {
    Case {
        name,
        layers,
        strict: true,
        rules: &[],
        expect: Doc(doc),
    }
}

const fn conflict(name: &'static str, layers: &'static [&'static str], path: &'static str) -> Case {
    Case {
        name,
        layers,
        strict: true,
        rules: &[],
        expect: Error(TypeConflict, path),
    }
}

const fn ruled(
    name: &'static str,
    layers: &'static [&'static str],
    rules: &'static [(&'static str, Strategy)],
    expect: Expect,
) -> Case {
    Case {
        name,
        layers,
        strict: false,
        rules,
        expect,
    }
}

const fn strict_ruled(
    name: &'static str,
    layers: &'static [&'static str],
    rules: &'static [(&'static str, Strategy)],
    expect: Expect,
) -> Case {
    Case {
        name,
        layers,
        strict: true,
        rules,
        expect,
    }
}

/// Rules are validated once per case; every `CASES` entry must be a legal set.
fn options(case: &Case) -> MergeOptions {
    let rules = case.rules.iter().map(|(dotted, strategy)| {
        (
            dotted.split('.').map(str::to_string).collect::<Vec<_>>(),
            *strategy,
        )
    });
    MergeOptions {
        strict: case.strict,
        rules: match case.rules.is_empty() {
            true => None,
            false => Some(Rules::build(rules).expect(case.name)),
        },
    }
}

fn err_kind(err: &MergeError) -> ErrKind {
    match err {
        MergeError::TypeConflict { .. } => TypeConflict,
        MergeError::Locked { .. } => Locked,
        MergeError::AppendKind { .. } => AppendKind,
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

    // --- per-path strategies ------------------------------------------------
    ruled("append concatenates base ++ overlay", &[r#"{"a":[1,2]}"#, r#"{"a":[3]}"#], &[("a", Append)], Doc(r#"{"a":[1,2,3]}"#)),
    ruled("append across three layers", &[r#"{"a":[1]}"#, r#"{"a":[2]}"#, r#"{"a":[3]}"#], &[("a", Append)], Doc(r#"{"a":[1,2,3]}"#)),
    // The seed is empty, so the first layer *inserts*: one layer must never
    // double its own array.
    ruled("append inserts where the key is absent", &[r#"{"a":[1,2]}"#], &[("a", Append)], Doc(r#"{"a":[1,2]}"#)),
    ruled("append inserts into a missing branch", &[r#"{"b":1}"#, r#"{"a":[1]}"#], &[("a", Append)], Doc(r#"{"b":1,"a":[1]}"#)),
    ruled("append needs an array on the left", &[r#"{"a":1}"#, r#"{"a":[2]}"#], &[("a", Append)], Error(AppendKind, "a")),
    ruled("append needs an array on the right", &[r#"{"a":[1]}"#, r#"{"a":2}"#], &[("a", Append)], Error(AppendKind, "a")),
    ruled("append applies at depth", &[r#"{"a":{"b":[1]}}"#, r#"{"a":{"b":[2]}}"#], &[("a.b", Append)], Doc(r#"{"a":{"b":[1,2]}}"#)),

    // Replace is the whole point: object over object stops recursing, so keys
    // the overlay omits are gone.
    ruled("replace does not recurse into objects", &[r#"{"a":{"x":1,"y":2}}"#, r#"{"a":{"y":9}}"#], &[("a", Replace)], Doc(r#"{"a":{"y":9}}"#)),
    ruled("replace still inserts where the key is absent", &[r#"{"b":1}"#, r#"{"a":{"y":9}}"#], &[("a", Replace)], Doc(r#"{"b":1,"a":{"y":9}}"#)),
    ruled("replace at depth leaves the parent merging", &[r#"{"a":{"b":{"x":1},"c":{"x":1}}}"#, r#"{"a":{"b":{"y":2},"c":{"y":2}}}"#], &[("a.b", Replace)], Doc(r#"{"a":{"b":{"y":2},"c":{"x":1,"y":2}}}"#)),

    // Fail pins a path to whichever layer defined it first.
    ruled("fail allows the first insert", &[r#"{"a":1}"#], &[("a", Fail)], Doc(r#"{"a":1}"#)),
    ruled("fail rejects the second layer", &[r#"{"a":1}"#, r#"{"a":2}"#], &[("a", Fail)], Error(Locked, "a")),
    ruled("fail rejects an identical value too", &[r#"{"a":1}"#, r#"{"a":1}"#], &[("a", Fail)], Error(Locked, "a")),
    ruled("fail reports a nested path", &[r#"{"a":{"b":1}}"#, r#"{"a":{"b":2}}"#], &[("a.b", Fail)], Error(Locked, "a.b")),

    // A rule is exact: siblings and parents merge as usual.
    ruled("a rule at a.b leaves a.c alone", &[r#"{"a":{"b":[1],"c":[1]}}"#, r#"{"a":{"b":[2],"c":[2]}}"#], &[("a.b", Append)], Doc(r#"{"a":{"b":[1,2],"c":[2]}}"#)),
    ruled("an unrelated rule changes nothing", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], &[("zz", Replace)], Doc(r#"{"a":{"x":1,"y":2}}"#)),

    // --strict is orthogonal: it kind-checks wherever a replacement happens,
    // and --replace makes object-over-object one of those places.
    strict_ruled("strict kind-checks under replace", &[r#"{"a":{"x":1}}"#, r#"{"a":5}"#], &[("a", Replace)], Error(TypeConflict, "a")),
    strict_ruled("strict allows a same-kind replace", &[r#"{"a":{"x":1}}"#, r#"{"a":{"y":2}}"#], &[("a", Replace)], Doc(r#"{"a":{"y":2}}"#)),
    strict_ruled("strict has nothing to check on append", &[r#"{"a":[1]}"#, r#"{"a":[2]}"#], &[("a", Append)], Doc(r#"{"a":[1,2]}"#)),
];

#[test]
fn table() {
    for case in CASES {
        let layers = case.layers.iter().map(|s| ir(s));
        let got = merge_with(layers, &options(case));

        match (&case.expect, got) {
            (Doc(want), Ok(got)) => {
                assert_eq!(got, ir(want), "case `{}`", case.name);
            }
            (Doc(want), Err(e)) => {
                panic!("case `{}`: expected {want}, got error: {e}", case.name);
            }
            (Error(kind, want), Err(e)) => {
                assert_eq!(
                    err_kind(&e),
                    *kind,
                    "case `{}`: wrong error: {e}",
                    case.name
                );
                assert_eq!(
                    e.path().join("."),
                    *want,
                    "case `{}`: wrong path",
                    case.name
                );
            }
            (Error(kind, want), Ok(got)) => {
                panic!(
                    "case `{}`: expected {kind:?} at `{want}`, merged to {got:?}",
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
        let Doc(want) = case.expect else {
            continue;
        };
        let opts = options(case);
        let mut acc = Value::Object(Default::default());
        for layer in case.layers {
            knf_core::merge_into(&mut acc, ir(layer), &opts).expect(case.name);
        }
        assert_eq!(acc, ir(want), "case `{}`", case.name);
    }
}

/// [`knf_core::merge`] is last-wins [`merge_with`].
#[test]
fn merge_is_last_wins() {
    for case in CASES {
        if case.strict || !case.rules.is_empty() {
            continue;
        }
        let Doc(want) = case.expect else {
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
        other => panic!("expected a type conflict, got {other}"),
    }
}

/// The two rule-driven errors, whose text is what a user acts on. Neither may
/// name a flag: `--append` and `--fail` are the caller's words, not the core's.
#[test]
fn rule_error_messages_carry_paths_only() {
    let locked = merge_with(
        [ir(r#"{"db":{"host":"a"}}"#), ir(r#"{"db":{"host":"b"}}"#)],
        &MergeOptions {
            strict: false,
            rules: Some(Rules::build([(vec!["db".into(), "host".into()], Fail)]).expect("valid")),
        },
    )
    .unwrap_err();
    assert_eq!(
        locked.to_string(),
        "`db.host` is locked: an earlier layer already set it"
    );

    let bad_append = merge_with(
        [
            ir(r#"{"plugins":"auth"}"#),
            ir(r#"{"plugins":["metrics"]}"#),
        ],
        &MergeOptions {
            strict: false,
            rules: Some(Rules::build([(vec!["plugins".into()], Append)]).expect("valid")),
        },
    )
    .unwrap_err();
    assert_eq!(
        bad_append.to_string(),
        "cannot append array to string at `plugins`"
    );
}
