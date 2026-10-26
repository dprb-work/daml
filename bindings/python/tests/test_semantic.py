"""Editable semantic values cross the native boundary without losing identity."""

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


def private_registry(tmp_path):
    sources = {
        "daml.bundle": '{app: "./app"\nprovider: "./decoy"}',
        "app/daml.bundle": '{provider: "./provider"}',
        "app/Host.daml": "!Object { child: (!Object = !provider.Adapter) }",
        "app/provider/daml.bundle": "{}",
        "app/provider/Adapter.daml": (
            '!Object { value: !String { min: 1 } } = { value: "private" }'
        ),
        "decoy/daml.bundle": "{}",
        "decoy/Adapter.daml": "!Int",
    }
    for name, source in sources.items():
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source)
    document = daml.parse("{host: !app.Host\ndecoy: !provider.Adapter 7}")
    schemas = daml.Bundle.open(tmp_path).compile(document)
    return schemas, document


def assert_exact_numbers(value, tokens):
    assert value["kind"] == "list"
    for item, expected in zip(value["items"], tokens, strict=True):
        assert item["kind"] == "number"
        raw = item["value"]
        assert isinstance(raw, str)
        actual_number = Decimal(raw)
        expected_number = Decimal(expected)
        assert actual_number == expected_number
        assert any(char in raw for char in ".eE") == any(
            char in expected for char in ".eE"
        )
        if expected_number.is_zero():
            assert actual_number.is_signed() == expected_number.is_signed()


def test_document_value_and_writing_preserve_exact_numbers_and_real_kind():
    tokens = [
        "900719925474099312345678901234567890",
        "1234567890.01234567890123456789",
        "-0.0",
        "1.2300e-40",
        "1.0",
    ]
    document = daml.parse("[\n" + "\n".join(tokens) + "\n]")
    expected = {"kind": "list", "items": [number(token) for token in tokens]}
    assert_exact_numbers(document.value(), tokens)
    exported = document.value()
    exported["items"][0]["value"] = "2"
    assert_exact_numbers(document.value(), tokens)
    del document

    schemas = compose({})
    resolved = schemas.resolve_semantic(expected)
    assert isinstance(resolved, daml.ResolvedDocument)
    assert_exact_numbers(resolved.value(), tokens)
    plain = resolved.to_python()
    assert plain == [
        int(tokens[0]),
        Decimal(tokens[1]),
        Decimal("-0.0"),
        Decimal(tokens[3]),
        Decimal("1.0"),
    ]
    assert isinstance(plain[0], int)
    assert all(isinstance(value, Decimal) for value in plain[1:])
    assert plain[2].is_zero() and plain[2].is_signed()
    assert_exact_numbers(
        daml.parse(daml.write(expected, line_width=24)).value(), tokens
    )
    assert_exact_numbers(
        daml.parse(schemas.write(expected, line_width=24)).value(), tokens
    )


def test_private_default_binding_readmits_but_cannot_be_written_as_source(tmp_path):
    schemas, document = private_registry(tmp_path)
    original = schemas.resolve(document)
    exported = original.value()
    child = field(field(exported, "host"), "child")
    private_tag = child["tag"].copy()
    assert private_tag["name"] == "provider.Adapter"
    assert isinstance(private_tag["binding"], str)
    assert child["kind"] == "map"
    assert field(child, "value") == {"kind": "string", "value": "private"}

    with pytest.raises(daml.SemanticError) as caught:
        schemas.write(exported)
    assert caught.value.kind == "UnrepresentableTag"
    assert caught.value.pointer == "/host/child"
    assert isinstance(caught.value, ValueError)
    with pytest.raises(daml.SemanticError) as caught:
        daml.write(exported)
    assert caught.value.kind == "UnrepresentableTag"

    admitted = schemas.resolve_semantic(exported)
    assert field(field(admitted.value(), "host"), "child")["tag"] == private_tag
    assert admitted.to_python() == {
        "host": {"child": {"value": "private"}},
        "decoy": 7,
    }
    field(child, "value")["value"] = ""
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic(exported)
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/host/child/value"
    assert caught.value.span is None

    # This literal name has a valid integer meaning at the entry boundary.
    # An unknown supplied binding must not fall back to that meaning.
    forged = number("7") | {
        "tag": {"name": "provider.Adapter", "binding": "urn:unknown:binding"}
    }
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic(forged)
    assert caught.value.kind == "UnknownTag"
    assert caught.value.path == ""
    assert caught.value.span is None

    first = admitted.value()
    field(field(first, "host"), "child")["tag"]["binding"] = "mutated"
    field(field(field(first, "host"), "child"), "value")["value"] = "mutated"
    first["entries"].clear()
    del schemas, document, original, exported
    fresh = admitted.value()
    assert field(field(fresh, "host"), "child")["tag"] == private_tag
    assert field(field(field(fresh, "host"), "child"), "value")["value"] == "private"
    assert admitted.tags()["/host/child"] == "provider.Adapter"
    assert admitted.to_python()["decoy"] == 7


