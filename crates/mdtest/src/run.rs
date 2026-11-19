//! Executes parsed fixture tests against the configured binary and reports or rewrites expectations.

use std::path::Path;
use std::process::Command;

use similar::TextDiff;

use crate::Config;
use crate::parse::{Expected, Test};

pub(crate) struct Outcome {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

pub(crate) fn execute(test: &Test, config: &Config) -> Outcome {
    let dir = tempfile::tempdir().expect("tempdir");
    for file in &test.files {
        let full = dir.path().join(&file.path);
        std::fs::create_dir_all(full.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(&full, &file.contents).expect("write input file");
    }
    let mut cmd = Command::new(config.bin);
    cmd.current_dir(dir.path()).env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    for (name, value) in &test.invocation.env {
        cmd.env(name, value);
    }
    let out = cmd
        .args(&test.invocation.args)
        .output()
        .expect("spawn binary");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
    }
}

fn expected_text(expected: &Option<Expected>) -> &str {
    expected.as_ref().map_or("", |e| e.body.as_str())
}

/// One human-readable failure per mismatching stream, each anchored at `file:line`.
pub(crate) fn mismatches(file: &str, test: &Test, outcome: &Outcome) -> Vec<String> {
    let mut failures = Vec::new();
    let mut fail = |what: &str, detail: String| {
        failures.push(format!(
            "{file}:{line}: {name}\n{what} mismatch:\n{detail}\nrerun with MDTEST_FILTER='{title}'",
            line = test.line,
            name = test.name,
            title = test.name.rsplit(" - ").next().unwrap_or(&test.name),
        ));
    };

    if outcome.code != Some(test.invocation.exit_code) {
        fail(
            "exit code",
            format!(
                "expected {}, got {:?}\nstderr:\n{}",
                test.invocation.exit_code, outcome.code, outcome.stderr
            ),
        );
    }
    for (what, expected, actual) in [
        ("stdout", expected_text(&test.stdout), &outcome.stdout),
        ("stderr", expected_text(&test.stderr), &outcome.stderr),
    ] {
        if expected != actual {
            let diff = TextDiff::from_lines(expected, actual.as_str());
            fail(
                what,
                diff.unified_diff().header("expected", "actual").to_string(),
            );
        }
    }
    failures
}

struct Edit {
    range: std::ops::Range<usize>,
    replacement: String,
}

fn fenced(label: &str, body: &str) -> String {
    format!("`{label}`:\n\n```text\n{body}```\n")
}

fn rewrite_stream(text: &str, expected: &Option<Expected>, actual: &str) -> Option<Edit> {
    match (expected, actual.is_empty()) {
        (None, true) => None,
        (Some(e), true) => {
            let preceding_blank = text[..e.block_range.start].ends_with("\n\n");
            let start = e.block_range.start - usize::from(preceding_blank);
            Some(Edit {
                range: start..e.block_range.end,
                replacement: String::new(),
            })
        }
        (Some(e), false) if e.body == actual => None,
        (Some(e), false) => Some(Edit {
            range: e.body_range.clone(),
            replacement: actual.to_owned(),
        }),
        (None, false) => unreachable!("insertions are handled by the caller"),
    }
}

/// Rewrites `stdout`/`stderr` fences of `tests` to match `outcomes`, returning the new text.
pub(crate) fn blessed(text: &str, tests: &[(&Test, Outcome)]) -> String {
    let mut edits = Vec::new();
    for (test, outcome) in tests {
        let mut insertion = String::new();
        for (label, expected, actual) in [
            ("stdout", &test.stdout, &outcome.stdout),
            ("stderr", &test.stderr, &outcome.stderr),
        ] {
            match (expected, actual.is_empty()) {
                (None, false) => {
                    if !insertion.is_empty() || label == "stdout" || test.stdout.is_none() {
                        insertion.push('\n');
                        insertion.push_str(&fenced(label, actual));
                    } else if let Some(stdout) = &test.stdout {
                        edits.push(Edit {
                            range: stdout.block_range.end..stdout.block_range.end,
                            replacement: format!("\n{}", fenced(label, actual)),
                        });
                    }
                }
                _ => edits.extend(rewrite_stream(text, expected, actual)),
            }
        }
        if !insertion.is_empty() {
            edits.push(Edit {
                range: test.console_end..test.console_end,
                replacement: insertion,
            });
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse((e.range.start, e.range.end)));
    let mut out = text.to_owned();
    for edit in edits {
        out.replace_range(edit.range, &edit.replacement);
    }
    out
}

pub(crate) fn run_file(path: &Path, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .expect("utf-8 name");
    let text = std::fs::read_to_string(path)?;
    let tests =
        crate::parse::parse(name, &text, config.program).map_err(|e| format!("{name}: {e}"))?;
    let filter = std::env::var("MDTEST_FILTER").ok();
    let selected: Vec<&Test> = tests
        .iter()
        .filter(|t| filter.as_deref().is_none_or(|f| t.name.contains(f)))
        .collect();

    if std::env::var_os("MDTEST_UPDATE").is_some() {
        let outcomes: Vec<(&Test, Outcome)> =
            selected.iter().map(|t| (*t, execute(t, config))).collect();
        let updated = blessed(&text, &outcomes);
        if updated != text {
            std::fs::write(path, updated)?;
        }
        return Ok(());
    }

    let mut failures = Vec::new();
    for test in selected {
        failures.extend(mismatches(name, test, &execute(test, config)));
    }
    if failures.is_empty() {
        return Ok(());
    }
    for failure in &failures {
        eprintln!("\n{failure}");
    }
    Err(format!("{name}: {} section(s) failed", failures.len()).into())
}
