//! Markdown fixture tests: each `resources/mdtest/*.md` file is one libtest case whose
//! sections run `knf` against inline input files and compare the output.
//!
//! `MDTEST_FILTER=<substring>` selects sections; `MDTEST_UPDATE=1` rewrites expected output.

mod parse;
mod run;

datatest_stable::harness! {
    { test = run::run_file, root = "resources/mdtest", pattern = r"\.md$" },
}
