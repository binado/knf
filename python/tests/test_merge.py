"""In-memory merge matches file load for the same layers."""

import datetime
import json
import math

import pytest

from knf import InterpolationError, deep_merge, load, merge


def write(tmp_path, name, text):
    path = tmp_path / name
    path.write_text(text)
    return path


def json_files(tmp_path, layers):
    return [write(tmp_path, f"{index}.json", json.dumps(layer)) for index, layer in enumerate(layers)]


def test_nested_objects_arrays_and_nulls_match_json_files(tmp_path):
    layers = [
        {
            "name": "app",
            "server": {"host": "local", "port": 80, "tags": [1, {"k": 1}], "extra": {"old": True}},
            "gone": {"a": 1},
            "nullable": "x",
            "items": [1, 2, 3],
            "id": 2**64 - 1,
        },
        {
            "server": {"port": 443, "tags": [9], "extra": None},
            "gone": None,
            "items": (3,),
            "nullable": None,
            "added": {"n": None, "ok": False},
        },
    ]
    expected = {
        "name": "app",
        "server": {"host": "local", "port": 443, "tags": [9], "extra": None},
        "gone": None,
        "nullable": None,
        "items": [3],
        "id": 2**64 - 1,
        "added": {"n": None, "ok": False},
    }
    assert merge(layers) == expected
    assert merge(layers) == load(json_files(tmp_path, layers))


def test_left_fold_is_not_associative(tmp_path):
    layers = [{"a": {"b": 1}}, {"a": 5}, {"a": {"c": 2}}]
    assert merge(layers) == {"a": {"c": 2}}
    assert merge(layers) == load(json_files(tmp_path, layers))


def test_merge_keeps_key_order_of_file_load(tmp_path):
    layers = [{"z": 0, "a": {"c": 1, "d": 2}}, {"a": {"b": 3}, "m": 1}]
    merged = merge(layers)
    loaded = load(json_files(tmp_path, layers))
    assert list(merged) == list(loaded) == ["z", "a", "m"]
    assert list(merged["a"]) == list(loaded["a"]) == ["c", "d", "b"]


@pytest.mark.parametrize("pattern", [None, "*", "db", "db.*", "**.pool", "{db,cache}.*", "missing.*"])
def test_shallow_paths_match_json_and_toml_files(tmp_path, pattern):
    base = {"db": {"pool": {"min": 1, "max": 5}, "host": "local"}, "app": {"x": 1}}
    over = {"db": {"pool": {"max": 9}}, "app": {"y": 2}}
    json_paths = json_files(tmp_path, [base, over])
    toml_paths = [
        write(tmp_path, "base.toml", '[db]\nhost = "local"\n[db.pool]\nmin = 1\nmax = 5\n[app]\nx = 1\n'),
        write(tmp_path, "over.toml", "[db.pool]\nmax = 9\n[app]\ny = 2\n"),
    ]
    merged = merge([base, over], shallow=pattern)
    assert merged == load(json_paths, shallow=pattern)
    assert merged == load(toml_paths, shallow=pattern)


def test_quoted_shallow_path_matches_json_files(tmp_path):
    base = {"foo.bar": {"pool": {"min": 1, "max": 5}}, "foo": {"bar": {"pool": {"min": 2}}}}
    over = {"foo.bar": {"pool": {"max": 9}}, "foo": {"bar": {"pool": {"max": 8}}}}
    merged = merge([base, over], shallow="'foo.bar'.*")
    assert merged == load(json_files(tmp_path, [base, over]), shallow="'foo.bar'.*")
    assert merged == {
        "foo.bar": {"pool": {"max": 9}},
        "foo": {"bar": {"pool": {"min": 2, "max": 8}}},
    }


def test_interpolation_and_env_match_json_files(tmp_path, monkeypatch):
    monkeypatch.setenv("KNF_PY_TEST_PORT", "8080")
    monkeypatch.setenv("KNF_PY_NULL", "null")
    layers = [
        {
            "server": {"host": "local", "port": 80},
            "servers": [{"host": "first"}],
            "url": "http://${server.host}:${server.port}/",
            "copy": "${server}",
            "first": "${servers[0].host}",
            "port": "${env:KNF_PY_TEST_PORT}",
            "literal": "$${env:KNF_PY_TEST_PORT}",
            "empty": "${env:KNF_PY_NULL}",
        },
        {"server": {"host": "prod", "port": 443}},
    ]
    files = json_files(tmp_path, layers)
    assert merge(layers) == load(files)
    merged = merge(layers, interpolate=True, format="json")
    assert merged == load(files, interpolate=True)
    assert merged["url"] == "http://prod:443/"
    assert merged["copy"] == {"host": "prod", "port": 443}
    assert merged["first"] == "first"
    assert merged["port"] == 8080
    assert merged["empty"] is None
    assert merged["literal"] == "${env:KNF_PY_TEST_PORT}"
    assert type(merged["server"]["port"]) is int


