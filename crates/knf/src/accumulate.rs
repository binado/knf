//! Expand one relative CLI target into its ordered file arguments.

use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use clap::builder::{OsStringValueParser, TypedValueParser, ValueParser};
use knf::Format;

/// A relative JSON or TOML path accepted as the value of `--accumulate`.
///
/// `CurDir` components are stripped, so `./foo/./bar/target.toml` is stored as
/// `foo/bar/target.toml`. The format is the one [`Format::from_path`] accepted;
/// discovery does not look the extension up again.
#[derive(Debug, Clone)]
pub struct AccumulateTarget {
    path: PathBuf,
    format: Format,
}

impl AccumulateTarget {
    /// Rejects stdin, absolute paths, `..`, and a missing JSON/TOML extension.
    pub fn parse(raw: OsString) -> Result<Self, String> {
        let raw_path = Path::new(&raw);
        if raw_path.as_os_str() == knf::STDIN {
            return Err("--accumulate does not accept stdin".into());
        }
        if raw_path.components().any(|component| {
            matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::ParentDir
            )
        }) {
            return Err(
                "--accumulate requires a relative target path without .. components".into(),
            );
        }
        let path: PathBuf = raw_path
            .components()
            .filter(|component| !matches!(component, Component::CurDir))
            .collect();
        let format = Format::from_path(&path).ok_or_else(|| {
            "--accumulate requires a target with a JSON or TOML extension".to_string()
        })?;
        Ok(Self { path, format })
    }
}

/// clap value parser for `--accumulate`. Takes [`OsString`] so a non-UTF-8
/// filename survives.
pub fn parser() -> ValueParser {
    OsStringValueParser::new()
        .try_map(AccumulateTarget::parse)
        .into()
}

pub fn accumulate(target: &AccumulateTarget) -> anyhow::Result<Vec<PathBuf>> {
    let format = target.format;
    let target_path = &target.path;
    let metadata = fs::metadata(target_path)
        .with_context(|| format!("inspecting `{}`", target_path.display()))?;
    if !metadata.is_file() {
        bail!("`{}` is not a regular file", target_path.display());
    }

    let mut files = Vec::new();
    let mut directory = PathBuf::new();
    // `parent` is `Some` for every path `parse` accepts: `None` is only an
    // empty path or a root, both rejected above. A bare filename's parent is
    // `""`, so this loop does not visit the working directory.
    if let Some(parent) = target_path.parent() {
        for component in parent.components() {
            directory.push(component);
            let entries = fs::read_dir(&directory)
                .with_context(|| format!("listing `{}`", directory.display()))?;
            let mut matching = Vec::new();
            for entry in entries {
                let entry = entry.with_context(|| format!("listing `{}`", directory.display()))?;
                let path = entry.path();
                if path == *target_path || Format::from_path(&path) != Some(format) {
                    continue;
                }
                // metadata follows file symlinks and preserves inspection errors.
                let metadata = fs::metadata(&path)
                    .with_context(|| format!("inspecting `{}`", path.display()))?;
                if metadata.is_file() {
                    matching.push((entry.file_name(), path));
                }
            }
            matching.sort_by(|left, right| left.0.cmp(&right.0));
            files.extend(matching.into_iter().map(|(_, path)| path));
        }
    }
    files.push(target_path.clone());
    Ok(files)
}
