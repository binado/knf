//! Filesystem-as-config-groups: discover mutually exclusive alternatives in a
//! directory tree and enumerate their cartesian product.
//!
//! A directory is a group and the files within it are mutually exclusive
//! alternatives; each subdirectory is an independent axis that also applies.
//! Grouping is keyed by parent directory, not by depth: pooling by depth would
//! turn sibling `db/` and `server/` into one four-way axis, yielding configs
//! with a db *or* a server and never both.
//!
//! This crate is deliberately tiny and dependency-free beyond `thiserror` and
//! `walkdir`. It knows nothing about config formats, merging, or the command
//! line; which files count as eligible is the caller's decision, passed as a
//! predicate to [`discover`].

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

// --- errors ---------------------------------------------------------------

/// Failures from [`discover`].
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("`{path}` is not a directory")]
    NotADirectory { path: PathBuf },

    #[error("`{path}` contains no eligible files\nhelp: add a file, or remove the directory")]
    EmptyDirectory { path: PathBuf },

    #[error(
        "group `{id}` is defined twice: the root directory's basename collides with a subdirectory\n\
         help: rename the subdirectory, or point `knf matrix` at a differently named root"
    )]
    DuplicateGroup { id: String },

    #[error("resolving `{path}`")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("reading `{path}`")]
    ReadEntry {
        path: PathBuf,
        #[source]
        source: walkdir::Error,
    },
}

/// Failures from [`resolve_axes`].
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("no group `{group_id}`\nhelp: groups are {known}")]
    UnknownGroup { group_id: String, known: String },

    #[error(
        "group `{group_id}` has no alternative `{choice}`\nhelp: alternatives are {alternatives}"
    )]
    UnknownChoice {
        group_id: String,
        choice: String,
        alternatives: String,
    },
}

// --- the group model ------------------------------------------------------

/// A set of mutually exclusive alternatives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Path relative to the root, `/`-separated (`db`, `db/tuning`). The root
    /// directory's own file group is named by the root's basename.
    pub id: String,
    /// Never empty: a directory with no eligible files contributes no group.
    pub alternatives: Vec<Alternative>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternative {
    /// File stem — the name used in pins and in output filenames.
    pub name: String,
    pub path: PathBuf,
}

impl Group {
    /// Whether this group has exactly one alternative and thus auto-selects.
    pub fn is_singleton(&self) -> bool {
        self.alternatives.len() == 1
    }

    /// The `GROUP=CHOICE` strings for each alternative, in stored order.
    pub fn choice_names(&self) -> Vec<String> {
        self.alternatives
            .iter()
            .map(|a| format!("{}={}", self.id, a.name))
            .collect()
    }
}

// --- enumeration (pure: no filesystem) ------------------------------------

/// Resolves pins into the list of selectable alternatives per group.
///
/// A pinned group offers exactly its pin, a singleton auto-selects, and any
/// other group becomes a free axis offering all of its alternatives.
pub fn resolve_axes(
    groups: &[Group],
    pins: &BTreeMap<String, String>,
) -> Result<Vec<Vec<usize>>, ResolveError> {
    for group_id in pins.keys() {
        if !groups.iter().any(|g| &g.id == group_id) {
            let known: Vec<&str> = groups.iter().map(|g| g.id.as_str()).collect();
            return Err(ResolveError::UnknownGroup {
                group_id: group_id.clone(),
                known: known.join(", "),
            });
        }
    }

    groups
        .iter()
        .map(|group| match pins.get(&group.id) {
            Some(choice) => {
                let index = group
                    .alternatives
                    .iter()
                    .position(|a| &a.name == choice)
                    .ok_or_else(|| ResolveError::UnknownChoice {
                        group_id: group.id.clone(),
                        choice: choice.clone(),
                        alternatives: group.choice_names().join(", "),
                    })?;
                Ok(vec![index])
            }
            None if group.is_singleton() => Ok(vec![0]),
            None => Ok((0..group.alternatives.len()).collect()),
        })
        .collect()
}

/// Cartesian product over the axes, the last axis varying fastest.
///
/// Everything-pinned is the degenerate case — every axis has length one and the
/// product is a single tuple — so there is no separate `pick` code path.
pub fn enumerate(axes: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut tuples = vec![Vec::with_capacity(axes.len())];
    for axis in axes {
        tuples = tuples
            .into_iter()
            .flat_map(|prefix| {
                axis.iter().map(move |&choice| {
                    let mut next = prefix.clone();
                    next.push(choice);
                    next
                })
            })
            .collect();
    }
    tuples
}

/// How many documents the axes describe. `None` on overflow, so a pathological
/// tree hits `--max` rather than wrapping.
pub fn product_size(axes: &[Vec<usize>]) -> Option<usize> {
    axes.iter()
        .try_fold(1usize, |acc, axis| acc.checked_mul(axis.len()))
}