def test_semantic_expected_root_and_explicit_assertion_both_validate():
    schemas = compose(
        {
            "Minimum": {"schema": {"type": "integer", "minimum": 10}},
            "Maximum": {"schema": {"type": "integer", "maximum": 15}},
        }
    )
    valid = number("12") | {"tag": {"name": "Maximum"}}
    assert schemas.resolve_semantic(valid, expected="Minimum").to_python() == 12
    for raw in ["3", "20"]:
        edited = number(raw) | {"tag": {"name": "Maximum"}}
        with pytest.raises(daml.ResolveError) as caught:
            schemas.resolve_semantic(edited, expected="Minimum")
        assert caught.value.kind == "SchemaViolation"
        assert caught.value.path == ""
        assert caught.value.span is None


def test_semantic_completion_is_owned_and_edits_are_checked_again():
    schemas = compose(
        {
            "Config": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "count": {"type": "integer", "minimum": 1, "default": 7}
                    },
                    "required": ["count"],
                }
            }
        }
    )
    source = mapping()
    resolved = schemas.resolve_semantic(source, expected="Config")
    assert source == mapping()
    assert resolved.value() == mapping(count=number("7"))
    edited = resolved.value()
    field(edited, "count")["value"] = "0"
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic(edited, expected="Config")
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/count"
    assert caught.value.span is None
    source["entries"].append({"key": "other", "value": {"kind": "null"}})
    del schemas, source
    assert resolved.value() == mapping(count=number("7"))


@pytest.mark.parametrize(
    "malformed",
    [
        {"value": "missing-kind"},
        {"kind": "future"},
        {"kind": "null", "value": None},
        {"kind": "string", "value": "x", "unknown": True},
        {"kind": "bool", "value": 1},
        {"kind": "number", "value": True},
        {"kind": "number", "value": 1.5},
        {"kind": "list", "items": {}},
        {"kind": "map", "entries": {"key": "value"}},
        {"kind": "null", "tag": {"name": "T", "unknown": True}},
    ],
    ids=[
        "missing-kind",
        "unknown-kind",
        "null-payload",
        "unknown-field",
        "integer-as-bool",
        "bool-as-number",
        "float-as-number",
        "non-list-items",
        "non-list-entries",
        "unknown-tag-field",
    ],
)
def test_malformed_wire_is_a_public_semantic_error(malformed):
    schemas = compose({})
    with pytest.raises(daml.SemanticError) as caught:
        schemas.resolve_semantic(malformed)
    assert isinstance(caught.value, ValueError)
    assert caught.value.pointer == ""


@pytest.mark.parametrize("raw", ["01", "+1", "NaN", "1 2"])
def test_invalid_number_tokens_are_not_coerced(raw):
    with pytest.raises(daml.SemanticError) as caught:
        daml.write(number(raw))
    assert caught.value.pointer == ""


def test_duplicate_map_entries_are_rejected_not_overwritten():
    duplicate = {
        "kind": "map",
        "entries": [
            {"key": "same", "value": number("1")},
            {"key": "same", "value": number("2")},
        ],
    }
    with pytest.raises(daml.SemanticError) as caught:
        compose({}).resolve_semantic(duplicate)
    assert caught.value.pointer == "/same"


