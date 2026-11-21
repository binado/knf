use std::path::Path;

use knf::Value;
use knf_convert::{InputFormat, LoadError, from_json, resolve_format};
use tempfile::TempDir;

fn string(text: &str) -> Value {
    Value::String(text.into())
}

fn inline(format: InputFormat, text: &str) -> Value {
    format.parse_inline(text.into())
}

#[test]
fn json_inline_literals_are_typed() {
    let json = InputFormat::Json;
    assert_eq!(inline(json, "8080"), Value::Integer(8080));
    assert_eq!(inline(json, "1.0"), Value::Float(1.0));
    assert_eq!(inline(json, "true"), Value::Bool(true));
    assert_eq!(inline(json, "null"), Value::Null);
    assert_eq!(inline(json, "\"1.0\""), string("1.0"));
    assert_eq!(
        inline(json, "[1,2]"),
        Value::Array(vec![Value::Integer(1), Value::Integer(2)])
    );
    assert_eq!(
        inline(json, "18446744073709551615"),
        Value::Unsigned(u64::MAX)
    );
    assert_eq!(
        inline(json, "9223372036854775807"),
        Value::Integer(i64::MAX)
    );
    assert_eq!(
        inline(json, "9223372036854775808"),
        Value::Unsigned(i64::MAX as u64 + 1)
    );
}

#[test]
fn json_inline_falls_back_to_the_original_string() {
    let json = InputFormat::Json;
    assert_eq!(inline(json, "hello"), string("hello"));
    assert_eq!(inline(json, "[a,b]"), string("[a,b]"));
    assert_eq!(inline(json, "[1,2"), string("[1,2"));
    assert_eq!(inline(json, "TRUE"), string("TRUE"));
    assert_eq!(inline(json, ""), string(""));
}

#[test]
fn toml_inline_literals_are_typed() {
    let toml = InputFormat::Toml;
    assert_eq!(inline(toml, "8080"), Value::Integer(8080));
    assert_eq!(inline(toml, "1.0"), Value::Float(1.0));
    assert_eq!(inline(toml, "true"), Value::Bool(true));
    assert_eq!(inline(toml, "\"x\""), string("x"));
    assert!(matches!(inline(toml, "inf"), Value::Float(f) if f.is_infinite()));
    assert!(matches!(
        inline(toml, "1979-05-27T07:32:00Z"),
        Value::Datetime(_)
    ));
}

#[test]
fn toml_inline_has_no_null_so_it_stays_a_string() {
    assert_eq!(inline(InputFormat::Toml, "null"), string("null"));
    assert_eq!(inline(InputFormat::Json, "null"), Value::Null);
}

#[test]
fn toml_inline_trims_only_ascii_whitespace() {
    let toml = InputFormat::Toml;
    assert_eq!(inline(toml, " \t8080\r\n"), Value::Integer(8080));
    assert_eq!(inline(toml, "\u{a0}8080\u{a0}"), string("\u{a0}8080\u{a0}"));
}

#[test]
fn inline_nan_and_inf_differ_between_formats() {
    assert_eq!(inline(InputFormat::Json, "nan"), string("nan"));
    assert!(matches!(inline(InputFormat::Toml, "nan"), Value::Float(f) if f.is_nan()));
}

#[test]
fn extensions_select_a_format_case_insensitively() {
    let format = |name: &str| InputFormat::from_path(Path::new(name));
    assert_eq!(format("a.json"), Some(InputFormat::Json));
    assert_eq!(format("a.JSON"), Some(InputFormat::Json));
    assert_eq!(format("dir/a.Toml"), Some(InputFormat::Toml));
    assert_eq!(format("a.yaml"), None);
    assert_eq!(format("a.yml"), None);
    assert_eq!(format("a"), None);
}

#[test]
fn documents_must_have_an_object_root() {
    for (format, text, found) in [
        (InputFormat::Json, "[1]", "array"),
        (InputFormat::Json, "3", "number"),
        (InputFormat::Json, "\"s\"", "string"),
    ] {
        let err = format.parse_document(text).unwrap_err().to_string();
        assert_eq!(
            err,
            format!("expected an object at the top level, found {found}")
        );
    }
    assert!(InputFormat::Json.parse_document("").is_err());
    assert_eq!(
        InputFormat::Toml.parse_document("").unwrap(),
        Value::Object(Default::default())
    );
}

#[test]
fn load_names_the_file_in_diagnostics() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.json");
    let err = format!("{:#}", InputFormat::Json.load(&missing).unwrap_err());
    assert!(
        err.starts_with(&format!("reading `{}`", missing.display())),
        "{err}"
    );

    let array = dir.path().join("array.json");
    std::fs::write(&array, "[1]").unwrap();
    let err = InputFormat::Json.load(&array).unwrap_err().to_string();
    assert_eq!(
        err,
        format!(
            "{}: expected an object at the top level, found array",
            array.display()
        )
    );

    let broken = dir.path().join("broken.toml");
    std::fs::write(&broken, "a = ").unwrap();
    let err = InputFormat::Toml.load(&broken).unwrap_err().to_string();
    assert_eq!(err, format!("{}: invalid TOML", broken.display()));
}

#[test]
fn resolve_format_checks_extensions_before_reading() {
    assert_eq!(resolve_format::<&str>(&[]).unwrap(), InputFormat::Json);
    assert_eq!(
        resolve_format(&["a.toml", "b.TOML"]).unwrap(),
        InputFormat::Toml
    );
    assert_eq!(
        resolve_format(&["a.json", "b.toml"]).unwrap_err(),
        LoadError::MixedFormats
    );
    assert_eq!(
        resolve_format(&["a.yaml"]).unwrap_err(),
        LoadError::UnknownExtension {
            path: "a.yaml".into()
        }
    );
}

#[test]
fn json_numbers_keep_their_kind() {
    let value = from_json(serde_json::json!({"whole": 1.0, "int": 1, "big": 1e30}));
    let object = value.as_object().unwrap();
    assert_eq!(object["whole"], Value::Float(1.0));
    assert_eq!(object["int"], Value::Integer(1));
    assert_eq!(object["big"], Value::Float(1e30));
}
