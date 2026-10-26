"""Staged native admission remains bounded, strict, and consumer-owned."""

import json
from decimal import Decimal

import daml
import pytest


def compose(types):
    return daml.Schemas.compose(
        [daml.SchemaModule.from_json(json.dumps({"types": types}))]
    )


def number(raw):
    return {"kind": "number", "value": raw}


def mapping(**fields):
    return {
        "kind": "map",
        "entries": [{"key": key, "value": value} for key, value in fields.items()],
    }


def field(value, key):
    return next(entry["value"] for entry in value["entries"] if entry["key"] == key)


class Stage:
    def __init__(self, materialize=None, defer=None, number_class=None):
        self.on_materialize = materialize
        self.on_defer = defer
        if number_class is not None:
            self.number_class = number_class

    def defer(self, summary, path):
        return self.on_defer(summary, path) if self.on_defer else False

    def materialize(self, value, copies, round):
        if self.on_materialize:
            return self.on_materialize(value, copies, round)
        return None


def defer_reference(summary, path):
    return summary["kind"] == "map" and any(
        field["key"] == "ref" for field in summary["fields"]
    )


def test_source_and_semantic_stages_copy_only_after_native_completion():
    schemas = compose(
        {
            "Root": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "source": {"type": "integer", "default": 7},
                        "target": {"type": "integer"},
                    },
                    "required": ["source", "target"],
                }
            }
        }
    )
    document = daml.parse('!Root { target: { ref: "/source" } }')
    for supplied in [document, document.value()]:
        seen = []

        def materialize(value, copies, round):
            assert field(value, "source") == number("7")
            seen.append(copies)
            if field(value, "target")["kind"] == "map":
                return [{"op": "copy", "path": "/target", "from": "/source"}]
            assert copies == [{"path": "/target", "from": "/source"}]
            value["entries"].clear()
            return b"completed-plan"

        stage = Stage(materialize=materialize, defer=defer_reference)
        if isinstance(supplied, daml.Document):
            result = schemas.resolve_staged(supplied, stage)
        else:
            result = schemas.resolve_semantic_staged(supplied, stage)
        assert result.to_python() == {"source": 7, "target": 7}
        assert result.meaning() == b"completed-plan"
        assert seen == [[], [{"path": "/target", "from": "/source"}]]


def test_defer_summary_is_shallow_and_precedes_ancestor_record_default_merges():
    schemas = compose(
        {
            "Root": {
                "schema": {
                    "type": "object",
                    "default": {"target": {"context": 1}},
                    "properties": {
                        "source": {"default": {"own": 7}},
                        "target": {"type": "object"},
                    },
                }
            }
        }
    )
    document = daml.parse('!Root { target: { ref: "/source" } }')
    marker_summaries = []

    def defer(summary, path):
        if path == "":
            target = next(
                field["summary"]
                for field in summary["fields"]
                if field["key"] == "target"
            )
            assert target == {"kind": "map", "length": 1}
        if path == "/target" and defer_reference(summary, path):
            marker_summaries.append(summary)
        return defer_reference(summary, path)

    def materialize(value, copies, round):
        target = field(value, "target")
        if any(entry["key"] == "ref" for entry in target["entries"]):
            assert target == mapping(ref={"kind": "string", "value": "/source"})
            return [{"op": "copy", "path": "/target", "from": "/source"}]
        return None

    result = schemas.resolve_staged(document, Stage(materialize, defer))
    assert result.to_python() == {"source": {"own": 7}, "target": {"own": 7}}
    assert marker_summaries[0] == {
        "kind": "map",
        "length": 1,
        "fields": [{"key": "ref", "summary": {"kind": "string", "value": "/source"}}],
    }


