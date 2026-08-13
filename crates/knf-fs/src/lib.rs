//! Filesystem-as-config-paths: discover a directory tree and enumerate
//! saturating DFS paths from the root to each leaf file.
//!
//! Eligible files in a directory are mutually exclusive layers for that node.
//! Subdirectories are alternative branches, not independent axes: a path picks
//! one child and continues. Files on the way down always apply.
//!
//! This crate is deliberately tiny and dependency-free beyond `thiserror` and
//! `walkdir`. It knows nothing about config formats, merging, or the command
//! line; which files count as eligible, and which entries a glob keeps, are
//! the caller's predicates, passed to [`discover`].

use std::path::{Path, PathBuf};

// --- errors ---------------------------------------------------------------

/// Failures from [`discover`].
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("`{path}` is not a directory")]
    NotADirectory { path: PathBuf },

    #[error("`{path}` contains no eligible files\nhelp: add a file, or remove the directory")]
    EmptyDirectory { path: PathBuf },

    #[error("no paths matched\nhelp: loosen --glob, or raise --max-depth")]
    NoMatch,

    #[error("reading `{path}`")]
    ReadEntry {
        path: PathBuf,
        #[source]
        source: walkdir::Error,
    },
}

/// Failures from [`paths_capped`].
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("this tree describes more than {max} documents")]
    TooMany { max: usize },
}

// --- the tree -------------------------------------------------------------

/// An eligible file. Naming is the path relative to the discover root, not a
/// stem — stems existed for `GROUP=CHOICE` pins, which this crate no longer
/// has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
}

/// A directory that was entered during discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dir {
    pub path: PathBuf,
    /// Eligible files, byte-wise sorted. Mutually exclusive at this node.
    pub files: Vec<File>,
    /// Children that were entered, sorted by name.
    pub dirs: Vec<Dir>,
}

/// Whether [`discover`]'s `include` predicate is looking at a file or a
/// directory. Directories decide descent; files decide leaf emission only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
}

/// Path of `file` relative to `root`, `/`-separated.
pub fn rel(root: &Path, file: &File) -> String {
    file.path
        .strip_prefix(root)
        .unwrap_or(&file.path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

// --- enumeration (pure: no filesystem) ------------------------------------

/// Saturating DFS: files at a node are OR; children are OR (a sum), not AND
/// (a product). Each inner vec is shallow → deep; the last file is the leaf.
pub fn paths(root: &Dir) -> Vec<Vec<&File>> {
    let mut out = Vec::new();
    walk_paths(root, Vec::new(), &mut out, None).expect("uncapped walk cannot overflow");
    out
}

/// Like [`paths`], but errors instead of returning more than `max` paths, so a
/// pathological tree hits `--max` rather than writing half a tree.
pub fn paths_capped(root: &Dir, max: usize) -> Result<Vec<Vec<&File>>, PathsError> {
    let mut out = Vec::new();
    walk_paths(root, Vec::new(), &mut out, Some(max))?;
    Ok(out)
}

fn walk_paths<'a>(
    dir: &'a Dir,
    prefix: Vec<&'a File>,
    out: &mut Vec<Vec<&'a File>>,
    max: Option<usize>,
) -> Result<(), PathsError> {
    // No files here means "pass through" — a directory that only contributes
    // via a subdirectory still saturates to the leaf below.
    let choices: Vec<Option<&File>> = if dir.files.is_empty() {
        vec![None]
    } else {
        dir.files.iter().map(Some).collect()
    };

    if dir.dirs.is_empty() {
        for file in choices {
            let mut next = prefix.clone();
            if let Some(file) = file {
                next.push(file);
            }
            if next.is_empty() {
                continue;
            }
            push_path(out, next, max)?;
        }
        return Ok(());
    }

    for file in choices {
        let mut next = prefix.clone();
        if let Some(file) = file {
            next.push(file);
        }
        for child in &dir.dirs {
            walk_paths(child, next.clone(), out, max)?;
        }
    }
    Ok(())
}

fn push_path<'a>(
    out: &mut Vec<Vec<&'a File>>,
    path: Vec<&'a File>,
    max: Option<usize>,
) -> Result<(), PathsError> {
    if let Some(max) = max
        && out.len() >= max
    {
        return Err(PathsError::TooMany { max });
    }
    out.push(path);
    Ok(())
}

// --- discovery ------------------------------------------------------------

