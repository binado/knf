//! CLI-level behaviour: round-trips per format, exit codes, stdin, and the
//! multi-line error messages whose formatting is worth reviewing.
//!
//! Commands run with `current_dir` set to the fixture, so paths in output are
//! relative and the snapshots stay stable.

use assert_cmd::Command;
use tempfile::TempDir;

fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, contents) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(&full, contents).expect("write");
    }
    dir
}

fn knf(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("knf").expect("binary built");
    cmd.current_dir(dir.path());
    cmd
}

/// stdout of a run that must succeed.
fn run(dir: &TempDir, args: &[&str]) -> String {
    let out = knf(dir).args(args).output().expect("spawn");
    assert!(
        out.status.success(),
        "`knf {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8 stdout")
}

/// stderr of a run that must fail with exit code 1.
fn run_err(dir: &TempDir, args: &[&str]) -> String {
    let out = knf(dir).args(args).output().expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(1),
        "`knf {}` should exit 1:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stderr).expect("utf-8 stderr")
}

// --- round-trips ----------------------------------------------------------

const DATED: &str = "\
name = \"svc\"
date = 1979-05-27T07:32:00Z
";

/// TOML → TOML is lossless because the IR has a `Datetime` variant; the merge
/// never sees it as a string.
#[test]
fn toml_datetime_survives_a_toml_round_trip() {
    let dir = tree(&[("f.toml", DATED)]);
    let out = run(&dir, &["f.toml"]);
    assert!(
        out.contains("date = 1979-05-27T07:32:00Z"),
        "datetime was not emitted unquoted:\n{out}"
    );
    assert!(
        !out.contains("__toml_private"),
        "sentinel leaked into output:\n{out}"
    );
}

/// A datetime stays a datetime even when `--set` stacks a JSON-typed layer on
/// top of it.
#[test]
fn toml_datetime_survives_set() {
    let dir = tree(&[("f.toml", DATED)]);
    let out = run(&dir, &["f.toml", "--set", "extra=1"]);
    assert!(
        out.contains("date = 1979-05-27T07:32:00Z"),
        "datetime was not emitted unquoted:\n{out}"
    );
    assert!(
        !out.contains("__toml_private"),
        "sentinel leaked into output:\n{out}"
    );
    assert!(out.contains("extra = 1"), "set layer missing:\n{out}");
}

