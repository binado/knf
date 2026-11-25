from datetime import date

from knf import deep_merge, load


def test_deep_merge_preserves_inputs_and_nested_siblings():
    day = date(2024, 3, 14)
    cfg = {"solver": {"steps": 100, "name": "pilot"}, "date": day}
    overrides = {"solver": {"steps": 10}, "replicate": 2}

    merged = deep_merge(cfg, overrides)

    assert merged == {
        "solver": {"steps": 10, "name": "pilot"},
        "date": day,
        "replicate": 2,
    }
    assert merged is not cfg
    assert merged is not overrides
    assert cfg == {"solver": {"steps": 100, "name": "pilot"}, "date": day}
    assert overrides == {"solver": {"steps": 10}, "replicate": 2}


def test_deep_merge_replaces_non_dict_pairs_wholesale():
    cfg = {"items": [1, 2], "scalar": 1, "table": {"a": 1}, "nullable": "value"}
    overrides = {"items": [3], "scalar": {"a": 2}, "table": 2, "nullable": None}

    assert deep_merge(cfg, overrides) == overrides


def test_deep_merge_does_not_reinterpolate_resolved_values(tmp_path):
    base = tmp_path / "base.toml"
    base.write_text('root = "old"\npath = "${root}/data"\n', encoding="utf-8")

    cfg = load([base], interpolate=True)
    assert deep_merge(cfg, {"root": "new"}) == {"root": "new", "path": "old/data"}


def test_deep_merge_returns_an_independent_tree():
    shared_list = [1, 2]
    shared_table = {"a": 1}
    cfg = {"solver": {"steps": 100, "items": shared_list}}
    overrides = {"solver": {"steps": 10}, "table": shared_table}

    merged = deep_merge(cfg, overrides)

    merged["solver"]["steps"] = -1
    merged["solver"]["items"].append(3)
    merged["table"]["a"] = 9
    assert cfg["solver"] == {"steps": 100, "items": [1, 2]}
    assert overrides == {"solver": {"steps": 10}, "table": {"a": 1}}
    assert merged["solver"]["items"] is not shared_list
    assert merged["table"] is not shared_table


def test_deep_merge_handles_empty_inputs():
    cfg = {"a": {"b": 1}}
    assert deep_merge(cfg, {}) == cfg
    assert deep_merge(cfg, {}) is not cfg
    assert deep_merge({}, cfg) == cfg
    assert deep_merge({}, {}) == {}


def test_deep_merge_merges_several_levels_and_keeps_key_order():
    cfg = {"a": {"b": {"c": 1, "d": 2}}, "z": 0}
    overrides = {"new": 1, "a": {"b": {"c": 9}, "e": 3}}

    merged = deep_merge(cfg, overrides)

    assert merged == {"a": {"b": {"c": 9, "d": 2}, "e": 3}, "z": 0, "new": 1}
    assert list(merged) == ["a", "z", "new"]
    assert list(merged["a"]) == ["b", "e"]


def test_deep_merge_none_and_scalars_replace_dicts():
    assert deep_merge({"a": {"b": 1}}, {"a": None}) == {"a": None}
    assert deep_merge({"a": None}, {"a": {"b": 1}}) == {"a": {"b": 1}}
    assert deep_merge({"a": [1]}, {"a": {"b": 1}}) == {"a": {"b": 1}}
