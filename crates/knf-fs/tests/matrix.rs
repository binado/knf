//! Path enumeration as a plain table test (no filesystem), plus filesystem
//! fixtures for walker behaviour only.

use std::path::{Path, PathBuf};

use knf_fs::{Dir, EntryKind, File, discover, paths, paths_capped, rel};
use tempfile::TempDir;

/// The eligibility predicate used by the filesystem fixtures: `.json` or
/// `.toml`. This mirrors what `knf matrix` passes — the point of `knf-fs` is
/// that the crate itself has no opinion about which files count.
fn is_json_or_toml(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("toml"))
}

fn include_all(_: &str, _: EntryKind) -> bool {
    true
}

fn file(path: &str) -> File {
    File {
        path: PathBuf::from(path),
    }
}

fn dir(path: &str, files: Vec<File>, dirs: Vec<Dir>) -> Dir {
    Dir {
        path: PathBuf::from(path),
        files,
        dirs,
    }
}

fn path_names(tree: &Dir) -> Vec<Vec<String>> {
    paths(tree)
        .into_iter()
        .map(|p| {
            p.into_iter()
                .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        })
        .collect()
}

// --- pure enumeration -----------------------------------------------------

#[test]
fn sibling_dirs_are_alternative_branches() {
    // root/base + foo/{leaf,leaf-two} + bar/{leaf-three,leaf-four}
    let tree = dir(
        "root",
        vec![file("root/base.toml")],
        vec![
            dir(
                "root/bar",
                vec![
                    file("root/bar/leaf-four.toml"),
                    file("root/bar/leaf-three.toml"),
                ],
                vec![],
            ),
            dir(
                "root/foo",
                vec![file("root/foo/leaf-two.toml"), file("root/foo/leaf.toml")],
                vec![],
            ),
        ],
    );
    assert_eq!(
        path_names(&tree),
        vec![
            vec!["base.toml", "leaf-four.toml"],
            vec!["base.toml", "leaf-three.toml"],
            vec!["base.toml", "leaf-two.toml"],
            vec!["base.toml", "leaf.toml"],
        ]
    );
}

#[test]
fn nested_single_child_still_crosses_files_along_the_path() {
    let tree = dir(
        "config",
        vec![file("config/base.toml")],
        vec![dir(
            "config/db",
            vec![
                file("config/db/mysql.toml"),
                file("config/db/postgres.toml"),
            ],
            vec![dir(
                "config/db/tuning",
                vec![
                    file("config/db/tuning/large.toml"),
                    file("config/db/tuning/small.toml"),
                ],
                vec![],
            )],
        )],
    );
    assert_eq!(
        path_names(&tree),
        vec![
            vec!["base.toml", "mysql.toml", "large.toml"],
            vec!["base.toml", "mysql.toml", "small.toml"],
            vec!["base.toml", "postgres.toml", "large.toml"],
            vec!["base.toml", "postgres.toml", "small.toml"],
        ]
    );
}

#[test]
fn singleton_chain_is_one_path() {
    let tree = dir(
        "config",
        vec![file("config/base.toml")],
        vec![dir("config/db", vec![file("config/db/only.toml")], vec![])],
    );
    assert_eq!(path_names(&tree), vec![vec!["base.toml", "only.toml"]]);
}

#[test]
fn directory_with_no_files_passes_through() {
    let tree = dir(
        "outer",
        vec![],
        vec![dir("outer/inner", vec![file("outer/inner/x.toml")], vec![])],
    );
    assert_eq!(path_names(&tree), vec![vec!["x.toml"]]);
}

// --- the walker (filesystem fixtures) -------------------------------------

/// Builds a tree from `("relative/path.toml", contents)` pairs.
fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, contents) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(&full, contents).expect("write");
    }
    dir
}

fn discovered(root: &Path) -> Dir {
    discover(root, is_json_or_toml, include_all, None).expect("discover")
}

