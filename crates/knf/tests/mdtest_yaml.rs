//! Markdown fixture tests: each `resources/mdtest_yaml/*.md` file is one libtest case whose
//! sections run `knf` against inline input files and compare the output.

use std::path::Path;

fn knf(path: &Path) -> datatest_stable::Result<()> {
    mdtest::run_file(
        path,
        &mdtest::Config {
            program: "knf",
            bin: Path::new(env!("CARGO_BIN_EXE_knf")),
        },
    )
}

datatest_stable::harness! {
    { test = knf, root = "resources/mdtest_yaml", pattern = r"\.md$" },
}
