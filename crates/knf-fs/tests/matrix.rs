//! Tuple enumeration as a plain table test (no filesystem), plus filesystem
//! fixtures for walker behaviour only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use knf_fs::{Alternative, Group, discover, enumerate, name_pairs, resolve_axes};
use tempfile::TempDir;

/// The eligibility predicate used by the filesystem fixtures: `.json` or
/// `.toml`. This mirrors what `knf matrix` passes — the point of `knf-fs` is
/// that the crate itself has no opinion about which files count.
fn is_json_or_toml(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("toml"))
}

// --- pure enumeration -----------------------------------------------------

fn group(id: &str, alternatives: &[&str]) -> Group {
    Group {
        id: id.to_string(),
        alternatives: alternatives
            .iter()
            .map(|name| Alternative {
                name: (*name).to_string(),
                path: PathBuf::from(format!("{id}/{name}.toml")),
            })
            .collect(),
    }
}

struct Case {
    name: &'static str,
    /// Groups in DFS order, as the walker would produce them.
    groups: &'static [(&'static str, &'static [&'static str])],
    pins: &'static [(&'static str, &'static str)],
    /// One entry per resolved tuple, rendered as its output name.
    expect: &'static [&'static str],
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    Case {
        name: "singletons auto-select and name nothing",
        groups: &[("config", &["base"]), ("db", &["only"])],
        pins: &[],
        expect: &[""],
    },
    // The §4.3 worked example: 2 x 2 = 4 documents.
    Case {
        name: "two free axes cross-product",
        groups: &[("config", &["base"]), ("db", &["mysql", "postgres"]), ("server", &["apache", "nginx"])],
        pins: &[],
        expect: &[
            "db=mysql,server=apache",
            "db=mysql,server=nginx",
            "db=postgres,server=apache",
            "db=postgres,server=nginx",
        ],
    },
    Case {
        name: "pinning one axis leaves the other free",
        groups: &[("config", &["base"]), ("db", &["mysql", "postgres"]), ("server", &["apache", "nginx"])],
        pins: &[("db", "postgres")],
        // The pinned group stays in the name: dropping it would make two
        // different tuples share a filename once a second axis is pinned too.
        expect: &["db=postgres,server=apache", "db=postgres,server=nginx"],
    },
    Case {
        name: "everything pinned is one document",
        groups: &[("config", &["base"]), ("db", &["mysql", "postgres"]), ("server", &["apache", "nginx"])],
        pins: &[("db", "postgres"), ("server", "nginx")],
        expect: &["db=postgres,server=nginx"],
    },
    Case {
        name: "nested groups are independent axes",
        groups: &[("config", &["base"]), ("db", &["mysql", "postgres"]), ("db/tuning", &["large", "small"])],
        expect: &[
            "db=mysql,db/tuning=large",
            "db=mysql,db/tuning=small",
            "db=postgres,db/tuning=large",
            "db=postgres,db/tuning=small",
        ],
        pins: &[],
    },
    Case {
        name: "three axes",
        groups: &[("a", &["1", "2"]), ("b", &["1", "2"]), ("c", &["1", "2"])],
        pins: &[],
        expect: &[
            "a=1,b=1,c=1", "a=1,b=1,c=2", "a=1,b=2,c=1", "a=1,b=2,c=2",
            "a=2,b=1,c=1", "a=2,b=1,c=2", "a=2,b=2,c=1", "a=2,b=2,c=2",
        ],
    },
    Case {
        name: "pinning a singleton is allowed",
        groups: &[("config", &["base"])],
        pins: &[("config", "base")],
        expect: &[""],
    },
];

#[test]
fn enumeration_table() {
    for case in CASES {
        let groups: Vec<Group> = case
            .groups
            .iter()
            .map(|(id, alts)| group(id, alts))
            .collect();
        let pins: BTreeMap<String, String> = case
            .pins
            .iter()
            .map(|(g, c)| ((*g).to_string(), (*c).to_string()))
            .collect();

        let axes = resolve_axes(&groups, &pins).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let names: Vec<String> = enumerate(&axes)
            .iter()
            .map(|tuple| name_pairs(&groups, tuple).join(","))
            .collect();

        assert_eq!(names, case.expect, "case `{}`", case.name);
    }
}