def test_toml_environment_values_match_load(tmp_path, monkeypatch):
    monkeypatch.setenv("KNF_PY_DAY", "1979-05-27")
    monkeypatch.setenv("KNF_PY_NULL", "null")
    monkeypatch.setenv("KNF_PY_TABLE", "{host='local'}")
    monkeypatch.setenv("KNF_PY_TERMINAL", "${missing}")
    layer = {
        "day": "${env:KNF_PY_DAY}",
        "null": "${env:KNF_PY_NULL}",
        "raw": "day:${env:KNF_PY_DAY}",
        "table": "${env:KNF_PY_TABLE}",
        "terminal": "${env:KNF_PY_TERMINAL}",
    }
    path = write(
        tmp_path,
        "env.toml",
        "day = '${env:KNF_PY_DAY}'\n"
        "null = '${env:KNF_PY_NULL}'\n"
        "raw = 'day:${env:KNF_PY_DAY}'\n"
        "table = '${env:KNF_PY_TABLE}'\n"
        "terminal = '${env:KNF_PY_TERMINAL}'\n",
    )
    merged = merge([layer], interpolate=True, format="toml")
    assert merged == load([path], interpolate=True)
    assert merged == {
        "day": datetime.date(1979, 5, 27),
        "null": "null",
        "raw": "day:1979-05-27",
        "table": {"host": "local"},
        "terminal": "${missing}",
    }
    assert merge([layer], interpolate=True)["day"] == "1979-05-27"


def test_toml_datetimes_and_nonfinite_floats_match_load(tmp_path):
    at = datetime.datetime(1979, 5, 27, 7, 32, tzinfo=datetime.timezone.utc)
    day = datetime.date(1979, 5, 27)
    clock = datetime.time(7, 32)
    layer = {"at": at, "day": day, "clock": clock, "positive": math.inf, "copy": "${day}"}
    path = write(
        tmp_path,
        "values.toml",
        "at = 1979-05-27T07:32:00Z\n"
        "day = 1979-05-27\n"
        "clock = 07:32:00\n"
        "positive = inf\n"
        "copy = '${day}'\n",
    )
    merged = merge([layer], interpolate=True, format="toml")
    loaded = load([path], interpolate=True)
    assert merged == loaded
    assert type(merged["at"]) is datetime.datetime
    assert type(merged["day"]) is datetime.date
    assert type(merged["clock"]) is datetime.time
    assert merged["positive"] == math.inf


def test_context_matches_file_context(tmp_path):
    layers = [
        {"host": "base", "url": "${service.url}"},
        {"host": "prod"},
    ]
    context = {"host": "shared", "unused": "${missing}", "service": {"url": "https://${host}/api"}}
    files = json_files(tmp_path, layers)
    ctx = write(tmp_path, "context.json", json.dumps(context))
    assert merge(layers, interpolate=True, context=context) == load(files, interpolate=True, context=ctx)
    assert merge([], interpolate=True, context=context) == {}


def test_context_shallow_and_references_match_load(tmp_path, monkeypatch):
    monkeypatch.setenv("KNF_CONTEXT", "${missing}")
    base = {"db": {"host": "prod", "port": 1}}
    over = {"db": {"host": "prod"}, "port": "${db.port}", "copy": "${db}"}
    context = {"db": {"host": False, "port": 5432}}
    files = json_files(tmp_path, [base, over])
    ctx = write(tmp_path, "context.json", json.dumps(context))
    assert merge([base, over], interpolate=True, shallow="db", context=context) == load(
        files, interpolate=True, shallow="db", context=ctx
    )

    doc = {"value": None, "copy": "${value}", "settings": "${shared}"}
    context = {
        "value": 1,
        "shared": {"big": 2**64 - 1, "null": None, "env": "${env:KNF_CONTEXT}", "literal": "$${missing}"},
    }
    path = write(tmp_path, "doc.json", json.dumps(doc))
    ctx = write(tmp_path, "shared.json", json.dumps(context))
    assert merge([doc], interpolate=True, context=context) == load([path], interpolate=True, context=ctx)


