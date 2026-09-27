//! Interpolation is the identity on documents without `$`.

use knf::{ConfigFormat, ConfigValue, Env, interpolate};
use proptest::prelude::*;

/// An empty environment.
struct NoEnv;

impl Env for NoEnv {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }
}

/// Arbitrary native values without `$` or floats (so equality is total).
macro_rules! properties {
    ($value:ty, $map:ty) => {
        type Value = $value;
        type Map = $map;
        fn arb_value() -> impl Strategy<Value = Value> {
            let leaf = prop_oneof![
                Just(Value::parse_inline("null".into())),
                any::<bool>().prop_map(|b| Value::parse_inline(b.to_string())),
                any::<i64>().prop_map(|n| Value::parse_inline(n.to_string())),
                "[a-z{} :.]{0,6}".prop_map(Value::String),
                Just(Value::parse_inline("1979-05-27T07:32:00Z".into())),
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
                .prop_map(|entries| Value::object(entries.into_iter().collect::<Map>()))
        }

        fn arb_doc() -> impl Strategy<Value = Value> {
            arb_object(arb_value())
        }

        proptest! {
            /// A document with no `$` is unchanged, even with `{}` and `:`.
            #[test]
            fn a_document_without_a_dollar_is_unchanged(doc in arb_doc()) {
                prop_assert_eq!(interpolate(doc.clone(), &NoEnv).expect("nothing to resolve"), doc);
            }
        }
    };
}

mod json {
    use super::*;
    properties!(serde_json::Value, serde_json::Map<String, serde_json::Value>);
}
mod toml {
    use super::*;
    properties!(::toml::Value, ::toml::Table);
}
