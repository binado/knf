"""Path filtering through the installed Python API."""

import os
from pathlib import Path

import pytest
from knf import ParseError, filter_paths, load


def test_filter_matches_supplied_spelling_before_path_normalization():
    files = ["./foo/prod.toml", Path("foo/dev.toml"), "foo/prod.toml", "foo/prod.toml"]
    assert filter_paths(files, "foo/*.toml") == [
        Path("foo/dev.toml"), Path("foo/prod.toml"), Path("foo/prod.toml"),
    ]
    assert filter_paths(files, "./foo/*.toml") == [Path("foo/prod.toml")]
    assert filter_paths(files, "prod.toml", filename_only=True) == [Path("foo/prod.toml")] * 3
    assert filter_paths(files, "missing") == []
    assert filter_paths([], "*") == []
    assert load(filter_paths(files, "missing")) == {}


@pytest.mark.parametrize(
    ("pattern", "expected"),
    [
        ("{defaults,prod}.toml", ["defaults.toml", "prod.toml"]),
        ("[pd]??.toml", ["dev.toml"]),
        ("!{defaults,prod}.toml", ["Prod.toml", "dev.toml"]),
    ],
)
def test_filter_patterns_are_case_sensitive(pattern, expected):
    files = ["defaults.toml", "prod.toml", "Prod.toml", "dev.toml"]
    assert filter_paths(files, pattern, filename_only=True) == list(map(Path, expected))


def test_filter_special_candidates_and_invalid_patterns():
    assert filter_paths(["-", ".", ".."], "!prod.toml", filename_only=True) == [Path("-")]
    assert filter_paths(["é.toml"], "?.toml") == []
    assert filter_paths(["é.toml"], "??.toml") == [Path("é.toml")]
    with pytest.raises(ValueError) as info:
        filter_paths([], "[")
    assert not isinstance(info.value, ParseError)


@pytest.mark.skipif(os.name != "nt", reason="needs Windows separators")
def test_windows_separators_are_normalized_only_for_matching():
    assert filter_paths([r"foo\prod.toml"], "foo/*.toml") == [Path(r"foo\prod.toml")]

