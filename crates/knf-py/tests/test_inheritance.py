"""Object inheritance through the shared native resolver."""

import datetime
import json
import math
import subprocess

import pytest

from knf import InterpolationError, load


def write(tmp_path, name, text):
    path = tmp_path / name
    path.write_text(text)
    return path


@pytest.mark.parametrize("format", ["json", "toml"])
def test_inheritance_sees_final_layers_and_destination_paths(tmp_path, format):
    if format == "json":
        base_text = '{"foo":{"a":1,"b":2,"c":3},"bar":{"extends":"${foo}","c":4,"read":"${bar.a}"}}'
        over_text = '{"foo":{"a":9}}'
    else:
        base_text = "[foo]\na=1\nb=2\nc=3\n[bar]\nextends='${foo}'\nc=4\nread='${bar.a}'\n"
        over_text = "[foo]\na=9\n"
    base = write(tmp_path, f"base.{format}", base_text)
    over = write(tmp_path, f"over.{format}", over_text)
    assert load([base, over], interpolate=True, merge_key="extends") == {
        "foo": {"a": 9, "b": 2, "c": 3},
        "bar": {"a": 9, "b": 2, "c": 4, "read": 9},
    }


def test_inheritance_is_opt_in(tmp_path):
    doc = write(tmp_path, "doc.json", '{"foo":{"a":1},"bar":{"extends":"${foo}","c":4}}')
    assert load([doc])["bar"] == {"extends": "${foo}", "c": 4}
    assert load([doc], interpolate=True)["bar"] == {"extends": {"a": 1}, "c": 4}


def test_inheritance_context_is_lazy_and_honors_shallow(tmp_path):
    doc = write(tmp_path, "doc.json", '{"bar":{"extends":"${base}","bad":false,"db":{"port":90}}}')
    context = write(tmp_path, "context.json", '{"base":{"a":1,"bad":"${missing}","db":{"host":"shared","port":80}},"unused":{"extends":42}}')
    assert load([doc], interpolate=True, merge_key="extends", context=context, shallow="bar.db") == {
        "bar": {"a": 1, "bad": False, "db": {"port": 90}},
    }


def test_inherited_native_values_and_terminal_environment(tmp_path, monkeypatch):
    doc = write(tmp_path, "doc.json", '{"bar":{"extends":"${env:KNF_BASE}","a":2}}')
    monkeypatch.setenv("KNF_BASE", json.dumps({"a": 1, "big": 2**64 - 1, "null": None, "raw": "${missing}"}))
    assert load([doc], interpolate=True, merge_key="extends") == {
        "bar": {"a": 2, "big": 2**64 - 1, "null": None, "raw": "${missing}"},
    }
    doc = write(tmp_path, "doc.toml", "[bar]\nextends='${base}'\n")
    context = write(tmp_path, "context.toml", "[base]\nat=1979-05-27T07:32:00.123456789+05:45\nday=1979-05-27\ntime=07:32:00\ninfinite=inf\nnan=nan\n")
    values = load([doc], interpolate=True, merge_key="extends", context=context)["bar"]
    assert values["at"] == datetime.datetime(1979, 5, 27, 7, 32, 0, 123456, datetime.timezone(datetime.timedelta(hours=5, minutes=45)))
    assert values["day"] == datetime.date(1979, 5, 27)
    assert values["time"] == datetime.time(7, 32)
    assert math.isinf(values["infinite"])
    assert math.isnan(values["nan"])


@pytest.mark.parametrize("marker", ["extends", "foo.bar", "*", ""])
def test_merge_key_is_literal(tmp_path, marker):
    doc = write(tmp_path, "doc.json", json.dumps({"foo": {"a": 1}, "bar": {marker: "${foo}", "c": 4}}))
    assert load([doc], interpolate=True, merge_key=marker)["bar"] == {"a": 1, "c": 4}


def test_merge_key_requires_interpolation_before_reads(tmp_path):
    with pytest.raises(ValueError, match="merge_key requires interpolate=True"):
        load([tmp_path / "missing.json"], merge_key="extends")
    with pytest.raises(TypeError):
        load([], interpolate=True, merge_key=42)


@pytest.mark.parametrize("text, message", [
    ('{"bar":{"extends":42}}', "bar.extends: expected one whole-string reference"),
    ('{"foo":42,"bar":{"extends":"${foo}"}}', "bar.extends: expected an object, found number"),
    ('{"bar":{"extends":"${missing}"}}', "bar.extends: `missing`"),
    ('{"bar":{"extends":"${bar}"}}', "reference cycle"),
])
def test_inheritance_errors_keep_exception_type_and_paths(tmp_path, text, message):
    doc = write(tmp_path, "doc.json", text)
    with pytest.raises(InterpolationError, match=message):
        load([doc], interpolate=True, merge_key="extends")


def test_wheel_cli_accepts_merge_key(tmp_path):
    doc = write(tmp_path, "doc.json", '{"foo":{"a":1},"bar":{"extends":"${foo}","c":4}}')
    result = subprocess.run(["knf", str(doc), "-i", "-m", "extends"], capture_output=True, text=True, check=True)
    assert json.loads(result.stdout)["bar"] == {"a": 1, "c": 4}