def test_opaque_default_cannot_lend_fields_or_its_tag_to_a_supplied_record():
    schemas = compose(
        {
            "Root": {
                "schema": {
                    "properties": {
                        "slot": {
                            "type": "object",
                            "default": {"ref": "/source", "nested": None},
                            "x-daml-default-tags": [
                                {"path": "", "tag": "Marker"},
                                {"path": "/nested", "tag": "Missing"},
                            ],
                        }
                    }
                }
            },
            "Marker": {"schema": {"type": "object"}},
        }
    )
    result = schemas.resolve_staged(
        daml.parse("!Root { slot: {} }"), Stage(defer=defer_reference)
    )
    assert result.to_python() == {"slot": {}}
    assert field(result.value(), "slot") == mapping()


def test_ready_never_turns_an_opaque_placeholder_into_checked_data():
    schemas = compose({"Count": {"schema": {"type": "integer"}}})
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse('{ ref: "/missing" }'),
            Stage(defer=defer_reference),
            expected="Count",
        )
    assert caught.value.kind == "SchemaViolation"


def test_initial_typed_edits_suppress_default_conflicts_and_expect_actual_target():
    bundle = daml.Bundle.from_sources(
        ".",
        {
            "daml.bundle": "{}",
            "Root.daml": "!Object { target: !Any }",
            "Target.daml": "closed !Object { count: !Int { min: 1 } } = { count: 4 }",
        },
    )
    document = daml.parse("!self.Root { target: {} }")
    schemas = bundle.compile(document, additional_types=["self.Target"])
    initial = [{"op": "expect", "path": "/target", "name": "self.Target"}]
    result = schemas.resolve_staged(document, Stage(), initial_edits=initial)
    assert result.to_python() == {"target": {"count": 4}}
    assert result.meaning() is None
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("!self.Root { target: { extra: true } }"),
            Stage(),
            initial_edits=initial,
        )
    assert caught.value.kind == "SchemaViolation"

    choices = compose(
        {
            "Choice": {
                "schema": {
                    "anyOf": [
                        {"properties": {"n": {"default": 1}}},
                        {"properties": {"n": {"default": 2}}},
                    ]
                }
            }
        }
    )
    result = choices.resolve_staged(
        daml.parse("{}"),
        Stage(),
        expected="Choice",
        initial_edits=[{"op": "replace", "path": "/n", "value": number("3")}],
    )
    assert result.to_python() == {"n": 3}


def test_expect_round_exposes_typed_defaults_containing_a_further_reference():
    schemas = compose(
        {
            "Root": {
                "schema": {"properties": {"source": {"default": 9}, "target": {}}}
            },
            "Target": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "child": {"type": "integer", "default": {"ref": "/source"}}
                    },
                    "required": ["child"],
                }
            },
        }
    )
    seen = []

    def materialize(value, copies, round):
        target = field(value, "target")
        seen.append(round)
        if not target["entries"]:
            return [{"op": "expect", "path": "/target", "name": "Target"}]
        if field(target, "child")["kind"] == "map":
            return [{"op": "copy", "path": "/target/child", "from": "/source"}]
        return None

    result = schemas.resolve_staged(
        daml.parse("!Root { target: {} }"),
        Stage(materialize, defer_reference),
    )
    assert result.to_python() == {"source": 9, "target": {"child": 9}}
    assert seen == [0, 1, 2]


def test_copy_preserves_different_assertions_and_rebases_transitive_obligations():
    schemas = compose(
        {
            "Source": {"schema": {"type": "integer", "minimum": 5}},
            "Destination": {"schema": {"type": "integer", "maximum": 10}},
            "Final": {"schema": {"type": "integer", "minimum": 1}},
        }
    )
    document = daml.parse(
        "{ source: !Source 7\ndest: !Destination 8\nfinal: !Final 9 }"
    )
    rounds = [
        [{"op": "copy", "path": "/dest", "from": "/source"}],
        [{"op": "copy", "path": "/final", "from": "/dest"}],
    ]
    history = [
        {"path": "/dest", "from": "/source"},
        {"path": "/final", "from": "/dest"},
    ]

    def materialize(value, copies, round):
        assert copies == history[: min(round, len(history))]
        return rounds[round] if round < len(rounds) else None

    result = schemas.resolve_staged(document, Stage(materialize))
    assert result.to_python() == {"source": 7, "dest": 7, "final": 7}
    assert result.tags() == {
        "/source": "Source",
        "/dest": "Destination",
        "/final": "Final",
    }

    def violate(value, copies, round):
        assert copies == history[: min(round, len(history))]
        if round < len(rounds):
            return rounds[round]
        if round == 2:
            return [{"op": "replace", "path": "/final", "value": number("3")}]
        return None

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(document, Stage(violate))
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/final"


