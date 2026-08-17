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
/// [`KeyPath`](knf_dotted::KeyPath) split are resolution's business, not the
/// scanner's, and keeping them apart is what makes this a `find` loop rather
/// than a grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Literal(&'a str),
    Ref(&'a str),
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
    /// where the body is parsed as a key path.
    #[error("empty segment in reference `${{{body}}}`")]
    EmptySegment { body: String },
}

/// Splits `s` into literals and reference bodies.
///
/// Returns an **empty** vector when `s` contains no `$` at all — the common
/// case, and the caller's signal to leave the value alone rather than rebuild an
/// identical string.
///
/// `$$` yields a literal `$`; a `$` followed by anything else is ordinary text,
/// so `USD $5` needs no escaping.
pub fn scan(s: &str) -> Result<Vec<Piece<'_>>, Syntax> {
    if !s.contains('$') {
        return Ok(Vec::new());
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
                    return Err(Syntax::Unterminated { offset: at });
                };
                let body = &s[body_start..body_start + rel_end];
                if body.is_empty() {
                    return Err(Syntax::EmptyRef);
                }
                if body.contains("${") {
                    return Err(Syntax::Nested {
                        body: body.to_string(),
                    });
                }
                push_literal(&mut pieces, &s[literal..at]);
                pieces.push(Piece::Ref(body));
                cursor = body_start + rel_end + 1;
                literal = cursor;
            }
            // A bare `$`: ordinary text, and part of the pending literal.
            _ => cursor = at + 1,
        }
    }
    push_literal(&mut pieces, &s[literal..]);
    Ok(pieces)
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

    /// The empty result is load-bearing: it is how the resolver tells "nothing
    /// to do" from "all literal, rebuild it".
    #[test]
    fn a_string_without_a_dollar_scans_to_nothing() {
        assert_eq!(scan("plain text").unwrap(), []);
        assert_eq!(scan("").unwrap(), []);
    }

    #[test]
    fn a_whole_string_reference_is_one_piece() {
        assert_eq!(scan("${db.host}").unwrap(), [re("db.host")]);
        assert_eq!(scan("${env:PORT}").unwrap(), [re("env:PORT")]);
    }

    #[test]
    fn embedded_references_keep_their_surroundings() {
        assert_eq!(
            scan("http://${host}:${port}/health").unwrap(),
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
        assert_eq!(scan("${a}${b}").unwrap(), [re("a"), re("b")]);
    }

    #[test]
    fn dollar_dollar_is_a_literal_dollar() {
        assert_eq!(scan("$$").unwrap(), [lit("$")]);
        assert_eq!(scan("$${a}").unwrap(), [lit("$"), lit("{a}")]);
        assert_eq!(scan("a$$b").unwrap(), [lit("a"), lit("$"), lit("b")]);
    }

    /// Only `${` starts a reference, so prose and prices need no escaping.
    #[test]
    fn a_bare_dollar_is_ordinary_text() {
        assert_eq!(scan("USD $5").unwrap(), [lit("USD $5")]);
        assert_eq!(scan("$").unwrap(), [lit("$")]);
        assert_eq!(scan("$ {a}").unwrap(), [lit("$ {a}")]);
        assert_eq!(scan("a$").unwrap(), [lit("a$")]);
    }

    #[test]
    fn malformed_references_are_rejected() {
        assert_eq!(
            scan("a ${b").unwrap_err(),
            Syntax::Unterminated { offset: 2 }
        );
        assert_eq!(scan("${}").unwrap_err(), Syntax::EmptyRef);
        assert_eq!(
            scan("${a${b}}").unwrap_err(),
            Syntax::Nested {
                body: "a${b".to_string()
            }
        );
    }

    /// Multi-byte text must not shift the offsets a `${` is found at.
    #[test]
    fn non_ascii_literals_survive() {
        assert_eq!(
            scan("héllo ${who} ☃").unwrap(),
            [lit("héllo "), re("who"), lit(" ☃")]
        );
    }

    #[test]
    fn spelling_round_trips_a_reference() {
        assert_eq!(Spelled("db.host").to_string(), "${db.host}");
    }
}
