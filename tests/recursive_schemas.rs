//! Finite structural recursion, completion and resource bounds at the public API.

use daml::{ErrorOrigin, Origin, ResolveErrorKind, SchemaModule, Schemas, SemanticNode, parse};
use serde_json::json;

fn recursive_nodes() -> Schemas {
    let root = SchemaModule::from_value(json!({
        "types": {"app.Node": {"schema": {"$ref": "urn:recursive:node"}}}
    }))
    .unwrap();
    let resource = SchemaModule::from_value(json!({
        "resources": [{"uri": "urn:recursive:node", "schema": {
            "$defs": {"Count": {"type": "integer", "minimum": 0}},
            "type": "object",
            "properties": {
                "value": {"$ref": "#/$defs/Count"},
                "copies": {"type": "integer", "default": 2},
                "next": {"$ref": "urn:recursive:node"},
                "children": {"type": "array", "items": {"$ref": "urn:recursive:node"}, "default": []}
            },
            "required": ["value", "copies", "children"],
            "additionalProperties": false
        }}]
    }))
    .unwrap();
    Schemas::compose([root, resource]).unwrap()
}

#[test]
fn recursive_resources_validate_descendants_and_complete_finite_defaults() {
    let schemas = recursive_nodes();
    let supplied = parse("{\nvalue: 1\nnext: {value: 2}\nchildren: [{value: 3}]\n}").unwrap();
    let before = SemanticNode::from_document(&supplied);
    let missing = schemas
        .validate_semantic(&before, Some("app.Node"))
        .unwrap_err();
    assert_eq!(missing.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(SemanticNode::from_document(&supplied), before);

    let completed = schemas.resolve_as(&supplied, "app.Node").unwrap();
    assert_eq!(
        completed.to_json(),
        json!({
            "value": 1, "copies": 2,
            "next": {"value": 2, "copies": 2, "children": []},
            "children": [{"value": 3, "copies": 2, "children": []}]
        })
    );
    let origin = completed
        .root()
        .get("next")
        .unwrap()
        .get("copies")
        .unwrap()
        .origin();
    let Origin::Default(location) = origin else {
        panic!("nested completion must retain its schema default origin");
    };
    assert_eq!(location.resource, "urn:recursive:node");
    assert_eq!(location.pointer, "/properties/copies/default");
    schemas
        .validate_semantic(
            &daml::SemanticNode::from_resolved(completed.root()),
            Some("app.Node"),
        )
        .unwrap();
    assert_eq!(SemanticNode::from_document(&supplied), before);
}

#[test]
fn recursive_object_constraint_failure_keeps_the_descendant_pointer() {
    let schemas = recursive_nodes();
    let supplied = parse(
        "{\nvalue: 1\ncopies: 2\nchildren: []\nnext: {\nvalue: -1\ncopies: 2\nchildren: []\n}\n}",
    )
    .unwrap();
    let error = schemas
        .validate_semantic(&SemanticNode::from_document(&supplied), Some("app.Node"))
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/next/value");
    assert_eq!(error.origin, ErrorOrigin::Semantic);
}

#[test]
fn recursive_array_alternatives_preserve_leaf_constraints() {
    let module = SchemaModule::from_value(json!({"types": {"app.Nested": {"schema": {
        "anyOf": [
            {"type": "integer", "minimum": 0},
            {"type": "array", "items": {"$ref": "#"}}
        ]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let valid = parse("[1\n[2\n[3]]]").unwrap();
    schemas
        .validate_semantic(&SemanticNode::from_document(&valid), Some("app.Nested"))
        .unwrap();
    let invalid = parse("[1\n[2\n[-3]]]").unwrap();
    assert_eq!(
        schemas
            .validate_semantic(&SemanticNode::from_document(&invalid), Some("app.Nested"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn unreferenced_definition_containment_is_not_recursive_evaluation() {
    let module = SchemaModule::from_value(json!({"types": {"app.Value": {"schema": {
        "type": "integer", "minimum": 1,
        "$defs": {"Root": {"$ref": "#"}}
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let valid = parse("2").unwrap();
    schemas
        .validate_semantic(&SemanticNode::from_document(&valid), Some("app.Value"))
        .unwrap();
    let invalid = parse("0").unwrap();
    assert_eq!(
        schemas
            .validate_semantic(&SemanticNode::from_document(&invalid), Some("app.Value"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn unending_required_or_default_container_completion_is_bounded() {
    for schema in [
        json!({"type": "object", "properties": {"child": {"$ref": "#"}}, "required": ["child"]}),
        json!({"type": "object", "properties": {"child": {"$ref": "#", "default": {}}}}),
    ] {
        let module =
            SchemaModule::from_value(json!({"types": {"app.Loop": {"schema": schema}}})).unwrap();
        let schemas = Schemas::compose([module]).unwrap();
        let supplied = parse("{}").unwrap();
        let error = schemas.resolve_as(&supplied, "app.Loop").unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
        assert_eq!(supplied.source(), "{}");
    }
}

#[test]
fn recursive_branch_expansion_cannot_bypass_validation_work_limits() {
    let module = SchemaModule::from_value(json!({"types": {"app.Branches": {"schema": {
        "type": "object",
        "allOf": [
            {"properties": {"child": {"$ref": "#"}}},
            {"properties": {"child": {"$ref": "#"}}}
        ]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let mut source = "{}".to_owned();
    for _ in 0..30 {
        source = format!("{{child: {source}}}");
    }
    let supplied = parse(&source).unwrap();
    let error = schemas
        .validate_semantic(
            &SemanticNode::from_document(&supplied),
            Some("app.Branches"),
        )
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
}
