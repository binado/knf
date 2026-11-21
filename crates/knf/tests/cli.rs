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

/// stderr of a run that must fail with exit code 1.
fn run_err(dir: &TempDir, args: &[&str]) -> String {
    err_stderr(knf(dir).args(args), args)
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

// --- glob filters --------------------------------------------------------

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

// --- typed inline values -------------------------------------------------

// --- multi-line error messages (snapshotted) ------------------------------
