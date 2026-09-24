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
    ok_stdout(knf(dir).args(args), args)
}

/// stderr of a run that must fail with exit code 1.
fn run_err(dir: &TempDir, args: &[&str]) -> String {
    err_stderr(knf(dir).args(args), args)
}

fn ok_stdout(cmd: &mut Command, args: &[&str]) -> String {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "`knf {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8 stdout")
}

fn err_stderr(cmd: &mut Command, args: &[&str]) -> String {
    let out = cmd.output().expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(1),
        "`knf {}` should exit 1:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stderr).expect("utf-8 stderr")
}

/// Sets or unsets named variables, so a `${env:...}` test never depends on the
/// environment the suite happens to run in. `None` removes.
fn with_env<'a>(cmd: &'a mut Command, vars: &[(&str, Option<&str>)]) -> &'a mut Command {
    for (name, value) in vars {
        match value {
            Some(value) => cmd.env(name, value),
            None => cmd.env_remove(name),
        };
    }
    cmd
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

// --- deep and shallow merge ----------------------------------------------

const BASE: &str = "plugins = [\"auth\"]\n[db]\nhost = \"local\"\nport = 5432\n";
const PROD: &str = "plugins = [\"metrics\"]\n[db]\nhost = \"prod\"\n";

/// The default is jq's `*`: the array replaces, the table merges.
#[test]
fn by_default_arrays_replace_and_tables_merge() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    let out = run(&dir, &["base.toml", "prod.toml"]);
    assert!(out.contains("plugins = [\"metrics\"]"), "{out}");
    assert!(out.contains("port = 5432"), "{out}");
}

/// `--shallow` is jq's `+`: the overlay's table is taken whole and `port` is
/// gone. Emitted as compact JSON so the assertion is one exact string.
#[test]
fn shallow_takes_top_level_tables_whole() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    let out = run(
        &dir,
        &[
            "base.toml",
            "prod.toml",
            "--shallow",
            "-f",
            "json",
            "--compact",
        ],
    );
    assert_eq!(
        out,
        "{\"plugins\":[\"metrics\"],\"db\":{\"host\":\"prod\"}}\n"
    );
}

/// One layer is still a no-op under `--shallow`.
#[test]
fn shallow_single_layer_is_identity() {
    let dir = tree(&[("base.toml", BASE)]);
    assert_eq!(
        run(&dir, &["base.toml", "--shallow"]),
        run(&dir, &["base.toml"])
    );
}

