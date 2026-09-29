//! Resolution tests over a stub [`Env`].

use std::collections::HashMap;

use crate::{ConfigFormat, ConfigValue};

use super::*;

/// A raw-text environment, with no process reads.
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
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

macro_rules! common_tests {
    () => {
        fn obj(entries: Vec<(&str, Value)>) -> Value {
            Value::object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect::<Map>(),
            )
        }

        fn s(text: &str) -> Value {
            Value::string(text.to_string())
        }

        fn n(v: i64) -> Value {
            Value::parse_inline(v.to_string())
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

        #[test]
        fn context_falls_back_per_complete_path_without_merging() {
            let doc = obj(vec![
                ("db", obj(vec![("host", s("prod"))])),
                ("port", s("${db.port}")),
                ("copy", s("${db}")),
                ("items", Value::array(vec![n(1)])),
                ("last", s("${items[1]}")),
                ("scalar", n(0)),
                ("nested", s("${scalar.child}")),
            ]);
            let context = obj(vec![
                ("db", obj(vec![("host", s("shared")), ("port", n(5432))])),
                ("items", Value::array(vec![n(2), n(3)])),
                ("scalar", obj(vec![("child", n(4))])),
                ("unused", s("${missing}")),
                ("cycle", s("${cycle}")),
                ("malformed", s("${}")),
            ]);
            let out = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap();
            let map = out.as_object().unwrap();
            assert_eq!(map["port"], n(5432));
            assert_eq!(map["copy"], obj(vec![("host", s("prod"))]));
            assert_eq!(map["last"], n(3));
            assert_eq!(map["nested"], n(4));
            assert!(map.get("unused").is_none());
            assert!(map.get("cycle").is_none());
        }

        #[test]
        fn context_dependencies_prefer_output_and_containers_keep_their_children() {
            let doc = obj(vec![
                ("host", s("prod")),
                ("url", s("${service.url}")),
                ("copy", s("${service}")),
            ]);
            let service = obj(vec![
                ("host", s("own")),
                ("url", s("https://${host}/api")),
                ("ports", Value::array(vec![n(5432)])),
            ]);
            let context = obj(vec![("host", s("shared")), ("service", service)]);
            let out = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap();
            let map = out.as_object().unwrap();
            assert_eq!(map["url"], s("https://prod/api"));
            assert_eq!(
                map["copy"],
                obj(vec![
                    ("host", s("own")),
                    ("url", s("https://prod/api")),
                    ("ports", Value::array(vec![n(5432)])),
                ])
            );
        }

        #[test]
        fn context_environment_is_terminal_and_escaping_is_preserved() {
            let doc = obj(vec![("copy", s("${settings}"))]);
            let context = obj(vec![(
                "settings",
                obj(vec![
                    ("literal", s("$${missing}")),
                    ("raw", s("${env:TEXT}")),
                    ("port", s("${env:PORT}")),
                ]),
            )]);
            let out = interpolate_with_context(
                doc,
                &context,
                &StubEnv::new(&[("TEXT", "${missing}"), ("PORT", "8080")]),
            )
            .unwrap();
            assert_eq!(
                out,
                obj(vec![(
                    "copy",
                    obj(vec![
                        ("literal", s("${missing}")),
                        ("raw", s("${missing}")),
                        ("port", n(8080)),
                    ])
                )])
            );
        }

        #[test]
        fn context_does_not_hide_output_errors() {
            let doc = obj(vec![("value", s("${missing}")), ("copy", s("${value}"))]);
            let context = obj(vec![("value", n(42))]);
            let err = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap_err();
            assert_eq!(
                err.to_string(),
                "unresolved reference\n  --> value: `missing`"
            );
        }

        #[test]
        fn repeated_context_dependencies_report_one_problem_at_the_source_path() {
            let doc = obj(vec![("a", s("${settings.bad}")), ("b", s("${settings}"))]);
            let context = obj(vec![("settings", obj(vec![("bad", s("${missing}"))]))]);
            let err = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap_err();
            assert_eq!(
                err.to_string(),
                "unresolved reference\n  --> settings.bad: `missing`"
            );
            assert!(!err.to_string().ends_with('\n'));
        }

        #[test]
        fn context_and_cross_source_cycles_are_detected() {
            let doc = obj(vec![("copy", s("${a}"))]);
            let context = obj(vec![("a", s("${b}")), ("b", s("${a}"))]);
            let err = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap_err();
            assert_eq!(err.to_string(), "reference cycle: `a` -> `b` -> `a`");
            let doc = obj(vec![("a", s("${b}"))]);
            let context = obj(vec![("b", s("${a}"))]);
            let err = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap_err();
            assert_eq!(err.to_string(), "reference cycle: `a` -> `b` -> `a`");
        }

        #[test]
        fn context_container_beneath_an_output_scalar_keeps_its_source() {
            let doc = obj(vec![("a", s("${a.child}")), ("again", s("${a.child}"))]);
            let context = obj(vec![(
                "a",
                obj(vec![(
                    "child",
                    obj(vec![("value", n(7)), ("copy", s("${a.child.value}"))]),
                )]),
            )]);
            let out = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap();
            let expected = obj(vec![("value", n(7)), ("copy", n(7))]);
            assert_eq!(out, obj(vec![("a", expected.clone()), ("again", expected)]));
        }

        #[test]
        fn embedded_context_containers_are_rejected() {
            let doc = obj(vec![("copy", s("x${settings}"))]);
            let context = obj(vec![("settings", obj(vec![("port", n(80))]))]);
            let err = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap_err();
            assert_eq!(
                err.to_string(),
                "reference cannot be rendered into a string\n  --> copy: `settings` is an object"
            );
        }

        // --- positions ------------------------------------------------------------

        /// A whole-string reference keeps its type.
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
            let map = out.as_object().expect("an object");
            assert_eq!(map["url"], s("http://db:8080/health"));
        }

        /// Embedded floats retain the established spelling, including integral floats.
        #[test]
        fn an_embedded_float_keeps_its_point() {
            let doc = obj(vec![
                ("v", Value::parse_inline("1.0".into())),
                ("tag", s("v${v}")),
            ]);
            let out = interp(doc).unwrap();
            let map = out.as_object().expect("an object");
            assert_eq!(map["tag"], s("v1.0"));
        }

        #[test]
        fn embedded_exponents_keep_existing_spelling_while_whole_references_keep_values() {
            for spelling in ["1e20", "1e300", "1e-7"] {
                let value = Value::parse_inline(spelling.into());
                let doc = obj(vec![
                    ("v", value.clone()),
                    ("tag", s("n=${v}")),
                    ("whole", s("${v}")),
                ]);
                let out = interp(doc).unwrap();
                let map = out.as_object().unwrap();
                assert_eq!(map["tag"], s(&format!("n={spelling}")));
                assert_eq!(map["whole"], value);
                let emitted = crate::format::emit(out.clone(), false).unwrap();
                let round = Value::parse_document(&emitted).unwrap();
                assert_eq!(round, out);
            }
        }

        #[test]
        fn whole_environment_literals_ignore_whitespace_but_embedded_text_and_fallbacks_keep_it() {
            let doc = obj(vec![
                ("port", s("${env:PORT}")),
                ("raw", s("n=${env:PORT}")),
                ("fallback", s("${env:TEXT}")),
            ]);
            let out = interp_env(doc, &[("PORT", " \t8080\r\n"), ("TEXT", " text\n")]).unwrap();
            assert_eq!(
                out,
                obj(vec![
                    ("port", n(8080)),
                    ("raw", s("n= \t8080\r\n")),
                    ("fallback", s(" text\n"))
                ])
            );
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

        /// Forward and backward references resolve the same.
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

        /// A reference into its own enclosing container is a cycle.
        #[test]
        fn a_cycle_through_a_container_names_every_hop() {
            let doc = obj(vec![("a", obj(vec![("b", s("${a}"))]))]);
            assert_eq!(err(doc), "reference cycle: `a` -> `a.b` -> `a`");
        }

        // --- containers -----------------------------------------------------------

        /// A whole-string container reference aliases the resolved subtree.
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
            let map = out.as_object().expect("an object");
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

        /// A whole-string null reference yields null.

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

        /// Embedded environment values are spliced verbatim.
        #[test]
        fn an_embedded_variable_splices_its_raw_text() {
            let doc = obj(vec![("v", s("[${env:V}]"))]);
            assert_eq!(
                interp_env(doc, &[("V", "1.500")]).unwrap(),
                obj(vec![("v", s("[1.500]"))])
            );
        }

        /// Environment values are never re-scanned.
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

        /// `env:` is a prefix match; other colons are ordinary key characters.
        #[test]
        fn a_colon_in_a_key_is_not_a_namespace() {
            let doc = obj(vec![("a:b", n(1)), ("v", s("${a:b}"))]);
            assert_eq!(interp(doc).unwrap(), obj(vec![("a:b", n(1)), ("v", n(1))]));
        }

        /// A dotted path containing a colon is not a namespace.
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

        /// All problem kinds are reported in one run.
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

        /// A syntax error does not hide other references in the same string.
        #[test]
        fn problems_in_one_string_are_collected() {
            let doc = obj(vec![("value", s("${before} ${} ${after}"))]);
            assert_eq!(
                err(doc),
                "invalid reference\n\
         \x20 --> value: empty reference `${}`\n\
         unresolved reference\n\
         \x20 --> value: `before`\n\
         \x20 --> value: `after`"
            );
        }

        /// A broken key referenced many times is reported once per site.
        #[test]
        fn a_problem_is_reported_once_per_site() {
            let doc = obj(vec![
                ("a", s("${gone}")),
                ("b", s("${a}")),
                ("c", s("${a}")),
            ]);
            assert_eq!(err(doc), "unresolved reference\n  --> a: `gone`");
        }

        /// `${env:}` is malformed, not a lookup of the empty name.
        #[test]
        fn an_empty_variable_name_is_syntax_not_a_lookup() {
            assert_eq!(
                err_env(obj(vec![("a", s("${env:}"))]), &[("", "x")]),
                "invalid reference\n  --> a: empty variable name in `${env:}`"
            );
        }

        // --- shape ----------------------------------------------------------------

        /// References may live inside arrays.
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
            let out = interp(doc).unwrap();
            let map = out.as_object().expect("an object");
            assert_eq!(map.keys().collect::<Vec<_>>(), ["zebra", "apple", "middle"]);
        }

        // --- indexed references ---------------------------------------------------

        /// A whole-string reference to an element keeps the element's type.
        #[test]
        fn a_whole_string_indexed_reference_takes_the_elements_type() {
            let doc = obj(vec![
                ("tags", Value::Array(vec![n(8080)])),
                ("port", s("${tags[0]}")),
            ]);
            assert_eq!(
                interp(doc).unwrap(),
                obj(vec![
                    ("tags", Value::Array(vec![n(8080)])),
                    ("port", n(8080))
                ])
            );
        }

        #[test]
        fn an_index_chain_reads_through_arrays() {
            let doc = obj(vec![
                (
                    "servers",
                    Value::Array(vec![obj(vec![("host", s("a")), ("port", n(1))])]),
                ),
                ("url", s("http://${servers[0].host}:${servers[0].port}")),
            ]);
            let out = interp(doc).unwrap();
            let map = out.as_object().expect("an object");
            assert_eq!(map["url"], s("http://a:1"));
        }

        /// A whole-string container element aliases the resolved subtree.
        #[test]
        fn a_whole_string_element_reference_aliases_a_resolved_subtree() {
            let doc = obj(vec![
                ("host", s("db.internal")),
                (
                    "servers",
                    Value::Array(vec![obj(vec![("host", s("${host}")), ("port", n(5432))])]),
                ),
                ("primary", s("${servers[0]}")),
            ]);
            let out = interp(doc).unwrap();
            let map = out.as_object().expect("an object");
            let expected = obj(vec![("host", s("db.internal")), ("port", n(5432))]);
            assert_eq!(map["primary"], expected);
        }

        /// An embedded container element is an error.
        #[test]
        fn an_embedded_element_container_is_rejected() {
            let doc = obj(vec![
                ("servers", Value::Array(vec![obj(vec![("host", s("x"))])])),
                ("url", s("http://${servers[0]}/")),
            ]);
            assert_eq!(
                err(doc),
                "reference cannot be rendered into a string\n  --> url: `servers[0]` is an object"
            );
        }

        /// A whole-string datetime element stays a datetime.

        #[test]
        fn an_out_of_range_index_is_unresolved() {
            let doc = obj(vec![
                ("tags", Value::Array(vec![n(1)])),
                ("t", s("${tags[9]}")),
            ]);
            assert_eq!(err(doc), "unresolved reference\n  --> t: `tags[9]`");
        }

        #[test]
        fn a_malformed_index_is_a_syntax_problem() {
            let doc = obj(vec![("a", s("${tags[x]}")), ("b", s("${tags[]}"))]);
            assert_eq!(
                err(doc),
                "invalid reference\n\
         \x20 --> a: malformed index in reference `${tags[x]}`\n\
         \x20 --> b: malformed index in reference `${tags[]}`"
            );
        }

        #[test]
        fn a_cycle_through_an_array_names_the_index() {
            let doc = obj(vec![
                ("a", s("${b[0]}")),
                ("b", Value::Array(vec![s("${a}")])),
            ]);
            assert_eq!(err(doc), "reference cycle: `a` -> `b[0]` -> `a`");
        }

        /// `${a[0]}` indexes the array, never a key literally named `a[0]`.
        #[test]
        fn a_bracket_body_reads_as_an_index_not_a_weird_key() {
            let doc = obj(vec![
                ("a", Value::Array(vec![s("elem")])),
                ("a[0]", s("weird key")),
                ("v", s("${a[0]}")),
            ]);
            let out = interp(doc).unwrap();
            let map = out.as_object().expect("an object");
            assert_eq!(map["v"], s("elem"));
        }
    };
}

