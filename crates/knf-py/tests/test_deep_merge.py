"""`knf.load` and the `knf` executable, as an installed wheel exposes them."""

import errno
import json
import os
import shutil
import subprocess
import sys

import pytest

from knf import InterpolationError, ParseError, load


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
    merged = load([path])
    assert merged == doc
    assert list(merged) == ["b", "a", "c"]


def test_files_fold_left_to_right_across_formats(write):
    base = write("base.toml", 'name = "app"\n[server]\nhost = "localhost"\nport = 80\n')
    prod = write("prod.json", '{"server": {"port": 443}}')
    assert load([base, prod]) == {
        "name": "app",
        "server": {"host": "localhost", "port": 443},
    }
    assert load([prod, base])["server"]["port"] == 80


def test_paths_may_be_str(write):
    path = write("a.json", '{"a": 1}')
    assert load([str(path)]) == {"a": 1}


def test_empty_file_list_returns_empty_dict():
    assert load([]) == {}


def test_arrays_and_none_load_from_document(write):
    path = write("a.json", '{"xs": [1, 2, 3], "k": {"v": 1}}')
    assert load([path]) == {"xs": [1, 2, 3], "k": {"v": 1}}


def test_toml_datetime_is_its_toml_spelling(write):
    path = write("a.toml", "at = 1979-05-27T07:32:00Z\nday = 1979-05-27\n")
    assert load([path]) == {"at": "1979-05-27T07:32:00Z", "day": "1979-05-27"}


def test_non_finite_floats_pass_through(write):
    path = write("a.toml", "x = inf\n")
    assert load([path]) == {"x": float("inf")}


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
        load([write(name, text)])
    assert isinstance(info.value, ValueError)


def test_non_utf8_text_raises_parse_error(tmp_path):
    path = tmp_path / "a.json"
    path.write_bytes(b'{"a": "\xff"}')
    with pytest.raises(ParseError, match="a.json"):
        load([path])


def test_unknown_extension_is_a_value_error_not_a_parse_error(write):
    with pytest.raises(ValueError, match="cannot infer a format") as info:
        load([write("a.yaml", "x: 1")])
    assert not isinstance(info.value, ParseError)


def test_missing_file_raises_like_open(write, tmp_path):
    missing = tmp_path / "nope.json"
    with pytest.raises(FileNotFoundError) as info:
        load([write("a.json", "{}"), missing])
    assert info.value.errno == errno.ENOENT
    assert info.value.filename == str(missing)
    assert str(info.value) == f"[Errno {errno.ENOENT}] {os.strerror(errno.ENOENT)}: {str(missing)!r}"


def test_directory_raises_is_a_directory_error(tmp_path):
    with pytest.raises(IsADirectoryError) as info:
        load([tmp_path])
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
            load([path])
    finally:
        path.chmod(0o600)
    assert info.value.filename == str(path)


def test_interpolation_is_opt_in(write):
    path = write("refs.json", '{"port": 8080, "copy": "${port}"}')
    assert load([path])["copy"] == "${port}"
    assert load([path], interpolate=False)["copy"] == "${port}"
    assert load([path], interpolate=True)["copy"] == 8080


def test_interpolation_sees_final_layers_and_nested_values(write):
    base = write(
        "base.json",
        json.dumps({
            "server": {"host": "local", "port": 80},
            "servers": [{"host": "first"}],
            "url": "http://${server.host}:${server.port}/",
            "copy": "${server}",
            "first": "${servers[0].host}",
            "next": "${url}",
            "final": "${next}",
        }),
    )
    prod = write("prod.toml", '[server]\nhost = "prod"\nport = 443\n')
    merged = load([base, prod], interpolate=True)
    assert merged["url"] == "http://prod:443/"
    assert merged["copy"] == {"host": "prod", "port": 443}
    assert merged["first"] == "first"
    assert merged["final"] == "http://prod:443/"
    assert type(merged["server"]["port"]) is int


def test_interpolation_reads_process_environment(monkeypatch, write):
    monkeypatch.setenv("KNF_PY_TEST_PORT", "8080")
    path = write(
        "env.json",
        json.dumps({
            "port": "${env:KNF_PY_TEST_PORT}",
            "url": "http://localhost:${env:KNF_PY_TEST_PORT}/",
            "literal": "$${env:KNF_PY_TEST_PORT}",
        }),
    )
    merged = load([path], interpolate=True)
    assert merged == {
        "port": 8080,
        "url": "http://localhost:8080/",
        "literal": "${env:KNF_PY_TEST_PORT}",
    }


@pytest.mark.parametrize(
    ("doc", "message"),
    [
        ({"a": "${missing}"}, "unresolved reference\n  --> a: `missing`"),
        ({"a": "${}"}, "invalid reference\n  --> a: empty reference"),
        ({"a": "${b}", "b": "${a}"}, "reference cycle: `a` -> `b` -> `a`"),
        ({"box": {"x": 1}, "text": "box=${box}"}, "text: `box` is an object"),
    ],
)
def test_interpolation_errors_are_value_errors_with_paths(write, doc, message):
    path = write("errors.json", json.dumps(doc))
    with pytest.raises(InterpolationError) as info:
        load([path], interpolate=True)
    assert isinstance(info.value, ValueError)
    assert message in str(info.value)


def test_interpolation_reports_file_errors(tmp_path):
    missing = tmp_path / "missing.json"
    with pytest.raises(FileNotFoundError) as info:
        load([missing], interpolate=True)
    assert info.value.filename == str(missing)


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


def test_the_knf_executable_accumulates_and_lists_files(tmp_path):
    knf = shutil.which("knf")
    assert knf is not None
    (tmp_path / "foo" / "bar").mkdir(parents=True)
    (tmp_path / "foo" / "base.toml").write_text("base = 1\nvalue = 1\n")
    (tmp_path / "foo" / "bar" / "target.toml").write_text("value = 2\n")
    (tmp_path / "ignored.toml").write_text("invalid ignored root")
    out = subprocess.run(
        [knf, "-a", "foo/bar/target.toml", "-f", "json", "--compact"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=True,
    )
    assert json.loads(out.stdout) == {"base": 1, "value": 2}
    out = subprocess.run(
        [knf, "-a", "foo/bar/target.toml", "--list-files"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=True,
    )
    assert out.stdout.splitlines() == [
        os.path.join("foo", "base.toml"),
        os.path.join("foo", "bar", "target.toml"),
    ]


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
        load(str(path))
