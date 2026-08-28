//! What a downstream crate can actually name, checked by being one.
//!
//! `knf-config`'s own integration tests cannot catch a missing `pub use`: an
//! integration test sees its package's dependencies, so `knf_core::Number`
//! stays reachable there whether or not `knf-config` re-exports it. This
//! package is the witness that costs nothing to keep — `knf-cli` depends on
//! `knf-config` and on neither `knf-core` nor `knf-interp`, so every path below
//! resolves through the re-exports or not at all.
//!
//! The rule this pins is not "re-export what the signatures mention" but
//! "re-export what a caller has to write down". Reaching *through* a public
//! type counts: `Number` is the payload of `Value::Number`, `Cycle` and
//! `Syntax` of two `InterpError` arms, and `render_path` is the only renderer
//! for the `Seg`s a `RefPath` hands out. Each was missing when this file was
//! written, and each fails to compile if it goes missing again.

use std::str::FromStr;

use knf::{
    BadDatetime, Cycle, Env, EnvValue, Format, IntegerOutOfRange, InterpError, LoadError, Map,
    MergeError, MergeOpts, NonFiniteFloat, NullInToml, Number, PathError, PathLeaf, Problem,
    RefPath, RuleError, RuleErrors, Rules, STDIN, Seg, Strategy, Syntax, TomlError, Value,
    json_or_string, load_layers, merge, merge_layers, merge_with_env, render_path,
};

/// The types a caller writes into its own signatures, named in signatures.
mod named {
    use super::*;

    pub fn number(n: &Number) -> String {
        format!("{n:?}")
    }

    pub fn cycle(c: &Cycle) -> String {
        c.to_string()
    }

    pub fn syntax(s: &Syntax) -> String {
        s.to_string()
    }

    pub fn path(segs: &[Seg]) -> String {
        render_path(segs)
    }

    pub fn knobs(_: &Map, _: &MergeOpts, _: &Rules, _: Strategy, _: Format, _: &dyn Env) {}
}

/// Type position is the whole requirement for an error a caller matches on
/// rather than builds, so one field of each is the witness. Never constructed —
/// several of these have no public constructor, by design.
#[allow(dead_code)]
struct EveryError {
    merge: MergeError,
    rule: RuleError,
    rules: RuleErrors,
    path: PathError,
    load: LoadError,
    null: NullInToml,
    integer: IntegerOutOfRange,
    non_finite: NonFiniteFloat,
    interp: InterpError,
    problem: Problem,
}

fn dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).expect("writing a fixture");
    }
    dir
}

