"""Normal callback editing and bounded expected-type admission regressions."""

import json

import daml
import pytest


def compose(types):
    return daml.Schemas.compose(
        [daml.SchemaModule.from_json(json.dumps({"types": types}))]
    )


def field(value, key):
    return next(entry["value"] for entry in value["entries"] if entry["key"] == key)


class Ready:
    def defer(self, summary, path):
        return False

    def materialize(self, value, copies, round):
        return None


def test_callback_alias_binding_is_admissible_when_its_snapshot_is_edited():
    schemas = compose(
        {
            "Base": {"schema": {"$id": "urn:review:text", "type": "string"}},
            "AliasA": {"schema": {"$ref": "urn:review:text"}},
            "Display": {"schema": {"$ref": "urn:review:text"}},
            "Root": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "value": {
                            "default": "ok",
                            "x-daml-default-tags": [
                                {"path": "", "tag": "Display", "binding": "AliasA"}
                            ],
                        }
                    },
                }
            },
        }
    )
    document = daml.parse("!Root")
    checked_value = field(schemas.resolve(document).value(), "value")
    binding = checked_value["tag"]["binding"]

    class EditSnapshot(Ready):
        def materialize(self, value, copies, round):
            inherited = field(value, "value")
            assert inherited["tag"]["name"] == "Display"
            assert inherited["tag"]["binding"] == binding
            if round == 0:
                value["entries"].append(
                    {
                        "key": "extra",
                        "value": {"kind": "bool", "value": True},
                    }
                )
                return [{"op": "replace", "path": "", "value": value}]
            return None

    resolved = schemas.resolve_staged(document, EditSnapshot())
    assert resolved.to_python() == {"value": "ok", "extra": True}
    assert field(resolved.value(), "value")["tag"] == checked_value["tag"]


def failure_identity(action):
    # Keep an incorrectly unbounded expected-name diagnostic out of test output.
    try:
        action()
    except (daml.SemanticError, daml.ResolveError) as error:
        return type(error), error.kind
    except ValueError as error:
        return type(error), None
    pytest.fail("expected bounded semantic input failure")


@pytest.mark.parametrize(
    "method", ["resolve_semantic", "resolve_staged", "resolve_semantic_staged"]
)
def test_new_expected_type_parameters_use_the_existing_scalar_bound(method):
    schemas = compose({})
    expected = "X" * 1_048_577
    semantic = {"kind": "null"}

    def action():
        if method == "resolve_semantic":
            return schemas.resolve_semantic(semantic, expected=expected)
        if method == "resolve_staged":
            return schemas.resolve_staged(
                daml.parse("null"), Ready(), expected=expected
            )
        return schemas.resolve_semantic_staged(semantic, Ready(), expected=expected)

    assert failure_identity(action) == (daml.SemanticError, "LimitExceeded")


def test_expected_type_invalid_unicode_uses_the_existing_text_error_contract():
    schemas = compose({})
    assert failure_identity(
        lambda: schemas.resolve_semantic({"kind": "null"}, expected="\ud800")
    ) == (daml.SemanticError, "InvalidInput")


def test_semantic_payload_and_expected_name_share_one_cumulative_budget():
    schemas = compose({})
    payload = {
        "kind": "list",
        "items": [{"kind": "string", "value": "x" * 1_000_000}] * 8,
    }
    # Each scalar and the payload alone fit; their total with expected exceeds
    # eight MiB. A separate expected-name budget would wrongly reach lookup.
    assert failure_identity(
        lambda: schemas.resolve_semantic(payload, expected="X" * 500_000)
    ) == (daml.SemanticError, "LimitExceeded")
