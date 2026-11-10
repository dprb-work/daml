"""One-shot source editing through the real public Python binding."""

from copy import deepcopy
from decimal import Decimal
from typing import Annotated

import daml
import daml.daml as native
import pytest
from pydantic import BaseModel, Field


def number(raw):
    return {"kind": "number", "value": raw}


def string(text):
    return {"kind": "string", "value": text}


def field(value, key):
    return next(entry["value"] for entry in value["entries"] if entry["key"] == key)


@pytest.fixture(params=["document", "schemas"])
def edit(request):
    if request.param == "document":
        return daml.Document.edit
    return daml.Schemas.compose([]).edit


@pytest.fixture
def worker_schemas():
    class Worker(BaseModel):
        name: str = Field(min_length=1)
        retries: int = Field(default=3, ge=0)

    return daml.Schemas.compose([daml.schema_from_types({"app.Worker": Worker})])


def test_document_edit_preserves_outside_bytes_and_original():
    selected = '!app.Old {\r\n  # internal disappears\r\n  name: "古い"\r\n }'
    source = (
        "\ufeff# café\r\n{\r\n # before selected\n selected: "
        + selected
        + " # after selected\r\n keep: 1.2300e+02\r}\n# after root"
    )
    document = daml.parse(source)
    original_value = document.value()
    original_tags = document.tags()
    edits = [{"op": "replace", "path": "/selected", "value": string("新しい🌍")}]
    original_edits = deepcopy(edits)

    edited = document.edit(edits)

    assert isinstance(edited, daml.Document)
    assert edited is not document
    assert edited.source == source.replace(selected, '"新しい🌍"', 1)
    assert document.source == source
    assert document.value() == original_value
    assert document.tags() == original_tags
    assert edits == original_edits
    # Exports and operation records are not retained as mutable document state.
    original_value["entries"].clear()
    edits[0]["value"]["value"] = "mutated"
    assert edited.to_python() == {"selected": "新しい🌍", "keep": Decimal("123.00")}
    assert document.tags() == {"/selected": "app.Old"}


def test_batch_array_indices_refer_to_original_snapshot(edit):
    source = '{items:[{name:"zero"}\n{name:"one"}\n{name:"two"}]\nkeep:4}'
    document = daml.parse(source)
    appended = {
        "kind": "map",
        "entries": [{"key": "name", "value": string("appended")}],
    }
    edits = (
        {"op": "remove", "path": "/items/0"},
        {"op": "replace", "path": "/items/1/name", "value": string("changed one")},
        {"op": "insert", "path": "/items/3", "value": appended},
    )
    edited = edit(document, edits)
    assert edited.to_python() == {
        "items": [{"name": "changed one"}, {"name": "two"}, {"name": "appended"}],
        "keep": 4,
    }
    assert edit(document, tuple(reversed(edits))).source == edited.source
    assert document.source == source


def test_remove_keeps_adjacent_comments_but_removes_owned_subtree(edit):
    removed = "gone: # between key and value\n !app.Old {\n  # internal\n  x:1\n }"
    source = "{\n # before\n " + removed + " # after\n keep:2.00e0\n}"
    document = daml.parse(source)
    edited = edit(document, [{"op": "remove", "path": "/gone"}])
    assert edited.source == source.replace(removed, "", 1)
    assert edited.to_python() == {"keep": Decimal("2.00")}
    assert document.source == source


def test_insert_into_empty_containers_and_before_original_array_element(edit):
    document = daml.parse("{empty:{}\nitems:[1\n2]}")
    edited = edit(
        document,
        [
            {
                "op": "insert",
                "path": "/empty/enabled",
                "value": {"kind": "bool", "value": True},
            },
            {"op": "insert", "path": "/items/1", "value": number("9")},
        ],
    )
    assert edited.to_python() == {"empty": {"enabled": True}, "items": [1, 9, 2]}
    assert document.to_python() == {"empty": {}, "items": [1, 2]}


def test_root_replacement_keeps_external_bom_and_comments(edit):
    source = "\ufeff# before\r\n!app.Old {old:1} # after"
    document = daml.parse(source)
    value = {"kind": "list", "items": [{"kind": "bool", "value": True}]}
    edited = edit(document, [{"op": "replace", "path": "", "value": value}])
    assert edited.source == "\ufeff# before\r\n[true] # after"
    assert edited.value() == value
    assert document.source == source


