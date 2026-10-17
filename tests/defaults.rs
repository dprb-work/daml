//! Partial-input completion, precedence, unions, and completion bounds.

use daml::{
    ErrorOrigin, ResolutionLimits, ResolveErrorKind, ResolvedValue, SchemaModule, Schemas, parse,
};
use serde_json::json;

fn schemas(schema: serde_json::Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({
        "types": { "app.Config": { "schema": schema } }
    }))
    .expect("module")])
    .expect("schemas")
}

fn object_schema(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": true
    })
}

#[test]
fn required_is_checked_after_explicit_default_merge() {
    let schemas = schemas(object_schema(
        json!({
            "settings": { "type": "object", "default": { "mode": "safe" }, "properties": {
                "mode": { "type": "string" }, "enabled": { "type": "boolean", "default": true }
            }, "required": ["mode"] }
        }),
        &["settings"],
    ));
    let input = parse("!app.Config { settings: {}\n}").unwrap();
    let resolved = schemas.resolve(&input).unwrap();
    assert_eq!(
        resolved
            .root()
            .get("settings")
            .unwrap()
            .get("mode")
            .unwrap()
            .string(),
        Some("safe")
    );
    assert_eq!(
        resolved
            .root()
            .get("settings")
            .unwrap()
            .get("enabled")
            .unwrap()
            .value(),
        &ResolvedValue::Bool(true)
    );

    let missing = parse("!app.Config").unwrap();
    let completed = schemas.resolve(&missing).unwrap();
    assert_eq!(
        completed
            .root()
            .get("settings")
            .unwrap()
            .get("mode")
            .unwrap()
            .string(),
        Some("safe")
    );
}

#[test]
fn provided_partial_maps_override_nested_defaults_recursively() {
    let schemas = schemas(object_schema(
        json!({ "settings": {
            "type": "object",
            "default": { "theme": "dark", "layout": { "columns": 2, "compact": false } },
            "properties": {
                "layout": { "type": "object", "properties": {
                    "columns": { "type": "integer" },
                    "compact": { "type": "boolean" },
                    "label": { "type": "string", "default": "default" }
                } }
            }
        } }),
        &["settings"],
    ));
    let input =
        parse("!app.Config { settings: { layout: { columns: 3\n custom: 7 }\n}\n}").unwrap();
    let resolved = schemas.resolve(&input).unwrap();
    let settings = resolved.root().get("settings").unwrap();
    let layout = settings.get("layout").unwrap();
    assert_eq!(layout.get("columns").unwrap().to_json(), json!(3));
    assert_eq!(
        layout.get("compact").unwrap().value(),
        &ResolvedValue::Bool(false)
    );
    assert_eq!(layout.get("label").unwrap().string(), Some("default"));
    assert_eq!(layout.get("custom").unwrap().to_json(), json!(7));
}

#[test]
fn explicit_null_and_arrays_replace_defaults() {
    let schemas = schemas(object_schema(
        json!({
            "nullable": { "default": "fallback" },
            "items": { "default": [1, 2, 3] }
        }),
        &[],
    ));
    let input = parse("!app.Config { nullable: null\n items: [9]\n}").unwrap();
    let resolved = schemas.resolve(&input).unwrap();
    assert_eq!(
        resolved.root().get("nullable").unwrap().value(),
        &ResolvedValue::Null
    );
    assert_eq!(resolved.root().get("items").unwrap().to_json(), json!([9]));
}

#[test]
fn schema_defaults_report_their_origin() {
    let schemas = schemas(json!({
        "type": "object",
        "properties": { "count": { "type": "integer", "default": "not-an-integer" } },
        "required": ["count"]
    }));
    let input = parse("!app.Config {}").unwrap();
    let error = schemas.resolve(&input).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert!(matches!(error.origin, ErrorOrigin::Default(_)));
}

