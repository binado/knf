//! What interpolation reports, and how it reads.
//!
//! Key paths and nothing else. Not a rule to enforce here so much as one that
//! cannot be broken: this pass runs after the merge, and no layer outlives the
//! merge, so the filename a reference was written in is genuinely unavailable.
//! No flag names either — `crates/knf/src/lib.rs` adds the `help:` line that
//! knows what the flags are called.

use std::fmt;

use crate::path::{Seg, render_path};
use crate::scan::Syntax;

/// Why interpolation failed.
///
/// Two shapes because the two failures differ in kind. Everything a document
/// gets *wrong* is collected and reported together — references are written all
/// over a config, and rediscovering them one run at a time is the experience
/// this avoids. A cycle is the exception: resolution cannot continue past it, so
/// it is an early return and arrives alone.
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
                // Grouped by kind rather than listed in document order: one
                // header per kind keeps a mixed report as readable as a pure
                // one, and the group order is fixed so the message never
                // depends on where in the document the first mistake happened.
                //
                // No trailing newline — the caller appends its own `help:`
                // lines, exactly as `NullInToml` does.
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
    /// A reference that is not spelled like one.
    Syntax { path: Vec<Seg>, error: Syntax },
    /// A reference that names nothing: a key the merged document does not have,
    /// or a variable the environment does not set.
    ///
    /// An error rather than a pass-through. Leaving `${db.hostname}` in the
    /// output would ship a typo as a literal, and the document is already the
    /// authority on what exists.
    Unresolved { path: Vec<Seg>, reference: String },
    /// A container or a null in embedded position — `url = "http://${db}/"`.
    ///
    /// Legal in *whole-string* position, where it aliases the subtree. Embedded
    /// it has no format-independent rendering, so it is rejected in v1.
    NotStringifiable {
        path: Vec<Seg>,
        reference: String,
        kind: &'static str,
    },
}

impl Problem {
    /// Where in the merged document the offending string lives.
    pub fn path(&self) -> &[Seg] {
        match self {
            Self::Syntax { path, .. }
            | Self::Unresolved { path, .. }
            | Self::NotStringifiable { path, .. } => path,
        }
    }

    fn group(&self) -> Group {
        match self {
            Self::Syntax { .. } => Group::Syntax,
            Self::Unresolved { .. } => Group::Unresolved,
            Self::NotStringifiable { .. } => Group::NotStringifiable,
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Syntax { error, .. } => error.to_string(),
            Self::Unresolved { reference, .. } => format!("`{reference}`"),
            Self::NotStringifiable {
                reference, kind, ..
            } => format!("`{reference}` is {} {kind}", article(kind)),
        }
    }
}

/// `object` and `array` take `an`; `null` takes `a`. Three possible inputs, so
/// the vowel test is exact rather than a heuristic that will meet `hour`.
fn article(kind: &str) -> &'static str {
    if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Syntax,
    Unresolved,
    NotStringifiable,
}

impl Group {
    /// Fixed order, most fundamental first: a malformed reference was never
    /// going to resolve, so saying so before listing what is missing reads in
    /// the order the user will fix things.
    const ALL: [Self; 3] = [Self::Syntax, Self::Unresolved, Self::NotStringifiable];

    fn header(self) -> &'static str {
        match self {
            Self::Syntax => "invalid reference",
            Self::Unresolved => "unresolved reference",
            Self::NotStringifiable => "reference cannot be rendered into a string",
        }
    }
}

/// A reference that resolves, directly or indirectly, to itself.
///
/// Reported as the whole chain rather than the one path it closed at: a two-hop
/// cycle is obvious from either end, but a five-hop one is not.
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

    /// Mixed kinds group, in a fixed order that does not depend on where in the
    /// document each mistake was found.
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
