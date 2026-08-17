//! Resolution behaviour, over a stub [`Env`] so nothing here reads the ambient
//! environment.

use std::collections::HashMap;

use knf_core::{Map, Number, Value};

use super::*;

/// A `HashMap` environment. `typed` is filled by a miniature of the caller's
/// JSON-or-string rule — enough to exercise the raw/typed split without dragging
/// `serde_json` into this crate's tree.
#[derive(Default)]
struct StubEnv(HashMap<String, String>);

impl StubEnv {
    fn new(vars: &[(&str, &str)]) -> Self {
        Self(
            vars.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
}

impl Env for StubEnv {
    fn lookup(&self, name: &str) -> Option<EnvValue> {
        let raw = self.0.get(name)?.clone();
        let typed = match raw.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            text => match text.parse::<i64>() {
                Ok(n) => Value::Number(Number::I64(n)),
                Err(_) => Value::String(raw.clone()),
            },
        };
        Some(EnvValue { raw, typed })
    }
}

fn obj(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<Map>(),
    )
}

fn s(text: &str) -> Value {
    Value::String(text.to_string())
}

fn n(v: i64) -> Value {
    Value::Number(Number::I64(v))
}

/// Interpolate with no environment at all.
fn interp(doc: Value) -> Result<Value, InterpError> {
    interpolate(doc, &StubEnv::default())
}

fn interp_env(doc: Value, vars: &[(&str, &str)]) -> Result<Value, InterpError> {
    interpolate(doc, &StubEnv::new(vars))
}

fn err(doc: Value) -> String {
    interp(doc).expect_err("should fail").to_string()
}

fn err_env(doc: Value, vars: &[(&str, &str)]) -> String {
    interp_env(doc, vars).expect_err("should fail").to_string()
}

// --- positions ------------------------------------------------------------

/// The whole-string rule: the reference *is* the value, so it keeps its type.
#[test]
fn a_whole_string_reference_takes_the_referents_type() {
    let doc = obj(vec![("p", n(8080)), ("port", s("${p}"))]);
    assert_eq!(
        interp(doc).unwrap(),
        obj(vec![("p", n(8080)), ("port", n(8080))])
    );
}

#[test]
fn an_embedded_reference_stringifies() {
    let doc = obj(vec![
        ("host", s("db")),
        ("p", n(8080)),
        ("url", s("http://${host}:${p}/health")),
    ]);
    let out = interp(doc).unwrap();
    let Value::Object(map) = &out else {
        panic!("expected an object");
    };
    assert_eq!(map["url"], s("http://db:8080/health"));
}

/// A float embedded in text must render as the emitters would write it.
#[test]
fn an_embedded_float_keeps_its_point() {
    let doc = obj(vec![
        ("v", Value::Number(Number::F64(1.0))),
        ("tag", s("v${v}")),
    ]);
    let out = interp(doc).unwrap();
    let Value::Object(map) = &out else {
        panic!("expected an object");
    };
    assert_eq!(map["tag"], s("v1.0"));
}

#[test]
fn a_datetime_splices_as_its_source_spelling() {
    let doc = obj(vec![
        ("d", Value::Datetime("1979-05-27T07:32:00Z".into())),
        ("stamp", s("at ${d}")),
        ("copy", s("${d}")),
    ]);
    let out = interp(doc).unwrap();
    let Value::Object(map) = &out else {
        panic!("expected an object");
    };
    assert_eq!(map["stamp"], s("at 1979-05-27T07:32:00Z"));
    // Whole-string keeps the *type*: a copied datetime, not a string.
    assert_eq!(map["copy"], Value::Datetime("1979-05-27T07:32:00Z".into()));
}

// --- escapes and non-references -------------------------------------------

#[test]
fn dollar_dollar_escapes_to_one_dollar() {
    let doc = obj(vec![("literal", s("$${NOT_A_REF}"))]);
    assert_eq!(
        interp(doc).unwrap(),
        obj(vec![("literal", s("${NOT_A_REF}"))])
    );
}

#[test]
fn a_bare_dollar_is_left_alone() {
    let doc = obj(vec![("price", s("USD $5")), ("plain", s("no refs here"))]);
    assert_eq!(interp(doc.clone()).unwrap(), doc);
}

/// Keys are never interpolated; values only.
#[test]
fn keys_are_not_interpolated() {
    let doc = obj(vec![("a", s("x")), ("${a}", n(1))]);
    assert_eq!(interp(doc.clone()).unwrap(), doc);
}

// --- transitivity, order, cycles ------------------------------------------

