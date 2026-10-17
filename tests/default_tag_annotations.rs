//! Native tag metadata is separate from default data and independent of consumers.

use daml::{Origin, ResolveErrorKind, SchemaErrorKind, SchemaModule, Schemas, parse};
use serde_json::{Value, json};

#[test]
fn default_tags_target_nested_values_without_reinterpreting_data_fields() {
    let module = SchemaModule::from_value(json!({"types": {
        "Host": {"schema": {"type": "object", "properties": {"payload": {
            "default": {"odd/key~": [7, {}], "__type__": "ordinary", "__fn__": "also ordinary"},
            "x-daml-default-tags": [
                {"path": "", "tag": "Payload"},
                {"path": "/odd~1key~0", "tag": "List"},
                {"path": "/odd~1key~0/0", "tag": "Count"},
                {"path": "/odd~1key~0/1", "tag": "Empty"}
            ]
        }}}},
        "Payload": {"schema": {"type": "object"}},
        "List": {"schema": {"type": "array"}},
        "Count": {"schema": {"type": "integer", "minimum": 1}},
        "Empty": {"schema": {"type": "object", "properties": {
            "enabled": {"type": "boolean", "default": true}
        }}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let input = parse("!Host").unwrap();
    let completed = schemas.resolve(&input).unwrap();
    let payload = completed.root().get("payload").unwrap();
    assert_eq!(payload.tag(), Some("Payload"));
    let list = payload.get("odd/key~").unwrap();
    assert_eq!(list.tag(), Some("List"));
    let items = list.items().unwrap();
    assert_eq!(items[0].tag(), Some("Count"));
    assert_eq!(items[1].tag(), Some("Empty"));
    let Origin::Default(location) = items[0].origin() else {
        panic!("default provenance")
    };
    assert_eq!(location.pointer, "/properties/payload/default");
    assert_eq!(
        completed.to_json(),
        json!({"payload": {
            "odd/key~": [7, {"enabled": true}],
            "__type__": "ordinary", "__fn__": "also ordinary"
        }})
    );
    assert!(input.get(input.root(), "payload").is_none());
}

#[test]
fn json_and_daml_modules_share_the_native_default_tag_contract() {
    let source = r#"{
        types: {
            Host: {
                schema: {
                    properties: {
                        count: {
                            default: 7
                            x-daml-default-tags: [{ path: ""
                                tag: "Count" }]
                        }
                    }
                }
            }
            Count: { schema: { type: "integer"
                minimum: 8 } }
        }
    }"#;
    let json_module = json!({"types": {
        "Host": {"schema": {"properties": {"count": {
            "default": 7, "x-daml-default-tags": [{"path": "", "tag": "Count"}]
        }}}},
        "Count": {"schema": {"type": "integer", "minimum": 8}}
    }});
    for module in [
        SchemaModule::from_daml(source).unwrap(),
        SchemaModule::from_value(json_module).unwrap(),
    ] {
        let schemas = Schemas::compose([module]).unwrap();
        let input = parse("!Host").unwrap();
        let error = schemas.resolve(&input).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert_eq!(error.path, "/count");
        let daml::ErrorOrigin::Default(location) = error.origin else {
            panic!("generated input")
        };
        assert_eq!(location.pointer, "/properties/count/default");
    }
}

#[test]
fn malformed_default_tag_annotations_are_rejected_before_composition() {
    let invalid = [
        json!({"x-daml-default-tags": [{"path": "", "tag": "T"}]}),
        json!({"default": {}, "x-daml-default-tags": []}),
        json!({"default": {}, "x-daml-default-tags": {"path": "", "tag": "T"}}),
        json!({"default": {}, "x-daml-default-tags": [{"path": "", "tag": ""}]}),
        json!({"default": {}, "x-daml-default-tags": [{"path": "", "tag": "T", "unknown": true}]}),
        json!({"default": {}, "x-daml-default-tags": [{"tag": "T"}]}),
        json!({"default": {}, "x-daml-default-tags": [{"path": ""}]}),
        json!({"default": {}, "x-daml-default-tags": [{"path": "missing", "tag": "T"}]}),
        json!({"default": {}, "x-daml-default-tags": [{"path": "/missing", "tag": "T"}]}),
        json!({"default": {"~2": 1}, "x-daml-default-tags": [{"path": "/~2", "tag": "T"}]}),
        json!({"default": [1], "x-daml-default-tags": [{"path": "/00", "tag": "T"}]}),
        json!({"default": {}, "x-daml-default-tags": [{"path": "", "tag": "T"}, {"path": "", "tag": "U"}]}),
    ];
    for schema in invalid {
        let error = SchemaModule::from_value(json!({"types": {"Host": {"schema": schema}}}))
            .err()
            .unwrap();
        assert_eq!(error.kind, SchemaErrorKind::InvalidSchema);
    }
}

#[test]
fn annotations_in_literal_data_are_not_schema_directives() {
    let data = json!({"x-daml-default-tags": "ordinary data", "__type__": "ordinary data"});
    let module = SchemaModule::from_value(json!({"types": {"Host": {"schema": {
        "properties": {"payload": {"default": data}}
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let input = parse("!Host").unwrap();
    let completed = schemas.resolve(&input).unwrap();
    let payload = completed.root().get("payload").unwrap();
    assert_eq!(payload.tag(), None);
    assert_eq!(payload.to_json(), data);
}

#[test]
fn generated_unknown_tags_are_rejected_instead_of_returning_unchecked_data() {
    let module = SchemaModule::from_value(json!({"types": {"Host": {"schema": {
        "properties": {"payload": {"default": Value::Null,
            "x-daml-default-tags": [{"path": "", "tag": "Absent"}]}}
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let input = parse("!Host").unwrap();
    let error = schemas.resolve(&input).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::UnknownTag);
    assert_eq!(error.path, "/payload");
}

#[test]
fn supplied_objects_override_annotated_scalar_and_sequence_defaults() {
    let module = SchemaModule::from_value(json!({"types": {
        "Host": {"schema": {"properties": {
            "scalar": {"default": 7, "x-daml-default-tags": [{"path": "", "tag": "Count"}]},
            "sequence": {"default": [1], "x-daml-default-tags": [{"path": "", "tag": "List"}]}
        }}},
        "Count": {"schema": {"type": "integer"}},
        "List": {"schema": {"type": "array"}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let supplied = parse("!Host { scalar: {}\n sequence: {} }").unwrap();
    let completed = schemas.resolve(&supplied).unwrap();
    assert_eq!(completed.to_json(), json!({"scalar": {}, "sequence": {}}));
    assert_eq!(completed.root().get("scalar").unwrap().tag(), None);
    assert_eq!(completed.root().get("sequence").unwrap().tag(), None);
    let missing = parse("!Host").unwrap();
    let completed = schemas.resolve(&missing).unwrap();
    assert_eq!(completed.root().get("scalar").unwrap().tag(), Some("Count"));
    assert_eq!(
        completed.root().get("sequence").unwrap().tag(),
        Some("List")
    );
}

#[test]
fn supplied_objects_override_annotated_nonobject_parent_presets() {
    let module = SchemaModule::from_value(json!({"types": {
        "Host": {"schema": {
            "default": {"scalar": 7, "sequence": [1]},
            "x-daml-default-tags": [
                {"path": "/scalar", "tag": "Count"},
                {"path": "/sequence", "tag": "List"}
            ]
        }},
        "Count": {"schema": {"type": "integer"}},
        "List": {"schema": {"type": "array"}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let supplied = parse("!Host { scalar: {}\n sequence: {} }").unwrap();
    let completed = schemas.resolve(&supplied).unwrap();
    assert_eq!(completed.to_json(), json!({"scalar": {}, "sequence": {}}));
    assert_eq!(completed.root().get("scalar").unwrap().tag(), None);
    assert_eq!(completed.root().get("sequence").unwrap().tag(), None);
}