@pytest.mark.parametrize("edits", [[], ()], ids=["list", "tuple"])
def test_empty_batch_returns_new_document_with_exact_source(edit, edits):
    source = "\ufeff# keep\r\n{value:1.00e0}\n# tail"
    document = daml.parse(source)
    copied = edit(document, edits)
    assert isinstance(copied, daml.Document)
    assert copied is not document
    assert copied.source == source
    assert copied.value() == document.value()


def test_pointers_use_literal_keys_and_rfc6901_escapes(edit):
    document = daml.parse("{a/b:1\na.b:2\n00:3\na:{b:4}}")
    edited = edit(
        document,
        [
            {"op": "replace", "path": path, "value": number("9")}
            for path in ["/a~1b", "/a.b", "/00"]
        ],
    )
    assert edited.to_python() == {"a/b": 9, "a.b": 9, "00": 9, "a": {"b": 4}}


@pytest.mark.parametrize(
    "operation,path,kind",
    [
        ("remove", "", "InvalidTarget"),
        ("insert", "", "InvalidTarget"),
        ("replace", "missing", "InvalidPointer"),
        ("remove", "/absent/~2", "InvalidPointer"),
        ("remove", "/array/01", "InvalidPointer"),
        ("insert", "/array/-", "InvalidPointer"),
        ("replace", "/absent", "PathNotFound"),
        ("insert", "/absent/child", "PathNotFound"),
        ("remove", "/array/2", "PathNotFound"),
        ("insert", "/array/3", "PathNotFound"),
        ("insert", "/existing", "InvalidTarget"),
        ("insert", "/scalar/child", "InvalidTarget"),
    ],
)
def test_source_path_failures_are_public_and_atomic(edit, operation, path, kind):
    source = "{array:[1\n2]\nscalar:1\nexisting:true\nkeep:7}"
    document = daml.parse(source)
    invalid = {"op": operation, "path": path}
    if operation != "remove":
        invalid["value"] = number("9")
    with pytest.raises(daml.SourceEditError) as caught:
        edit(
            document,
            [{"op": "replace", "path": "/keep", "value": number("8")}, invalid],
        )
    error = caught.value
    assert type(error) is native.SourceEditError
    assert isinstance(error, ValueError)
    assert error.kind == kind
    assert error.path == path
    assert error.operations is None
    assert error.__cause__ is None
    assert document.source == source
    assert document.to_python()["keep"] == 7


@pytest.mark.parametrize("conflict", ["duplicate", "ancestor", "coincident"])
def test_conflicts_report_original_operation_indices(edit, conflict):
    source = '{keep:7\nitems:[{name:"one"}\n{name:"two"}]\nmap:{a:1}}'
    document = daml.parse(source)
    if conflict == "duplicate":
        pair = [
            {"op": "remove", "path": "/map/a"},
            {"op": "replace", "path": "/map/a", "value": number("9")},
        ]
    elif conflict == "ancestor":
        pair = [
            {"op": "replace", "path": "/items/1/name", "value": string("new")},
            {"op": "remove", "path": "/items/1"},
        ]
    else:
        pair = [
            {"op": "insert", "path": "/map/b", "value": number("9")},
            {"op": "insert", "path": "/map/c", "value": number("10")},
        ]
    with pytest.raises(daml.SourceEditError) as caught:
        edit(document, [{"op": "remove", "path": "/keep"}, *pair])
    assert caught.value.kind == "Conflict"
    assert caught.value.path is None
    assert caught.value.operations == (1, 2)
    assert isinstance(caught.value.operations, tuple)
    assert document.source == source


def test_python_derived_validation_keeps_defaults_omitted(worker_schemas):
    source = '# before\r\n!app.Worker {name:"old"} # after'
    document = daml.parse(source)
    edited = worker_schemas.edit(
        document, [{"op": "replace", "path": "/name", "value": string("new")}]
    )
    assert edited.source == source.replace('"old"', '"new"', 1)
    assert isinstance(edited, daml.Document)
    assert edited.to_python() == {"name": "new"}
    assert worker_schemas.resolve(edited).to_python() == {"name": "new", "retries": 3}
    copied = worker_schemas.edit(edited, [], expected="app.Worker")
    assert copied is not edited
    assert copied.source == edited.source
    assert copied.to_python() == {"name": "new"}
    with pytest.raises(daml.SourceEditError) as caught:
        worker_schemas.edit(
            edited,
            [{"op": "replace", "path": "/retries", "value": number("4")}],
        )
    assert caught.value.kind == "PathNotFound"
    assert caught.value.path == "/retries"
    explicit = worker_schemas.edit(
        edited, [{"op": "insert", "path": "/retries", "value": number("4")}]
    )
    assert explicit.to_python() == {"name": "new", "retries": 4}
    assert document.source == source


