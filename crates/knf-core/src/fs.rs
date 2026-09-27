//! Discover ordered configuration inputs and filter path lists without loading them.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{Format, STDIN};

/// An invalid glob pattern.
pub use fast_glob::Error as GlobError;

/// A validated relative JSON or TOML target, with `.` components removed.
#[derive(Debug, Clone)]
pub struct AccumulateTarget {
    path: PathBuf,
    format: Format,
}

/// Why a target cannot be used for discovery.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccumulateTargetError {
    /// Standard input cannot be discovered.
    #[error("discovery does not accept stdin")]
    Stdin,
    /// Targets must be relative and must not traverse parents.
    #[error("discovery requires a relative target path without .. components")]
    InvalidPath,
    /// The target extension must select JSON or TOML.
    #[error("discovery requires a target with a JSON or TOML extension")]
    UnknownExtension,
}

impl TryFrom<PathBuf> for AccumulateTarget {
    type Error = AccumulateTargetError;

    fn try_from(raw: PathBuf) -> Result<Self, Self::Error> {
        if raw.as_os_str() == STDIN {
            return Err(AccumulateTargetError::Stdin);
        }
        if raw.components().any(|component| {
            matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::ParentDir
            )
        }) {
            return Err(AccumulateTargetError::InvalidPath);
        }
        let path: PathBuf = raw
            .components()
            .filter(|component| !matches!(component, Component::CurDir))
            .collect();
        let format = Format::from_path(&path).ok_or(AccumulateTargetError::UnknownExtension)?;
        Ok(Self { path, format })
    }
}

/// A filesystem failure during discovery. Paths are structured diagnostic data.
#[derive(Debug, thiserror::Error)]
pub enum AccumulateError {
    /// The working directory could not be obtained for a relative base.
    #[error("cannot determine the working directory")]
    CurrentDirectory(#[source] io::Error),
    /// Metadata could not be read (symlinks are followed).
    #[error("cannot inspect a discovery path")]
    Inspect {
        /// The path whose metadata failed.
        path: PathBuf,
        /// The underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// A directory or one of its entries could not be read.
    #[error("cannot list a discovery directory")]
    List {
        /// The directory being listed.
        path: PathBuf,
        /// The underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// The named target is a directory.
    #[error("discovery target is a directory")]
    Directory {
        /// The target path.
        path: PathBuf,
    },
    /// The named target is neither a directory nor a regular file.
    #[error("discovery target is not a regular file")]
    NonRegular {
        /// The target path.
        path: PathBuf,
    },
}

/// Discover same-format files along a target's directories, then append the target.
///
/// The base directory itself is excluded. Each visited directory contributes its
/// regular files in native filename order. Symlinks are followed without
/// canonicalization or deduplication. Discovery never reads configuration contents.
///
/// With no base, paths are relative to the working directory. An explicit base
/// is made absolute once and results are absolute, without changing the working
/// directory. Any inspection failure aborts discovery; no partial list is returned.
pub fn accumulate(
    target: &AccumulateTarget,
    base_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, AccumulateError> {
    let base = match base_dir {
        None => PathBuf::new(),
        Some(base) if base.is_absolute() => base.to_path_buf(),
        // Use the platform's absolute-path rules, including Windows drive-relative
        // and root-relative bases. An empty base denotes the working directory.
        Some(base) => std::path::absolute(if base.as_os_str().is_empty() {
            Path::new(".")
        } else {
            base
        })
        .map_err(AccumulateError::CurrentDirectory)?,
    };
    let target_path = base.join(&target.path);
    let metadata = fs::metadata(&target_path).map_err(|source| AccumulateError::Inspect {
        path: target_path.clone(),
        source,
    })?;
    if metadata.is_dir() {
        return Err(AccumulateError::Directory { path: target_path });
    }
    if !metadata.is_file() {
        return Err(AccumulateError::NonRegular { path: target_path });
    }

    let mut files = Vec::new();
    let mut directory = base;
    if let Some(parent) = target.path.parent() {
        for component in parent.components() {
            directory.push(component);
            let listing_error = |source| AccumulateError::List {
                path: directory.clone(),
                source,
            };
            let entries = fs::read_dir(&directory).map_err(listing_error)?;
            let mut matching = Vec::new();
            for entry in entries {
                let entry = entry.map_err(listing_error)?;
                let path = entry.path();
                if path == target_path || Format::from_path(&path) != Some(target.format) {
                    continue;
                }
                let metadata = fs::metadata(&path).map_err(|source| AccumulateError::Inspect {
                    path: path.clone(),
                    source,
                })?;
                if metadata.is_file() {
                    matching.push((entry.file_name(), path));
                }
            }
            matching.sort_by(|left, right| left.0.cmp(&right.0));
            files.extend(matching.into_iter().map(|(_, path)| path));
        }
    }
    files.push(target_path);
    Ok(files)
}

/// A case-sensitive glob predicate, validated before any filesystem access.
#[derive(Debug, Clone)]
pub struct GlobPattern(String);

impl std::str::FromStr for GlobPattern {
    type Err = GlobError;

    fn from_str(pattern: &str) -> Result<Self, Self::Err> {
        fast_glob::validate(pattern)?;
        Ok(Self(pattern.to_owned()))
    }
}

impl GlobPattern {
    /// Match the entire supplied spelling, normalizing Windows separators only.
    pub fn matches_path(&self, path: &Path) -> bool {
        let bytes = path.as_os_str().as_encoded_bytes();
        #[cfg(windows)]
        let bytes = bytes
            .iter()
            .map(|&byte| if byte == b'\\' { b'/' } else { byte })
            .collect::<Vec<_>>();
        fast_glob::glob_match(&self.0, bytes)
    }

    /// Match only the filename; paths without one do not match.
    pub fn matches_filename(&self, path: &Path) -> bool {
        path.file_name()
            .is_some_and(|name| fast_glob::glob_match(&self.0, name.as_encoded_bytes()))
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
