//! The path vocabulary: one step type, several spellings.
//!
//! [`Seg`] is the single step every path in the workspace is built from —
//! an object key or an array index — and [`render_path`] the one display for
//! a chain of them. Two spellings and one witness are built over `Vec<Seg>`,
//! each with its own constructor and its own predicate:
//!
//! - [`KeyPath`] is the *merge-side* spelling: a `Vec<Seg>` that is all
//!   [`Key`](Seg::Key), built only from text or key strings. It is the only
//!   path a user can write for a dotted grammar (`--set`, rule flags), so an
//!   index can never reach a consumer that takes one.
//! - [`RefPath`] is the *reference* spelling: dotted keys plus `[n]` steps,
//!   for a consumer that only reads a document.
//! - A bare `Vec<Seg>` is the *witness*: built by walking a document, never
//!   parsed, and free to hold [`Index`](Seg::Index) — a value can live inside
//!   an array, and an error must still be able to say so.
//!
//! [`PathLeaf`] parses `key.path=value`. The path is always typed; the leaf
//! type `V` is chosen by the caller. [`FromStr`] for [`PathLeaf<String>`] keeps
//! the RHS raw. The `json` feature parses that RHS as JSON into
//! [`serde_json::Value`], falling back to a string — a rule exported on its own
//! as [`json_or_string`], for callers that need the same typing without a path.
//!
//! [`KeyPath`] is the same path with no `=value` half, for callers that address
//! a location rather than assign to one.
//!
//! This crate knows nothing about files, the command line, or the document
//! being addressed — it never sees `knf_core::Value`, which is what keeps
//! lookups with the walkers. Provenance (`--set`, filenames) is the caller's
//! job.
//!
//! [`From<PathLeaf<V>>`](From) expands to a nested object: `server.port=8080` →
//! `{"server":{"port":8080}}`. There are deliberately no `Serialize`/
//! `Deserialize` impls — a `PathLeaf` is an expression, and serializing one
//! could reasonably mean either the string or the object, so callers pick
//! explicitly via [`Display`](fmt::Display) or the conversion.

use std::fmt;
use std::str::FromStr;

/// A leaf value addressed by a dotted path.
///
/// [`FromStr`] for [`PathLeaf<String>`] splits `key.path=value` and stores the
/// RHS as-is. The typed [`FromStr`] impl (behind `json`) parses that RHS as
/// JSON, falling back to a string: `port=8080` is a number, `name=foo` is a
/// string.
///
/// `Display` of a typed leaf is canonical — dotted path, `=`, compact JSON of
/// the leaf — so `name=foo` displays as `name="foo"`. [`FromStr`] ∘ [`Display`](fmt::Display)
/// preserves path and leaf, not the original spelling. [`PathLeaf<String>`]
/// displays the raw RHS.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLeaf<V> {
    path: KeyPath,
    leaf: V,
}

/// Why a `key.path=value` expression (or a programmatic constructor) was rejected.
///
/// Carries the dotted key when a segment is empty, and nothing else — no
/// `--set`, no filenames. Provenance is the caller's job.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// No `=` in the expression.
    #[error("expected KEY.PATH=VALUE")]
    MissingEquals,
    /// The path is empty (`=1`, or `new` with an empty vec).
    #[error("empty key")]
    EmptyKey,
    /// A path segment is empty (`a..b=1`, `.a=1`, `a.=1`).
    #[error("empty segment in key `{key}`")]
    EmptySegment {
        /// The dotted path that contained an empty segment.
        key: String,
    },
}

/// One step of a path into a document.
///
/// The single step vocabulary every path in the workspace is built from. Two
/// predicates are enforced over it, by two kinds of construction: [`KeyPath`]
/// wraps a `Vec<Seg>` that is by construction all [`Key`](Seg::Key) — the only
/// path dot-spelling grammars accept, so an index can never reach a consumer
/// that takes one — while a bare `Vec<Seg>` is the witness a walker builds by
/// descending a document, free to hold [`Index`](Seg::Index) because a value
/// can live inside an array.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Seg {
    /// An object key.
    Key(String),
    /// An array position.
    Index(usize),
}

