//! The path vocabulary: one step type, one parsed spelling, one witness.
//!
//! [`Seg`] is the single step every path in the workspace is built from —
//! an object key or an array index — and [`render_path`] the one display for
//! a chain of them. Over `Vec<Seg>` there is one parsed spelling and one
//! witness:
//!
//! - [`RefPath`] parses `a.b[2].c` — dotted keys plus `[n]` steps. It is the
//!   only spelling a `${...}` reference uses, and the only grammar the
//!   command-line path flags parse.
//! - A bare `Vec<Seg>` is the *witness*: built by walking a document, never
//!   parsed, and free to hold [`Index`](Seg::Index) — a value can live inside
//!   an array, and an error must still be able to say so.
//!
//! Writers take keys only. Reading an array element has one obvious meaning;
//! writing one conflicts with arrays replacing wholesale on the merge side,
//! so an [`Index`](Seg::Index) step can never address a merge location. That
//! predicate is enforced once, at the boundary, by
//! [`RefPath::try_into_keys`] — a path that reaches a writer has been through
//! it, and [`IndexInKeyPath`](PathError::IndexInKeyPath) is the failure. A
//! key literally spelled `a[0]` is consequently unwritable from the command
//! line and unreferenceable from `${...}` — the same accepted loss as keys
//! containing a literal dot, which the dotted grammar has always split.
//!
//! [`PathLeaf`] parses `key.path=value`. The path is always typed; the leaf
//! type `V` is chosen by the caller. [`FromStr`] for [`PathLeaf<String>`] keeps
//! the RHS raw. The `json` feature parses that RHS as JSON into
//! [`serde_json::Value`], falling back to a string — a rule exported on its own
//! as [`json_or_string`], for callers that need the same typing without a path.
//!
//! This crate knows nothing about files, the command line, or the document
//! being addressed — it never sees `knf_core::Value`, which is what keeps
//! lookups with the walkers. Provenance (`--set`, filenames) is the caller's
//! job.
//!
//! [`TryFrom<PathLeaf<V>>`](TryFrom) expands to a nested object:
//! `server.port=8080` → `{"server":{"port":8080}}`. There are deliberately no
//! `Serialize`/`Deserialize` impls — a `PathLeaf` is an expression, and
//! serializing one could reasonably mean either the string or the object, so
//! callers pick explicitly via [`Display`](fmt::Display) or the conversion.

use std::fmt;
use std::str::FromStr;

/// A leaf value addressed by a parsed path.
///
/// [`FromStr`] for [`PathLeaf<String>] splits `key.path=value`, parses the LHS
/// as a [`RefPath`], and stores the RHS as-is. The typed [`FromStr`] impl
/// (behind `json`) parses that RHS as JSON, falling back to a string:
/// `port=8080` is a number, `name=foo` is a string.
///
/// The grammar accepts bracket steps — `a[0]=1` parses — because it is the
/// one spelling references also use. Whether such a path may *write* is
/// [`RefPath::try_into_keys`]' question, asked at conversion time.
///
/// `Display` of a typed leaf is canonical — path, `=`, compact JSON of the
/// leaf — so `name=foo` displays as `name="foo"`. [`FromStr`] ∘ [`Display`](fmt::Display)
/// preserves path and leaf, not the original spelling. [`PathLeaf<String>`]
/// displays the raw RHS.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLeaf<V> {
    path: RefPath,
    leaf: V,
}

/// Why a path expression was rejected.
///
/// Carries the path text and nothing else — no `--set`, no `--interpolate`,
/// no filenames. Provenance is the caller's job.
///
/// [`MissingEquals`](PathError::MissingEquals) is only reachable from
/// [`PathLeaf`]'s parser — a bare [`RefPath`] has no `=` to miss — and
/// [`IndexInKeyPath`](PathError::IndexInKeyPath) only from
/// [`RefPath::try_into_keys`], never from parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// No `=` in a `key.path=value` expression.
    #[error("expected KEY.PATH=VALUE")]
    MissingEquals,
    /// The path is empty (`=1`, an empty `${...}` body, an empty vec).
    #[error("empty path")]
    EmptyPath,
    /// A path segment is empty (`a..b`, `.a`, `a.`).
    #[error("empty segment in path `{path}`")]
    EmptySegment {
        /// The path that contained an empty segment.
        path: String,
    },
    /// A bracket step is malformed: empty, not a number, too big, or unclosed.
    #[error("malformed index in path `{path}`")]
    BadIndex {
        /// The path that contained the bad bracket.
        path: String,
    },
    /// A path that arrived at a writer contained an array index.
    ///
    /// Raised by [`RefPath::try_into_keys`] only, never by parsing: writers
    /// take keys one per segment because arrays replace wholesale on the
    /// merge side, so there is no meaning to writing element `n`.
    #[error("`{path}` contains an array index; merge paths take keys only")]
    IndexInKeyPath {
        /// The full path, rendered.
        path: String,
    },
}

