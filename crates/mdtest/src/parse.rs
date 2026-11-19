//! Parser for the Markdown fixture format described in the crate README.

use std::fmt;
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InputFile {
    pub path: String,
    pub contents: String,
}

/// An expected-output fence, located by byte ranges so it can be rewritten in place.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Expected {
    pub body: String,
    /// The body lines, excluding the fence lines.
    pub body_range: Range<usize>,
    /// The label line through the closing fence line, newline included.
    pub block_range: Range<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    pub exit_code: i32,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Test {
    pub name: String,
    pub line: usize,
    pub files: Vec<InputFile>,
    pub invocation: Invocation,
    /// Byte offset just past the console block.
    pub console_end: usize,
    pub stdout: Option<Expected>,
    pub stderr: Option<Expected>,
}

struct Section {
    level: usize,
    line: usize,
    title: String,
    files: Vec<InputFile>,
    pending: Pending,
}

#[derive(Default)]
struct Pending {
    console: Option<(Invocation, usize)>,
    stdout: Option<Expected>,
    stderr: Option<Expected>,
}

impl Pending {
    fn is_test(&self) -> bool {
        self.console.is_some()
    }

    fn has_output(&self) -> bool {
        self.stdout.is_some() || self.stderr.is_some()
    }
}

struct Fence<'a> {
    info: &'a str,
    body_range: Range<usize>,
    end: usize,
    next_line: usize,
}

struct Lines<'a> {
    text: &'a str,
    /// Byte offset of the start of each line, plus a final entry for end of text.
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        if *starts.last().expect("non-empty") != text.len() {
            starts.push(text.len());
        }
        Self { text, starts }
    }

    fn len(&self) -> usize {
        self.starts.len() - 1
    }

    fn get(&self, i: usize) -> &'a str {
        let line = &self.text[self.starts[i]..self.starts[i + 1]];
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line)
    }

    fn start(&self, i: usize) -> usize {
        self.starts[i]
    }
}

fn fence_ticks(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b'`').count()
}

fn is_indented_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.len() != line.len() && fence_ticks(trimmed) >= 3
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    let rest = line[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, rest.trim()))
}

fn label(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('`')?.strip_suffix("`:")?;
    (!inner.is_empty() && !inner.contains('`')).then_some(inner)
}

fn read_fence<'a>(lines: &Lines<'a>, open: usize) -> Result<Fence<'a>, ParseError> {
    let opener = lines.get(open);
    let ticks = fence_ticks(opener);
    let info = opener[ticks..].trim();
    if info.contains('`') {
        return err(open + 1, "invalid fence info string");
    }
    for close in open + 1..lines.len() {
        let line = lines.get(close);
        let closing = fence_ticks(line);
        if closing >= ticks && line[closing..].trim().is_empty() {
            return Ok(Fence {
                info,
                body_range: lines.start(open + 1)..lines.start(close),
                end: lines.start(close + 1),
                next_line: close + 1,
            });
        }
    }
    err(open + 1, "unterminated code fence")
}

fn validate_path(line: usize, path: &str) -> Result<(), ParseError> {
    let escapes = path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..");
    if escapes || path.contains('\\') {
        return err(
            line,
            format!(
                "invalid input file path `{path}`: expected a relative path inside the fixture"
            ),
        );
    }
    Ok(())
}

fn parse_console(line: usize, body: &str, program: &str) -> Result<Invocation, ParseError> {
    let mut rows = body.lines();
    let Some(command) = rows.next().and_then(|row| row.strip_prefix("$ ")) else {
        return err(line, "a console block must start with `$ `");
    };
    let exit_code = match rows.next() {
        None => 0,
        Some(row) => {
            let Some(code) = row.strip_prefix("? ").and_then(|n| n.trim().parse().ok()) else {
                return err(line, format!("expected `? <exit code>`, found `{row}`"));
            };
            code
        }
    };
    if rows.next().is_some() {
        return err(
            line,
            "a console block holds one `$ ` command and an optional `? N` line",
        );
    }
    let Some(tokens) = shlex::split(command) else {
        return err(line, "unbalanced quotes in command");
    };
    let mut tokens = tokens.into_iter().peekable();
    let mut env = Vec::new();
    while let Some(token) = tokens.next_if(|t| is_assignment(t)) {
        let (name, value) = token.split_once('=').expect("checked by is_assignment");
        env.push((name.to_owned(), value.to_owned()));
    }
    if tokens.next().as_deref() != Some(program) {
        return err(
            line,
            format!("the command must be `[NAME=VALUE ...] {program} <args>`"),
        );
    }
    Ok(Invocation {
        env,
        args: tokens.collect(),
        exit_code,
    })
}