#[test]
fn references_resolve_transitively() {
    let doc = obj(vec![
        ("a", s("${b}")),
        ("b", s("${c}")),
        ("c", s("deep")),
        ("joined", s("<${a}>")),
    ]);
    assert_eq!(
        interp(doc).unwrap(),
        obj(vec![
            ("a", s("deep")),
            ("b", s("deep")),
            ("c", s("deep")),
            ("joined", s("<deep>")),
        ])
    );
}

/// Declaration order must not matter: a forward reference resolves the same as
/// a backward one.
#[test]
fn a_forward_reference_resolves_like_a_backward_one() {
    let forward = interp(obj(vec![("a", s("${b}")), ("b", s("v"))])).unwrap();
    let backward = interp(obj(vec![("b", s("v")), ("a", s("${b}"))])).unwrap();
    assert_eq!(forward, obj(vec![("a", s("v")), ("b", s("v"))]));
    assert_eq!(backward, obj(vec![("b", s("v")), ("a", s("v"))]));
}

#[test]
fn a_direct_cycle_is_reported_as_a_chain() {
    let doc = obj(vec![("a", s("${b}")), ("b", s("${a}"))]);
    assert_eq!(err(doc), "reference cycle: `a` -> `b` -> `a`");
}

#[test]
fn a_self_reference_is_a_cycle() {
    assert_eq!(
        err(obj(vec![("a", s("${a}"))])),
        "reference cycle: `a` -> `a`"
    );
}

/// A reference back into the container currently being resolved closes a cycle
/// through the container, and the chain says so.
#[test]
fn a_cycle_through_a_container_names_every_hop() {
    let doc = obj(vec![("a", obj(vec![("b", s("${a}"))]))]);
    assert_eq!(err(doc), "reference cycle: `a` -> `a.b` -> `a`");
}

// --- containers -----------------------------------------------------------

/// Whole-string: allowed, and the alias is the *resolved* subtree.
#[test]
fn a_whole_string_container_reference_aliases_a_resolved_subtree() {
    let doc = obj(vec![
        ("host", s("db.internal")),
        (
            "primary",
            obj(vec![("host", s("${host}")), ("port", n(5432))]),
        ),
        ("replica", s("${primary}")),
    ]);
    let out = interp(doc).unwrap();
    let Value::Object(map) = &out else {
        panic!("expected an object");
    };
    let expected = obj(vec![("host", s("db.internal")), ("port", n(5432))]);
    assert_eq!(map["primary"], expected);
    assert_eq!(map["replica"], expected);
}

#[test]
fn an_embedded_container_reference_is_rejected() {
    let doc = obj(vec![
        ("db", obj(vec![("host", s("x"))])),
        ("xs", Value::Array(vec![n(1)])),
        ("url", s("http://${db}/")),
        ("tag", s("<${xs}>")),
    ]);
    assert_eq!(
        err(doc),
        "reference cannot be rendered into a string\n\
         \x20 --> url: `db` is an object\n\
         \x20 --> tag: `xs` is an array"
    );
}

#[test]
fn an_embedded_null_reference_is_rejected() {
    let doc = obj(vec![("n", Value::Null), ("t", s("[${n}]"))]);
    assert_eq!(
        err(doc),
        "reference cannot be rendered into a string\n  --> t: `n` is a null"
    );
}

/// A null referent is fine as a whole string — it becomes an ordinary null, and
/// meets the existing TOML-null error (and its existing escape) at emit.
#[test]
fn a_whole_string_null_reference_is_an_ordinary_null() {
    let doc = obj(vec![("n", Value::Null), ("copy", s("${n}"))]);
    assert_eq!(
        interp(doc).unwrap(),
        obj(vec![("n", Value::Null), ("copy", Value::Null)])
    );
}

// --- environment ----------------------------------------------------------

#[test]
fn env_is_typed_whole_string_and_raw_embedded() {
    let doc = obj(vec![
        ("port", s("${env:PORT}")),
        ("url", s("http://localhost:${env:PORT}/health")),
    ]);
    assert_eq!(
        interp_env(doc, &[("PORT", "8080")]).unwrap(),
        obj(vec![
            ("port", n(8080)),
            ("url", s("http://localhost:8080/health")),
        ])
    );
}

/// The raw form is spliced verbatim: parsing it and printing it back is the one
/// thing guaranteed to be able to corrupt it.
#[test]
fn an_embedded_variable_splices_its_raw_text() {
    let doc = obj(vec![("v", s("[${env:V}]"))]);
    assert_eq!(
        interp_env(doc, &[("V", "1.500")]).unwrap(),
        obj(vec![("v", s("[1.500]"))])
    );
}

/// Environment values are terminal — a variable holding `${x}` is text, not a
/// second round of resolution.
#[test]
fn environment_values_are_not_rescanned() {
    let doc = obj(vec![("x", s("secret")), ("v", s("${env:V}"))]);
    assert_eq!(
        interp_env(doc, &[("V", "${x}")]).unwrap(),
        obj(vec![("x", s("secret")), ("v", s("${x}"))])
    );
}

