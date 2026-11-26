from datetime import date, datetime, time, timedelta, timezone, tzinfo
from io import StringIO
import json
import math
import tomllib

import pytest

from knf import dump, dumps, load


def parse(text: str) -> dict:
    return tomllib.loads(text)


class NoOffset(tzinfo):
    """A tzinfo whose offset is unknown, like a DST-aware zone on a bare time."""

    def utcoffset(self, dt):
        return None


@pytest.mark.parametrize("pretty", [True, False])
def test_roundtrip(pretty, tmp_path):
    data = {
        "unicode": "olá\nworld",
        "bool": True,
        "min": -(2**63),
        "max": 2**63 - 1,
        "float": 1.25,
        "day": date(2026, 11, 13),
        "time": time(1, 2, 3, 456789),
        "local": datetime(2026, 11, 13, 1, 2, 3, 456789),
        "offset": datetime(2026, 11, 13, tzinfo=timezone(timedelta(minutes=-195))),
        "empty": [],
        "nested": {"a.b": [1, "two", False]},
        "tables": [{"x": 1}, {"x": 2}],
    }
    text = dumps(data, format="toml", pretty=pretty)
    assert text.endswith("\n")
    assert parse(text) == data
    path = tmp_path / "out.toml"
    with path.open("w", encoding="utf-8") as fp:
        assert dump(data, fp, format="toml", pretty=pretty) is None
    assert load([path]) == data


def test_insertion_order_with_tables_after_scalars():
    data = {"z": 1, "t": {"q": 1, "b": {"y": 1}, "a": 2}, "a": 2}
    result = parse(dumps(data, format="toml"))
    assert list(result) == ["z", "a", "t"]
    assert list(result["t"]) == ["q", "a", "b"]


def test_special_values():
    result = parse(dumps({"tuple": (1, 2), "inf": math.inf, "nan": math.nan}, format="toml"))
    assert result["tuple"] == [1, 2]
    assert result["inf"] == math.inf
    assert math.isnan(result["nan"])
    assert parse(dumps({}, format="toml")) == {}


@pytest.mark.parametrize("value, error", [
    (None, TypeError), (object(), TypeError), (b"bytes", TypeError),
    (2**63, ValueError), (-(2**63) - 1, ValueError),
    (time(tzinfo=timezone.utc), ValueError),
    (time(tzinfo=NoOffset()), ValueError),
    (datetime(2026, 1, 1, tzinfo=NoOffset()), ValueError),
    (datetime(2026, 1, 1, tzinfo=timezone(timedelta(seconds=1))), ValueError),
])
def test_invalid_values_include_path(value, error):
    with pytest.raises(error, match=r"nested.*items.*\[0\]"):
        dumps({"nested": {"items": [value]}}, format="toml")


def test_invalid_keys_and_root():
    with pytest.raises(TypeError, match="keys must be strings"):
        dumps({"nested": {1: "x"}}, format="toml")
    with pytest.raises(TypeError):
        dumps([], format="toml")


def test_cycles_depth_and_shared_values():
    cycle = []
    cycle.append(cycle)
    with pytest.raises(ValueError, match="circular reference"):
        dumps({"cycle": cycle}, format="toml")
    deep = {}
    for _ in range(128):
        deep = {"child": deep}
    with pytest.raises(ValueError, match="nesting exceeds"):
        dumps(deep, format="toml")
    shared = {"x": 1}
    assert parse(dumps({"a": shared, "b": shared}, format="toml")) == {"a": shared, "b": shared}


def test_dump_serializes_before_writing_and_propagates_io_errors():
    fp = StringIO("unchanged")
    with pytest.raises(TypeError):
        dump({"bad": None}, fp, format="toml")
    assert fp.getvalue() == "unchanged"
    fp.close()
    with pytest.raises(ValueError, match="closed file"):
        dump({"x": 1}, fp, format="toml")


@pytest.mark.parametrize(
    ("format", "data", "parse_text"),
    [
        ("json", {"nested": [None, -(2**63), 2**64 - 1, "olá"]}, json.loads),
        ("toml", {"nested": {"date": date(2026, 11, 13)}}, tomllib.loads),
    ],
)
@pytest.mark.parametrize("pretty", [True, False])
def test_dump_formats(format, data, parse_text, pretty, tmp_path):
    text = dumps(data, format=format, pretty=pretty)
    assert text.endswith("\n")
    assert parse_text(text) == data

    path = tmp_path / f"config.{format.upper()}"
    assert dump(data, path, pretty=pretty) is None
    assert path.read_text(encoding="utf-8") == text

    path = tmp_path / "config.txt"
    assert dump(data, str(path), format=format, pretty=pretty) is None
    assert path.read_text(encoding="utf-8") == text

    stream = StringIO()
    assert dump(data, stream, format=format, pretty=pretty) is None
    assert stream.getvalue() == text


