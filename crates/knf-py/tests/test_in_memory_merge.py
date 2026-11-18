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


def test_deep_merge_keeps_previously_interpolated_values(tmp_path):
    base = tmp_path / "base.toml"
    base.write_text('root = "old"\npath = "${root}/data"\n', encoding="utf-8")

    cfg = load([base], interpolate=True)
    assert deep_merge(cfg, {"root": "new"}) == {"root": "new", "path": "old/data"}