def test_whole_string_reference_merges_like_load(tmp_path):
    layers = [
        {"bar": {"a": 1, "d": 1}, "foo": {"b": "${bar}"}},
        {"foo": {"b": {"a": 4}}},
        {"bar": {"d": 9}},
    ]
    files = json_files(tmp_path, layers)
    toml = [
        write(tmp_path, "base.toml", "[bar]\na = 1\nd = 1\n[foo]\nb = '${bar}'\n"),
        write(tmp_path, "over.toml", "[foo.b]\na = 4\n"),
        write(tmp_path, "late.toml", "[bar]\nd = 9\n"),
    ]
    assert merge(layers[:2], interpolate=True) == load(files[:2], interpolate=True)
    assert merge(layers, interpolate=True)["foo"] == {"b": {"a": 4, "d": 9}}
    assert merge(layers, interpolate=True, format="toml") == load(toml, interpolate=True)
    assert merge(layers[:2], interpolate=True, shallow="foo.b") == load(
        files[:2], interpolate=True, shallow="foo.b"
    )


def test_interpolation_errors_match_load(tmp_path):
    doc = {"a": "${b}", "b": "${a}"}
    path = write(tmp_path, "cycle.json", json.dumps(doc))
    with pytest.raises(InterpolationError) as mem:
        merge([doc], interpolate=True)
    with pytest.raises(InterpolationError) as files:
        load([path], interpolate=True)
    assert str(mem.value) == str(files.value)

    doc = {"copy": "${a}"}
    context = {"a": "${missing}"}
    path = write(tmp_path, "doc.json", json.dumps(doc))
    ctx = write(tmp_path, "context.json", json.dumps(context))
    with pytest.raises(InterpolationError) as mem:
        merge([doc], interpolate=True, context=context)
    with pytest.raises(InterpolationError) as files:
        load([path], interpolate=True, context=ctx)
    assert str(mem.value) == str(files.value)


def test_deep_merge_matches_merge_and_load(tmp_path):
    cfg = {"solver": {"steps": 100, "name": "pilot"}, "items": [1, 2], "nullable": "x"}
    overrides = {"solver": {"steps": 10}, "items": [3], "nullable": None}
    assert deep_merge(cfg, overrides) == merge([cfg, overrides])
    assert deep_merge(cfg, overrides) == load(json_files(tmp_path, [cfg, overrides]))


def test_merge_returns_an_independent_tree():
    cfg = {"server": {"port": 80, "tags": [1]}, "table": {"a": 1}}
    over = {"server": {"port": 443}}
    snapshot = (json.dumps(cfg), json.dumps(over))
    merged = merge([cfg, over], interpolate=True)
    merged["server"]["tags"].append(2)
    merged["table"]["a"] = 9
    assert (json.dumps(cfg), json.dumps(over)) == snapshot
    assert merged is not cfg
    assert merged["server"] is not cfg["server"]
    assert merged["server"]["tags"] is not cfg["server"]["tags"]


def test_empty_and_single_layers_copy():
    assert merge([]) == {}
    cfg = {"a": {"b": [1]}}
    merged = merge([cfg])
    assert merged == cfg
    assert merged is not cfg
    assert merged["a"] is not cfg["a"]
    assert merged["a"]["b"] is not cfg["a"]["b"]


def test_argument_errors():
    with pytest.raises(ValueError, match="context requires interpolate=True"):
        merge([{"a": 1}], context={"a": 2})
    with pytest.raises(ValueError, match="format must be 'json' or 'toml'"):
        merge([{"a": 1}], format="yaml")
    with pytest.raises(ValueError, match="empty key-path glob"):
        merge([], shallow="")
    with pytest.raises(TypeError):
        merge({"a": 1})
    with pytest.raises(TypeError):
        merge([{"a": 1}, 2])
    with pytest.raises(TypeError):
        merge([], interpolate=True, context="context.json")
    with pytest.raises(TypeError):
        merge([], shallow=True)
    with pytest.raises(TypeError):
        merge([{"a": 1}], True)
