//! Declarative modules, reference resolution, and composition failures.

use daml::{SchemaErrorKind, SchemaModule, Schemas, parse};
use serde_json::json;

fn config_schema() -> serde_json::Value {
    json!({
        "types": {
            "app.Config": {
                "schema": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}},
                    "required": ["name"]
                }
            }
        }
    })
}

#[test]
fn json_daml_and_value_modules_have_equivalent_consumer_behavior() {
    let json_module = SchemaModule::from_json(
        r#"{"types":{"app.Config":{"schema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}}}"#,
    ).expect("JSON module");
    let daml_module = SchemaModule::from_daml(
        r#"{
          types: {
            app.Config: {
              schema: {
                type: "object"
                properties: { name: { type: "string" } }
                required: ["name"]
              }
            }
          }
        }"#,
    )
    .expect("DAML module");
    let value_module = SchemaModule::from_value(config_schema()).expect("value module");
    let schemas = Schemas::compose([json_module, daml_module, value_module]);
    assert_eq!(
        schemas
            .err()
            .expect("duplicate bindings must be rejected")
            .kind,
        SchemaErrorKind::DuplicateName
    );

    let json_schemas = Schemas::compose([SchemaModule::from_json(
        r#"{"types":{"app.Config":{"schema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}}}"#,
    ).expect("JSON module")]).expect("compose JSON");
    let daml_schemas = Schemas::compose([SchemaModule::from_daml(
        r#"{
          types: {
            app.Config: {
              schema: {
                type: "object"
                properties: { name: { type: "string" } }
                required: ["name"]
              }
            }
          }
        }"#,
    )
    .expect("DAML module")])
    .expect("compose DAML");
    let json = json_schemas
        .resolve(&parse("!app.Config { name: \"Ada\" }").expect("JSON-shaped data"))
        .expect("JSON-derived schema accepts data")
        .to_json();
    let daml = daml_schemas
        .resolve(&parse("!app.Config { name: \"Ada\" }").expect("DAML-shaped data"))
        .expect("DAML-derived schema accepts data")
        .to_json();
    assert_eq!(json, daml);
}

