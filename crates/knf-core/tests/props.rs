//! Merge properties.

use knf::{ConfigFormat, ConfigValue, MergeOptions, merge, merge_into};
use proptest::prelude::*;

/// Arbitrary native values without floats, so equality is total.
macro_rules! properties { ($value:ty, $map:ty) => {
type Value = $value;
type Map = $map;
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::parse_inline("null".into())),
        any::<bool>().prop_map(|b| Value::parse_inline(b.to_string())),
        any::<i64>().prop_map(|n| Value::parse_inline(n.to_string())),
        "[a-z]{0,3}".prop_map(Value::String),
        Just(Value::parse_inline("1979-05-27T07:32:00Z".into())),
    ];
    // Small key alphabet so layers collide often.
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

fn merged(mut base: Value, over: Value) -> Value {
    merge_into(&mut base, over, &MergeOptions::LAST_WINS).expect("non-strict merge cannot fail");
    base
}

fn shallow(mut base: Value, over: Value) -> Value {
    merge_into(&mut base, over, &MergeOptions::shallow_root())
        .expect("non-strict merge cannot fail");
    base
}

fn shallow_at(mut base: Value, over: Value, path: &[&str]) -> Value {
    let opts = MergeOptions {
        shallow: Some(format!("{}.*", path.join(".")).parse().unwrap()),
        ..MergeOptions::default()
    };
    merge_into(&mut base, over, &opts).expect("non-strict merge cannot fail");
    base
}

/// The document without one top-level key.
fn without(doc: &Value, key: &str) -> Value {
    let map = doc.as_object().expect("object");
    let map: Map = map.iter()
        .filter(|(k, _)| k.as_str() != key)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Value::object(map)
}

proptest! {
    /// A layer merged over itself changes nothing.
    #[test]
    fn merging_a_layer_with_itself_is_a_no_op(a in arb_doc()) {
        prop_assert_eq!(merged(a.clone(), a.clone()), a);
    }

    /// Applying the same override twice is the same as applying it once.
    #[test]
    fn overriding_twice_is_the_same_as_once(a in arb_doc(), b in arb_doc()) {
        let once = merged(a, b.clone());
        prop_assert_eq!(merged(once.clone(), b), once);
    }

    /// Strict mode only rejects; when it succeeds it matches the default merge.
    #[test]
    fn strict_agrees_with_default_when_it_succeeds(a in arb_doc(), b in arb_doc()) {
        let mut strict = a.clone();
        if merge_into(&mut strict, b.clone(), &MergeOptions::STRICT).is_ok() {
            prop_assert_eq!(strict, merged(a, b));
        }
    }

    /// One layer is unchanged under shallow merge.
    #[test]
    fn a_single_layer_is_identity_under_shallow(a in arb_doc()) {
        let got = merge([a.clone()], &MergeOptions::shallow_root()).expect("non-strict");
        prop_assert_eq!(got, a);
    }

    /// Root-shallow merge is jq's `a + b`.
    #[test]
    fn shallow_assigns_top_level_keys(a in arb_doc(), b in arb_doc()) {
        let mut want = a.clone().into_object().unwrap();
        let over = b.clone().into_object().unwrap();
        for (k, v) in over {
            want.insert(k, v);
        }
        prop_assert_eq!(shallow(a, b), Value::object(want));
    }

    /// A shallow path no layer reaches (`z`) is a deep merge.
    #[test]
    fn an_unreached_shallow_path_is_a_deep_merge(a in arb_doc(), b in arb_doc()) {
        prop_assert_eq!(shallow_at(a.clone(), b.clone(), &["z"]), merged(a, b));
    }

    /// `a.*` is `+` at `a` and `*` everywhere else.
    #[test]
    fn a_shallow_path_is_plus_there_and_star_elsewhere(a in arb_doc(), b in arb_doc()) {
        let got = shallow_at(a.clone(), b.clone(), &["a"]);
        prop_assert_eq!(without(&got, "a"), without(&merged(a.clone(), b.clone()), "a"));
        let (ma, mb, mg) = (a.as_object().unwrap(), b.as_object().unwrap(), got.as_object().unwrap());
        if let (Some(x), Some(y)) = (ma.get("a"), mb.get("a")) {
            if x.as_object().is_none() || y.as_object().is_none() { return Ok(()); }
            prop_assert_eq!(&mg["a"], &shallow(x.clone(), y.clone()));
        }
    }

    /// Unlike the deep merge, root-shallow merge is associative.
    #[test]
    fn shallow_is_associative(a in arb_doc(), b in arb_doc(), c in arb_doc()) {
        let left = shallow(shallow(a.clone(), b.clone()), c.clone());
        let right = shallow(a, shallow(b, c));
        prop_assert_eq!(left, right);
    }
}

}; }

mod json {
    use super::*;
    properties!(serde_json::Value, serde_json::Map<String, serde_json::Value>);
}
mod toml {
    use super::*;
    properties!(::toml::Value, ::toml::Table);
}
