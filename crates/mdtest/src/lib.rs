//! Markdown fixture harness: each section of a `.md` file is a command-line test.
//! See the crate README for the format.

mod parse;
mod run;

use std::path::Path;

/// The binary under test.
pub struct Config<'a> {
    /// Name expected after `$ ` in console blocks, e.g. `knf`.
    pub program: &'a str,
    /// Executable actually run.
    pub bin: &'a Path,
}

/// Runs every selected section of the fixture at `path`, or rewrites its expected output
/// when `MDTEST_UPDATE` is set.
pub fn run_file(path: &Path, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    run::run_file(path, config)
}
