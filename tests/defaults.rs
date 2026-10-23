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
fn declared_properties_take_precedence_over_additional_property_defaults() {
    let schemas = schemas(json!({
        "type": "object",
        "properties": {
            "named": {"type": "object", "properties": {"label": {"default": "named"}}}
        },
        "additionalProperties": {
            "type": "object", "properties": {"label": {"default": "fallback"}}
        }
    }));
    let input = parse("!app.Config { named: {}\n extra: {} }").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"named": {"label": "named"}, "extra": {"label": "fallback"}})
    );
}

#[test]
fn tuple_prefixes_do_not_receive_tail_defaults_or_create_missing_items() {
    let schemas = schemas(json!({
        "type": "array",
        "prefixItems": [
            {"type": "object", "properties": {"label": {"default": "prefix"}}},
            true
        ],
        "items": {"type": "object", "properties": {"label": {"default": "tail"}}}
    }));
    let input = parse("!app.Config [{}\n{}\n{}]").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!([{"label": "prefix"}, {}, {"label": "tail"}])
    );
    let short = parse("!app.Config []").unwrap();
    assert_eq!(schemas.resolve(&short).unwrap().to_json(), json!([]));

    let invalid = parse("!app.Config [{}\nnull\n1]").unwrap();
    let error = schemas.resolve(&invalid).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/2");
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
            "app.Config": { "schema": { "type": "object" } },
            "app.Text": { "schema": { "type": "string" } },
            "app.List": { "schema": { "type": "array", "items": { "type": "integer" } } },
            "app.Empty": { "schema": { "type": "object", "maxProperties": 0 } }
        }
    }))
    .unwrap()])
    .unwrap();
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
fn root_preset_is_applied_before_union_branch_fallbacks() {
    let schemas = schemas(json!({
        "default": {"choice": 1},
        "anyOf": [{"default": {"choice": 2}}]
    }));
    let input = parse("!app.Config").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"choice": 1})
    );
    let supplied = parse("!app.Config { choice: 9 }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"choice": 9})
    );
}