/// `--shallow` applies to `--set` layers, which are ordinary terminal layers.
/// Correct, and surprising enough to pin: the whole table is replaced by the
/// one key.
#[test]
fn shallow_applies_to_set_layers_too() {
    let dir = tree(&[("base.toml", BASE)]);
    let out = run(&dir, &["base.toml", "--shallow", "--set", "db.host=x"]);
    assert!(out.contains("host = \"x\""), "{out}");
    assert!(
        !out.contains("port"),
        "--set layer did not replace db:\n{out}"
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
/// to be exempt, emitting the string `"null"`.
#[test]
fn set_null_on_toml_is_an_error() {
    let dir = tree(&[("f.toml", "a = 0\n")]);
    insta::assert_snapshot!(run_err(&dir, &["f.toml", "--set", "proxy=null"]));
}

// --- --null-as ------------------------------------------------------------

/// The escape hatch from the error above: a string the *user* picked, written
/// wherever a null would have been.
#[test]
fn null_as_substitutes_on_toml_output() {
    let dir = tree(&[("f.toml", "a = 0\n")]);
    assert_eq!(
        run(
            &dir,
            &["f.toml", "--set", "proxy=null", "--null-as", "none"]
        ),
        "a = 0\nproxy = \"none\"\n"
    );
}

/// Inside an array a null cannot be dropped without shifting every index after
/// it — the case where `yq` and `tomlq` each silently invent a different string
/// (`""` and `"None"`). Substituting keeps the length and the user's choice.
#[test]
fn null_as_substitutes_inside_arrays() {
    let dir = tree(&[("f.json", r#"{"xs":[1,null,3]}"#)]);
    assert_eq!(
        run(&dir, &["f.json", "-f", "toml", "--null-as", "none"]),
        "xs = [\n    1,\n    \"none\",\n    3,\n]\n"
    );
}

/// JSON can hold a null, so the flag has nothing to rescue there and must not
/// corrupt a document that was never in trouble.
#[test]
fn null_as_leaves_json_output_alone() {
    let dir = tree(&[("f.json", r#"{"proxy":null}"#)]);
    assert_eq!(
        run(&dir, &["f.json", "--null-as", "none"]),
        "{\n  \"proxy\": null\n}\n"
    );
}

// --- --interpolate --------------------------------------------------------

/// A document that is nothing but references, for the tests that must show it
/// passing through untouched.
const REFS: &str = "\
root = \"/srv\"
data_dir = \"${root}/data\"
port = \"${env:KNF_TEST_PORT}\"
url = \"http://localhost:${env:KNF_TEST_PORT}/health\"
literal = \"$${NOT_A_REF}\"
";

/// The reason the flag is opt-in. knf sits upstream of compose files, Actions
/// workflows and Helm charts, whose own syntax is `${...}`; eating those by
/// default would be silent corruption, so without the flag the document is
/// byte-identical — even with the variable set.
#[test]
fn references_pass_through_untouched_without_the_flag() {
    let dir = tree(&[("f.toml", REFS)]);
    let out = ok_stdout(
        with_env(&mut knf(&dir), &[("KNF_TEST_PORT", Some("8080"))]).args(["f.toml"]),
        &["f.toml"],
    );
    assert_eq!(out, REFS);
}

/// The plan's worked example, end to end: a document reference, an environment
/// reference in both positions, and the escape.
#[test]
fn interpolate_resolves_documents_and_the_environment() {
    let dir = tree(&[("f.toml", REFS)]);
    let args = ["f.toml", "--interpolate"];
    let out = ok_stdout(
        with_env(&mut knf(&dir), &[("KNF_TEST_PORT", Some("8080"))]).args(args),
        &args,
    );
    assert_eq!(
        out,
        "root = \"/srv\"\n\
         data_dir = \"/srv/data\"\n\
         port = 8080\n\
         url = \"http://localhost:8080/health\"\n\
         literal = \"${NOT_A_REF}\"\n"
    );
}

/// A whole-string reference takes the referent's *type*, so `"${p}"` emits an
/// unquoted number and `"${db}"` a whole table — while the same reference
/// inside text stringifies.
#[test]
fn whole_string_references_keep_the_referents_type() {
    let dir = tree(&[(
        "f.json",
        r#"{"p":8080,"db":{"host":"local"},"port":"${p}","alias":"${db}","label":"port ${p}"}"#,
    )]);
    assert_eq!(
        run(&dir, &["f.json", "--interpolate", "--compact"]),
        "{\"p\":8080,\"db\":{\"host\":\"local\"},\"port\":8080,\
         \"alias\":{\"host\":\"local\"},\"label\":\"port 8080\"}\n"
    );
}

/// Brackets let a reference read an array element, whole-string or chained —
/// typed exactly as a key reference would be.
#[test]
fn references_read_array_elements() {
    let dir = tree(&[(
        "f.json",
        r#"{"servers":[{"host":"a","port":5432}],"url":"http://${servers[0].host}:${servers[0].port}","primary":"${servers[0]}"}"#,
    )]);
    assert_eq!(
        run(&dir, &["f.json", "--interpolate", "--compact"]),
        "{\"servers\":[{\"host\":\"a\",\"port\":5432}],\
         \"url\":\"http://a:5432\",\"primary\":{\"host\":\"a\",\"port\":5432}}\n"
    );
}

/// The pass runs on the *merged* document, so a reference sees the value the
/// last layer actually left there, not the one in the file it was written in.
#[test]
fn references_read_the_merged_document() {
    let dir = tree(&[
        ("base.json", r#"{"host":"local","url":"http://${host}/"}"#),
        ("prod.json", r#"{"host":"prod"}"#),
    ]);
    assert_eq!(
        run(
            &dir,
            &["base.json", "prod.json", "--interpolate", "--compact"]
        ),
        "{\"host\":\"prod\",\"url\":\"http://prod/\"}\n"
    );
}

/// `--set` is an ordinary layer, so its values interpolate like any other.
#[test]
fn set_layers_interpolate_too() {
    let dir = tree(&[("f.json", r#"{"root":"/srv"}"#)]);
    assert_eq!(
        run(
            &dir,
            &[
                "f.json",
                "--set",
                "data=${root}/data",
                "--interpolate",
                "--compact"
            ]
        ),
        "{\"root\":\"/srv\",\"data\":\"/srv/data\"}\n"
    );
}

/// `--strict` runs during the merge, before any substitution, so it compares
/// the types values had when they were *written*: a `"${p}"` was a string when
/// it looked, whatever it is about to become.
#[test]
fn strict_sees_types_as_written_not_as_resolved() {
    let dir = tree(&[
        ("a.json", r#"{"p":8080,"port":80}"#),
        ("b.json", r#"{"port":"${p}"}"#),
    ]);
    let err = run_err(&dir, &["a.json", "b.json", "--strict", "--interpolate"]);
    assert!(
        err.contains("type conflict at `port`: number would be replaced by string"),
        "{err}"
    );
}

/// A reference resolving to null is an ordinary null: it meets the existing
/// TOML error, and the existing escape rescues it.
#[test]
fn a_null_referent_meets_the_existing_toml_null_error() {
    let dir = tree(&[("f.json", r#"{"n":null,"copy":"${n}"}"#)]);
    let err = run_err(&dir, &["f.json", "-f", "toml", "--interpolate"]);
    assert!(err.contains("cannot serialize null to TOML"), "{err}");
    assert!(err.contains("--> copy"), "{err}");
    assert_eq!(
        run(
            &dir,
            &["f.json", "-f", "toml", "--interpolate", "--null-as", "none"]
        ),
        "n = \"none\"\ncopy = \"none\"\n"
    );
}

// --- --interpolate errors (snapshotted) -----------------------------------

/// Every offender in one run, with paths into the merged document — array
/// indices included, since a reference may live inside an array.
#[test]
fn unresolved_reference_error() {
    let dir = tree(&[(
        "f.json",
        r#"{"server":{"url":"${db.hostname}"},"tags":["${env:KNF_TEST_REGION}"]}"#,
    )]);
    let args = ["f.json", "--interpolate"];
    insta::assert_snapshot!(err_stderr(
        with_env(&mut knf(&dir), &[("KNF_TEST_REGION", None)]).args(args),
        &args,
    ));
}

#[test]
fn embedded_container_reference_error() {
    let dir = tree(&[(
        "f.json",
        r#"{"db":{"host":"x"},"xs":[1],"url":"http://${db}/","tag":"<${xs}>"}"#,
    )]);
    insta::assert_snapshot!(run_err(&dir, &["f.json", "--interpolate"]));
}

#[test]
fn malformed_reference_error() {
    let dir = tree(&[(
        "f.json",
        r#"{"a":"${b","c":"${missing} ${}","d":"${env:}","e":"${x..y}"}"#,
    )]);
    insta::assert_snapshot!(run_err(&dir, &["f.json", "--interpolate"]));
}

#[test]
fn malformed_index_error() {
    let dir = tree(&[("f.json", r#"{"tags":["a"],"t":"${tags[x]}"}"#)]);
    insta::assert_snapshot!(run_err(&dir, &["f.json", "--interpolate"]));
}

#[test]
fn reference_cycle_error() {
    let dir = tree(&[("f.json", r#"{"a":"${b}","b":"${c}","c":"${a}"}"#)]);
    insta::assert_snapshot!(run_err(&dir, &["f.json", "--interpolate"]));
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

/// §3.2. Paths into the *merged* document and nothing else: no layer survives
/// the merge to be named, and both escapes the help line offers (`-f json`,
/// `--null-as`) work without knowing which file the null came from.
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

/// The same shape one format over. TOML's number grammar has `inf` and `nan`
/// literals and JSON's has neither, so an ordinary `.toml` input cannot be
/// emitted as JSON — it used to become `0`, a value that was in no input. The
/// help points at `-f toml`, which is the reverse of the escape the null error
/// offers, and a user who has met that one will reach for `-f json` by reflex.
#[test]
fn non_finite_in_json_error() {
    let dir = tree(&[("a.toml", "timeout = inf\nbackoff = [1.0, nan]\n")]);
    insta::assert_snapshot!(run_err(&dir, &["a.toml", "-f", "json"]));
}

/// The third TOML impossibility, and the one a document reaches. TOML integers
/// are signed 64-bit, so the snowflake ID that `integers_above_i64_max_are_exact`
/// round-trips through JSON has no TOML spelling at all; it used to round through
/// `f64` and emit `1e19`, discarding the digits `Number::U64` exists to keep.
#[test]
fn integer_out_of_range_in_toml_error() {
    let dir = tree(&[(
        "a.json",
        r#"{"id":10000000000000000001,"ok":42,"ids":[1,18446744073709551615]}"#,
    )]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "-f", "toml"]));
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

/// A missing `-f` is a mistake in argv alone, so it is reported before any
/// error in the documents themselves — never one run at a time, where the user
/// fixes the type conflict only to learn about the flag on the next attempt.
/// The layers here conflict under `--strict`; the format check still wins.
#[test]
fn mixed_input_formats_error_precedes_merge_errors() {
    let dir = tree(&[
        ("a.json", r#"{"port":80}"#),
        ("b.toml", "port = \"eighty\"\n"),
    ]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "b.toml", "--strict"]));
}

/// Same precedence with interpolation: it runs after the merge, so a reference
/// that cannot resolve is further still from argv than the type conflict above.
#[test]
fn mixed_input_formats_error_precedes_interpolation_errors() {
    let dir = tree(&[("a.json", r#"{"a":"${nope}"}"#), ("b.toml", "b = 1\n")]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "b.toml", "--interpolate"]));
}

/// A bracketed `--set` parses (a reference may read an element) but cannot
/// write, and saying so must not depend on reading anything either: the
/// file here does not exist.
#[test]
fn set_with_an_array_index_errors_before_file_io() {
    let dir = tree(&[]);
    let err = run_err(&dir, &["missing.toml", "--set", "servers[0]=1"]);
    assert!(
        !err.contains("missing.toml"),
        "the --set path should be rejected before the file is read:\n{err}"
    );
    insta::assert_snapshot!(err);
}

#[test]
fn strict_type_conflict_error() {
    let dir = tree(&[
        ("a.json", r#"{"server":{"port":80}}"#),
        ("b.json", r#"{"server":5}"#),
    ]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "b.json", "--strict"]));
}
