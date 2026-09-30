//! Interpolation errors. They name key paths only, never files or flags.

use std::fmt;

use super::scan::Syntax;
use crate::{Seg, render_path};

/// Why interpolation failed: every problem found, or a single cycle.
#[derive(Debug)]
pub enum InterpError {
    Problems(Vec<Problem>),
    Cycle(Cycle),
}

impl std::error::Error for InterpError {}

impl fmt::Display for InterpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cycle(cycle) => write!(f, "{cycle}"),
            Self::Problems(problems) => {
                // Grouped by kind in a fixed order. No trailing newline.
                let mut lines: Vec<String> = Vec::new();
                for group in Group::ALL {
                    let members = problems.iter().filter(|p| p.group() == group);
                    let mut any = false;
                    for problem in members {
                        if !any {
                            lines.push(group.header().to_string());
                            any = true;
                        }
                        lines.push(format!(
                            "  --> {}: {}",
                            render_path(problem.path()),
                            problem.detail()
                        ));
                    }
                }
                f.write_str(&lines.join("\n"))
            }
        }
    }
}

/// One thing wrong with one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The selected inheritance field must be one whole-string reference.
    MergeReference { path: Vec<Seg> },
    /// An inheritance reference resolved to a non-object value.
    MergeObject { path: Vec<Seg>, kind: &'static str },
    /// A reference that is not spelled like one.
    Syntax { path: Vec<Seg>, error: Syntax },
    /// A missing key or unset environment variable.
    Unresolved { path: Vec<Seg>, reference: String },
    /// A container or null in embedded position: `url = "http://${db}/"`.
    NotStringifiable {
        path: Vec<Seg>,
        reference: String,
        kind: &'static str,
    },
}

impl Problem {
    /// Where in the merged document or context the offending string lives.
    pub fn path(&self) -> &[Seg] {
        match self {
            Self::Syntax { path, .. }
            | Self::Unresolved { path, .. }
            | Self::MergeReference { path }
            | Self::MergeObject { path, .. }
            | Self::NotStringifiable { path, .. } => path,
        }
    }

    fn group(&self) -> Group {
        match self {
            Self::Syntax { .. } => Group::Syntax,
            Self::Unresolved { .. } => Group::Unresolved,
            Self::NotStringifiable { .. } => Group::NotStringifiable,
            Self::MergeReference { .. } | Self::MergeObject { .. } => Group::Inheritance,
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Syntax { error, .. } => error.to_string(),
            Self::Unresolved { reference, .. } => format!("`{reference}`"),
            Self::MergeReference { .. } => {
                "expected one whole-string reference to an object".into()
            }
            Self::MergeObject { kind, .. } => format!("expected an object, found {kind}"),
            Self::NotStringifiable {
                reference, kind, ..
            } => format!("`{reference}` is {} {kind}", article(kind)),
        }
    }
}

/// `an` for `object`/`array`, `a` for `null`.
fn article(kind: &str) -> &'static str {
    if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Inheritance,
    Syntax,
    Unresolved,
    NotStringifiable,
}

impl Group {
    /// Report order, most fundamental first.
    const ALL: [Self; 4] = [
        Self::Syntax,
        Self::Unresolved,
        Self::NotStringifiable,
        Self::Inheritance,
    ];

    fn header(self) -> &'static str {
        match self {
            Self::Syntax => "invalid reference",
            Self::Unresolved => "unresolved reference",
            Self::NotStringifiable => "reference cannot be rendered into a string",
            Self::Inheritance => "invalid object inheritance",
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

    #[test]
    fn one_kind_renders_as_a_header_and_a_list() {
        let err = InterpError::Problems(vec![
            Problem::Unresolved {
                path: key("server.url"),
                reference: "db.hostname".into(),
            },
            Problem::Unresolved {
                path: vec![Seg::Key("tags".into()), Seg::Index(0)],
                reference: "env:REGION".into(),
            },
        ]);
        assert_eq!(
            err.to_string(),
            "unresolved reference\n\
             \x20 --> server.url: `db.hostname`\n\
             \x20 --> tags[0]: `env:REGION`"
        );
    }

    #[test]
    fn kinds_group_in_a_fixed_order() {
        let err = InterpError::Problems(vec![
            Problem::NotStringifiable {
                path: key("url"),
                reference: "db".into(),
                kind: "object",
            },
            Problem::Unresolved {
                path: key("a"),
                reference: "nope".into(),
            },
            Problem::Syntax {
                path: key("b"),
                error: Syntax::EmptyRef,
            },
        ]);
        assert_eq!(
            err.to_string(),
            "invalid reference\n\
             \x20 --> b: empty reference `${}`\n\
             unresolved reference\n\
             \x20 --> a: `nope`\n\
             reference cannot be rendered into a string\n\
             \x20 --> url: `db` is an object"
        );
    }

    #[test]
    fn a_cycle_reads_as_a_chain() {
        let err = InterpError::Cycle(Cycle::new(vec![key("a"), key("b"), key("a")]));
        assert_eq!(err.to_string(), "reference cycle: `a` -> `b` -> `a`");
    }

    #[test]
    fn articles_match_the_three_possible_kinds() {
        assert_eq!(article("object"), "an");
        assert_eq!(article("array"), "an");
        assert_eq!(article("null"), "a");
    }
}
