//! Two cheap properties that catch real bugs in the recursion.

use knf::{Map, MergeOptions, Number, Value, merge, merge_into};
use proptest::prelude::*;

/// Arbitrary IR values, deliberately without floats so that equality is total —
/// a NaN leaf would make every property vacuously fail.
///
/// `Datetime` is in the leaf set because it is a scalar kind that is not
/// `String`, so it exercises replace-wholesale on a variant nothing else covers.
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| Value::Number(Number::I64(n))),
        "[a-z]{0,3}".prop_map(Value::String),
        Just(Value::Datetime("1979-05-27T07:32:00Z".to_string())),
    ];
    // Small alphabets for keys, so distinct layers actually collide often
    // enough to exercise the merge rather than just unioning disjoint trees.
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
        shallow: vec![path.iter().map(|k| k.to_string()).collect()],
        ..MergeOptions::default()
    };
    merge_into(&mut base, over, &opts).expect("non-strict merge cannot fail");
    base
}

/// The document without one top-level key.
fn without(doc: &Value, key: &str) -> Value {
    let Value::Object(map) = doc else {
        unreachable!("arb_doc generates objects")
    };
    let mut map = map.clone();
    map.shift_remove(key);
    Value::Object(map)
}

proptest! {
    /// A layer merged over itself changes nothing. This is the invariant that
    /// `knf a.json` must be a no-op depends on, and the one RFC 7386 delete
    /// semantics would break for any document containing a null.
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

    /// Strict mode never *changes* a result, it only rejects one: whenever it
    /// succeeds it must agree with the default merge.
    #[test]
    fn strict_agrees_with_default_when_it_succeeds(a in arb_doc(), b in arb_doc()) {
        let mut strict = a.clone();
        if merge_into(&mut strict, b.clone(), &MergeOptions::STRICT).is_ok() {
            prop_assert_eq!(strict, merged(a, b));
        }
    }

    /// One layer is still the identity under shallow merge: against the empty
    /// seed every key is absent, so it is inserted rather than replaced. This is
    /// what the byte-level no-op of `knf --shallow a.json` rests on.
    #[test]
    fn a_single_layer_is_identity_under_shallow(a in arb_doc()) {
        let got = merge([a.clone()], &MergeOptions::shallow_root()).expect("non-strict");
        prop_assert_eq!(got, a);
    }

    /// Shallow merge is jq's `a + b`: every top-level key of `b` is assigned
    /// over `a` whole, and nothing below the top level is looked at.
    #[test]
    fn shallow_assigns_top_level_keys(a in arb_doc(), b in arb_doc()) {
        let (Value::Object(mut want), Value::Object(over)) = (a.clone(), b.clone()) else {
            unreachable!("arb_doc generates objects")
        };
        for (k, v) in over {
            want.insert(k, v);
        }
        prop_assert_eq!(shallow(a, b), Value::Object(want));
    }

    /// A shallow path no layer reaches is a deep merge. Keys are drawn from
    /// `[a-c]`, so `z` never appears.
    #[test]
    fn an_unreached_shallow_path_is_a_deep_merge(a in arb_doc(), b in arb_doc()) {
        prop_assert_eq!(shallow_at(a.clone(), b.clone(), &["z"]), merged(a, b));
    }

    /// Shallow at `a` is exactly `+` at `a` and `*` everywhere else: outside
    /// `a` the result is the deep merge's, and where both sides hold an object
    /// at `a` it is the root-shallow merge of those two objects.
    #[test]
    fn a_shallow_path_is_plus_there_and_star_elsewhere(a in arb_doc(), b in arb_doc()) {
        let got = shallow_at(a.clone(), b.clone(), &["a"]);
        prop_assert_eq!(without(&got, "a"), without(&merged(a.clone(), b.clone()), "a"));
        let (Value::Object(ma), Value::Object(mb), Value::Object(mg)) = (&a, &b, &got) else {
            unreachable!("arb_doc generates objects")
        };
        if let (Some(x @ Value::Object(_)), Some(y @ Value::Object(_))) = (ma.get("a"), mb.get("a")) {
            prop_assert_eq!(&mg["a"], &shallow(x.clone(), y.clone()));
        }
    }

    /// Unlike the deep merge, a merge shallow at the root is associative — a scalar shadowing
    /// an object loses nothing a later object could have merged back into.
    #[test]
    fn shallow_is_associative(a in arb_doc(), b in arb_doc(), c in arb_doc()) {
        let left = shallow(shallow(a.clone(), b.clone()), c.clone());
        let right = shallow(a, shallow(b, c));
        prop_assert_eq!(left, right);
    }
}
