"""Public consumer contracts for Spring cron values; no Java runtime required."""

import json
import subprocess
import sys
from pathlib import Path

import daml
import pytest


def test_invalid_cron_literal_is_rejected_at_source_admission():
    # This assertion also runs on the pre-Cron API: an unchecked tag is not valid.
    with pytest.raises(daml.ParseError):
        daml.parse('!Cron "not a cron expression"')


@pytest.mark.parametrize(
    "expression",
    [
        "60 * * * * *",
        "0 0 24 * * *",
        "0 0 0 * * 8",
        "* * * * *",
        "0 0 0 * * * 2027",
        "0 0 22-2 * * *",
        "@reboot",
        "0 0 0 ? * MON#0",
    ],
)
def test_invalid_cron_constructor_and_literal(expression):
    with pytest.raises(ValueError):
        daml.Cron(expression)
    with pytest.raises(daml.ParseError):
        daml.parse("!Cron " + json.dumps(expression))


def test_cron_python_value_is_frozen_and_not_a_datetime_adapter():
    value = daml.Cron(" @midnight ")
    assert value.expression == "0 0 0 * * *"
    assert str(value) == value.expression
    assert "Cron" in repr(value) and value.expression in repr(value)
    assert value == daml.Cron("0 0 0 * * *")
    assert value != daml.Cron("0 0 0 * * 0")
    assert value != value.expression
    assert not hasattr(value, "isoformat")
    with pytest.raises((AttributeError, TypeError)):
        value.expression = "@hourly"
    parsed = daml.parse('!Cron "@midnight"')
    assert parsed.to_python() == value
    assert parsed.value() == {"kind": "cron", "value": value.expression}
    assert daml.parse(daml.write(parsed.value())).value() == parsed.value()


def test_cron_default_resolved_semantic_and_json_boundaries():
    module = daml.SchemaModule.from_json(
        json.dumps(
            {
                "types": {
                    "app.Schedule": {
                        "schema": {
                            "type": "object",
                            "properties": {"when": {"x-daml-scalar": {"kind": "cron"}}},
                            "default": {"when": "@weekly"},
                            "x-daml-default-temporals": [
                                {"path": "/when", "kind": "cron"}
                            ],
                        }
                    }
                }
            }
        )
    )
    schemas = daml.Schemas.compose([module])
    result = schemas.resolve(daml.parse("!app.Schedule"), expected="app.Schedule")
    assert result.to_python()["when"] == daml.Cron("0 0 0 * * 0")
    # Completed snapshots carry schema bindings; their writer needs the registry.
    written = schemas.write(result.value())
    assert daml.parse(written).to_python()["when"] == daml.Cron("@weekly")
    with pytest.raises(daml.ResolveError):
        schemas.resolve(
            daml.parse('!app.Schedule { when: "0 0 0 * * 0" }'),
            expected="app.Schedule",
        )


def test_cron_base_import_has_no_optional_dependency():
    package = Path(daml.__file__).resolve().parent
    result = subprocess.run(
        [
            sys.executable,
            "-S",
            "-c",
            """import daml
assert daml.parse('!Cron "@hourly"').to_python() == daml.Cron('0 0 * * * *')
""",
        ],
        cwd=package.parent,
        env={"PYTHONPATH": str(package.parent)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr


def test_native_cron_and_staged_public_snapshots_keep_kind():
    bundle = daml.Bundle.from_sources(
        ".",
        {
            "daml.bundle": "{}",
            "Schedule.daml": '!Object { when: !Cron } = { when: !Cron "@weekly" }',
        },
    )
    document = daml.parse("!self.Schedule")
    schemas = bundle.compile(document)
    expected = {"kind": "cron", "value": "0 0 0 * * 0"}

    class Copy:
        saw_summary = False

        def defer(self, summary, path):
            if path == "/when":
                assert summary == expected
                self.saw_summary = True
            return False

        def materialize(self, value, copies, round):
            when = next(
                entry["value"] for entry in value["entries"] if entry["key"] == "when"
            )
            assert when == expected
            if round == 0:
                return [{"op": "copy", "path": "/copied", "from": "/when"}]
            return None

    stage = Copy()
    result = schemas.resolve_staged(document, stage)
    assert stage.saw_summary
    assert result.to_python()["copied"] == daml.Cron("@weekly")
    snapshot = result.value()
    del schemas, bundle, document
    assert result.to_python()["when"] == daml.Cron("@weekly")
    assert (
        next(
            entry["value"] for entry in snapshot["entries"] if entry["key"] == "copied"
        )
        == expected
    )


def test_cron_semantic_input_must_be_admitted_before_write():
    with pytest.raises(daml.SemanticError):
        daml.write({"kind": "cron", "value": "0 0 24 * * *"})
