"""`knf.load` and the `knf` executable, as an installed wheel exposes them."""

import datetime
import math

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


def test_toml_files_fold_left_to_right(write):
    base = write("base.toml", 'name = "app"\n[server]\nhost = "localhost"\nport = 80\n')
    prod = write("prod.toml", "[server]\nport = 443\n")
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


@pytest.mark.parametrize("format", ["json", "toml"])
@pytest.mark.parametrize(
    "pattern, expected_db",
    [
        (None, {"pool": {"min": 1, "max": 9}, "host": "local"}),
        ("*", {"pool": {"max": 9}}),
        ("db", {"pool": {"max": 9}}),
        ("db.*", {"pool": {"max": 9}, "host": "local"}),
        ("**.pool", {"pool": {"max": 9}, "host": "local"}),
        ("{db,cache}.*", {"pool": {"max": 9}, "host": "local"}),
        ("missing.*", {"pool": {"min": 1, "max": 9}, "host": "local"}),
    ],
)
def test_shallow_key_globs(write, format, pattern, expected_db):
    if format == "json":
        base = json.dumps({"db": {"pool": {"min": 1, "max": 5}, "host": "local"}, "app": {"x": 1}})
        over = json.dumps({"db": {"pool": {"max": 9}}, "app": {"y": 2}})
    else:
        base = '[db]\nhost = "local"\n[db.pool]\nmin = 1\nmax = 5\n[app]\nx = 1\n'
        over = "[db.pool]\nmax = 9\n[app]\ny = 2\n"
    files = [write(f"base.{format}", base), write(f"over.{format}", over)]
    assert load(files, shallow=pattern) == {
        "db": expected_db,
        "app": {"y": 2} if pattern == "*" else {"x": 1, "y": 2},
    }


def test_shallow_glob_matches_literal_keys(write):
    base = {"foo.bar": {"pool": {"min": 1, "max": 5}}, "foo": {"bar": {"pool": {"min": 2}}}}
    over = {"foo.bar": {"pool": {"max": 9}}, "foo": {"bar": {"pool": {"max": 8}}}}
    files = [write("base.json", json.dumps(base)), write("over.json", json.dumps(over))]
    assert load(files, shallow="'foo.bar'.*") == {
        "foo.bar": {"pool": {"max": 9}},
        "foo": {"bar": {"pool": {"min": 2, "max": 8}}},
    }


@pytest.mark.parametrize("pattern", ["", "[", "{db,", "'unclosed"])
@pytest.mark.parametrize("empty", [False, True])
def test_invalid_shallow_glob_fails_before_io(tmp_path, pattern, empty):
    with pytest.raises(ValueError) as info:
        load([] if empty else [tmp_path / "missing.json"], shallow=pattern)
    assert not isinstance(info.value, ParseError)


@pytest.mark.parametrize("shallow", [True, ["db"], 1])
def test_shallow_requires_a_string(shallow):
    with pytest.raises(TypeError):
        load([], shallow=shallow)


def test_shallow_interpolates_after_merging_and_keeps_native_toml(write):
    base = write("base.toml", "[db]\nold = '${missing}'\n[db.pool]\nmin = 1\n")
    over = write("over.toml", "copy = '${db}'\n[db]\nday = 1979-05-27\n[db.pool]\nmax = 9\n")
    db = {"day": datetime.date(1979, 5, 27), "pool": {"max": 9}}
    assert load([base, over], shallow="db", interpolate=True) == {"db": db, "copy": db}


def test_shallow_load_still_folds_left_to_right(write):
    files = [
        write("base.json", '{"a":{"old":1},"other":{"old":1}}'),
        write("middle.json", '{"a":5,"other":{"new":2}}'),
        write("last.json", '{"a":{"new":2}}'),
    ]
    assert load(files, shallow="other") == {"a": {"new": 2}, "other": {"new": 2}}


