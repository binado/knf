//! Reference-aware merging agrees with native merge, and a reference merges
//! exactly as the value it names.

use knf::{
    Env, InterpolationOptions, MergeOptions, Value, interpolate, merge, merge_with_interpolation,
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
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Integer),
        "[a-z{} :.]{0,6}".prop_map(Value::String),
        Just(Value::Datetime("1979-05-27T07:32:00Z".parse().unwrap())),
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
        .prop_map(|entries| Value::Object(entries.into_iter().collect()))
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
        let opts = MergeOptions { shallow: selector };
        let expected = merge(layers.clone(), &opts);
        let out = merge_with_interpolation(layers, &opts, &options(None)).unwrap();
        prop_assert_eq!(out, expected);
    }

    #[test]
    fn a_reference_merges_exactly_like_its_referent(
        left in arb_value(),
        right in arb_value(),
        reference_right in any::<bool>(),
        shallow in prop::option::of(prop::sample::select(vec!["derived", "derived.a", "derived.*", "*"])),
    ) {
        let wrap = |value| Value::Object([("derived".to_owned(), value)].into_iter().collect());
        let opts = MergeOptions { shallow: shallow.map(|pattern| pattern.parse().unwrap()) };
        let expected = merge_with_interpolation([wrap(left.clone()), wrap(right.clone())], &opts, &options(None)).unwrap();
        let (referent, layers) = if reference_right {
            (right, [wrap(left), wrap(Value::String("${referent}".into()))])
        } else {
            (left, [wrap(Value::String("${referent}".into())), wrap(right)])
        };
        let context = Value::Object([("referent".to_owned(), referent)].into_iter().collect());
        let out = merge_with_interpolation(layers, &opts, &options(Some(&context))).unwrap();
        prop_assert_eq!(out, expected);
    }

    #[test]
    fn a_document_without_a_dollar_is_unchanged(doc in arb_doc()) {
        prop_assert_eq!(interpolate(doc.clone(), &options(None)).expect("nothing to resolve"), doc);
    }
}

fn options(context: Option<&Value>) -> InterpolationOptions<'_> {
    InterpolationOptions {
        context,
        env: &NoEnv,
        parse_inline: &Value::String,
    }
}