/// Renders a witness path for display: `servers.primary.host`, `tags[0]`.
pub fn render_path(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

/// A validated dotted path, with no `=value` half.
///
/// A newtype over `Vec<Seg>` whose only constructors — [`new`](KeyPath::new)
/// from key strings and [`FromStr`] from dotted text — guarantee every segment
/// is a [`Seg::Key`]. No constructor accepts a ready-made `Vec<Seg>`, so an
/// [`Seg::Index`] is unrepresentable here: the all-key shape is a fact the
/// compiler carries, not a rule callers re-check.
///
/// The same path [`PathLeaf`] carries, for callers that address a location
/// rather than assign to one. [`PathLeaf::new`] is defined in terms of it, so
/// the empty-key and empty-segment rules exist in exactly one place.
///
/// [`ParseError::MissingEquals`] is shared but unreachable from
/// [`FromStr::from_str`]: there is no `=` to miss.
///
/// Dots separate segments, so a key containing a literal dot is not addressable
/// by a `KeyPath`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyPath {
    path: Vec<Seg>,
}

impl KeyPath {
    /// Build from key strings. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>) -> Result<Self, ParseError> {
        if path.is_empty() {
            return Err(ParseError::EmptyKey);
        }
        if path.iter().any(|k| k.is_empty()) {
            return Err(ParseError::EmptySegment {
                key: path.join("."),
            });
        }
        Ok(Self {
            path: path.into_iter().map(Seg::Key).collect(),
        })
    }

    /// The segments as steps. Always all [`Seg::Key`], by construction.
    pub fn segs(&self) -> &[Seg] {
        &self.path
    }

    /// The segments as steps, zero-copy. Always all [`Seg::Key`], by construction.
    pub fn into_segs(self) -> Vec<Seg> {
        self.path
    }

    /// The key strings, in order. `server.port` yields `"server"`, `"port"`.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.path.iter().map(|seg| match seg {
            Seg::Key(key) => key.as_str(),
            Seg::Index(_) => unreachable!("a KeyPath holds no Index segments"),
        })
    }

    /// The owned key strings, for consumers whose grammar is still strings —
    /// `knf-core`'s merge paths, the nested-object expansion in `json.rs`.
    pub(crate) fn into_keys(self) -> Vec<String> {
        self.path
            .into_iter()
            .map(|seg| match seg {
                Seg::Key(key) => key,
                Seg::Index(_) => unreachable!("a KeyPath holds no Index segments"),
            })
            .collect()
    }
}

impl FromStr for KeyPath {
    type Err = ParseError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        Self::new(split_key(key)?)
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, key) in self.keys().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            f.write_str(key)?;
        }
        Ok(())
    }
}

/// Why a reference body was rejected.
///
/// Carries the body text and nothing else — no `${...}`, no `--interpolate`,
/// no filenames. Provenance is the caller's job.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    /// The reference is empty.
    #[error("empty key")]
    EmptyKey,
    /// A key segment is empty (`a..b`, `.a`, `[0].a` — no key before the step).
    #[error("empty segment in reference `{reference}`")]
    EmptySegment {
        /// The reference body that contained the empty segment.
        reference: String,
    },
    /// A bracket step is malformed: empty, not a number, too big, or unclosed.
    #[error("malformed index in reference `{reference}`")]
    BadIndex {
        /// The reference body that contained the bad bracket.
        reference: String,
    },
}

/// A reference target: dotted keys plus bracket array indices, `a.b[2].c`.
///
/// The second spelling over [`Seg`], for a consumer that only *reads* a
/// document: a `${...}` body may name an array element, where indexing into a
/// merge is a different question entirely — [`KeyPath`] stays keys-only for
/// exactly that split. Built only from text; there is no bulk constructor, so
/// an empty path or an empty key segment is unrepresentable.
///
/// `Display` is [`render_path`]. Not injective with respect to document keys:
/// a key literally spelled `a[0]` exists, but a reference can no longer name
/// it — the same accepted loss as keys containing a literal dot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefPath {
    path: Vec<Seg>,
}

