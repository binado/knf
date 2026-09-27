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

// --- accumulate ----------------------------------------------------------

#[test]
fn accumulate_matches_explicit_layers_and_lists_in_order() {
    // Create files out of order. The target sorts first but must apply last.
    let dir = tree(&[
        ("foo/z.toml", "value = 2\n"),
        ("foo/bar/z.toml", "value = 3\n"),
        ("foo/a.toml", "value = 1\n"),
        ("foo/.hidden.TOML", "hidden = true\n"),
        ("foo/bar/00-target.toml", "value = 4\n"),
        ("root.toml", "invalid ignored root"),
        ("foo/ignored.json", "invalid ignored format"),
        ("foo/other/ignored.toml", "invalid ignored branch"),
        (
            "foo/directory.toml/ignored.toml",
            "invalid ignored directory",
        ),
    ]);
    let files = [
        "foo/.hidden.TOML",
        "foo/a.toml",
        "foo/z.toml",
        "foo/bar/z.toml",
        "foo/bar/00-target.toml",
    ];
    assert_eq!(
        run(&dir, &["-a", "./foo/./bar/00-target.toml"]),
        run(&dir, &files)
    );
    let expected = files
        .iter()
        .map(|path| std::path::Path::new(path).display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(
        run(
            &dir,
            &["--accumulate", "foo/bar/00-target.toml", "--list-files"]
        ),
        expected
    );
    assert!(run(&dir, &["-a", "foo/bar/00-target.toml"]).contains("value = 4"));
}

#[test]
fn accumulate_json_keeps_a_non_associative_flat_fold() {
    // Grouping the last two layers would retain "old" from the first layer.
    let dir = tree(&[
        ("foo/a.json", r#"{"branch":{"old":1}}"#),
        ("foo/middle/b.JSON", r#"{"branch":false}"#),
        ("foo/middle/deep/target.json", r#"{"branch":{"new":2}}"#),
        ("foo/ignored.toml", "invalid ignored format"),
    ]);
    assert_eq!(
        run(&dir, &["-a", "foo/middle/deep/target.json", "--compact"]),
        "{\"branch\":{\"new\":2}}\n"
    );
}

#[test]
fn accumulate_empty_intermediate_directories_and_cwd_target() {
    let dir = tree(&[
        ("foo/empty/deep/target.toml", "value = 1\n"),
        ("target.toml", "value = 2\n"),
        ("other.toml", "invalid ignored root"),
    ]);
    for target in ["foo/empty/deep/target.toml", "target.toml"] {
        assert_eq!(run(&dir, &["-a", target]), run(&dir, &[target]));
    }
    assert_eq!(
        run(&dir, &["-a", "./target.toml", "--list-files"]),
        "target.toml\n"
    );
}

#[test]
fn accumulate_uses_the_existing_merge_and_interpolation_pipeline() {
    let dir = tree(&[
        ("foo/base.toml", "[db]\nhost = \"base\"\nport = 80\n"),
        (
            "foo/bar/target.toml",
            "url = \"${db.host}\"\n[db]\nhost = \"target\"\n",
        ),
    ]);
    for flags in [
        vec!["--strict"],
        vec!["--shallow"],
        vec!["--shallow=db"],
        vec!["--set", "db.port=8080"],
        vec!["--interpolate", "--compact"],
    ] {
        let mut accumulate = vec!["-a", "foo/bar/target.toml"];
        accumulate.extend(&flags);
        let mut explicit = vec!["foo/base.toml", "foo/bar/target.toml"];
        explicit.extend(&flags);
        assert_eq!(run(&dir, &accumulate), run(&dir, &explicit));
    }
    assert_toml_eq_json(
        &run(
            &dir,
            &["-a", "foo/bar/target.toml", "--interpolate", "--compact"],
        ),
        "{\"db\":{\"host\":\"target\",\"port\":80},\"url\":\"target\"}\n",
    );
    let dir = tree(&[
        ("foo/base.json", r#"{"branch":{}}"#),
        ("foo/target.json", r#"{"branch":false}"#),
    ]);
    assert_eq!(
        run_err(&dir, &["-a", "foo/target.json", "--strict"]),
        run_err(&dir, &["foo/base.json", "foo/target.json", "--strict"])
    );
}

#[test]
fn accumulate_input_format_changes_parsing_only() {
    let dir = tree(&[
        ("foo/base.toml", r#"{"base":1}"#),
        ("foo/bar/target.TOML", r#"{"target":2}"#),
        ("foo/ignored.json", "invalid ignored format"),
    ]);
    assert_eq!(
        run(
            &dir,
            &["-a", "foo/bar/target.TOML", "-f", "json", "--compact"]
        ),
        "{\"base\":1,\"target\":2}\n"
    );
}

#[test]
fn accumulate_listing_does_not_parse_or_resolve_files() {
    let dir = tree(&[
        ("foo/base.toml", "invalid TOML"),
        ("foo/target.toml", "value = \"${missing}\"\n"),
    ]);
    let expected = format!(
        "{}\n{}\n",
        std::path::Path::new("foo/base.toml").display(),
        std::path::Path::new("foo/target.toml").display()
    );
    assert_eq!(
        run(
            &dir,
            &[
                "-a",
                "foo/target.toml",
                "--list-files",
                "--interpolate",
                "--strict",
                "-f",
                "json"
            ]
        ),
        expected
    );
    assert!(run_err(&dir, &["-a", "foo/target.toml"]).contains("foo/base.toml: invalid TOML"));
}

#[test]
fn list_files_prints_positional_paths_without_reading_them() {
    let dir = tree(&[("foo.toml", "invalid TOML")]);
    assert_eq!(
        run(&dir, &["--list-files", "./foo.toml", "missing.toml"]),
        "./foo.toml\nmissing.toml\n"
    );
    assert_eq!(run(&dir, &["--list-files"]), "");
}

#[test]
fn accumulate_usage_is_validated_before_filesystem_access() {
    let dir = tree(&[]);
    let absolute = dir.path().join("abs.toml").display().to_string();
    for (args, expected) in [
        (vec!["-a"], "a value is required"),
        (vec!["-a", "a.toml", "b.toml"], "cannot be used with"),
        (vec!["-a", "-"], "does not accept stdin"),
        (vec!["-a", "foo/../a.toml"], "without .. components"),
        (vec!["-a", absolute.as_str()], "relative target path"),
        (vec!["-a", "a.yaml", "-f", "toml"], "JSON or TOML extension"),
    ] {
        let out = knf(&dir).args(&args).output().expect("spawn");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        let stderr = String::from_utf8(out.stderr).expect("utf-8");
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
    }
    let error = run_err(&dir, &["-a", "missing.toml", "--set", "a[0]=1"]);
    assert!(error.contains("--set takes KEY.PATH=VALUE"));
    assert!(!error.contains("inspecting"));
}

#[test]
fn accumulate_filesystem_errors_do_not_produce_partial_lists() {
    let dir = tree(&[("foo/base.toml", "value = 1\n")]);
    for target in ["foo/missing.toml", "missing.toml"] {
        let error = run_err(&dir, &["-a", target, "--list-files"]);
        assert!(error.contains(target));
    }
    std::fs::create_dir(dir.path().join("foo/directory.toml")).unwrap();
    insta::assert_snapshot!(
        "accumulate_directory_target",
        run_err(&dir, &["-a", "foo/directory.toml"])
    );
}

#[cfg(unix)]
#[test]
fn accumulate_follows_symlinks_without_deduplicating_aliases() {
    use std::os::unix::fs::symlink;
    let dir = tree(&[
        ("outside/base.toml", "base = 1\n"),
        ("outside/target.toml", "target = 2\n"),
    ]);
    symlink("outside", dir.path().join("foo")).unwrap();
    symlink("target.toml", dir.path().join("outside/alias.toml")).unwrap();
    assert_eq!(
        run(&dir, &["-a", "foo/target.toml", "--list-files"]),
        "foo/alias.toml\nfoo/base.toml\nfoo/target.toml\n"
    );
    assert_eq!(
        run(&dir, &["-a", "foo/target.toml"]),
        run(
            &dir,
            &["foo/alias.toml", "foo/base.toml", "foo/target.toml"]
        )
    );
    symlink("missing", dir.path().join("outside/broken.toml")).unwrap();
    let out = knf(&dir)
        .args(["-a", "foo/target.toml", "--list-files"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("foo/broken.toml")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn accumulate_preserves_non_utf8_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let dir = tree(&[("foo/base.json", r#"{"base":1}"#)]);
    let filename = OsString::from_vec(b"target-\xff.json".to_vec());
    let relative = std::path::Path::new("foo").join(filename);
    std::fs::write(dir.path().join(&relative), r#"{"target":2}"#).unwrap();
    let out = knf(&dir)
        .arg("-a")
        .arg(relative)
        .arg("--compact")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"{\"base\":1,\"target\":2}\n");
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
                "--set",
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
            run(&dir, &["missing.toml", flag, "*.json", "--compact"]),
            "{}\n"
        );
        assert_eq!(
            run(
                &dir,
                &[
                    "missing.toml",
                    flag,
                    "*.json",
                    "--set",
                    "value=2",
                    "--compact"
                ]
            ),
            "{\"value\":2}\n"
        );
        assert_eq!(
            run(&dir, &["missing.toml", flag, "*.json", "--list-files"]),
            ""
        );
        assert_eq!(run(&dir, &[flag, "*.json", "--compact"]), "{}\n");
    }
}

#[test]
fn glob_filters_accumulated_targets_without_changing_discovery_errors() {
    let dir = tree(&[
        ("foo/defaults.toml", "value = 1\n"),
        ("foo/bad.toml", "invalid ignored TOML"),
        ("foo/bar/defaults.toml", "value = 2\n"),
        ("foo/bar/prod.toml", "value = 3\n"),
    ]);
    assert_eq!(
        run(
            &dir,
            &[
                "-a",
                "foo/bar/prod.toml",
                "-G",
                "defaults.toml",
                "--list-files"
            ]
        ),
        format!(
            "{}\n{}\n",
            std::path::Path::new("foo/defaults.toml").display(),
            std::path::Path::new("foo/bar/defaults.toml").display()
        )
    );
    assert_eq!(
        run(&dir, &["-a", "foo/bar/prod.toml", "-G", "defaults.toml"]),
        "value = 2\n"
    );
    assert_eq!(
        run(
            &dir,
            &[
                "-a",
                "foo/bar/prod.toml",
                "-g",
                "foo/**/{defaults,prod}.toml"
            ]
        ),
        "value = 3\n"
    );
    assert!(
        run_err(
            &dir,
            &["-a", "foo/missing.toml", "-G", "none", "--list-files"]
        )
        .contains("inspecting")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing", dir.path().join("foo/broken.toml")).unwrap();
        assert!(
            run_err(&dir, &["-a", "foo/bar/prod.toml", "-G", "defaults.toml"])
                .contains("foo/broken.toml")
        );
    }
}

#[test]
fn glob_treats_stdin_as_a_literal_candidate() {
    let dir = tree(&[]);
    for flag in ["-g", "-G"] {
        let out = knf(&dir)
            .args(["-", flag, "{*.json,-}", "-f", "json", "--compact"])
            .write_stdin(r#"{"value":1}"#)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"{\"value\":1}\n");
        assert_eq!(run(&dir, &["-", flag, "*.toml", "--compact"]), "{}\n");
        assert_eq!(run(&dir, &["-", flag, "-", "--list-files"]), "-\n");
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
        run_err(&dir, &["missing.toml", "-G", "none", "--set", "a[0]=1"])
            .contains("--set takes KEY.PATH=VALUE")
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

/// TOML datetimes remain native throughout the pipeline.
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

/// A datetime stays native when inline layers are appended.
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

#[test]
fn a_different_format_option_changes_parsing_not_output_conversion() {
    let dir = tree(&[("f.toml", DATED), ("g.json", r#"{"extra":1}"#)]);
    assert!(run_err(&dir, &["f.toml", "g.json", "-f", "toml"]).contains("g.json: invalid TOML"));
    assert!(run_err(&dir, &["f.toml", "-f", "json"]).contains("f.toml: invalid JSON"));
}

#[test]
fn strict_catches_a_string_over_a_native_toml_datetime() {
    let dir = tree(&[
        ("f.toml", DATED),
        ("g.toml", "date = '1979-05-27T07:32:00Z'"),
    ]);
    let err = run_err(&dir, &["f.toml", "g.toml", "--strict"]);
    assert!(
        err.contains("type conflict at `date`: datetime would be replaced by string"),
        "{err}"
    );
}

#[test]
fn explicit_format_overrides_extensions_for_every_layer() {
    let dir = tree(&[("a.toml", r#"{"a":1}"#), ("b.json", r#"{"b":2}"#)]);
    assert_eq!(
        run(&dir, &["a.toml", "b.json", "-f", "json", "--compact"]),
        "{\"a\":1,\"b\":2}
"
    );
    let dir = tree(&[("a.json", "a = 1"), ("b.toml", "b = 2")]);
    assert_eq!(
        run(&dir, &["a.json", "b.toml", "--format", "toml"]),
        "a = 1
b = 2
"
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
/// native integer values must retain every digit.
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
fn stdin_is_a_layer() {
    let dir = tree(&[("base.json", r#"{"a":1,"b":2}"#)]);
    let out = knf(&dir)
        .args(["base.json", "-", "-f", "json", "--compact"])
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
/// gone. Compare the native TOML output with the expected value.
#[test]
fn shallow_takes_top_level_tables_whole() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    let out = run(&dir, &["base.toml", "prod.toml", "--shallow", "--compact"]);
    assert_toml_eq_json(
        &out,
        "{\"plugins\":[\"metrics\"],\"db\":{\"host\":\"prod\"}}\n",
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

/// `--shallow=db` is `+` at `db` only: `db.pool` is prod's whole, `db.host`
/// survives, and `app` is still a deep merge.
#[test]
fn shallow_at_a_path_keeps_everything_else_deep() {
    let dir = tree(&[("base.toml", NESTED_BASE), ("prod.toml", NESTED_PROD)]);
    let out = run(
        &dir,
        &["base.toml", "prod.toml", "--shallow=db", "--compact"],
    );
    assert_toml_eq_json(
        &out,
        "{\"db\":{\"host\":\"local\",\"pool\":{\"max\":9}},\"app\":{\"pool\":{\"min\":1,\"max\":9}}}\n",
    );
}

#[test]
fn shallow_paths_are_repeatable() {
    let dir = tree(&[("base.toml", NESTED_BASE), ("prod.toml", NESTED_PROD)]);
    let out = run(
        &dir,
        &[
            "base.toml",
            "prod.toml",
            "--shallow=db",
            "--shallow=app",
            "--compact",
        ],
    );
    assert_toml_eq_json(
        &out,
        "{\"db\":{\"host\":\"local\",\"pool\":{\"max\":9}},\"app\":{\"pool\":{\"max\":9}}}\n",
    );
}

/// The `=` is required, so a bare `--shallow` ahead of the files cannot take
/// the first one as its path.
#[test]
fn bare_shallow_before_files_still_reads_them() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    assert_eq!(
        run(&dir, &["--shallow", "base.toml", "prod.toml"]),
        run(&dir, &["base.toml", "prod.toml", "--shallow"])
    );
}

/// `--shallow=` is the empty path, which is the root, like the bare flag.
#[test]
fn empty_shallow_path_is_the_root() {
    let dir = tree(&[("base.toml", BASE), ("prod.toml", PROD)]);
    assert_eq!(
        run(&dir, &["base.toml", "prod.toml", "--shallow="]),
        run(&dir, &["base.toml", "prod.toml", "--shallow"])
    );
}

/// A string fallback can be overwritten by a later typed inline layer.
#[test]
fn set_null_overwritten_before_toml_emit() {
    let dir = tree(&[("f.toml", "a = 0\n")]);
    assert_eq!(
        run(&dir, &["f.toml", "--set", "a=null", "--set", "a=1"]),
        "a = 1\n"
    );
}

#[test]
fn toml_inline_typing_uses_native_values_and_string_fallback() {
    let dir = tree(&[]);
    let out = run(
        &dir,
        &[
            "-f",
            "toml",
            "--set",
            "proxy=null",
            "--set",
            "day=1979-05-27",
            "--set",
            "db={host='local'}",
            "--set",
            "large=18446744073709551615",
            "--set",
            "bad=[a,b]",
            "--set",
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

// --- native inline values -------------------------------------------------

#[test]
fn removed_options_are_usage_errors() {
    let dir = tree(&[]);
    for args in [["--input-format", "json"], ["--null-as", "none"]] {
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
fn empty_runs_use_the_selected_native_format() {
    let dir = tree(&[]);
    assert_eq!(
        run(&dir, &["--compact"]),
        "{}
"
    );
    assert_eq!(
        run(&dir, &["-f", "toml"]),
        "
"
    );
    assert_eq!(
        run(&dir, &["-f", "toml", "--set", "a=1"]),
        "a = 1
"
    );
    assert_eq!(
        run(
            &dir,
            &["missing.json", "-g", "*.toml", "-f", "toml", "--set", "a=1"]
        ),
        "a = 1
"
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

/// Mixed formats fail before merge, even when the documents would conflict.
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

/// Like `--set`, an index in a `--shallow` path is an argv mistake, reported
/// before any file is read.
#[test]
fn shallow_with_an_array_index_errors_before_file_io() {
    let dir = tree(&[]);
    let err = run_err(&dir, &["missing.toml", "--shallow=servers[0]"]);
    assert!(
        !err.contains("missing.toml"),
        "the --shallow path should be rejected before the file is read:\n{err}"
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
