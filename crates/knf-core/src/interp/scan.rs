//! Splitting a parsed string value into literal text and reference bodies.

use std::fmt;

/// One span of a scanned string. `Ref` bodies are left unparsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Literal(&'a str),
    Ref(&'a str),
    Malformed { spelling: &'a str, error: Syntax },
}

/// A malformed reference.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Syntax {
    /// `${` with no `}` after it.
    #[error("unterminated `${{` at offset {offset}")]
    Unterminated { offset: usize },
    /// References nested deeper than [`MAX_NESTING`].
    #[error("references nested deeper than {MAX_NESTING} levels at offset {offset}")]
    TooDeep { offset: usize },
    /// `${}` — a reference to nothing.
    #[error("empty reference `${{}}`")]
    EmptyRef,
    /// `${env:}`. Raised by the resolver.
    #[error("empty variable name in `${{env:}}`")]
    EmptyEnvName,
    /// `${a..b}`. Raised by the resolver.
    #[error("empty segment in reference `${{{body}}}`")]
    EmptySegment { body: String },
    /// `${servers[x]}`. Raised by the resolver.
    #[error("malformed index in reference `${{{body}}}`")]
    BadIndex { body: String },
}

/// The deepest `${` nesting a reference may have; `${a}` is one level.
pub const MAX_NESTING: usize = 10;

/// Splits `s` into literals and reference bodies.
///
/// Returns an empty vector when `s` has no `$`. Malformed references become
/// [`Piece::Malformed`]; an unterminated one consumes the rest of the string.
pub fn scan(s: &str) -> Vec<Piece<'_>> {
    if !s.contains('$') {
        return Vec::new();
    }

    let mut pieces = Vec::new();
    let mut cursor = 0; // where the next `$` is searched from
    let mut literal = 0; // start of the pending literal run

    while let Some(rel) = s[cursor..].find('$') {
        let at = cursor + rel;
        match s.as_bytes().get(at + 1) {
            Some(b'$') => {
                push_literal(&mut pieces, &s[literal..at]);
                pieces.push(Piece::Literal("$"));
                cursor = at + 2;
                literal = cursor;
            }
            Some(b'{') => {
                let body_start = at + 2;
                let Some((rel_end, depth)) = closing_brace(&s[body_start..]) else {
                    push_literal(&mut pieces, &s[literal..at]);
                    pieces.push(Piece::Malformed {
                        spelling: &s[at..],
                        error: Syntax::Unterminated { offset: at },
                    });
                    cursor = s.len();
                    literal = cursor;
                    break;
                };
                let body = &s[body_start..body_start + rel_end];
                let after = body_start + rel_end + 1;
                if depth > MAX_NESTING {
                    push_literal(&mut pieces, &s[literal..at]);
                    pieces.push(Piece::Malformed {
                        spelling: &s[at..after],
                        error: Syntax::TooDeep { offset: at },
                    });
                    cursor = after;
                    literal = cursor;
                    continue;
                }
                if body.is_empty() {
                    push_literal(&mut pieces, &s[literal..at]);
                    pieces.push(Piece::Malformed {
                        spelling: &s[at..after],
                        error: Syntax::EmptyRef,
                    });
                    cursor = after;
                    literal = cursor;
                    continue;
                }
                push_literal(&mut pieces, &s[literal..at]);
                pieces.push(Piece::Ref(body));
                cursor = after;
                literal = cursor;
            }
            // A bare `$` is ordinary text.
            _ => cursor = at + 1,
        }
    }
    push_literal(&mut pieces, &s[literal..]);
    pieces
}