def test_schema_edit_can_repair_initially_invalid_document(worker_schemas):
    source = "{name:7}"
    document = daml.parse(source)
    with pytest.raises(daml.ResolveError):
        worker_schemas.resolve(document, expected="app.Worker")
    repaired = worker_schemas.edit(
        document,
        [{"op": "replace", "path": "/name", "value": string("fixed")}],
        expected="app.Worker",
    )
    assert repaired.source == '{name:"fixed"}'
    assert repaired.to_python() == {"name": "fixed"}
    assert worker_schemas.resolve(repaired, expected="app.Worker").to_python() == {
        "name": "fixed",
        "retries": 3,
    }
    assert document.source == source


@pytest.mark.parametrize("empty", [False, True], ids=["edited", "empty-batch"])
def test_schema_edit_validates_entire_final_document_and_chains_resolve_error(
    worker_schemas, empty
):
    source = "{name:7\nretries:1}"
    document = daml.parse(source)
    edits = (
        [] if empty else [{"op": "replace", "path": "/retries", "value": number("2")}]
    )
    with pytest.raises(daml.SourceEditError) as caught:
        worker_schemas.edit(document, edits, expected="app.Worker")
    error = caught.value
    assert error.kind == "Resolve"
    assert error.path is None
    assert error.operations is None
    assert isinstance(error.__cause__, daml.ResolveError)
    assert error.__cause__.kind == "SchemaViolation"
    assert error.__cause__.path == "/name"
    assert error.__cause__.span is not None
    assert document.source == source


def test_expected_contract_adds_to_explicit_root_assertion():
    schemas = daml.Schemas.compose(
        [
            daml.schema_from_types(
                {
                    "Minimum": Annotated[int, Field(ge=10)],
                    "Maximum": Annotated[int, Field(le=15)],
                }
            )
        ]
    )
    document = daml.parse("!Maximum 12")
    assert schemas.edit(document, [], expected="Minimum").to_python() == 12
    for raw in ["3", "20"]:
        replacement = number(raw) | {"tag": {"name": "Maximum"}}
        with pytest.raises(daml.SourceEditError) as caught:
            schemas.edit(
                document,
                [{"op": "replace", "path": "", "value": replacement}],
                expected="Minimum",
            )
        assert caught.value.kind == "Resolve"
        assert isinstance(caught.value.__cause__, daml.ResolveError)
        assert caught.value.__cause__.path == ""
    assert document.source == "!Maximum 12"


def test_exact_numbers_and_intrinsic_kinds_survive_new_source_regions(edit):
    tokens = [
        "900719925474099312345678901234567890",
        "1234567890.01234567890123456789",
        "-0.0",
        "1.2300e-40",
        "1e400",
    ]
    temporals = [
        {"kind": "date", "value": "0000-02-29"},
        {"kind": "time", "value": "09:30:00.123456789123"},
        {"kind": "datetime", "value": "2026-10-29T09:30:00.123456789Z"},
        {"kind": "cron", "value": "0 0 9 * * *"},
    ]
    value = {"kind": "list", "items": [*[number(raw) for raw in tokens], *temporals]}
    document = daml.parse("{value:null\nkeep:9.00e2}")
    edited = edit(document, [{"op": "replace", "path": "/value", "value": value}])
    actual = field(edited.value(), "value")["items"]
    for item, token in zip(actual[: len(tokens)], tokens, strict=True):
        assert item["kind"] == "number"
        raw = item["value"]
        assert isinstance(raw, str)
        assert Decimal(raw) == Decimal(token)
        assert any(char in raw for char in ".eE") == any(
            char in token for char in ".eE"
        )
        if Decimal(token).is_zero():
            assert Decimal(raw).is_signed() == Decimal(token).is_signed()
    assert actual[len(tokens) :] == temporals
    converted = edited.to_python()["value"]
    assert isinstance(converted[0], int)
    assert isinstance(converted[1], Decimal)
    assert isinstance(converted[-4], daml.Date)
    assert isinstance(converted[-3], daml.Time)
    assert isinstance(converted[-2], daml.DateTime)
    assert isinstance(converted[-1], daml.Cron)
    assert "keep:9.00e2" in edited.source


