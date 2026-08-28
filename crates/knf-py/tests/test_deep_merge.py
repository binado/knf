"""The binding, as a consumer sees it.

Every fixture is written into `tmp_path` rather than committed, so a test says
what document it is about in the same place it says what should happen to it.
Nothing here reads the ambient environment: the one test that could sets both
`os.environ` and `env=` explicitly, precisely to show they are different things.
"""

from __future__ import annotations

import datetime
import json
import math
import os

import pytest

import knf

# --- the identity property -------------------------------------------------


def test_one_layer_is_the_identity(tmp_path):
    """The Python echo of the byte-level no-op proptest in knf-core.

    One layer, no options: whatever the file said, unchanged. Nulls are ordinary
    values rather than delete instructions, which is what makes this hold at all.
    """
    document = {
        "name": "svc",
        "port": 8080,
        "ratio": 1.5,
        "debug": True,
        "proxy": None,
        "tags": ["a", "b"],
        "db": {"host": "localhost", "opts": {"tls": False}},
        "empty": {},
        "holes": [1, None, 3],
    }
    path = tmp_path / "app.json"
    path.write_text(json.dumps(document))

    assert knf.deep_merge([path]) == document


def test_key_order_is_the_document_order(tmp_path):
    path = tmp_path / "ordered.json"
    path.write_text('{"z": 1, "a": 2, "m": {"y": 1, "b": 2}}')

    merged = knf.deep_merge([path])

    assert list(merged) == ["z", "a", "m"]
    assert list(merged["m"]) == ["y", "b"]


def test_no_arguments_at_all_is_an_empty_document():
    assert knf.deep_merge([]) == {}


# --- the fold --------------------------------------------------------------


@pytest.fixture
def layers(tmp_path):
    """A TOML base and a JSON override, which is the mixed-format case."""
    base = tmp_path / "base.toml"
    base.write_text(
        "name = 'svc'\n"
        "plugins = ['a', 'b']\n"
        "\n[db]\n"
        "host = 'localhost'\n"
        "port = 5432\n"
    )
    prod = tmp_path / "prod.json"
    prod.write_text(
        json.dumps({"plugins": ["c"], "db": {"host": "prod.example"}, "extra": 1})
    )
    return base, prod


def test_layers_fold_left_to_right_across_formats(layers):
    base, prod = layers

    assert knf.deep_merge([base, prod]) == {
        "name": "svc",
        # Arrays replace wholesale unless a rule says otherwise.
        "plugins": ["c"],
        "db": {"host": "prod.example", "port": 5432},
        "extra": 1,
    }
    # The fold is strictly left, so the order of the arguments is the answer.
    assert knf.deep_merge([prod, base])["db"]["host"] == "localhost"


def test_paths_accept_os_pathlike_and_str(layers):
    base, prod = layers
    assert knf.deep_merge([base, prod]) == knf.deep_merge([str(base), str(prod)])


def test_paths_and_overlays_take_any_iterable(layers):
    """A generator, not only a list — nothing here indexes its arguments."""
    base, prod = layers

    merged = knf.deep_merge(
        (p for p in (base, prod)), overlays=({"n": n} for n in (1, 2))
    )

    assert merged["n"] == 2


def test_the_option_arguments_take_any_mapping(layers):
    """`rules=` and `env=` are options, so a `Mapping` that is not a `dict` —
    a `MappingProxyType`, an `OrderedDict`, a `TypedDict` — works."""
    import collections
    import types

    base, prod = layers
    rules = types.MappingProxyType({"plugins": "append"})

    assert knf.deep_merge([base, prod], rules=rules)["plugins"] == ["a", "b", "c"]

    path = base.parent / "ref.json"
    path.write_text('{"port": "${env:PORT}"}')
    env = collections.OrderedDict(PORT="8080")
    assert knf.deep_merge([path], interpolate=True, env=env)["port"] == 8080


def test_rules_and_env_must_be_mappings():
    with pytest.raises(TypeError, match="rules="):
        knf.deep_merge([], rules=5)
    with pytest.raises(TypeError, match="env="):
        knf.deep_merge([], env=5)


def test_strict_rejects_a_layer_that_changes_a_type(tmp_path):
    base = tmp_path / "base.json"
    base.write_text('{"db": {"host": "localhost"}}')
    over = tmp_path / "over.json"
    over.write_text('{"db": {"host": 5432}}')

    # Without --strict this is an ordinary override.
    assert knf.deep_merge([base, over])["db"]["host"] == 5432

    with pytest.raises(knf.MergeError) as caught:
        knf.deep_merge([base, over], strict=True)

    # The structured half, so nothing has to be recovered from the message.
    assert caught.value.path == ("db", "host")
    assert "db.host" in str(caught.value)


# --- rules -----------------------------------------------------------------