def test_copy_with_default_origin_has_supplied_assertion_precedence():
    schemas = compose(
        {
            "Root": {
                "schema": {
                    "properties": {
                        "source": {
                            "default": {"own": 2},
                            "x-daml-default-tags": [{"path": "", "tag": "Provider"}],
                        },
                        "target": {"default": {"context": True, "own": 1}},
                    }
                }
            },
            "Provider": {"schema": {"type": "object"}},
        }
    )

    def materialize(value, copies, round):
        if round == 0:
            return [{"op": "copy", "path": "/target", "from": "/source"}]
        return None

    result = schemas.resolve_staged(daml.parse("!Root"), Stage(materialize))
    assert result.to_python() == {"source": {"own": 2}, "target": {"own": 2}}
    assert result.tags()["/target"] == "Provider"


def numeric_choices(combiner):
    return daml.Schemas.compose(
        [
            daml.SchemaModule.from_json(
                '{"types":{"Choice":{"schema":{"type":"object","'
                + combiner
                + '":[{"properties":{"n":{"default":1}}},'
                '{"properties":{"n":{"default":1.0}}}]}}}}'
            )
        ]
    )


def number_representation_class(lexeme):
    if any(char in lexeme for char in ".eE"):
        return 2 if Decimal(lexeme).is_zero() and lexeme.startswith("-") else 1
    return 0


@pytest.mark.parametrize("combiner", ["allOf", "anyOf"])
def test_number_classes_refine_internal_merges_and_final_candidate_equality(combiner):
    schemas = numeric_choices(combiner)
    document = daml.parse("!Choice")
    ordinary = schemas.resolve_staged(document, Stage())
    assert ordinary.to_python()["n"] == 1
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            document, Stage(number_class=number_representation_class)
        )
    assert caught.value.kind == "AmbiguousDefault"


def test_number_class_cannot_coarsen_native_equality_or_change_schema_mathematics():
    schemas = compose(
        {
            "Different": {
                "schema": {
                    "anyOf": [
                        {"properties": {"n": {"default": 1}}},
                        {"properties": {"n": {"default": 2}}},
                    ]
                }
            },
            "Unique": {"schema": {"type": "array", "uniqueItems": True}},
            "One": {"schema": {"const": 1}},
        }
    )
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("!Different"), Stage(number_class=lambda raw: 0)
        )
    assert caught.value.kind == "AmbiguousDefault"
    stage = Stage(number_class=number_representation_class)
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(daml.parse("[1\n1.0]"), stage, expected="Unique")
    assert caught.value.kind == "SchemaViolation"
    result = schemas.resolve_staged(daml.parse("1.0"), stage, expected="One")
    assert isinstance(result.to_python(), Decimal)
    assert result.to_python() == Decimal("1")


def test_negative_real_zero_can_be_a_distinct_class_without_changing_its_value():
    schemas = daml.Schemas.compose(
        [
            daml.SchemaModule.from_json(
                '{"types":{"Choice":{"schema":{"anyOf":['
                '{"properties":{"n":{"default":0.0}}},'
                '{"properties":{"n":{"default":-0.0}}}'
                "]}}}}"
            )
        ]
    )
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("!Choice"), Stage(number_class=number_representation_class)
        )
    assert caught.value.kind == "AmbiguousDefault"


def plan_choices():
    return compose(
        {
            "Choice": {
                "schema": {
                    "anyOf": [
                        {"properties": {"pick": {"default": "first"}}},
                        {"properties": {"pick": {"default": "second"}}},
                    ]
                }
            }
        }
    )


