"""Explicit virtual resources and extra native roots do not widen trust."""

import daml
import pytest


def test_virtual_bundle_and_filesystem_bundle_admit_explicit_target_contracts(tmp_path):
    sources = {
        "daml.bundle": "{}",
        "Config.daml": "!Object { name: !String }",
        "Target.daml": "closed !Object { count: !Int { min: 1 } } = { count: 4 }",
        "Unused.daml": "!missing.Invalid",
    }
    virtual = daml.Bundle.from_sources(".", sources)
    for name, source in sources.items():
        (tmp_path / name).write_text(source)
    disk = daml.Bundle.open(tmp_path)
    sources["Target.daml"] = "!Never"
    document = daml.parse('!self.Config { name: "worker" }')
    empty = daml.parse("{}")
    for bundle in [virtual, disk]:
        schemas = bundle.compile(document, additional_types=iter(["self.Target"]))
        assert schemas.resolve(document).to_python() == {"name": "worker"}
        assert schemas.resolve(empty).to_python() == {}
        assert schemas.resolve(empty, expected="self.Target").to_python() == {
            "count": 4
        }
        with pytest.raises(daml.ResolveError) as caught:
            schemas.resolve(daml.parse("{ count: 0 }"), expected="self.Target")
        assert caught.value.kind == "SchemaViolation"
        with pytest.raises(daml.ResolveError) as caught:
            schemas.resolve(daml.parse("{ extra: true }"), expected="self.Target")
        assert caught.value.kind == "SchemaViolation"


def test_unknown_explicit_target_name_has_no_fabricated_source_span():
    bundle = daml.Bundle.from_sources(".", {"daml.bundle": "{}"})
    with pytest.raises(daml.NativeError) as caught:
        bundle.compile(daml.parse("{}"), additional_types=["self.Absent"])
    assert caught.value.kind == "MissingDefinition"
    assert caught.value.reference == "self.Absent"
    assert caught.value.span is None


def test_virtual_bundle_cannot_fall_back_to_filesystem_resources(tmp_path, monkeypatch):
    (tmp_path / "daml.bundle").write_text("{}")
    (tmp_path / "Hidden.daml").write_text("!String")
    monkeypatch.chdir(tmp_path)
    bundle = daml.Bundle.from_sources(".", {"daml.bundle": "{}"})
    with pytest.raises(daml.NativeError) as caught:
        bundle.compile(daml.parse('!self.Hidden "hidden"'))
    assert caught.value.kind == "MissingDefinition"
    assert caught.value.reference == "self.Hidden"


def test_virtual_resources_are_strict_bounded_utf8_text():
    for sources in [
        {"daml.bundle": "{}", "Unused.daml": "\ud800"},
        {"daml.bundle": "{}", "Unused.daml": "x" * (8 * 1024 * 1024 + 1)},
    ]:
        with pytest.raises(daml.SemanticError):
            daml.Bundle.from_sources(".", sources)
    bundle = daml.Bundle.from_sources(".", {"daml.bundle": "{}"})
    with pytest.raises((daml.NativeError, daml.SemanticError)) as caught:
        bundle.compile(daml.parse("{}"), additional_types=["X" * 1_048_577])
    assert caught.value.kind == "LimitExceeded"