def test_append_concatenates_only_at_its_path(layers):
    base, prod = layers

    merged = knf.deep_merge([base, prod], rules={"plugins": "append"})

    assert merged["plugins"] == ["a", "b", "c"]


def test_append_does_not_double_a_lone_layer(layers):
    """A strategy applies only where the accumulator already holds a value."""
    base, _ = layers

    assert knf.deep_merge([base], rules={"plugins": "append"})["plugins"] == ["a", "b"]


def test_replace_takes_the_later_table_whole(layers):
    base, prod = layers

    merged = knf.deep_merge([base, prod], rules={"db": "replace"})

    # `port` came only from the base, and replace does not recurse into it.
    assert merged["db"] == {"host": "prod.example"}


def test_fail_pins_a_path_to_the_first_layer_that_sets_it(layers):
    base, prod = layers

    with pytest.raises(knf.MergeError) as caught:
        knf.deep_merge([base, prod], rules={"db.host": "fail"})

    assert caught.value.path == ("db", "host")

    # The path may still be absent from every later layer.
    assert knf.deep_merge([base], rules={"db.host": "fail"})["db"]["host"] == "localhost"


def test_rule_order_never_changes_the_result(layers):
    base, prod = layers

    forwards = knf.deep_merge([base, prod], rules={"plugins": "append", "db": "replace"})
    backwards = knf.deep_merge([base, prod], rules={"db": "replace", "plugins": "append"})

    assert forwards == backwards


def test_a_rule_beneath_another_can_never_fire():
    """Rejected when the set is built, so before any file is opened."""
    with pytest.raises(knf.RuleError) as caught:
        knf.deep_merge(["does-not-exist.json"], rules={"db": "replace", "db.host": "fail"})

    assert caught.value.paths == (("db", "host"),)


def test_an_unknown_strategy_is_a_value_error():
    with pytest.raises(ValueError, match="append"):
        knf.deep_merge([], rules={"db": "merge"})


def test_a_rule_may_not_name_an_array_element():
    with pytest.raises(knf.PathError):
        knf.deep_merge([], rules={"servers[0].host": "replace"})


# --- overlays --------------------------------------------------------------


def test_overlays_are_terminal_layers(layers):
    base, prod = layers

    merged = knf.deep_merge([base, prod], overlays=[{"db": {"port": 1}}, {"db": {"port": 2}}])

    assert merged["db"] == {"host": "prod.example", "port": 2}


def test_overlays_are_ordinary_layers_under_a_rule(layers):
    base, _ = layers

    merged = knf.deep_merge([base], rules={"db": "replace"}, overlays=[{"db": {"host": "x"}}])

    assert merged["db"] == {"host": "x"}


def test_an_overlay_carries_every_python_scalar():
    merged = knf.deep_merge(
        [],
        overlays=[
            {
                "none": None,
                "yes": True,
                "no": False,
                "int": 7,
                "float": 1.5,
                "str": "s",
                "list": [1, [2], {"k": 3}],
                "tuple": (1, 2),
                "dict": {"nested": {"deep": 1}},
            }
        ],
    )

    # A bool must not arrive as an int: Python's bool subclasses int, and the
    # conversion tests for it first precisely so this holds.
    assert merged["yes"] is True
    assert merged["int"] == 7 and not isinstance(merged["int"], bool)
    assert merged["tuple"] == [1, 2]
    assert merged["dict"] == {"nested": {"deep": 1}}


def test_an_oversized_int_in_an_overlay_is_rejected_by_path():
    """Never truncated — the digits are the value."""
    with pytest.raises(OverflowError) as caught:
        knf.deep_merge([], overlays=[{"a": {"ids": [1, 2**70]}}])

    assert "overlays[0].a.ids[1]" in str(caught.value)


def test_every_offender_in_an_overlay_is_named_at_once():
    with pytest.raises(TypeError) as caught:
        knf.deep_merge([], overlays=[{"a": {"b"}, "c": [object()]}])

    message = str(caught.value)
    assert "overlays[0].a" in message
    assert "overlays[0].c[0]" in message


def test_a_datetime_may_not_be_written_into_a_layer():
    """A `Value::Datetime` carries a TOML source spelling and is only ever read.

    Synthesising one from a Python object would be inventing that spelling from
    outside the grammar that owns it.
    """
    with pytest.raises(TypeError, match="datetime"):
        knf.deep_merge([], overlays=[{"when": datetime.datetime(1979, 5, 27)}])


def test_a_non_string_key_is_rejected():
    with pytest.raises(TypeError, match="str"):
        knf.deep_merge([], overlays=[{"a": {1: "x"}}])


def test_a_self_referential_overlay_raises_rather_than_overflowing_the_stack():
    loop: dict = {}
    loop["self"] = loop

    with pytest.raises(ValueError, match="deep"):
        knf.deep_merge([], overlays=[loop])


