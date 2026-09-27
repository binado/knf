use std::path::Path;

use knf::{Layers, MergeOptions, format::Format};
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, contents) in files {
        std::fs::write(dir.path().join(path), contents).expect("write fixture");
    }
    dir
}

fn overlay(value: Value) -> Value {
    assert!(value.is_object());
    value
}

/// The whole file pipeline, as a caller composes it: load, then fold.
fn merge_files<P: AsRef<Path>>(paths: &[P], opts: &MergeOptions) -> anyhow::Result<Value> {
    let Layers::Json(layers) = knf::load_layers(paths, None)? else {
        panic!("JSON fixtures")
    };
    Ok(knf::merge(layers, opts)?)
}

#[test]
fn load_and_merge_paths_without_a_cli() {
    let dir = tree(&[
        ("base.json", r#"{"server":{"host":"local","port":80}}"#),
        ("prod.json", r#"{"server":{"port":443},"debug":true}"#),
    ]);
    let paths = [dir.path().join("base.json"), dir.path().join("prod.json")];

    let merged = merge_files(&paths, &MergeOptions::default()).expect("merge succeeds");

    assert_eq!(
        merged,
        json!({"server": {"host": "local", "port": 443}, "debug": true})
    );
}

#[test]
fn shallow_terminal_overlays_and_interpolation_compose() {
    let dir = tree(&[
        (
            "base.json",
            r#"{"db":{"host":"a","port":1},"port":80,"root":"/srv"}"#,
        ),
        ("prod.json", r#"{"db":{"host":"b"}}"#),
    ]);
    let paths = [dir.path().join("base.json"), dir.path().join("prod.json")];
    let overlay = overlay(json!({"port": 443, "data": "${root}/data"}));

    let Layers::Json(mut layers) = knf::load_layers(&paths, None).expect("layers load") else {
        panic!("JSON")
    };
    layers.push(overlay);
    let merged = knf::merge(layers, &MergeOptions::shallow_root()).expect("shallow merge succeeds");
    let merged = knf::interpolate(merged, &knf::ProcessEnv).expect("document references resolve");

    assert_eq!(
        merged,
        json!({
            "db": {"host": "b"},
            "port": 443,
            "root": "/srv",
            "data": "/srv/data"
        })
    );
}

#[test]
fn strict_overlay_errors_name_the_key_path() {
    let dir = tree(&[("base.json", r#"{"port":80}"#)]);
    let paths = [dir.path().join("base.json")];
    let overlay = overlay(json!({"port": "wrong kind"}));

    let Layers::Json(mut layers) = knf::load_layers(&paths, None).expect("layers load") else {
        panic!("JSON")
    };
    layers.push(overlay);
    let err = knf::merge(layers, &MergeOptions::STRICT).expect_err("strict overlay must fail");

    assert_eq!(err.path(), ["port"]);
}

#[test]
fn selected_values_keep_their_native_scalar_representations() {
    let opts = MergeOptions {
        shallow: Some("payload.*".parse().unwrap()),
        ..Default::default()
    };
    let json = knf::merge(
        [
            json!({"payload": {"n": 0, "nil": {"x": 1}}}),
            json!({"payload": {"n": u64::MAX, "nil": null}}),
        ],
        &opts,
    )
    .unwrap();
    assert_eq!(json["payload"]["n"].as_u64(), Some(u64::MAX));
    assert!(json["payload"]["nil"].is_null());

    let layers = [
        "[payload]\nstamp = 1979-05-27T07:32:00.123456789+02:00\nx = 0.0\n",
        "[payload]\nstamp = 1980-01-01T00:00:00.987654321Z\nx = inf\n",
    ]
    .map(|text| toml::from_str::<toml::Value>(text).unwrap());
    let toml = knf::merge(layers, &opts).unwrap();
    assert_eq!(
        toml["payload"]["stamp"].as_datetime().unwrap().to_string(),
        "1980-01-01T00:00:00.987654321Z"
    );
    assert_eq!(toml["payload"]["x"].as_float(), Some(f64::INFINITY));
}

#[test]
fn input_format_can_override_paths_without_extensions() {
    let dir = tree(&[("base", r#"{"a":1}"#), ("over", r#"{"b":2}"#)]);
    let paths = [dir.path().join("base"), dir.path().join("over")];

    let Layers::Json(layers) =
        knf::load_layers(&paths, Some(Format::Json)).expect("explicit format")
    else {
        panic!("JSON")
    };
    let merged = knf::merge(layers, &MergeOptions::default()).expect("merge succeeds");

    assert_eq!(merged, json!({"a": 1, "b": 2}));
}

#[test]
fn load_layers_accepts_borrowed_paths() {
    let dir = tree(&[("base.json", r#"{"a":1}"#)]);
    let path = dir.path().join("base.json");

    let merged = merge_files(&[Path::new(&path)], &MergeOptions::default()).expect("borrowed path");

    assert_eq!(merged, json!({"a": 1}));
}

/// The load failures are typed, so a caller can act on the *kind* rather than
/// matching a message — and, unlike the string they replaced, they name no
/// command-line flag for a caller that has no command line.
#[test]
fn load_errors_are_typed_and_flag_free() {
    let dir = tree(&[("layer", "{}")]);

    let err = knf::load_layers(&[dir.path().join("layer")], None)
        .expect_err("an extensionless path cannot be typed");
    let load = err
        .downcast_ref::<knf::LoadError>()
        .expect("preserves the load error");
    assert!(matches!(load, knf::LoadError::UnknownExtension { .. }));
    assert_eq!(load.path(), Some(dir.path().join("layer").as_path()));
    assert!(
        !load.to_string().contains("--"),
        "a library error must not name a flag: {load}"
    );

    let err = knf::load_layers(&[dir.path()], None).expect_err("a directory is no layer");
    assert!(matches!(
        err.downcast_ref::<knf::LoadError>(),
        Some(knf::LoadError::Directory { .. })
    ));
}

/// `${env:...}` resolves against whatever the caller calls the environment.
/// Nothing here reads process state, which is the whole point of the seam.
#[test]
fn interpolate_resolves_against_a_supplied_environment() {
    struct StubEnv;
    impl knf::Env for StubEnv {
        fn lookup(&self, name: &str) -> Option<String> {
            (name == "PORT").then(|| "8080".to_string())
        }
    }

    let dir = tree(&[(
        "base.json",
        r#"{"port":"${env:PORT}","url":"x:${env:PORT}"}"#,
    )]);
    let merged = merge_files(&[dir.path().join("base.json")], &MergeOptions::default())
        .expect("merge succeeds");
    let merged = knf::interpolate(merged, &StubEnv).expect("the stub supplies PORT");

    // Whole-string takes the typed value, embedded splices the raw text.
    assert_eq!(merged, json!({"port": 8080, "url": "x:8080"}));
}

/// Interpolation is a separate step: `merge` alone preserves the values in a
/// document full of `${...}`.
#[test]
fn merge_does_not_interpolate() {
    let dir = tree(&[("base.json", r#"{"a":"${b}","b":"literal"}"#)]);
    let merged = merge_files(&[dir.path().join("base.json")], &MergeOptions::default())
        .expect("no references are resolved");
    assert_eq!(merged, json!({"a": "${b}", "b": "literal"}));
}

#[test]
fn mixed_formats_are_rejected_before_contents_are_read() {
    let paths = ["missing.json", "missing.toml"];
    let err = knf::load_layers(&paths, None).unwrap_err();
    let load = err.downcast_ref::<knf::LoadError>().unwrap();
    assert_eq!(*load, knf::LoadError::MixedFormats);
    assert_eq!(load.path(), None);
    assert!(!load.to_string().contains("-f"));
    assert!(!load.to_string().ends_with('\n'));
}

#[test]
fn empty_layers_select_the_default_or_explicit_native_type() {
    assert_eq!(
        knf::load_layers::<&str>(&[], None).unwrap(),
        Layers::Json(vec![])
    );
    assert_eq!(
        knf::load_layers::<&str>(&[], Some(Format::Toml)).unwrap(),
        Layers::Toml(vec![])
    );
    assert_eq!(
        knf::resolve_format(&["-"], None),
        Err(knf::LoadError::StdinNeedsFormat)
    );
    assert_eq!(
        knf::resolve_format(&["-"], Some(Format::Toml)).unwrap(),
        Format::Toml
    );
}

#[test]
fn native_toml_preserves_special_values_and_precision() {
    let source = "date = 1979-05-27T07:32:00.123456789Z\ninf = inf\nnan = nan\n";
    let dir = tree(&[("base.toml", source)]);
    let Layers::Toml(layers) = knf::load_layers(&[dir.path().join("base.toml")], None).unwrap()
    else {
        panic!("TOML")
    };
    let mut merged = knf::merge(layers, &MergeOptions::default()).unwrap();
    let overlay = "copy=${date}"
        .parse::<knf::PathLeaf<String>>()
        .unwrap()
        .into_layer::<toml::Value>()
        .unwrap();
    knf::merge_into(&mut merged, overlay, &MergeOptions::default()).unwrap();
    let merged = knf::interpolate(merged, &EmptyEnv).unwrap();
    assert_eq!(merged["date"], merged["copy"]);
    let emitted = knf::format::emit(merged, true).unwrap();
    assert!(emitted.contains(".123456789Z"));
    let reparsed: toml::Value = toml::from_str(&emitted).unwrap();
    assert!(reparsed["inf"].as_float().unwrap().is_infinite());
    assert!(reparsed["nan"].as_float().unwrap().is_nan());
}

struct EmptyEnv;
impl knf::Env for EmptyEnv {
    fn lookup(&self, _: &str) -> Option<String> {
        None
    }
}

#[test]
fn explicit_format_overrides_extensions_without_converting_values() {
    let dir = tree(&[
        ("a.toml", r#"{"null":null,"id":18446744073709551615}"#),
        ("b.json", r#"{"a":1}"#),
    ]);
    let Layers::Json(layers) = knf::load_layers(
        &[dir.path().join("a.toml"), dir.path().join("b.json")],
        Some(Format::Json),
    )
    .unwrap() else {
        panic!("JSON")
    };
    let emitted =
        knf::format::emit(knf::merge(layers, &MergeOptions::default()).unwrap(), false).unwrap();
    assert_eq!(
        emitted,
        "{\"null\":null,\"id\":18446744073709551615,\"a\":1}\n"
    );
}