mod json {
    use super::*;
    use serde_json::Value;
    type Map = serde_json::Map<String, Value>;
    common_tests!();
    #[test]
    fn context_preserves_unsigned_integers_and_null_precedence() {
        let doc = serde_json::json!({"value": null, "copy": "${value}", "big": "${big_value}", "nested": "${value.child}", "null": "${null_value}"});
        let context =
            serde_json::json!({"value": {"child": 4}, "big_value": u64::MAX, "null_value": null});
        let out = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap();
        assert_eq!(
            out,
            serde_json::json!({"value": null, "copy": null, "big": u64::MAX, "nested": 4, "null": null})
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
    #[test]
    fn a_whole_string_null_reference_is_an_ordinary_null() {
        let doc = obj(vec![("n", Value::Null), ("copy", s("${n}"))]);
        assert_eq!(
            interp(doc).unwrap(),
            obj(vec![("n", Value::Null), ("copy", Value::Null)])
        );
    }
}

mod toml {
    use super::*;
    use ::toml::Value;
    type Map = ::toml::Table;
    common_tests!();
    #[test]
    fn context_preserves_native_datetimes_and_nonfinite_floats() {
        let doc = Value::parse_document("date = '${day}'\nfloat = '${infinite}'\n").unwrap();
        let context = Value::parse_document("day = 1979-05-27\ninfinite = inf\n").unwrap();
        let out = interpolate_with_context(doc, &context, &StubEnv::default()).unwrap();
        assert_eq!(out["date"], context["day"]);
        assert_eq!(out["float"], context["infinite"]);
    }
    #[test]
    fn a_datetime_splices_as_its_source_spelling() {
        let doc = obj(vec![
            ("d", Value::parse_inline("1979-05-27T07:32:00Z".into())),
            ("stamp", s("at ${d}")),
            ("copy", s("${d}")),
        ]);
        let out = interp(doc).unwrap();
        let map = out.as_object().expect("an object");
        assert_eq!(map["stamp"], s("at 1979-05-27T07:32:00Z"));
        // Whole-string keeps the *type*: a copied datetime, not a string.
        assert_eq!(
            map["copy"],
            Value::parse_inline("1979-05-27T07:32:00Z".into())
        );
    }
    #[test]
    fn an_indexed_datetime_copies_whole_string() {
        let doc = obj(vec![
            (
                "dates",
                Value::Array(vec![Value::parse_inline("1979-05-27T07:32:00Z".into())]),
            ),
            ("stamp", s("${dates[0]}")),
        ]);
        let out = interp(doc).unwrap();
        let map = out.as_object().expect("an object");
        assert_eq!(
            map["stamp"],
            Value::parse_inline("1979-05-27T07:32:00Z".into())
        );
    }
}
