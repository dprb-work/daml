"""Consumer-visible contracts for Python type schema projection."""

import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated, TypeVar

import daml
import msgspec
import pytest
from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    WithJsonSchema,
    field_validator,
    model_serializer,
    model_validator,
)
from typing_extensions import TypeVar as TypeVarWithDefault


def resolve(types, value, expected, **kwargs):
    module = daml.schema_from_types(types, **kwargs)
    return (
        daml.Schemas.compose([module])
        .resolve(daml.parse(value), expected=expected)
        .to_python()
    )


@pytest.mark.parametrize("kind", ["pydantic", "dataclass"])
def test_defaults_constraints_and_plain_values(kind):
    if kind == "pydantic":

        class Worker(BaseModel):
            name: str
            retries: int = Field(default=3, ge=0, le=8)
    else:

        @dataclass
        class Worker:
            name: str
            retries: int = Field(default=3, ge=0, le=8)

    assert resolve({"app.Worker": Worker}, '{name: "demo"}', "app.Worker") == {
        "name": "demo",
        "retries": 3,
    }
    assert (
        resolve({"app.Worker": Worker}, '{name: "demo"\nretries: 6}', "app.Worker")[
            "retries"
        ]
        == 6
    )
    with pytest.raises(daml.ResolveError) as caught:
        resolve({"app.Worker": Worker}, '{name: "demo"\nretries: -1}', "app.Worker")
    assert caught.value.kind == "SchemaViolation"
    with pytest.raises(daml.ResolveError):
        resolve({"app.Worker": Worker}, '{name: "demo"\nretries: "6"}', "app.Worker")


def test_nested_definitions_aliases_unions_and_container_semantics_across_roots():
    def left_types():
        class Inner(BaseModel):
            left_value: int = Field(alias="value")

        class Left(BaseModel):
            inner: Inner
            maybe: int | None
            items: list[int]

        return Left

    def right_types():
        class Inner(BaseModel):
            right_value: str = Field(alias="value")

        class Right(BaseModel):
            inner: Inner
            choice: int | str

        return Right

    Left = left_types()
    Right = right_types()

    schemas = daml.Schemas.compose(
        [daml.schema_from_types({"app.Left": Left, "app.Right": Right})]
    )
    assert schemas.resolve(
        daml.parse("{inner: {value: 9007199254740993}\nmaybe: null}"),
        expected="app.Left",
    ).to_python() == {"inner": {"value": 9007199254740993}, "maybe": None, "items": []}
    assert schemas.resolve(
        daml.parse('{inner: {value: "right"}\nchoice: "x"}'), expected="app.Right"
    ).to_python() == {"inner": {"value": "right"}, "choice": "x"}


def test_projection_is_not_model_validate_coercion_or_extra_field_filtering():
    class Item(BaseModel):
        value: int

    assert resolve({"app.Item": Item}, '{value: 1.0\nextra: "kept"}', "app.Item") == {
        "value": 1,
        "extra": "kept",
    }


def test_non_string_mapping_key_annotation_is_rejected():
    with pytest.raises((TypeError, ValueError)):
        daml.schema_from_types({"app.Mapping": dict[int, str]})


def test_lossy_mapping_schema_forms_fail_with_binding_context():
    class OpenExtra(BaseModel):
        model_config = ConfigDict(extra="allow")
        __pydantic_extra__: dict[Annotated[str, Field(min_length=3)], int]
        value: str

    class Hidden:
        @classmethod
        def __get_pydantic_core_schema__(cls, source_type, handler):
            from pydantic_core import core_schema

            return core_schema.int_schema()

    hidden_union = TypeVar("HiddenOrString", Hidden, str)
    hidden_bound = TypeVar("HiddenBound", bound=Hidden)
    cases = (
        ("app.Keyed", dict[Annotated[str, Field(pattern="^x")], int]),
        ("app.OpenExtra", OpenExtra),
        (
            "app.HiddenExtra",
            type(
                "HiddenExtra",
                (BaseModel,),
                {
                    "model_config": ConfigDict(extra="allow"),
                    "__annotations__": {
                        "__pydantic_extra__": dict[str, Hidden],
                        "value": str,
                    },
                },
            ),
        ),
        ("app.HiddenUnion", hidden_union),
        ("app.HiddenBound", hidden_bound),
    )
    for binding, expression in cases:
        try:
            daml.schema_from_types({binding: expression})
        except (TypeError, ValueError) as error:
            assert binding in str(error)
        else:
            pytest.fail(f"{binding} accepted a lossy schema form")


def test_plain_string_keyed_mappings_and_typed_extras_remain_supported():
    class OpenExtra(BaseModel):
        model_config = ConfigDict(extra="allow")
        __pydantic_extra__: dict[str, int]
        values: dict[str, int]

    module = daml.schema_from_types(
        {"app.OpenExtra": OpenExtra, "app.Mapping": dict[str, int]}
    )
    assert daml.Schemas.compose([module]).resolve(
        daml.parse("{values: {one: 1}\nextra: 2}"), expected="app.OpenExtra"
    ).to_python() == {"values": {"one": 1}, "extra": 2}


def test_local_forward_annotation_with_open_model_resolves():
    def model_type():
        class Inner(BaseModel):
            value: int

        class Outer(BaseModel):
            model_config = ConfigDict(extra="allow")
            inner: "Inner"

        return Outer

    assert resolve({"app.Outer": model_type()}, "{inner: {value: 1}}", "app.Outer") == {
        "inner": {"value": 1}
    }


