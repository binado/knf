//! The property the opt-in flag exists to protect.

use knf_core::{Map, Number, Value};
use knf_interp::{Env, EnvValue, interpolate};
use proptest::prelude::*;

/// An environment that has nothing in it. The property below never reaches a
/// lookup — no generated string contains a `$` — so an empty one is the honest
/// stub rather than a limitation.
struct NoEnv;

impl Env for NoEnv {
    fn lookup(&self, _name: &str) -> Option<EnvValue> {
        None
    }
}

/// Arbitrary IR values over an alphabet with no `$` in it, and no floats — the
/// same exclusion `knf-core`'s `props.rs` makes, so equality stays total.
///
/// This is a second, smaller copy of that generator on purpose: test-only
/// strategies do not cross crate boundaries.
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| Value::Number(Number::I64(n))),
        "[a-z{} :.]{0,6}".prop_map(Value::String),
        Just(Value::Datetime("1979-05-27T07:32:00Z".to_string())),
    ];
    leaf.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
            arb_object(inner),
        ]
    })
}

fn arb_object(inner: impl Strategy<Value = Value>) -> impl Strategy<Value = Value> {
    prop::collection::vec(("[a-c]{1,2}", inner), 0..3)
        .prop_map(|entries| Value::Object(entries.into_iter().collect::<Map>()))
}

fn arb_doc() -> impl Strategy<Value = Value> {
    arb_object(arb_value())
}

proptest! {
    /// A document with no `$` in it is unchanged by a pass. Braces and colons
    /// *are* in the alphabet, so `{}` and `a:b` have to survive on their own —
    /// only `$` starts a reference.
    ///
    /// This is the identity `--interpolate` is layered on top of, and the reason
    /// the flag is opt-in: `knf a.json` stays a byte-level no-op, and a document
    /// full of `${...}` bound for compose or Actions passes through untouched
    /// unless the user asked otherwise.
    #[test]
    fn a_document_without_a_dollar_is_unchanged(doc in arb_doc()) {
        prop_assert_eq!(interpolate(doc.clone(), &NoEnv).expect("nothing to resolve"), doc);
    }
}
