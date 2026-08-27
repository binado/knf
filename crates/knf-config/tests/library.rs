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

/// The load failures are typed, so a caller can act on the *kind* rather than
/// matching a message — and, unlike the string they replaced, they name no
/// command-line flag for a caller that has no command line.
#[test]
fn load_errors_are_typed_and_flag_free() {
    let dir = tree(&[("layer", "{}")]);

    let err = knf::merge(&[dir.path().join("layer")], MergeOpts::default())
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

    let err = knf::merge(&[dir.path()], MergeOpts::default()).expect_err("a directory is no layer");
    assert!(matches!(
        err.downcast_ref::<knf::LoadError>(),
        Some(knf::LoadError::Directory { .. })
    ));
}

/// `${env:...}` resolves against whatever the caller calls the environment.
/// Nothing here reads process state, which is the whole point of the seam.
#[test]
fn merge_with_env_resolves_against_a_supplied_environment() {
    struct StubEnv;
    impl knf::Env for StubEnv {
        fn lookup(&self, name: &str) -> Option<knf::EnvValue> {
            (name == "PORT").then(|| knf::EnvValue {
                raw: "8080".to_string(),
                typed: knf::value::from_json(knf::json_or_string("8080".to_string())),
            })
        }
    }

    let dir = tree(&[(
        "base.json",
        r#"{"port":"${env:PORT}","url":"x:${env:PORT}"}"#,
    )]);
    let merged = knf::merge_with_env(
        &[dir.path().join("base.json")],
        MergeOpts {
            interpolate: true,
            ..MergeOpts::default()
        },
        &StubEnv,
    )
    .expect("the stub supplies PORT");

    // Whole-string takes the typed value, embedded splices the raw text.
    assert_eq!(as_json(merged), json!({"port": 8080, "url": "x:8080"}));
}

/// Interpolation is opt-in, and the default has to keep saying so: `merge` with
/// default options is a byte-level no-op over a document full of `${...}`.
#[test]
fn interpolation_is_off_by_default() {
    assert!(!MergeOpts::default().interpolate);

    let dir = tree(&[("base.json", r#"{"a":"${b}","b":"literal"}"#)]);
    let merged = knf::merge(&[dir.path().join("base.json")], MergeOpts::default())
        .expect("no references are resolved");
    assert_eq!(as_json(merged), json!({"a": "${b}", "b": "literal"}));
}

/// The other error a caller has to be able to act on without a command line.
///
/// A null cannot go to TOML, and every remedy for that is interface-shaped —
/// emit JSON, substitute a string, drop the null — so the report locates the
/// nulls and says nothing about how to spell the fix. Asserted against the two
/// flags specifically rather than a bare `--`, because the report's own path
/// lines are spelled `  --> a.b`.
#[test]
fn the_null_in_toml_report_locates_the_nulls_and_names_no_flag() {
    let dir = tree(&[("base.json", r#"{"a":{"b":null},"c":[1,null]}"#)]);
    let merged = knf::merge(&[dir.path().join("base.json")], MergeOpts::default())
        .expect("a null is an ordinary value up to the emit");

    let err = knf::format::emit(merged, Format::Toml, true, None)
        .expect_err("a null cannot be serialized to TOML");
    let report = err
        .downcast_ref::<knf::NullInToml>()
        .expect("preserves the typed error");

    let text = report.to_string();
    assert_eq!(text, "cannot serialize null to TOML\n  --> a.b\n  --> c[1]");
    for flag in ["--null-as", "-f json"] {
        assert!(
            !text.contains(flag),
            "a library error must not name a flag: {text}"
        );
    }
}
