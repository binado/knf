//! Validated glob predicates over the resolved CLI inputs, without filesystem I/O.

use std::path::Path;

/// A user-written pattern validated once by clap, before any filesystem access.
#[derive(Debug, Clone)]
pub struct GlobPattern(String);

impl std::str::FromStr for GlobPattern {
    type Err = fast_glob::Error;

    fn from_str(pattern: &str) -> Result<Self, Self::Err> {
        fast_glob::validate(pattern)?;
        Ok(Self(pattern.to_owned()))
    }
}

impl GlobPattern {
    pub fn matches_path(&self, path: &Path) -> bool {
        let bytes = path.as_os_str().as_encoded_bytes();
        // Normalize separators only for matching; preserve the original input.
        #[cfg(windows)]
        let bytes = bytes
            .iter()
            .map(|&byte| if byte == b'\\' { b'/' } else { byte })
            .collect::<Vec<_>>();
        fast_glob::glob_match(&self.0, bytes)
    }

    pub fn matches_filename(&self, path: &Path) -> bool {
        path.file_name()
            .is_some_and(|name| fast_glob::glob_match(&self.0, name.as_encoded_bytes()))
    }
}