def test_an_overlay_must_be_a_mapping():
    """Coerced nowhere: a layer is an object, the rule every file follows. A list
    of pairs is what `dict()` would happily accept, which is exactly why the
    *option* arguments take that coercion and the data argument does not."""
    with pytest.raises(TypeError, match=r"overlays\[0\]"):
        knf.deep_merge([], overlays=[[("a", 1)]])


def test_overlays_must_be_iterable():
    with pytest.raises(TypeError, match="overlays="):
        knf.deep_merge([], overlays=5)


# --- interpolation ---------------------------------------------------------


def test_interpolation_is_off_by_default(tmp_path):
    """knf sits upstream of tools whose own syntax is ${...}."""
    path = tmp_path / "compose.json"
    path.write_text('{"image": "app:${TAG}"}')

    assert knf.deep_merge([path])["image"] == "app:${TAG}"


def test_references_resolve_over_the_merged_document(tmp_path):
    path = tmp_path / "app.toml"
    path.write_text(
        "root = '/srv'\n"
        "data_dir = '${root}/data'\n"
        "port = '${env:PORT}'\n"
        "url = 'x:${env:PORT}'\n"
        "literal = '$${NOT_A_REF}'\n"
    )

    merged = knf.deep_merge([path], interpolate=True, env={"PORT": "8080"})

    assert merged["data_dir"] == "/srv/data"
    # Whole-string, so it takes a type: an int, exactly as `--set port=8080` does.
    assert merged["port"] == 8080
    # Embedded, so it splices raw text.
    assert merged["url"] == "x:8080"
    assert merged["literal"] == "${NOT_A_REF}"


def test_env_is_a_snapshot_and_never_os_environ(tmp_path, monkeypatch):
    path = tmp_path / "app.json"
    path.write_text('{"port": "${env:PORT}", "other": "${env:ONLY_IN_PROCESS}"}')

    monkeypatch.setenv("PORT", "9999")
    monkeypatch.setenv("ONLY_IN_PROCESS", "yes")

    # `env=` replaces the environment rather than extending it, so a variable
    # only the process has is unset as far as the merge is concerned.
    with pytest.raises(knf.InterpolationError) as caught:
        knf.deep_merge([path], interpolate=True, env={"PORT": "8080"})

    # The reference is the spelling inside the braces, as knf-interp reports it.
    assert caught.value.problems == (("unresolved", "other", "env:ONLY_IN_PROCESS"),)

    # And the value that is given wins over the process's.
    path.write_text('{"port": "${env:PORT}"}')
    assert knf.deep_merge([path], interpolate=True, env={"PORT": "8080"})["port"] == 8080
    # Without `env=`, the process environment is what is read.
    assert knf.deep_merge([path], interpolate=True)["port"] == 9999


def test_an_empty_env_shadows_the_process_entirely(tmp_path, monkeypatch):
    path = tmp_path / "app.json"
    path.write_text('{"port": "${env:PORT}"}')
    monkeypatch.setenv("PORT", "9999")

    with pytest.raises(knf.InterpolationError):
        knf.deep_merge([path], interpolate=True, env={})


def test_a_cycle_arrives_alone(tmp_path):
    path = tmp_path / "cycle.json"
    path.write_text('{"a": "${b}", "b": "${a}"}')

    with pytest.raises(knf.InterpolationError) as caught:
        knf.deep_merge([path], interpolate=True)

    # Resolution cannot continue past a cycle, so there is no list to hand over.
    assert caught.value.problems == ()


# --- datetimes -------------------------------------------------------------


TOML_DATETIMES = """\
offset = 1979-05-27T07:32:00Z
offset_west = 1979-05-27T00:32:00.999999-07:00
local = 1979-05-27T07:32:00
date = 1979-05-27
time = 07:32:00.5
"""


def test_the_four_toml_datetime_forms_match_tomllib(tmp_path):
    """The same file through `tomllib.load` and through knf agrees, value for
    value — one grammar, decomposed rather than re-implemented."""
    tomllib = pytest.importorskip("tomllib")

    path = tmp_path / "when.toml"
    path.write_text(TOML_DATETIMES)

    with path.open("rb") as handle:
        expected = tomllib.load(handle)

    assert knf.deep_merge([path]) == expected


def test_datetimes_have_the_python_types_they_should(tmp_path):
    """Spelled out rather than left to the tomllib comparison, which is skipped
    on Python 3.9 and 3.10."""
    path = tmp_path / "when.toml"
    path.write_text(TOML_DATETIMES)

    merged = knf.deep_merge([path])

    assert merged["offset"] == datetime.datetime(
        1979, 5, 27, 7, 32, tzinfo=datetime.timezone.utc
    )
    assert merged["offset_west"] == datetime.datetime(
        1979, 5, 27, 0, 32, 0, 999999, tzinfo=datetime.timezone(datetime.timedelta(hours=-7))
    )
    # No offset at all, which is a naive datetime rather than a UTC one.
    assert merged["local"] == datetime.datetime(1979, 5, 27, 7, 32)
    assert merged["local"].tzinfo is None
    assert merged["date"] == datetime.date(1979, 5, 27)
    assert merged["time"] == datetime.time(7, 32, 0, 500000)