/// The offset of the `}` closing a body that starts at `body[0]`, and the
/// deepest nesting seen, skipping nested `${...}` and `$$` pairs.
fn closing_brace(body: &str) -> Option<(usize, usize)> {
    let bytes = body.as_bytes();
    let mut depth = 1;
    let mut deepest = 1;
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1)) {
            (b'$', Some(b'$')) => i += 2,
            (b'$', Some(b'{')) => {
                depth += 1;
                deepest = deepest.max(depth);
                i += 2;
            }
            (b'}', _) => {
                depth -= 1;
                if depth == 0 {
                    return Some((i, deepest));
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

fn push_literal<'a>(pieces: &mut Vec<Piece<'a>>, text: &'a str) {
    if !text.is_empty() {
        pieces.push(Piece::Literal(text));
    }
}

/// Renders a reference body back as `${body}`.
pub struct Spelled<'a>(pub &'a str);

impl fmt::Display for Spelled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${{{}}}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Piece<'_> {
        Piece::Literal(s)
    }

    fn re(s: &str) -> Piece<'_> {
        Piece::Ref(s)
    }

    fn malformed(spelling: &str, error: Syntax) -> Piece<'_> {
        Piece::Malformed { spelling, error }
    }

    #[test]
    fn a_string_without_a_dollar_scans_to_nothing() {
        assert_eq!(scan("plain text"), []);
        assert_eq!(scan(""), []);
    }

    #[test]
    fn a_whole_string_reference_is_one_piece() {
        assert_eq!(scan("${db.host}"), [re("db.host")]);
        assert_eq!(scan("${env:PORT}"), [re("env:PORT")]);
    }

    #[test]
    fn embedded_references_keep_their_surroundings() {
        assert_eq!(
            scan("http://${host}:${port}/health"),
            [
                lit("http://"),
                re("host"),
                lit(":"),
                re("port"),
                lit("/health"),
            ]
        );
    }

    #[test]
    fn adjacent_references_have_no_literal_between_them() {
        assert_eq!(scan("${a}${b}"), [re("a"), re("b")]);
    }

    #[test]
    fn dollar_dollar_is_a_literal_dollar() {
        assert_eq!(scan("$$"), [lit("$")]);
        assert_eq!(scan("$${a}"), [lit("$"), lit("{a}")]);
        assert_eq!(scan("a$$b"), [lit("a"), lit("$"), lit("b")]);
    }

    #[test]
    fn a_bare_dollar_is_ordinary_text() {
        assert_eq!(scan("USD $5"), [lit("USD $5")]);
        assert_eq!(scan("$"), [lit("$")]);
        assert_eq!(scan("$ {a}"), [lit("$ {a}")]);
        assert_eq!(scan("a$"), [lit("a$")]);
    }

    #[test]
    fn malformed_references_are_returned_as_pieces() {
        assert_eq!(
            scan("a ${b"),
            [
                lit("a "),
                malformed("${b", Syntax::Unterminated { offset: 2 })
            ]
        );
        assert_eq!(scan("${}"), [malformed("${}", Syntax::EmptyRef)]);
        assert_eq!(
            scan("${a${b}"),
            [malformed("${a${b}", Syntax::Unterminated { offset: 0 })]
        );
    }

    #[test]
    fn nested_references_keep_their_raw_body() {
        assert_eq!(scan("${a${b}}"), [re("a${b}")]);
        assert_eq!(scan("${a.${b}.c}"), [re("a.${b}.c")]);
        assert_eq!(scan("${${${a}}}"), [re("${${a}}")]);
        assert_eq!(scan("x${a.${b}}y"), [lit("x"), re("a.${b}"), lit("y")]);
    }

    #[test]
    fn nesting_is_limited() {
        let deep = |n: usize| format!("{}a{}", "${".repeat(n), "}".repeat(n));
        assert_eq!(scan(&deep(MAX_NESTING)).len(), 1);
        assert!(matches!(
            scan(&deep(MAX_NESTING)).as_slice(),
            [Piece::Ref(_)]
        ));
        let too_deep = deep(MAX_NESTING + 1);
        assert_eq!(
            scan(&too_deep),
            [malformed(&too_deep, Syntax::TooDeep { offset: 0 })]
        );
    }

    #[test]
    fn an_escaped_open_inside_a_body_does_not_nest() {
        assert_eq!(scan("${a$${b}"), [re("a$${b")]);
    }

    #[test]
    fn scanning_continues_around_malformed_references() {
        assert_eq!(
            scan("${before} ${} ${after}"),
            [
                re("before"),
                lit(" "),
                malformed("${}", Syntax::EmptyRef),
                lit(" "),
                re("after"),
            ]
        );
        assert_eq!(
            scan("${before} ${after"),
            [
                re("before"),
                lit(" "),
                malformed("${after", Syntax::Unterminated { offset: 10 }),
            ]
        );
    }

    #[test]
    fn non_ascii_literals_survive() {
        assert_eq!(
            scan("héllo ${who} ☃"),
            [lit("héllo "), re("who"), lit(" ☃")]
        );
    }

    #[test]
    fn spelling_round_trips_a_reference() {
        assert_eq!(Spelled("db.host").to_string(), "${db.host}");
    }
}
