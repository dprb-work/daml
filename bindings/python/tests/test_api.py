"""DAML Python boundary: exact values, assertions, ownership, and diagnostics."""

from decimal import Decimal
from pathlib import Path

import daml
import pytest

NATIVE = Path(__file__).resolve().parents[3] / "examples" / "native"


def test_parse_preserves_exact_numbers_and_unknown_tags():
    document = daml.parse(
        "{\nbig: 123456789012345678901234567890\n"
        "fraction: 1.2300e-40\n"
        "unknown: !third.party.Future {\nvalue: 2\n}\n}"
    )

    assert document.to_python() == {
        "big": 123456789012345678901234567890,
        "fraction": Decimal("1.2300e-40"),
        "unknown": {"value": 2},
    }
    assert document.tags() == {"/unknown": "third.party.Future"}


def test_parse_error_span_counts_utf8_bytes_after_unicode():
    source = '"東京" !'
    with pytest.raises(daml.ParseError) as caught:
        daml.parse(source)

    error = caught.value
    assert error.kind == "TrailingInput"
    assert error.span == (9, 10)
    assert error.path is None


def test_native_bundle_completes_nested_defaults_and_exposes_tags():
    bundle = daml.Bundle.open(NATIVE)
    document = daml.parse((NATIVE / "data" / "omitted.daml").read_text())
    resolved = bundle.compile(document).resolve(document)

    assert resolved.to_python() == {
        "name": "demo",
        "mode": "read",
        "adapter": {"addr": "localhost", "timeout": 45, "compression": True},
        "workers": [{"name": "primary", "retries": 3}],
        "labels": {},
        "range": [1, 10],
    }
    assert resolved.tags()[""] == "app.Config"
    assert resolved.tags()["/adapter"] == "provider.Http"


def test_supplied_native_tags_survive_as_rfc6901_paths():
    document = daml.parse((NATIVE / "data" / "supplied.daml").read_text())
    resolved = daml.Bundle.open(NATIVE).compile(document).resolve(document)

    assert resolved.tags()[""] == "app.Config"
    assert resolved.tags()["/adapter"] == "http.Http"
    assert resolved.to_python()["labels"] == {
        "app.version": "v2",
        "/health": "ok",
        "200": "ok",
        "東京": "Tokyo",
    }


def test_resolved_snapshot_survives_inputs_registry_and_export_mutation():
    bundle = daml.Bundle.open(NATIVE)
    document = daml.parse('!app.Config {\nname: "snapshot"\nrange: [\n1\n10\n]\n}')
    schemas = bundle.compile(document)
    resolved = schemas.resolve(document)
    expected_tags = resolved.tags()

    first = resolved.to_python()
    first["adapter"]["addr"] = "mutated"
    first["workers"].clear()
    del schemas, bundle, document

    assert resolved.to_python()["adapter"]["addr"] == "localhost"
    assert resolved.to_python()["workers"] == [{"name": "primary", "retries": 3}]
    assert resolved.tags() == expected_tags


def test_expected_root_is_applied_in_both_compile_and_resolve_phases(tmp_path):
    (tmp_path / "daml.bundle").write_text("{}")
    (tmp_path / "Config.daml").write_text(
        "!Object {\nname: !String\ncount: !Int\n} = { count: 4 }"
    )
    document = daml.parse('{\nname: "worker"\n}')
    bundle = daml.Bundle.open(tmp_path)
    schemas = bundle.compile(document, expected="self.Config")

    assert schemas.resolve(document).to_python() == {"name": "worker"}
    assert schemas.resolve(document, expected="self.Config").to_python() == {
        "name": "worker",
        "count": 4,
    }


def test_resolve_error_retains_public_category_pointer_and_source_location(tmp_path):
    (tmp_path / "daml.bundle").write_text("{}")
    (tmp_path / "Config.daml").write_text(
        "!Object {\nname: !String\nworkers: !List {\n"
        "items: !Object {\nname: !String\n}\n}\n}"
    )
    source = '{\nname: "x"\nworkers: [\n{}\n]\n}'
    document = daml.parse(source)
    schemas = daml.Bundle.open(tmp_path).compile(document, expected="self.Config")

    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve(document, expected="self.Config")
    error = caught.value
    assert error.kind == "SchemaViolation"
    assert error.path == "/workers/0"
    assert error.span == (source.index("{}"), source.index("{}") + 2)
    assert isinstance(error.schema, tuple) and len(error.schema) == 2
    assert error.origin is None


def test_unknown_tag_is_parseable_but_rejected_when_compiled():
    document = daml.parse("{\npath/key: !vendor.Unknown {\nvalue: 1\n}\n}")
    assert document.tags() == {"/path~1key": "vendor.Unknown"}

    with pytest.raises(daml.NativeError) as caught:
        daml.Bundle.open(NATIVE).compile(document)
    error = caught.value
    assert error.kind == "UnknownReference"
    assert error.reference == "vendor.Unknown"
    assert error.span == (13, 13 + len("vendor.Unknown"))
    assert error.path is None


def test_low_level_modules_compose_and_resolve_without_native_bundle():
    base = daml.SchemaModule.from_json(
        '{"types":{"app.Item":{"schema":{"$id":"urn:item",'
        '"type":"object","properties":{"value":{"type":"integer"}},'
        '"required":["value"],"additionalProperties":false}}}}'
    )
    root = daml.SchemaModule.from_json(
        '{"types":{"app.Root":{"schema":{"type":"object",'
        '"properties":{"item":{"$ref":"urn:item"}},'
        '"required":["item"],"additionalProperties":false}}}}'
    )
    schemas = daml.Schemas.compose([base, root])
    document = daml.parse("{\nitem: {\nvalue: 7\n}\n}")

    assert schemas.resolve(document, expected="app.Root").to_python() == {
        "item": {"value": 7}
    }


def test_low_level_compose_accepts_one_shot_module_iterators():
    module = daml.SchemaModule.from_json(
        '{"types":{"app.Positive":{"schema":{"type":"integer","minimum":1}}}}'
    )
    schemas = daml.Schemas.compose(iter([module]))

    assert schemas.resolve(daml.parse("7"), expected="app.Positive").to_python() == 7
    with pytest.raises(daml.ResolveError) as caught:
        schemas.resolve(daml.parse("0"), expected="app.Positive")
    assert caught.value.kind == "SchemaViolation"


def test_document_formatter_retains_exact_values_and_is_idempotent():
    source = '{\nvalue: 12345678901234567890\nlabel: "a much longer string"\n}'
    document = daml.parse(source)
    formatted = document.format(line_width=24)
    assert daml.parse(formatted).to_python() == document.to_python()
    assert daml.parse(formatted).format(line_width=24) == formatted