def test_arrays_and_none_load_from_document(write):
    path = write("a.json", '{"xs": [1, 2, 3], "k": {"v": 1}}')
    assert load([path]) == {"xs": [1, 2, 3], "k": {"v": 1}}


def test_toml_datetimes_are_native_python_objects(write):
    path = write("a.toml", "at = 1979-05-27T07:32:00Z\nday = 1979-05-27\n")
    assert load([path]) == {"at": datetime.datetime(1979, 5, 27, 7, 32, tzinfo=datetime.timezone.utc), "day": datetime.date(1979, 5, 27)}


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
    prod = write("prod.json", '{"server":{"host":"prod","port":443}}')
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
        [knf, str(path), "-c", "b=2", "--compact"],
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
        [knf, "-a", "foo/bar/target.toml", "--compact"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=True,
    )
    assert out.stdout == "base = 1\nvalue = 2\n"
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


@pytest.mark.parametrize(
    ("pattern", "expected"),
    [
        ("*", {"db": {"pool": {"max": 9}}, "foo.bar": {"new": 2}}),
        ("db.*", {"db": {"host": "local", "pool": {"max": 9}}, "foo.bar": {"old": 1, "new": 2}}),
        ("'foo.bar'", {"db": {"host": "local", "pool": {"min": 1, "max": 9}}, "foo.bar": {"new": 2}}),
        ("{db,'foo.bar'}", {"db": {"pool": {"max": 9}}, "foo.bar": {"new": 2}}),
    ],
)
def test_the_knf_executable_selects_shallow_globs(tmp_path, pattern, expected):
    executable = shutil.which("knf")
    assert executable is not None
    base = {"db": {"host": "local", "pool": {"min": 1}}, "foo.bar": {"old": 1}}
    over = {"db": {"pool": {"max": 9}}, "foo.bar": {"new": 2}}
    (tmp_path / "base.json").write_text(json.dumps(base))
    (tmp_path / "over.json").write_text(json.dumps(over))
    out = subprocess.run(
        [executable, "base.json", "over.json", "--shallow", pattern],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=True,
    )
    assert json.loads(out.stdout) == expected


@pytest.mark.parametrize("args", [["--shallow"], ["--shallow="], ["--shallow=a", "--shallow=b"], ["--shallow='unclosed"]])
def test_the_knf_executable_rejects_invalid_shallow_before_io(tmp_path, args):
    executable = shutil.which("knf")
    assert executable is not None
    out = subprocess.run(
        [executable, "missing.json", *args], cwd=tmp_path, capture_output=True, text=True
    )
    assert out.returncode == 2
    assert "No such file" not in out.stderr