fn lineages(root: &Path) -> Vec<Vec<String>> {
    paths(&discovered(root))
        .into_iter()
        .map(|p| p.into_iter().map(|f| rel(root, f)).collect())
        .collect()
}

fn file_names_at(dir: &Dir) -> Vec<String> {
    dir.files
        .iter()
        .map(|f| f.path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn sibling_branches_from_disk() {
    let dir = tree(&[
        ("base.toml", "a=1"),
        ("foo/leaf.toml", "a=1"),
        ("foo/leaf-two.toml", "a=1"),
        ("bar/leaf-three.toml", "a=1"),
        ("bar/leaf-four.toml", "a=1"),
    ]);
    assert_eq!(
        lineages(dir.path()),
        vec![
            vec!["base.toml", "bar/leaf-four.toml"],
            vec!["base.toml", "bar/leaf-three.toml"],
            vec!["base.toml", "foo/leaf-two.toml"],
            vec!["base.toml", "foo/leaf.toml"],
        ]
    );
}

#[test]
fn nested_tuning_is_on_the_db_path() {
    let dir = tree(&[
        ("config/base.toml", "a=1"),
        ("config/db/mysql.toml", "a=1"),
        ("config/db/postgres.toml", "a=1"),
        ("config/db/tuning/small.toml", "a=1"),
        ("config/server/nginx.toml", "a=1"),
    ]);
    let root = dir.path().join("config");
    assert_eq!(
        lineages(&root),
        vec![
            vec!["base.toml", "db/mysql.toml", "db/tuning/small.toml"],
            vec!["base.toml", "db/postgres.toml", "db/tuning/small.toml"],
            vec!["base.toml", "server/nginx.toml"],
        ]
    );
}

#[test]
fn skips_dotfiles_and_dot_directories() {
    let dir = tree(&[
        ("base.toml", "a=1"),
        (".hidden.toml", "a=1"),
        (".git/config.toml", "a=1"),
    ]);
    assert_eq!(lineages(dir.path()), vec![vec!["base.toml"]]);
}

/// A README in a config directory is not an error.
#[test]
fn skips_files_outside_the_extension_allowlist() {
    let dir = tree(&[
        ("base.toml", "a=1"),
        ("over.json", "{}"),
        ("README.md", "hi"),
        ("notes.yaml", "a: 1"),
        ("noextension", "a=1"),
    ]);
    assert_eq!(file_names_at(&discovered(dir.path())), vec!["base", "over"]);
}

#[test]
fn empty_directory_is_an_error_naming_it() {
    let dir = tree(&[("base.toml", "a=1")]);
    std::fs::create_dir(dir.path().join("empty")).expect("mkdir");
    let err = discover(dir.path(), is_json_or_toml, include_all, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("empty"), "{err}");
    assert!(err.contains("no eligible files"), "{err}");
}

/// A directory whose only content is a contributing subdirectory is fine.
#[test]
fn a_directory_contributing_only_via_a_subdirectory_is_not_empty() {
    let dir = tree(&[("outer/inner/x.toml", "a=1")]);
    assert_eq!(lineages(dir.path()), vec![vec!["outer/inner/x.toml"]]);
}

#[test]
fn glob_prunes_unused_branches() {
    let dir = tree(&[
        ("base.toml", "a=1"),
        ("foo/leaf.toml", "a=1"),
        ("foo/leaf-two.toml", "a=1"),
        ("bar/leaf-three.toml", "a=1"),
        ("bar/leaf-four.toml", "a=1"),
    ]);
    let include = |rel: &str, kind: EntryKind| match kind {
        EntryKind::Dir => rel == "foo" || rel.starts_with("foo/"),
        EntryKind::File => rel.starts_with("foo/"),
    };
    let tree = discover(dir.path(), is_json_or_toml, include, None).expect("discover");
    let names: Vec<Vec<String>> = paths(&tree)
        .into_iter()
        .map(|p| p.into_iter().map(|f| rel(dir.path(), f)).collect())
        .collect();
    assert_eq!(
        names,
        vec![
            vec!["base.toml", "foo/leaf-two.toml"],
            vec!["base.toml", "foo/leaf.toml"],
        ]
    );
}

#[test]
fn glob_that_matches_nothing_is_no_match() {
    let dir = tree(&[("base.toml", "a=1"), ("foo/leaf.toml", "a=1")]);
    let include = |rel: &str, kind: EntryKind| match kind {
        EntryKind::Dir => false,
        EntryKind::File => rel.starts_with("missing/"),
    };
    let err = discover(dir.path(), is_json_or_toml, include, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no paths matched"), "{err}");
}

#[test]
fn max_depth_zero_keeps_only_root_files() {
    let dir = tree(&[
        ("base.toml", "a=1"),
        ("foo/leaf.toml", "a=1"),
        ("bar/leaf-three.toml", "a=1"),
    ]);
    let tree = discover(dir.path(), is_json_or_toml, include_all, Some(0)).expect("discover");
    assert!(tree.dirs.is_empty(), "{tree:?}");
    assert_eq!(lineages_of(dir.path(), &tree), vec![vec!["base.toml"]]);
}

fn lineages_of(root: &Path, tree: &Dir) -> Vec<Vec<String>> {
    paths(tree)
        .into_iter()
        .map(|p| p.into_iter().map(|f| rel(root, f)).collect())
        .collect()
}

#[test]
fn max_depth_one_stops_before_nested_tuning() {
    let dir = tree(&[
        ("config/base.toml", "a=1"),
        ("config/db/mysql.toml", "a=1"),
        ("config/db/postgres.toml", "a=1"),
        ("config/db/tuning/small.toml", "a=1"),
        ("config/server/nginx.toml", "a=1"),
    ]);
    let root = dir.path().join("config");
    let tree = discover(&root, is_json_or_toml, include_all, Some(1)).expect("discover");
    assert_eq!(
        lineages_of(&root, &tree),
        vec![
            vec!["base.toml", "db/mysql.toml"],
            vec!["base.toml", "db/postgres.toml"],
            vec!["base.toml", "server/nginx.toml"],
        ]
    );
}

#[test]
fn symlinks_are_not_followed() {
    let dir = tree(&[("base.toml", "a=1"), ("real/r.toml", "a=1")]);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).expect("link");
        std::os::unix::fs::symlink(dir.path().join("real/r.toml"), dir.path().join("l.toml"))
            .expect("link");
    }
    assert_eq!(
        lineages(dir.path()),
        vec![vec!["base.toml", "real/r.toml"],]
    );
}

/// Byte-wise lexicographic, explicitly not natural sort: `10-x` precedes `2-x`.
#[test]
fn sorting_is_byte_wise_not_natural() {
    let dir = tree(&[
        ("2-b.toml", "a=1"),
        ("10-a.toml", "a=1"),
        ("Z-upper.toml", "a=1"),
        ("a-lower.toml", "a=1"),
    ]);
    assert_eq!(
        file_names_at(&discovered(dir.path())),
        vec!["10-a", "2-b", "Z-upper", "a-lower"]
    );
}

#[test]
fn colliding_root_basename_is_just_a_nested_path() {
    let dir = tree(&[("cfg/base.toml", "a=1"), ("cfg/cfg/x.toml", "a=1")]);
    let root = dir.path().join("cfg");
    assert_eq!(lineages(&root), vec![vec!["base.toml", "cfg/x.toml"]]);
}

#[test]
fn paths_capped_from_disk_hits_max() {
    let dir = tree(&[
        ("foo/a.toml", "a=1"),
        ("foo/b.toml", "a=1"),
        ("bar/c.toml", "a=1"),
    ]);
    let tree = discovered(dir.path());
    assert!(paths_capped(&tree, 2).is_err());
    assert_eq!(paths(&tree).len(), 3);
}