def test_parse_only_large_exponent_survives_but_checked_edit_keeps_resolver_bounds():
    document = daml.parse("{value:null}")
    edits = [{"op": "replace", "path": "/value", "value": number("1e1000")}]
    edited = document.edit(edits)
    supplied = field(edited.value(), "value")
    assert supplied["kind"] == "number"
    assert isinstance(supplied["value"], str)
    assert Decimal(supplied["value"]) == Decimal("1e1000")

    # Parsing/rendering retains exact lexemes beyond the resolver's domain.
    # Checked editing must still apply the existing numeric validation bounds.
    with pytest.raises(daml.SourceEditError) as caught:
        daml.Schemas.compose([]).edit(document, edits)
    assert caught.value.kind == "Resolve"
    assert caught.value.path is None
    assert caught.value.operations is None
    assert isinstance(caught.value.__cause__, daml.ResolveError)
    assert caught.value.__cause__.kind == "NumericLimit"
    assert caught.value.__cause__.path == "/value"
    assert document.source == "{value:null}"


def test_visible_native_bound_tag_identity_requires_context_and_explicit_admission():
    bundle = daml.Bundle.from_sources(
        ".",
        {
            "daml.bundle": "{}",
            "Host.daml": "!Object {payload: !self.Text}",
            "Text.daml": "!String",
        },
    )
    document = daml.parse('{payload:"old"}')
    # additional_types exposes Rust compile_with_types admission in Python.
    schemas = bundle.compile(
        document, expected="self.Host", additional_types=["self.Text"]
    )
    supplied = schemas.resolve(daml.parse('!self.Text "new"')).value()
    original_supplied = deepcopy(supplied)
    assert isinstance(supplied["tag"]["binding"], str)
    edits = [{"op": "replace", "path": "/payload", "value": supplied}]
    with pytest.raises(daml.SourceEditError) as caught:
        document.edit(edits)
    assert caught.value.kind == "Semantic"
    assert isinstance(caught.value.__cause__, daml.SemanticError)
    assert caught.value.__cause__.kind == "UnrepresentableTag"
    edited = schemas.edit(document, edits, expected="self.Host")
    resolved = schemas.resolve(edited, expected="self.Host")
    assert resolved.to_python() == {"payload": "new"}
    assert field(resolved.value(), "payload")["tag"] == supplied["tag"]
    assert supplied == original_supplied
    assert document.source == '{payload:"old"}'

    # Having a resource table is not permission to admit a newly edited tag.
    unadmitted = bundle.compile(document)
    with pytest.raises(daml.SourceEditError) as caught:
        unadmitted.edit(
            document,
            [
                {
                    "op": "replace",
                    "path": "/payload",
                    "value": string("new") | {"tag": {"name": "self.Text"}},
                }
            ],
        )
    # Native visibility is enforced while rendering the supplied subtree,
    # before final source resolution; it cannot trigger resource admission.
    assert caught.value.kind == "Semantic"
    assert isinstance(caught.value.__cause__, daml.SemanticError)
    assert caught.value.__cause__.kind == "UnrepresentableTag"
    assert caught.value.__cause__.pointer == ""
    assert document.source == '{payload:"old"}'


def test_private_native_binding_is_not_stripped_or_rebound_by_visible_name():
    bundle = daml.Bundle.from_sources(
        ".",
        {
            "daml.bundle": '{app:"./app"\nprovider:"./decoy"}',
            "app/daml.bundle": '{provider:"./provider"}',
            "app/Host.daml": "!Object {child: (!Object = !provider.Adapter)}",
            "app/provider/daml.bundle": "{}",
            "app/provider/Adapter.daml": (
                '!Object {value: !String {min:1}} = {value:"private"}'
            ),
            "decoy/daml.bundle": "{}",
            "decoy/Adapter.daml": "!Int",
        },
    )
    document = daml.parse("{host: !app.Host\ndecoy: !provider.Adapter 7}")
    schemas = bundle.compile(document)
    exported = schemas.resolve(document).value()
    child = field(field(exported, "host"), "child")
    assert child["tag"]["name"] == "provider.Adapter"
    assert isinstance(child["tag"]["binding"], str)
    before = deepcopy(exported)
    with pytest.raises(daml.SourceEditError) as caught:
        schemas.edit(document, [{"op": "replace", "path": "", "value": exported}])
    assert caught.value.kind == "Semantic"
    assert caught.value.path is None
    assert caught.value.operations is None
    assert isinstance(caught.value.__cause__, daml.SemanticError)
    assert caught.value.__cause__.kind == "UnrepresentableTag"
    assert caught.value.__cause__.pointer == "/host/child"
    assert exported == before
    assert document.to_python() == {"host": {}, "decoy": 7}


