//! Expand a validated CLI target into its ordered file arguments.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use knf::Format;

pub fn accumulate(target: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let target: PathBuf = target
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect();
    let format = Format::from_path(&target).expect("validated target extension");
    let metadata =
        fs::metadata(&target).with_context(|| format!("inspecting `{}`", target.display()))?;
    if !metadata.is_file() {
        bail!("`{}` is not a regular file", target.display());
    }

    let mut files = Vec::new();
    let mut directory = PathBuf::new();
    for component in target.parent().expect("target has a parent").components() {
        directory.push(component);
        let entries = fs::read_dir(&directory)
            .with_context(|| format!("listing `{}`", directory.display()))?;
        let mut matching = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| format!("listing `{}`", directory.display()))?;
            let path = entry.path();
            if path == target || Format::from_path(&path) != Some(format) {
                continue;
            }
            // metadata follows file symlinks and preserves inspection errors.
            let metadata =
                fs::metadata(&path).with_context(|| format!("inspecting `{}`", path.display()))?;
            if metadata.is_file() {
                matching.push(path);
            }
        }
        matching.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
        files.extend(matching);
    }
    files.push(target);
    Ok(files)
}
