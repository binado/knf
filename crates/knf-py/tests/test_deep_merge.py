"""`knf.deep_merge` and the `knf` executable, as an installed wheel exposes them."""

import datetime
import json
import shutil
import subprocess

import pytest

from knf import KnfError, deep_merge


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
        ("a.json", "{", "a.json"),
        ("a.toml", "x = ", "a.toml"),
        ("a.yaml", "x: 1", "cannot infer a format"),
        ("a.json", "[1, 2]", "a.json"),
    ],
)
def test_bad_files_raise_knf_error(write, name, text, message):
    with pytest.raises(KnfError, match=message):
        deep_merge([write(name, text)])


def test_missing_file_raises_knf_error(tmp_path):
    with pytest.raises(KnfError, match="nope.json"):
        deep_merge([tmp_path / "nope.json"])


def test_bad_override_is_reported_before_files_are_read(tmp_path):
    with pytest.raises(TypeError, match="`a.b`"):
        deep_merge([tmp_path / "nope.json"], override={"a": {"b": object()}})


@pytest.mark.parametrize(
    ("override", "error", "message"),
    [
        ([("a", 1)], TypeError, "dict"),
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


def test_a_bare_str_is_not_a_list_of_files(write):
    path = write("a.json", "{}")
    with pytest.raises(TypeError):
        deep_merge(str(path))


def test_the_knf_executable_comes_with_it(write):
    """pyknf depends on knf-cli, so installing one installs the executable."""
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