/// Mixing a JSON layer in no longer costs the datetime its type: there is no
/// JSON detour to launder it through, only the one IR both formats parse into.
#[test]
fn mixed_toml_json_to_toml_preserves_datetime() {
    let dir = tree(&[("f.toml", DATED), ("g.json", r#"{"extra":1}"#)]);
    let out = run(&dir, &["f.toml", "g.json", "-f", "toml"]);
    assert!(
        out.contains("date = 1979-05-27T07:32:00Z"),
        "datetime was not emitted unquoted:\n{out}"
    );
    assert!(
        !out.contains("__toml_private"),
        "sentinel leaked into output:\n{out}"
    );
    assert!(out.contains("extra = 1"), "json overlay missing:\n{out}");
}

/// The other half of the same change: `--strict` compares a datetime against a
/// string rather than string against string, so the conflict is caught.
#[test]
fn strict_catches_a_json_string_over_a_toml_datetime() {
    let dir = tree(&[
        ("f.toml", DATED),
        ("g.json", r#"{"date":"1979-05-27T07:32:00Z"}"#),
    ]);
    let err = run_err(&dir, &["f.toml", "g.json", "-f", "toml", "--strict"]);
    assert!(
        err.contains("type conflict at `date`: datetime would be replaced by string"),
        "{err}"
    );
}

/// Under `-f json` the same datetime is a plain string, not the sentinel map.
#[test]
fn toml_datetime_becomes_a_json_string() {
    let dir = tree(&[("f.toml", DATED)]);
    let out = run(&dir, &["f.toml", "-f", "json", "--compact"]);
    assert_eq!(
        out,
        "{\"name\":\"svc\",\"date\":\"1979-05-27T07:32:00Z\"}\n"
    );
}

/// `preserve_order` must hold on both sides: serde_json's map preserves input
/// order, and `toml`'s writer must not re-sort it on the way out.
#[test]
fn key_order_is_preserved_in_both_formats() {
    let dir = tree(&[
        ("a.json", r#"{"zebra":1,"apple":2,"middle":3}"#),
        ("a.toml", "zebra = 1\napple = 2\nmiddle = 3\n"),
    ]);
    assert_eq!(
        run(&dir, &["a.json", "--compact"]),
        "{\"zebra\":1,\"apple\":2,\"middle\":3}\n"
    );
    assert_eq!(run(&dir, &["a.toml"]), "zebra = 1\napple = 2\nmiddle = 3\n");
}

/// A JSON integer above `i64::MAX` — a snowflake ID, a hash — must round-trip
/// exactly. Routing it through `f64` would round it to ...808 silently, which
/// is the one data corruption the IR could plausibly introduce.
#[test]
fn integers_above_i64_max_are_exact() {
    let doc = r#"{"id":10000000000000000001,"max":18446744073709551615}"#;
    let dir = tree(&[("a.json", doc)]);
    assert_eq!(run(&dir, &["a.json", "--compact"]), format!("{doc}\n"));
}

/// §2.1: one argument must be a no-op, which is why null is a value and not a
/// delete instruction.
#[test]
fn a_single_layer_is_a_no_op() {
    let doc = r#"{"a":{"b":1},"n":null,"xs":[1,2]}"#;
    let dir = tree(&[("a.json", doc)]);
    assert_eq!(run(&dir, &["a.json", "--compact"]), format!("{doc}\n"));
}

#[test]
fn cross_format_layers_merge() {
    let dir = tree(&[
        ("base.toml", "[server]\nport = 80\nhost = \"local\"\n"),
        ("over.json", r#"{"server":{"port":443}}"#),
    ]);
    assert_eq!(
        run(&dir, &["base.toml", "over.json", "-f", "json", "--compact"]),
        "{\"server\":{\"port\":443,\"host\":\"local\"}}\n"
    );
}

#[test]
fn stdin_is_a_layer() {
    let dir = tree(&[("base.json", r#"{"a":1,"b":2}"#)]);
    let out = knf(&dir)
        .args(["base.json", "-", "--input-format", "json", "--compact"])
        .write_stdin(r#"{"b":99}"#)
        .output()
        .expect("spawn");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).expect("utf-8"),
        "{\"a\":1,\"b\":99}\n"
    );
}

#[test]
fn inline_configs_apply_last() {
    let dir = tree(&[("a.json", r#"{"server":{"port":80}}"#)]);
    assert_eq!(
        run(&dir, &["a.json", "--set", "server.port=8080", "--compact"]),
        "{\"server\":{\"port\":8080}}\n"
    );
    // With no file inputs at all, the output defaults to JSON.
    assert_eq!(
        run(&dir, &["--set", "a.b=1", "--compact"]),
        "{\"a\":{\"b\":1}}\n"
    );
}

/// The null pre-check runs on the *merged* document, so a null that a later
/// layer overwrites never reaches TOML conversion and is not an error.
#[test]
fn set_null_overwritten_before_toml_emit() {
    let dir = tree(&[("f.toml", "a = 0\n")]);
    assert_eq!(
        run(&dir, &["f.toml", "--set", "a=null", "--set", "a=1"]),
        "a = 1\n"
    );
}

/// §3.2: a null reaching TOML is an error wherever it came from. `--set` used
/// to be exempt, emitting the string `"null"`; provenance names the expression.
#[test]
fn set_null_on_toml_is_an_error() {
    let dir = tree(&[("f.toml", "a = 0\n")]);
    insta::assert_snapshot!(run_err(&dir, &["f.toml", "--set", "proxy=null"]));
}

// --- exit codes -----------------------------------------------------------

/// clap's own usage errors exit 2; everything else exits 1.
#[test]
fn usage_errors_exit_two() {
    let dir = tree(&[]);
    let out = knf(&dir).arg("--no-such-flag").output().expect("spawn");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn malformed_set_exits_two() {
    let dir = tree(&[]);
    let out = knf(&dir)
        .args(["--set", "noequals"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("invalid value"), "{err}");
    assert!(err.contains("expected KEY.PATH=VALUE"), "{err}");
}

#[test]
fn a_missing_file_exits_one() {
    let dir = tree(&[]);
    let err = run_err(&dir, &["nope.json"]);
    assert!(err.contains("nope.json"), "{err}");
}

/// §2.3: a bare array is legal JSON but is not a config.
#[test]
fn a_non_object_root_is_rejected_by_name() {
    let dir = tree(&[("a.json", "[1,2]")]);
    let err = run_err(&dir, &["a.json"]);
    assert!(err.contains("a.json"), "{err}");
    assert!(err.contains("found array"), "{err}");
}

// --- multi-line error messages (snapshotted) ------------------------------

/// §3.2. The paths and the originating file come from a post-hoc lookup over
/// the parsed inputs, not from provenance threaded through the merge.
#[test]
fn null_in_toml_error() {
    let dir = tree(&[
        ("base.toml", "[servers.primary]\nhost = \"a\"\n"),
        (
            "override.json",
            r#"{"servers":{"primary":{"proxy":null}},"logging":{"sink":null}}"#,
        ),
    ]);
    insta::assert_snapshot!(run_err(&dir, &["base.toml", "override.json", "-f", "toml"]));
}

/// Directories are files-as-layers, never expanded. The help line must be
/// runnable exactly as printed.
#[test]
fn directory_in_the_default_command_error() {
    let dir = tree(&[("config/base.toml", "a = 1\n")]);
    insta::assert_snapshot!(run_err(&dir, &["config"]));
}

#[test]
fn mixed_input_formats_error() {
    let dir = tree(&[("a.toml", "a = 1\n"), ("b.json", "{}")]);
    insta::assert_snapshot!(run_err(&dir, &["a.toml", "b.json"]));
}

#[test]
fn strict_type_conflict_error() {
    let dir = tree(&[
        ("a.json", r#"{"server":{"port":80}}"#),
        ("b.json", r#"{"server":5}"#),
    ]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "b.json", "--strict"]));
}
