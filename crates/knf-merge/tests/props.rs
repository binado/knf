//! Two cheap properties that catch real bugs in the recursion.

use knf_merge::{MergeOptions, merge_json};
use proptest::prelude::*;
use serde_json::{Map, Value};

/// Arbitrary JSON, deliberately without floats so that equality is total —
/// a NaN leaf would make every property vacuously fail.
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| Value::Number(n.into())),
        "[a-z]{0,3}".prop_map(Value::String),
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
        .prop_map(|entries| Value::Object(entries.into_iter().collect::<Map<String, Value>>()))
}

fn arb_doc() -> impl Strategy<Value = Value> {
    arb_object(arb_value())
}

fn merged(mut base: Value, over: Value) -> Value {
    merge_json(&mut base, over, &MergeOptions::LAST_WINS).expect("non-strict merge cannot fail");
    base
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
        if merge_json(&mut strict, b.clone(), &MergeOptions::STRICT).is_ok() {
            prop_assert_eq!(strict, merged(a, b));
        }
    }
}
