"""Whole-string references merge as the values they name."""

import pytest

from knf import InterpolationError, load


def write(tmp_path, name, text):
    path = tmp_path / name
    path.write_text(text)
    return path


@pytest.mark.parametrize("format", ["json", "toml"])
def test_a_referenced_object_merges_with_a_later_object(tmp_path, format):
    if format == "json":
        texts = ['{"bar":{"a":1,"d":1},"foo":{"b":"${bar}"}}', '{"foo":{"b":{"a":4}}}', '{"bar":{"d":9}}']
    else:
        texts = ["[bar]\na = 1\nd = 1\n[foo]\nb = '${bar}'\n", "[foo.b]\na = 4\n", "[bar]\nd = 9\n"]
    base, over, late = (write(tmp_path, f"{name}.{format}", text) for name, text in zip(["base", "over", "late"], texts))
    assert load([base, over], interpolate=True)["foo"] == {"b": {"a": 4, "d": 1}}
    assert load([base, over, late], interpolate=True)["foo"] == {"b": {"a": 4, "d": 9}}
    assert load([base, over], interpolate=True, shallow="foo.b")["foo"] == {"b": {"a": 4}}
    assert load([base, over])["foo"] == {"b": {"a": 4}}


def test_a_reference_deciding_a_merge_must_resolve(tmp_path):
    base = write(tmp_path, "base.json", '{"db":"${env:KNF_UNSET_DB}","gone":"${missing}"}')
    over = write(tmp_path, "over.json", '{"db":{"host":"prod"},"gone":1}')
    with pytest.raises(InterpolationError, match="db: `env:KNF_UNSET_DB`"):
        load([base, over], interpolate=True)
