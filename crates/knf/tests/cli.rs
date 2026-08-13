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

/// The correction to the design: `toml` deserializes a datetime into a sentinel
/// map and re-serializes it only from a struct name, so TOML -> TOML is lossless
/// only because emission unwraps the sentinel back into a real `Datetime`.
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
fn set_layers_apply_last() {
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

// --- exit codes -----------------------------------------------------------

/// clap's own usage errors exit 2; everything else exits 1.
#[test]
fn usage_errors_exit_two() {
    let dir = tree(&[]);
    let out = knf(&dir).arg("--no-such-flag").output().expect("spawn");
    assert_eq!(out.status.code(), Some(2));
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
