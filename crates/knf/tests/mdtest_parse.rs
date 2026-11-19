//! Unit tests for the fixture parser. The `mdtest` target has a custom harness, so they live here.

#[allow(dead_code)]
#[path = "mdtest/parse.rs"]
mod parse;

use parse::parse;

fn parse_err(text: &str) -> String {
    parse("t.md", text).expect_err("should be rejected").message
}

#[test]
fn parses_files_command_and_expected_output() {
    let text = "# A\n\n## B\n\n`x/a.toml`:\n\n```toml\nk = 1\n```\n\n```console\n$ FOO='a b' knf x/a.toml -i\n? 1\n```\n\n`stderr`:\n\n```text\nboom\n```\n";
    let tests = parse("t.md", text).unwrap();
    assert_eq!(tests.len(), 1);
    let test = &tests[0];
    assert_eq!(test.name, "t.md - A - B");
    assert_eq!(test.files[0].path, "x/a.toml");
    assert_eq!(test.files[0].contents, "k = 1\n");
    assert_eq!(test.invocation.env, [("FOO".into(), "a b".into())]);
    assert_eq!(test.invocation.args, ["x/a.toml", "-i"]);
    assert_eq!(test.invocation.exit_code, 1);
    assert!(test.stdout.is_none());
    let stderr = test.stderr.as_ref().unwrap();
    assert_eq!(stderr.body, "boom\n");
    assert_eq!(&text[stderr.body_range.clone()], "boom\n");
}

#[test]
fn children_inherit_and_override_parent_files() {
    let text = "# G\n\n`a.toml`:\n\n```toml\nx = 1\n```\n\n`b.toml`:\n\n```toml\ny = 1\n```\n\n## T\n\n`b.toml`:\n\n```toml\ny = 2\n```\n\n```console\n$ knf a.toml b.toml\n```\n";
    let tests = parse("t.md", text).unwrap();
    assert_eq!(tests.len(), 1);
    let files: Vec<_> = tests[0]
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.contents.as_str()))
        .collect();
    assert_eq!(files, [("a.toml", "x = 1\n"), ("b.toml", "y = 2\n")]);
}

#[test]
fn rejects_indented_fence() {
    assert!(parse_err("# T\n\n  ```console\n  $ knf\n  ```\n").contains("indented"));
}

#[test]
fn rejects_unterminated_fence() {
    assert!(parse_err("# T\n\n```console\n$ knf\n").contains("unterminated"));
}

#[test]
fn rejects_two_console_blocks() {
    let text = "# T\n\n```console\n$ knf\n```\n\n```console\n$ knf\n```\n";
    assert!(parse_err(text).contains("exactly one console"));
}

#[test]
fn rejects_unknown_labels() {
    assert!(parse_err("# T\n\n`a.toml`:\n\nprose\n").contains("not followed"));
    assert!(parse_err("# T\n\n`../a.toml`:\n\n```toml\n```\n").contains("invalid input file"));
    assert!(parse_err("# T\n\n```toml\na = 1\n```\n").contains("unlabelled"));
}

#[test]
fn rejects_heading_inside_a_test_section() {
    let text = "# T\n\n```console\n$ knf\n```\n\n## Sub\n";
    assert!(parse_err(text).contains("subsections"));
}

#[test]
fn rejects_output_without_a_console_block() {
    assert!(parse_err("# T\n\n`stdout`:\n\n```text\nx\n```\n").contains("no console"));
}

#[test]
fn rejects_malformed_console_blocks() {
    assert!(parse_err("# T\n\n```console\nknf\n```\n").contains("start with"));
    assert!(parse_err("# T\n\n```console\n$ echo hi\n```\n").contains("must be"));
    assert!(parse_err("# T\n\n```console\n$ knf\n? x\n```\n").contains("exit code"));
    assert!(parse_err("# T\n\n```console\n$ knf 'a\n```\n").contains("quotes"));
}

#[test]
fn headings_inside_fences_are_not_sections() {
    let text =
        "# T\n\n`a.toml`:\n\n```toml\n# not a heading\n```\n\n```console\n$ knf a.toml\n```\n";
    assert_eq!(parse("t.md", text).unwrap().len(), 1);
}
