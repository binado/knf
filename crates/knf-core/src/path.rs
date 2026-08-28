//! The path vocabulary: one step type, one parsed spelling, one witness.
//!
//! [`Seg`] is the single step every path in the workspace is built from — an
//! object key or an array index — and [`render_path`] the one display for a
//! chain of them. Over `Vec<Seg>` there is one parsed spelling and one witness:
//!
//! - [`RefPath`] parses `a.b[2].c` — dotted keys plus `[n]` steps. It is the
//!   only spelling a `${...}` reference uses, and the only grammar the
//!   command-line path flags parse.
//! - A bare `Vec<Seg>` is the *witness*: built by walking a document, never
//!   parsed, and free to hold [`Index`](Seg::Index) — a value can live inside
//!   an array, and an error must still be able to say so.
//!
//! Writers take keys only. Reading an array element has one obvious meaning;
//! writing one conflicts with arrays replacing wholesale on the merge side, so
//! an [`Index`](Seg::Index) step can never address a merge location. That
//! predicate is enforced once, at the boundary, by [`RefPath::try_into_keys`] —
//! a path that reaches a writer has been through it, and
//! [`IndexInKeyPath`](PathError::IndexInKeyPath) is the failure. A key
//! literally spelled `a[0]` is consequently unwritable from the command line
//! and unreferenceable from `${...}` — the same accepted loss as keys
//! containing a literal dot, which the dotted grammar has always split.
//!
//! Parsing is pure text: nothing here reads a [`Value`](crate::Value), and the
//! walkers that do — `knf-interp`'s `lookup`, this crate's `merge_at` — build
//! or consume these types rather than living in them. Provenance is the
//! caller's job too: a [`PathError`] carries the path text and never a flag
//! name or a filename.
//!
//! Two renderers, because there are two representations and they disagree
//! about the empty case. [`render_path`] takes a witness and renders nothing
//! for an empty one — a [`RefPath`] cannot be empty, so the case never reaches
//! it from a parse. [`render_keys`] takes the merge side's `&[String]` key
//! paths, where empty is reachable and means the document root.

use std::fmt;
use std::str::FromStr;

/// Why a path expression was rejected.
///
/// Carries the path text and nothing else — no `--set`, no `--interpolate`,
/// no filenames. Provenance is the caller's job.
///
/// [`IndexInKeyPath`](PathError::IndexInKeyPath) is raised by
/// [`RefPath::try_into_keys`] alone, never by parsing.
/// [`MissingEquals`](PathError::MissingEquals) has no producer in this crate at
/// all — a bare [`RefPath`] has no `=` to miss. It belongs to `knf-config`'s
/// `PathLeaf`, which parses `key.path=value` over this grammar and shares this
/// error rather than wrapping it, so one spelling of a bad path reads the same
/// wherever it was typed.
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

/// Renders a key path for display. An empty path is the document root.
///
/// The merge side's paths are `&[String]`: keys only, by the invariant
/// [`RefPath::try_into_keys`] enforces, and reachable empty because the fold
/// starts at the root. [`render_path`] is the same rendering over a witness
/// that may hold indices.
pub(crate) fn render_keys(path: &[String]) -> String {
    if path.is_empty() {
        "<root>".to_string()
    } else {
        path.join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
