"""Bounded Python type projections into DAML's JSON Schema module boundary."""

import dataclasses
import enum
import json
import math
import re
from collections.abc import Callable, Mapping
from typing import (
    Annotated,
    Any,
    Literal,
    NewType,
    TypeAliasType,
    TypeVar,
    get_args,
    get_origin,
    get_type_hints,
)

from .daml import SchemaModule


def schema_from_types(
    types: Mapping[str, Any],
    *,
    exporter: Callable[[Any], dict[str, Any] | bool] | None = None,
) -> SchemaModule:
    """Derive a schema module from explicitly named Python type expressions.

    The default exporter requires ``daml[types]`` and uses Pydantic's validation
    JSON Schema, with aliases. Only declarative JSON contracts are supported;
    this does not instantiate models or reproduce Python coercion/serialization.
    An explicit exporter owns its translation and does not require Pydantic.
    """
    if not isinstance(types, Mapping):
        raise TypeError("types must be a mapping of DAML names to Python types")
    if exporter is not None and not callable(exporter):
        raise TypeError("exporter must be callable")

    bindings = {}
    for name, type_expression in types.items():
        if not isinstance(name, str):
            raise TypeError("type binding names must be strings")
        context = f"type binding {name!r}"
        schema = (
            _export_pydantic(type_expression, context)
            if exporter is None
            else exporter(type_expression)
        )
        if not isinstance(schema, (dict, bool)):
            raise TypeError(
                f"{context}: exporter must return a JSON Schema object or boolean"
            )
        _check_json(schema, context)
        _check_formats(schema, context)
        bindings[name] = {"schema": schema}

    # Native admission owns names, bounds, resource identities and schema shape.
    # In particular, keep each root's local $defs scope; do not flatten or compose.
    return SchemaModule.from_json(json.dumps({"types": bindings}, allow_nan=False))


def _check_json(value: Any, context: str, *, python_default: bool = False) -> None:
    """Check representation without copying, coercing keys, or calling serializers."""
    active = set()
    pending = [(value, "$", False)]
    while pending:
        item, path, leaving = pending.pop()
        if leaving:
            active.remove(id(item))
            continue
        if python_default and isinstance(item, enum.Enum):
            pending.append((item.value, path, False))
            continue
        if item is None or type(item) in (str, bool, int):
            continue
        if type(item) is float:
            if not math.isfinite(item):
                raise ValueError(
                    f"{context} at {path}: non-finite numbers are not JSON"
                )
            continue
        is_array = isinstance(item, list) or (
            python_default and isinstance(item, tuple)
        )
        if not isinstance(item, dict) and not is_array:
            raise TypeError(
                f"{context} at {path}: {type(item).__name__} is not a JSON value"
            )
        if id(item) in active:
            raise ValueError(f"{context} at {path}: cyclic JSON data is unsupported")
        active.add(id(item))
        pending.append((item, path, True))
        if isinstance(item, dict):
            for key, child in item.items():
                if not isinstance(key, str):
                    raise TypeError(
                        f"{context} at {path}: JSON object keys must be strings"
                    )
                pending.append((child, f"{path}[{key!r}]", False))
        else:
            pending.extend(
                (child, f"{path}[{index}]", False) for index, child in enumerate(item)
            )


_SCHEMA_MAPS = frozenset(
    ("$defs", "definitions", "properties", "patternProperties", "dependentSchemas")
)
_SCHEMA_ARRAYS = frozenset(("allOf", "anyOf", "oneOf", "prefixItems"))
_SCHEMA_SINGLES = frozenset(
    (
        "additionalProperties",
        "unevaluatedProperties",
        "propertyNames",
        "contains",
        "additionalItems",
        "unevaluatedItems",
        "not",
        "if",
        "then",
        "else",
        "contentSchema",
    )
)


