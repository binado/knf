//! Two cheap properties that catch real bugs in the recursion.

// `Strategy` is proptest's central trait, so the merge strategy is aliased
// rather than shadowing it in a file full of `impl Strategy<Value = _>`.
use knf_core::{Map, MergeOptions, Number, Rules, Strategy as Rule, Value, merge_into, merge_with};
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

/// Valid rule sets over the same small key alphabet as [`arb_doc`], so the
/// rules actually land on paths the document has.
///
/// Paths are non-empty, matching every rule a command line can express; the
/// empty path names the accumulator itself, which is seeded rather than absent.
fn arb_rules() -> impl Strategy<Value = Rules> {
    let strategy = prop_oneof![
        Just(Rule::Merge),
        Just(Rule::Append),
        Just(Rule::Replace),
        Just(Rule::Fail),
    ];
    let path = prop::collection::vec("[a-c]{1,2}", 1..3);
    prop::collection::vec((path, strategy), 0..4)
        .prop_filter_map("rule sets must be valid", |rules| Rules::build(rules).ok())
}

fn merged(mut base: Value, over: Value) -> Value {
    merge_into(&mut base, over, &MergeOptions::LAST_WINS).expect("non-strict merge cannot fail");
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
        if merge_into(&mut strict, b.clone(), &MergeOptions::STRICT).is_ok() {
            prop_assert_eq!(strict, merged(a, b));
        }
    }

    /// One layer is still the identity, whatever the rules say. Nothing in a
    /// rule set may fire against the empty seed: every key is *absent*, so it is
    /// inserted rather than combined. This is what `--append` most threatens —
    /// a lone layer's array must not double — and what the byte-level no-op of
    /// `knf a.json` rests on.
    #[test]
    fn a_single_layer_is_identity_for_any_rule_set(a in arb_doc(), rules in arb_rules()) {
        let opts = MergeOptions { strict: false, rules: Some(rules) };
        let got = merge_with([a.clone()], &opts).expect("no rule fires against an empty seed");
        prop_assert_eq!(got, a);
    }
}