def test_typevar_default_cannot_hide_custom_core_schema_behavior():
    class Hidden:
        @classmethod
        def __get_pydantic_core_schema__(cls, source_type, handler):
            from pydantic_core import core_schema

            return core_schema.int_schema()

    defaulted = TypeVarWithDefault("HiddenDefault", default=Hidden)
    with pytest.raises((TypeError, ValueError), match="app.HiddenDefault"):
        daml.schema_from_types({"app.HiddenDefault": defaulted})


def test_msgspec_public_exporter_marker_data_and_immutability():
    class Record(msgspec.Struct):
        kind: str
        value: int

    exported = msgspec.json.schema(Record)
    schemas = exported.get("$defs", exported)
    schema = next(
        item
        for item in schemas.values()
        if isinstance(item, dict) and "properties" in item
    )
    schema["properties"]["kind"]["const"] = "widget"
    before = repr(exported)
    actual = resolve(
        {"app.Record": Record},
        '{kind: "widget"\nvalue: 4}',
        "app.Record",
        exporter=lambda _: exported,
    )
    assert actual == {"kind": "widget", "value": 4}
    assert repr(exported) == before
    with pytest.raises(daml.ResolveError):
        resolve(
            {"app.Record": Record},
            '{kind: "other"\nvalue: 4}',
            "app.Record",
            exporter=lambda _: exported,
        )


def test_exporter_exception_identity_and_non_json_data():
    failure = RuntimeError("export failed")
    with pytest.raises(RuntimeError) as caught:
        daml.schema_from_types(
            {"app.X": int}, exporter=lambda _: (_ for _ in ()).throw(failure)
        )
    assert caught.value is failure
    for schema in (
        {"type": "object", "properties": {1: {"type": "string"}}},
        {"type": "number", "default": float("nan")},
        {"type": "number", "default": float("inf")},
    ):
        with pytest.raises((TypeError, ValueError)):
            daml.schema_from_types(
                {"app.X": int}, exporter=lambda _, schema=schema: schema
            )


def test_formats_rejected_at_schema_positions_not_in_property_or_default_data():
    for schema in (
        {
            "type": "object",
            "properties": {"date": {"type": "string", "format": "date"}},
        },
        {
            "$defs": {"Nested": {"type": "string", "format": "date"}},
            "$ref": "#/$defs/Nested",
        },
    ):
        with pytest.raises((TypeError, ValueError)):
            daml.schema_from_types({"app.X": int}, exporter=lambda _: schema)
    schema = {
        "type": "object",
        "properties": {
            "format": {"type": "string"},
            "data": {
                "type": "object",
                "default": {"format": "date", "type": "function-after"},
            },
        },
    }
    module = daml.schema_from_types({"app.X": int}, exporter=lambda _: schema)
    assert daml.Schemas.compose([module]).resolve(
        daml.parse('{format: "date"\ndata: {format: "date"\ntype: "function-after"}}'),
        expected="app.X",
    ).to_python() == {
        "format": "date",
        "data": {"format": "date", "type": "function-after"},
    }


def test_unsupported_pydantic_behavior_and_default_factory_not_invoked():
    calls = []

    class Factory(BaseModel):
        values: list[int] = Field(default_factory=lambda: calls.append(True) or [1])

    class Validator(BaseModel):
        value: int

        @field_validator("value")
        @classmethod
        def normalize(cls, value):
            return value + 1

    class ModelValidator(BaseModel):
        value: int

        @model_validator(mode="after")
        def normalize(self):
            self.value += 1
            return self

    class Serializer(BaseModel):
        value: int

        @model_serializer
        def serialize(self):
            return {"value": str(self.value)}

    class SchemaOverride(BaseModel):
        value: int = Field(json_schema_extra={"type": "string"})

    class JsonSchemaOverride(BaseModel):
        value: Annotated[int, WithJsonSchema({"type": "string"})]

    class NormalizeConfig(BaseModel):
        model_config = ConfigDict(str_strip_whitespace=True)
        value: str

    @dataclass
    class PostInit:
        value: int

        def __post_init__(self):
            self.value += 1

    class LiteralDefault(BaseModel):
        data: dict[str, str] = {"format": "date", "type": "function-after"}

    for model in (
        Factory,
        Validator,
        ModelValidator,
        Serializer,
        SchemaOverride,
        JsonSchemaOverride,
        NormalizeConfig,
        PostInit,
    ):
        with pytest.raises((TypeError, ValueError)):
            daml.schema_from_types({"app.X": model})
    assert calls == []

    literal = daml.schema_from_types({"app.Literal": LiteralDefault})
    assert daml.Schemas.compose([literal]).resolve(
        daml.parse("{}"), expected="app.Literal"
    ).to_python() == {"data": {"format": "date", "type": "function-after"}}


def test_base_import_and_custom_exporter_work_without_site_packages():
    package = Path(daml.__file__).resolve().parent
    script = """import daml
class T: pass
module = daml.schema_from_types({'app.T': T}, exporter=lambda _: {'type': 'string'})
schemas = daml.Schemas.compose([module])
assert schemas.resolve(daml.parse('"ok"'), expected='app.T').to_python() == 'ok'
"""
    result = subprocess.run(
        [sys.executable, "-S", "-c", script],
        cwd=package.parent,
        env={"PYTHONPATH": str(package.parent)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