@pytest.mark.parametrize("width,error_type", [("80", TypeError), (-1, OverflowError)])
def test_line_width_argument_conversion_keeps_python_errors(edit, width, error_type):
    with pytest.raises(error_type):
        edit(daml.parse("{}"), [], line_width=width)


def test_source_core_semantic_failure_retains_typed_public_cause(edit):
    with pytest.raises(daml.SourceEditError) as caught:
        edit(daml.parse("{}"), [], line_width=0)
    assert caught.value.kind == "Semantic"
    assert caught.value.path is None
    assert caught.value.operations is None
    assert isinstance(caught.value.__cause__, daml.SemanticError)
    assert caught.value.__cause__.kind == "InvalidLineWidth"
    assert caught.value.__cause__.pointer == ""


@pytest.mark.parametrize(
    "malformed",
    [
        None,
        {},
        {"op": "replace", "path": ""},
        {"op": "remove"},
        {"op": "remove", "path": 0},
        {"op": "remove", "path": "", "value": None},
        {"op": "remove", "path": "", "unknown": True},
        {"op": "copy", "path": "", "from": "/value"},
        {"op": "expect", "path": "", "name": "Config"},
        {"op": "replace", "path": "", "value": None},
        {"op": "replace", "path": "", "value": {"kind": "null"}, "extra": True},
        {"op": "remove", "path": "/\ud800"},
        {"op": "\ud800", "path": ""},
    ],
    ids=[
        "not-record",
        "missing-op",
        "missing-value",
        "missing-path",
        "non-string-path",
        "remove-forbids-null-value",
        "unknown-op-field",
        "copy-is-not-source-edit",
        "expect-is-not-source-edit",
        "not-semantic-value",
        "unknown-replace-field",
        "invalid-path-unicode",
        "invalid-op-unicode",
    ],
)
def test_malformed_operation_records_are_semantic_admission_errors(edit, malformed):
    document = daml.parse("{value:1}")
    with pytest.raises(daml.SemanticError) as caught:
        edit(document, [malformed])
    assert caught.value.kind == "InvalidInput"
    assert isinstance(caught.value.pointer, str)
    assert document.source == "{value:1}"


@pytest.mark.parametrize(
    "value",
    [
        {"value": "missing-kind"},
        {"kind": "future"},
        {"kind": "null", "value": None},
        {"kind": "string", "value": "x", "unknown": True},
        {"kind": "bool", "value": 1},
        number(1),
        number(1.5),
        number(True),
        number(Decimal("1.5")),
        {"kind": "list", "items": ()},
        {"kind": "string", "value": "\ud800"},
        {"kind": "null", "tag": {"name": "T", "unknown": True}},
    ],
    ids=[
        "missing-kind",
        "unknown-kind",
        "null-payload",
        "unknown-node-field",
        "non-boolean",
        "integer-is-not-lexeme",
        "lossy-float",
        "bool-is-not-number",
        "decimal-is-not-lexeme",
        "non-list-items",
        "invalid-value-unicode",
        "unknown-tag-field",
    ],
)
def test_malformed_semantic_values_are_not_coerced(edit, value):
    document = daml.parse("{value:1}")
    with pytest.raises(daml.SemanticError) as caught:
        edit(document, [{"op": "replace", "path": "/value", "value": value}])
    assert caught.value.kind == "InvalidInput"
    assert document.source == "{value:1}"


@pytest.mark.parametrize("raw", ["01", "+1", "NaN", "1 2"])
def test_invalid_numeric_lexemes_fail_transport_not_source_editing(edit, raw):
    with pytest.raises(daml.SemanticError) as caught:
        edit(daml.parse("null"), [{"op": "replace", "path": "", "value": number(raw)}])
    assert caught.value.kind == "InvalidNumber"


@pytest.mark.parametrize("edits", [None, {}, "[]"])
def test_edits_require_actual_list_or_tuple(edit, edits):
    with pytest.raises(daml.SemanticError) as caught:
        edit(daml.parse("{}"), edits)
    assert caught.value.kind == "InvalidInput"


