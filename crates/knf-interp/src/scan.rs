//! Splitting a string into literal text and reference bodies.
//!
//! Flat and non-recursive by design. This runs on [`Value::String`] leaves
//! *after* the document has been parsed, so it never meets a TOML literal-vs-basic
//! string, a multi-line string, or a JSON `\u` escape — those were resolved by
//! the format parser long before the merge, let alone this pass.
//!
//! [`Value::String`]: knf_core::Value::String

use std::fmt;

/// One span of a scanned string.
///
/// A `Ref` body is deliberately left unparsed: the `env:` prefix and the
/// [`RefPath`](knf_core::RefPath) split are resolution's business, not the
/// scanner's, and keeping them apart is what makes this a `find` loop rather
/// than a grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Literal(&'a str),
    Ref(&'a str),
    Malformed { spelling: &'a str, error: Syntax },
}

/// A malformed reference.
///
/// Carries the offending text or offset and nothing else — no key path (the
/// caller knows where it was reading) and no flag names.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Syntax {
    /// `${` with no `}` after it.
    #[error("unterminated `${{` at offset {offset}")]
    Unterminated { offset: usize },
    /// `${}` — a reference to nothing.
    #[error("empty reference `${{}}`")]
    EmptyRef,
    /// `${a${b}}`. Finding the end of a reference is `find('}')`, so a nested
    /// `${` has no reading that is not a guess.
    #[error("nested `${{` in `${{{body}}}`")]
    Nested { body: String },
    /// `${env:}` — the namespace with no variable after it. Raised downstream,
    /// where the prefix is recognised, but it is the same class of mistake.
    #[error("empty variable name in `${{env:}}`")]
    EmptyEnvName,
    /// `${a..b}` — a dotted path with an empty segment. Also raised downstream,
    /// where the body is parsed as a reference path.
    #[error("empty segment in reference `${{{body}}}`")]
    EmptySegment { body: String },
    /// `${servers[x]}` — a bracket step that is not an array index: empty,
    /// non-numeric, too big, or unclosed. Also raised downstream.
    #[error("malformed index in reference `${{{body}}}`")]
    BadIndex { body: String },
}

/// Splits `s` into literals and reference bodies.
///
/// Returns an **empty** vector when `s` contains no `$` at all — the common
/// case, and the caller's signal to leave the value alone rather than rebuild an
/// identical string.
///
/// `$$` yields a literal `$`; a `$` followed by anything else is ordinary text,
/// so `USD $5` needs no escaping.
///
/// Malformed references are returned as pieces rather than aborting the scan.
/// A delimited malformed reference is recoverable, so later references are
/// still found; an unterminated reference consumes the remainder of the string.
pub fn scan(s: &str) -> Vec<Piece<'_>> {
    if !s.contains('$') {
        return Vec::new();
    }

    let mut pieces = Vec::new();
    let mut cursor = 0; // where the next `$` is searched from
    let mut literal = 0; // start of the pending literal run

    while let Some(rel) = s[cursor..].find('$') {
        let at = cursor + rel;
        // `$` is ASCII, so `at + 1` is in bounds-or-None and a UTF-8
        // continuation byte can never equal `$` or `{`.
        match s.as_bytes().get(at + 1) {
            Some(b'$') => {
                push_literal(&mut pieces, &s[literal..at]);
                pieces.push(Piece::Literal("$"));
                cursor = at + 2;
                literal = cursor;
            }
            Some(b'{') => {
                let body_start = at + 2;
                let Some(rel_end) = s[body_start..].find('}') else {
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
                if body.contains("${") {
                    push_literal(&mut pieces, &s[literal..at]);
                    pieces.push(Piece::Malformed {
                        spelling: &s[at..after],
                        error: Syntax::Nested {
                            body: body.to_string(),
                        },
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
            // A bare `$`: ordinary text, and part of the pending literal.
            _ => cursor = at + 1,
        }
    }
    push_literal(&mut pieces, &s[literal..]);
    pieces
}

fn push_literal<'a>(pieces: &mut Vec<Piece<'a>>, text: &'a str) {
    if !text.is_empty() {
        pieces.push(Piece::Literal(text));
    }
}

/// The source spelling of a reference, for splicing back the text of a piece
/// that could not be resolved.
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

    /// The empty result is load-bearing: it is how the resolver tells "nothing
    /// to do" from "all literal, rebuild it".
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

    /// Adjacent references have no literal between them, which is exactly the
    /// case an off-by-one in the cursor would corrupt.
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

    /// Only `${` starts a reference, so prose and prices need no escaping.
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
            scan("${a${b}}"),
            [
                malformed(
                    "${a${b}",
                    Syntax::Nested {
                        body: "a${b".to_string()
                    }
                ),
                lit("}")
            ]
        );
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

    /// Multi-byte text must not shift the offsets a `${` is found at.
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
