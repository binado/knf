use std::path::Path;

use knf::{Map, MergeOptions, Value, format::Format};
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
    knf::value::to_json(value).expect("no non-finite floats in these fixtures")
}

/// An overlay is built from a [`Map`], which keeps a scalar layer — which would
/// replace the whole document rather than shadow a key — out of the fold.
fn overlay(json: serde_json::Value) -> Value {
    let serde_json::Value::Object(map) = json else {
        panic!("an overlay fixture must be an object")
    };
    Value::Object(knf::value::object_from_json(map))
}

/// The whole file pipeline, as a caller composes it: load, then fold.
fn merge_files<P: AsRef<Path>>(paths: &[P], opts: &MergeOptions) -> anyhow::Result<Value> {
    let (layers, _formats) = knf::load_layers(paths, None)?;
    Ok(knf::merge(layers, opts)?)
}

#[test]
fn load_and_merge_paths_without_a_cli() {
    let dir = tree(&[
        ("base.toml", "[server]\nhost = \"local\"\nport = 80\n"),
        ("prod.json", r#"{"server":{"port":443},"debug":true}"#),
    ]);
    let paths = [dir.path().join("base.toml"), dir.path().join("prod.json")];

    let merged = merge_files(&paths, &MergeOptions::default()).expect("merge succeeds");

    assert_eq!(
        as_json(merged),
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

    let (mut layers, _) = knf::load_layers(&paths, None).expect("layers load");
    layers.push(overlay);
    let merged = knf::merge(layers, &MergeOptions::SHALLOW).expect("shallow merge succeeds");
    let merged = knf::interpolate(merged, &knf::ProcessEnv).expect("document references resolve");

    assert_eq!(
        as_json(merged),
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

    let (mut layers, _) = knf::load_layers(&paths, None).expect("layers load");
    layers.push(overlay);
    let err = knf::merge(layers, &MergeOptions::STRICT).expect_err("strict overlay must fail");

    assert_eq!(err.path(), ["port"]);
}

#[test]
fn input_format_can_override_paths_without_extensions() {
    let dir = tree(&[("base", r#"{"a":1}"#), ("over", r#"{"b":2}"#)]);
    let paths = [dir.path().join("base"), dir.path().join("over")];

    let (layers, formats) = knf::load_layers(&paths, Some(Format::Json)).expect("explicit format");
    assert_eq!(formats, [Format::Json, Format::Json]);
    let merged = knf::merge(layers, &MergeOptions::default()).expect("merge succeeds");

    assert_eq!(as_json(merged), json!({"a": 1, "b": 2}));
}

#[test]
fn load_layers_accepts_borrowed_paths() {
    let dir = tree(&[("base.json", r#"{"a":1}"#)]);
    let path = dir.path().join("base.json");

    let merged = merge_files(&[Path::new(&path)], &MergeOptions::default()).expect("borrowed path");

    assert_eq!(as_json(merged), json!({"a": 1}));
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
    let merged = merge_files(&[dir.path().join("base.json")], &MergeOptions::default())
        .expect("merge succeeds");
    let merged = knf::interpolate(merged, &StubEnv).expect("the stub supplies PORT");

    // Whole-string takes the typed value, embedded splices the raw text.
    assert_eq!(as_json(merged), json!({"port": 8080, "url": "x:8080"}));
}

/// Interpolation is a separate step: `merge` alone is a byte-level no-op over a
/// document full of `${...}`.
#[test]
fn merge_does_not_interpolate() {
    let dir = tree(&[("base.json", r#"{"a":"${b}","b":"literal"}"#)]);
    let merged = merge_files(&[dir.path().join("base.json")], &MergeOptions::default())
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
    let merged = merge_files(&[dir.path().join("base.json")], &MergeOptions::default())
        .expect("a null is an ordinary value up to the emit");

    let err = knf::format::emit(merged, Format::Toml, true, None)
        .expect_err("a null cannot be serialized to TOML");
    let Some(knf::TomlError::Null(report)) = err.downcast_ref::<knf::TomlError>() else {
        panic!("preserves the typed error, got {err}")
    };

    let text = report.to_string();
    assert_eq!(text, "cannot serialize null to TOML\n  --> a.b\n  --> c[1]");
    for flag in ["--null-as", "-f json"] {
        assert!(
            !text.contains(flag),
            "a library error must not name a flag: {text}"
        );
    }
}

/// The other half of that error, and the one a consumer reaches by accident.
///
/// `Value::Datetime` is a public variant holding a plain `String`, and an overlay
/// is a `Map` of whatever the caller put in it — so a layer assembled in memory
/// can carry a spelling that is not a TOML datetime. Nothing in the pipeline
/// produces one (`${env:...}` and `--set` both type through JSON, which has no
/// datetime), which is why the conversion used to assume it could not happen and
/// abort the process. It reports the path instead.
#[test]
fn a_hand_built_datetime_that_does_not_reparse_is_an_error_not_a_panic() {
    let dir = tree(&[("base.toml", "name = \"svc\"\n")]);
    let mut overlay = Map::new();
    overlay.insert("created".to_string(), Value::Datetime("nope".to_string()));

    let (mut layers, _) = knf::load_layers(&[dir.path().join("base.toml")], None).expect("toml");
    layers.push(Value::Object(overlay));
    let merged = knf::merge(layers, &MergeOptions::default())
        .expect("a datetime is an ordinary value right up to the emit");

    let err = knf::format::emit(merged, Format::Toml, true, None)
        .expect_err("`nope` is not a TOML datetime");
    let Some(knf::TomlError::Datetime(report)) = err.downcast_ref::<knf::TomlError>() else {
        panic!("preserves the typed error, got {err}")
    };

    let text = report.to_string();
    assert_eq!(
        text,
        "cannot serialize datetime to TOML\n  --> created: `nope`"
    );
    // Asserted flag by flag rather than against a bare `--`, for the reason the
    // null report is: the path lines are themselves spelled `  --> created`.
    for flag in ["--null-as", "-f json", "--set"] {
        assert!(
            !text.contains(flag),
            "a library error must not name a flag: {text}"
        );
    }
}

/// TOML's number grammar has `inf`, `-inf` and `nan`; JSON's has none of them.
///
/// So an ordinary `.toml` layer carries a value that cannot be emitted as JSON,
/// and this is the one impossibility on that side of the boundary. It used to be
/// swallowed — the float became `0`, which is a value that was in no input — and
/// it now names every offending key, saying nothing about how to spell the fix.
#[test]
fn the_non_finite_report_locates_the_floats_and_names_no_flag() {
    let dir = tree(&[("base.toml", "timeout = inf\nbackoff = [1.0, nan]\n")]);
    let merged = merge_files(&[dir.path().join("base.toml")], &MergeOptions::default())
        .expect("inf is an ordinary value up to the emit");

    let err = knf::format::emit(merged, Format::Json, true, None)
        .expect_err("JSON has no spelling for inf");
    let Some(report) = err.downcast_ref::<knf::NonFiniteFloat>() else {
        panic!("preserves the typed error, got {err}")
    };

    let text = report.to_string();
    assert_eq!(
        text,
        "cannot serialize non-finite number to JSON\n  --> timeout: `inf`\n  --> backoff[1]: `nan`"
    );
    // Flag by flag rather than a bare `--`, for the reason the null report is:
    // the path lines are themselves spelled `  --> timeout`.
    for flag in ["-f toml", "--input-format"] {
        assert!(
            !text.contains(flag),
            "a library error must not name a flag: {text}"
        );
    }
}

/// The value `Number::U64` exists to carry, met at the one boundary that cannot
/// carry it.
///
/// A snowflake ID above `i64::MAX` round-trips exactly through JSON, which is why
/// the variant is there at all; TOML integers are signed 64-bit, so the same
/// document has no TOML spelling. It used to round through `f64` and emit
/// `1e19` — silently discarding the digits — and now reports the path instead.
#[test]
fn the_integer_out_of_range_report_locates_the_integers_and_names_no_flag() {
    let dir = tree(&[("base.json", r#"{"id":10000000000000000001,"ok":42}"#)]);
    let merged = merge_files(&[dir.path().join("base.json")], &MergeOptions::default())
        .expect("a large integer is an ordinary value up to the emit");

    let err = knf::format::emit(merged, Format::Toml, true, None)
        .expect_err("TOML integers are signed 64-bit");
    let Some(knf::TomlError::Integer(report)) = err.downcast_ref::<knf::TomlError>() else {
        panic!("preserves the typed error, got {err}")
    };

    let text = report.to_string();
    assert_eq!(
        text,
        "cannot serialize integer to TOML\n  --> id: `10000000000000000001`"
    );
    for flag in ["-f json", "--input-format"] {
        assert!(
            !text.contains(flag),
            "a library error must not name a flag: {text}"
        );
    }
}
