//! Focused public-API checks for explanation constraints, pointers, and recursion.
use daml::{ExplainError, ResolveErrorKind, ResolvedValue, SchemaModule, Schemas, parse};
use serde_json::{Value, json};

fn schemas(types: Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({"types": types})).unwrap()]).unwrap()
}

#[test]
fn nested_tag_constraints_include_open_container_and_ancestor_contracts() {
    let registry = schemas(json!({
        "app.Root": {"schema": {"$id": "urn:explain:root", "type": "object", "properties": {
            "child": {"type": "object", "properties": {"score": {"type": "integer", "minimum": 3}}}
        }}},
        "app.Child": {"schema": {"$id": "urn:explain:child", "type": "object", "properties": {
            "score": {"type": "integer", "minimum": 3}, "label": {"type": "string", "minLength": 2}
        }}}
    }));
    let document =
        parse("!app.Root {\nchild: !app.Child {\nscore: 4\nlabel: \"ok\"\nextra: true\n}\n}")
            .unwrap();
    let explanation = registry
        .explain(&document, Some("app.Root"), "/child")
        .unwrap();
    let constraints = explanation.constraints();
    assert!(constraints.iter().any(|c| c.instance_path.is_empty()
        && c.location.resource == "urn:explain:root"
        && c.schema["type"] == "object"));
    assert!(constraints.iter().any(|c| c.instance_path == "/child"
        && c.location.resource == "urn:explain:child"
        && c.schema["properties"]["label"]["minLength"] == 2));
    assert!(constraints.iter().any(|c| c.instance_path == "/child"
        && c.location.resource == "urn:explain:root"
        && c.location.pointer == "/properties/child"
        && c.schema["type"] == "object"));
}

#[test]
fn rejected_contains_alternative_is_not_reported_as_applied_scalar_constraint() {
    let schemas = schemas(
        json!({"app.Values": {"schema": {"$id": "urn:explain:contains", "type": "array", "contains": {"type": "integer", "minimum": 10}}}}),
    );
    let document = parse("[2\n12]").unwrap();
    let explanation = schemas
        .explain(&document, Some("app.Values"), "/0")
        .unwrap();
    assert!(
        matches!(explanation.node().value(), ResolvedValue::Number(number) if number.as_ref().as_str() == "2")
    );
    assert!(
        !explanation
            .constraints()
            .iter()
            .any(|c| c.schema.get("minimum").is_some())
    );
    assert!(
        explanation
            .constraints()
            .iter()
            .any(|c| c.schema.get("contains").is_some())
    );
}

#[test]
fn pointer_tokens_distinguish_numeric_map_keys_and_arrays_and_decode_escapes() {
    let schemas =
        schemas(json!({"app.Data": {"schema": {"$id": "urn:explain:pointer", "type": "object"}}}));
    let document = parse("{0: {\na/b: 7\n}\n00: 9\nitems: [8]\n}").unwrap();
    let keyed = schemas
        .explain(&document, Some("app.Data"), "/0/a~1b")
        .unwrap();
    assert_eq!(keyed.path(), "/0/a~1b");
    assert!(
        matches!(keyed.node().value(), ResolvedValue::Number(number) if number.as_ref().as_str() == "7")
    );
    let numeric_key = schemas.explain(&document, Some("app.Data"), "/00").unwrap();
    assert!(
        matches!(numeric_key.node().value(), ResolvedValue::Number(number) if number.as_ref().as_str() == "9")
    );
    assert!(matches!(
        schemas.explain(&document, Some("app.Data"), "/items/00"),
        Err(ExplainError::InvalidPointer(_))
    ));
    assert!(matches!(
        schemas.explain(&document, Some("app.Data"), "/items/~2"),
        Err(ExplainError::InvalidPointer(_))
    ));
}

#[test]
fn bounded_recursive_failure_and_finite_leaf_keep_recursive_schema_identity() {
    let registry = schemas(json!({"app.Node": {"schema": {
        "$id": "urn:explain:node", "type": "object",
        "properties": {"value": {"type": "integer"}, "next": {"$ref": "#"}}
    }}}));
    let document = parse("{value: 1\nnext: {value: 2\nnext: {value: 3}}}").unwrap();
    let leaf = registry
        .explain(&document, Some("app.Node"), "/next/next/value")
        .unwrap();
    assert!(
        matches!(leaf.node().value(), ResolvedValue::Number(number) if number.as_ref().as_str() == "3")
    );
    assert!(
        leaf.constraints()
            .iter()
            .any(|c| c.schema["type"] == "integer"
                && c.location.resource == "urn:explain:node"
                && c.location.pointer == "/properties/value")
    );

    let recursive = schemas(json!({"app.Required": {"schema": {
        "$id": "urn:explain:required",
        "type": "object",
        "properties": {"next": {"$ref": "#"}},
        "required": ["next"]
    }}}));
    let empty = parse("{}").unwrap();
    assert!(
        matches!(recursive.explain(&empty, Some("app.Required"), ""), Err(ExplainError::Resolve(error)) if error.kind == ResolveErrorKind::LimitExceeded)
    );
}