def test_generator_is_rejected_without_consuming_it(edit):
    visited = []

    def operations():
        visited.append(True)
        yield {"op": "remove", "path": "/value"}

    with pytest.raises(daml.SemanticError) as caught:
        edit(daml.parse("{value:1}"), operations())
    assert caught.value.kind == "InvalidInput"
    assert visited == []


def test_cycles_fail_but_shared_acyclic_values_are_valid(edit):
    document = daml.parse("{left:null\nright:null}")
    cyclic = {"kind": "list", "items": []}
    cyclic["items"].append(cyclic)
    with pytest.raises(daml.SemanticError) as caught:
        edit(document, [{"op": "replace", "path": "/left", "value": cyclic}])
    assert caught.value.kind == "InvalidInput"
    assert caught.value.pointer.endswith("/0")
    shared = {"kind": "list", "items": [number("7")]}
    edited = edit(
        document,
        [
            {"op": "replace", "path": path, "value": shared}
            for path in ["/left", "/right"]
        ],
    )
    assert edited.to_python() == {"left": [7], "right": [7]}
    assert document.to_python() == {"left": None, "right": None}


@pytest.mark.parametrize("container", [list, tuple], ids=["list", "tuple"])
def test_operation_limit_precedes_any_element_conversion(edit, container):
    # Invalid elements would produce SemanticError if decoded before the cap.
    oversized = container([object()] * 1025)
    with pytest.raises(daml.SourceEditError) as caught:
        edit(daml.parse("{}"), oversized)
    assert caught.value.kind == "LimitExceeded"
    assert caught.value.path is None
    assert caught.value.operations is None
    assert caught.value.__cause__ is None


def test_semantic_node_budget_is_cumulative_across_operations(edit):
    document = daml.parse("{left:null\nright:null}")
    value = {"kind": "list", "items": [{"kind": "null"}] * 60_000}
    edits = [
        {"op": "replace", "path": path, "value": value} for path in ["/left", "/right"]
    ]
    assert len(edit(document, edits[:1]).to_python()["left"]) == 60_000
    with pytest.raises(daml.SemanticError) as caught:
        edit(document, edits)
    assert caught.value.kind == "LimitExceeded"
    assert document.to_python() == {"left": None, "right": None}


def test_semantic_byte_budget_is_cumulative_across_operations(edit):
    document = daml.parse("{" + "\n".join(f"v{i}:null" for i in range(10)) + "}")
    shared = string("x" * 900_000)
    edits = [{"op": "replace", "path": f"/v{i}", "value": shared} for i in range(10)]
    assert edit(document, edits[:1]).to_python()["v0"] == shared["value"]
    with pytest.raises(daml.SemanticError) as caught:
        edit(document, edits)
    assert caught.value.kind == "LimitExceeded"
    assert all(value is None for value in document.to_python().values())


def test_expected_unicode_uses_semantic_admission(worker_schemas):
    with pytest.raises(daml.SemanticError) as caught:
        worker_schemas.edit(daml.parse('{name:"valid"}'), [], expected="\ud800")
    assert caught.value.kind == "InvalidInput"


def test_editing_has_no_file_effects_or_implicit_schema_discovery(
    tmp_path, monkeypatch
):
    source = "# authored\n{value:1.00e0}"
    (tmp_path / "config.daml").write_text(source)
    (tmp_path / "daml.bundle").write_text("{}")
    (tmp_path / "Hidden.daml").write_text("!String")
    before = {path.name: path.read_bytes() for path in tmp_path.iterdir()}
    monkeypatch.chdir(tmp_path)
    document = daml.parse((tmp_path / "config.daml").read_text())
    schemas = daml.Schemas.compose([])
    edits = [{"op": "replace", "path": "/value", "value": number("2")}]
    assert document.edit(edits).to_python() == {"value": 2}
    assert schemas.edit(document, edits).to_python() == {"value": 2}
    with pytest.raises(daml.SourceEditError) as caught:
        schemas.edit(
            document,
            [
                {
                    "op": "replace",
                    "path": "/value",
                    "value": string("hidden") | {"tag": {"name": "self.Hidden"}},
                }
            ],
        )
    assert caught.value.kind == "Resolve"
    assert isinstance(caught.value.__cause__, daml.ResolveError)
    assert caught.value.__cause__.kind == "UnknownTag"
    assert document.source == source
    assert {path.name: path.read_bytes() for path in tmp_path.iterdir()} == before
