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

/// Filter existing candidates without I/O, preserving spelling, order and duplicates.
///
/// Matching uses native encoded bytes, so `?` matches one byte. An empty
/// selection is allowed. Set `filename_only` to ignore directory components.
pub fn filter_paths<P: AsRef<Path>>(
    files: &[P],
    pattern: &GlobPattern,
    filename_only: bool,
) -> Vec<PathBuf> {
    files
        .iter()
        .map(AsRef::as_ref)
        .filter(|path| {
            if filename_only {
                pattern.matches_filename(path)
            } else {
                pattern.matches_path(path)
            }
        })
        .map(Path::to_path_buf)
        .collect()
}