def _check_formats(schema: dict[str, Any] | bool, context: str) -> None:
    # Only vocabulary-defined schema positions are schemas. Defaults, examples,
    # extension metadata and property *names* must remain ordinary caller data.
    pending = [(schema, "$")]
    while pending:
        node, path = pending.pop()
        if not isinstance(node, dict):
            continue  # Native admission/composition diagnoses malformed schemas.
        if "format" in node:
            raise ValueError(
                f"{context} at {path}: format is unsupported; "
                "DAML does not assert JSON Schema formats"
            )
        for keyword, value in node.items():
            child_path = f"{path}.{keyword}"
            if keyword in _SCHEMA_MAPS and isinstance(value, dict):
                pending.extend(
                    (child, f"{child_path}[{key!r}]") for key, child in value.items()
                )
            elif keyword in _SCHEMA_ARRAYS and isinstance(value, list):
                pending.extend(
                    (child, f"{child_path}[{index}]")
                    for index, child in enumerate(value)
                )
            elif keyword in _SCHEMA_SINGLES:
                pending.append((value, child_path))
            elif keyword == "items":
                if isinstance(value, list):  # Earlier JSON Schema dialects.
                    pending.extend(
                        (child, f"{child_path}[{index}]")
                        for index, child in enumerate(value)
                    )
                else:
                    pending.append((value, child_path))
            elif keyword == "dependencies" and isinstance(value, dict):
                pending.extend(
                    (child, f"{child_path}[{key!r}]")
                    for key, child in value.items()
                    if isinstance(child, (dict, bool))
                )


def _export_pydantic(type_expression: Any, context: str) -> dict[str, Any]:
    # Pydantic is optional: base imports and explicit exporters never load it.
    try:
        from pydantic import BaseModel, TypeAdapter
        from pydantic.json_schema import GenerateJsonSchema
    except ModuleNotFoundError as error:
        if error.name != "pydantic":
            raise
        raise ImportError(
            "schema_from_types requires the optional 'types' extra; "
            "install daml[types] or supply exporter="
        ) from error

    class DeclarativeJsonSchema(GenerateJsonSchema):
        # The default generator suppresses skipped-union-choice warnings. Fail
        # even for those: dropping an unsupported alternative weakens a contract.
        def emit_warning(self, kind: Any, detail: str) -> None:
            raise ValueError(f"{context}: JSON Schema warning ({kind}): {detail}")

    try:
        _check_type_hooks(type_expression, BaseModel, context)
        adapter = TypeAdapter(type_expression)
        _check_core_schema(adapter.core_schema, BaseModel, context)
        return adapter.json_schema(
            mode="validation", by_alias=True, schema_generator=DeclarativeJsonSchema
        )
    except (TypeError, ValueError, NameError) as error:
        error_type = TypeError if isinstance(error, TypeError) else ValueError
        raise error_type(f"{context}: {error}") from error


def _check_config(config: Mapping[str, Any], context: str) -> None:
    for key in ("str_strip_whitespace", "str_to_lower", "str_to_upper"):
        if config.get(key):
            raise ValueError(f"{context}: normalization setting {key!r} is unsupported")
    for key in (
        "json_schema_extra",
        "json_schema_mode_override",
        "model_title_generator",
        "field_title_generator",
        "json_encoders",
    ):
        if config.get(key) is not None:
            raise ValueError(
                f"{context}: custom schema/serialization setting {key!r} is unsupported"
            )