#[test]
fn branch_completions_with_different_values_are_ambiguous() {
    let schemas = schemas(json!({
        "oneOf": [
            { "type": "object", "properties": { "choice": { "type": "string", "const": "left", "default": "left" } }, "required": ["choice"] },
            { "type": "object", "properties": { "choice": { "type": "string", "const": "right", "default": "right" } }, "required": ["choice"] }
        ]
    }));
    let input = parse("!app.Config {}").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn any_of_branches_with_identical_completion_are_allowed() {
    let schemas = schemas(json!({
        "anyOf": [
            { "type": "object", "properties": { "choice": { "type": "string", "default": "same" } }, "required": ["choice"] },
            { "type": "object", "properties": { "choice": { "type": "string", "default": "same" } }, "required": ["choice"] }
        ]
    }));
    let input = parse("!app.Config {}").unwrap();
    assert_eq!(
        schemas
            .resolve(&input)
            .unwrap()
            .root()
            .get("choice")
            .unwrap()
            .string(),
        Some("same")
    );
}

#[test]
fn any_of_equal_values_ignore_default_origin_and_number_spelling() {
    let schemas = schemas(json!({
        "anyOf": [
            { "type": "object", "properties": { "choice": { "default": 1 } }, "required": ["choice"] },
            { "type": "object", "properties": { "choice": { "default": 1.0 } }, "required": ["choice"] }
        ]
    }));
    let input = parse("!app.Config {}\n").unwrap();
    assert_eq!(
        schemas
            .resolve(&input)
            .unwrap()
            .root()
            .get("choice")
            .unwrap()
            .to_json(),
        json!(1)
    );
}

#[test]
fn overlapping_union_keywords_are_both_completed() {
    let schemas = schemas(json!({
        "anyOf": [
            { "type": "object", "properties": { "left": { "default": true } }, "required": ["left"] },
            { "type": "object", "properties": { "left": { "default": true } }, "required": ["left"] }
        ],
        "oneOf": [
            { "type": "object", "properties": { "right": { "default": 2 } }, "required": ["right"] },
            { "type": "null" }
        ]
    }));
    let input = parse("!app.Config {}\n").unwrap();
    let root = schemas.resolve(&input).unwrap();
    assert_eq!(
        root.root().get("left").unwrap().value(),
        &ResolvedValue::Bool(true)
    );
    assert_eq!(root.root().get("right").unwrap().to_json(), json!(2));
}

#[test]
fn nested_unions_complete_without_choosing_a_different_output() {
    let schemas = schemas(json!({
        "anyOf": [
            { "type": "object", "properties": { "nested": {
                "default": {},
                "anyOf": [
                    { "type": "object", "properties": { "value": { "default": "same" } }, "required": ["value"] },
                    { "type": "object", "properties": { "value": { "default": "same" } }, "required": ["value"] }
                ]
            } }, "required": ["nested"] },
            { "type": "object", "properties": { "nested": {
                "default": {},
                "anyOf": [
                    { "type": "object", "properties": { "value": { "default": "same" } }, "required": ["value"] },
                    { "type": "object", "properties": { "value": { "default": "same" } }, "required": ["value"] }
                ]
            } }, "required": ["nested"] }
        ]
    }));
    let input = parse("!app.Config {}\n").unwrap();
    assert_eq!(
        schemas
            .resolve(&input)
            .unwrap()
            .root()
            .get("nested")
            .unwrap()
            .get("value")
            .unwrap()
            .string(),
        Some("same")
    );
}

#[test]
fn conflicting_defaults_for_one_missing_path_are_ambiguous() {
    let schemas = schemas(json!({
        "allOf": [
            { "type": "object", "properties": { "choice": { "default": "first" } } },
            { "type": "object", "properties": { "choice": { "default": "second" } } }
        ]
    }));
    let input = parse("!app.Config {}\n").unwrap();
    let error = schemas.resolve(&input).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::AmbiguousDefault);
    assert!(matches!(error.origin, ErrorOrigin::Default(_)));
}

#[test]
fn untagged_root_still_validates_deep_tags_and_unknown_input_tags() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({
        "types": {
            "app.Config": { "schema": { "type": "object", "properties": {
                "known": { "default": { "value": 1 }, "type": "object" }
            } } },
            "app.Child": { "schema": { "type": "object", "properties": { "value": { "type": "integer" } } } }
        }
    })).unwrap()]).unwrap();
    let input = parse("{ known: !app.Child { value: 1 }\n}").unwrap();
    assert_eq!(
        schemas
            .resolve(&input)
            .unwrap()
            .root()
            .get("known")
            .unwrap()
            .tag(),
        Some("app.Child")
    );

    let input = parse("{ child: !app.Unknown {}\n}").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::UnknownTag
    );
}

#[test]
fn valid_default_union_does_not_hide_unknown_supplied_tag() {
    let schemas = schemas(json!({
        "anyOf": [
            { "type": "object", "properties": { "child": {
                "type": "object", "default": {}, "properties": {}
            } } },
            { "type": "object", "properties": { "fallback": { "default": true } } }
        ]
    }));
    let input = parse("!app.Config { child: !app.Unknown {}\n}").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::UnknownTag
    );
}

