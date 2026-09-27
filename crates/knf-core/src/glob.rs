//! Validated glob predicates and quoted, dotted key-path selectors.

use std::str::FromStr;

/// An invalid glob pattern, with a byte offset into its source.
pub use fast_glob::Error as GlobError;

/// A case-sensitive byte glob, validated once before matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobPattern(Vec<u8>);

impl FromStr for GlobPattern {
    type Err = GlobError;

    fn from_str(pattern: &str) -> Result<Self, Self::Err> {
        Self::from_bytes(pattern.as_bytes().to_vec())
    }
}

impl GlobPattern {
    fn from_bytes(pattern: Vec<u8>) -> Result<Self, GlobError> {
        fast_glob::validate(&pattern)?;
        Ok(Self(pattern))
    }

    /// Match the entire candidate. `?` matches one byte, not one Unicode character.
    pub fn matches(&self, candidate: impl AsRef<[u8]>) -> bool {
        fast_glob::glob_match(&self.0, candidate)
    }
}

/// Why a quoted key-path glob was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyGlobError {
    /// No selector was supplied. A quoted empty key (`''`) is valid.
    #[error("empty key-path glob; use '*' to select top-level keys")]
    Empty,
    /// A literal span has no closing single quote.
    #[error("unclosed quote at byte {index}")]
    UnclosedQuote {
        /// Byte offset of the opening quote in the original expression.
        index: usize,
    },
    /// Invalid glob syntax, with the offset mapped to the original expression.
    #[error("{0}")]
    Glob(GlobError),
}

/// A glob selecting full key paths for wholesale replacement.
///
/// Unquoted dots separate keys; single-quoted spans are literal, including
/// wildcard characters. Thus `foo.*` selects children of `foo`, whereas
/// `'foo.bar'` selects a single key containing a dot. Backslash escapes the
/// next byte literally inside quotes; outside quotes it follows fast-glob.
/// Arrays are never traversed and brackets are character classes, not indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyGlobPattern(GlobPattern);

// These bytes cannot occur in UTF-8 keys. Encoding literal separators keeps
// the matching grammar independent of std::path::is_separator on Windows.
const LITERAL_SLASH: u8 = 0xff;
const LITERAL_BACKSLASH: u8 = 0xfe;

fn encode(byte: u8) -> u8 {
    match byte {
        b'/' => LITERAL_SLASH,
        b'\\' => LITERAL_BACKSLASH,
        byte => byte,
    }
}

impl KeyGlobPattern {
    /// Match actual key segments, never a dotted diagnostic rendering.
    /// The empty slice denotes the document root and is never selected.
    pub fn matches_keys(&self, keys: &[String]) -> bool {
        if keys.is_empty() {
            return false;
        }
        let mut candidate = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            if index != 0 {
                candidate.push(b'/');
            }
            candidate.extend(key.bytes().map(encode));
        }
        self.0.matches(candidate)
    }
}

struct Normalizer<'a> {
    source: &'a [u8],
    bytes: Vec<u8>,
    offsets: Vec<usize>,
}

impl Normalizer<'_> {
    fn push(&mut self, byte: u8, offset: usize) {
        self.bytes.push(byte);
        self.offsets.push(offset);
    }

    fn literal(&mut self, byte: u8, offset: usize) {
        let byte = encode(byte);
        if matches!(byte, b'*' | b'?' | b'[' | b']' | b'{' | b'}' | b',' | b'!') {
            self.push(b'\\', offset);
        }
        self.push(byte, offset);
    }

    fn glob_error(&self, kind: fast_glob::ErrorKind, index: usize) -> KeyGlobError {
        KeyGlobError::Glob(GlobError { kind, index })
    }

    // Decode exactly fast-glob's escapes outside quoted literal spans.
    fn escaped(&self, index: &mut usize) -> Result<u8, KeyGlobError> {
        let offset = *index;
        let mut byte = self.source[*index];
        if byte == b'\\' {
            *index += 1;
            byte = *self
                .source
                .get(*index)
                .ok_or_else(|| self.glob_error(fast_glob::ErrorKind::TrailingBackslash, offset))?;
            byte = match byte {
                b'b' => 8,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                byte => byte,
            };
        }
        *index += 1;
        Ok(byte)
    }

    fn class(&mut self, index: &mut usize) -> Result<(), KeyGlobError> {
        let start = *index;
        *index += 1;
        let negated = matches!(self.source.get(*index), Some(b'!' | b'^'));
        if negated {
            *index += 1;
        }
        let mut members = [false; 256];
        let mut first = true;
        loop {
            let Some(&byte) = self.source.get(*index) else {
                return Err(self.glob_error(fast_glob::ErrorKind::UnclosedBracket, start));
            };
            if byte == b']' && !first {
                *index += 1;
                break;
            }
            let low = self.escaped(index)?;
            let high = if self.source.get(*index) == Some(&b'-')
                && self.source.get(*index + 1).is_some_and(|b| *b != b']')
            {
                *index += 1;
                self.escaped(index)?
            } else {
                low
            };
            if low <= high {
                members[usize::from(low)..=usize::from(high)].fill(true);
            }
            first = false;
        }

        self.push(b'[', start);
        let mut emitted = false;
        // 0xfe and 0xff cannot occur in an original UTF-8 key. Every other
        // original byte maps injectively, including slash and backslash.
        for original in 0..=0xfd_u8 {
            if members[usize::from(original)] != negated {
                let byte = encode(original);
                if matches!(byte, b']' | b'-' | b'^' | b'!' | b'[') {
                    self.push(b'\\', start);
                }
                self.push(byte, start);
                emitted = true;
            }
        }
        if !emitted {
            // A class containing only the internal separator never matches.
            self.push(b'/', start);
        }
        self.push(b']', start);
        Ok(())
    }
}

