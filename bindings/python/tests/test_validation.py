"""Constraint validation never performs completion or source-key admission."""

import copy
import json

import daml
import pytest


@pytest.fixture
def schemas():
    return daml.Schemas.compose(
        [
            daml.SchemaModule.from_json(
                json.dumps(
                    {
                        "types": {
                            "app.Config": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "config": {"type": "object"},
                                        "items": {"type": "array"},
                                        "mode": {"type": "string", "default": "safe"},
                                    },
                                    "required": ["config", "items", "mode"],
                                    "additionalProperties": False,
                                }
                            },
                            "app.Payload": {
                                "schema": {
                                    "type": "object",
                                    "properties": {"payload": {}},
                                    "required": ["payload"],
                                }
                            },
                            "app.Positive": {
                                "schema": {"type": "integer", "minimum": 0}
                            },
                            "app.Literal": {
                                "schema": {
                                    "type": "object",
                                    "properties": {"$ref": {"type": "string"}},
                                    "required": ["$ref"],
                                    "additionalProperties": False,
                                }
                            },
                        }
                    }
                )
            )
        ]
    )


def test_validation_rejects_missing_required_containers_without_completion(schemas):
    supplied = daml.parse("!app.Config {}").value()
    original = copy.deepcopy(supplied)

    with pytest.raises(daml.ResolveError) as caught:
        schemas.validate_semantic(supplied, expected="app.Config")
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == ""
    assert supplied == original

    completed = schemas.resolve_semantic(supplied, expected="app.Config")
    assert completed.to_python() == {"config": {}, "items": [], "mode": "safe"}
    schemas.validate_semantic(completed.value(), expected="app.Config")
    assert supplied == original


def test_validation_checks_nested_assertions_and_expected_type(schemas):
    invalid = daml.parse("{payload: !app.Positive -1}").value()
    with pytest.raises(daml.ResolveError) as caught:
        schemas.validate_semantic(invalid, expected="app.Payload")
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/payload"

    valid = daml.parse("{payload: !app.Positive 2}").value()
    schemas.validate_semantic(valid, expected="app.Payload")
    with pytest.raises(daml.ResolveError) as caught:
        schemas.validate_semantic(valid, expected="missing.Type")
    assert caught.value.kind == "UnknownExpectedType"

    unknown = daml.parse("{payload: !missing.Type 2}").value()
    with pytest.raises(daml.ResolveError) as caught:
        schemas.validate_semantic(unknown, expected="app.Payload")
    assert caught.value.kind == "UnknownTag"
    assert caught.value.path == "/payload"


def test_validation_preserves_literal_json_keys_without_relaxing_source_rules(schemas):
    supplied = {
        "kind": "map",
        "entries": [
            {
                "key": "$ref",
                "value": {
                    "kind": "string",
                    "value": "literal data, not a schema reference",
                },
            }
        ],
    }
    original = copy.deepcopy(supplied)
    schemas.validate_semantic(supplied, expected="app.Literal")
    assert supplied == original

    invalid = {
        "kind": "map",
        "entries": [{"key": "$ref", "value": {"kind": "number", "value": "12"}}],
    }
    with pytest.raises(daml.ResolveError) as caught:
        schemas.validate_semantic(invalid, expected="app.Literal")
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/$ref"

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic(supplied, expected="app.Literal")
    assert caught.value.kind == "InvalidKey"
    assert supplied == original


def test_validation_does_not_charge_discarded_canonical_bindings():
    schemas = daml.Schemas.compose(
        [
            daml.SchemaModule.from_json(
                json.dumps(
                    {
                        "types": {
                            "X": {
                                "schema": {
                                    "$id": "urn:bound:" + "x" * 262_144,
                                    "type": "integer",
                                }
                            }
                        }
                    }
                )
            )
        ]
    )
    supplied = {
        "kind": "list",
        "items": [
            {"kind": "number", "value": "1", "tag": {"name": "X"}} for _ in range(40)
        ],
    }
    original = copy.deepcopy(supplied)
    schemas.validate_semantic(supplied)
    assert supplied == original

    invalid = copy.deepcopy(supplied)
    invalid["items"][-1]["value"] = "1.5"
    with pytest.raises(daml.ResolveError) as caught:
        schemas.validate_semantic(invalid)
    assert caught.value.kind == "SchemaViolation"
    assert caught.value.path == "/39"

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve_semantic(supplied)
    assert caught.value.kind == "LimitExceeded"