fn is_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Parses a fixture file into runnable tests.
pub(crate) fn parse(file_name: &str, text: &str, program: &str) -> Result<Vec<Test>, ParseError> {
    let lines = Lines::new(text);
    let mut stack: Vec<Section> = Vec::new();
    let mut tests = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines.get(i);

        if is_indented_fence(line) {
            return err(i + 1, "code fences must not be indented");
        }

        if fence_ticks(line) >= 3 {
            let Some(section) = stack.last_mut() else {
                return err(i + 1, "code block outside any section");
            };
            let fence = read_fence(&lines, i)?;
            if fence.info != "console" {
                return err(
                    i + 1,
                    "unlabelled code block: put a `path`: label line before it",
                );
            }
            if section.pending.console.is_some() {
                return err(i + 1, "a test has exactly one console block");
            }
            let body = &text[fence.body_range.clone()];
            let invocation = parse_console(i + 1, body, program)?;
            section.pending.console = Some((invocation, fence.end));
            i = fence.next_line;
            continue;
        }

        if let Some((level, title)) = heading(line) {
            if let Some(parent) = stack.last()
                && parent.pending.is_test()
                && level > parent.level
            {
                return err(
                    i + 1,
                    "a section with a console block cannot have subsections",
                );
            }
            while stack.last().is_some_and(|s| s.level >= level) {
                let done = stack.pop().expect("checked non-empty");
                finish(file_name, &stack, done, &mut tests)?;
            }
            stack.push(Section {
                level,
                line: i + 1,
                title: title.to_owned(),
                files: Vec::new(),
                pending: Pending::default(),
            });
            i += 1;
            continue;
        }

        if let Some(name) = label(line) {
            let Some(section) = stack.last_mut() else {
                return err(i + 1, "label outside any section");
            };
            let mut open = i + 1;
            while open < lines.len() && lines.get(open).trim().is_empty() {
                open += 1;
            }
            if open >= lines.len() || fence_ticks(lines.get(open)) < 3 {
                return err(
                    i + 1,
                    format!("label `{name}` is not followed by a code block"),
                );
            }
            let fence = read_fence(&lines, open)?;
            let body = text[fence.body_range.clone()].to_owned();
            let block_range = lines.start(i)..fence.end;
            let expected = Expected {
                body,
                body_range: fence.body_range,
                block_range,
            };
            match name {
                "stdout" | "stderr" => {
                    let slot = if name == "stdout" {
                        &mut section.pending.stdout
                    } else {
                        &mut section.pending.stderr
                    };
                    if slot.is_some() {
                        return err(i + 1, format!("duplicate `{name}` block"));
                    }
                    *slot = Some(expected);
                }
                path => {
                    validate_path(i + 1, path)?;
                    if section.files.iter().any(|f| f.path == path) {
                        return err(i + 1, format!("duplicate input file `{path}`"));
                    }
                    section.files.push(InputFile {
                        path: path.to_owned(),
                        contents: expected.body,
                    });
                }
            }
            i = fence.next_line;
            continue;
        }

        i += 1;
    }

    while let Some(done) = stack.pop() {
        finish(file_name, &stack, done, &mut tests)?;
    }
    Ok(tests)
}

fn finish(
    file_name: &str,
    ancestors: &[Section],
    section: Section,
    tests: &mut Vec<Test>,
) -> Result<(), ParseError> {
    let Section {
        line,
        title,
        files,
        pending,
        ..
    } = section;
    let Some((invocation, console_end)) = pending.console else {
        if pending.has_output() {
            return err(
                line,
                format!("`{title}` has expected output but no console block"),
            );
        }
        return Ok(());
    };

    let mut inherited: Vec<InputFile> = Vec::new();
    for source in ancestors.iter().map(|s| &s.files).chain([&files]) {
        for file in source {
            inherited.retain(|f| f.path != file.path);
            inherited.push(InputFile {
                path: file.path.clone(),
                contents: file.contents.clone(),
            });
        }
    }

    let name = std::iter::once(file_name.to_owned())
        .chain(ancestors.iter().map(|s| s.title.clone()))
        .chain([title])
        .collect::<Vec<_>>()
        .join(" - ");
    tests.push(Test {
        name,
        line,
        files: inherited,
        invocation,
        console_end,
        stdout: pending.stdout,
        stderr: pending.stderr,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn parse_err(text: &str) -> String {
        parse("t.md", text, "knf")
            .expect_err("should be rejected")
            .message
    }

    #[test]
    fn parses_files_command_and_expected_output() {
        let text = "# A\n\n## B\n\n`x/a.toml`:\n\n```toml\nk = 1\n```\n\n```console\n$ FOO='a b' knf x/a.toml -i\n? 1\n```\n\n`stderr`:\n\n```text\nboom\n```\n";
        let tests = parse("t.md", text, "knf").unwrap();
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
        let tests = parse("t.md", text, "knf").unwrap();
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
        assert_eq!(parse("t.md", text, "knf").unwrap().len(), 1);
    }
}