#[test]
fn required_parent_containers_are_materialized_before_child_defaults() {
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
        schemas.resolve(&input).unwrap().to_json(),
        json!({"settings": {"enabled": true}})
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

#[test]
fn definition_defaults_fill_only_fields_missing_from_the_per_use_preset() {
    let host = SchemaModule::from_value(json!({"types": {
        "Host": {"schema": {
            "type": "object",
            "default": {"value": 1},
            "x-daml-default-tags": [{"path": "", "tag": "Provider"}]
        }}
    }}))
    .unwrap();
    let provider = SchemaModule::from_value(json!({"types": {
        "Provider": {"schema": {"type": "object", "default": {"value": 2}}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([host, provider]).unwrap();
    let partial = parse("{}").unwrap();
    assert_eq!(
        schemas.resolve_as(&partial, "Host").unwrap().to_json(),
        json!({"value": 1})
    );
    let explicitly_tagged = parse("!Provider").unwrap();
    assert_eq!(
        schemas
            .resolve_as(&explicitly_tagged, "Host")
            .unwrap()
            .to_json(),
        json!({"value": 2})
    );
    let supplied = parse("{ value: 9 }").unwrap();
    let completed = schemas.resolve_as(&supplied, "Host").unwrap();
    assert_eq!(completed.root().tag(), Some("Provider"));
    assert_eq!(completed.to_json(), json!({"value": 9}));
}

#[test]
fn tagged_definition_fallback_does_not_conflict_with_a_per_use_default() {
    let host = SchemaModule::from_value(json!({"types": {
        "Host": {"schema": {
            "$id": "urn:host",
            "type": "object",
            "default": {"value": 1},
            "x-daml-default-tags": [{"path": "", "tag": "Provider"}]
        }}
    }}))
    .unwrap();
    let module = SchemaModule::from_value(json!({
        "types": {
            "Provider": {"schema": {"type": "object", "default": {"value": 2}}},
            "Container": {"schema": {"type": "object", "properties": {
                "child": {"$ref": "urn:host"}
            }}}
        }
    }))
    .unwrap();
    let schemas = Schemas::compose([host, module]).unwrap();
    for input in ["!Container", "!Container { child: {} }"] {
        let document = parse(input).unwrap();
        assert_eq!(
            schemas.resolve(&document).unwrap().to_json(),
            json!({"child": {"value": 1}})
        );
    }
}

#[test]
fn different_generated_tags_make_equal_json_completions_ambiguous() {
    let host = SchemaModule::from_value(json!({"types": {
        "Host": {"schema": {
            "type": "object",
            "anyOf": [
                {
                    "default": {"value": 1},
                    "x-daml-default-tags": [{"path": "", "tag": "A"}]
                },
                {
                    "default": {"value": 1},
                    "x-daml-default-tags": [{"path": "", "tag": "B"}]
                }
            ]
        }}
    }}))
    .unwrap();
    let providers = SchemaModule::from_value(json!({"types": {
        "A": {"schema": {"type": "object"}},
        "B": {"schema": {"type": "object"}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([host, providers]).unwrap();
    let partial = parse("{}").unwrap();
    assert_eq!(
        schemas.resolve_as(&partial, "Host").unwrap_err().kind,
        ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn one_of_still_requires_exactly_one_branch_after_equal_completion() {
    let schemas = schemas(json!({
        "type": "object",
        "oneOf": [
            {"properties": {"value": {"default": 1}}, "required": ["value"]},
            {"properties": {"value": {"default": 1.0}}, "maxProperties": 1}
        ]
    }));
    let input = parse("!app.Config").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn uniqueness_uses_json_data_not_tag_metadata() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({"types": {
        "List": {"schema": {"type": "array", "uniqueItems": true}},
        "A": {"schema": {"type": "integer"}},
        "B": {"schema": {"type": "integer"}}
    }}))
    .unwrap()])
    .unwrap();
    let input = parse("!List [!A 1\n!B 1.0]").unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
    let distinct = parse("!List [1e400\n2e400]").unwrap();
    let resolved = schemas.resolve(&distinct).unwrap();
    let items = resolved.root().items().unwrap();
    assert_ne!(items[0].to_json(), items[1].to_json());
}

#[test]
fn expensive_exact_number_uniqueness_fails_through_the_work_budget() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({"types": {
        "List": {"schema": {"type": "array", "uniqueItems": true}}
    }}))
    .unwrap()])
    .unwrap();
    let mut source = String::from("!List [\n");
    for coefficient in 1..=2_000 {
        source.push_str(&format!("{coefficient}e400\n"));
    }
    source.push(']');
    let input = parse(source).unwrap();
    let error = schemas.resolve(&input).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
}

#[test]
fn uniqueness_work_propagates_through_references_to_default_created_arrays() {
    let values = (0..500).map(|value| json!(value)).collect::<Vec<_>>();
    let schema = |unique, values| {
        json!({
            "type": "object",
            "properties": {"values": {
                "$ref": "#/$defs/Items",
                "default": values
            }},
            "required": ["values"],
            "$defs": {"Items": {"type": "array", "uniqueItems": unique}}
        })
    };
    let ordinary = schemas(schema(false, values.clone()));
    let exact = schemas(schema(true, values));
    let input = parse("!app.Config").unwrap();
    let limits = ResolutionLimits {
        max_steps: 10_000_000,
        ..Default::default()
    };
    ordinary
        .resolve_with_limits(&input, None, limits)
        .expect("ordinary validation fits the same budget");
    assert_eq!(
        exact
            .resolve_with_limits(&input, None, limits)
            .unwrap_err()
            .kind,
        ResolveErrorKind::LimitExceeded
    );
    let small = parse("!app.Config { values: [1\n2] }").unwrap();
    exact
        .resolve_with_limits(&small, None, limits)
        .expect("small exact comparison fits the same budget");
}
#[test]
fn required_containers_are_constructed_but_optional_containers_stay_absent() {
    let schemas = schemas(json!({
        "type": "object",
        "properties": {
            "object": {"type": "object"},
            "array": {"type": "array"},
            "optional": {"type": "object"}
        },
        "required": ["object", "array"]
    }));
    let document = parse("!app.Config").unwrap();
    let resolved = schemas.resolve(&document).unwrap();
    assert_eq!(resolved.to_json(), json!({"object": {}, "array": []}));
}

#[test]
fn implicit_container_union_candidates_with_different_outputs_are_ambiguous() {
    let schemas = schemas(json!({
        "anyOf": [
            {
                "type": "object",
                "properties": {"left": {"type": "object"}},
                "required": ["left"]
            },
            {
                "type": "object",
                "properties": {"right": {"type": "array"}},
                "required": ["right"]
            }
        ]
    }));
    assert_eq!(
        schemas
            .resolve(&parse("!app.Config").unwrap())
            .unwrap_err()
            .kind,
        ResolveErrorKind::AmbiguousDefault
    );
}
#[test]
fn lean_container_completion_expands_references_and_conjunctions() {
    let schemas = schemas(json!({
        "type": "object",
        "properties": {"settings": {"$ref": "#/$defs/Settings"}},
        "required": ["settings"],
        "$defs": {
            "Settings": {
                "allOf": [
                    {"type": "object"},
                    {
                        "properties": {"entries": {"type": "array"}},
                        "required": ["entries"]
                    }
                ]
            }
        }
    }));
    assert_eq!(
        schemas
            .resolve(&parse("!app.Config").unwrap())
            .unwrap()
            .to_json(),
        json!({"settings": {"entries": []}})
    );
}

#[test]
fn supplied_objects_complete_required_containers_through_nested_union_branches() {
    let schemas = schemas(json!({
        "anyOf": [
            {
                "type": "object",
                "properties": {
                    "nested": {"anyOf": [
                        {"type": "array", "items": {"type": "string"}},
                        {"type": "integer"}
                    ]}
                },
                "required": ["nested"]
            },
            {"type": "string"}
        ]
    }));
    let document = parse("!app.Config {}").unwrap();
    assert_eq!(
        schemas.resolve(&document).unwrap().to_json(),
        json!({"nested": []})
    );
}

#[test]
fn union_completion_discovers_required_containers_inside_supplied_children() {
    for (schema, source, expected) in [
        (
            json!({"anyOf": [
                {"type": "object", "properties": {
                    "child": {"type": "object", "properties": {
                        "entries": {"type": "array", "items": {"type": "integer"}}
                    }, "required": ["entries"]}
                }, "required": ["child"]},
                false
            ]}),
            "{ child: {} }",
            json!({"child": {"entries": []}}),
        ),
        (
            json!({"anyOf": [
                {"type": "array", "items": {
                    "type": "object", "properties": {
                        "entries": {"type": "array", "items": {"type": "integer"}}
                    }, "required": ["entries"]
                }},
                false
            ]}),
            "[{}]",
            json!([{"entries": []}]),
        ),
    ] {
        let schemas = schemas(schema);
        let input = parse(source).unwrap();
        assert_eq!(
            schemas.resolve_as(&input, "app.Config").unwrap().to_json(),
            expected
        );
    }
}

#[test]
fn external_union_fallback_precedence_is_independent_of_schema_factoring() {
    for branch in [
        json!({"default": {"choice": 2}}),
        json!({"allOf": [{"default": {"choice": 2}}]}),
        json!({"$ref": "#/$defs/Fallback"}),
    ] {
        let schemas = schemas(json!({
            "$defs": {"Fallback": {"default": {"choice": 2}}},
            "default": {"choice": 1},
            "anyOf": [branch]
        }));
        let input = parse("{}").unwrap();
        assert_eq!(
            schemas.resolve_as(&input, "app.Config").unwrap().to_json(),
            json!({"choice": 1})
        );
    }
}

#[test]
fn union_container_completion_combines_conjunctive_requiredness_and_shape() {
    for schema in [
        json!({"anyOf": [
            {"allOf": [
                {"properties": {"entries": {"type": "array"}}},
                {"required": ["entries"]}
            ]},
            false
        ]}),
        json!({
            "$defs": {"Required": {"required": ["entries"]}},
            "anyOf": [
                {"allOf": [
                    {"properties": {"entries": {"type": "array"}}},
                    {"$ref": "#/$defs/Required"}
                ]},
                false
            ]
        }),
        json!({"allOf": [
            {"required": ["entries"]},
            {"anyOf": [
                {"properties": {"entries": {"type": "array"}}},
                false
            ]}
        ]}),
    ] {
        let schemas = schemas(schema);
        let input = parse("{}").unwrap();
        assert_eq!(
            schemas.resolve_as(&input, "app.Config").unwrap().to_json(),
            json!({"entries": []})
        );
    }
}

#[test]
fn common_completion_does_not_activate_independent_inert_unions() {
    let schemas = schemas(json!({
        "properties": {"entries": {"type": "array"}},
        "required": ["entries"],
        "default": {"entries": []},
        "allOf": (0..20).map(|_| json!({"anyOf": [
            {"type": "object"}, {"maxProperties": 3}
        ]})).collect::<Vec<_>>()
    }));
    let input = parse("{}").unwrap();
    assert_eq!(
        schemas.resolve_as(&input, "app.Config").unwrap().to_json(),
        json!({"entries": []})
    );
}

#[test]
fn union_discovery_includes_additional_properties_and_child_conjunctions() {
    let additional = json!({
        "type": "object",
        "additionalProperties": {
            "type": "object",
            "properties": {"entries": {"type": "array"}},
            "required": ["entries"]
        }
    });
    let split_child = json!({"allOf": [
        {"properties": {"child": {"properties": {"entries": {"type": "array"}}}}},
        {"properties": {"child": {"required": ["entries"]}}}
    ]});
    let input = parse("{ child: {} }").unwrap();
    for branch in [additional.clone(), split_child] {
        let schemas = schemas(json!({"anyOf": [branch, false]}));
        assert_eq!(
            schemas.resolve_as(&input, "app.Config").unwrap().to_json(),
            json!({"child": {"entries": []}})
        );
    }
    let schemas = schemas(json!({"anyOf": [additional, true]}));
    assert_eq!(
        schemas.resolve_as(&input, "app.Config").unwrap_err().kind,
        ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn branch_owned_work_survives_nested_unions_and_other_common_completion() {
    for (schema, source, expected) in [
        (
            json!({"anyOf": [
                {"anyOf": [
                    {"properties": {"entries": {"type": "array"}}, "required": ["entries"]},
                    false
                ]},
                false
            ]}),
            "!app.Config",
            json!({"entries": []}),
        ),
        (
            json!({"allOf": [
                {"properties": {"entries": {"type": "array"}}, "required": ["entries"]},
                {"anyOf": [
                    {"properties": {"extra": {"type": "array"}}, "required": ["extra"]},
                    false
                ]}
            ]}),
            "{}",
            json!({"entries": [], "extra": []}),
        ),
    ] {
        let schemas = schemas(schema);
        let input = parse(source).unwrap();
        assert_eq!(
            schemas.resolve_as(&input, "app.Config").unwrap().to_json(),
            expected
        );
    }
}