@pytest.mark.parametrize("distinct_meanings", [False, True])
def test_ready_meanings_refine_equal_final_values_and_round_zero_resets_plans(
    distinct_meanings,
):
    schemas = plan_choices()
    current = None
    starts = []
    initial = [{"op": "replace", "path": "/fixed", "value": number("7")}]

    def materialize(value, copies, round):
        nonlocal current
        assert field(value, "fixed") == number("7")
        initial[0]["value"]["value"] = "99"
        if round == 0:
            current = field(value, "pick")["value"]
            starts.append(current)
            return [
                {
                    "op": "replace",
                    "path": "/pick",
                    "value": {"kind": "string", "value": "same"},
                }
            ]
        return current.encode() if distinct_meanings else b"shared-plan"

    if distinct_meanings:
        with pytest.raises(daml.ResolveError) as caught:
            schemas.resolve_staged(
                daml.parse("!Choice"), Stage(materialize), initial_edits=initial
            )
        assert caught.value.kind == "AmbiguousDefault"
    else:
        result = schemas.resolve_staged(
            daml.parse("!Choice"), Stage(materialize), initial_edits=initial
        )
        assert result.to_python() == {"pick": "same", "fixed": 7}
        assert result.meaning() == b"shared-plan"
    assert set(starts) == {"first", "second"}


def test_selected_meaning_belongs_to_native_valid_winner_not_first_ready_candidate():
    schemas = compose(
        {
            "Choice": {
                "schema": {
                    "anyOf": [
                        {"properties": {"n": {"default": 0, "minimum": 1}}},
                        {"properties": {"n": {"default": 2, "minimum": 1}}},
                    ]
                }
            }
        }
    )
    emitted = set()

    def materialize(value, copies, round):
        raw = field(value, "n")["value"]
        emitted.add(raw)
        return raw.encode()

    result = schemas.resolve_staged(daml.parse("!Choice"), Stage(materialize))
    assert result.to_python() == {"n": 2}
    assert result.meaning() == b"2"
    assert emitted == {"0", "2"}
    exported = result.value()
    exported["entries"].clear()
    del schemas
    assert result.to_python() == {"n": 2}
    assert result.meaning() == b"2"


def test_returned_rejection_discards_candidate_but_raised_error_is_fatal_unchanged():
    schemas = compose(
        {
            "Choice": {
                "schema": {
                    "anyOf": [
                        {
                            "properties": {
                                "pick": {"default": "first", "const": "first"}
                            }
                        },
                        {
                            "properties": {
                                "pick": {"default": "second", "const": "second"}
                            }
                        },
                    ]
                }
            }
        }
    )
    rejection = ValueError("domain rejected candidate")
    fatal = RuntimeError("host failure")

    def reject_first(value, copies, round):
        if field(value, "pick")["value"] == "first":
            return rejection
        return b"second-plan"

    result = schemas.resolve_staged(daml.parse("!Choice"), Stage(reject_first))
    assert result.to_python() == {"pick": "second"}
    assert result.meaning() == b"second-plan"

    def raise_fatal(value, copies, round):
        raise fatal

    with pytest.raises(RuntimeError) as caught:
        schemas.resolve_staged(daml.parse("!Choice"), Stage(raise_fatal))
    assert caught.value is fatal
    with pytest.raises(ValueError) as caught:
        schemas.resolve_staged(
            daml.parse("!Choice"), Stage(lambda value, copies, round: rejection)
        )
    assert caught.value is rejection