def test_cycles_fail_but_shared_acyclic_values_are_admitted():
    cyclic = {"kind": "list", "items": []}
    cyclic["items"].append(cyclic)
    with pytest.raises(daml.SemanticError) as caught:
        compose({}).resolve_semantic(cyclic)
    assert caught.value.pointer == "/0"
    with pytest.raises(daml.SemanticError):
        daml.write(cyclic)

    shared = number("7")
    repeated = {"kind": "list", "items": [shared, shared]}
    assert compose({}).resolve_semantic(repeated).to_python() == [7, 7]


def test_core_writer_errors_preserve_kind_and_escaped_data_pointer():
    invalid_tag = {"kind": "null", "tag": {"name": "not/a/tag"}}
    invalid = {
        "kind": "map",
        "entries": [{"key": "a/b~c", "value": {"kind": "null"}}],
    }
    with pytest.raises(daml.SemanticError) as caught:
        daml.write(invalid)
    assert caught.value.kind == "InvalidKey"
    assert caught.value.pointer == "/a~1b~0c"
    with pytest.raises(daml.SemanticError) as caught:
        daml.write(mapping(valid=invalid_tag))
    assert caught.value.kind == "InvalidTag"
    assert caught.value.pointer == "/valid"
    with pytest.raises(daml.SemanticError) as caught:
        daml.write({"kind": "null"}, line_width=0)
    assert caught.value.kind == "InvalidLineWidth"
    assert caught.value.pointer == ""


def test_invalid_unicode_is_rejected_before_native_admission():
    with pytest.raises(daml.SemanticError) as caught:
        compose({}).resolve_semantic({"kind": "string", "value": "\ud800"})
    assert caught.value.pointer == ""


def test_python_semantic_resource_bounds_are_finite():
    schemas = compose({})
    at_depth = {"kind": "list", "items": []}
    for _ in range(127):
        at_depth = {"kind": "list", "items": [at_depth]}
    assert schemas.resolve_semantic(at_depth).value() == at_depth
    with pytest.raises(daml.SemanticError) as caught:
        schemas.resolve_semantic({"kind": "list", "items": [at_depth]})
    assert caught.value.kind == "LimitExceeded"

    # Shared Python identities still count as repeated semantic value nodes.
    oversized = {"kind": "list", "items": [{"kind": "null"}] * 100_000}
    with pytest.raises(daml.SemanticError) as caught:
        schemas.resolve_semantic(oversized)
    assert caught.value.kind == "LimitExceeded"
    with pytest.raises(daml.SemanticError) as caught:
        schemas.resolve_semantic({"kind": "string", "value": "x" * 1_048_577})
    assert caught.value.kind == "LimitExceeded"
    byte_heavy = {
        "kind": "list",
        "items": [{"kind": "string", "value": "x" * 1_048_000}] * 9,
    }
    with pytest.raises(daml.SemanticError) as caught:
        schemas.resolve_semantic(byte_heavy)
    assert caught.value.kind == "LimitExceeded"


def test_resolution_bounds_canonical_binding_metadata_before_snapshot_export():
    schemas = compose(
        {"X": {"schema": {"$id": "urn:bound:" + "x" * 262_144, "type": "integer"}}}
    )
    small = daml.parse("[\n" + "!X 1\n" * 2 + "]")
    resolved = schemas.resolve(small)
    exported = resolved.value()
    assert resolved.to_python() == [1, 1]
    assert schemas.resolve_semantic(exported).value() == exported

    # The tiny source fits, but repeated canonical binding metadata exceeds
    # the normal resolution byte budget before an owned Python snapshot exists.
    oversized = daml.parse("[\n" + "!X 1\n" * 40 + "]")
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve(oversized)
    assert caught.value.kind == "LimitExceeded"
    assert caught.value.span is not None

    unbound = {
        "kind": "list",
        "items": [number("1") | {"tag": {"name": "X"}} for _ in range(40)],
    }
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic(unbound)
    assert caught.value.kind == "LimitExceeded"
    assert caught.value.span is None
