//! Interpolation error types and formatting.

use std::fmt;

use super::scan::ScanError;
use crate::{RefPathError, Seg, ValueKind, render_path};

/// Why interpolation failed: every problem found, or a single cycle.
#[derive(Debug)]
pub enum InterpError {
    Problems(Problems),
    Cycle(Cycle),
}

impl std::error::Error for InterpError {}

impl fmt::Display for InterpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cycle(cycle) => write!(f, "{cycle}"),
            Self::Problems(problems) => {
                // No trailing newline.
                let mut lines: Vec<String> = Vec::new();
                for group in Group::ALL {
                    let members = problems.iter().filter(|p| p.kind.group() == group);
                    let mut any = false;
                    for problem in members {
                        if !any {
                            lines.push(group.header().to_string());
                            any = true;
                        }
                        lines.push(format!(
                            "  --> {}{}: {}",
                            render_path(&problem.path),
                            problem.origin.suffix(),
                            problem.kind.detail()
                        ));
                    }
                }
                f.write_str(&lines.join("\n"))
            }
        }
    }
}

/// At least one [`Problem`], in the order found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problems(Vec<Problem>);

impl Problems {
    pub(super) fn new(problems: Vec<Problem>) -> Option<Self> {
        (!problems.is_empty()).then_some(Self(problems))
    }

    /// The problems as a non-empty slice.
    pub fn as_slice(&self) -> &[Problem] {
        &self.0
    }

    /// Iterate in the order found.
    pub fn iter(&self) -> std::slice::Iter<'_, Problem> {
        self.0.iter()
    }
}

impl IntoIterator for Problems {
    type Item = Problem;
    type IntoIter = std::vec::IntoIter<Problem>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Problems {
    type Item = &'a Problem;
    type IntoIter = std::slice::Iter<'a, Problem>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// One thing wrong with one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The tree that holds the offending string.
    pub origin: Origin,
    /// Where in that tree the string lives.
    pub path: Vec<Seg>,
    pub kind: ProblemKind,
}

/// The tree a string was written in, or merged into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The configs being merged.
    Output,
    /// [`InterpolationOptions::context`](crate::InterpolationOptions::context).
    Context,
}

impl Origin {
    /// What follows a path in a report; output paths are unmarked.
    fn suffix(self) -> &'static str {
        match self {
            Self::Output => "",
            Self::Context => " (context)",
        }
    }
}

/// What is wrong with a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemKind {
    /// A reference that is not spelled like one.
    Syntax(Syntax),
    /// A missing key or unset environment variable.
    Unresolved { reference: String },
    /// A container or null in embedded position: `url = "http://${db}/"`.
    NotStringifiable { reference: String, kind: ValueKind },
}

impl ProblemKind {
    fn group(&self) -> Group {
        match self {
            Self::Syntax(_) => Group::Syntax,
            Self::Unresolved { .. } => Group::Unresolved,
            Self::NotStringifiable { .. } => Group::NotStringifiable,
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Syntax(error) => error.to_string(),
            Self::Unresolved { reference } => format!("`{reference}`"),
            Self::NotStringifiable { reference, kind } => {
                format!("`{reference}` is {} {kind}", kind.article())
            }
        }
    }
}

/// A malformed reference.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Syntax {
    /// Rejected while delimiting `${...}`.
    #[error(transparent)]
    Scan(#[from] ScanError),
    /// `${env:}`.
    #[error("empty variable name in `${{env:}}`")]
    EmptyEnvName,
    /// A body that is not a path: `${a..b}`, `${servers[x]}`.
    #[error("{} in reference `${{{body}}}`", defect(error))]
    Path { body: String, error: RefPathError },
}

fn defect(error: &RefPathError) -> &'static str {
    match error {
        // A body is only empty when a nested reference expands to `""`.
        RefPathError::EmptyPath | RefPathError::EmptySegment { .. } => "empty segment",
        RefPathError::BadIndex { .. } => "malformed index",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Syntax,
    Unresolved,
    NotStringifiable,
}