impl FromStr for KeyGlobPattern {
    type Err = KeyGlobError;

    fn from_str(pattern: &str) -> Result<Self, Self::Err> {
        if pattern.is_empty() {
            return Err(KeyGlobError::Empty);
        }
        let mut norm = Normalizer {
            source: pattern.as_bytes(),
            bytes: Vec::new(),
            offsets: Vec::new(),
        };
        let mut index = 0;
        while index < norm.source.len() {
            let offset = index;
            match norm.source[index] {
                b'\'' => {
                    index += 1;
                    loop {
                        let Some(&byte) = norm.source.get(index) else {
                            return Err(KeyGlobError::UnclosedQuote { index: offset });
                        };
                        if byte == b'\'' {
                            index += 1;
                            break;
                        }
                        let literal_offset = index;
                        if byte == b'\\' {
                            index += 1;
                            if index == norm.source.len() {
                                return Err(norm.glob_error(
                                    fast_glob::ErrorKind::TrailingBackslash,
                                    literal_offset,
                                ));
                            }
                        }
                        norm.literal(norm.source[index], literal_offset);
                        index += 1;
                    }
                }
                b'[' => norm.class(&mut index)?,
                b'\\' => {
                    let byte = norm.escaped(&mut index)?;
                    norm.literal(byte, offset);
                }
                b'.' => {
                    norm.push(b'/', offset);
                    index += 1;
                }
                byte => {
                    norm.push(encode(byte), offset);
                    index += 1;
                }
            }
        }
        GlobPattern::from_bytes(norm.bytes)
            .map(Self)
            .map_err(|mut err| {
                err.index = norm
                    .offsets
                    .get(err.index)
                    .copied()
                    .unwrap_or(pattern.len());
                KeyGlobError::Glob(err)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, keys: &[&str]) -> bool {
        pattern
            .parse::<KeyGlobPattern>()
            .unwrap()
            .matches_keys(&keys.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn selectors_match_full_paths_and_respect_depth() {
        for (pattern, keys, expected) in [
            ("*", vec!["foo"], true),
            ("*", vec!["foo.bar/baz\\qux"], true),
            ("*", vec!["foo", "bar"], false),
            ("foo", vec!["foo"], true),
            ("foo", vec!["parent", "foo"], false),
            ("foo.*", vec!["foo", "bar"], true),
            ("foo.*", vec!["foo", "bar", "baz"], false),
            ("foo.*", vec!["foo"], false),
            ("foo.**", vec!["foo", "bar", "baz"], true),
            ("**.cache", vec!["cache"], true),
            ("**.cache", vec!["foo", "bar", "cache"], true),
            ("**.cache", vec!["foo", "cache", "bar"], false),
            ("{foo,bar}.*", vec!["bar", "baz"], true),
            ("{foo.bar,baz}.*", vec!["foo", "bar", "qux"], true),
            ("{foo,bar}.*", vec!["baz", "qux"], false),
            ("!foo", vec!["foo"], false),
            ("!foo", vec!["bar"], true),
            ("!!foo", vec!["foo"], true),
            ("!foo.*", vec!["foo"], true),
            ("*", vec![], false),
            ("''", vec![""], true),
            ("''", vec![], false),
            ("foo.''", vec!["foo", ""], true),
        ] {
            assert_eq!(matches(pattern, &keys), expected, "{pattern:?}: {keys:?}");
        }
    }

    #[test]
    fn literals_do_not_collide_with_nested_paths_or_glob_syntax() {
        for (pattern, keys, expected) in [
            ("'foo.bar'", vec!["foo.bar"], true),
            ("'foo.bar'", vec!["foo", "bar"], false),
            ("foo.bar", vec!["foo.bar"], false),
            ("'foo.bar'.*", vec!["foo.bar", "pool"], true),
            (r"foo\.bar", vec!["foo.bar"], true),
            ("foo/bar", vec!["foo/bar"], true),
            ("foo/bar", vec!["foo", "bar"], false),
            ("'foo/bar'", vec!["foo/bar"], true),
            (r"'foo\\bar'", vec![r"foo\bar"], true),
            (r"foo\\bar", vec![r"foo\bar"], true),
            (r"'it\'s'", vec!["it's"], true),
            ("'*'", vec!["*"], true),
            ("'*'", vec!["anything"], false),
            ("'!foo'", vec!["!foo"], true),
            ("'!foo'", vec!["foo"], false),
            ("'{a,b}[0]?'", vec!["{a,b}[0]?"], true),
            ("{'foo.bar',baz}.*", vec!["foo.bar", "x"], true),
            ("foo'bar'.*", vec!["foobar", "x"], true),
            ("'µ'", vec!["µ"], true),
            ("?", vec!["µ"], false),
            ("??", vec!["µ"], true),
            ("?", vec!["/"], true),
            ("?", vec!["\\"], true),
            ("?", vec!["", ""], false),
            (r"\n", vec!["\n"], true),
            (r"'\n'", vec!["n"], true),
        ] {
            assert_eq!(matches(pattern, &keys), expected, "{pattern:?}: {keys:?}");
        }
    }

    #[test]
    fn classes_preserve_original_byte_membership() {
        for pattern in [
            "[a-z]", "[!a-z]", "[^a-z]", "[.]", "[/]", r"[\\]", "[.-0]", r"[Z-\^]", "[]]", "[[]",
            "[-]", "[z-a]", "[!z-a]", "['.]", r"[\n]",
        ] {
            let selector: KeyGlobPattern = pattern.parse().unwrap();
            // A class always selects one byte within a key. Unlike filesystem
            // matching, slash and backslash are ordinary key bytes here.
            for byte in 0..=127_u8 {
                let reference = pattern.as_bytes().to_vec();
                // Compare the original class against a non-separator byte;
                // separators are checked explicitly below instead.
                if matches!(byte, b'/' | b'\\') {
                    continue;
                }
                assert_eq!(
                    selector.matches_keys(&[String::from_utf8(vec![byte]).unwrap()]),
                    fast_glob::glob_match(&reference, [byte]),
                    "{pattern:?} byte {byte}"
                );
            }
        }
        for (pattern, key, expected) in [
            ("[/]", "/", true),
            (r"[\\]", "\\", true),
            ("[.-0]", "/", true),
            ("[!.-0]", "/", false),
            (r"[Z-\^]", "\\", true),
            ("[!a-z]", "/", true),
            ("[.]", ".", true),
            ("[.]", "/", false),
            ("[z-a]", "z", false),
        ] {
            assert_eq!(matches(pattern, &[key]), expected, "{pattern:?}: {key:?}");
        }
        assert!(!matches("[!a-z]", &["", ""]));
    }

    #[test]
    fn errors_name_original_source_offsets() {
        assert_eq!("".parse::<KeyGlobPattern>(), Err(KeyGlobError::Empty));
        assert_eq!(
            "µ.'unclosed".parse::<KeyGlobPattern>(),
            Err(KeyGlobError::UnclosedQuote { index: 3 })
        );
        for (pattern, kind, index) in [
            (
                "'long literal'.{foo",
                fast_glob::ErrorKind::UnclosedBrace,
                15,
            ),
            ("[!a-z].{foo", fast_glob::ErrorKind::UnclosedBrace, 7),
            ("foo.[bar", fast_glob::ErrorKind::UnclosedBracket, 4),
            ("foo.\\", fast_glob::ErrorKind::TrailingBackslash, 4),
            ("'foo\\", fast_glob::ErrorKind::TrailingBackslash, 4),
        ] {
            let err = pattern.parse::<KeyGlobPattern>().unwrap_err();
            assert_eq!(err, KeyGlobError::Glob(GlobError { kind, index }));
            assert!(!err.to_string().ends_with('\n'));
            assert!(!err.to_string().contains("--shallow"));
        }
    }

    proptest::proptest! {
        #[test]
        fn quoted_keys_round_trip_without_separator_collisions(key in ".*") {
            let quoted = format!("'{}'", key.replace('\\', "\\\\").replace('\'', "\\'"));
            let selector: KeyGlobPattern = quoted.parse().unwrap();
            proptest::prop_assert!(selector.matches_keys(std::slice::from_ref(&key)));
            if key.contains('/') {
                let nested = key.split('/').map(String::from).collect::<Vec<_>>();
                proptest::prop_assert!(!selector.matches_keys(&nested));
            }
        }
    }
}