#[test]
fn an_unset_variable_is_unresolved() {
    let doc = obj(vec![("port", s("${env:PORT}")), ("url", s("x${env:HOST}"))]);
    assert_eq!(
        err(doc),
        "unresolved reference\n\
         \x20 --> port: `env:PORT`\n\
         \x20 --> url: `env:HOST`"
    );
}

/// `env:` is a prefix match, not a split on the first `:`, so a key containing a
/// colon is an ordinary key.
#[test]
fn a_colon_in_a_key_is_not_a_namespace() {
    let doc = obj(vec![("a:b", n(1)), ("v", s("${a:b}"))]);
    assert_eq!(interp(doc).unwrap(), obj(vec![("a:b", n(1)), ("v", n(1))]));
}

/// The same rule keeps a dotted path with a colon in it from reading as a
/// namespace nobody named.
#[test]
fn a_dotted_path_with_a_colon_reports_as_a_key() {
    let doc = obj(vec![("v", s("${db.host:port}"))]);
    assert_eq!(err(doc), "unresolved reference\n  --> v: `db.host:port`");
}

// --- problems -------------------------------------------------------------

#[test]
fn missing_keys_are_collected_with_their_paths() {
    let doc = obj(vec![
        ("server", obj(vec![("url", s("${db.hostname}"))])),
        ("tags", Value::Array(vec![s("${env:REGION}")])),
    ]);
    assert_eq!(
        err(doc),
        "unresolved reference\n\
         \x20 --> server.url: `db.hostname`\n\
         \x20 --> tags[0]: `env:REGION`"
    );
}

#[test]
fn syntax_problems_are_collected() {
    let doc = obj(vec![
        ("a", s("${b")),
        ("b", s("${}")),
        ("c", s("${env:}")),
        ("d", s("${x..y}")),
        ("e", s("${p${q}}")),
    ]);
    assert_eq!(
        err(doc),
        "invalid reference\n\
         \x20 --> a: unterminated `${` at offset 0\n\
         \x20 --> b: empty reference `${}`\n\
         \x20 --> c: empty variable name in `${env:}`\n\
         \x20 --> d: empty segment in reference `${x..y}`\n\
         \x20 --> e: nested `${` in `${p${q}`"
    );
}

/// Every kind in one run, so a user fixing a config learns everything at once.
#[test]
fn every_kind_of_problem_is_reported_in_one_run() {
    let doc = obj(vec![
        ("db", obj(vec![("host", s("x"))])),
        ("bad", s("${}")),
        ("gone", s("${nope}")),
        ("url", s("http://${db}/")),
    ]);
    assert_eq!(
        err(doc),
        "invalid reference\n\
         \x20 --> bad: empty reference `${}`\n\
         unresolved reference\n\
         \x20 --> gone: `nope`\n\
         reference cannot be rendered into a string\n\
         \x20 --> url: `db` is an object"
    );
}

/// Several references to one broken key report once per *site*, not once per
/// visit: memoization is what keeps a widely-referenced value from flooding the
/// report.
#[test]
fn a_problem_is_reported_once_per_site() {
    let doc = obj(vec![
        ("a", s("${gone}")),
        ("b", s("${a}")),
        ("c", s("${a}")),
    ]);
    assert_eq!(err(doc), "unresolved reference\n  --> a: `gone`");
}

/// `${env:}` is malformed, not a lookup of the empty name — so an environment
/// that somehow holds one cannot make it resolve.
#[test]
fn an_empty_variable_name_is_syntax_not_a_lookup() {
    assert_eq!(
        err_env(obj(vec![("a", s("${env:}"))]), &[("", "x")]),
        "invalid reference\n  --> a: empty variable name in `${env:}`"
    );
}

// --- shape ----------------------------------------------------------------

/// References may live inside arrays even though they can never point into one.
#[test]
fn references_resolve_inside_arrays() {
    let doc = obj(vec![
        ("region", s("eu")),
        ("tags", Value::Array(vec![s("${region}"), s("a-${region}")])),
    ]);
    assert_eq!(
        interp(doc).unwrap(),
        obj(vec![
            ("region", s("eu")),
            ("tags", Value::Array(vec![s("eu"), s("a-eu")])),
        ])
    );
}

#[test]
fn key_order_survives_a_pass() {
    let doc = obj(vec![
        ("zebra", n(1)),
        ("apple", n(2)),
        ("middle", s("${zebra}")),
    ]);
    let Value::Object(map) = interp(doc).unwrap() else {
        panic!("expected an object");
    };
    assert_eq!(map.keys().collect::<Vec<_>>(), ["zebra", "apple", "middle"]);
}