/// Every re-export, exercised the way a consumer would reach it.
#[test]
fn the_public_surface_is_nameable_without_knf_core_or_knf_interp() {
    // The path vocabulary, and the renderer for the segments it hands out.
    let refpath = RefPath::from_str("servers[0].host").expect("a well-formed path");
    assert_eq!(named::path(refpath.segs()), "servers[0].host");
    let keys = RefPath::from_keys(vec!["db".into(), "host".into()]).expect("keys are a path");
    assert_eq!(keys.try_into_keys().expect("no indices"), ["db", "host"]);
    assert!(matches!(
        refpath.try_into_keys(),
        Err(PathError::IndexInKeyPath { .. })
    ));

    // `--set`-shaped input: the leaf parser and the typing rule behind it.
    let leaf = PathLeaf::<String>::from_str("port=8080").expect("a well-formed expression");
    assert_eq!(leaf.path().len(), 1);
    assert_eq!(json_or_string("8080".to_string()), serde_json::json!(8080));

    // The merge knobs, and a merged document whose numbers are `Number`s.
    let opts = MergeOpts {
        rules: Rules::build(vec![(vec!["xs".to_string()], Strategy::Append)]).expect("one rule"),
        ..MergeOpts::default()
    };
    named::knobs(
        &Map::new(),
        &opts,
        &Rules::default(),
        Strategy::Replace,
        Format::Json,
        &knf::ProcessEnv,
    );

    let tree = dir(&[
        ("a.json", r#"{"xs":[1],"port":8080}"#),
        ("b.json", r#"{"xs":[2]}"#),
    ]);
    let merged = merge(
        &[tree.path().join("a.json"), tree.path().join("b.json")],
        opts,
    )
    .expect("the rule appends");
    let Value::Object(map) = &merged else {
        panic!("the top level is an object")
    };
    let Value::Number(port) = &map["port"] else {
        panic!("8080 is a number")
    };
    assert_eq!(named::number(port), named::number(&Number::from_u64(8080)));

    // Interpolation reaches through `InterpError` to `Cycle` and `Syntax`.
    let tree = dir(&[("cycle.json", r#"{"a":"${b}","b":"${a}"}"#)]);
    let err = merge_with_env(
        &[tree.path().join("cycle.json")],
        MergeOpts {
            interpolate: true,
            ..MergeOpts::default()
        },
        &StubEnv,
    )
    .expect_err("a is b is a");
    match err.downcast_ref::<InterpError>().expect("a typed error") {
        InterpError::Cycle(cycle) => assert!(named::cycle(cycle).starts_with("reference cycle:")),
        other => panic!("expected a cycle, got {other}"),
    }

    let tree = dir(&[("syntax.json", r#"{"a":"${}"}"#)]);
    let err = merge_with_env(
        &[tree.path().join("syntax.json")],
        MergeOpts {
            interpolate: true,
            ..MergeOpts::default()
        },
        &StubEnv,
    )
    .expect_err("`${}` names nothing");
    match err.downcast_ref::<InterpError>().expect("a typed error") {
        InterpError::Problems(problems) => match &problems[0] {
            Problem::Syntax { error, .. } => {
                assert_eq!(named::syntax(error), Syntax::EmptyRef.to_string())
            }
            other => panic!("expected a syntax problem, got {other:?}"),
        },
        other => panic!("expected problems, got {other}"),
    }

    // The two typed errors a caller matches on rather than reads.
    let tree = dir(&[
        ("layer", "{}"),
        ("null.json", r#"{"a":null}"#),
        ("big.json", r#"{"id":10000000000000000001}"#),
        ("inf.toml", "timeout = inf\n"),
    ]);
    let err = merge(&[tree.path().join("layer")], MergeOpts::default())
        .expect_err("no extension, no format");
    assert!(matches!(
        err.downcast_ref::<LoadError>(),
        Some(LoadError::UnknownExtension { .. })
    ));
    let (layers, formats) =
        load_layers(&[tree.path().join("null.json")], None).expect("json by extension");
    assert_eq!(formats, vec![Format::Json]);
    let merged = merge_layers(layers, MergeOpts::default(), &StubEnv).expect("one layer");
    let err = knf::format::emit(merged, Format::Toml, true, None).expect_err("a null");
    let Some(TomlError::Null(report)) = err.downcast_ref::<TomlError>() else {
        panic!("expected the null variant, got {err}")
    };
    let _: &NullInToml = report;

    // The sibling variant. Unreachable from argv — nothing the CLI can spell
    // builds a `Value::Datetime` — but a caller assembling a layer in memory can,
    // so both it and the report it carries have to be nameable from here.
    let mut hand_built = Map::new();
    hand_built.insert("d".to_string(), Value::Datetime("nope".to_string()));
    let err = knf::format::emit(Value::Object(hand_built), Format::Toml, true, None)
        .expect_err("`nope` is not a datetime");
    let Some(TomlError::Datetime(report)) = err.downcast_ref::<TomlError>() else {
        panic!("expected the datetime variant, got {err}")
    };
    let _: &BadDatetime = report;

    // The third variant, and the one a *document* reaches: an integer past
    // `i64::MAX` has no TOML spelling, and `Number::U64` is the reason it survived
    // the merge intact enough to say so.
    let (layers, _) = load_layers(&[tree.path().join("big.json")], None).expect("json");
    let merged = merge_layers(layers, MergeOpts::default(), &StubEnv).expect("one layer");
    let err = knf::format::emit(merged, Format::Toml, true, None).expect_err("past i64::MAX");
    let Some(TomlError::Integer(report)) = err.downcast_ref::<TomlError>() else {
        panic!("expected the integer variant, got {err}")
    };
    let _: &IntegerOutOfRange = report;

    // The impossibility on the other side of the boundary, and the reason it is a
    // bare type rather than a variant: JSON has exactly one. Reached from a TOML
    // input, whose grammar spells the `inf` that JSON's cannot.
    let (layers, _) = load_layers(&[tree.path().join("inf.toml")], None).expect("toml");
    let merged = merge_layers(layers, MergeOpts::default(), &StubEnv).expect("one layer");
    let err = knf::format::emit(merged, Format::Json, true, None).expect_err("inf");
    let report = err
        .downcast_ref::<NonFiniteFloat>()
        .expect("the typed error survives");
    let _: &NonFiniteFloat = report;

    assert_eq!(STDIN, "-");
}

/// `Env` and `EnvValue` are the seam a caller supplies, so both must be
/// nameable from here — a stub that reads no process state is the proof.
struct StubEnv;

impl Env for StubEnv {
    fn lookup(&self, name: &str) -> Option<EnvValue> {
        (name == "PORT").then(|| EnvValue {
            raw: "8080".to_string(),
            typed: knf::value::from_json(json_or_string("8080".to_string())),
        })
    }
}