#[test]
fn ambiguous_defaults_in_a_type_incompatible_union_branch_do_not_poison_valid_branch() {
    let schemas = schemas(json!({
        "anyOf": [
            {
                "type": "null",
                "allOf": [
                    { "properties": { "choice": { "default": 1 } } },
                    { "properties": { "choice": { "default": 2 } } }
                ]
            },
            { "type": "object" }
        ]
    }));
    let input = parse("!app.Config {}\n").unwrap();
    assert_eq!(schemas.resolve(&input).unwrap().root().to_json(), json!({}));
}

#[test]
fn materialization_and_numeric_limits_are_enforced() {
    let number_schemas = schemas(json!({ "type": "number", "default": 1 }));
    let schemas = schemas(object_schema(
        json!({ "items": { "default": [1, 2, 3] } }),
        &[],
    ));
    let input = parse("!app.Config {}").unwrap();
    let limits = ResolutionLimits {
        max_nodes: 2,
        max_bytes: 1024,
        max_steps: 100,
    };
    assert_eq!(
        schemas
            .resolve_with_limits(&input, None, limits)
            .unwrap_err()
            .kind,
        ResolveErrorKind::LimitExceeded
    );

    let long = format!("!app.Config {}", "9".repeat(513));
    let numeric_input = parse(long).unwrap();
    assert_eq!(
        number_schemas.resolve(&numeric_input).unwrap_err().kind,
        ResolveErrorKind::NumericLimit
    );
}

#[test]
fn tags_validate_scalar_array_and_empty_object_nodes() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({
        "types": {
            "app.Config": { "schema": { "type": "object", "x-daml-tag": "app.Config" } },
            "app.Text": { "schema": { "type": "string", "x-daml-tag": "app.Text" } },
            "app.List": { "schema": { "type": "array", "items": { "type": "integer" }, "x-daml-tag": "app.List" } },
            "app.Empty": { "schema": { "type": "object", "maxProperties": 0, "x-daml-tag": "app.Empty" } }
        }
    })).unwrap()]).unwrap();
    let input = parse(
        "!app.Config { text: !app.Text \"ok\"\n list: !app.List [1\n2]\n empty: !app.Empty {}\n}",
    )
    .unwrap();
    let resolved = schemas.resolve(&input).unwrap();
    assert_eq!(resolved.root().get("text").unwrap().tag(), Some("app.Text"));
    assert_eq!(resolved.root().get("list").unwrap().tag(), Some("app.List"));
    assert_eq!(
        resolved.root().get("empty").unwrap().tag(),
        Some("app.Empty")
    );
}

#[test]
fn ancestor_constraints_filter_nested_union_completions() {
    let schemas = schemas(json!({
        "type": "object",
        "const": {"choice": {"value": 2}},
        "properties": {"choice": {
            "default": {},
            "anyOf": [
                {"properties": {"value": {"default": 1}}, "required": ["value"]},
                {"properties": {"value": {"default": 2}}, "required": ["value"]}
            ]
        }}
    }));
    let input = parse("!app.Config").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"choice": {"value": 2}})
    );
}

#[test]
fn a_missing_property_can_take_an_unambiguous_union_default() {
    let schemas = schemas(json!({
        "type": "object",
        "properties": {"choice": {"anyOf": [
            {"type": "integer", "default": 2},
            {"type": "integer", "default": 2.0}
        ]}},
        "required": ["choice"]
    }));
    let input = parse("!app.Config").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"choice": 2})
    );
}

#[test]
fn union_selection_does_not_give_root_defaults_precedence_over_branch_defaults() {
    let schemas = schemas(json!({
        "default": {"choice": 1},
        "anyOf": [{"default": {"choice": 2}}]
    }));
    let input = parse("!app.Config").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::AmbiguousDefault
    );
    let supplied = parse("!app.Config { choice: 9 }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"choice": 9})
    );
}

#[test]
fn defaults_on_children_do_not_invent_a_missing_parent_object() {
    let schemas = schemas(json!({
        "type": "object",
        "properties": {"settings": {
            "type": "object",
            "properties": {"enabled": {"default": true}}
        }},
        "required": ["settings"]
    }));
    let input = parse("!app.Config").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn unions_without_defaults_are_validated_without_candidate_explosion() {
    let all = (0..20)
        .map(|_| {
            json!({"anyOf": [
                {"type": "object"}, {"maxProperties": 3}
            ]})
        })
        .collect::<Vec<_>>();
    let schemas = schemas(json!({"allOf": all}));
    let input = parse("!app.Config { value: 1 }").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"value": 1})
    );
}