/// Names are sorted by group id, so walk order cannot change a filename.
#[test]
fn names_are_independent_of_group_order() {
    let forward = vec![group("db", &["mysql", "postgres"]), group("app", &["web"])];
    let reversed = vec![forward[1].clone(), forward[0].clone()];
    assert_eq!(
        name_pairs(&forward, &[1, 0]),
        name_pairs(&reversed, &[0, 1])
    );
}

#[test]
fn unknown_pins_are_rejected() {
    let groups = vec![group("db", &["mysql", "postgres"])];
    let pin = |g: &str, c: &str| BTreeMap::from([(g.to_string(), c.to_string())]);

    let err = resolve_axes(&groups, &pin("cache", "redis")).unwrap_err();
    assert!(err.to_string().contains("no group `cache`"), "{err}");

    let err = resolve_axes(&groups, &pin("db", "oracle")).unwrap_err();
    assert!(err.to_string().contains("no alternative `oracle`"), "{err}");
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

/// Group ids paired with their alternative names, in discovery order.
fn shape(root: &Path) -> Vec<(String, Vec<String>)> {
    discover(root, is_json_or_toml)
        .expect("discover")
        .into_iter()
        .map(|g| (g.id, g.alternatives.into_iter().map(|a| a.name).collect()))
        .collect()
}

/// The root's file group is named by the root basename; subdirectories by their
/// path relative to it. A directory's own group precedes its subdirectories.
#[test]
fn discovery_order_and_ids() {
    let dir = tree(&[
        ("config/base.toml", "a=1"),
        ("config/db/mysql.toml", "a=1"),
        ("config/db/postgres.toml", "a=1"),
        ("config/db/tuning/small.toml", "a=1"),
        ("config/server/nginx.toml", "a=1"),
    ]);

    assert_eq!(
        shape(&dir.path().join("config")),
        vec![
            ("config".to_string(), vec!["base".to_string()]),
            (
                "db".to_string(),
                vec!["mysql".to_string(), "postgres".to_string()]
            ),
            ("db/tuning".to_string(), vec!["small".to_string()]),
            ("server".to_string(), vec!["nginx".to_string()]),
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
    let shape = shape(dir.path());
    assert_eq!(shape.len(), 1, "{shape:?}");
    assert_eq!(shape[0].1, vec!["base".to_string()]);
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
    assert_eq!(
        shape(dir.path())[0].1,
        vec!["base".to_string(), "over".to_string()]
    );
}

#[test]
fn empty_directory_is_an_error_naming_it() {
    let dir = tree(&[("base.toml", "a=1")]);
    std::fs::create_dir(dir.path().join("empty")).expect("mkdir");
    let err = discover(dir.path(), is_json_or_toml)
        .unwrap_err()
        .to_string();
    assert!(err.contains("empty"), "{err}");
    assert!(err.contains("no eligible files"), "{err}");
}

/// A directory whose only content is a contributing subdirectory is fine.
#[test]
fn a_directory_contributing_only_via_a_subdirectory_is_not_empty() {
    let dir = tree(&[("outer/inner/x.toml", "a=1")]);
    assert_eq!(
        shape(dir.path()),
        vec![("outer/inner".to_string(), vec!["x".to_string()])]
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
    let shape = shape(dir.path());
    assert_eq!(shape.len(), 2, "{shape:?}");
    assert_eq!(shape[0].1, vec!["base".to_string()]);
    assert_eq!(shape[1].0, "real");
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
        shape(dir.path())[0].1,
        vec![
            "10-a".to_string(),
            "2-b".to_string(),
            "Z-upper".to_string(),
            "a-lower".to_string(),
        ]
    );
}

#[test]
fn root_basename_colliding_with_a_subdirectory_is_an_error() {
    let dir = tree(&[("cfg/base.toml", "a=1"), ("cfg/cfg/x.toml", "a=1")]);
    let err = discover(&dir.path().join("cfg"), is_json_or_toml)
        .unwrap_err()
        .to_string();
    assert!(err.contains("defined twice"), "{err}");
}
