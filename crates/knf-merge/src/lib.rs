//! Layered merge over JSON-like value trees.
//!
//! Format-specific walks live behind Cargo features: `merge_json` /
//! `merge_all_json` (`json`) and `merge_toml` / `merge_all_toml` (`toml`). This
//! crate knows nothing about files or the command line; provenance is the
//! caller's job.
//!
//! `thiserror` is the only hard dependency. `serde_json` and `toml` are optional.

#[cfg(any(feature = "json", feature = "toml"))]
mod strict;

/// Knobs on the merge itself. Passed by reference rather than encoded as cargo
/// features: features are additive and unify across a dependency graph, so a
/// `strict` feature would silently change behaviour for one consumer the moment
/// a second consumer enabled it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeOptions {
    pub strict: bool,
}

impl MergeOptions {
    /// The default: last layer wins, no type checking.
    pub const LAST_WINS: Self = Self { strict: false };
    /// Error when a layer changes the kind of an existing key.
    pub const STRICT: Self = Self { strict: true };
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    /// A layer replaced an existing key with a value of a different kind.
    ///
    /// Carries a key path and nothing else — no filenames, no layer indices.
    #[error(
        "type conflict at `{}`: {expected} would be replaced by {found}",
        render_path(path)
    )]
    TypeConflict {
        path: Vec<String>,
        expected: &'static str,
        found: &'static str,
    },
}

impl MergeError {
    /// The dotted key path the conflict occurred at.
    pub fn path(&self) -> &[String] {
        match self {
            Self::TypeConflict { path, .. } => path,
        }
    }
}

/// Renders a key path for display. An empty path is the document root.
fn render_path(path: &[String]) -> String {
    if path.is_empty() {
        "<root>".to_string()
    } else {
        path.join(".")
    }
}

#[cfg(feature = "json")]
mod json;
#[cfg(feature = "json")]
pub use json::{json_kind, merge_all_json, merge_json};

#[cfg(feature = "toml")]
mod toml;
#[cfg(feature = "toml")]
pub use toml::{merge_all_toml, merge_toml, toml_kind};
