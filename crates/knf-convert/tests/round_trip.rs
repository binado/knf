use knf::Value;
use knf_convert::{InputFormat, Layout, OutputFormat, Policy};

#[test]
fn json_round_trip_preserves_order_integer_limits_null_and_negative_zero() {
    let source = serde_json::json!({
        "z": i64::MIN, "positive": i64::MAX, "large": u64::MAX, "nothing": null,
        "a": [1.0, -0.0, {"y": true, "b": false}]
    });
    let value = InputFormat::Json
        .parse_document(&source.to_string())
        .unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object["positive"], Value::Integer(i64::MAX));
    assert_eq!(object["large"], Value::unsigned(u64::MAX));
    let emitted = OutputFormat::Json
        .emit(&value, Policy::Strict, Layout::Compact)
        .unwrap();
    assert!(!emitted.lossy);
    let native: serde_json::Value = serde_json::from_str(&emitted.value).unwrap();
    assert_eq!(native, source);
    assert_eq!(
        native.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["z", "positive", "large", "nothing", "a"]
    );
    assert_eq!(
        native["a"][2]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["y", "b"]
    );
    assert!(native["a"][0].is_f64());
    assert_eq!(
        native["a"][1].as_f64().unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
}

#[test]
fn toml_round_trip_preserves_order_datetime_precision_and_float_kinds() {
    let text = "
        z = 1979-05-27T07:32:00.123456789-07:00
        local = 1979-05-27T07:32:00.987654321
        date = 1979-05-27
        time = 07:32:00.000000001
        a = [-0.0, 1.0, nan, inf, -inf]
    ";
    let source: toml::Table = toml::from_str(text).unwrap();
    let value = InputFormat::Toml.parse_document(text).unwrap();
    let emitted = OutputFormat::Toml
        .emit(&value, Policy::Strict, Layout::Compact)
        .unwrap();
    assert!(!emitted.lossy);
    let native: toml::Table = toml::from_str(&emitted.value).unwrap();
    assert_eq!(
        native.keys().collect::<Vec<_>>(),
        ["z", "local", "date", "time", "a"]
    );
    for key in ["z", "local", "date", "time"] {
        assert_eq!(
            native[key].as_datetime().unwrap(),
            source[key].as_datetime().unwrap()
        );
    }
    let floats: Vec<_> = native["a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_float().unwrap())
        .collect();
    assert_eq!(floats[0].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(floats[1], 1.0);
    assert!(floats[2].is_nan());
    assert_eq!(floats[3..], [f64::INFINITY, f64::NEG_INFINITY]);
}

#[cfg(feature = "yaml")]
#[test]
fn yaml_keeps_null_unsigned_and_nonfinite_values_in_one_document() {
    let value = Value::Object(
        [
            ("nil".into(), Value::Null),
            ("max".into(), Value::unsigned(u64::MAX)),
            (
                "floats".into(),
                Value::Array(vec![Value::Float(f64::NAN), Value::Float(f64::INFINITY)]),
            ),
        ]
        .into(),
    );
    let out = OutputFormat::Yaml
        .emit(&value, Policy::Strict, Layout::Pretty)
        .unwrap();
    assert!(!out.lossy);
    assert_eq!(
        out.value,
        "nil: null\nmax: 18446744073709551615\nfloats:\n- .nan\n- .inf\n"
    );
    let date = knf_convert::InputFormat::Toml
        .parse_document("[nested]\nday = 1979-05-27")
        .unwrap();
    let error = OutputFormat::Yaml
        .emit(&date, Policy::Strict, Layout::Pretty)
        .unwrap_err();
    let knf_convert::EmitError::Conversion(error) = error else {
        panic!("{error}")
    };
    assert_eq!(error.path(), "nested.day");
    assert!(
        OutputFormat::Yaml
            .emit(&date, Policy::Lossy, Layout::Pretty)
            .unwrap()
            .lossy
    );
}