def _check_type_hooks(type_expression: Any, base_model: type, context: str) -> None:
    """Find hooks which can disappear entirely when a core schema is generated."""
    # This inspection only runs on the optional Pydantic exporter route.
    from pydantic.fields import FieldInfo

    pending = [type_expression]
    seen = set()
    while pending:
        expression = pending.pop()
        if id(expression) in seen:
            continue
        seen.add(id(expression))
        # Reflection is confined to heterogeneous framework types/metadata:
        # a user core hook may otherwise disappear into a plain scalar schema.
        for hook in ("__get_pydantic_core_schema__", "__get_pydantic_json_schema__"):
            method = getattr(expression, hook, None)
            if method is None:
                continue
            function = method.__func__ if hasattr(method, "__func__") else method
            base_function = getattr(base_model, hook).__func__
            if function is not base_function:
                raise ValueError(f"{context}: custom {hook} is unsupported")
        if isinstance(expression, TypeAliasType):
            pending.append(expression.__value__)
        if isinstance(expression, NewType):
            pending.append(expression.__supertype__)
        if isinstance(expression, TypeVar):
            pending.extend(expression.__constraints__)
            if expression.__bound__ is not None:
                pending.append(expression.__bound__)
        origin = get_origin(expression)
        if origin is not Literal:
            pending.extend(get_args(expression))
        if origin is not None and origin not in (Annotated, Literal):
            pending.append(origin)
        if origin is Annotated:
            continue
        if isinstance(expression, type) and issubclass(expression, base_model):
            _check_config(expression.model_config, context)
            if expression.model_config.get("extra") == "allow":
                pending.append(
                    get_type_hints(expression, include_extras=True).get(
                        "__pydantic_extra__"
                    )
                )
            for name, field in expression.model_fields.items():
                _check_field_info(field, f"{context}.{name}", pending)
        elif isinstance(expression, type) and dataclasses.is_dataclass(expression):
            _check_config(getattr(expression, "__pydantic_config__", {}), context)
            init = expression.__init__
            code = getattr(init, "__code__", None)
            if code is None or code.co_filename != "<string>":
                raise ValueError(f"{context}: dataclass custom-init is unsupported")
            if hasattr(expression, "__post_init__"):
                raise ValueError(f"{context}: dataclass post-init is unsupported")
            # Resolved annotations also expose hooks hidden by a scalar core
            # schema. Evaluation is trusted Python, just like TypeAdapter itself.
            annotations = get_type_hints(expression, include_extras=True)
            for field in dataclasses.fields(expression):
                pending.append(annotations[field.name])
                if field.default_factory is not dataclasses.MISSING:
                    raise ValueError(
                        f"{context}.{field.name}: default factory is unsupported"
                    )
                if isinstance(field.default, FieldInfo):
                    _check_field_info(field.default, f"{context}.{field.name}", pending)
        elif isinstance(expression, type) and hasattr(expression, "__required_keys__"):
            pending.extend(get_type_hints(expression, include_extras=True).values())
        # FieldInfo may appear directly inside Annotated, including its metadata.
        if isinstance(expression, FieldInfo):
            _check_field_info(expression, context, pending)


def _check_field_info(field: Any, context: str, pending: list[Any]) -> None:
    if field.default_factory is not None:
        raise ValueError(f"{context}: default factory is unsupported")
    if field.json_schema_extra is not None:
        raise ValueError(f"{context}: json_schema_extra override is unsupported")
    if field.field_title_generator is not None:
        raise ValueError(f"{context}: field_title_generator is unsupported")
    if field.exclude_if is not None:
        raise ValueError(f"{context}: custom serialization exclude_if is unsupported")
    pending.append(field.annotation)
    pending.extend(field.metadata)


_CORE_LEAVES = frozenset(
    ("any", "none", "bool", "int", "float", "str", "literal", "enum", "definition-ref")
)
_CORE_WRAPPERS = frozenset(
    (
        "default",
        "nullable",
        "model",
        "dataclass",
        "model-field",
        "typed-dict-field",
        "dataclass-field",
    )
)


