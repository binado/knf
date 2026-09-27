"""Discovery and filtering through the installed Python API."""

import errno
import inspect
import os
import sys
from pathlib import Path

import pytest

from knf import ParseError, accumulate, filter_paths, load


@pytest.fixture
def tree(tmp_path):
    files = {
        "excluded.toml": "excluded = true\n",
        "foo/z.toml": "value = 1\n",
        "foo/.hidden.TOML": "hidden = true\n",
        "foo/base.toml": "base = true\n",
        "foo/other.json": "not JSON",
        "foo/elsewhere/ignored.toml": "ignored = true\n",
        "foo/bar/z.toml": "value = 2\n",
        "foo/bar/00-target.toml": "value = 3\ncopy = '${value}'\n",
    }
    for name, text in files.items():
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    (tmp_path / "foo" / "directory.toml").mkdir()
    return tmp_path


def test_signatures_are_keyword_only():
    assert str(inspect.signature(accumulate)) == "(target, *, base_dir=None)"
    assert str(inspect.signature(filter_paths)) == "(files, pattern, *, filename_only=False)"
    with pytest.raises(TypeError):
        accumulate("target.toml", ".")
    with pytest.raises(TypeError):
        filter_paths([], "*", True)


def test_explicit_base_returns_ordered_absolute_paths_and_composes_with_load(tree):
    cwd = Path.cwd()
    files = accumulate("./foo/./bar/00-target.toml", base_dir=tree)
    assert files == [
        tree / "foo/.hidden.TOML",
        tree / "foo/base.toml",
        tree / "foo/z.toml",
        tree / "foo/bar/z.toml",
        tree / "foo/bar/00-target.toml",
    ]
    assert all(isinstance(path, Path) and path.is_absolute() for path in files)
    assert Path.cwd() == cwd
    assert load(files, interpolate=True) == {
        "hidden": True, "base": True, "value": 3, "copy": 3,
    }
    selected = filter_paths(files, "{base,00-target}.toml", filename_only=True)
    assert load(selected, interpolate=True) == {"base": True, "value": 3, "copy": 3}


def test_default_and_relative_bases(tree, monkeypatch):
    monkeypatch.chdir(tree)
    relative = accumulate(Path("foo/bar/00-target.toml"))
    absolute = accumulate("foo/bar/00-target.toml", base_dir=".")
    assert [tree / path for path in relative] == absolute
    assert all(not path.is_absolute() for path in relative)
    assert Path.cwd() == tree
    assert accumulate("excluded.toml", base_dir="") == [tree / "excluded.toml"]
    assert accumulate("excluded.toml") == [Path("excluded.toml")]
    assert accumulate("bar/00-target.toml", base_dir="foo") == [
        tree / "foo/bar/z.toml", tree / "foo/bar/00-target.toml",
    ]


class CustomPath:
    def __init__(self, spelling):
        self.spelling = spelling

    def __fspath__(self):
        return self.spelling


def test_custom_pathlike_inputs(tree):
    assert accumulate(CustomPath("excluded.toml"), base_dir=CustomPath(str(tree))) == [
        tree / "excluded.toml",
    ]
    assert filter_paths([CustomPath("foo/prod.toml")], "foo/*.toml") == [
        Path("foo/prod.toml"),
    ]


@pytest.mark.parametrize("target", ["-", "../missing.toml", "foo/../missing.toml", "missing.yaml", ""])
def test_invalid_targets_fail_before_filesystem_access(tmp_path, target):
    with pytest.raises(ValueError) as info:
        accumulate(target, base_dir=tmp_path / "missing-base")
    assert not isinstance(info.value, ParseError)
    assert "--accumulate" not in str(info.value)


def test_absolute_target_is_rejected(tmp_path):
    with pytest.raises(ValueError, match="relative target"):
        accumulate(tmp_path / "missing.toml")