/// One step of a path into a document.
///
/// The single step vocabulary every path in the workspace is built from.
/// [`RefPath`] parses a chain of these from text; a bare `Vec<Seg>` is the
/// witness a walker builds by descending a document, free to hold
/// [`Index`](Seg::Index) because a value can live inside an array.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Seg {
    /// An object key.
    Key(String),
    /// An array position.
    Index(usize),
}

/// Renders a path for display: `servers.primary.host`, `tags[0]`.
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

/// A parsed path: dotted keys plus bracket array indices, `a.b[2].c`.
///
/// The one spelling over [`Seg`]. References parse it directly; the
/// command-line path flags parse it too, and their write-side callers then
/// run [`try_into_keys`](RefPath::try_into_keys), which rejects any
/// [`Index`](Seg::Index) step — reading an array element has one obvious
/// meaning, writing one does not. Empty paths and empty segments are
/// unrepresentable: both [`from_str`](RefPath::from_str) and
/// [`from_keys`](RefPath::from_keys) reject them.
///
/// `Display` is [`render_path`]. Not injective with respect to document keys:
/// a key literally spelled `a[0]` exists, but this grammar reads it as a key
/// and an index — the same accepted loss as keys containing a literal dot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefPath {
    path: Vec<Seg>,
}

impl RefPath {
    /// Build from key strings. Rejects an empty path or any empty segment.
    pub fn from_keys(keys: Vec<String>) -> Result<Self, PathError> {
        if keys.is_empty() {
            return Err(PathError::EmptyPath);
        }
        if keys.iter().any(|k| k.is_empty()) {
            return Err(PathError::EmptySegment {
                path: keys.join("."),
            });
        }
        Ok(Self {
            path: keys.into_iter().map(Seg::Key).collect(),
        })
    }

    /// The segments as steps.
    pub fn segs(&self) -> &[Seg] {
        &self.path
    }

    /// The segments as steps, zero-copy.
    pub fn into_segs(self) -> Vec<Seg> {
        self.path
    }

    /// The owned key strings, checked all-key.
    ///
    /// The one write-side predicate: a caller that assigns into a document
    /// takes keys one per segment, so an [`Index`](Seg::Index) step fails
    /// here rather than at the merge — before any file is read, and with the
    /// whole path rendered into the error.
    pub fn try_into_keys(self) -> Result<Vec<String>, PathError> {
        if self.path.iter().any(|seg| matches!(seg, Seg::Index(_))) {
            return Err(PathError::IndexInKeyPath {
                path: render_path(&self.path),
            });
        }
        Ok(self
            .path
            .into_iter()
            .map(|seg| match seg {
                Seg::Key(key) => key,
                Seg::Index(_) => unreachable!("index steps were rejected above"),
            })
            .collect())
    }
}

impl FromStr for RefPath {
    type Err = PathError;