impl Group {
    /// Report order, most fundamental first.
    const ALL: [Self; 3] = [Self::Syntax, Self::Unresolved, Self::NotStringifiable];

    fn header(self) -> &'static str {
        match self {
            Self::Syntax => "invalid reference",
            Self::Unresolved => "unresolved reference",
            Self::NotStringifiable => "reference cannot be rendered into a string",
        }
    }
}

/// A reference that resolves to itself, reported as the full chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    chain: Vec<Vec<Seg>>,
}

impl Cycle {
    pub(crate) fn new(chain: Vec<Vec<Seg>>) -> Self {
        Self { chain }
    }
}

impl fmt::Display for Cycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hops: Vec<String> = self
            .chain
            .iter()
            .map(|p| format!("`{}`", render_path(p)))
            .collect();
        write!(f, "reference cycle: {}", hops.join(" -> "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(dotted: &str) -> Vec<Seg> {
        dotted.split('.').map(|s| Seg::Key(s.to_string())).collect()
    }

    fn output(path: Vec<Seg>, kind: ProblemKind) -> Problem {
        Problem {
            origin: Origin::Output,
            path,
            kind,
        }
    }

    fn unresolved(reference: &str) -> ProblemKind {
        ProblemKind::Unresolved {
            reference: reference.into(),
        }
    }

    fn report(problems: Vec<Problem>) -> String {
        InterpError::Problems(Problems::new(problems).expect("non-empty")).to_string()
    }

    #[test]
    fn problems_cannot_be_empty() {
        assert_eq!(Problems::new(Vec::new()), None);
    }

    #[test]
    fn one_kind_renders_as_a_header_and_a_list() {
        assert_eq!(
            report(vec![
                output(key("server.url"), unresolved("db.hostname")),
                output(
                    vec![Seg::Key("tags".into()), Seg::Index(0)],
                    unresolved("env:REGION")
                ),
            ]),
            "unresolved reference\n\
             \x20 --> server.url: `db.hostname`\n\
             \x20 --> tags[0]: `env:REGION`"
        );
    }

    #[test]
    fn kinds_group_in_a_fixed_order() {
        assert_eq!(
            report(vec![
                output(
                    key("url"),
                    ProblemKind::NotStringifiable {
                        reference: "db".into(),
                        kind: ValueKind::Object,
                    }
                ),
                output(key("a"), unresolved("nope")),
                output(key("b"), ProblemKind::Syntax(ScanError::EmptyRef.into())),
            ]),
            "invalid reference\n\
             \x20 --> b: empty reference `${}`\n\
             unresolved reference\n\
             \x20 --> a: `nope`\n\
             reference cannot be rendered into a string\n\
             \x20 --> url: `db` is an object"
        );
    }

    #[test]
    fn context_paths_are_marked() {
        let problem = Problem {
            origin: Origin::Context,
            path: key("settings.url"),
            kind: unresolved("missing"),
        };
        assert_eq!(
            report(vec![problem]),
            "unresolved reference\n  --> settings.url (context): `missing`"
        );
    }

    #[test]
    fn path_syntax_names_the_defect() {
        for (body, want) in [
            ("a..b", "empty segment in reference `${a..b}`"),
            ("", "empty segment in reference `${}`"),
            ("a[x]", "malformed index in reference `${a[x]}`"),
        ] {
            let error = body.parse::<crate::RefPath>().unwrap_err();
            let syntax = Syntax::Path {
                body: body.into(),
                error,
            };
            assert_eq!(syntax.to_string(), want);
        }
    }

    #[test]
    fn a_cycle_reads_as_a_chain() {
        let err = InterpError::Cycle(Cycle::new(vec![key("a"), key("b"), key("a")]));
        assert_eq!(err.to_string(), "reference cycle: `a` -> `b` -> `a`");
    }
}