def test_a_datetime_survives_a_mixed_format_merge(tmp_path):
    """It stays a datetime rather than becoming a string on the way through."""
    base = tmp_path / "base.toml"
    base.write_text("created = 1979-05-27T07:32:00Z\n")
    over = tmp_path / "over.json"
    over.write_text('{"other": 1}')

    assert isinstance(knf.deep_merge([base, over])["created"], datetime.datetime)


# --- what Python, uniquely, does not reject --------------------------------


def test_non_finite_floats_survive(tmp_path):
    """`knf -f json` refuses these; JSON has no syntax for them and Python does."""
    path = tmp_path / "timeouts.toml"
    path.write_text("a = inf\nb = -inf\nc = nan\n")

    merged = knf.deep_merge([path])

    assert merged["a"] == math.inf
    assert merged["b"] == -math.inf
    assert math.isnan(merged["c"])


def test_an_integer_past_i64_max_survives(tmp_path):
    """`knf -f toml` refuses this; TOML integers are signed 64-bit, and Python's
    are arbitrary precision."""
    path = tmp_path / "ids.json"
    path.write_text('{"id": 18446744073709551615}')

    assert knf.deep_merge([path])["id"] == 18446744073709551615


def test_a_null_survives(tmp_path):
    """`knf -f toml` refuses this too, TOML having no null at all."""
    path = tmp_path / "proxy.json"
    path.write_text('{"proxy": null, "xs": [1, null]}')

    assert knf.deep_merge([path]) == {"proxy": None, "xs": [1, None]}


# --- loading ---------------------------------------------------------------


def test_an_unknown_extension_needs_an_explicit_format(tmp_path):
    path = tmp_path / "app.conf"
    path.write_text('{"a": 1}')

    with pytest.raises(knf.LoadError) as caught:
        knf.deep_merge([path])
    assert caught.value.path == str(path)
    assert "input_format=" in str(caught.value)

    assert knf.deep_merge([path], input_format="json") == {"a": 1}


def test_a_directory_is_not_a_layer(tmp_path):
    with pytest.raises(knf.LoadError) as caught:
        knf.deep_merge([tmp_path])

    assert caught.value.path == str(tmp_path)


def test_a_missing_file_is_the_builtin_for_it(tmp_path):
    with pytest.raises(FileNotFoundError):
        knf.deep_merge([tmp_path / "absent.json"])


def test_a_malformed_document_is_a_parse_error(tmp_path):
    path = tmp_path / "broken.json"
    path.write_text("{not json")

    with pytest.raises(knf.ParseError):
        knf.deep_merge([path])


def test_every_input_must_be_an_object_at_the_top_level(tmp_path):
    path = tmp_path / "array.json"
    path.write_text("[1, 2]")

    with pytest.raises(knf.ParseError):
        knf.deep_merge([path])


def test_stdin_has_no_extension_to_infer_from():
    with pytest.raises(knf.LoadError) as caught:
        knf.deep_merge(["-"])

    assert caught.value.path is None
    assert "input_format=" in str(caught.value)


def test_input_format_takes_the_two_formats_there_are():
    with pytest.raises(ValueError, match="input_format="):
        knf.deep_merge([], input_format="yaml")


def test_a_bare_string_is_not_an_iterable_of_paths():
    """It *is* iterable, which is exactly why it is rejected by name: without
    this it would become one layer per character."""
    with pytest.raises(TypeError, match="paths="):
        knf.deep_merge("base.json")


def test_paths_must_be_paths():
    with pytest.raises(TypeError, match=r"paths\[1\]"):
        knf.deep_merge(["a.json", 7])


# --- the exception hierarchy -----------------------------------------------


def test_every_pipeline_error_is_a_knf_error():
    for exception in (
        knf.LoadError,
        knf.ParseError,
        knf.MergeError,
        knf.RuleError,
        knf.InterpolationError,
        knf.PathError,
    ):
        assert issubclass(exception, knf.KnfError)
    assert issubclass(knf.KnfError, Exception)


def test_the_package_is_typed():
    """`py.typed` and the stubs ship beside the extension, which is the whole
    reason the compiled module is `_knf` behind a Python wrapper."""
    package = os.path.dirname(knf.__file__)
    assert os.path.exists(os.path.join(package, "py.typed"))
    assert os.path.exists(os.path.join(package, "__init__.pyi"))