def test_dump_yaml(tmp_path):
    data = {"port": 443, "nested": {"missing": None, "label": "true"}}
    text = 'port: 443\nnested:\n  missing: null\n  label: "true"\n'
    assert dumps(data, format="yaml") == text

    for suffix in (".YAML", ".yml"):
        path = tmp_path / f"config{suffix}"
        dump(data, path)
        assert path.read_text(encoding="utf-8") == text

    stream = StringIO()
    dump(data, stream, format="yaml")
    assert stream.getvalue() == text

    with pytest.raises(ValueError, match="pretty=True"):
        dumps(data, format="yaml", pretty=False)


@pytest.mark.parametrize(
    ("format", "value", "error"),
    [
        ("json", date(2026, 11, 13), TypeError),
        ("json", math.inf, ValueError),
        ("json", 2**64, ValueError),
        ("toml", None, TypeError),
    ],
)
def test_dump_serializes_before_opening_path(format, value, error, tmp_path):
    path = tmp_path / f"config.{format}"
    path.write_text("unchanged", encoding="utf-8")
    with pytest.raises(error, match=r"nested.*items.*\[0\]"):
        dump({"nested": {"items": [value]}}, path)
    assert path.read_text(encoding="utf-8") == "unchanged"


def test_dump_requires_a_known_format(tmp_path):
    with pytest.raises(TypeError):
        dumps({})
    with pytest.raises(ValueError, match="format"):
        dumps({}, format="xml")
    with pytest.raises(ValueError, match="format"):
        dump({}, tmp_path / "config.xml")
    with pytest.raises(ValueError, match="format"):
        dump({}, StringIO())


def test_dumps_toml_datetime_with_unknown_offset_names_tzinfo():
    with pytest.raises(ValueError, match=r"utcoffset\(\) is None.*`when`"):
        dumps({"when": datetime(2026, 1, 1, tzinfo=NoOffset())}, format="toml")


@pytest.mark.parametrize("format", ["json", "toml"])
def test_dumps_circular_reference_raises_with_path(format):
    cycle: dict = {}
    cycle["self"] = cycle
    with pytest.raises(ValueError, match=r"circular reference at `a\.self`"):
        dumps({"a": cycle}, format=format)


@pytest.mark.parametrize("format", ["json", "toml"])
def test_dumps_non_string_key_raises_type_error(format):
    with pytest.raises(TypeError, match="keys must be strings at `nested`"):
        dumps({"nested": {1: "x"}}, format=format)


@pytest.mark.parametrize("format", ["json", "toml"])
def test_dumps_nesting_beyond_128_raises(format):
    deep: dict = {}
    for _ in range(128):
        deep = {"child": deep}
    with pytest.raises(ValueError, match="nesting exceeds 128"):
        dumps(deep, format=format)


def test_dumps_toml_time_with_tzinfo_raises():
    with pytest.raises(ValueError, match="times with a tzinfo"):
        dumps({"t": time(1, tzinfo=timezone.utc)}, format="toml")


@pytest.mark.parametrize("value", [2**63, -(2**63) - 1])
def test_dumps_toml_integer_outside_i64_raises(value):
    with pytest.raises(ValueError, match="64-bit range"):
        dumps({"n": value}, format="toml")


@pytest.mark.parametrize("value", [-(2**63), 2**63 - 1, 2**64 - 1])
def test_dumps_json_integer_within_64_bits_roundtrips(value):
    assert json.loads(dumps({"n": value}, format="json")) == {"n": value}


@pytest.mark.parametrize("value", [2**64, -(2**63) - 1])
def test_dumps_json_integer_outside_64_bits_raises(value):
    with pytest.raises(ValueError, match="64-bit"):
        dumps({"n": value}, format="json")


def test_dumps_json_none_and_unsigned_roundtrip():
    data = {"missing": None, "big": 2**64 - 1}
    assert json.loads(dumps(data, format="json")) == data


def test_dumps_yaml_keeps_none_and_unsigned():
    text = dumps({"missing": None, "big": 2**64 - 1}, format="yaml")
    assert text == "missing: null\nbig: 18446744073709551615\n"


def test_dumps_toml_unsigned_above_i64_is_rejected_not_rounded():
    with pytest.raises(ValueError, match="signed 64-bit"):
        dumps({"big": 2**64 - 1}, format="toml")


def test_dumps_yaml_keeps_non_finite_floats():
    text = dumps({"x": math.nan, "floats": [math.inf, -math.inf]}, format="yaml")
    assert text == "x: .nan\nfloats:\n- .inf\n- -.inf\n"


@pytest.mark.parametrize(
    ("format", "value", "error", "message"),
    [
        ("json", math.nan, ValueError, "non-finite floats"),
        ("json", date(2026, 11, 13), TypeError, "dates, times or datetimes"),
        ("yaml", date(2026, 11, 13), TypeError, "dates, times and datetimes"),
        ("toml", None, TypeError, "TOML cannot represent null"),
        ("toml", 2**64 - 1, ValueError, "signed 64-bit range"),
    ],
)
def test_dumps_rejects_what_the_format_cannot_represent(format, value, error, message):
    with pytest.raises(error, match=rf"^a\.b\[0\]: .*{message}"):
        dumps({"a": {"b": [value]}}, format=format)
