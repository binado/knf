//! Splitting a string value into literal text and reference bodies.

use std::borrow::Cow;

/// What a string value holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scanned<'a> {
    /// No reference: the text, with each `$$` unescaped.
    Plain(Cow<'a, str>),
    /// Exactly one reference and nothing else: its raw body.
    Whole(&'a str),
    /// References, well formed or not, possibly among literal text.
    Template(Vec<Piece<'a>>),
}

/// One span of a [`Scanned::Template`]. `Ref` bodies are left unparsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Literal(&'a str),
    Ref(&'a str),
    Malformed(ScanError),
}

/// A reference the scanner cannot delimit or accept.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScanError {
    /// `${` with no `}` after it.
    #[error("unterminated `${{` at offset {offset}")]
    Unterminated { offset: usize },
    /// References nested deeper than 10 levels.
    #[error("references nested deeper than {MAX_NESTING} levels at offset {offset}")]
    TooDeep { offset: usize },
    /// `${}` — a reference to nothing.
    #[error("empty reference `${{}}`")]
    EmptyRef,
}

/// The deepest `${` nesting a reference may have; `${a}` is one level.
pub const MAX_NESTING: usize = 10;

/// Classifies `s`. A malformed reference becomes [`Piece::Malformed`]; an
/// unterminated one consumes the rest of the string.
pub fn scan(s: &str) -> Scanned<'_> {
    if !s.contains('$') {
        return Scanned::Plain(Cow::Borrowed(s));
    }
    let pieces = pieces(s);
    if let [Piece::Ref(body)] = pieces[..] {
        return Scanned::Whole(body);
    }
    let literals: Option<String> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Literal(text) => Some(*text),
            Piece::Ref(_) | Piece::Malformed(_) => None,
        })
        .collect();
    match literals {
        Some(text) => Scanned::Plain(Cow::Owned(text)),
        None => Scanned::Template(pieces),
    }
}

fn pieces(s: &str) -> Vec<Piece<'_>> {
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
                push_literal(&mut pieces, &s[literal..at]);
                let body_start = at + 2;
                let Some((rel_end, depth)) = closing_brace(&s[body_start..]) else {
                    pieces.push(Piece::Malformed(ScanError::Unterminated { offset: at }));
                    literal = s.len();
                    break;
                };
                let body = &s[body_start..body_start + rel_end];
                pieces.push(if depth > MAX_NESTING {
                    Piece::Malformed(ScanError::TooDeep { offset: at })
                } else if body.is_empty() {
                    Piece::Malformed(ScanError::EmptyRef)
                } else {
                    Piece::Ref(body)
                });
                cursor = body_start + rel_end + 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Piece<'_> {
        Piece::Literal(s)
    }

    fn re(s: &str) -> Piece<'_> {
        Piece::Ref(s)
    }

    fn malformed<'a>(error: ScanError) -> Piece<'a> {
        Piece::Malformed(error)
    }

    fn plain(s: &str) -> Scanned<'_> {
        Scanned::Plain(s.into())
    }

    fn template(s: &str) -> Vec<Piece<'_>> {
        match scan(s) {
            Scanned::Template(pieces) => pieces,
            other => panic!("{s}: {other:?}"),
        }
    }

    #[test]
    fn a_string_without_a_dollar_is_plain_and_borrowed() {
        for s in ["plain text", ""] {
            assert!(matches!(scan(s), Scanned::Plain(Cow::Borrowed(text)) if text == s));
        }
    }

    #[test]
    fn a_whole_string_reference_is_whole() {
        assert_eq!(scan("${db.host}"), Scanned::Whole("db.host"));
        assert_eq!(scan("${env:PORT}"), Scanned::Whole("env:PORT"));
    }

    #[test]
    fn embedded_references_keep_their_surroundings() {
        assert_eq!(
            template("http://${host}:${port}/health"),
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
        assert_eq!(template("${a}${b}"), [re("a"), re("b")]);
    }

    #[test]
    fn dollar_dollar_is_a_literal_dollar() {
        assert_eq!(scan("$$"), plain("$"));
        assert_eq!(scan("$${a}"), plain("${a}"));
        assert_eq!(scan("a$$b"), plain("a$b"));
        assert_eq!(template("$$${a}"), [lit("$"), re("a")]);
    }

    #[test]
    fn a_bare_dollar_is_ordinary_text() {
        for s in ["USD $5", "$", "$ {a}", "a$"] {
            assert_eq!(scan(s), plain(s));
        }
    }

    #[test]
    fn malformed_references_are_returned_as_pieces() {
        assert_eq!(
            template("a ${b"),
            [lit("a "), malformed(ScanError::Unterminated { offset: 2 })]
        );
        assert_eq!(template("${}"), [malformed(ScanError::EmptyRef)]);
        assert_eq!(
            template("${a${b}"),
            [malformed(ScanError::Unterminated { offset: 0 })]
        );
    }

    #[test]
    fn nested_references_keep_their_raw_body() {
        assert_eq!(scan("${a${b}}"), Scanned::Whole("a${b}"));
        assert_eq!(scan("${a.${b}.c}"), Scanned::Whole("a.${b}.c"));
        assert_eq!(scan("${${${a}}}"), Scanned::Whole("${${a}}"));
        assert_eq!(template("x${a.${b}}y"), [lit("x"), re("a.${b}"), lit("y")]);
    }

    #[test]
    fn nesting_is_limited() {
        let deep = |n: usize| format!("{}a{}", "${".repeat(n), "}".repeat(n));
        assert!(matches!(scan(&deep(MAX_NESTING)), Scanned::Whole(_)));
        assert_eq!(
            template(&deep(MAX_NESTING + 1)),
            [malformed(ScanError::TooDeep { offset: 0 })]
        );
    }

    #[test]
    fn an_escaped_open_inside_a_body_does_not_nest() {
        assert_eq!(scan("${a$${b}"), Scanned::Whole("a$${b"));
    }

    #[test]
    fn scanning_continues_around_malformed_references() {
        assert_eq!(
            template("${before} ${} ${after}"),
            [
                re("before"),
                lit(" "),
                malformed(ScanError::EmptyRef),
                lit(" "),
                re("after"),
            ]
        );
        assert_eq!(
            template("${before} ${after"),
            [
                re("before"),
                lit(" "),
                malformed(ScanError::Unterminated { offset: 10 }),
            ]
        );
    }

    #[test]
    fn non_ascii_literals_survive() {
        assert_eq!(
            template("héllo ${who} ☃"),
            [lit("héllo "), re("who"), lit(" ☃")]
        );
    }
}
