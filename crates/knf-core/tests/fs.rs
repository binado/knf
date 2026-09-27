//! Discovery and filtering as reusable library operations.

use std::path::{Path, PathBuf};

use knf::fs::{
    AccumulateError, AccumulateTarget, AccumulateTargetError, GlobError, GlobPattern, accumulate,
    filter_paths,
};

fn target(path: &str) -> AccumulateTarget {
    AccumulateTarget::try_from(PathBuf::from(path)).unwrap()
}

fn tree(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for file in files {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Discovery must not parse contents.
        std::fs::write(path, "not a document").unwrap();
    }
    dir
}

#[test]
fn discovery_visits_only_target_directories_and_places_target_last() {
    let dir = tree(&[
        "root.toml",
        "foo/z.toml",
        "foo/.hidden.TOML",
        "foo/a.toml",
        "foo/other.json",
        "foo/elsewhere/ignored.toml",
        "foo/bar/z.toml",
        "foo/bar/00-target.toml",
    ]);
    std::fs::create_dir(dir.path().join("foo/directory.toml")).unwrap();
    let files = accumulate(&target("./foo/./bar/00-target.toml"), Some(dir.path())).unwrap();
    assert_eq!(
        files,
        [
            "foo/.hidden.TOML",
            "foo/a.toml",
            "foo/z.toml",
            "foo/bar/z.toml",
            "foo/bar/00-target.toml"
        ]
        .map(|p| dir.path().join(p))
    );
    assert_eq!(
        accumulate(&target("root.toml"), Some(dir.path())).unwrap(),
        [dir.path().join("root.toml")]
    );
}

#[test]
fn discovery_crosses_empty_directories_and_selects_json() {
    let dir = tree(&[
        "foo/base.JSON",
        "foo/ignored.toml",
        "foo/empty/end/target.json",
    ]);
    assert_eq!(
        accumulate(&target("foo/empty/end/target.json"), Some(dir.path())).unwrap(),
        [
            dir.path().join("foo/base.JSON"),
            dir.path().join("foo/empty/end/target.json")
        ]
    );
}

#[test]
fn relative_explicit_base_returns_absolute_paths_without_changing_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let dir = tempfile::tempdir_in(&cwd).unwrap();
    std::fs::write(dir.path().join("target.json"), "invalid").unwrap();
    let relative_base = dir.path().strip_prefix(&cwd).unwrap();
    let files = accumulate(&target("target.json"), Some(relative_base)).unwrap();
    assert_eq!(files, [dir.path().join("target.json")]);
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}

#[test]
fn default_base_returns_relative_paths() {
    let cwd = std::env::current_dir().unwrap();
    let dir = tempfile::tempdir_in(&cwd).unwrap();
    std::fs::write(dir.path().join("target.json"), "invalid").unwrap();
    let relative = dir.path().strip_prefix(&cwd).unwrap().join("target.json");
    let target = AccumulateTarget::try_from(relative.clone()).unwrap();
    assert_eq!(accumulate(&target, None).unwrap(), [relative]);
}

#[test]
fn invalid_targets_are_typed_and_never_need_filesystem_access() {
    for (path, expected) in [
        ("-", AccumulateTargetError::Stdin),
        ("foo/../target.toml", AccumulateTargetError::InvalidPath),
        ("target.yaml", AccumulateTargetError::UnknownExtension),
        ("", AccumulateTargetError::UnknownExtension),
    ] {
        let err = AccumulateTarget::try_from(PathBuf::from(path)).unwrap_err();
        assert_eq!(err, expected);
        assert!(!err.to_string().ends_with('\n'));
        assert!(!err.to_string().contains("--"));
    }
    let absolute = std::env::current_dir().unwrap().join("missing.toml");
    assert_eq!(
        AccumulateTarget::try_from(absolute).unwrap_err(),
        AccumulateTargetError::InvalidPath
    );
}

