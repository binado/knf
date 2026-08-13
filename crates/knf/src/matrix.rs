//! `knf matrix` — enumerate saturating paths through a config directory and
//! merge each one.
//!
//! Discovery and path enumeration live in the `knf-fs` crate; this module is
//! the command layer that wires them into parse → merge → format → write.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use globset::{GlobBuilder, GlobSetBuilder};
use knf_fs::{self, Dir, EntryKind, File};
use knf_merge::merge_all;

use crate::cli::MatrixArgs;
use crate::format::{self, Format, Source, SourceName};

// --- errors ---------------------------------------------------------------

/// Several matching paths with nowhere to write the resulting documents.
#[derive(Debug)]
pub struct AmbiguousPaths {
    leaves: Vec<String>,
    glob_command: String,
    out_dir_command: String,
}

impl fmt::Display for AmbiguousPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} matching paths", self.leaves.len())?;
        for leaf in &self.leaves {
            writeln!(f, "  {leaf}")?;
        }
        writeln!(f, "help: {}", self.glob_command)?;
        write!(f, "help: or {}", self.out_dir_command)
    }
}

impl std::error::Error for AmbiguousPaths {}

// --- the command ----------------------------------------------------------

pub fn run(args: &MatrixArgs) -> anyhow::Result<()> {
    let root = &args.dir;
    let include = compile_globs(&args.glob)?;
    let tree = knf_fs::discover(
        root,
        |p| Format::from_path(p).is_some(),
        |rel, kind| include(rel, kind),
        args.max_depth,
    )?;

    // Checked before anything is materialised, so `matrix` pointed at the wrong
    // directory fails fast instead of after writing half a tree.
    let lineages = knf_fs::paths_capped(&tree, args.max).map_err(|err| {
        anyhow::anyhow!("{err}\nhelp: tighten --glob, lower --max-depth, or raise --max")
    })?;

    if lineages.len() > 1 && args.out_dir.is_none() && !args.list {
        return Err(ambiguity_error(&lineages, root).into());
    }

    if args.list {
        return list(root, &lineages);
    }

    // Parse every eligible file once, up front: it surfaces a syntax error
    // before any output is written, and resolves the output format over the
    // whole tree so a mixed tree consistently requires -f rather than failing
    // on only some documents.
    let parsed = parse_all(&tree)?;
    let formats: Vec<Format> = files_in(&tree)
        .filter_map(|f| Format::from_path(&f.path))
        .collect();
    let out_format = crate::resolve_output_format(args.common.format, &formats)?;
    let opts = crate::merge_options(&args.common);

    for lineage in &lineages {
        let sources: Vec<Source> = lineage
            .iter()
            .map(|file| parsed[&file.path].clone())
            .collect();
        let merged = merge_all(sources.iter().map(|(_, v)| v.clone()), &opts)?;
        let text = format::emit(out_format, merged, !args.common.compact, &sources)?;

        match &args.out_dir {
            None => crate::write_stdout(&text)?,
            Some(out_dir) => {
                let leaf = lineage.last().expect("paths are non-empty");
                let path = output_path(
                    out_dir,
                    &knf_fs::rel(root, leaf),
                    args.separator.as_deref(),
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

type Include = Box<dyn Fn(&str, EntryKind) -> bool>;

fn compile_globs(patterns: &[String]) -> anyhow::Result<Include> {
    if patterns.is_empty() {
        return Ok(Box::new(|_: &str, _: EntryKind| true));
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid glob `{pattern}`"))?;
        builder.add(glob);
    }
    let set = builder.build().context("building glob set")?;
    let patterns = patterns.to_vec();
    Ok(Box::new(move |rel: &str, kind: EntryKind| match kind {
        EntryKind::File => set.is_match(rel),
        EntryKind::Dir => patterns.iter().any(|p| dir_prefix_may_match(p, rel)),
    }))
}

/// Whether walking into `dir_rel` could still produce a leaf matching `pattern`.
fn dir_prefix_may_match(pattern: &str, dir_rel: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let dir: Vec<&str> = dir_rel.split('/').filter(|s| !s.is_empty()).collect();
    prefix_match(&pat, &dir)
}

fn prefix_match(pat: &[&str], dir: &[&str]) -> bool {
    match (pat.split_first(), dir.split_first()) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some((&"**", rest)), Some((_, drest))) => {
            prefix_match(rest, dir) || prefix_match(pat, drest)
        }
        (Some((p, rest)), Some((d, drest))) => component_match(p, d) && prefix_match(rest, drest),
    }
}

fn component_match(pat: &str, component: &str) -> bool {
    if pat == "**" {
        return true;
    }
    GlobBuilder::new(pat)
        .literal_separator(true)
        .build()
        .map(|g| g.compile_matcher().is_match(component))
        .unwrap_or(false)
}

fn ambiguity_error(lineages: &[Vec<&File>], root: &Path) -> AmbiguousPaths {
    let leaves: Vec<String> = lineages
        .iter()
        .map(|path| knf_fs::rel(root, path.last().expect("paths are non-empty")))
        .collect();
    let dir = root.display();
    let first = leaves.first().map(String::as_str).unwrap_or("*");
    AmbiguousPaths {
        glob_command: format!("knf matrix {dir} --glob '{first}'"),
        out_dir_command: format!("write every path — knf matrix {dir} --out-dir out/"),
        leaves,
    }
}

fn files_in(dir: &Dir) -> impl Iterator<Item = &File> {
    fn walk<'a>(dir: &'a Dir, out: &mut Vec<&'a File>) {
        out.extend(&dir.files);
        for child in &dir.dirs {
            walk(child, out);
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.into_iter()
}

fn parse_all(tree: &Dir) -> anyhow::Result<HashMap<PathBuf, Source>> {
    let mut parsed = HashMap::new();
    for file in files_in(tree) {
        if parsed.contains_key(&file.path) {
            continue;
        }
        let format = Format::from_path(&file.path).expect("discovery filtered by extension");
        let text = std::fs::read_to_string(&file.path)
            .with_context(|| format!("reading `{}`", file.path.display()))?;
        let name = SourceName::File(file.path.clone());
        let value = format::parse(format, &text, &name)?;
        parsed.insert(file.path.clone(), (name, value));
    }
    Ok(parsed)
}

/// `--list` — the answer to "why did this value win" until `--explain` exists.
fn list(root: &Path, lineages: &[Vec<&File>]) -> anyhow::Result<()> {
    let mut out = String::new();
    for lineage in lineages {
        let leaf = lineage.last().expect("paths are non-empty");
        out.push_str(&knf_fs::rel(root, leaf));
        out.push('\n');
        for file in lineage {
            out.push_str(&format!("  {}\n", file.path.display()));
        }
    }
    crate::write_stdout(&out)
}

/// Where one document goes under `--out-dir`.
fn output_path(out_dir: &Path, leaf_rel: &str, separator: Option<&str>, format: Format) -> PathBuf {
    let stem = match leaf_rel.rsplit_once('.') {
        Some((stem, _)) => stem,
        None => leaf_rel,
    };
    let stem = match separator {
        Some(sep) => stem.replace('/', sep),
        None => stem.to_string(),
    };
    let mut path = out_dir.to_path_buf();
    path.push(format!("{stem}.{}", format.extension()));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_prefix_prunes_sibling_branches() {
        assert!(dir_prefix_may_match("foo/**", "foo"));
        assert!(dir_prefix_may_match("foo/leaf.toml", "foo"));
        assert!(!dir_prefix_may_match("foo/**", "bar"));
        assert!(!dir_prefix_may_match("foo/leaf.toml", "bar"));
        assert!(dir_prefix_may_match("**/*.toml", "bar"));
        assert!(dir_prefix_may_match("db/postgres.toml", "db"));
        assert!(!dir_prefix_may_match("db/postgres.toml", "server"));
    }

    #[test]
    fn output_path_preserves_slashes_by_default() {
        let path = output_path(Path::new("out"), "db/mysql.toml", None, Format::Toml);
        assert_eq!(path, PathBuf::from("out/db/mysql.toml"));
    }

    #[test]
    fn output_path_flattens_with_separator() {
        let path = output_path(Path::new("out"), "db/mysql.toml", Some(","), Format::Json);
        assert_eq!(path, PathBuf::from("out/db,mysql.json"));
    }
}
