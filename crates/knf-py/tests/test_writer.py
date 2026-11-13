from datetime import date, datetime, time, timedelta, timezone
from io import StringIO
import math

import pytest

from knf import dump_toml, dumps_toml, load


def parse(text):
    return pytest.importorskip("tomllib").loads(text)


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
    text = dumps_toml(data, pretty=pretty)
    assert text.endswith("\n")
    assert parse(text) == data
    path = tmp_path / "out.toml"
    with path.open("w", encoding="utf-8") as fp:
        assert dump_toml(data, fp, pretty=pretty) is None
    assert load([path]) == data
    assert list(parse(text))[:3] == list(data)[:3]


def test_special_values():
    result = parse(dumps_toml({"tuple": (1, 2), "inf": math.inf, "nan": math.nan}))
    assert result["tuple"] == [1, 2]
    assert result["inf"] == math.inf
    assert math.isnan(result["nan"])
    assert parse(dumps_toml({})) == {}


@pytest.mark.parametrize("value, error", [
    (None, TypeError), (object(), TypeError), (b"bytes", TypeError),
    (2**63, ValueError), (-(2**63) - 1, ValueError),
    (time(tzinfo=timezone.utc), ValueError),
    (datetime(2026, 1, 1, tzinfo=timezone(timedelta(seconds=1))), ValueError),
])
def test_invalid_values_include_path(value, error):
    with pytest.raises(error, match=r"nested.*items.*\[0\]"):
        dumps_toml({"nested": {"items": [value]}})


def test_invalid_keys_and_root():
    with pytest.raises(TypeError, match="keys must be strings"):
        dumps_toml({"nested": {1: "x"}})
    with pytest.raises(TypeError):
        dumps_toml([])


def test_cycles_depth_and_shared_values():
    cycle = []
    cycle.append(cycle)
    with pytest.raises(ValueError, match="circular reference"):
        dumps_toml({"cycle": cycle})
    deep = {}
    for _ in range(128):
        deep = {"child": deep}
    with pytest.raises(ValueError, match="nesting exceeds"):
        dumps_toml(deep)
    shared = {"x": 1}
    assert parse(dumps_toml({"a": shared, "b": shared})) == {"a": shared, "b": shared}


def test_dump_serializes_before_writing_and_propagates_io_errors():
    fp = StringIO("unchanged")
    with pytest.raises(TypeError):
        dump_toml({"bad": None}, fp)
    assert fp.getvalue() == "unchanged"
    fp.close()
    with pytest.raises(ValueError, match="closed file"):
        dump_toml({"x": 1}, fp)