def _check_core_schema(root: dict[str, Any], base_model: type, context: str) -> None:
    pending = [(root, "$")]
    seen = set()
    while pending:
        node, path = pending.pop()
        if id(node) in seen:
            continue
        seen.add(id(node))
        location = f"{context} at {path}"
        kind = node["type"]
        if node.get("serialization") is not None:
            raise ValueError(f"{location}: custom serialization is unsupported")
        if node.get("computed_fields"):
            raise ValueError(f"{location}: computed fields are unsupported")
        for flag in (
            "custom_init",
            "post_init",
            "strip_whitespace",
            "to_lower",
            "to_upper",
            "ascii_only",
            "serialization_exclude_if",
            "init_only",
        ):
            if node.get(flag):
                raise ValueError(f"{location}: {flag} behavior is unsupported")
        if node.get("on_error", "raise") != "raise":
            raise ValueError(f"{location}: default on_error behavior is unsupported")
        alias = node.get("validation_alias")
        if alias is not None and not isinstance(alias, str):
            raise ValueError(f"{location}: alias paths/choices are unsupported")
        if node.get("default_factory") is not None:
            raise ValueError(f"{location}: default factory is unsupported")
        if "default" in node:
            _check_json(node["default"], location, python_default=True)
        if "config" in node:
            _check_config(node["config"], location)
        if kind in ("model", "dataclass", "typed-dict") and "cls" in node:
            _check_type_hooks(node["cls"], base_model, location)
        if kind == "str" and isinstance(node.get("pattern"), re.Pattern):
            # Pydantic exports .pattern but drops compiled regex flags.
            if node["pattern"].flags != re.UNICODE:
                raise ValueError(f"{location}: compiled regex flags are unsupported")
        if kind == "literal":
            _check_json(node["expected"], location, python_default=True)
        if kind == "enum":
            cls = node["cls"]
            if cls._missing_.__func__ is not enum.Enum._missing_.__func__:
                raise ValueError(f"{location}: custom enum _missing_ is unsupported")
            for member in node["members"]:
                _check_json(member.value, location)
        _check_core_metadata(node.get("metadata", {}), kind, base_model, location)

        if kind in _CORE_LEAVES:
            continue
        if kind in _CORE_WRAPPERS:
            pending.append((node["schema"], f"{path}.schema"))
        elif kind == "definitions":
            pending.append((node["schema"], f"{path}.schema"))
            pending.extend(
                (child, f"{path}.definitions[{index}]")
                for index, child in enumerate(node["definitions"])
            )
        elif kind == "list":
            if "items_schema" in node:
                pending.append((node["items_schema"], f"{path}.items_schema"))
        elif kind == "tuple":
            pending.extend(
                (child, f"{path}.items_schema[{index}]")
                for index, child in enumerate(node["items_schema"])
            )
        elif kind == "dict":
            keys = node.get("keys_schema", {"type": "any"})
            if keys["type"] not in ("str", "any"):
                raise ValueError(
                    f"{location}: only string-keyed mappings are supported"
                )
            if "pattern" in keys:
                raise ValueError(
                    f"{location}: mapping-key patterns lose constraints in JSON Schema"
                )
            pending.append((keys, f"{path}.keys_schema"))
            if "values_schema" in node:
                pending.append((node["values_schema"], f"{path}.values_schema"))
        elif kind in ("model-fields", "typed-dict", "dataclass-args"):
            fields = node["fields"]
            children = enumerate(fields) if isinstance(fields, list) else fields.items()
            pending.extend(
                (child, f"{path}.fields[{name!r}]") for name, child in children
            )
            if node.get("extras_keys_schema") is not None:
                raise ValueError(
                    f"{location}: constraints on extra-field keys are unsupported"
                )
            if "extras_schema" in node:
                pending.append((node["extras_schema"], f"{path}.extras_schema"))
        elif kind == "union":
            pending.extend(
                (
                    child[0] if isinstance(child, tuple) else child,
                    f"{path}.choices[{index}]",
                )
                for index, child in enumerate(node["choices"])
            )
        elif kind == "tagged-union":
            if not isinstance(node["discriminator"], str):
                raise ValueError(
                    f"{location}: callable/path discriminators are unsupported"
                )
            pending.extend(
                (child, f"{path}.choices[{name!r}]")
                for name, child in node["choices"].items()
            )
        else:
            raise ValueError(f"{location}: unsupported Pydantic core schema {kind!r}")


def _check_core_metadata(
    metadata: dict[str, Any], kind: str, base_model: type, context: str
) -> None:
    if metadata.get("pydantic_js_annotation_functions"):
        raise ValueError(
            f"{context}: custom JSON Schema annotation override is unsupported"
        )
    if metadata.get("pydantic_js_extra") is not None:
        raise ValueError(f"{context}: JSON Schema extra override is unsupported")
    updates = metadata.get("pydantic_js_updates", {})
    if updates.keys() - {"title", "description", "examples", "deprecated"}:
        raise ValueError(f"{context}: JSON Schema constraint override is unsupported")
    for method in metadata.get("pydantic_js_functions", ()):
        function = method.__func__ if hasattr(method, "__func__") else method
        if function is base_model.__get_pydantic_json_schema__.__func__:
            continue
        # Pydantic attaches its own annotation-only enum title/docstring hook.
        # No arbitrary function (including a caller's enum hook) is admitted.
        if (
            kind == "enum"
            and getattr(function, "__module__", None)
            == "pydantic._internal._generate_schema"
            and getattr(function, "__qualname__", None)
            == "GenerateSchema._enum_schema.<locals>.get_json_schema"
        ):
            continue
        raise ValueError(
            f"{context}: custom JSON Schema function override is unsupported"
        )
