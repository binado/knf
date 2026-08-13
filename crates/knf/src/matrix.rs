//! `knf matrix` — enumerate a directory tree of config groups and merge each
//! combination.
//!
//! Group discovery and tuple enumeration live in the `knf-fs` crate; this
//! module is the command layer that wires them into parse → merge → format →
//! write.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use knf_fs::{self, Group};
use knf_merge::merge_all;

use crate::cli::MatrixArgs;
use crate::format::{self, Format, Source, SourceName};

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
    let groups = knf_fs::discover(&root, |p| Format::from_path(p).is_some())?;
    let axes = knf_fs::resolve_axes(&groups, &pins)?;

    // Checked before anything is materialised, so `matrix` pointed at the wrong
    // directory fails fast instead of after writing half a tree.
    let total = knf_fs::product_size(&axes).filter(|&n| n <= args.max);
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

    let tuples = knf_fs::enumerate(&axes);
    let root_id = knf_fs::root_group_id(&root)?;

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
                    &knf_fs::name_pairs(&groups, tuple),
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
        let pairs = knf_fs::name_pairs(groups, tuple);
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
}