@pytest.mark.parametrize(
    ("flag", "pattern"),
    [("--glob", "config/*.prod.toml"), ("--glob-filename", "*.prod.toml")],
)
def test_the_knf_executable_filters_inputs(tmp_path, flag, pattern):
    knf = shutil.which("knf")
    assert knf is not None
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "a.prod.toml").write_text("value = 1\n")
    (tmp_path / "config" / "b.prod.toml").write_text("value = 2\n")
    (tmp_path / "config" / "bad.json").write_text("invalid ignored JSON")
    args = [
        knf,
        "config/b.prod.toml",
        "config/bad.json",
        "config/missing.json",
        "config/a.prod.toml",
        flag,
        pattern,
    ]
    out = subprocess.run(
        args + ["--compact"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=True,
    )
    assert out.stdout == "value = 1\n"
    out = subprocess.run(
        args + ["--list-files"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=True,
    )
    assert out.stdout.splitlines() == ["config/b.prod.toml", "config/a.prod.toml"]


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


@pytest.mark.parametrize("interpolate", [False, True])
def test_mixed_formats_fail_before_reading_contents(tmp_path, interpolate):
    with pytest.raises(ValueError, match="inputs mix JSON and TOML") as info:
        load([tmp_path / "missing.json", tmp_path / "missing.toml"], interpolate=interpolate)
    assert not isinstance(info.value, ParseError)


@pytest.mark.parametrize("literal, expected", [
    ("1979-05-27T07:32:00.123456789Z", datetime.datetime(1979, 5, 27, 7, 32, 0, 123456, tzinfo=datetime.timezone.utc)),
    ("1979-05-27T07:32:00.123456789-07:30", datetime.datetime(1979, 5, 27, 7, 32, 0, 123456, tzinfo=datetime.timezone(datetime.timedelta(hours=-7, minutes=-30)))),
    ("1979-05-27T07:32:00+05:45", datetime.datetime(1979, 5, 27, 7, 32, tzinfo=datetime.timezone(datetime.timedelta(hours=5, minutes=45)))),
    ("1979-05-27T07:32:00.123456789", datetime.datetime(1979, 5, 27, 7, 32, 0, 123456)),
    ("1979-05-27", datetime.date(1979, 5, 27)),
    ("07:32:00.123456789", datetime.time(7, 32, 0, 123456)),
    ("07:32", datetime.time(7, 32)),
    ("1979-05-27T07:32", datetime.datetime(1979, 5, 27, 7, 32)),
])
def test_all_toml_datetime_forms_and_precision(write, literal, expected):
    text = f"value = {literal}\ncopy = '${{value}}'\n"
    path = write("dates.toml", text)
    result = load([path], interpolate=True)
    assert result["value"] == expected
    assert result["copy"] == expected
    assert type(result["value"]) is type(expected)
    if sys.version_info >= (3, 11) and literal not in ("07:32", "1979-05-27T07:32"):
        import tomllib
        assert load([path])["value"] == tomllib.loads(text)["value"]


def test_unrepresentable_toml_datetime_reports_key_and_array_path(write):
    path = write("dates.toml", "[nested]\nvalues = [0000-01-01]\n")
    with pytest.raises(ValueError, match=r"nested.values\[0\]") as info:
        load([path])
    assert not isinstance(info.value, ParseError)


def test_toml_environment_typing_is_native_and_terminal(write, monkeypatch):
    monkeypatch.setenv("KNF_PY_DAY", "1979-05-27")
    monkeypatch.setenv("KNF_PY_NULL", "null")
    monkeypatch.setenv("KNF_PY_TABLE", "{host='local'}")
    monkeypatch.setenv("KNF_PY_TERMINAL", "${missing}")
    path = write("env.toml", "day = '${env:KNF_PY_DAY}'\nnull = '${env:KNF_PY_NULL}'\nraw = 'day:${env:KNF_PY_DAY}'\ntable = '${env:KNF_PY_TABLE}'\nterminal = '${env:KNF_PY_TERMINAL}'\n")
    assert load([path], interpolate=True) == {
        "day": datetime.date(1979, 5, 27), "null": "null", "raw": "day:1979-05-27",
        "table": {"host": "local"}, "terminal": "${missing}",
    }


def test_toml_nonfinite_values_and_large_json_integers(write):
    path = write("limits.toml", "positive = inf\nnegative = -inf\nnan = nan\n")
    values = load([path])
    assert values["positive"] == math.inf
    assert values["negative"] == -math.inf
    assert math.isnan(values["nan"])
    path = write("large.json", '{"id":18446744073709551615,"null":null}')
    assert load([path]) == {"id": 18446744073709551615, "null": None}


def test_wheel_cli_uses_one_native_format_option(write):
    path = write("a.toml", "name = 'native'")
    assert subprocess.run(["knf", str(path), "-f", "json"], capture_output=True).returncode == 1
    out = subprocess.run(["knf", "-f", "toml", "-c", "day=1979-05-27"], capture_output=True, text=True)
    assert out.returncode == 0, out.stderr
    assert "day = 1979-05-27" in out.stdout
    for flag in ("--input-format", "--null-as"):
        assert subprocess.run(["knf", flag, "json"], capture_output=True).returncode == 2
