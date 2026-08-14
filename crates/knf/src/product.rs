//! Pure, filesystem-free logic for Cartesian-product mode: recognizing the
//! `x` separator, splitting positionals into factors, enumerating
//! combinations, and deriving output names/labels. Orchestration that
//! touches disk (reading inputs, creating the output directory, writing
//! files) stays in `lib.rs`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use knf_core::Value;
use knf_dotted::PathLeaf;

use crate::STDIN;
use crate::format::{Format, SourceName};

/// The positional that separates two Cartesian-product factors.
const PRODUCT: &str = "x";
/// Leaves room below the common 255-byte filesystem component limit.
const MAX_OUTPUT_NAME_BYTES: usize = 240;

#[derive(Clone)]
pub(super) struct LoadedLayer {
    pub(super) source: SourceName,
    pub(super) value: Value,
    pub(super) label: String,
}

/// One Cartesian-product combination's selected layers and the output name
/// derived from them, computed once and shared between preflight and the
/// write loop so the two can never drift apart.
pub(super) struct Combination<'a> {
    pub(super) selected: Vec<&'a LoadedLayer>,
    pub(super) name: String,
}

pub(super) fn is_product(path: &Path) -> bool {
    path.as_os_str() == OsStr::new(PRODUCT)
}

pub(super) fn split_factors(paths: &[PathBuf]) -> anyhow::Result<Vec<Vec<&Path>>> {
    let mut factors = Vec::new();
    let mut current = Vec::new();

    for path in paths {
        if is_product(path) {
            if current.is_empty() {
                bail!("`x` cannot start a product or follow another `x`: factor is empty");
            }
            factors.push(std::mem::take(&mut current));
        } else {
            current.push(path.as_path());
        }
    }

    if current.is_empty() {
        bail!("`x` cannot end a product: factor is empty");
    }
    factors.push(current);
    Ok(factors)
}

pub(super) fn load_set(path_leaf: &PathLeaf<String>) -> anyhow::Result<LoadedLayer> {
    let expression = path_leaf.to_string();
    reject_unsafe_label(&expression)?;
    let source = SourceName::Set(expression.clone());
    let json = serde_json::Value::from(PathLeaf::<serde_json::Value>::from(path_leaf.clone()));
    Ok(LoadedLayer {
        source,
        value: crate::value::from_json(json),
        label: expression,
    })
}

pub(super) fn enumerate_combinations<'a>(
    factors: &'a [Vec<LoadedLayer>],
    separator: &str,
    format: Format,
) -> anyhow::Result<Vec<Combination<'a>>> {
    let lengths: Vec<usize> = factors.iter().map(Vec::len).collect();
    let total = lengths
        .iter()
        .try_fold(1usize, |total, length| total.checked_mul(*length))
        .context("Cartesian product is too large to enumerate")?;

    let mut combinations = Vec::with_capacity(total);
    for_each_combination(&lengths, |indices| {
        let selected: Vec<&LoadedLayer> = indices
            .iter()
            .enumerate()
            .map(|(factor, choice)| &factors[factor][*choice])
            .collect();
        let name = output_name(&selected, separator, format)?;
        combinations.push(Combination { selected, name });
        Ok(())
    })?;
    Ok(combinations)
}

fn output_name(
    selected: &[&LoadedLayer],
    separator: &str,
    format: Format,
) -> anyhow::Result<String> {
    let mut name = selected
        .iter()
        .map(|layer| layer.label.as_str())
        .collect::<Vec<_>>()
        .join(separator);
    name.push('.');
    name.push_str(format.extension());

    if name.len() > MAX_OUTPUT_NAME_BYTES {
        bail!("generated output name exceeds {MAX_OUTPUT_NAME_BYTES} bytes: `{name}`");
    }
    Ok(name)
}

pub(super) fn file_label(path: &Path) -> anyhow::Result<String> {
    if path.as_os_str() == OsStr::new(STDIN) {
        return Ok("stdin".to_string());
    }
    let stem = path.file_stem().with_context(|| {
        format!(
            "cannot derive an output name from input `{}`",
            path.display()
        )
    })?;
    let stem = stem.to_str().with_context(|| {
        format!(
            "cannot derive an output name from input `{}`: file name is not valid UTF-8",
            path.display()
        )
    })?;
    if stem.is_empty() {
        bail!(
            "cannot derive an output name from input `{}`",
            path.display()
        );
    }
    reject_unsafe_label(stem)?;
    Ok(stem.to_string())
}

/// Guards against a `--set` expression (the one label source not shaped by
/// the filesystem) smuggling a path separator or NUL into a generated name.
/// A no-op for file stems in practice, since a single path component cannot
/// structurally contain these characters.
fn reject_unsafe_label(text: &str) -> anyhow::Result<()> {
    if text.contains(['/', '\\', '\0']) {
        bail!(
            "cannot use `{text}` in an output filename: it contains a path \
             separator or null byte"
        );
    }
    Ok(())
}

fn for_each_combination(
    lengths: &[usize],
    mut visit: impl FnMut(&[usize]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    debug_assert!(!lengths.is_empty());
    debug_assert!(lengths.iter().all(|length| *length > 0));

    let mut indices = vec![0; lengths.len()];
    loop {
        visit(&indices)?;

        let Some(position) = (0..indices.len())
            .rev()
            .find(|position| indices[*position] + 1 < lengths[*position])
        else {
            return Ok(());
        };
        indices[position] += 1;
        for index in &mut indices[position + 1..] {
            *index = 0;
        }
    }
}
