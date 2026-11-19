//! Reference-aware merging agrees with native merge, and a reference merges
//! exactly as the value it names.

use knf::{
    ConfigFormat, ConfigValue, Env, InterpOptions, MergeOptions, interpolate, merge,
    merge_interpolate,
};
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
            #[test]
            fn layers_without_references_fold_like_native_merge(
                layers in prop::collection::vec(arb_doc(), 0..4),
                shallow in prop::option::of(prop::sample::select(vec!["a", "a.b", "a.*", "*", "{a,b}.c"])),
            ) {
                let selector: Option<knf::glob::KeyGlobPattern> = shallow.map(|pattern| pattern.parse().unwrap());
                let expected = merge(layers.clone(), &MergeOptions { shallow: selector.clone() });
                let out = merge_interpolate(layers, None, &NoEnv, &InterpOptions { shallow: selector }).unwrap();
                prop_assert_eq!(out, expected);
            }

            #[test]
            fn a_reference_merges_exactly_like_its_referent(
                left in arb_value(),
                right in arb_value(),
                reference_right in any::<bool>(),
                shallow in prop::option::of(prop::sample::select(vec!["derived", "derived.a", "derived.*", "*"])),
            ) {
                let wrap = |value| Value::object([("derived".to_owned(), value)].into_iter().collect::<Map>());
                let options = InterpOptions { shallow: shallow.map(|pattern| pattern.parse().unwrap()) };
                let expected = merge_interpolate([wrap(left.clone()), wrap(right.clone())], None, &NoEnv, &options).unwrap();
                let (referent, layers) = if reference_right {
                    (right, [wrap(left), wrap(Value::string("${referent}".into()))])
                } else {
                    (left, [wrap(Value::string("${referent}".into())), wrap(right)])
                };
                let context = Value::object([("referent".to_owned(), referent)].into_iter().collect::<Map>());
                let out = merge_interpolate(layers, Some(&context), &NoEnv, &options).unwrap();
                prop_assert_eq!(out, expected);
            }

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