    /// Grammar: a leading key, then any mix of `.key` and `[N]` steps, where
    /// a key is a run of anything but `.[]` and `N` a bare `usize`.
    fn from_str(body: &str) -> Result<Self, Self::Err> {
        if body.is_empty() {
            return Err(PathError::EmptyPath);
        }
        let bad_index = || PathError::BadIndex {
            path: body.to_string(),
        };
        let empty_segment = || PathError::EmptySegment {
            path: body.to_string(),
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
    pub fn new(path: Vec<String>, leaf: V) -> Result<Self, PathError> {
        Ok(Self {
            path: RefPath::from_keys(path)?,
            leaf,
        })
    }

    /// The path as steps. May hold [`Index`](Seg::Index) steps from the
    /// grammar — consumers that write run
    /// [`try_into_keys`](RefPath::try_into_keys).
    pub fn path(&self) -> &[Seg] {
        self.path.segs()
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
    pub(crate) fn try_into_nested(self, nest: impl Fn(String, V) -> V) -> Result<V, PathError> {
        let keys = self.path.try_into_keys()?;
        Ok(keys
            .into_iter()
            .rev()
            .fold(self.leaf, |acc, key| nest(key, acc)))
    }
}

impl FromStr for PathLeaf<String> {
    type Err = PathError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        // Split on the first `=` so the RHS may contain more of them.
        let Some((lhs, rhs)) = expr.split_once('=') else {
            return Err(PathError::MissingEquals);
        };
        Ok(Self {
            path: lhs.parse()?,
            leaf: rhs.to_string(),
        })
    }
}

impl fmt::Display for PathLeaf<String> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.path, self.leaf)
    }
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
            ("noequals", PathError::MissingEquals),
            ("=1", PathError::EmptyPath),
            (
                "a..b=1",
                PathError::EmptySegment {
                    path: "a..b".into(),
                },
            ),
            (".a=1", PathError::EmptySegment { path: ".a".into() }),
            ("a.=1", PathError::EmptySegment { path: "a.".into() }),
            ("a[]=1", PathError::BadIndex { path: "a[]".into() }),
        ] {
            assert_eq!(bad.parse::<PathLeaf<String>>().unwrap_err(), want, "{bad}");
        }
    }

    #[test]
    fn new_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            PathLeaf::<String>::new(vec![], "1".into()).unwrap_err(),
            PathError::EmptyPath
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["".into()], "1".into()).unwrap_err(),
            PathError::EmptySegment { path: "".into() }
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["a".into(), "".into()], "1".into()).unwrap_err(),
            PathError::EmptySegment { path: "a.".into() }
        );
    }

    #[test]
    fn raw_fromstr_keeps_the_rhs_unparsed() {
        let path_leaf: PathLeaf<String> = "port=8080".parse().unwrap();
        assert_eq!(path_leaf.path(), [Seg::Key("port".into())]);
        assert_eq!(path_leaf.leaf(), "8080");
        assert_eq!(path_leaf.to_string(), "port=8080");
    }

    /// The grammar accepts bracket steps; only writers reject them.
    #[test]
    fn path_leaf_accepts_brackets_its_writers_reject() {
        let parsed: PathLeaf<String> = "a[0]=1".parse().unwrap();
        assert_eq!(parsed.path(), [Seg::Key("a".into()), Seg::Index(0)]);
        assert_eq!(parsed.to_string(), "a[0]=1");
    }

    /// A witness may mix keys and indices.
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
        let cases: &[(&str, PathError)] = &[
            (".a", empty_segment(".a")),
            ("a..b", empty_segment("a..b")),
            ("a[0]..b", empty_segment("a[0]..b")),
            ("[0].a", empty_segment("[0].a")),
            ("a[]", bad_index("a[]")),
            ("a[x]", bad_index("a[x]")),
            ("a[1", bad_index("a[1")),
            ("a[-1]", bad_index("a[-1]")),
            ("a]", bad_index("a]")),
            (
                "a[99999999999999999999999999]",
                bad_index("a[99999999999999999999999999]"),
            ),
        ];
        for (body, want) in cases {
            assert_eq!(&body.parse::<RefPath>().unwrap_err(), want, "{body}");
        }
        assert_eq!("".parse::<RefPath>().unwrap_err(), PathError::EmptyPath);
    }

    fn empty_segment(path: &str) -> PathError {
        PathError::EmptySegment { path: path.into() }
    }

    fn bad_index(path: &str) -> PathError {
        PathError::BadIndex { path: path.into() }
    }

    #[test]
    fn ref_path_displays_as_rendered_witness() {
        let parsed: RefPath = "a.b[2].c".parse().unwrap();
        assert_eq!(parsed.to_string(), "a.b[2].c");
        assert_eq!("a.b".parse::<RefPath>().unwrap().to_string(), "a.b");
    }

    #[test]
    fn from_keys_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            RefPath::from_keys(vec![]).unwrap_err(),
            PathError::EmptyPath
        );
        assert_eq!(
            RefPath::from_keys(vec!["".into()]).unwrap_err(),
            PathError::EmptySegment { path: "".into() }
        );
        assert_eq!(
            RefPath::from_keys(vec!["a".into(), "".into()]).unwrap_err(),
            PathError::EmptySegment { path: "a.".into() }
        );
        // from_keys and parsing agree on the dotted all-key grammar.
        assert_eq!(
            RefPath::from_keys(vec!["db".into(), "plugins".into()]).unwrap(),
            "db.plugins".parse().unwrap()
        );
    }

    #[test]
    fn try_into_keys_passes_all_key_paths_whole() {
        let parsed: RefPath = "db.plugins".parse().unwrap();
        assert_eq!(parsed.try_into_keys().unwrap(), ["db", "plugins"]);
    }

    #[test]
    fn try_into_keys_rejects_index_steps_with_the_full_path() {
        for (indexed, want) in [("a[0]", "a[0]"), ("a.b[2].c", "a.b[2].c")] {
            let err = indexed
                .parse::<RefPath>()
                .unwrap()
                .try_into_keys()
                .unwrap_err();
            assert_eq!(err, PathError::IndexInKeyPath { path: want.into() });
        }
        let err = "a[0]"
            .parse::<RefPath>()
            .unwrap()
            .try_into_keys()
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`a[0]` contains an array index; merge paths take keys only"
        );
    }

    /// A `=` has no special meaning in a bare path: just part of a (weird)
    /// key rather than a `MissingEquals`-shaped hole.
    #[test]
    fn equals_is_an_ordinary_key_character() {
        let parsed: RefPath = "a=b".parse().unwrap();
        assert_eq!(parsed.try_into_keys().unwrap(), ["a=b"]);
    }

    #[test]
    fn map_leaf_preserves_the_path() {
        let path_leaf = PathLeaf::new(vec!["a".into()], "xy".to_string())
            .unwrap()
            .map_leaf(|s| s.len());
        assert_eq!(path_leaf.path(), [Seg::Key("a".into())]);
        assert_eq!(*path_leaf.leaf(), 2);
    }
}
