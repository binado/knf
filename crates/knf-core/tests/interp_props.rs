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
            /// Inheritance follows the native merge contract, including
            /// destination selectors, nested objects and wholesale arrays.
            #[test]
            fn inheritance_agrees_with_native_merge(
                base in arb_doc(),
                over in arb_doc(),
                shallow in prop::option::of(prop::sample::select(vec!["derived", "derived.a", "derived.*", "*", "'derived.a'"])),
            ) {
                let wrap = |value| Value::object([("derived".to_owned(), value)].into_iter().collect::<Map>());
                let selector = shallow.map(|pattern| pattern.parse().unwrap());
                let expected = merge([wrap(base.clone()), wrap(over.clone())], &MergeOptions {
                    shallow: selector.clone(), ..Default::default()
                }).unwrap();
                let mut local = over;
                local.as_object_mut().unwrap().insert("extends".into(), Value::string("${base}".into()));
                let context = Value::object([("base".to_owned(), base)].into_iter().collect::<Map>());
                let out = merge_interpolate([wrap(local)], Some(&context), &NoEnv, &InterpOptions {
                    merge_key: Some("extends".into()), shallow: selector,
                }).unwrap();
                prop_assert_eq!(out, expected);
            }

            /// Without references, layers fold exactly like native merge,
            /// including its non-associative replacements.
            #[test]
            fn layers_without_references_fold_like_native_merge(
                layers in prop::collection::vec(arb_doc(), 0..4),
                shallow in prop::option::of(prop::sample::select(vec!["a", "a.b", "a.*", "*", "{a,b}.c"])),
            ) {
                let selector: Option<knf::glob::KeyGlobPattern> = shallow.map(|pattern| pattern.parse().unwrap());
                let expected = merge(layers.clone(), &MergeOptions { shallow: selector.clone(), ..Default::default() }).unwrap();
                let out = merge_interpolate(layers, None, &NoEnv, &InterpOptions { merge_key: None, shallow: selector }).unwrap();
                prop_assert_eq!(out, expected);
            }

            /// Replacing a value with a reference to it never changes a merge,
            /// on either side and for every kind.
            #[test]
            fn a_reference_merges_exactly_like_its_referent(
                left in arb_value(),
                right in arb_value(),
                reference_right in any::<bool>(),
                shallow in prop::option::of(prop::sample::select(vec!["derived", "derived.a", "derived.*", "*"])),
            ) {
                let wrap = |value| Value::object([("derived".to_owned(), value)].into_iter().collect::<Map>());
                let options = InterpOptions { merge_key: None, shallow: shallow.map(|pattern| pattern.parse().unwrap()) };
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