/// The `GROUP=CHOICE` pairs identifying a tuple, sorted by group id.
///
/// Only groups with more than one alternative appear — pinned ones included,
/// since dropping them would make filenames irreversible, and singletons
/// excluded, since they carry no information. Sorting by group id means a tuple
/// always yields the same name regardless of walk order.
pub fn name_pairs(groups: &[Group], tuple: &[usize]) -> Vec<String> {
    let mut pairs: Vec<(&str, String)> = groups
        .iter()
        .zip(tuple)
        .filter(|(group, _)| !group.is_singleton())
        .map(|(group, &choice)| {
            (
                group.id.as_str(),
                format!("{}={}", group.id, group.alternatives[choice].name),
            )
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs.into_iter().map(|(_, pair)| pair).collect()
}

// --- discovery ------------------------------------------------------------

/// Walks the tree, collecting groups in DFS order.
///
/// A directory's own file group comes before its subdirectories and
/// subdirectories are visited in sorted order, so a tuple's layers come out
/// shallow -> deep and §2.1's left-fold rule is unchanged.
///
/// `is_eligible` decides which files count as alternatives — the crate itself
/// has no notion of file format, so the caller passes the predicate (e.g.
/// "has a `.json` or `.toml` extension").
pub fn discover(
    root: &Path,
    is_eligible: impl Fn(&Path) -> bool,
) -> Result<Vec<Group>, DiscoverError> {
    if !root.is_dir() {
        return Err(DiscoverError::NotADirectory {
            path: root.to_path_buf(),
        });
    }
    let root_id = root_group_id(root)?;
    let mut groups = Vec::new();
    walk(root, "", &root_id, &mut groups, &is_eligible)?;

    // A root basename colliding with a subdirectory name would make pins
    // ambiguous. Cheap to detect here, so detect it here rather than producing
    // a confusing pin error later.
    let mut seen: HashSet<&str> = HashSet::new();
    for group in &groups {
        if !seen.insert(group.id.as_str()) {
            return Err(DiscoverError::DuplicateGroup {
                id: group.id.clone(),
            });
        }
    }
    Ok(groups)
}

/// The root's own file group is named by its basename, so `knf matrix config/`
/// gives group `config`.
pub fn root_group_id(root: &Path) -> Result<String, DiscoverError> {
    if let Some(name) = root.file_name().and_then(|n| n.to_str()) {
        return Ok(name.to_string());
    }
    // `.` and `..` have no file_name of their own.
    let canonical = root
        .canonicalize()
        .map_err(|source| DiscoverError::Canonicalize {
            path: root.to_path_buf(),
            source,
        })?;
    Ok(canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("root")
        .to_string())
}

/// Returns whether `dir` contributed anything — either eligible files of its
/// own or a subdirectory that did.
///
/// `rel` is the path from the root, `/`-separated and empty at the root itself.
/// It, not the parent's group id, is what names a group: the root's file group
/// is named by the root basename, so threading ids would make the root's
/// subdirectory `db` come out as `config/db` and stop matching the pin `db`.
fn walk<F: Fn(&Path) -> bool>(
    dir: &Path,
    rel: &str,
    root_id: &str,
    groups: &mut Vec<Group>,
    is_eligible: &F,
) -> Result<bool, DiscoverError> {
    let (files, subdirs) = read_entries(dir, is_eligible)?;

    let mut contributed = false;
    // The directory's own file group comes before its subdirectories, so the
    // DFS order of `groups` is also the shallow -> deep layer order.
    if !files.is_empty() {
        contributed = true;
        groups.push(Group {
            id: if rel.is_empty() {
                root_id.to_string()
            } else {
                rel.to_string()
            },
            alternatives: files,
        });
    }

    for (name, path) in subdirs {
        let child_rel = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if walk(&path, &child_rel, root_id, groups, is_eligible)? {
            contributed = true;
        }
    }

    if !contributed {
        return Err(DiscoverError::EmptyDirectory {
            path: dir.to_path_buf(),
        });
    }
    Ok(contributed)
}

/// Eligible files and subdirectories of one directory, each sorted byte-wise.
type Entries = (Vec<Alternative>, Vec<(String, PathBuf)>);

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
            continue; // A non-UTF-8 name could never be a group or choice name.
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
            let stem = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name)
                .to_string();
            files.push(Alternative {
                name: stem,
                path: entry.path().to_path_buf(),
            });
        }
    }

    // Byte-wise lexicographic, explicitly not natural/numeric sort — the latter
    // looks friendly right up until someone has both `2-x` and `10-x`.
    files.sort_by(|a, b| a.name.cmp(&b.name));
    subdirs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((files, subdirs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2^64 documents must reach `--max` rather than wrapping to a small
    /// number and quietly writing something.
    #[test]
    fn product_overflow_is_not_a_panic() {
        assert_eq!(product_size(&[]), Some(1));
        assert_eq!(product_size(&[vec![0, 1], vec![0, 1, 2]]), Some(6));
        assert_eq!(product_size(&vec![vec![0usize, 1]; 64]), None);
    }
}