def test_missing_and_directory_targets_have_errno_and_filename(tmp_path):
    missing = tmp_path / "missing.toml"
    with pytest.raises(FileNotFoundError) as info:
        accumulate("missing.toml", base_dir=tmp_path)
    assert info.value.errno == errno.ENOENT
    assert info.value.filename == str(missing)
    directory = tmp_path / "directory.toml"
    directory.mkdir()
    with pytest.raises(IsADirectoryError) as info:
        accumulate("directory.toml", base_dir=tmp_path)
    assert info.value.errno == errno.EISDIR
    assert info.value.filename == str(directory)


def test_file_used_as_base_raises_not_a_directory(tmp_path):
    base = tmp_path / "file"
    base.write_text("text")
    with pytest.raises(NotADirectoryError) as info:
        accumulate("target.toml", base_dir=base)
    assert info.value.errno == errno.ENOTDIR
    assert info.value.filename == str(base / "target.toml")


@pytest.mark.skipif(not hasattr(os, "mkfifo"), reason="needs POSIX FIFO")
def test_other_non_regular_target_is_a_value_error(tmp_path):
    os.mkfifo(tmp_path / "pipe.toml")
    with pytest.raises(ValueError, match="not a regular file"):
        accumulate("pipe.toml", base_dir=tmp_path)


@pytest.mark.skipif(os.name != "posix", reason="needs POSIX symlinks")
def test_symlinks_preserve_aliases_and_errors_abort_discovery(tree):
    (tree / "alias").symlink_to("foo", target_is_directory=True)
    (tree / "foo/bar/alias.toml").symlink_to("00-target.toml")
    files = accumulate("alias/bar/00-target.toml", base_dir=tree)
    assert tree / "alias/bar/alias.toml" in files
    assert files[-1] == tree / "alias/bar/00-target.toml"
    (tree / "foo/broken.toml").symlink_to("missing")
    with pytest.raises(FileNotFoundError) as info:
        accumulate("alias/bar/00-target.toml", base_dir=tree)
    assert info.value.filename == str(tree / "alias/broken.toml")


@pytest.mark.skipif(
    not hasattr(os, "geteuid") or os.geteuid() == 0,
    reason="needs POSIX permissions, which root ignores",
)
def test_permission_errors_are_os_errors(tree):
    directory = tree / "foo/bar"
    # Target inspection can succeed, while directory listing must fail.
    directory.chmod(0o111)
    try:
        with pytest.raises(PermissionError) as info:
            accumulate("foo/bar/00-target.toml", base_dir=tree)
    finally:
        directory.chmod(0o755)
    assert info.value.errno == errno.EACCES
    assert info.value.filename == str(directory)


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


@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="needs a filesystem accepting non-UTF-8 names")
def test_non_utf8_paths_round_trip_in_results_and_exceptions(tmp_path):
    (tmp_path / "foo").mkdir()
    name = os.fsdecode(b"target-\xff.json")
    target = tmp_path / "foo" / name
    target.write_text('{"value":1}')
    files = accumulate("foo/" + name, base_dir=tmp_path)
    assert files == [target]
    assert os.fsencode(files[0]) == os.fsencode(target)
    assert filter_paths(files, "target-?.json", filename_only=True) == [target]
    assert load(files) == {"value": 1}
    target.unlink()
    with pytest.raises(FileNotFoundError) as info:
        accumulate("foo/" + name, base_dir=tmp_path)
    assert os.fsencode(info.value.filename) == os.fsencode(target)


@pytest.mark.skipif(os.name != "posix", reason="needs native byte filenames")
def test_non_utf8_candidates_and_error_filenames_without_filesystem_creation(tmp_path):
    spelling = "foo/" + os.fsdecode(b"target-\xff.json")
    assert filter_paths([spelling], "target-?.json", filename_only=True) == [Path(spelling)]
    with pytest.raises(FileNotFoundError) as info:
        accumulate(spelling, base_dir=tmp_path)
    assert os.fsencode(info.value.filename) == os.fsencode(tmp_path / spelling)


@pytest.mark.skipif(os.name != "nt", reason="needs Windows separators")
def test_windows_separators_are_normalized_only_for_matching():
    assert filter_paths([r"foo\prod.toml"], "foo/*.toml") == [Path(r"foo\prod.toml")]