@pytest.mark.parametrize(
    "edits",
    [
        [],
        [{"op": "replace", "path": "n", "value": number("2")}],
        [{"op": "replace", "path": "/absent/n", "value": number("2")}],
        [{"op": "replace", "path": "/list/1", "value": number("2")}],
        [{"op": "replace", "path": "/n", "value": number("0")}],
        [
            {"op": "replace", "path": "/n", "value": number("2")},
            {"op": "replace", "path": "/n", "value": number("3")},
        ],
        [
            {"op": "replace", "path": "/parent", "value": mapping()},
            {"op": "replace", "path": "/parent/n", "value": number("2")},
        ],
        [
            {"op": "copy", "path": "/n", "from": "/parent"},
            {"op": "replace", "path": "/parent/n", "value": number("2")},
        ],
    ],
    ids=[
        "empty",
        "pointer",
        "missing-parent",
        "out-of-range",
        "unchanged",
        "duplicate-destination",
        "overlapping-destinations",
        "modified-copy-source",
    ],
)
def test_invalid_or_nonprogressing_edit_batches_fail_without_retry(edits):
    schemas = compose({})
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("{n: 0\nparent: {n: 1}\nlist: [null]}"),
            Stage(lambda value, copies, round: edits),
        )
    assert caught.value.kind == "InvalidStage"


def test_copy_requires_completed_source_and_initial_copy_is_not_allowed():
    schemas = compose(
        {
            "Root": {
                "schema": {"properties": {"source": {"type": "integer"}, "target": {}}}
            }
        }
    )
    document = daml.parse('!Root {source: {ref: "/later"}\ntarget: null}')
    edit = {"op": "copy", "path": "/target", "from": "/source"}
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            document,
            Stage(lambda value, copies, round: [edit], defer_reference),
        )
    assert caught.value.kind == "InvalidStage"
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(document, Stage(), initial_edits=[edit])
    assert caught.value.kind == "InvalidStage"


def test_duplicate_expect_is_not_progress_and_still_combines_with_data_edits():
    schemas = compose({"Count": {"schema": {"type": "integer", "minimum": 1}}})
    expect = {"op": "expect", "path": "/n", "name": "Count"}
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("{n: 1}"),
            Stage(lambda value, copies, round: [expect]),
        )
    assert caught.value.kind == "InvalidStage"
    result = schemas.resolve_staged(
        daml.parse("{n: 0}"),
        Stage(),
        initial_edits=[{"op": "replace", "path": "/n", "value": number("2")}, expect],
    )
    assert result.to_python() == {"n": 2}


@pytest.mark.parametrize("invalid", [True, -1, 2**32, 1.5])
def test_number_class_must_be_a_non_bool_u32(invalid):
    with pytest.raises(daml.SemanticError):
        compose({}).resolve_staged(
            daml.parse("1"), Stage(number_class=lambda raw: invalid)
        )


def test_callback_wire_is_strict_and_synchronous():
    class Awaitable:
        def __await__(self):
            yield

    for invalid in [
        Awaitable(),
        [{"op": "replace", "path": "", "value": number("2"), "unknown": True}],
    ]:
        with pytest.raises(daml.SemanticError):
            compose({}).resolve_staged(
                daml.parse("1"), Stage(lambda value, copies, round: invalid)
            )
    with pytest.raises(daml.SemanticError):
        compose({}).resolve_staged(
            daml.parse("null"), Stage(defer=lambda summary, path: 1)
        )
    empty_key = compose({}).resolve_staged(
        daml.parse("null"), Stage(lambda value, copies, round: b"")
    )
    assert empty_key.meaning() == b""


def test_callback_materialization_bytes_and_meanings_share_cumulative_bounds():
    schemas = compose({})
    calls = 0

    def grow(value, copies, round):
        nonlocal calls
        calls += 1
        assert calls <= 16, "resolution must not reset its byte budget each round"
        return [
            {
                "op": "replace",
                "path": "/text",
                "value": {"kind": "string", "value": str(round) + "x" * 999_000},
            }
        ]

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(daml.parse('{text: ""}'), Stage(grow))
    assert caught.value.kind == "LimitExceeded"

    def oversized_meaning(value, copies, round):
        if field(value, "pick")["value"] == "first":
            return b"x" * (8 * 1024 * 1024 + 1)
        return b"second-plan"

    with pytest.raises((daml.ResolveError, daml.SemanticError)) as caught:
        plan_choices().resolve_staged(daml.parse("!Choice"), Stage(oversized_meaning))
    assert caught.value.kind == "LimitExceeded"


