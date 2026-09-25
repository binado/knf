"""`knf.deep_merge` and the `knf` executable, as an installed wheel exposes them."""

import datetime
import errno
import json
import os
import shutil
import subprocess
import sys
from collections import UserDict

import pytest

from knf import ParseError, deep_merge


@pytest.fixture
def write(tmp_path):
    def write(name, text):
        path = tmp_path / name
        path.write_text(text)
        return path

    return write


def test_one_file_is_the_identity(write):
    doc = {"b": 1, "a": {"z": [1, 2], "y": None}, "c": 1.5}
    path = write("a.json", json.dumps(doc))
    merged = deep_merge([path])
    assert merged == doc
    assert list(merged) == ["b", "a", "c"]


def test_files_fold_left_to_right_across_formats(write):
    base = write("base.toml", 'name = "app"\n[server]\nhost = "localhost"\nport = 80\n')
    prod = write("prod.json", '{"server": {"port": 443}}')
    assert deep_merge([base, prod]) == {
        "name": "app",
        "server": {"host": "localhost", "port": 443},
    }
    assert deep_merge([prod, base])["server"]["port"] == 80


def test_paths_may_be_str(write):
    path = write("a.json", '{"a": 1}')
    assert deep_merge([str(path)]) == {"a": 1}


def test_override_is_merged_last(write):
    path = write("a.json", '{"server": {"host": "h", "port": 80}}')
    merged = deep_merge([path], override={"server": {"port": 8080}, "debug": True})
    assert merged == {"server": {"host": "h", "port": 8080}, "debug": True}


def test_override_alone():
    override = {"a": [1, "two", 3.0, None, {"b": False}], "t": (1, 2)}
    assert deep_merge([], override=override) == {
        "a": [1, "two", 3.0, None, {"b": False}],
        "t": [1, 2],
    }


def test_arrays_replace_and_none_overwrites(write):
    path = write("a.json", '{"xs": [1, 2, 3], "k": {"v": 1}}')
    merged = deep_merge([path], override={"xs": [9], "k": None})
    assert merged == {"xs": [9], "k": None}


def test_bool_stays_bool():
    merged = deep_merge([], override={"t": True, "one": 1})
    assert merged["t"] is True
    assert type(merged["one"]) is int


@pytest.mark.parametrize("n", [-(2**63), 2**63 - 1, 2**64 - 1])
def test_64_bit_integers_survive(n):
    assert deep_merge([], override={"n": n}) == {"n": n}


def test_toml_datetime_is_its_toml_spelling(write):
    path = write("a.toml", "at = 1979-05-27T07:32:00Z\nday = 1979-05-27\n")
    assert deep_merge([path]) == {"at": "1979-05-27T07:32:00Z", "day": "1979-05-27"}


def test_non_finite_floats_pass_through(write):
    path = write("a.toml", "x = inf\n")
    assert deep_merge([path]) == {"x": float("inf")}


@pytest.mark.parametrize(
    ("name", "text", "message"),
    [
        ("a.json", "{", "a.json: invalid JSON"),
        ("a.toml", "x = ", "a.toml: invalid TOML"),
        ("a.json", "[1, 2]", "a.json: expected an object"),
    ],
)
def test_invalid_documents_raise_parse_error(write, name, text, message):
    with pytest.raises(ParseError, match=message) as info:
        deep_merge([write(name, text)])
    assert isinstance(info.value, ValueError)


def test_non_utf8_text_raises_parse_error(tmp_path):
    path = tmp_path / "a.json"
    path.write_bytes(b'{"a": "\xff"}')
    with pytest.raises(ParseError, match="a.json"):
        deep_merge([path])


def test_unknown_extension_is_a_value_error_not_a_parse_error(write):
    with pytest.raises(ValueError, match="cannot infer a format") as info:
        deep_merge([write("a.yaml", "x: 1")])
    assert not isinstance(info.value, ParseError)


def test_missing_file_raises_like_open(write, tmp_path):
    missing = tmp_path / "nope.json"
    with pytest.raises(FileNotFoundError) as info:
        deep_merge([write("a.json", "{}"), missing])
    assert info.value.errno == errno.ENOENT
    assert info.value.filename == str(missing)
    assert str(info.value) == f"[Errno {errno.ENOENT}] {os.strerror(errno.ENOENT)}: {str(missing)!r}"


