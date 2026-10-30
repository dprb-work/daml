//! Public registry contracts for recursive normalization, locations, and traversal.

use daml::{
    ResolveErrorKind, SchemaErrorKind, SchemaModule, Schemas, SemanticNode, SemanticValue, parse,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn nested_resource_recursion_preserves_relative_anchors_siblings_and_aliases() {
    let module = SchemaModule::from_value(json!({
        "types": {
            "app.Tree": {"schema": {"$ref": "https://schemas.invalid/nested/node.json#node"}},
            "app.TreeAlias": {"schema": {"$ref": "https://schemas.invalid/nested/node.json#node"}}
        },
        "resources": [{"uri": "https://schemas.invalid/root.json", "schema": {
            "$id": "https://schemas.invalid/root.json",
            "$defs": {
                "node": {
                    "$id": "nested/node.json",
                    "$anchor": "node",
                    "type": "object",
                    "properties": {
                        "value": {"$ref": "types/count.json#count", "minimum": 3},
                        "next": {"$ref": "#node"}
                    },
                    "required": ["value"],
                    "additionalProperties": false
                },
                "count": {
                    "$id": "nested/types/count.json",
                    "$anchor": "count",
                    "type": "integer",
                    "minimum": 1
                }
            }
        }}]
    }))
    .expect("schema module");
    let schemas =
        Schemas::compose([module]).expect("nested resources and recursive anchor compose");

    let valid = parse("!app.TreeAlias {value: 3\nnext: {value: 4}} ").expect("finite tree");
    assert_eq!(
        schemas
            .resolve(&valid)
            .expect("alias resolves recursively")
            .to_json(),
        json!({"value": 3, "next": {"value": 4}})
    );

    let too_small = parse("!app.Tree {value: 3\nnext: {value: 2}} ").expect("finite tree");
    let error = schemas
        .resolve(&too_small)
        .expect_err("reference sibling minimum applies recursively");
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/next/value");
    let location = error.schema.expect("constraint source location");
    assert_eq!(
        location.resource,
        "https://schemas.invalid/nested/node.json"
    );
    assert_eq!(location.pointer, "/properties/value/minimum");
}

#[test]
fn recursive_validation_keeps_literal_numeric_and_escaped_instance_keys_public() {
    let module = SchemaModule::from_value(json!({"types": {"app.Payload": {"schema": {
        "$id": "urn:recursive-registry:payload",
        "type": "object",
        "properties": {
            "0": {"$ref": "#/$defs/record"}
        },
        "$defs": {"record": {
            "type": "object",
            "properties": {"a/b": {"type": "integer", "minimum": 5}}
        }}
    }}}}))
    .expect("schema module");
    let schemas = Schemas::compose([module]).expect("schema registry");
    let scalar = SemanticNode {
        tag: None,
        value: SemanticValue::Number(1.into()),
    };
    let record = SemanticNode {
        tag: None,
        value: SemanticValue::Mapping(BTreeMap::from([("a/b".to_owned(), scalar)])),
    };
    let input = SemanticNode {
        tag: None,
        value: SemanticValue::Mapping(BTreeMap::from([("0".to_owned(), record)])),
    };
    let error = schemas
        .validate_semantic(&input, Some("app.Payload"))
        .expect_err("semantic validation rejects the nested value");

    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/0/a~1b");
    let location = error.schema.expect("authored constraint location");
    assert_eq!(location.resource, "urn:recursive-registry:payload");
    assert_eq!(location.pointer, "/$defs/record/properties/a~1b/minimum");
}

#[test]
fn recursive_unique_items_validate_finite_nested_arrays() {
    let module = SchemaModule::from_value(json!({"types": {"app.NestedList": {"schema": {
        "type": "array",
        "uniqueItems": true,
        "items": {"anyOf": [{"type": "integer"}, {"$ref": "#"}]}
    }}}}))
    .expect("schema module");
    let schemas = Schemas::compose([module]).expect("structurally recursive uniqueness schema");

    let distinct = parse("[[1]\n[2]]").expect("finite arrays");
    schemas
        .validate_semantic(
            &SemanticNode::from_document(&distinct),
            Some("app.NestedList"),
        )
        .expect("distinct nested arrays pass uniqueness");

    let duplicate = parse("[[1]\n[1]]").expect("finite arrays");
    let error = schemas
        .validate_semantic(
            &SemanticNode::from_document(&duplicate),
            Some("app.NestedList"),
        )
        .expect_err("equal nested arrays violate uniqueness");
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
}

#[test]
fn unsupported_applicator_defaults_are_found_after_recursive_reference_edges() {
    let module = SchemaModule::from_value(json!({"types": {"app.Bad": {"schema": {
        "type": "object",
        "not": {"$ref": "#/$defs/target"},
        "$defs": {"target": {
            "type": "object",
            "properties": {
                "a-loop": {"$ref": "#/$defs/target"},
                "z-value": {"default": 1}
            }
        }}
    }}}}))
    .expect("schema module");
    let error = Schemas::compose([module])
        .err()
        .expect("defaults reachable only under not remain unsupported");
    assert_eq!(error.kind, SchemaErrorKind::UnsupportedDefaults);
}

#[test]
fn unevaluated_helper_compilation_bounds_repeated_targets_in_mixed_registries() {
    for keyword in ["unevaluatedProperties", "unevaluatedItems"] {
        let mut definitions = serde_json::Map::new();
        definitions.insert(
            "d0".into(),
            json!({"properties": {"a": true}, "prefixItems": [true]}),
        );
        for level in 1..=16 {
            let reference = format!("#/$defs/d{}", level - 1);
            definitions.insert(
                format!("d{level}"),
                json!({"allOf": [{"$ref": reference}, {"$ref": reference}]}),
            );
        }
        let mut guarded = json!({"$defs": definitions, "$ref": "#/$defs/d16"});
        guarded
            .as_object_mut()
            .unwrap()
            .insert(keyword.into(), json!(false));
        let module = SchemaModule::from_value(json!({"types": {
            "app.Recursive": {"schema": {
                "type": "object", "properties": {"next": {"$ref": "#"}}
            }},
            "app.Guarded": {"schema": guarded}
        }}))
        .unwrap();
        let error = Schemas::compose([module])
            .err()
            .expect("expanded annotation helpers must fit compilation work bounds");
        assert_eq!(error.kind, SchemaErrorKind::LimitExceeded, "{keyword}");
    }
}

#[test]
fn structural_recursion_retains_unevaluated_constraints_at_finite_descendants() {
    for (keyword, mut schema, valid, invalid) in [
        (
            "unevaluatedProperties",
            json!({
                "type": "object",
                "properties": {"value": {"type": "integer"}, "next": {"$ref": "#"}},
                "required": ["value"]
            }),
            "{\nvalue: 1\nnext: {value: 2}\n}",
            "{\nvalue: 1\nnext: {\nvalue: 2\nextra: 3\n}\n}",
        ),
        (
            "unevaluatedItems",
            json!({
                "type": "array",
                "prefixItems": [{"type": "integer"}, {"$ref": "#"}]
            }),
            "[1\n[2\n[]]]",
            "[1\n[2\n[]\n3]]",
        ),
    ] {
        schema
            .as_object_mut()
            .unwrap()
            .insert(keyword.into(), json!(false));
        let module = SchemaModule::from_value(json!({
            "types": {"app.Tree": {"schema": schema}}
        }))
        .unwrap();
        let schemas = Schemas::compose([module]).unwrap();
        let supplied = parse(valid).unwrap();
        schemas
            .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Tree"))
            .unwrap();
        let supplied = parse(invalid).unwrap();
        let error = schemas
            .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Tree"))
            .unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert_eq!(error.schema.unwrap().pointer, format!("/{keyword}"));
    }
}

#[test]
fn recursive_validation_does_not_repeatedly_charge_inert_description_payloads() {
    let module = SchemaModule::from_value(json!({"types": {"app.Node": {"schema": {
        "type": "object",
        "description": "documentation ".repeat(8_000),
        "properties": {
            "value": {"type": "integer"},
            "next": {"$ref": "#"}
        },
        "required": ["value"]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let mut source = "{value: 1}".to_owned();
    for _ in 0..20 {
        source = format!("{{\nvalue: 1\nnext: {source}\n}}");
    }
    let supplied = parse(&source).unwrap();
    schemas
        .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Node"))
        .unwrap();
}

#[test]
fn recursive_boolean_container_constraints_preserve_instance_locations() {
    for (schema, invalid, instance_path, schema_path) in [
        (
            json!({
                "type": "object", "properties": {"next": {"$ref": "#"}},
                "additionalProperties": false
            }),
            "{next: {extra: 1}}",
            "/next",
            "/additionalProperties",
        ),
        (
            json!({
                "type": "array", "prefixItems": [true, {"$ref": "#"}], "items": false
            }),
            "[1\n[2\n[]\n3]]",
            "/1/2",
            "/items",
        ),
    ] {
        let module = SchemaModule::from_value(json!({
            "types": {"app.Tree": {"schema": schema}}
        }))
        .unwrap();
        let schemas = Schemas::compose([module]).unwrap();
        let supplied = parse(invalid).unwrap();
        let error = schemas
            .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Tree"))
            .unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert_eq!(error.path, instance_path);
        assert_eq!(error.schema.unwrap().pointer, schema_path);
    }
}

#[test]
fn nested_validity_probes_do_not_charge_error_copies_of_all_descendants() {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "rows": {"type": "array", "items": true},
            "next": {"$ref": "#"}
        },
        "required": ["rows"],
        "additionalProperties": false
    });
    for _ in 0..3 {
        schema = json!({"anyOf": [schema, false]});
    }
    let module = SchemaModule::from_value(json!({
        "types": {"app.Node": {"schema": schema}}
    }))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let source = format!("{{rows: [\n{}]}}", "0\n".repeat(5_000));
    let supplied = parse(&source).unwrap();
    schemas
        .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Node"))
        .unwrap();
}

#[test]
fn property_name_error_collection_reserves_each_enclosing_object_copy() {
    let module = SchemaModule::from_value(json!({"types": {"app.Node": {"schema": {
        "type": "object",
        "properties": {"next": {"$ref": "#"}},
        "anyOf": [
            {"propertyNames": {"allOf": vec![json!({"pattern": "^ok"}); 16]}},
            false
        ]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let supplied = SemanticNode {
        tag: None,
        value: SemanticValue::Mapping(
            (0..32)
                .map(|index| {
                    (
                        format!("bad{index}"),
                        SemanticNode {
                            tag: None,
                            value: SemanticValue::String("x".repeat(4096)),
                        },
                    )
                })
                .collect(),
        ),
    };
    let error = schemas
        .validate_semantic(&supplied, Some("app.Node"))
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
}

#[test]
fn negation_diagnostics_do_not_expose_private_compiled_schema_references() {
    for (schema, source) in [
        (json!({"not": {"type": "integer"}}), "1"),
        (
            json!({"propertyNames": {"not": {"type": "string"}}}),
            "{bad: 1}",
        ),
    ] {
        let module = SchemaModule::from_value(json!({
            "types": {"app.Value": {"schema": schema}}
        }))
        .unwrap();
        let schemas = Schemas::compose([module]).unwrap();
        let supplied = parse(source).unwrap();
        let error = schemas
            .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Value"))
            .unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert!(!error.message.contains("#/$defs/"), "{}", error.message);
    }
}