def test_staged_copy_preserves_private_collision_binding_and_exact_numbers():
    bundle = daml.Bundle.from_sources(
        ".",
        {
            "daml.bundle": '{app: "./app"\nprovider: "./decoy"}',
            "app/daml.bundle": '{provider: "./provider"}',
            "app/Host.daml": "!Object { child: (!Object = !provider.Adapter) }",
            "app/provider/daml.bundle": "{}",
            "app/provider/Adapter.daml": (
                "!Object {\nexact: !Int\nreal: !Number\nzero: !Number\n} = {\n"
                "exact: 900719925474099312345678901234567890\n"
                "real: 1.0\nzero: -0.0\n}"
            ),
            "decoy/daml.bundle": "{}",
            "decoy/Adapter.daml": "!Int",
        },
    )
    document = daml.parse(
        "{source: !app.Host\nsentinel: !provider.Adapter 1\n"
        'target: {ref: "/source/child"}}'
    )
    schemas = bundle.compile(document)
    binding_summaries = []

    def defer_private(summary, path):
        if path == "/source/child" and "tag" in summary:
            assert isinstance(summary["tag"]["binding"], str)
            binding_summaries.append(summary["tag"])
        return defer_reference(summary, path)

    def materialize(value, copies, round):
        if round == 0:
            return [{"op": "copy", "path": "/target", "from": "/source/child"}]
        return None

    result = schemas.resolve_staged(document, Stage(materialize, defer_private))
    assert binding_summaries[0]["name"] == "provider.Adapter"
    exported = result.value()
    source = field(field(exported, "source"), "child")
    target = field(exported, "target")
    assert target["tag"] == source["tag"]
    assert target["tag"]["name"] == "provider.Adapter"
    plain = result.to_python()["target"]
    assert plain["exact"] == 900719925474099312345678901234567890
    assert isinstance(plain["real"], Decimal) and plain["real"] == Decimal("1")
    assert plain["zero"].is_zero() and plain["zero"].is_signed()
    assert field(document.value(), "target") == mapping(
        ref={"kind": "string", "value": "/source/child"}
    )
    with pytest.raises(daml.SemanticError) as caught:
        schemas.write(exported)
    assert caught.value.kind == "UnrepresentableTag"


def test_copy_cannot_ignore_incompatible_destination_or_erase_source_obligations():
    schemas = compose(
        {
            "Source": {"schema": {"type": "integer", "minimum": 5}},
            "Text": {"schema": {"type": "string"}},
        }
    )
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse('{source: !Source 7\ntarget: !Text ""}'),
            Stage(
                lambda value, copies, round: (
                    [{"op": "copy", "path": "/target", "from": "/source"}]
                    if round == 0
                    else None
                )
            ),
        )
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/target"

    def erase(value, copies, round):
        if round == 0:
            return [{"op": "copy", "path": "/target", "from": "/source"}]
        if round == 1:
            return [{"op": "replace", "path": "", "value": mapping(source=number("7"))}]
        return None

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("{source: !Source 7\ntarget: 0}"), Stage(erase)
        )
    assert caught.value.kind == "SchemaViolation"


def test_completed_record_copy_can_retain_strictly_valid_runtime_opaque_descendants():
    runtime = {
        "type": "object",
        "properties": {"runtime": {"type": "string"}},
        "required": ["runtime"],
        "additionalProperties": False,
    }
    schemas = compose(
        {
            "Root": {
                "schema": {
                    "properties": {
                        "source": {"type": "object", "properties": {"child": runtime}},
                        "target": {"type": "object"},
                    }
                }
            }
        }
    )

    def defer_runtime(summary, path):
        return summary["kind"] == "map" and any(
            entry["key"] == "runtime" for entry in summary["fields"]
        )

    def materialize(value, copies, round):
        if round == 0:
            return [{"op": "copy", "path": "/target", "from": "/source"}]
        return None

    result = schemas.resolve_staged(
        daml.parse('!Root {source: {child: {runtime: "environment"}}\ntarget: {}}'),
        Stage(materialize, defer_runtime),
    )
    assert result.to_python() == {
        "source": {"child": {"runtime": "environment"}},
        "target": {"child": {"runtime": "environment"}},
    }


