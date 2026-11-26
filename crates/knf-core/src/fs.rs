//! Filter path lists with glob patterns, without touching the filesystem.

use std::path::{Path, PathBuf};

/// An invalid glob pattern.
pub use crate::glob::{GlobError, GlobPattern};

impl GlobPattern {
    /// Match the entire supplied spelling, normalizing Windows separators only.
    pub fn matches_path(&self, path: &Path) -> bool {
        let bytes = path.as_os_str().as_encoded_bytes();
        #[cfg(windows)]
        let bytes = bytes
            .iter()
            .map(|&byte| if byte == b'\\' { b'/' } else { byte })
            .collect::<Vec<_>>();
        self.matches(bytes)
    }

    /// Match only the filename; paths without one do not match.
    pub fn matches_filename(&self, path: &Path) -> bool {
        path.file_name()
            .is_some_and(|name| self.matches(name.as_encoded_bytes()))
    }
}

/// The part of a path [`filter_paths`] matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobTarget {
    /// The whole supplied spelling.
    Path,
    /// The filename, ignoring directory components.
    Filename,
}

/// Filter existing candidates without I/O, preserving spelling, order and duplicates.
///
/// Matching uses native encoded bytes, so `?` matches one byte. An empty
/// selection is allowed.
pub fn filter_paths<P: AsRef<Path>>(
    files: &[P],
    pattern: &GlobPattern,
    target: GlobTarget,
) -> Vec<PathBuf> {
    files
        .iter()
        .map(AsRef::as_ref)
        .filter(|path| match target {
            GlobTarget::Path => pattern.matches_path(path),
            GlobTarget::Filename => pattern.matches_filename(path),
        })
        .map(Path::to_path_buf)
        .collect()
}