impl RefPath {
    /// The segments as steps.
    pub fn segs(&self) -> &[Seg] {
        &self.path
    }

    /// The segments as steps, zero-copy.
    pub fn into_segs(self) -> Vec<Seg> {
        self.path
    }
}

impl FromStr for RefPath {
    type Err = RefError;

    /// Grammar: a leading key, then any mix of `.key` and `[N]` steps, where
    /// a key is a run of anything but `.[]` and `N` a bare `usize`.
    fn from_str(body: &str) -> Result<Self, Self::Err> {
        if body.is_empty() {
            return Err(RefError::EmptyKey);
        }
        let bad_index = || RefError::BadIndex {
            reference: body.to_string(),
        };
        let empty_segment = || RefError::EmptySegment {
            reference: body.to_string(),
        };

        let bytes = body.as_bytes();
        let mut path = Vec::new();
        let mut cursor = 0;

        // The first step must be a key: there is no `[0]` into the root.
        let (key, next) = take_key(body, cursor);
        if key.is_empty() {
            // `.a` and `[0].a` open with no key; a stray `]` opens with none.
            return Err(if bytes[cursor] == b']' {
                bad_index()
            } else {
                empty_segment()
            });
        }
        path.push(Seg::Key(key));
        cursor = next;

        while cursor < body.len() {
            match bytes[cursor] {
                b'.' => {
                    let (key, next) = take_key(body, cursor + 1);
                    if key.is_empty() {
                        return Err(empty_segment());
                    }
                    path.push(Seg::Key(key));
                    cursor = next;
                }
                b'[' => {
                    let digits_start = cursor + 1;
                    let mut end = digits_start;
                    while end < body.len() && bytes[end].is_ascii_digit() {
                        end += 1;
                    }
                    if end == digits_start || bytes.get(end) != Some(&b']') {
                        return Err(bad_index());
                    }
                    let index: usize = body[digits_start..end].parse().map_err(|_| bad_index())?;
                    path.push(Seg::Index(index));
                    cursor = end + 1;
                }
                // A key run ends at `.[` or `]`; anything left over is a stray
                // bracket.
                _ => return Err(bad_index()),
            }
        }

        Ok(Self { path })
    }
}

/// A maximal run containing none of `.[]` — the delimiters are ASCII, so byte
/// scanning never splits a multibyte key.
fn take_key(body: &str, from: usize) -> (String, usize) {
    let mut end = from;
    while end < body.len() && !matches!(body.as_bytes()[end], b'.' | b'[' | b']') {
        end += 1;
    }
    (body[from..end].to_string(), end)
}

impl fmt::Display for RefPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render_path(&self.path))
    }
}

impl<V> PathLeaf<V> {
    /// Build from path segments and a leaf. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>, leaf: V) -> Result<Self, ParseError> {
        Ok(Self {
            path: KeyPath::new(path)?,
            leaf,
        })
    }

    /// The path as steps: all [`Seg::Key`], like the [`KeyPath`] it wraps.
    pub fn path(&self) -> &[Seg] {
        self.path.segs()
    }

    /// The key strings, in order. `server.port` yields `"server"`, `"port"`.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.path.keys()
    }

    /// The RHS value, not yet wrapped in nested objects.
    pub fn leaf(&self) -> &V {
        &self.leaf
    }

    /// Replace the leaf, keeping the path. The path is already valid, so this
    /// cannot fail the way [`new`](Self::new) can.
    pub fn map_leaf<T>(self, f: impl FnOnce(V) -> T) -> PathLeaf<T> {
        PathLeaf {
            path: self.path,
            leaf: f(self.leaf),
        }
    }

    /// [`map_leaf`](Self::map_leaf) when the conversion can fail.
    pub fn try_map_leaf<T, E>(self, f: impl FnOnce(V) -> Result<T, E>) -> Result<PathLeaf<T>, E> {
        Ok(PathLeaf {
            path: self.path,
            leaf: f(self.leaf)?,
        })
    }

    #[cfg(feature = "json")]
    fn into_nested(self, nest: impl Fn(String, V) -> V) -> V {
        self.path
            .into_keys()
            .into_iter()
            .rev()
            .fold(self.leaf, |acc, key| nest(key, acc))
    }
}