def test_recursive_candidate_merges_leave_supplied_opaque_mapping_atomic():
    schemas = compose(
        {
            "Root": {
                "schema": {
                    "properties": {"source": {"default": {"own": 9}}},
                    "allOf": [
                        {"properties": {"target": {"default": {"a": 1}}}},
                        {"properties": {"target": {"default": {"b": 2}}}},
                    ],
                }
            }
        }
    )

    def materialize(value, copies, round):
        if round == 0:
            assert field(value, "target") == mapping(
                ref={"kind": "string", "value": "/source"}
            )
            return [{"op": "copy", "path": "/target", "from": "/source"}]
        return None

    result = schemas.resolve_staged(
        daml.parse('!Root {target: {ref: "/source"}}'),
        Stage(materialize, defer_reference),
    )
    assert result.to_python() == {"source": {"own": 9}, "target": {"own": 9}}


def test_summary_scalar_numbers_stay_exact_and_tagged_field_containers_are_shallow():
    schemas = compose(
        {
            "Root": {"schema": {"type": "object"}},
            "Nested": {"schema": {"type": "object"}},
        }
    )
    summaries = []

    def inspect(summary, path):
        if path == "":
            summaries.append(summary)
        return False

    result = schemas.resolve_staged(
        daml.parse(
            "!Root {big: 900719925474099312345678901\n"
            "nested: !Nested {leaf: true}\nlist: [1\n2]}"
        ),
        Stage(defer=inspect),
    )
    summary = summaries[0]
    assert summary["tag"]["name"] == "Root"
    fields = {entry["key"]: entry["summary"] for entry in summary["fields"]}
    assert fields == {
        "big": number("900719925474099312345678901"),
        "nested": {"kind": "map", "length": 1},
        "list": {"kind": "list", "length": 2},
    }
    assert result.to_python()["big"] == 900719925474099312345678901
    assert field(result.value(), "nested")["tag"]["name"] == "Nested"


def test_initial_edits_cannot_bypass_binding_admission_or_pinned_assertions():
    schemas = compose({"Known": {"schema": {"type": "integer", "maximum": 1}}})
    initial = [{"op": "replace", "path": "", "value": number("2")}]
    foreign = number("1") | {"tag": {"name": "Known", "binding": "urn:foreign:unknown"}}
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic_staged(foreign, Stage(), initial_edits=initial)
    assert caught.value.kind == "UnknownTag"
    assert caught.value.span is None
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(daml.parse("!Known 1"), Stage(), initial_edits=initial)
    assert caught.value.kind == "SchemaViolation"


@pytest.mark.parametrize("replacement", [mapping(n=number("2")), mapping()])
def test_ancestor_replace_cannot_erase_displaced_descendant_assertions(replacement):
    schemas = compose({"Known": {"schema": {"type": "integer", "maximum": 1}}})
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(
            daml.parse("{box: {n: !Known 1}}"),
            Stage(),
            initial_edits=[{"op": "replace", "path": "/box", "value": replacement}],
        )
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/box/n"


def test_new_replacement_assertions_remain_constraints_in_later_rounds():
    schemas = compose({"Known": {"schema": {"type": "integer", "maximum": 1}}})

    def materialize(value, copies, round):
        if round == 0:
            return [
                {
                    "op": "replace",
                    "path": "/n",
                    "value": number("1") | {"tag": {"name": "Known"}},
                }
            ]
        if round == 1:
            return [{"op": "replace", "path": "/n", "value": number("2")}]
        return None

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_staged(daml.parse("{n: 0}"), Stage(materialize))
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/n"
