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

fn read(dir: &TempDir, path: &str) -> String {
    std::fs::read_to_string(dir.path().join(path)).expect("read generated file")
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

// --- Cartesian products ---------------------------------------------------

#[test]
fn two_factors_generate_the_cartesian_product_in_order() {
    let dir = tree(&[
        ("foo1.toml", "foo = 1\nwinner = \"foo1\"\n"),
        ("foo2.toml", "foo = 2\nwinner = \"foo2\"\n"),
        ("bar1.toml", "bar = 1\nwinner = \"bar1\"\n"),
        ("bar2.toml", "bar = 2\nwinner = \"bar2\"\n"),
    ]);

    let out = run(
        &dir,
        &[
            "foo1.toml",
            "foo2.toml",
            "x",
            "bar1.toml",
            "bar2.toml",
            "-o",
            "out",
        ],
    );

    assert_eq!(
        out,
        "\
out/foo1+bar1.toml
out/foo1+bar2.toml
out/foo2+bar1.toml
out/foo2+bar2.toml
"
    );
    assert_eq!(
        read(&dir, "out/foo1+bar2.toml"),
        "foo = 1\nwinner = \"bar2\"\nbar = 2\n"
    );
    assert_eq!(
        read(&dir, "out/foo2+bar1.toml"),
        "foo = 2\nwinner = \"bar1\"\nbar = 1\n"
    );
}

#[test]
fn three_factors_preserve_left_to_right_merge_order() {
    let dir = tree(&[
        ("base1.json", r#"{"base":1,"value":{"from":"base"}}"#),
        ("base2.json", r#"{"base":2,"value":{"from":"base"}}"#),
        ("middle.json", r#"{"middle":true,"value":5}"#),
        ("last1.json", r#"{"last":1,"value":{"from":"last1"}}"#),
        ("last2.json", r#"{"last":2,"value":{"from":"last2"}}"#),
    ]);

    let out = run(
        &dir,
        &[
            "base1.json",
            "base2.json",
            "x",
            "middle.json",
            "x",
            "last1.json",
            "last2.json",
            "-o",
            "out",
            "--compact",
        ],
    );

    assert_eq!(out.lines().count(), 4);
    assert_eq!(
        read(&dir, "out/base1+middle+last2.json"),
        "{\"base\":1,\"value\":{\"from\":\"last2\"},\"middle\":true,\"last\":2}\n"
    );
}

#[test]
fn repeated_sets_are_a_final_alternatives_factor() {
    let dir = tree(&[("foo.toml", "foo = true\n"), ("bar.toml", "bar = true\n")]);

    let out = run(
        &dir,
        &[
            "foo.toml",
            "x",
            "bar.toml",
            "--set",
            "region=us",
            "--set",
            "region=eu",
            "-o",
            "out",
        ],
    );

    assert_eq!(
        out,
        "out/foo+bar+region=us.toml\nout/foo+bar+region=eu.toml\n"
    );
    assert_eq!(
        read(&dir, "out/foo+bar+region=eu.toml"),
        "foo = true\nbar = true\nregion = \"eu\"\n"
    );
}

#[test]
fn product_names_percent_encode_unsafe_bytes() {
    let dir = tree(&[
        ("left+side.toml", "left = true\n"),
        ("right.toml", "right = true\n"),
    ]);

    let out = run(
        &dir,
        &[
            "left+side.toml",
            "x",
            "right.toml",
            "--set",
            "path=/tmp/a b",
            "-o",
            "out",
        ],
    );

    assert_eq!(out, "out/left%2Bside+right+path=%2Ftmp%2Fa%20b.toml\n");
}

#[test]
fn dot_slash_escapes_a_file_named_x() {
    let dir = tree(&[
        ("x", r#"{"left":true}"#),
        ("right.json", r#"{"right":true}"#),
    ]);

    let out = run(
        &dir,
        &[
            "./x",
            "x",
            "right.json",
            "--input-format",
            "json",
            "-o",
            "out",
            "--compact",
        ],
    );

    assert_eq!(out, "out/x+right.json\n");
    assert_eq!(
        read(&dir, "out/x+right.json"),
        "{\"left\":true,\"right\":true}\n"
    );
}

#[test]
fn malformed_product_factors_are_rejected() {
    let dir = tree(&[
        ("left.toml", "left = true\n"),
        ("right.toml", "right = true\n"),
    ]);

    for args in [
        vec!["x", "right.toml", "-o", "out"],
        vec!["left.toml", "x", "x", "right.toml", "-o", "out"],
        vec!["left.toml", "x", "-o", "out"],
    ] {
        let err = run_err(&dir, &args);
        assert!(err.contains("factor is empty"), "{err}");
    }
}

#[test]
fn output_dir_is_required_only_for_product_mode() {
    let dir = tree(&[
        ("left.toml", "left = true\n"),
        ("right.toml", "right = true\n"),
    ]);

    let missing = run_err(&dir, &["left.toml", "x", "right.toml"]);
    assert!(missing.contains("requires --output-dir"), "{missing}");

    let misplaced = run_err(&dir, &["left.toml", "-o", "out"]);
    assert!(
        misplaced.contains("--output-dir requires Cartesian-product mode"),
        "{misplaced}"
    );
}

#[test]
fn existing_outputs_are_never_overwritten() {
    let dir = tree(&[
        ("left.toml", "left = true\n"),
        ("right.toml", "right = true\n"),
        ("out/left+right.toml", "keep = \"me\"\n"),
    ]);

    let err = run_err(&dir, &["left.toml", "x", "right.toml", "-o", "out"]);
    assert!(err.contains("refusing to overwrite"), "{err}");
    assert_eq!(read(&dir, "out/left+right.toml"), "keep = \"me\"\n");
}

#[test]
fn duplicate_generated_names_are_rejected_before_writing() {
    let dir = tree(&[
        ("a/same.toml", "source = \"a\"\n"),
        ("b/same.toml", "source = \"b\"\n"),
        ("right.toml", "right = true\n"),
    ]);

    let err = run_err(
        &dir,
        &["a/same.toml", "b/same.toml", "x", "right.toml", "-o", "out"],
    );
    assert!(err.contains("multiple combinations"), "{err}");
    assert!(!dir.path().join("out").exists());
}

#[test]
fn a_generation_error_leaves_no_output_files() {
    let dir = tree(&[
        ("base1.json", r#"{"value":{"nested":true}}"#),
        ("base2.json", r#"{"value":1}"#),
        ("over.json", r#"{"value":2}"#),
    ]);

    let err = run_err(
        &dir,
        &[
            "base1.json",
            "base2.json",
            "x",
            "over.json",
            "--strict",
            "-o",
            "out",
        ],
    );
    assert!(err.contains("type conflict"), "{err}");
    assert_eq!(
        std::fs::read_dir(dir.path().join("out"))
            .expect("output directory")
            .count(),
        0
    );
}

#[test]
fn product_mode_resolves_one_format_across_all_candidates() {
    let dir = tree(&[
        ("left.toml", "left = true\n"),
        ("alternative.json", r#"{"alternative":true}"#),
        ("right.toml", "right = true\n"),
    ]);

    let err = run_err(
        &dir,
        &[
            "left.toml",
            "alternative.json",
            "x",
            "right.toml",
            "-o",
            "out",
        ],
    );
    assert!(err.contains("-f is required"), "{err}");

    let out = run(
        &dir,
        &[
            "left.toml",
            "alternative.json",
            "x",
            "right.toml",
            "-f",
            "json",
            "-o",
            "out",
            "--compact",
        ],
    );
    assert_eq!(out, "out/left+right.json\nout/alternative+right.json\n");
}

#[test]
fn stdin_is_loaded_once_and_reused_across_combinations() {
    let dir = tree(&[
        ("one.json", r#"{"choice":1}"#),
        ("two.json", r#"{"choice":2}"#),
    ]);
    let out = knf(&dir)
        .args([
            "-",
            "x",
            "one.json",
            "two.json",
            "--input-format",
            "json",
            "-o",
            "out",
            "--compact",
        ])
        .write_stdin(r#"{"base":true}"#)
        .output()
        .expect("spawn");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).expect("utf-8"),
        "out/stdin+one.json\nout/stdin+two.json\n"
    );
    assert_eq!(
        read(&dir, "out/stdin+two.json"),
        "{\"base\":true,\"choice\":2}\n"
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
