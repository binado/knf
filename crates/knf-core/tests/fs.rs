//! Path filtering as a reusable library operation.

use std::path::{Path, PathBuf};

use knf::fs::{GlobError, GlobPattern, filter_paths};

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
fn windows_separators_are_normalized_only_for_matching() {
    let input = PathBuf::from(r"foo\prod.toml");
    let selected = filter_paths(&[&input], &"foo/*.toml".parse().unwrap(), false);
    assert_eq!(selected[0].as_os_str(), input.as_os_str());
}
