"""File-backed interpolation contexts across both native formats."""

import datetime
import json
import math

import pytest

from knf import InterpolationError, ParseError, load


def write(tmp_path, name, text):
    path = tmp_path / name
    path.write_text(text)
    return path


@pytest.mark.parametrize("as_string", [False, True])
def test_context_is_not_merged_and_dependencies_prefer_output(tmp_path, as_string):
    base = write(tmp_path, "base.toml", "host = 'base'\nurl = '${service.url}'\n")
    prod = write(tmp_path, "prod.toml", "host = 'prod'\n")
    context = write(tmp_path, "context.toml", "host = 'shared'\nunused = '${missing}'\n[service]\nurl = 'https://${host}/api'\n")
    assert load([base, prod], interpolate=True, context=str(context) if as_string else context) == {
        "host": "prod", "url": "https://prod/api",
    }


def test_context_composes_with_shallow_and_complete_path_fallback(tmp_path):
    base = write(tmp_path, "base.json", '{"db":{"host":"prod","port":1}}')
    over = write(tmp_path, "over.json", '{"db":{"host":"prod"},"port":"${db.port}","copy":"${db}"}')
    context = write(tmp_path, "context.json", '{"db":{"host":false,"port":5432}}')
    assert load([base, over], interpolate=True, shallow="db", context=context) == {
        "db": {"host": "prod"}, "port": 5432, "copy": {"host": "prod"},
    }


def test_context_container_values_environment_and_native_json(tmp_path, monkeypatch):
    doc = write(tmp_path, "doc.json", '{"value":null,"copy":"${value}","settings":"${shared}"}')
    context = write(tmp_path, "context.json", json.dumps({
        "value": 1, "shared": {"big": 2**64 - 1, "null": None,
        "env": "${env:KNF_CONTEXT}", "literal": "$${missing}"},
    }))
    monkeypatch.setenv("KNF_CONTEXT", "${missing}")
    assert load([doc], interpolate=True, context=context) == {
        "value": None, "copy": None, "settings": {
            "big": 2**64 - 1, "null": None, "env": "${missing}", "literal": "${missing}",
        },
    }


def test_context_preserves_toml_datetime_and_nonfinite_values(tmp_path):
    doc = write(tmp_path, "doc.toml", "settings = '${shared}'\n")
    context = write(tmp_path, "context.toml", "[shared]\nat = 1979-05-27T07:32:00.123456789+05:45\nday = 1979-05-27\ntime = 07:32:00\ninfinite = inf\nnan = nan\n")
    values = load([doc], interpolate=True, context=context)["settings"]
    assert values["at"] == datetime.datetime(1979, 5, 27, 7, 32, 0, 123456,
        tzinfo=datetime.timezone(datetime.timedelta(hours=5, minutes=45)))
    assert values["day"] == datetime.date(1979, 5, 27)
    assert values["time"] == datetime.time(7, 32)
    assert values["infinite"] == math.inf
    assert math.isnan(values["nan"])


def test_context_alone_returns_empty_output_and_leaves_unused_references_unresolved(tmp_path):
    context = write(tmp_path, "context.toml", "unused = '${missing}'\ncycle = '${cycle}'\n")
    assert load([], interpolate=True, context=context) == {}


@pytest.mark.parametrize("context", ["missing.json", "-"])
def test_context_requires_interpolation_before_reads(tmp_path, context):
    with pytest.raises(ValueError, match="context requires interpolate=True"):
        load([tmp_path / "missing.json"], context=context)


def test_context_rejects_stdin_before_reads(tmp_path):
    with pytest.raises(ValueError, match="context does not accept stdin"):
        load([tmp_path / "missing.json"], interpolate=True, context="-")


@pytest.mark.parametrize("context", [{"port": 8080}, ["context.json"], 42])
def test_context_rejects_non_filepath_arguments(context):
    with pytest.raises(TypeError):
        load([], interpolate=True, context=context)


def test_context_mixed_formats_fail_before_reads(tmp_path):
    with pytest.raises(ValueError, match="inputs mix JSON and TOML"):
        load([tmp_path / "missing.json"], interpolate=True, context=tmp_path / "missing.toml")


def test_context_file_errors_keep_exception_type_and_filename(tmp_path):
    missing = tmp_path / "missing.json"
    with pytest.raises(FileNotFoundError) as info:
        load([], interpolate=True, context=missing)
    assert info.value.filename == str(missing)
    directory = tmp_path / "directory.json"
    directory.mkdir()
    with pytest.raises(IsADirectoryError) as info:
        load([], interpolate=True, context=directory)
    assert info.value.filename == str(directory)


@pytest.mark.parametrize("text, message", [("{", "invalid JSON"), ("[]", "expected an object")])
def test_context_is_parsed_even_when_unused(tmp_path, text, message):
    context = write(tmp_path, "context.json", text)
    with pytest.raises(ParseError, match=message):
        load([], interpolate=True, context=context)


@pytest.mark.parametrize("context_text, message", [
    ('{"a":"${missing}"}', "a: `missing`"),
    ('{"a":"${b}","b":"${a}"}', "reference cycle: `a` -> `b` -> `a`"),
    ('{"a":"${copy}"}', "reference cycle: `copy` -> `a` -> `copy`"),
])
def test_context_interpolation_errors_report_source_paths(tmp_path, context_text, message):
    doc = write(tmp_path, "doc.json", '{"copy":"${a}"}')
    context = write(tmp_path, "context.json", context_text)
    with pytest.raises(InterpolationError, match=message):
        load([doc], interpolate=True, context=context)


def test_output_interpolation_failure_does_not_fall_back(tmp_path):
    doc = write(tmp_path, "doc.json", '{"a":"${missing}","copy":"${a}"}')
    context = write(tmp_path, "context.json", '{"a":1}')
    with pytest.raises(InterpolationError, match="a: `missing`"):
        load([doc], interpolate=True, context=context)
