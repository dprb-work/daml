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
fn tag_keywords_import_aliases_and_unsupported_defaults_are_enforced() {
    let native = SchemaModule::from_json(r#"{"types":{"app.Config":{"schema":{"type":"object","x-daml-tag":"app.Config","properties":{"name":{"type":"string"}}}}}}"#).expect("native module");
    let schemas = Schemas::compose([native]).expect("compose native");
    assert!(
        schemas
            .resolve(&parse("!app.Config { name: \"ok\" }").expect("input"))
            .is_ok()
    );
    assert!(
        schemas
            .resolve_as(&parse("{ name: \"ok\" }").expect("input"), "app.Config")
            .is_err()
    );
    assert!(
        schemas
            .resolve(&parse("!app.Config { name: 7 }").expect("input"))
            .is_err()
    );

    let imported = SchemaModule::from_olaf_json("old.Type", r#"{"type":"object","properties":{"__type__":{"enum":["old.Type"]},"value":{"type":"integer"}},"required":["__type__","value"]}"#).expect("explicit Olaf adapter");
    let imported = Schemas::compose([imported]).expect("compose imported");
    assert!(
        imported
            .resolve(&parse("!old.Type { value: 2 }").expect("input"))
            .is_ok()
    );
    let imported_default = SchemaModule::from_olaf_json(
        "old.Defaulted",
        r#"{"type":"object","default":{"__type__":"old.Defaulted","value":2},"properties":{"__type__":{"const":"old.Defaulted"},"value":{"type":"integer"}},"required":["__type__","value"]}"#,
    ).expect("Olaf default module");
    let imported_default = Schemas::compose([imported_default]).expect("compose default importer");
    let empty = parse("{}").expect("empty object");
    let resolved = imported_default
        .resolve_as(&empty, "old.Defaulted")
        .expect("legacy default identity");
    assert_eq!(resolved.root().tag(), Some("old.Defaulted"));
    assert!(resolved.root().get("__type__").is_none());

    let unsupported = SchemaModule::from_json(
        r#"{"types":{"app.Bad":{"schema":{"not":{"properties":{"x":{"default":1}}}}}}}"#,
    )
    .expect("module parses");
    assert_eq!(
        Schemas::compose([unsupported])
            .err()
            .expect("unsupported default position")
            .kind,
        SchemaErrorKind::UnsupportedDefaults
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
fn explicit_olaf_references_resolve_only_supplied_resources() {
    let imported =
        SchemaModule::from_olaf_json("app.Imported", r#"{"$ref":"@/shared/value.json"}"#)
            .expect("Olaf import");
    let resource = SchemaModule::from_value(json!({
        "resources":[{"uri":"@/shared/value.json","schema":{"type":"string"}}]
    }))
    .expect("explicit resource");
    let schemas =
        Schemas::compose([imported, resource]).expect("provided Olaf resource resolves offline");
    assert!(
        schemas
            .resolve(&parse("!app.Imported \"ok\"").expect("input"))
            .is_ok()
    );
}

#[test]
fn additional_unsupported_default_applicators_are_rejected() {
    for schema in [
        json!({"dependentSchemas":{"trigger":{"properties":{"x":{"default":1}}}}}),
        json!({"items":false,"additionalItems":{"default":1}}),
        json!({"contentSchema":{"properties":{"x":{"default":1}}}}),
    ] {
        let module = SchemaModule::from_value(json!({"types":{"app.Bad":{"schema":schema}}}))
            .expect("module parses");
        assert_eq!(
            Schemas::compose([module])
                .err()
                .expect("unsupported defaults")
                .kind,
            SchemaErrorKind::UnsupportedDefaults
        );
    }
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

    let duplicate_olaf =
        SchemaModule::from_olaf_json("app.Imported", r#"{"type":"string","type":"number"}"#)
            .err()
            .expect("duplicate Olaf keyword");
    assert_eq!(duplicate_olaf.kind, SchemaErrorKind::Parse);
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
