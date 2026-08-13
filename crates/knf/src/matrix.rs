//! `knf matrix` — a directory tree of config groups.
//!
//! A directory is a group and the files within it are mutually exclusive
//! alternatives; each subdirectory is an independent axis that also applies.
//! Grouping is keyed by parent directory, not by depth: pooling by depth would
//! turn sibling `db/` and `server/` into one four-way axis, yielding configs
//! with a db *or* a server and never both.
//!
//! Kept internal per §6 — this is the least settled design in the document.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use knf_merge::merge_all;

use crate::cli::MatrixArgs;
use crate::format::{self, Format, Source, SourceName};

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
    fn is_singleton(&self) -> bool {
        self.alternatives.len() == 1
    }

    fn choice_names(&self) -> Vec<String> {
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
) -> anyhow::Result<Vec<Vec<usize>>> {
    for group_id in pins.keys() {
        if !groups.iter().any(|g| &g.id == group_id) {
            let known: Vec<&str> = groups.iter().map(|g| g.id.as_str()).collect();
            bail!(
                "no group `{group_id}`\nhelp: groups are {}",
                known.join(", ")
            );
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
                    .with_context(|| {
                        format!(
                            "group `{}` has no alternative `{choice}`\nhelp: alternatives are {}",
                            group.id,
                            group.choice_names().join(", ")
                        )
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
fn product_size(axes: &[Vec<usize>]) -> Option<usize> {
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
pub fn discover(root: &Path) -> anyhow::Result<Vec<Group>> {
    if !root.is_dir() {
        bail!("`{}` is not a directory", root.display());
    }
    let mut groups = Vec::new();
    walk(root, "", &root_group_id(root)?, &mut groups)?;

    // A root basename colliding with a subdirectory name would make pins
    // ambiguous. Cheap to detect here, so detect it here rather than producing
    // a confusing pin error later.
    let mut seen: HashSet<&str> = HashSet::new();
    for group in &groups {
        if !seen.insert(group.id.as_str()) {
            bail!(
                "group `{}` is defined twice: the root directory's basename collides with a subdirectory\n\
                 help: rename the subdirectory, or point `knf matrix` at a differently named root",
                group.id
            );
        }
    }
    Ok(groups)
}

/// The root's own file group is named by its basename, so `knf matrix config/`
/// gives group `config`.
fn root_group_id(root: &Path) -> anyhow::Result<String> {
    if let Some(name) = root.file_name().and_then(|n| n.to_str()) {
        return Ok(name.to_string());
    }
    // `.` and `..` have no file_name of their own.
    let canonical = root
        .canonicalize()
        .with_context(|| format!("resolving `{}`", root.display()))?;
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
fn walk(dir: &Path, rel: &str, root_id: &str, groups: &mut Vec<Group>) -> anyhow::Result<bool> {
    let (files, subdirs) = read_entries(dir)?;

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
        if walk(&path, &child_rel, root_id, groups)? {
            contributed = true;
        }
    }

    if !contributed {
        bail!(
            "`{}` contains no .json or .toml files\n\
             help: add a file, or remove the directory",
            dir.display()
        );
    }
    Ok(contributed)
}

/// Eligible files and subdirectories of one directory, each sorted byte-wise.
type Entries = (Vec<Alternative>, Vec<(String, PathBuf)>);

fn read_entries(dir: &Path) -> anyhow::Result<Entries> {
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
        let entry = entry.with_context(|| format!("reading `{}`", dir.display()))?;
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
        } else if Format::from_path(entry.path()).is_some() {
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

// --- errors ---------------------------------------------------------------

/// Free multi-alternative groups with nowhere to write the resulting documents.
#[derive(Debug)]
pub struct AmbiguousGroups {
    entries: Vec<(String, Vec<String>)>,
    pin_command: String,
    out_dir_command: String,
}

impl fmt::Display for AmbiguousGroups {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (id, choices) in &self.entries {
            writeln!(f, "ambiguous group `{id}` — {} alternatives", choices.len())?;
            writeln!(f, "  {}", choices.join(", "))?;
        }
        writeln!(f, "help: {}", self.pin_command)?;
        write!(f, "help: or {}", self.out_dir_command)
    }
}

impl std::error::Error for AmbiguousGroups {}

// --- the command ----------------------------------------------------------

pub fn run(args: &MatrixArgs) -> anyhow::Result<()> {
    let (root, pins) = split_args(&args.args)?;
    let groups = discover(&root)?;
    let axes = resolve_axes(&groups, &pins)?;

    // Checked before anything is materialised, so `matrix` pointed at the wrong
    // directory fails fast instead of after writing half a tree.
    let total = product_size(&axes).filter(|&n| n <= args.max);
    let Some(total) = total else {
        bail!(
            "this tree describes more than {} documents\nhelp: pin a group, or raise --max",
            args.max
        );
    };

    // Writing several documents to stdout is impossible, so free axes are an
    // error unless there is somewhere to put them (or nothing is being written).
    if total > 1 && args.out_dir.is_none() && !args.list {
        return Err(ambiguity_error(&groups, &pins, &root).into());
    }

    let tuples = enumerate(&axes);
    let root_id = root_group_id(&root)?;

    if args.list {
        return list(&groups, &tuples, &root_id, &args.separator);
    }

    // Parse every eligible file once, up front: it surfaces a syntax error
    // before any output is written, and resolves the output format over the
    // whole tree so a mixed tree consistently requires -f rather than failing
    // on only some documents.
    let parsed = parse_all(&groups)?;
    let formats: Vec<Format> = groups
        .iter()
        .flat_map(|g| &g.alternatives)
        .filter_map(|a| Format::from_path(&a.path))
        .collect();
    let out_format = crate::resolve_output_format(args.common.format, &formats)?;
    let opts = crate::merge_options(&args.common);

    for tuple in &tuples {
        let sources: Vec<Source> = layers(&groups, tuple)
            .map(|path| parsed[path].clone())
            .collect();
        let merged = merge_all(sources.iter().map(|(_, v)| v.clone()), &opts)?;
        let text = format::emit(out_format, merged, !args.common.compact, &sources)?;

        match &args.out_dir {
            None => crate::write_stdout(&text)?,
            Some(out_dir) => {
                let path = output_path(
                    out_dir,
                    &name_pairs(&groups, tuple),
                    &root_id,
                    args,
                    out_format,
                );
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating `{}`", parent.display()))?;
                }
                std::fs::write(&path, &text)
                    .with_context(|| format!("writing `{}`", path.display()))?;
            }
        }
    }
    Ok(())
}

/// Splits positionals into the directory and the pins.
///
/// Purely syntactic: an argument containing `=` is a pin, anything else is the
/// directory. Never dependent on whether a file of that name exists, which
/// leaves room for a future `--path` escape hatch.
fn split_args(args: &[String]) -> anyhow::Result<(PathBuf, BTreeMap<String, String>)> {
    let mut dirs = Vec::new();
    let mut pins = BTreeMap::new();

    for arg in args {
        match arg.split_once('=') {
            Some((group, choice)) => {
                if pins.insert(group.to_string(), choice.to_string()).is_some() {
                    bail!("group `{group}` is pinned twice");
                }
            }
            None => dirs.push(PathBuf::from(arg)),
        }
    }

    match dirs.len() {
        0 => bail!("expected a directory\nhelp: knf matrix config/"),
        1 => Ok((dirs.pop().expect("just checked"), pins)),
        _ => {
            let names: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
            bail!(
                "expected one directory, got {}: {}\n\
                 help: arguments containing `=` are group pins; everything else is the directory",
                dirs.len(),
                names.join(", ")
            )
        }
    }
}

fn ambiguity_error(
    groups: &[Group],
    pins: &BTreeMap<String, String>,
    root: &Path,
) -> AmbiguousGroups {
    let free: Vec<&Group> = groups
        .iter()
        .filter(|g| !g.is_singleton() && !pins.contains_key(&g.id))
        .collect();

    // The help line must be runnable as printed, so it pins each free group to
    // its first alternative and carries the pins the user already gave.
    let mut command = format!("knf matrix {}", root.display());
    for (group, choice) in pins {
        command.push_str(&format!(" {group}={choice}"));
    }
    let pin_command = free.iter().fold(command.clone(), |mut acc, group| {
        acc.push_str(&format!(" {}={}", group.id, group.alternatives[0].name));
        acc
    });

    AmbiguousGroups {
        entries: free
            .iter()
            .map(|g| (g.id.clone(), g.choice_names()))
            .collect(),
        pin_command,
        out_dir_command: format!("write every combination — {command} --out-dir out/"),
    }
}

/// The files a tuple merges, shallow -> deep.
fn layers<'a>(groups: &'a [Group], tuple: &'a [usize]) -> impl Iterator<Item = &'a PathBuf> {
    groups
        .iter()
        .zip(tuple)
        .map(|(group, &choice)| &group.alternatives[choice].path)
}

fn parse_all(groups: &[Group]) -> anyhow::Result<HashMap<PathBuf, Source>> {
    let mut parsed = HashMap::new();
    for alternative in groups.iter().flat_map(|g| &g.alternatives) {
        if parsed.contains_key(&alternative.path) {
            continue;
        }
        let format = Format::from_path(&alternative.path).expect("discovery filtered by extension");
        let text = std::fs::read_to_string(&alternative.path)
            .with_context(|| format!("reading `{}`", alternative.path.display()))?;
        let name = SourceName::File(alternative.path.clone());
        let value = format::parse(format, &text, &name)?;
        parsed.insert(alternative.path.clone(), (name, value));
    }
    Ok(parsed)
}

/// `--list` — the answer to "why did this value win" until `--explain` exists.
fn list(
    groups: &[Group],
    tuples: &[Vec<usize>],
    root_id: &str,
    separator: &str,
) -> anyhow::Result<()> {
    let mut out = String::new();
    for tuple in tuples {
        let pairs = name_pairs(groups, tuple);
        let name = if pairs.is_empty() {
            root_id.to_string()
        } else {
            pairs.join(separator)
        };
        out.push_str(&name);
        out.push('\n');
        for path in layers(groups, tuple) {
            out.push_str(&format!("  {}\n", path.display()));
        }
    }
    crate::write_stdout(&out)
}

/// Where one document goes under `--out-dir`.
///
/// Note that a nested group id (`db/tuning`) keeps its slash, so even in flat
/// mode such a tuple lands one directory down. Preserving the id verbatim is
/// what keeps the name reversible; inventing an escape character would not.
fn output_path(
    out_dir: &Path,
    pairs: &[String],
    root_id: &str,
    args: &MatrixArgs,
    format: Format,
) -> PathBuf {
    let mut path = out_dir.to_path_buf();
    let stem = match pairs.split_last() {
        // Nothing to name it after: every group was a singleton.
        None => root_id.to_string(),
        // `--tree` nests all but the last pair as directories — greppable per
        // group, and survives large matrices without 200-character filenames.
        Some((last, leading)) if args.tree => {
            path.extend(leading);
            last.clone()
        }
        Some(_) => pairs.join(&args.separator),
    };
    path.push(format!("{stem}.{}", format.extension()));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_args_is_syntactic() {
        let (dir, pins) = split_args(&[
            "config/".to_string(),
            "db=postgres".to_string(),
            "server=nginx".to_string(),
        ])
        .unwrap();
        assert_eq!(dir, PathBuf::from("config/"));
        assert_eq!(pins["db"], "postgres");
        assert_eq!(pins["server"], "nginx");
    }

    #[test]
    fn split_args_rejects_zero_or_two_directories() {
        assert!(split_args(&["db=x".to_string()]).is_err());
        assert!(split_args(&["a".to_string(), "b".to_string()]).is_err());
    }

    /// 2^64 documents must reach `--max` rather than wrapping to a small
    /// number and quietly writing something.
    #[test]
    fn product_overflow_is_not_a_panic() {
        assert_eq!(product_size(&[]), Some(1));
        assert_eq!(product_size(&[vec![0, 1], vec![0, 1, 2]]), Some(6));
        assert_eq!(product_size(&vec![vec![0usize, 1]; 64]), None);
    }
}