/// Walks the tree, building a [`Dir`] pruned by `include` and `max_depth`.
///
/// `is_eligible` decides which files count — the crate itself has no notion of
/// file format, so the caller passes the predicate (e.g. "has a `.json` or
/// `.toml` extension").
///
/// `include` gets a `/`-separated path relative to `root`. [`EntryKind::Dir`]
/// decides whether to enter a subdirectory; [`EntryKind::File`] decides whether
/// a file is emitted as a leaf. Ancestor files are always kept.
///
/// `max_depth` is counted from the root (depth 0). `Some(0)` keeps only the
/// root's files; `None` saturates.
pub fn discover(
    root: &Path,
    is_eligible: impl Fn(&Path) -> bool,
    include: impl Fn(&str, EntryKind) -> bool,
    max_depth: Option<usize>,
) -> Result<Dir, DiscoverError> {
    if !root.is_dir() {
        return Err(DiscoverError::NotADirectory {
            path: root.to_path_buf(),
        });
    }
    match walk(root, "", 0, max_depth, &is_eligible, &include)? {
        Some(dir) => Ok(dir),
        None => Err(DiscoverError::NoMatch),
    }
}

/// Returns `None` when glob or depth eliminated every contribution, so the
/// parent can omit this child rather than treating it as [`EmptyDirectory`].
fn walk<E, I>(
    dir: &Path,
    rel: &str,
    depth: usize,
    max_depth: Option<usize>,
    is_eligible: &E,
    include: &I,
) -> Result<Option<Dir>, DiscoverError>
where
    E: Fn(&Path) -> bool,
    I: Fn(&str, EntryKind) -> bool,
{
    let (all_files, subdirs) = read_entries(dir, is_eligible)?;
    let at_limit = max_depth.is_some_and(|max| depth >= max);

    let mut children = Vec::new();
    if !at_limit {
        for (name, path) in &subdirs {
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if !include(&child_rel, EntryKind::Dir) {
                continue;
            }
            if let Some(child) = walk(path, &child_rel, depth + 1, max_depth, is_eligible, include)?
            {
                children.push(child);
            }
        }
    }

    let files = if children.is_empty() {
        all_files
            .into_iter()
            .filter(|file| include(&file_rel(rel, file), EntryKind::File))
            .collect()
    } else {
        all_files
    };

    if files.is_empty() && children.is_empty() {
        if subdirs.is_empty() {
            return Err(DiscoverError::EmptyDirectory {
                path: dir.to_path_buf(),
            });
        }
        // Children exist on disk but glob or max_depth cut them, and this
        // node has no (matching) files of its own.
        return Ok(None);
    }

    Ok(Some(Dir {
        path: dir.to_path_buf(),
        files,
        dirs: children,
    }))
}

fn file_rel(dir_rel: &str, file: &File) -> String {
    let name = file.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if dir_rel.is_empty() {
        name.to_string()
    } else {
        format!("{dir_rel}/{name}")
    }
}

/// Eligible files and subdirectories of one directory, each sorted byte-wise.
type Entries = (Vec<File>, Vec<(String, PathBuf)>);

fn read_entries<F: Fn(&Path) -> bool>(
    dir: &Path,
    is_eligible: &F,
) -> Result<Entries, DiscoverError> {
    let mut files = Vec::new();
    let mut subdirs = Vec::new();

    // `walkdir`, not `ignore`: gitignore semantics would mean "my config was
    // skipped because of a .gitignore three levels up", which is surprising in
    // a merge tool. One level at a time, so grouping stays per-directory.
    let iter = walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false);

    for entry in iter {
        let entry = entry.map_err(|source| DiscoverError::ReadEntry {
            path: dir.to_path_buf(),
            source,
        })?;
        let Some(name) = entry.file_name().to_str() else {
            continue; // A non-UTF-8 name could never be a glob or output path.
        };
        // Skip dotfiles and dot-directories, so `knf matrix .` does not walk
        // `.git`. Symlinks are skipped rather than followed, which keeps the
        // walk acyclic and the result independent of where a link points.
        if name.starts_with('.') || entry.file_type().is_symlink() {
            continue;
        }

        if entry.file_type().is_dir() {
            subdirs.push((name.to_string(), entry.path().to_path_buf()));
        } else if is_eligible(entry.path()) {
            // Everything else is skipped silently — a README.md in a config
            // directory is not an error.
            files.push(File {
                path: entry.path().to_path_buf(),
            });
        }
    }

    // Byte-wise lexicographic, explicitly not natural/numeric sort — the latter
    // looks friendly right up until someone has both `2-x` and `10-x`.
    files.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    subdirs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((files, subdirs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> File {
        File {
            path: PathBuf::from(path),
        }
    }

    /// `--max` must fire rather than returning a huge list.
    #[test]
    fn paths_capped_errors_instead_of_returning_too_many() {
        let dir = Dir {
            path: PathBuf::from("root"),
            files: vec![file("a.toml"), file("b.toml"), file("c.toml")],
            dirs: vec![],
        };
        assert_eq!(paths(&dir).len(), 3);
        let err = paths_capped(&dir, 2).unwrap_err();
        assert!(matches!(err, PathsError::TooMany { max: 2 }));
    }
}