#[test]
fn discovery_failure_retains_path_and_source() {
    let dir = tree(&["foo/base.toml"]);
    let err = accumulate(&target("foo/missing.toml"), Some(dir.path())).unwrap_err();
    assert!(!err.to_string().ends_with('\n'));
    match err {
        AccumulateError::Inspect { path, source } => {
            assert_eq!(path, dir.path().join("foo/missing.toml"));
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("unexpected error: {other}"),
    }
    std::fs::create_dir(dir.path().join("foo/directory.toml")).unwrap();
    assert!(
        matches!(accumulate(&target("foo/directory.toml"), Some(dir.path())),
        Err(AccumulateError::Directory { path }) if path == dir.path().join("foo/directory.toml"))
    );
}

#[test]
fn filtering_keeps_spelling_order_duplicates_and_nonexistent_candidates() {
    let files = [
        "./foo/prod.toml",
        "foo/dev.toml",
        "foo/prod.toml",
        "foo/prod.toml",
        "-",
    ];
    let pattern = "foo/*.toml".parse::<GlobPattern>().unwrap();
    let selected = filter_paths(&files, &pattern, false);
    assert_eq!(
        selected,
        files[1..4].iter().map(PathBuf::from).collect::<Vec<_>>()
    );
    let pattern = "{prod.toml,-}".parse::<GlobPattern>().unwrap();
    let selected = filter_paths(&files, &pattern, true);
    assert_eq!(
        selected,
        [files[0], files[2], files[3], files[4]].map(PathBuf::from)
    );
    assert_eq!(selected[0].as_os_str(), files[0]);
    assert!(filter_paths(&files, &"missing".parse().unwrap(), false).is_empty());
    assert!(filter_paths::<PathBuf>(&[], &pattern, true).is_empty());
}

#[test]
fn patterns_cover_case_negation_classes_braces_and_native_bytes() {
    let files = ["defaults.toml", "prod.toml", "Prod.toml", "dev.toml"];
    for (pattern, expected) in [
        ("{defaults,prod}.toml", vec!["defaults.toml", "prod.toml"]),
        ("[pd]??.toml", vec!["dev.toml"]),
        ("!{defaults,prod}.toml", vec!["Prod.toml", "dev.toml"]),
    ] {
        assert_eq!(
            filter_paths(&files, &pattern.parse().unwrap(), true),
            expected.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }
    assert!(
        !"!prod.toml"
            .parse::<GlobPattern>()
            .unwrap()
            .matches_filename(Path::new(".."))
    );
    assert!(
        !"?.toml"
            .parse::<GlobPattern>()
            .unwrap()
            .matches_filename(Path::new("é.toml"))
    );
    assert!(
        "??.toml"
            .parse::<GlobPattern>()
            .unwrap()
            .matches_filename(Path::new("é.toml"))
    );
    let _: GlobError = "[".parse::<GlobPattern>().unwrap_err();
}

#[cfg(unix)]
#[test]
fn non_regular_targets_are_distinct_from_directories() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("socket.toml");
    let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
    assert!(
        matches!(accumulate(&target("socket.toml"), Some(dir.path())),
        Err(AccumulateError::NonRegular { path: actual }) if actual == path)
    );
}

#[cfg(unix)]
#[test]
fn symlink_aliases_remain_separate_and_broken_links_abort_discovery() {
    use std::os::unix::fs::symlink;
    let dir = tree(&["outside/base.toml", "outside/target.toml"]);
    symlink("outside", dir.path().join("foo")).unwrap();
    symlink("target.toml", dir.path().join("outside/alias.toml")).unwrap();
    assert_eq!(
        accumulate(&target("foo/target.toml"), Some(dir.path())).unwrap(),
        ["foo/alias.toml", "foo/base.toml", "foo/target.toml"].map(|p| dir.path().join(p))
    );
    symlink("missing", dir.path().join("outside/broken.toml")).unwrap();
    assert!(
        matches!(accumulate(&target("foo/target.toml"), Some(dir.path())),
        Err(AccumulateError::Inspect { path, .. }) if path == dir.path().join("foo/broken.toml"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_survive_discovery_and_filtering() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tree(&["foo/base.json"]);
    let filename = std::ffi::OsString::from_vec(b"target-\xff.json".to_vec());
    let relative = Path::new("foo").join(filename);
    std::fs::write(dir.path().join(&relative), "invalid").unwrap();
    let files = accumulate(
        &AccumulateTarget::try_from(relative.clone()).unwrap(),
        Some(dir.path()),
    )
    .unwrap();
    assert_eq!(
        filter_paths(&files, &"target-?.json".parse().unwrap(), true),
        [dir.path().join(relative)]
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_candidates_are_matched_without_replacement_characters() {
    use std::os::unix::ffi::OsStringExt;
    let input = PathBuf::from(std::ffi::OsString::from_vec(b"target-\xff.json".to_vec()));
    assert_eq!(
        filter_paths(&[&input], &"target-?.json".parse().unwrap(), true),
        [input]
    );
}

#[cfg(windows)]
#[test]
fn windows_root_relative_base_produces_absolute_results() {
    let cwd = std::env::current_dir().unwrap();
    let dir = tempfile::tempdir_in(&cwd).unwrap();
    std::fs::write(dir.path().join("target.toml"), "invalid").unwrap();
    let base: PathBuf = dir.path().components().skip(1).collect();
    assert!(base.has_root() && !base.is_absolute());
    let files = accumulate(&target("target.toml"), Some(&base)).unwrap();
    assert_eq!(files, [dir.path().join("target.toml")]);
    assert!(files[0].is_absolute());
}

#[cfg(windows)]
#[test]
fn windows_separators_are_normalized_only_for_matching() {
    let input = PathBuf::from(r"foo\prod.toml");
    let selected = filter_paths(&[&input], &"foo/*.toml".parse().unwrap(), false);
    assert_eq!(selected[0].as_os_str(), input.as_os_str());
}
