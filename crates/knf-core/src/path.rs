//! Paths into a document: [`Seg`] steps, the parsed [`RefPath`] (`a.b[2].c`),
//! and their renderings.
//!
//! Writers take keys only; [`RefPath::try_into_keys`] rejects index steps.
//! Merge selectors use [`crate::glob`] instead.

use std::fmt;
use std::str::FromStr;

/// Why a path expression was rejected. Carries only the path text.
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
    /// A path given to a writer contains an array index.
    #[error("`{path}` contains an array index; merge paths take keys only")]
    IndexInKeyPath {
        /// The full path, rendered.
        path: String,
    },
}

/// One step of a path into a document.
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

/// A non-empty parsed path: dotted keys plus bracket indices, `a.b[2].c`.
///
/// Keys containing `.`, `[` or `]` are not addressable.
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

    /// The keys, or an error if any step is an index.
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

    /// Grammar: a key, then any mix of `.key` and `[N]` steps.
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

        // The first step must be a key.
        let (key, next) = take_key(body, cursor);
        if key.is_empty() {
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
                // A stray `]`.
                _ => return Err(bad_index()),
            }
        }

        Ok(Self { path })
    }
}

/// The longest run from `from` containing none of `.[]`.
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

#[cfg(test)]
mod tests {
    use super::*;

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
            // `:` is an ordinary key character.
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

    #[test]
    fn equals_is_an_ordinary_key_character() {
        let parsed: RefPath = "a=b".parse().unwrap();
        assert_eq!(parsed.try_into_keys().unwrap(), ["a=b"]);
    }
}