impl FromStr for PathLeaf<String> {
    type Err = ParseError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        let (path, rhs) = split_expr(expr)?;
        Self::new(path, rhs.to_string())
    }
}

impl fmt::Display for PathLeaf<String> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.path, self.leaf)
    }
}

fn split_expr(expr: &str) -> Result<(Vec<String>, &str), ParseError> {
    // Split on the first `=` so the RHS may contain more of them.
    let Some((lhs, rhs)) = expr.split_once('=') else {
        return Err(ParseError::MissingEquals);
    };
    Ok((split_key(lhs)?, rhs))
}

/// Splits a dotted key into segments. `""` is an empty key rather than one
/// empty segment, which is the more useful of the two messages.
fn split_key(key: &str) -> Result<Vec<String>, ParseError> {
    if key.is_empty() {
        return Err(ParseError::EmptyKey);
    }
    Ok(key.split('.').map(str::to_string).collect())
}

#[cfg(feature = "json")]
mod json;

#[cfg(feature = "json")]
pub use json::json_or_string;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_expressions() {
        for (bad, want) in [
            ("noequals", ParseError::MissingEquals),
            ("=1", ParseError::EmptyKey),
            ("a..b=1", ParseError::EmptySegment { key: "a..b".into() }),
            (".a=1", ParseError::EmptySegment { key: ".a".into() }),
            ("a.=1", ParseError::EmptySegment { key: "a.".into() }),
        ] {
            assert_eq!(bad.parse::<PathLeaf<String>>().unwrap_err(), want, "{bad}");
        }
    }

    #[test]
    fn new_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            PathLeaf::<String>::new(vec![], "1".into()).unwrap_err(),
            ParseError::EmptyKey
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["".into()], "1".into()).unwrap_err(),
            ParseError::EmptySegment { key: "".into() }
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["a".into(), "".into()], "1".into()).unwrap_err(),
            ParseError::EmptySegment { key: "a.".into() }
        );
    }

    #[test]
    fn raw_fromstr_keeps_the_rhs_unparsed() {
        let path_leaf: PathLeaf<String> = "port=8080".parse().unwrap();
        assert_eq!(path_leaf.path(), [Seg::Key("port".into())]);
        assert!(path_leaf.keys().eq(["port"]));
        assert_eq!(path_leaf.leaf(), "8080");
        assert_eq!(path_leaf.to_string(), "port=8080");
    }

    /// The rules live in `KeyPath::new`, so both types reject the same spellings.
    #[test]
    fn key_path_rejects_what_path_leaf_rejects() {
        for (bad, want) in [
            ("", ParseError::EmptyKey),
            ("a..b", ParseError::EmptySegment { key: "a..b".into() }),
            (".a", ParseError::EmptySegment { key: ".a".into() }),
            ("a.", ParseError::EmptySegment { key: "a.".into() }),
        ] {
            assert_eq!(bad.parse::<KeyPath>().unwrap_err(), want, "{bad}");
        }
        assert_eq!(KeyPath::new(vec![]).unwrap_err(), ParseError::EmptyKey);
    }

    #[test]
    fn key_path_round_trips_through_display() {
        let key: KeyPath = "db.plugins".parse().unwrap();
        assert_eq!(
            key.segs(),
            [Seg::Key("db".into()), Seg::Key("plugins".into())]
        );
        assert!(key.keys().eq(["db", "plugins"]));
        assert_eq!(key.to_string(), "db.plugins");
        assert_eq!(
            key.into_segs(),
            [Seg::Key("db".into()), Seg::Key("plugins".into())]
        );
    }

    /// A `=` has no special meaning without a leaf to assign, so it is just part
    /// of a (weird) key rather than a `MissingEquals`-shaped hole.
    #[test]
    fn key_path_has_no_equals_half() {
        let key: KeyPath = "a=b".parse().unwrap();
        assert!(key.keys().eq(["a=b"]));
    }

    /// A witness may mix keys and indices; a `KeyPath` never can.
    #[test]
    fn render_path_mixes_keys_and_indices() {
        assert_eq!(render_path(&[]), "");
        assert_eq!(
            render_path(&[Seg::Key("a".into()), Seg::Key("b".into())]),
            "a.b"
        );
        assert_eq!(
            render_path(&[Seg::Key("tags".into()), Seg::Index(0)]),
            "tags[0]"
        );
    }

    fn seg(key: &str) -> Seg {
        Seg::Key(key.into())
    }

    #[test]
    fn ref_path_parses_mixed_steps() {
        let cases: &[(&str, &[Seg])] = &[
            ("a", &[seg("a")]),
            ("a.b", &[seg("a"), seg("b")]),
            ("a[0]", &[seg("a"), Seg::Index(0)]),
            ("a.b[2].c", &[seg("a"), seg("b"), Seg::Index(2), seg("c")]),
            ("a[0][1]", &[seg("a"), Seg::Index(0), Seg::Index(1)]),
            // A `:` is an ordinary key character; namespaces are the caller's.
            ("a:b[3]", &[seg("a:b"), Seg::Index(3)]),
        ];
        for (body, want) in cases {
            let parsed: RefPath = body.parse().unwrap();
            assert_eq!(parsed.segs(), *want, "{body}");
        }
    }

    #[test]
    fn ref_path_rejects_malformed_bodies() {
        let empty = RefError::EmptySegment {
            reference: String::new(),
        };
        let bad = RefError::BadIndex {
            reference: String::new(),
        };
        let cases: &[(&str, &str)] = &[
            (".a", "empty"),
            ("a..b", "empty"),
            ("a[0]..b", "empty"),
            ("[0].a", "empty"),
            ("a[]", "bad"),
            ("a[x]", "bad"),
            ("a[1", "bad"),
            ("a[-1]", "bad"),
            ("a]", "bad"),
            ("a[99999999999999999999999999]", "bad"),
        ];
        for (body, kind) in cases {
            let err = body.parse::<RefPath>().unwrap_err();
            match (*kind, err) {
                ("empty", RefError::EmptySegment { reference }) => {
                    assert_eq!(reference, *body, "{body}")
                }
                ("bad", RefError::BadIndex { reference }) => {
                    assert_eq!(reference, *body, "{body}")
                }
                other => panic!("{body}: expected {empty:?}/{bad:?} shape, got {other:?}"),
            }
        }
        assert_eq!("".parse::<RefPath>().unwrap_err(), RefError::EmptyKey);
        let _ = (empty, bad); // shapes documented above
    }

    #[test]
    fn ref_path_displays_as_rendered_witness() {
        let parsed: RefPath = "a.b[2].c".parse().unwrap();
        assert_eq!(parsed.to_string(), "a.b[2].c");
        // The all-key subset displays exactly as a KeyPath would.
        assert_eq!("a.b".parse::<RefPath>().unwrap().to_string(), "a.b");
    }

    #[test]
    fn map_leaf_preserves_the_path() {
        let path_leaf = PathLeaf::new(vec!["a".into()], "xy".to_string())
            .unwrap()
            .map_leaf(|s| s.len());
        assert!(path_leaf.keys().eq(["a"]));
        assert_eq!(*path_leaf.leaf(), 2);
    }
}
