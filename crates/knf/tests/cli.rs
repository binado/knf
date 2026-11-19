//! CLI integration tests. Commands run inside the fixture directory so output
//! paths are relative and snapshots stable.

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

/// Compare native TOML output with a small JSON fixture describing common values.
fn assert_toml_eq_json(output: &str, expected: &str) {
    let got: toml::Value = toml::from_str(output).unwrap();
    let json: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, toml::Value::try_from(json).unwrap());
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

/// Sets (`Some`) or removes (`None`) environment variables for one command.
fn with_env<'a>(cmd: &'a mut Command, vars: &[(&str, Option<&str>)]) -> &'a mut Command {
    for (name, value) in vars {
        match value {
            Some(value) => cmd.env(name, value),
            None => cmd.env_remove(name),
        };
    }
    cmd
}

#[test]
fn exec_requires_delimiter_and_command() {
    let dir = tree(&[]);
    for args in [
        &["--exec", "some-command"][..],
        &["--exec", "--"][..],
        &["--list-files", "--exec", "--", "some-command"][..],
    ] {
        knf(&dir).args(args).assert().code(2);
    }
}

#[test]
fn exec_scan_ignores_exec_and_delimiter_that_are_not_structural() {
    let dir = tree(&[("a.json", "{}")]);
    // `--exec` after the first `--` is a filename, not the flag.
    assert!(run_err(&dir, &["--", "--exec"]).contains("cannot infer a format from `--exec`"));
    // A repeated flag is a clap error, not a command.
    knf(&dir)
        .args(["a.json", "--exec", "--exec", "--", "true"])
        .assert()
        .code(2);
    // An option value of `--exec` or `--` is never structural.
    for args in [
        &["a.json", "--shallow", "--exec", "--", "true"][..],
        &["a.json", "-c", "--exec", "--", "true"][..],
        &["a.json", "--shallow", "--", "true"][..],
        &["a.json", "-c", "--", "true"][..],
    ] {
        knf(&dir).args(args).assert().code(2);
    }
    // Attached values are not mistaken for the flag.
    knf(&dir)
        .args(["a.json", "-i", "--shallow=--exec", "--exec", "--", "true"])
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn exec_command_may_itself_contain_exec_and_delimiters() {
    let dir = tree(&[("a.json", "{}")]);
    let out = knf(&dir)
        .args([
            "a.json", "--exec", "--", "echo", "--exec", "--", "--exec", "x",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"--exec -- --exec x\n");
}

#[cfg(unix)]
#[test]
fn exec_spawn_failure_reports_the_program_and_removes_snapshot() {
    let dir = tree(&[("a.json", "{}")]);
    let tmp = tempfile::tempdir().unwrap();
    let args = ["a.json", "--exec", "--", "knf-no-such-program"];
    let mut cmd = knf(&dir);
    let err = err_stderr(
        with_env(
            cmd.args(args),
            &[("TMPDIR", Some(tmp.path().to_str().unwrap()))],
        ),
        &args,
    );
    assert!(err.contains("running \"knf-no-such-program\""), "{err}");
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn exec_reports_signal_death_as_128_plus_signal() {
    let dir = tree(&[("a.json", r#"{"a":1}"#)]);
    let out = knf(&dir)
        .args([
            "a.json",
            "--exec",
            "--",
            "sh",
            "-c",
            r#"echo "$KNF_CONFIG_PATH"; kill -TERM $$"#,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(143));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!std::path::Path::new(stdout.trim()).exists());
}

#[cfg(unix)]
#[test]
fn exec_snapshot_follows_the_to_format() {
    let dir = tree(&[("a.json", r#"{"a":1}"#)]);
    let out = knf(&dir)
        .args([
            "a.json",
            "--to",
            "toml",
            "--exec",
            "--",
            "sh",
            "-c",
            r#"echo "$KNF_CONFIG_PATH"; cat "$KNF_CONFIG_PATH""#,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let (path, body) = stdout.split_once('\n').unwrap();
    assert_eq!(std::path::Path::new(path).extension().unwrap(), "toml");
    assert_eq!(body, "a = 1\n");
}

#[cfg(unix)]
#[test]
fn exec_snapshot_of_toml_input_is_toml_and_keeps_datetimes() {
    let dir = tree(&[("a.toml", "when = 1979-05-27T07:32:00Z\n")]);
    let out = knf(&dir)
        .args([
            "a.toml",
            "--exec",
            "--",
            "sh",
            "-c",
            r#"echo "$KNF_CONFIG_PATH"; cat "$KNF_CONFIG_PATH""#,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let (path, body) = stdout.split_once('\n').unwrap();
    assert_eq!(std::path::Path::new(path).extension().unwrap(), "toml");
    assert!(body.contains("when = 1979-05-27T07:32:00Z"), "{body}");
    assert!(!std::path::Path::new(path).exists());
}

#[cfg(unix)]
#[test]
fn exec_inherits_streams_and_cleans_snapshot_after_nonzero_exit() {
    let dir = tree(&[("base.json", r#"{"seed":1}"#)]);
    let output = knf(&dir)
        .args([
            "base.json",
            "-c",
            "seed=42",
            "--compact",
            "--exec",
            "--",
            "sh",
            "-c",
            r#"read -r input
               printf '%s\n' "$input" "$@" "$KNF_CONFIG_PATH"
               cat "$KNF_CONFIG_PATH" >&2
               exit 17"#,
            "sh",
            "--replicate",
            "two words",
            "--",
        ])
        .write_stdin("child stdin\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(17));
    assert_eq!(output.stderr, b"{\"seed\":42}\n");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(
        &lines[..4],
        &["child stdin", "--replicate", "two words", "--"]
    );
    let snapshot = std::path::Path::new(lines[4]);
    assert!(snapshot.is_absolute());
    assert_eq!(snapshot.extension().unwrap(), "json");
    assert!(!snapshot.exists());
}

#[cfg(unix)]
#[test]
fn exec_survives_sigint_and_cleans_snapshot() {
    let dir = tree(&[("base.json", r#"{"seed":1}"#)]);
    // SIGINT only the parent, as `kill -INT <knf>` would; the child keeps running.
    let output = knf(&dir)
        .args([
            "base.json",
            "--exec",
            "--",
            "sh",
            "-c",
            r#"echo "$KNF_CONFIG_PATH"; kill -INT "$PPID"; sleep 1"#,
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!std::path::Path::new(stdout.trim()).exists());
}

#[cfg(unix)]
#[test]
fn exec_does_not_launch_after_merge_error() {
    let dir = tree(&[("base.json", r#"{"value":"${missing}"}"#)]);
    knf(&dir)
        .args([
            "base.json",
            "-i",
            "--exec",
            "--",
            "sh",
            "-c",
            "touch launched",
        ])
        .assert()
        .code(1);
    assert!(!dir.path().join("launched").exists());
}

#[test]
fn a_reference_layer_inherits_and_a_later_layer_overrides_in_both_formats() {
    let dir = tree(&[
        (
            "base.json",
            r#"{"defaults":{"port":1,"host":"h"},"staging":"${defaults}","prod":"${staging}"}"#,
        ),
        (
            "over.json",
            r#"{"staging":{"port":2},"read":"${staging.port}"}"#,
        ),
        (
            "base.toml",
            "staging='${defaults}'\nprod='${staging}'\n[defaults]\nport=1\nhost='h'\n",
        ),
        ("over.toml", "read='${staging.port}'\n[staging]\nport=2\n"),
    ]);
    let inherited = r#"{"defaults":{"port":1,"host":"h"},"staging":{"port":2,"host":"h"},"prod":{"port":2,"host":"h"},"read":2}"#;
    let shallow =
        r#"{"defaults":{"port":1,"host":"h"},"staging":{"port":2},"prod":{"port":2},"read":2}"#;
    for (base, over) in [("base.json", "over.json"), ("base.toml", "over.toml")] {
        for (extra, expected) in [
            (&[][..], inherited),
            (&["--shallow", "staging"][..], shallow),
        ] {
            let mut args = vec![base, over, "-i"];
            args.extend(extra);
            let text = run(&dir, &args);
            if base.ends_with("toml") {
                assert_toml_eq_json(&text, expected);
            } else {
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                    serde_json::from_str::<serde_json::Value>(expected).unwrap()
                );
            }
        }
    }
}

#[test]
fn context_supplies_values_without_merging_and_sees_final_overrides() {
    let dir = tree(&[
        (
            "foo.toml",
            "host = 'base'\nurl = '${service.url}'\nport = '${service.port}'\n",
        ),
        ("bar.toml", "host = 'prod'\n"),
        (
            "config.toml",
            "host = 'shared'\nunused = '${missing}'\n[service]\nurl = 'https://${host}/api'\nport = 5432\n",
        ),
    ]);
    let args = ["foo.toml", "bar.toml", "-i", "--with", "config.toml"];
    assert_toml_eq_json(
        &run(&dir, &args),
        r#"{"host":"prod","url":"https://prod/api","port":5432}"#,
    );
    let mut args = args.to_vec();
    args.extend(["-c", "host=inline"]);
    assert_toml_eq_json(
        &run(&dir, &args),
        r#"{"host":"inline","url":"https://inline/api","port":5432}"#,
    );
}

#[test]
fn context_is_independent_of_filtering_and_merge_options() {
    let dir = tree(&[
        ("foo/base.json", r#"{"db":{"host":"prod","port":1}}"#),
        (
            "foo/bar/target.json",
            r#"{"db":{"host":"prod"},"port":"${db.port}"}"#,
        ),
        ("config.json", r#"{"db":{"host":false,"port":5432}}"#),
    ]);
    assert_eq!(
        run(
            &dir,
            &[
                "foo/base.json",
                "foo/bar/target.json",
                "-G",
                "{base,target}.json",
                "--shallow",
                "db",
                "-i",
                "--with",
                "config.json",
                "--compact"
            ]
        ),
        "{\"db\":{\"host\":\"prod\"},\"port\":5432}\n"
    );
}

#[test]
fn context_selects_format_for_inline_or_empty_output_and_respects_format_override() {
    let dir = tree(&[
        ("config.toml", "port = 5432\n"),
        ("dated.toml", "day = 1979-05-27\n"),
    ]);
    assert_toml_eq_json(
        &run(&dir, &["-i", "--with", "config.toml", "-c", "copy=${port}"]),
        r#"{"copy":5432}"#,
    );
    assert_toml_eq_json(&run(&dir, &["-i", "--with", "config.toml"]), "{}");
    let out = run(&dir, &["-i", "--with", "dated.toml", "-c", "copy=${day}"]);
    let out: toml::Value = toml::from_str(&out).unwrap();
    assert!(out["copy"].is_datetime());
    assert!(!out.as_table().unwrap().contains_key("day"));
}

#[test]
fn context_list_files_skips_context_io_and_format_checks() {
    let dir = tree(&[]);
    assert_eq!(
        run(
            &dir,
            &["a.json", "-i", "--with", "missing.toml", "--list-files"]
        ),
        "a.json\n"
    );
    assert_eq!(
        run(&dir, &["-i", "--with", "missing.toml", "--list-files"]),
        ""
    );
}

#[test]
fn context_argument_errors_are_usage_errors_before_io() {
    let dir = tree(&[]);
    for args in [
        vec!["missing.json", "--with", "context.json"],
        vec!["-i", "--with"],
        vec!["-i", "--with", "a.json", "--with", "b.json"],
    ] {
        let out = knf(&dir).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&out.stderr).contains("reading"));
    }
}

#[test]
fn context_formats_are_validated_before_document_reads() {
    let dir = tree(&[]);
    let err = run_err(&dir, &["missing.json", "-i", "--with", "missing.toml"]);
    assert!(err.contains("inputs mix JSON and TOML"));
    assert!(err.contains("inputs and the --with file must share one format"));
    assert!(!err.contains("merge JSON layers and TOML layers separately"));
    assert!(!err.contains("reading"));
}

#[test]
fn context_files_are_parsed_and_validated_even_when_unused() {
    let dir = tree(&[
        ("a.json", "{}"),
        ("bad.json", "{"),
        ("array.json", "[]"),
        ("unresolved.json", r#"{"unused":"${missing}"}"#),
    ]);
    for (file, expected) in [
        ("bad.json", "invalid JSON"),
        ("array.json", "expected an object"),
        ("missing.json", "reading"),
    ] {
        let err = run_err(&dir, &["a.json", "-i", "--with", file]);
        assert!(err.contains(file), "{err}");
        assert!(err.contains(expected), "{err}");
    }
    std::fs::create_dir(dir.path().join("directory.json")).unwrap();
    let err = run_err(&dir, &["a.json", "-i", "--with", "directory.json"]);
    assert!(err.contains("is a directory"), "{err}");
    assert!(err.contains("--with takes one file"), "{err}");
    assert!(!err.contains("merges its files as layers"), "{err}");
    assert_eq!(
        run(
            &dir,
            &["a.json", "-i", "--with", "unresolved.json", "--compact"]
        ),
        "{}\n"
    );
}

#[test]
fn context_interpolation_diagnostics_name_context_paths_and_frontend_help() {
    let dir = tree(&[
        ("a.json", r#"{"copy":"${settings.bad}"}"#),
        ("config.json", r#"{"settings":{"bad":"${missing}"}}"#),
    ]);
    insta::assert_snapshot!(
        "context_unresolved",
        run_err(&dir, &["a.json", "-i", "--with", "config.json"])
    );
}

#[test]
fn context_environment_uses_explicit_test_values() {
    let dir = tree(&[
        ("a.json", r#"{"copy":"${value}"}"#),
        ("config.json", r#"{"value":"${env:KNF_CONTEXT_VALUE}"}"#),
    ]);
    assert_eq!(
        ok_stdout(
            with_env(
                knf(&dir).args(["a.json", "-i", "--with", "config.json", "--compact"]),
                &[("KNF_CONTEXT_VALUE", Some("8080"))]
            ),
            &[]
        ),
        "{\"copy\":8080}\n"
    );
}

// --- glob filters --------------------------------------------------------

#[test]
fn glob_modes_match_paths_or_filenames_and_preserve_order_and_duplicates() {
    let dir = tree(&[
        ("config/a.prod.toml", "value = 1\n"),
        ("config/b.prod.toml", "value = 2\n"),
        ("config/c.dev.toml", "invalid ignored TOML"),
    ]);
    for flag in ["-G", "--glob-filename"] {
        assert_eq!(
            run(
                &dir,
                &[
                    "config/b.prod.toml",
                    "config/c.dev.toml",
                    "config/a.prod.toml",
                    "config/b.prod.toml",
                    flag,
                    "*.prod.toml",
                    "--list-files"
                ]
            ),
            "config/b.prod.toml\nconfig/a.prod.toml\nconfig/b.prod.toml\n"
        );
        assert_eq!(
            run(
                &dir,
                &[
                    "config/a.prod.toml",
                    "config/c.dev.toml",
                    "config/b.prod.toml",
                    flag,
                    "*.prod.toml"
                ]
            ),
            run(&dir, &["config/a.prod.toml", "config/b.prod.toml"])
        );
    }
    for flag in ["-g", "--glob"] {
        assert_eq!(
            run(
                &dir,
                &["config/a.prod.toml", flag, "*.prod.toml", "--list-files"]
            ),
            ""
        );
        assert_eq!(
            run(
                &dir,
                &[
                    "config/a.prod.toml",
                    flag,
                    "config/**/*.prod.toml",
                    "--list-files"
                ]
            ),
            "config/a.prod.toml\n"
        );
        assert_eq!(
            run(
                &dir,
                &[
                    "./config/a.prod.toml",
                    flag,
                    "config/*.prod.toml",
                    "--list-files"
                ]
            ),
            ""
        );
        assert_eq!(
            run(
                &dir,
                &[
                    "./config/a.prod.toml",
                    flag,
                    "./config/*.prod.toml",
                    "--list-files"
                ]
            ),
            "./config/a.prod.toml\n"
        );
    }
}

#[test]
fn glob_supports_case_sensitive_names_braces_classes_and_negation() {
    let dir = tree(&[]);
    let inputs = [
        "foo/defaults.toml",
        "foo/prod.toml",
        "foo/Prod.toml",
        "foo/dev.toml",
    ];
    for (pattern, expected) in [
        ("{defaults,prod}.toml", "foo/defaults.toml\nfoo/prod.toml\n"),
        ("[pd]??.toml", "foo/dev.toml\n"),
        ("!*.toml", ""),
        ("!{defaults,prod}.toml", "foo/Prod.toml\nfoo/dev.toml\n"),
        ("prod.toml", "foo/prod.toml\n"),
        (".prod.toml", ""),
    ] {
        let mut args = inputs.to_vec();
        args.extend(["-G", pattern, "--list-files"]);
        assert_eq!(run(&dir, &args), expected, "{pattern}");
    }
    // Even a negated pattern cannot match an input without a filename.
    assert_eq!(
        run(&dir, &[".", "..", "-G", "!prod.toml", "--list-files"]),
        ""
    );
    assert_eq!(run(&dir, &["é.toml", "-G", "?.toml", "--list-files"]), "");
    assert_eq!(
        run(&dir, &["é.toml", "-G", "??.toml", "--list-files"]),
        "é.toml\n"
    );
}

#[test]
fn glob_excluded_inputs_are_not_loaded_or_used_for_format_inference() {
    let dir = tree(&[("prod.toml", "value = 1\n"), ("bad.json", "invalid JSON")]);
    assert_eq!(
        run(
            &dir,
            &["bad.json", "missing.json", "prod.toml", "-G", "*.toml"]
        ),
        run(&dir, &["prod.toml"])
    );
    assert_eq!(
        run(
            &dir,
            &[
                "bad.json",
                "missing.json",
                "prod.toml",
                "-G",
                "*.toml",
                "-c",
                "value=2"
            ]
        ),
        "value = 2\n"
    );
}

#[test]
fn glob_empty_selections_keep_existing_empty_input_and_set_behaviour() {
    let dir = tree(&[]);
    for flag in ["-g", "-G"] {
        assert_eq!(
            run(
                &dir,
                &["missing.toml", flag, "*.json", "--to", "json", "--compact"]
            ),
            "{}\n"
        );
        assert_eq!(
            run(
                &dir,
                &[
                    "missing.toml",
                    flag,
                    "*.json",
                    "-c",
                    "value=2",
                    "--to",
                    "json",
                    "--compact"
                ]
            ),
            "{\"value\":2}\n"
        );
        assert_eq!(
            run(&dir, &["missing.toml", flag, "*.json", "--list-files"]),
            ""
        );
        assert_eq!(
            run(&dir, &[flag, "*.json", "--to", "json", "--compact"]),
            "{}\n"
        );
    }
}

#[test]
fn glob_usage_errors_precede_filesystem_access() {
    let dir = tree(&[]);
    for args in [
        vec!["-g"],
        vec!["-G"],
        vec!["-g", "*", "-G", "*"],
        vec!["-g", "*", "-g", "*"],
        vec!["-G", "*", "-G", "*"],
        vec!["-a", "missing.toml", "-g", "[broken"],
        vec!["missing.toml", "-G", "{broken"],
        vec!["missing.toml", "-g", "trailing\\"],
    ] {
        let out = knf(&dir).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(!stderr.contains("inspecting"));
        assert!(!stderr.contains("reading"));
    }
    assert!(
        run_err(&dir, &["missing.toml", "-G", "none", "-c", "a[0]=1"])
            .contains("-c takes KEY.PATH=VALUE")
    );
}

#[cfg(windows)]
#[test]
fn glob_normalizes_windows_separators_only_for_matching() {
    let dir = tree(&[]);
    assert_eq!(
        run(
            &dir,
            &[
                r".\config\prod.toml",
                "-g",
                "./config/*.toml",
                "--list-files"
            ]
        ),
        ".\\config\\prod.toml\n"
    );
    assert_eq!(
        run(
            &dir,
            &[r"config\prod.toml", "-G", "prod.toml", "--list-files"]
        ),
        "config\\prod.toml\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn glob_preserves_non_utf8_filenames_and_matches_native_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tree(&[]);
    let path = std::path::Path::new("config")
        .join(std::ffi::OsString::from_vec(b"prod-\xff.json".to_vec()));
    std::fs::create_dir(dir.path().join("config")).unwrap();
    std::fs::write(dir.path().join(&path), r#"{"value":1}"#).unwrap();
    for (flag, pattern) in [("-g", "config/prod-?.json"), ("-G", "prod-?.json")] {
        let out = knf(&dir)
            .arg(&path)
            .args([flag, pattern, "--compact"])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"{\"value\":1}\n");
    }
}

// --- round-trips ----------------------------------------------------------

const DATED: &str = "\
name = \"svc\"
date = 1979-05-27T07:32:00Z
";

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

#[test]
fn toml_datetime_survives_set() {
    let dir = tree(&[("f.toml", DATED)]);
    let out = run(&dir, &["f.toml", "-c", "extra=1"]);
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

#[test]
fn integers_above_i64_max_are_exact() {
    let doc = r#"{"id":10000000000000000001,"max":18446744073709551615}"#;
    let dir = tree(&[("a.json", doc)]);
    assert_eq!(run(&dir, &["a.json", "--compact"]), format!("{doc}\n"));
}

#[test]
fn a_single_layer_is_a_no_op() {
    let doc = r#"{"a":{"b":1},"n":null,"xs":[1,2]}"#;
    let dir = tree(&[("a.json", doc)]);
    assert_eq!(run(&dir, &["a.json", "--compact"]), format!("{doc}\n"));
}

#[test]
fn homogeneous_layers_merge_in_both_formats() {
    let dir = tree(&[
        (
            "base.toml",
            "[server]
port = 80
host = 'local'",
        ),
        (
            "over.toml",
            "[server]
port = 443",
        ),
    ]);
    assert_toml_eq_json(
        &run(&dir, &["base.toml", "over.toml"]),
        r#"{"server":{"port":443,"host":"local"}}"#,
    );
}

#[test]
fn paths_without_a_known_extension_are_rejected_with_a_rename_hint() {
    let dir = tree(&[("app.conf", "{}")]);
    for name in ["app.conf", "-"] {
        let err = run_err(&dir, &[name]);
        assert!(
            err.contains(&format!("cannot infer a format from `{name}`")),
            "{err}"
        );
        assert!(err.contains("help: name the file with a .json or .toml extension"));
    }
}

#[test]
fn inline_configs_apply_last() {
    let dir = tree(&[("a.json", r#"{"server":{"port":80}}"#)]);
    assert_eq!(
        run(&dir, &["a.json", "-c", "server.port=8080", "--compact"]),
        "{\"server\":{\"port\":8080}}\n"
    );
    // With no file inputs, --to selects the format.
    assert_eq!(
        run(&dir, &["-c", "a.b=1", "--to", "json", "--compact"]),
        "{\"a\":{\"b\":1}}\n"
    );
}

// --- deep and shallow merge ----------------------------------------------

const BASE: &str = "plugins = [\"auth\"]\n[db]\nhost = \"local\"\nport = 5432\n";
const PROD: &str = "plugins = [\"metrics\"]\n[db]\nhost = \"prod\"\n";

#[test]
fn by_default_arrays_replace_and_tables_merge() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    let out = run(&dir, &["base.toml", "prod.toml"]);
    assert!(out.contains("plugins = [\"metrics\"]"), "{out}");
    assert!(out.contains("port = 5432"), "{out}");
}

#[test]
fn shallow_takes_top_level_tables_whole() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    let out = run(
        &dir,
        &["base.toml", "prod.toml", "--shallow=*", "--compact"],
    );
    assert_toml_eq_json(
        &out,
        "{\"plugins\":[\"metrics\"],\"db\":{\"host\":\"prod\"}}\n",
    );
}

#[test]
fn shallow_single_layer_is_identity() {
    let dir = tree(&[("base.toml", BASE)]);
    assert_eq!(
        run(&dir, &["base.toml", "--shallow=*"]),
        run(&dir, &["base.toml"])
    );
}

#[test]
fn shallow_applies_to_set_layers_too() {
    let dir = tree(&[("base.toml", BASE)]);
    let out = run(&dir, &["base.toml", "--shallow=*", "-c", "db.host=x"]);
    assert!(out.contains("host = \"x\""), "{out}");
    assert!(!out.contains("port"), "-c layer did not replace db:\n{out}");
}

const NESTED_BASE: &str = "\
[db]
host = \"local\"
[db.pool]
min = 1
max = 5
[app.pool]
min = 1
max = 5
";
const NESTED_PROD: &str = "[db.pool]\nmax = 9\n[app.pool]\nmax = 9\n";

#[test]
fn shallow_at_a_path_keeps_everything_else_deep() {
    let dir = tree(&[("base.toml", NESTED_BASE), ("prod.toml", NESTED_PROD)]);
    let out = run(
        &dir,
        &["base.toml", "prod.toml", "--shallow=db.*", "--compact"],
    );
    assert_toml_eq_json(
        &out,
        "{\"db\":{\"host\":\"local\",\"pool\":{\"max\":9}},\"app\":{\"pool\":{\"min\":1,\"max\":9}}}\n",
    );
}

#[test]
fn shallow_braces_select_multiple_objects() {
    let dir = tree(&[("base.toml", NESTED_BASE), ("prod.toml", NESTED_PROD)]);
    let out = run(
        &dir,
        &[
            "base.toml",
            "prod.toml",
            "--shallow={db,app}.*",
            "--compact",
        ],
    );
    assert_toml_eq_json(
        &out,
        "{\"db\":{\"host\":\"local\",\"pool\":{\"max\":9}},\"app\":{\"pool\":{\"max\":9}}}\n",
    );
}

#[test]
fn shallow_exact_object_selector_drops_omitted_children() {
    let dir = tree(&[("base.toml", NESTED_BASE), ("prod.toml", NESTED_PROD)]);
    let out = run(&dir, &["base.toml", "prod.toml", "--shallow", "db"]);
    assert_toml_eq_json(
        &out,
        r#"{"db":{"pool":{"max":9}},"app":{"pool":{"min":1,"max":9}}}"#,
    );
}

#[test]
fn shallow_quoted_keys_and_nested_paths_are_distinct_in_both_formats() {
    let base = serde_json::json!({
        "foo.bar": {"old": 1},
        "foo/bar": {"old": 1},
        "foo\\bar": {"old": 1},
        "servers[0]": {"old": 1},
        "*": {"old": 1},
        "": {"old": 1},
        "foo": {"bar": {"old": 1}},
    });
    let over = serde_json::json!({
        "foo.bar": {"new": 2},
        "foo/bar": {"new": 2},
        "foo\\bar": {"new": 2},
        "servers[0]": {"new": 2},
        "*": {"new": 2},
        "": {"new": 2},
        "foo": {"bar": {"new": 2}},
    });
    let expected = serde_json::json!({
        "foo.bar": {"new": 2},
        "foo/bar": {"new": 2},
        "foo\\bar": {"new": 2},
        "servers[0]": {"new": 2},
        "*": {"new": 2},
        "": {"new": 2},
        "foo": {"bar": {"old": 1, "new": 2}},
    });
    let dir = tree(&[
        ("base.json", &base.to_string()),
        ("over.json", &over.to_string()),
        (
            "base.toml",
            &toml::to_string(&toml::Value::try_from(&base).unwrap()).unwrap(),
        ),
        (
            "over.toml",
            &toml::to_string(&toml::Value::try_from(&over).unwrap()).unwrap(),
        ),
    ]);
    let pattern = r"{'foo.bar','foo/bar','foo\\bar','servers[0]','*',''}";
    for (base_file, over_file) in [("base.json", "over.json"), ("base.toml", "over.toml")] {
        let out = run(&dir, &[base_file, over_file, "--shallow", pattern]);
        if base_file.ends_with("json") {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&out).unwrap(),
                expected
            );
        } else {
            assert_toml_eq_json(&out, &expected.to_string());
        }
    }
}

#[test]
fn shallow_glob_set_layers_and_interpolation_compose() {
    let dir = tree(&[("base.toml", NESTED_BASE)]);
    let out = run(
        &dir,
        &[
            "base.toml",
            "--shallow",
            "db.*",
            "-c",
            "db.pool={max=9}",
            "-c",
            "copy=${db.pool}",
            "--interpolate",
        ],
    );
    assert_toml_eq_json(
        &out,
        r#"{"db":{"host":"local","pool":{"max":9}},"app":{"pool":{"min":1,"max":5}},"copy":{"max":9}}"#,
    );
}

#[test]
fn shallow_accepts_a_separate_pattern_before_files() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    assert_eq!(
        run(&dir, &["--shallow", "*", "base.toml", "prod.toml"]),
        run(&dir, &["base.toml", "prod.toml", "--shallow=*"])
    );
}

#[test]
fn shallow_requires_one_nonempty_valid_pattern_before_file_io() {
    let dir = tree(&[]);
    for args in [
        vec!["missing.toml", "--shallow"],
        vec!["missing.toml", "--shallow="],
        vec!["missing.toml", "--shallow=a", "--shallow=b"],
        vec!["missing.toml", "--shallow='unclosed"],
        vec!["missing.toml", "--shallow={a,b"],
    ] {
        let output = knf(&dir).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let err = String::from_utf8(output.stderr).unwrap();
        assert!(!err.contains("No such file"), "{err}");
    }
}

/// A string fallback can be overwritten by a later typed inline layer.
#[test]
fn set_null_overwritten_before_toml_emit() {
    let dir = tree(&[("f.toml", "a = 0\n")]);
    assert_eq!(
        run(&dir, &["f.toml", "-c", "a=null", "-c", "a=1"]),
        "a = 1\n"
    );
}

#[test]
fn toml_inline_typing_uses_native_values_and_string_fallback() {
    let dir = tree(&[]);
    let out = run(
        &dir,
        &[
            "--to",
            "toml",
            "-c",
            "proxy=null",
            "-c",
            "day=1979-05-27",
            "-c",
            "db={host='local'}",
            "-c",
            "large=18446744073709551615",
            "-c",
            "bad=[a,b]",
            "-c",
            "limit=inf",
        ],
    );
    let value: toml::Value = toml::from_str(&out).unwrap();
    assert_eq!(value["proxy"].as_str(), Some("null"));
    assert_eq!(value["day"].type_str(), "datetime");
    assert_eq!(value["db"]["host"].as_str(), Some("local"));
    assert_eq!(value["large"].as_str(), Some("18446744073709551615"));
    assert_eq!(value["bad"].as_str(), Some("[a,b]"));
    assert!(value["limit"].as_float().unwrap().is_infinite());
}

#[test]
fn toml_inline_literals_accept_surrounding_whitespace() {
    let dir = tree(&[]);
    let out = run(
        &dir,
        &[
            "--to",
            "toml",
            "-c",
            "port= \t8080\r\n",
            "-c",
            "enabled=true\n",
            "-c",
            "ratio=1.0 ",
            "-c",
            "fallback= text\n",
            "-c",
            "quoted= ' text ' ",
        ],
    );
    let value: toml::Value = toml::from_str(&out).unwrap();
    assert_eq!(value["port"].as_integer(), Some(8080));
    assert_eq!(value["enabled"].as_bool(), Some(true));
    assert_eq!(value["ratio"].as_float(), Some(1.0));
    assert_eq!(value["fallback"].as_str(), Some(" text\n"));
    assert_eq!(value["quoted"].as_str(), Some(" text "));
}

// --- native inline values -------------------------------------------------

#[test]
fn removed_options_are_usage_errors() {
    let dir = tree(&[]);
    for args in [
        ["--input-format", "json"],
        ["--null-as", "none"],
        ["--accumulate", "a.toml"],
    ] {
        let out = knf(&dir).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(
            String::from_utf8(out.stderr)
                .unwrap()
                .contains("unexpected argument")
        );
    }
}

#[test]
fn toml_environment_values_share_inline_typing_and_remain_terminal() {
    let dir = tree(&[(
        "a.toml",
        "day = '${env:KNF_TEST_DAY}'
null = '${env:KNF_TEST_NULL}'
raw = 'x:${env:KNF_TEST_DAY}'
terminal = '${env:KNF_TEST_TERMINAL}'
",
    )]);
    let out = with_env(
        knf(&dir).args(["a.toml", "--interpolate"]),
        &[
            ("KNF_TEST_DAY", Some("1979-05-27")),
            ("KNF_TEST_NULL", Some("null")),
            ("KNF_TEST_TERMINAL", Some("${missing}")),
        ],
    )
    .output()
    .unwrap();
    assert!(out.status.success());
    let value: toml::Value = toml::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(value["day"].type_str(), "datetime");
    assert_eq!(value["null"].as_str(), Some("null"));
    assert_eq!(value["raw"].as_str(), Some("x:1979-05-27"));
    assert_eq!(value["terminal"].as_str(), Some("${missing}"));
}

#[test]
fn runs_without_input_files_need_to_for_a_format() {
    let dir = tree(&[]);
    for args in [&[][..], &["-c", "a=1"][..]] {
        let err = run_err(&dir, args);
        assert!(
            err.contains("no input files to infer a format from"),
            "{err}"
        );
        assert!(err.contains("help: pass --to json or --to toml"), "{err}");
    }
    assert_eq!(run(&dir, &["--to", "json", "--compact"]), "{}\n");
    assert_eq!(run(&dir, &["--to", "toml"]), "\n");
    assert_eq!(run(&dir, &["--to", "toml", "-c", "a=1"]), "a = 1\n");
    assert_eq!(
        run(
            &dir,
            &["missing.json", "-g", "*.toml", "--to", "toml", "-c", "a=1"]
        ),
        "a = 1\n"
    );
}

// --- --interpolate --------------------------------------------------------

/// A document made only of references.
const REFS: &str = "\
root = \"/srv\"
data_dir = \"${root}/data\"
port = \"${env:KNF_TEST_PORT}\"
url = \"http://localhost:${env:KNF_TEST_PORT}/health\"
literal = \"$${NOT_A_REF}\"
";

#[test]
fn references_pass_through_untouched_without_the_flag() {
    let dir = tree(&[("f.toml", REFS)]);
    let out = ok_stdout(
        with_env(&mut knf(&dir), &[("KNF_TEST_PORT", Some("8080"))]).args(["f.toml"]),
        &["f.toml"],
    );
    assert_eq!(out, REFS);
}

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

#[test]
fn nested_references_select_a_subtree_from_the_environment() {
    let dir = tree(&[(
        "f.toml",
        "db = \"${databases.${env:KNF_TEST_STAGE}}\"\n\
         [databases.dev]\nhost = \"localhost\"\n\
         [databases.prod]\nhost = \"db.example\"\n",
    )]);
    let args = ["f.toml", "--interpolate"];
    let out = ok_stdout(
        with_env(&mut knf(&dir), &[("KNF_TEST_STAGE", Some("prod"))]).args(args),
        &args,
    );
    assert!(out.contains("[db]\nhost = \"db.example\""), "{out}");
}

#[test]
fn an_unresolved_nested_reference_names_the_looked_up_path() {
    let dir = tree(&[("a.json", r#"{"stage":"qa","db":"${databases.${stage}}"}"#)]);
    insta::assert_snapshot!("nested_unresolved", run_err(&dir, &["a.json", "-i"]));
}

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

#[test]
fn set_layers_interpolate_too() {
    let dir = tree(&[("f.json", r#"{"root":"/srv"}"#)]);
    assert_eq!(
        run(
            &dir,
            &[
                "f.json",
                "-c",
                "data=${root}/data",
                "--interpolate",
                "--compact"
            ]
        ),
        "{\"root\":\"/srv\",\"data\":\"/srv/data\"}\n"
    );
}

#[test]
fn a_referenced_table_merges_with_a_later_table() {
    let dir = tree(&[
        (
            "base.toml",
            "[bar]\na = 1\nd = 1\n\n[foo]\nb = \"${bar}\"\n",
        ),
        ("over.toml", "[foo.b]\na = 4\n"),
        ("late.toml", "[bar]\nd = 9\n"),
    ]);
    let merged = |extra: &[&str]| {
        let mut args = vec!["base.toml", "over.toml"];
        args.extend_from_slice(extra);
        run(&dir, &args)
    };
    let table = |d: &str| format!("[bar]\na = 1\nd = {d}\n\n[foo.b]\na = 4\n");
    assert_eq!(merged(&["-i"]), format!("{}d = 1\n", table("1")));
    assert_eq!(
        merged(&["late.toml", "-i"]),
        format!("{}d = 9\n", table("9"))
    );
    assert_eq!(merged(&["-i", "--shallow", "foo.b"]), table("1"));
}

#[test]
fn a_null_referent_remains_native_json_null() {
    let dir = tree(&[("f.json", r#"{"n":null,"copy":"${n}"}"#)]);
    assert_eq!(
        run(&dir, &["f.json", "--interpolate", "--compact"]),
        "{\"n\":null,\"copy\":null}
"
    );
}

// --- --interpolate errors (snapshotted) -----------------------------------

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

// clap usage errors exit 2; runtime errors exit 1.
#[test]
fn usage_errors_exit_two() {
    let dir = tree(&[]);
    for args in [
        &["--no-such-flag"][..],
        &["--cascade", "foo/bar/config.toml"][..],
        &["-r", "foo/bar/config.toml"][..],
    ] {
        let out = knf(&dir).args(args).output().expect("spawn");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn malformed_set_exits_two() {
    let dir = tree(&[]);
    let out = knf(&dir).args(["-c", "noequals"]).output().expect("spawn");
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

#[test]
fn a_non_object_root_is_rejected_by_name() {
    let dir = tree(&[("a.json", "[1,2]")]);
    let err = run_err(&dir, &["a.json"]);
    assert!(err.contains("a.json"), "{err}");
    assert!(err.contains("found array"), "{err}");
}

// --- multi-line error messages (snapshotted) ------------------------------

#[test]
fn directory_in_the_default_command_error() {
    let dir = tree(&[("config/base.toml", "a = 1\n")]);
    insta::assert_snapshot!(run_err(&dir, &["config"]));
}

#[test]
fn directory_merge_input_keeps_layer_hint_with_context() {
    let dir = tree(&[("config/base.toml", "a = 1\n"), ("context.toml", "b = 1\n")]);
    let err = run_err(&dir, &["config", "-i", "--with", "context.toml"]);
    assert!(
        err.contains("`knf config/*.toml` merges its files as layers"),
        "{err}"
    );
    assert!(!err.contains("--with takes one file"), "{err}");
}

#[test]
fn context_directory_error() {
    let dir = tree(&[("a.json", "{}"), ("config/base.toml", "a = 1\n")]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "-i", "--with", "config"]));
}

#[test]
fn mixed_input_formats_error() {
    let dir = tree(&[("a.toml", "a = 1\n"), ("b.json", "{}")]);
    insta::assert_snapshot!(run_err(&dir, &["a.toml", "b.json"]));
}

#[test]
fn context_mixed_input_formats_error() {
    let dir = tree(&[("a.toml", "a = 1\n"), ("b.json", "{}")]);
    insta::assert_snapshot!(run_err(&dir, &["a.toml", "-i", "--with", "b.json"]));
}

#[test]
fn mixed_input_formats_error_precedes_interpolation_errors() {
    let dir = tree(&[("a.json", r#"{"a":"${nope}"}"#), ("b.toml", "b = 1\n")]);
    insta::assert_snapshot!(run_err(&dir, &["a.json", "b.toml", "--interpolate"]));
}

#[test]
fn set_with_an_array_index_errors_before_file_io() {
    let dir = tree(&[]);
    let err = run_err(&dir, &["missing.toml", "-c", "servers[0]=1"]);
    assert!(
        !err.contains("missing.toml"),
        "the -c path should be rejected before the file is read:\n{err}"
    );
    insta::assert_snapshot!(err);
}

#[test]
fn shallow_invalid_glob_errors_before_file_io() {
    let dir = tree(&[]);
    let output = knf(&dir)
        .args(["missing.toml", "--shallow=servers[0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(!err.contains("No such file"), "{err}");
    insta::assert_snapshot!(err);
}

#[test]
fn convert_after_merging_and_interpolation_in_the_input_format() {
    let dir = tree(&[
        (
            "base.toml",
            "[db]\nports=[80, 81]\nstarted=2024-01-01\ncopy='${db.started}'\n",
        ),
        ("prod.toml", "[db]\nports=[443]\n"),
        (
            "base.json",
            r#"{"db":{"ports":[80,81],"id":null,"copy":"${db.id}"}}"#,
        ),
        ("prod.json", r#"{"db":{"ports":[443],"id":42}}"#),
    ]);
    assert_eq!(
        run(
            &dir,
            &[
                "base.toml",
                "prod.toml",
                "-i",
                "-c",
                "db.started=2024-01-02",
                "--to",
                "json",
                "--compact",
            ],
        ),
        "{\"db\":{\"ports\":[443],\"started\":\"2024-01-02\",\"copy\":\"2024-01-02\"}}\n"
    );
    let text = run(
        &dir,
        &["base.json", "prod.json", "-i", "--to", "toml", "--strict"],
    );
    assert_toml_eq_json(&text, r#"{"db":{"ports":[443],"id":42,"copy":42}}"#);

    let args = ["base.json", "--compact"];
    assert_eq!(
        run(&dir, &args),
        run(
            &dir,
            &["base.json", "--compact", "--to", "json", "--strict"],
        )
    );
}

#[test]
fn lossy_conversion_warns_on_stderr_and_strict_names_the_key() {
    let dir = tree(&[
        ("lossy.toml", "[nested]\ndate=1979-05-27\n"),
        ("lossy.json", r#"{"nested":{"drop":null}}"#),
        ("clean.json", r#"{"nested":{"a":1}}"#),
    ]);
    for (file, to, key) in [
        ("lossy.toml", "json", "nested.date:"),
        ("lossy.json", "toml", "nested.drop:"),
    ] {
        let out = knf(&dir).args([file, "--to", to]).output().unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stderr).unwrap(),
            "warning: conversion was lossy; rerun with --strict to find the affected keys\n"
        );
        assert!(run_err(&dir, &[file, "--to", to, "--strict"]).contains(key));
    }
    let out = knf(&dir)
        .args(["clean.json", "--to", "toml", "--strict"])
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty());
}

#[test]
fn null_array_entries_fail_conversion_without_strict() {
    let dir = tree(&[("null_array.json", r#"{"nested":[1,null,2]}"#)]);
    let err = run_err(&dir, &["null_array.json", "--to", "toml"]);
    assert!(err.contains("nested[1]:"));
}

#[test]
fn strict_requires_an_output_format_before_reading_inputs() {
    let dir = tree(&[]);
    let out = knf(&dir)
        .args(["missing.json", "--strict"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("required arguments were not provided") && stderr.contains("--to"),
        "{stderr}"
    );
}

#[test]
fn same_format_strict_is_a_no_op() {
    let dir = tree(&[("a.json", r#"{"n":null,"big":18446744073709551615}"#)]);
    let args = ["a.json", "--compact"];
    let plain = run(&dir, &args);
    let out = knf(&dir)
        .args(["a.json", "--compact", "--to", "json", "--strict"])
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), plain);
}
