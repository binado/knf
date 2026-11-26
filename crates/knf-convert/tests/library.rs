use std::path::Path;

use knf::{InterpolationOptions, MergeOptions, Value};
use knf_convert::{Configs, InputFormat, Layout, LoadOptions, OutputFormat, Policy, Role};
use tempfile::TempDir;

macro_rules! json {
    ($($tokens:tt)*) => {
        InputFormat::Json
            .parse_document(&serde_json::json!($($tokens)*).to_string())
            .unwrap()
    };
}

fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, contents) in files {
        std::fs::write(dir.path().join(path), contents).expect("write fixture");
    }
    dir
}

fn overlay(value: Value) -> Value {
    assert!(value.as_object().is_some());
    value
}

/// The whole file pipeline, as a caller composes it: load, then fold.
fn merge_files<P: AsRef<Path>>(paths: &[P], opts: &MergeOptions) -> anyhow::Result<Value> {
    let configs = knf_convert::load_configs(paths, LoadOptions::default())?.values;
    Ok(knf::merge(configs, opts))
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

    let mut configs = knf_convert::load_configs(&paths, LoadOptions::default())
        .expect("configs load")
        .values;
    configs.push(overlay);
    let merged = knf::merge(configs, &MergeOptions::shallow_root());
    let merged = knf::interpolate(
        merged,
        &InterpolationOptions {
            context: None,
            env: &knf::ProcessEnv,
            parse_inline: &|text| InputFormat::Toml.parse_inline(text),
        },
    )
    .expect("document references resolve");

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
fn selected_values_keep_their_native_scalar_representations() {
    let opts = MergeOptions {
        shallow: Some("payload.*".parse().unwrap()),
    };
    let json = knf::merge(
        [
            json!({"payload": {"n": 0, "nil": {"x": 1}}}),
            json!({"payload": {"n": u64::MAX, "nil": null}}),
        ],
        &opts,
    );
    assert_eq!(
        json.as_object().unwrap()["payload"].as_object().unwrap()["n"],
        Value::unsigned(u64::MAX)
    );
    assert_eq!(
        json.as_object().unwrap()["payload"].as_object().unwrap()["nil"],
        Value::Null
    );

    let configs = [
        "[payload]\nstamp = 1979-05-27T07:32:00.123456789+02:00\nx = 0.0\n",
        "[payload]\nstamp = 1980-01-01T00:00:00.987654321Z\nx = inf\n",
    ]
    .map(|text| InputFormat::Toml.parse_document(text).unwrap());
    let toml = knf::merge(configs, &opts);
    assert_eq!(
        toml.as_object().unwrap()["payload"].as_object().unwrap()["stamp"],
        Value::Datetime("1980-01-01T00:00:00.987654321Z".parse().unwrap())
    );
    assert_eq!(
        toml.as_object().unwrap()["payload"].as_object().unwrap()["x"],
        Value::Float(f64::INFINITY)
    );
}

#[test]
fn load_configs_accepts_borrowed_paths() {
    let dir = tree(&[("base.json", r#"{"a":1}"#)]);
    let path = dir.path().join("base.json");

    let merged = merge_files(&[Path::new(&path)], &MergeOptions::default()).expect("borrowed path");

    assert_eq!(merged, json!({"a": 1}));
}

#[test]
fn load_errors_are_typed_and_flag_free() {
    let dir = tree(&[("config", "{}")]);

    let config = dir.path().join("config");
    let load = knf_convert::load_configs(&[&config], LoadOptions::default())
        .expect_err("an extensionless path cannot be typed");
    assert!(matches!(
        &load,
        knf_convert::LoadError::UnknownExtension { path, role: Role::Input } if *path == config
    ));
    assert!(
        !load.to_string().contains("--"),
        "a library error must not name a flag: {load}"
    );

    let err = knf_convert::load_configs(&[dir.path()], LoadOptions::default())
        .expect_err("a directory is no config");
    assert!(matches!(err, knf_convert::LoadError::Directory { .. }));

    let err = knf_convert::load_configs::<&str>(&[], LoadOptions::default())
        .expect_err("nothing to infer from");
    assert!(matches!(err, knf_convert::LoadError::NoInputs));
    assert!(!err.to_string().contains("--"));
}

#[test]
fn file_errors_name_the_failing_path() {
    let dir = tree(&[("base.json", "{}"), ("broken.json", "[")]);
    let broken = dir.path().join("broken.json");
    let err = knf_convert::load_configs(
        &[dir.path().join("base.json"), broken.clone()],
        LoadOptions::default(),
    )
    .unwrap_err();
    let knf_convert::LoadError::File { path, .. } = &err else {
        panic!("expected a file error, got {err:?}");
    };
    assert_eq!(*path, broken);
    let chain: Vec<String> =
        std::iter::successors(Some(&err as &dyn std::error::Error), |cause| cause.source())
            .map(ToString::to_string)
            .collect();
    assert_eq!(
        chain[..2],
        [
            format!("loading `{}`", broken.display()),
            "invalid JSON".into()
        ]
    );
    assert_eq!(chain.len(), 3, "keeps the parser cause: {chain:?}");

    let missing = dir.path().join("missing.json");
    let err = knf_convert::load_configs(
        &[dir.path().join("base.json")],
        LoadOptions {
            context: Some(&missing),
            ..LoadOptions::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(&err, knf_convert::LoadError::File { path, role: Role::Context, .. } if *path == missing)
    );
}

#[test]
fn context_loads_apart_from_the_configs() {
    let dir = tree(&[("base.toml", "a = 1"), ("ctx.toml", "b = 2")]);
    let configs = knf_convert::load_configs(
        &[dir.path().join("base.toml")],
        LoadOptions {
            context: Some(&dir.path().join("ctx.toml")),
            ..LoadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(configs.format, InputFormat::Toml);
    assert_eq!(configs.values, [json!({"a": 1})]);
    assert_eq!(configs.context, Some(json!({"b": 2})));

    let err = knf_convert::load_configs(
        &[dir.path().join("base.toml")],
        LoadOptions {
            context: Some(Path::new("ctx.json")),
            ..LoadOptions::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        knf_convert::LoadError::MixedFormats {
            role: Role::Context,
            ..
        }
    ));
}

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
    let merged = knf::interpolate(
        merged,
        &InterpolationOptions {
            context: None,
            env: &StubEnv,
            parse_inline: &|text| InputFormat::Json.parse_inline(text),
        },
    )
    .expect("the stub supplies PORT");

    // Whole-string takes the typed value, embedded splices the raw text.
    assert_eq!(merged, json!({"port": 8080, "url": "x:8080"}));
}

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
    let load = knf_convert::load_configs(&paths, LoadOptions::default()).unwrap_err();
    assert_eq!(
        load.to_string(),
        "`missing.toml` is TOML but `missing.json` is JSON; configs must use one format"
    );
    assert!(!load.to_string().ends_with('\n'));
}

#[test]
fn empty_configs_take_the_fallback_format() {
    assert_eq!(
        knf_convert::load_configs::<&str>(
            &[],
            LoadOptions {
                fallback: Some(InputFormat::Toml),
                ..LoadOptions::default()
            },
        )
        .unwrap(),
        Configs {
            format: InputFormat::Toml,
            values: vec![],
            context: None,
        }
    );
}

#[test]
fn native_toml_preserves_special_values_and_precision() {
    let source = "date = 1979-05-27T07:32:00.123456789Z\ninf = inf\nnan = nan\n";
    let dir = tree(&[("base.toml", source)]);
    let configs =
        knf_convert::load_configs(&[dir.path().join("base.toml")], LoadOptions::default())
            .unwrap()
            .values;
    let mut merged = knf::merge(configs, &MergeOptions::default());
    let overlay = "copy=${date}"
        .parse::<knf::PathLeaf<String>>()
        .unwrap()
        .map_leaf(|text| InputFormat::Toml.parse_inline(text))
        .into();
    knf::merge_into(&mut merged, overlay, &MergeOptions::default());
    let merged = knf::interpolate(
        merged,
        &InterpolationOptions {
            context: None,
            env: &EmptyEnv,
            parse_inline: &|text| InputFormat::Json.parse_inline(text),
        },
    )
    .unwrap();
    assert_eq!(
        merged.as_object().unwrap()["date"],
        merged.as_object().unwrap()["copy"]
    );
    let emitted = OutputFormat::Toml
        .emit(&merged, Policy::Strict, Layout::Pretty)
        .unwrap()
        .value;
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