def test_directory_raises_is_a_directory_error(tmp_path):
    with pytest.raises(IsADirectoryError) as info:
        deep_merge([tmp_path])
    assert info.value.errno == errno.EISDIR
    assert info.value.filename == str(tmp_path)


@pytest.mark.skipif(
    not hasattr(os, "geteuid") or os.geteuid() == 0,
    reason="needs POSIX permissions, which root ignores",
)
def test_unreadable_file_raises_permission_error(write):
    path = write("a.json", "{}")
    path.chmod(0)
    try:
        with pytest.raises(PermissionError) as info:
            deep_merge([path])
    finally:
        path.chmod(0o600)
    assert info.value.filename == str(path)


def test_bad_override_is_reported_before_files_are_read(tmp_path):
    with pytest.raises(TypeError, match="`a.b`"):
        deep_merge([tmp_path / "nope.json"], override={"a": {"b": object()}})


@pytest.mark.parametrize(
    ("override", "error", "message"),
    [
        ([("a", 1)], TypeError, "dict"),
        (UserDict({"a": 1}), TypeError, "dict"),
        ({1: "a"}, TypeError, "keys must be str"),
        ({"a": {2: "b"}}, TypeError, "under `a`"),
        ({"xs": [1, 2**64]}, ValueError, r"`xs\[1\]`"),
        ({"n": -(2**63) - 1}, ValueError, "64 bits"),
        ({"at": datetime.datetime(2020, 1, 1)}, TypeError, "datetime"),
        ({"s": {1, 2}}, TypeError, "set"),
    ],
)
def test_override_rejects_what_is_not_json_like(override, error, message):
    with pytest.raises(error, match=message):
        deep_merge([], override=override)


def self_containing_dict():
    d = {}
    d["self"] = d
    return d


def self_containing_list():
    xs = []
    xs.append(xs)
    return {"xs": xs}


def indirect_cycle():
    a = {}
    a["b"] = {"a": a}
    return a


def nested(depth):
    doc = 0
    for _ in range(depth):
        doc = {"k": doc}
    return doc


@pytest.mark.parametrize(
    ("override", "message"),
    [
        (self_containing_dict(), r"`self` refers back .* \(a cycle\)"),
        (self_containing_list(), r"`xs\[0\]` refers back"),
        (indirect_cycle(), r"`b\.a` refers back"),
        (nested(129), "deeper than 128 levels"),
    ],
)
def test_override_cycles_and_runaway_nesting_raise_instead_of_crashing(override, message):
    with pytest.raises(ValueError, match=message):
        deep_merge([], override=override)


def test_override_nesting_up_to_the_limit_converts():
    assert deep_merge([], override=nested(128)) == nested(128)


def test_a_value_shared_by_two_keys_is_not_a_cycle():
    shared = [1, {"x": 2}]
    assert deep_merge([], override={"a": shared, "b": shared}) == {
        "a": [1, {"x": 2}],
        "b": [1, {"x": 2}],
    }


def test_the_knf_executable_comes_with_the_wheel(write):
    """The pyknf wheel installs the `knf` binary; it does not depend on another package."""
    knf = shutil.which("knf")
    assert knf is not None
    path = write("a.json", '{"a": 1}')
    out = subprocess.run(
        [knf, str(path), "--set", "b=2", "--compact"],
        capture_output=True,
        text=True,
        check=True,
    )
    assert out.stdout.strip() == '{"a":1,"b":2}'


@pytest.mark.skipif(sys.platform != "linux", reason="Linux accepts raw bytes in filenames")
def test_the_knf_executable_accepts_non_utf8_filename(tmp_path):
    knf = shutil.which("knf")
    assert knf is not None
    path = os.fsencode(tmp_path) + b"/name-\xff.json"
    with open(path, "wb") as file:
        file.write(b'{"a":1}')

    out = subprocess.run(
        [os.fsencode(knf), path, b"--compact"],
        capture_output=True,
        check=True,
    )
    assert out.stdout.strip() == b'{"a":1}'


def test_a_bare_str_is_not_a_list_of_files(write):
    path = write("a.json", "{}")
    with pytest.raises(TypeError):
        deep_merge(str(path))
