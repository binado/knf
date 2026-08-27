use std::path::Path;

use knf::{Map, MergeOpts, Rules, Strategy, Value, format::Format};
use serde_json::json;
use tempfile::TempDir;

fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, contents) in files {
        std::fs::write(dir.path().join(path), contents).expect("write fixture");
    }
    dir
}

fn as_json(value: Value) -> serde_json::Value {
    knf::value::to_json(value)
}

/// An overlay is a [`Map`]: the type is what keeps a scalar layer — which would
/// replace the whole document rather than shadow a key — out of the fold.
fn overlay(json: serde_json::Value) -> Map {
    let serde_json::Value::Object(map) = json else {
        panic!("an overlay fixture must be an object")
    };
    knf::value::object_from_json(map)
}

#[test]
fn merge_loads_parses_and_merges_paths_without_a_cli() {
    let dir = tree(&[
        ("base.toml", "[server]\nhost = \"local\"\nport = 80\n"),
        ("prod.json", r#"{"server":{"port":443},"debug":true}"#),
    ]);
    let paths = [dir.path().join("base.toml"), dir.path().join("prod.json")];

    let merged = knf::merge(&paths, MergeOpts::default()).expect("merge succeeds");

    assert_eq!(
        as_json(merged),
        json!({"server": {"host": "local", "port": 443}, "debug": true})
    );
}

#[test]
fn merge_options_cover_rules_terminal_overlays_and_interpolation() {
    let dir = tree(&[
        (
            "base.json",
            r#"{"plugins":["auth"],"port":80,"root":"/srv"}"#,
        ),
        ("prod.json", r#"{"plugins":["metrics"]}"#),
    ]);
    let paths = [dir.path().join("base.json"), dir.path().join("prod.json")];
    let rules = Rules::build([(vec!["plugins".into()], Strategy::Append)]).expect("valid rule");
    let overlay = overlay(json!({"port": 443, "data": "${root}/data"}));

    let merged = knf::merge(
        &paths,
        MergeOpts {
            rules,
            overlays: vec![overlay],
            interpolate: true,
            ..MergeOpts::default()
        },
    )
    .expect("configured merge succeeds");

    assert_eq!(
        as_json(merged),
        json!({
            "plugins": ["auth", "metrics"],
            "port": 443,
            "root": "/srv",
            "data": "/srv/data"
        })
    );
}

#[test]
fn strict_overlay_errors_preserve_the_core_error() {
    let dir = tree(&[("base.json", r#"{"port":80}"#)]);
    let paths = [dir.path().join("base.json")];
    let overlay = overlay(json!({"port": "wrong kind"}));

    let err = knf::merge(
        &paths,
        MergeOpts {
            strict: true,
            overlays: vec![overlay],
            ..MergeOpts::default()
        },
    )
    .expect_err("strict overlay must fail");

    assert_eq!(
        err.downcast_ref::<knf::MergeError>()
            .expect("preserves the core error")
            .path(),
        ["port"]
    );
}

#[test]
fn input_format_can_override_paths_without_extensions() {
    let dir = tree(&[("base", r#"{"a":1}"#), ("over", r#"{"b":2}"#)]);
    let paths = [dir.path().join("base"), dir.path().join("over")];

    let merged = knf::merge(
        &paths,
        MergeOpts {
            input_format: Some(Format::Json),
            ..MergeOpts::default()
        },
    )
    .expect("explicit input format");

    assert_eq!(as_json(merged), json!({"a": 1, "b": 2}));
}

#[test]
fn merge_accepts_borrowed_paths() {
    let dir = tree(&[("base.json", r#"{"a":1}"#)]);
    let path = dir.path().join("base.json");

    let merged = knf::merge(&[Path::new(&path)], MergeOpts::default()).expect("borrowed path");

    assert_eq!(as_json(merged), json!({"a": 1}));
}