#[test]
fn module_and_identifier_collisions_are_rejected() {
    let first = SchemaModule::from_value(config_schema()).expect("first");
    let second = SchemaModule::from_value(config_schema()).expect("second");
    assert_eq!(
        Schemas::compose([first, second])
            .err()
            .expect("duplicate type")
            .kind,
        SchemaErrorKind::DuplicateName
    );

    let explicit_ids = SchemaModule::from_json(r#"{"types":{"a":{"schema":{"$id":"urn:same","type":"string"}},"b":{"schema":{"$id":"urn:same","type":"number"}}}}"#).expect("module parses");
    assert_eq!(
        Schemas::compose([explicit_ids])
            .err()
            .expect("duplicate ID")
            .kind,
        SchemaErrorKind::DuplicateName
    );
}

#[test]
fn unresolved_offline_resources_and_reference_cycles_are_rejected() {
    let missing = SchemaModule::from_json(
        r#"{"types":{"app.Config":{"schema":{"$ref":"https://schemas.invalid/shared"}}}}"#,
    )
    .expect("module parses");
    assert_eq!(
        Schemas::compose([missing])
            .err()
            .expect("offline missing ref")
            .kind,
        SchemaErrorKind::UnknownReference
    );

    let supplied = SchemaModule::from_value(json!({
        "types": {"app.Config": {"schema": {"$ref": "https://schemas.invalid/shared"}}},
        "resources": [{"uri":"https://schemas.invalid/shared","schema":{"type":"object"}}]
    }))
    .expect("module parses");
    assert!(Schemas::compose([supplied]).is_ok());

    let cycle = SchemaModule::from_json(r##"{"types":{"app.Config":{"schema":{"$ref":"#/$defs/self","$defs":{"self":{"$ref":"#"}}}}}}"##).expect("module parses");
    assert_eq!(
        Schemas::compose([cycle]).err().expect("cycle").kind,
        SchemaErrorKind::ReferenceCycle
    );
}
#[test]
fn reference_cycles_through_schema_child_positions_are_rejected_during_composition() {
    for schema in [
        json!({"properties":{"nested":{"$ref":"#"}}}),
        json!({"patternProperties":{"^nested":{"$ref":"#"}}}),
        json!({"allOf":[{"$ref":"#"}]}),
        json!({"anyOf":[{"$ref":"#"}]}),
        json!({"oneOf":[{"$ref":"#"}]}),
        json!({"prefixItems":[{"$ref":"#"}]}),
        json!({"items":{"$ref":"#"}}),
        json!({"additionalProperties":{"$ref":"#"}}),
        json!({"not":{"$ref":"#"}}),
        json!({"contains":{"$ref":"#"}}),
        json!({"propertyNames":{"$ref":"#"}}),
        json!({"if":{"$ref":"#"}}),
        json!({"then":{"$ref":"#"}}),
        json!({"else":{"$ref":"#"}}),
        json!({"unevaluatedProperties":{"$ref":"#"}}),
        json!({"unevaluatedItems":{"$ref":"#"}}),
        json!({"contentSchema":{"$ref":"#"}}),
        json!({"dependentSchemas":{"nested":{"$ref":"#"}}}),
        json!({"$defs":{"nested":{"$ref":"#"}}}),
        json!({"definitions":{"nested":{"$ref":"#"}}}),
    ] {
        let module = SchemaModule::from_value(json!({
            "types": {"app.Cycle": {"schema": schema}}
        }))
        .expect("cycle schema is structurally admitted");
        assert_eq!(
            Schemas::compose([module])
                .err()
                .expect("cycle rejected")
                .kind,
            SchemaErrorKind::ReferenceCycle
        );
    }
}

#[test]
fn discriminator_named_fields_remain_native_instance_data() {
    let module = SchemaModule::from_value(json!({
        "types": {"app.Record": {"schema": {
            "type": "object",
            "properties": {
                "__type__": {"type": "string"},
                "__fn__": {"type": "string"}
            },
            "required": ["__type__", "__fn__"]
        }}}
    }))
    .expect("native module");
    let schemas = Schemas::compose([module]).expect("compose");
    let input =
        parse("!app.Record { __type__: \"ordinary data\"\n __fn__: \"also ordinary data\" }")
            .expect("tagged data");
    let resolved = schemas.resolve(&input).expect("resolve native data");
    assert_eq!(
        resolved.to_json(),
        json!({"__type__":"ordinary data","__fn__":"also ordinary data"})
    );
}

#[test]
fn exact_numeric_limits_are_checked_during_loading() {
    let excessive = format!(
        r#"{{"types":{{"app.Number":{{"schema":{{"maximum":{}}}}}}}}}"#,
        "1".repeat(513)
    );
    let error = SchemaModule::from_json(&excessive).expect("JSON parses");
    assert_eq!(
        Schemas::compose([error]).err().expect("numeric bound").kind,
        SchemaErrorKind::LimitExceeded
    );
}

#[test]
fn relative_references_with_nested_ids_and_anchors_resolve_offline() {
    let module = SchemaModule::from_json(
        r##"{"types":{"app.Config":{"schema":{"$id":"https://schemas.invalid/root.json","type":"object","properties":{"value":{"$ref":"nested/#text"}},"required":["value"],"$defs":{"target":{"$id":"nested/","$anchor":"text","type":"string"}}}}}}"##,
    ).expect("module parses");
    let schemas = Schemas::compose([module]).expect("relative nested resource reference resolves");
    assert!(
        schemas
            .resolve(&parse("!app.Config { value: \"ok\" }").expect("input"))
            .is_ok()
    );
    assert!(
        schemas
            .resolve(&parse("!app.Config { value: 3 }").expect("input"))
            .is_err()
    );
}

#[test]
fn references_to_missing_nested_anchors_are_rejected() {
    let module = SchemaModule::from_json(
        r##"{"types":{"app.Config":{"schema":{"$id":"https://schemas.invalid/root.json","$ref":"nested/#missing","$defs":{"target":{"$id":"nested/","type":"string"}}}}}}"##,
    ).expect("module parses");
    assert_eq!(
        Schemas::compose([module])
            .err()
            .expect("unresolved anchor")
            .kind,
        SchemaErrorKind::UnknownReference
    );
}

#[test]
fn nested_identifier_and_anchor_collisions_are_rejected() {
    let duplicate_ids = SchemaModule::from_json(
        r##"{"types":{"app.Config":{"schema":{"$id":"https://schemas.invalid/root.json","$defs":{"a":{"$id":"nested/","type":"string"},"b":{"$id":"nested/","type":"number"}}}}}}"##,
    ).expect("module parses");
    assert_eq!(
        Schemas::compose([duplicate_ids])
            .err()
            .expect("nested ID collision")
            .kind,
        SchemaErrorKind::DuplicateName
    );

    let duplicate_anchors = SchemaModule::from_json(
        r##"{"types":{"app.Config":{"schema":{"$id":"https://schemas.invalid/root.json","$defs":{"a":{"$anchor":"shared","type":"string"},"b":{"$anchor":"shared","type":"number"}}}}}}"##,
    ).expect("module parses");
    assert_eq!(
        Schemas::compose([duplicate_anchors])
            .err()
            .expect("anchor collision")
            .kind,
        SchemaErrorKind::DuplicateName
    );
}

#[test]
fn unsupported_defaults_are_found_through_references_and_dag_work_is_bounded() {
    let through_ref = SchemaModule::from_json(
        r##"{"types":{"app.Bad":{"schema":{"not":{"$ref":"#/$defs/target"},"$defs":{"target":{"properties":{"x":{"default":1}}}}}}}}"##,
    ).expect("module parses");
    assert_eq!(
        Schemas::compose([through_ref])
            .err()
            .expect("referenced unsupported default")
            .kind,
        SchemaErrorKind::UnsupportedDefaults
    );

    let mut definitions = serde_json::Map::new();
    definitions.insert("d0".into(), json!({"type":"string"}));
    for index in 1..18 {
        let previous = format!("#/$defs/d{}", index - 1);
        definitions.insert(
            format!("d{index}"),
            json!({"allOf":[{"$ref":previous},{"$ref":previous}]}),
        );
    }
    let dag = SchemaModule::from_value(json!({
        "types":{"app.Dag":{"schema":{"$ref":"#/$defs/d17","$defs":definitions}}}
    }))
    .expect("module parses");
    assert_eq!(
        Schemas::compose([dag])
            .err()
            .expect("expanded DAG work bound")
            .kind,
        SchemaErrorKind::LimitExceeded
    );
}

#[test]
fn ordinary_type_bindings_resolve_only_explicit_resources() {
    let imported = SchemaModule::from_value(json!({
        "types": {"app.Imported": {"schema": {"$ref": "https://schemas.example.invalid/shared/value"}}},
        "resources": [{
            "uri": "https://schemas.example.invalid/shared/value",
            "schema": {"type": "string"}
        }]
    }))
    .expect("native resource binding");
    let schemas = Schemas::compose([imported]).expect("provided resource resolves offline");
    assert!(
        schemas
            .resolve(&parse("!app.Imported \"ok\"").expect("input"))
            .is_ok()
    );
    assert!(
        schemas
            .resolve(&parse("!app.Imported 2").expect("input"))
            .is_err()
    );
}

#[test]
fn ordinary_bindings_can_reference_one_canonical_schema_resource() {
    let module = SchemaModule::from_value(json!({
        "types": {
            "app.Short": {"schema": {"$ref": "https://schemas.example.invalid/canonical"}},
            "app.Canonical": {"schema": {"$ref": "https://schemas.example.invalid/canonical"}}
        },
        "resources": [{
            "uri": "https://schemas.example.invalid/canonical",
            "schema": {
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": false
            }
        }]
    }))
    .expect("canonical resource module");
    let schemas = Schemas::compose([module]).expect("compose canonical resource");
    for tag in ["app.Short", "app.Canonical"] {
        let valid = parse(format!("!{tag} {{ value: \"ok\" }}")).expect("valid input");
        assert!(schemas.resolve(&valid).is_ok());
        let invalid = parse(format!("!{tag} {{ value: 3 }}")).expect("invalid input");
        assert!(schemas.resolve(&invalid).is_err());
    }
}

#[test]
fn additional_unsupported_default_applicators_are_rejected() {
    for schema in [
        json!({"dependentSchemas":{"trigger":{"properties":{"x":{"default":1}}}}}),
        json!({"contentSchema":{"properties":{"x":{"default":1}}}}),
        json!({
            "patternProperties":{"^x":{"type":"string"}},
            "additionalProperties":{"properties":{"fallback":{"default":1}}}
        }),
    ] {
        let module = SchemaModule::from_value(json!({"types":{"app.Bad":{"schema":schema}}}))
            .expect("modern schema loads");
        assert_eq!(
            Schemas::compose([module])
                .err()
                .expect("unsupported defaults")
                .kind,
            SchemaErrorKind::UnsupportedDefaults
        );
    }
    let legacy = SchemaModule::from_value(json!({"types":{"app.Bad":{"schema":{
        "items": false, "additionalItems": {"default": 1}
    }}}}))
    .err()
    .expect("legacy applicator rejected during admission");
    assert_eq!(legacy.kind, SchemaErrorKind::InvalidSchema);
}

#[test]
fn duplicate_json_members_are_rejected_at_every_depth() {
    let duplicate_binding = SchemaModule::from_json(
        r#"{"types":{"app.Config":{"schema":{"type":"string"}},"app.Config":{"schema":{"type":"number"}}}}"#,
    ).err().expect("duplicate type member");
    assert_eq!(duplicate_binding.kind, SchemaErrorKind::Parse);

    let escaped_duplicate = SchemaModule::from_json(
        r#"{"types":{"app.Config":{"schema":{"$defs":{"a/b":{"type":"string"},"a\u002fb":{"type":"number"}}}}}}"#,
    ).err().expect("equivalent decoded nested keys");
    assert_eq!(escaped_duplicate.kind, SchemaErrorKind::Parse);
}

#[test]
fn repeated_reference_work_is_charged_before_runtime_validation() {
    let mut definitions = serde_json::Map::new();
    definitions.insert("X0".into(), json!({"type": "integer"}));
    for level in 1..=6 {
        let reference = format!("#/$defs/X{}", level - 1);
        definitions.insert(
            format!("X{level}"),
            json!({"allOf": [
                {"$ref": reference}, {"$ref": reference}
            ]}),
        );
    }
    let module = SchemaModule::from_value(json!({"types": {"Count": {"schema": {
        "$ref": "#/$defs/X6", "$defs": definitions
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let document = parse("!Count 1").unwrap();
    let limits = daml::ResolutionLimits {
        max_steps: 300,
        ..Default::default()
    };
    assert_eq!(
        schemas
            .resolve_with_limits(&document, None, limits)
            .unwrap_err()
            .kind,
        daml::ResolveErrorKind::LimitExceeded
    );
}

#[test]
fn relative_identifiers_are_compared_in_their_resolved_scope() {
    let independent = SchemaModule::from_value(json!({"resources": [
        {"uri": "https://example.invalid/a/root", "schema": {"$defs": {"Inner": {"$id": "inner", "type": "integer"}}}},
        {"uri": "https://example.invalid/b/root", "schema": {"$defs": {"Inner": {"$id": "inner", "type": "string"}}}}
    ]})).unwrap();
    Schemas::compose([independent]).unwrap();
    let collision = SchemaModule::from_value(json!({"resources": [
        {"uri": "https://example.invalid/a/root", "schema": {"$defs": {"Inner": {"$id": "../shared"}}}},
        {"uri": "https://example.invalid/shared", "schema": {}}
    ]})).unwrap();
    assert_eq!(
        Schemas::compose([collision]).err().unwrap().kind,
        SchemaErrorKind::DuplicateName
    );
}

#[test]
fn diagnostic_constraint_location_tracks_the_referenced_resource() {
    let module = SchemaModule::from_value(json!({
        "types": {"Config": {"schema": {"type": "object", "properties": {
            "port": {"$ref": "https://example.invalid/port"}
        }}}},
        "resources": [{"uri": "https://example.invalid/port", "schema": {"type": "integer", "minimum": 1}}]
    })).unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let document = parse("!Config { port: 0 }").unwrap();
    let error = schemas.resolve(&document).unwrap_err();
    assert_eq!(error.path, "/port");
    let location = error.schema.unwrap();
    assert_eq!(location.resource, "https://example.invalid/port");
    assert_eq!(location.pointer, "/minimum");
}

#[test]
fn definition_defaults_are_contained_but_not_applied_until_referenced() {
    let conjunctions = (0..20)
        .map(|index| {
            json!({"anyOf": [
                {"type":"string","$defs":{"unused":{"not":{"default":index}}}},
                {"type":"string","$defs":{"unused":{"not":{"default":index}}}}
            ]})
        })
        .collect::<Vec<_>>();
    let unused = SchemaModule::from_value(json!({
        "types":{"app.Safe":{"schema":{"type":"string","allOf":conjunctions}}}
    }))
    .expect("unused definition defaults are accepted");
    let schemas =
        Schemas::compose([unused]).expect("unreachable defaults do not affect applicability");
    let input = parse("!app.Safe \"ok\"").unwrap();
    schemas
        .resolve(&input)
        .expect("unused defaults do not enumerate union completions");

    let referenced = SchemaModule::from_value(json!({
        "types":{"app.Bad":{"schema":{
            "not":{"$ref":"#/$defs/target"},
            "$defs":{"target":{"properties":{"x":{"default":1}}}}
        }}}
    }))
    .expect("module parses");
    assert_eq!(
        Schemas::compose([referenced])
            .err()
            .expect("referenced default under not")
            .kind,
        SchemaErrorKind::UnsupportedDefaults
    );
}

#[test]
fn legacy_keywords_and_non_2020_dialects_are_rejected_at_schema_nodes() {
    for schema in [
        json!({"dependencies":{"trigger":["value"]}}),
        json!({"additionalItems":false}),
        json!({"items":[{"type":"string"}]}),
        json!({"$schema":"http://json-schema.org/draft-07/schema#"}),
        json!({"$schema":"https://json-schema.org/draft/2019-09/schema"}),
        json!({"properties":{"nested":{"$schema":"https://json-schema.org/draft/2019-09/schema"}}}),
        json!({"properties":{"nested":{"dependencies":{"x":["y"]}}}}),
        json!({"$defs":{"nested":{"items":[false]}}}),
    ] {
        let module = SchemaModule::from_value(json!({
            "types":{"app.Bad":{"schema":schema}}
        }))
        .err()
        .expect("unsupported schema profile");
        assert_eq!(module.kind, SchemaErrorKind::InvalidSchema);
    }

    let resource = json!({"resources":[{"uri":"https://example.invalid/schema","schema":{
        "$defs":{"nested":{"dependencies":{"x":["y"]}}}
    }}]});
    let error = SchemaModule::from_value(resource)
        .err()
        .expect("profile checked in resources");
    assert_eq!(error.kind, SchemaErrorKind::InvalidSchema);
    assert_eq!(
        SchemaModule::from_json(
            r#"{"types":{"Bad":{"schema":{"properties":{"nested":{"dependencies":{"x":["y"]}}}}}}}"#,
        )
        .err()
        .expect("JSON loader checks nested legacy keywords")
        .kind,
        SchemaErrorKind::InvalidSchema
    );
}

#[test]
fn modern_dependency_keywords_remain_supported_and_data_is_not_walked_as_schema() {
    let module = SchemaModule::from_value(json!({
        "types":{"app.Dependent":{"schema":{
            "type":"object",
            "properties":{
                "credit_card":{"type":"string"},
                "billing_address":{"type":"string"},
                "kind":{"type":"object"}
            },
            "dependentRequired":{"credit_card":["billing_address"]},
            "dependentSchemas":{"kind":{"required":["billing_address"]}},
            "examples":[{"$schema":"http://json-schema.org/draft-07/schema#","dependencies":{}}]
        }}}
    }))
    .expect("modern dependencies and schema-looking data are accepted");
    let schemas = Schemas::compose([module]).expect("compose");
    assert!(
        schemas
            .resolve(&parse("!app.Dependent { kind: { } }").unwrap())
            .is_err()
    );
    schemas
        .resolve(&parse("!app.Dependent { kind: { }\n billing_address: \"here\" }").unwrap())
        .expect("modern dependentSchemas accepts satisfied input");
    assert!(
        schemas
            .resolve(&parse("!app.Dependent { credit_card: \"123\" }").unwrap())
            .is_err()
    );
}

#[test]
fn schema_looking_default_const_enum_and_example_data_remain_inert() {
    let payload = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "dependencies": {"legacy": ["data"]},
        "x-daml-tag": "ordinary data"
    });
    let module = SchemaModule::from_value(json!({
        "types": {"app.Data": {"schema": {
            "type": "object",
            "default": {"dependencies": {"legacy": []}},
            "properties": {
                "payload": {
                    "const": payload,
                    "default": payload,
                    "enum": [payload]
                }
            },
            "examples": [payload]
        }}}
    }))
    .expect("schema-looking annotation values are ordinary data");
    let schemas = Schemas::compose([module]).expect("annotations are not schemas");
    let input = parse("!app.Data").expect("input");
    let completed = schemas.resolve(&input).expect("literal data resolves");
    assert_eq!(
        completed.to_json(),
        json!({
            "payload": payload, "dependencies": {"legacy": []}
        })
    );
}

#[test]
fn obsolete_schema_keywords_and_declarations_reject_at_admission() {
    for schema in [
        json!({"x-daml-tag": "app.Value"}),
        json!({"$defs": {"nested": {"x-daml-slot": "app.Shape"}}}),
        json!({"properties": {"value": {"x-daml-tag": "app.Value"}}}),
    ] {
        let error = SchemaModule::from_value(json!({
            "types": {"app.Bad": {"schema": schema}}
        }))
        .err()
        .expect("obsolete keyword is rejected while loading");
        assert_eq!(error.kind, SchemaErrorKind::InvalidSchema);
    }
    for value in [
        json!({"contracts": {"app.Shape": {"type": "object"}}}),
        json!({"types": {"app.Value": {
            "schema": {"type": "string"}, "implements": ["app.Shape"]
        }}}),
    ] {
        assert!(SchemaModule::from_value(value).is_err());
    }

    let error = SchemaModule::from_value(json!({
        "resources": [{
            "uri": "https://example.invalid/schema",
            "schema": {"$defs": {"nested": {"x-daml-slot": "app.Shape"}}}
        }]
    }))
    .err()
    .expect("resources reject obsolete keywords too");
    assert_eq!(error.kind, SchemaErrorKind::InvalidSchema);

    let literal = SchemaModule::from_value(json!({
        "types": {"app.Data": {"schema": {
            "type": "object",
            "properties": {"x-daml-slot": {"const": "literal"}},
            "default": {"x-daml-tag": "ordinary data"}
        }}}
    }))
    .expect("literal field names and default data are not schema keywords");
    Schemas::compose([literal]).expect("literal data is admitted");
}
