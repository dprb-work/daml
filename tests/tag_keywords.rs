//! Nominal assertions participate in structural and union validation.

use daml::{ResolveErrorKind, SchemaModule, Schemas, parse};
use serde_json::json;

fn schemas(module: serde_json::Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(module).expect("valid schema module")])
        .expect("composable schema module")
}

fn assert_valid(module: serde_json::Value, input: &str) {
    let schemas = schemas(module);
    let document = parse(input).expect("valid DAML input");
    schemas.resolve(&document).expect("input satisfies schema");
}

fn assert_invalid(module: serde_json::Value, input: &str) {
    let schemas = schemas(module);
    let document = parse(input).expect("valid DAML input");
    assert!(
        schemas.resolve(&document).is_err(),
        "input should fail schema validation"
    );
}

#[test]
fn tag_constraints_apply_to_scalar_array_object_and_empty_mapping_nodes() {
    assert_valid(
        json!({"types": {"app.Text": {"schema": {"x-daml-tag": "app.Text", "type": "string"}}}}),
        "!app.Text \"hello\"",
    );
    assert_valid(
        json!({"types": {"app.List": {"schema": {"x-daml-tag": "app.List", "type": "array", "items": {"type": "integer"}}}}}),
        "!app.List [1\n2\n3]",
    );
    assert_valid(
        json!({"types": {"app.Record": {"schema": {"x-daml-tag": "app.Record", "type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}}}}),
        "!app.Record { name: \"Ada\" }",
    );
    assert_valid(
        json!({"types": {"app.Empty": {"schema": {"x-daml-tag": "app.Empty", "type": "object", "maxProperties": 0}}}}),
        "!app.Empty",
    );
}

#[test]
fn tag_constraints_reject_a_different_known_nominal_tag() {
    assert_invalid(
        json!({"types": {"app.Textual": {"schema": {"x-daml-tag": "app.Text", "type": "string"}}}}),
        "!app.Textual \"hello\"",
    );
}

#[test]
fn nominal_tag_constraints_participate_inside_union_branches() {
    let schemas = schemas(json!({
        "types": {
            "app.Root": {"schema": {"anyOf": [
                {"x-daml-tag": "app.Text", "type": "string"},
                {"x-daml-tag": "app.Count", "type": "integer"}
            ]}},
            "app.Text": {"schema": {"type": "string"}},
            "app.Count": {"schema": {"type": "integer"}}
        }
    }));
    for (source, expected) in [
        ("!app.Text \"x\"", "app.Root"),
        ("!app.Count 7", "app.Root"),
    ] {
        let document = parse(source).expect("valid DAML input");
        schemas
            .resolve_as(&document, expected)
            .expect("one nominal union branch matches");
    }
    let document = parse("!app.Text 7").expect("valid DAML input");
    assert!(schemas.resolve_as(&document, "app.Root").is_err());
}

#[test]
fn slots_require_explicit_membership_even_inside_union_branches() {
    let schemas = schemas(json!({
        "types": {
            "app.Root": {"schema": {"anyOf": [
                {"x-daml-slot": "app.Shape", "type": "string"},
                {"x-daml-tag": "app.Other", "type": "string"}
            ]}},
            "app.Member": {"schema": {"type": "string"}, "implements": ["app.Shape"]},
            "app.Nonmember": {"schema": {"type": "string"}},
            "app.Other": {"schema": {"type": "string"}}
        },
        "contracts": {"app.Shape": {"type": "string"}}
    }));

    let member = parse("!app.Member \"x\"").expect("valid DAML input");
    schemas
        .resolve_as(&member, "app.Root")
        .expect("explicit member tag satisfies slot");

    let nonmember = parse("!app.Nonmember \"x\"").expect("valid DAML input");
    assert!(schemas.resolve_as(&nonmember, "app.Root").is_err());

    let other = parse("!app.Other \"x\"").expect("valid DAML input");
    schemas
        .resolve_as(&other, "app.Root")
        .expect("second tagged branch matches");
}

#[test]
fn exact_large_integer_is_validated_without_float_coercion() {
    let schemas = schemas(json!({
        "types": {"app.Count": {"schema": {"type": "integer", "minimum": 9007199254740993_i64}}}
    }));
    let at_minimum = parse("!app.Count 9007199254740993").expect("valid DAML input");
    schemas
        .resolve(&at_minimum)
        .expect("exact minimum is accepted");
    let below_minimum = parse("!app.Count 9007199254740992").expect("valid DAML input");
    assert_eq!(
        schemas.resolve(&below_minimum).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}
#[test]
fn property_names_are_validated_as_borrowed_string_nodes() {
    let schemas = schemas(json!({
        "types": {
            "app.Record": {
                "schema": {
                    "type": "object",
                    "propertyNames": {"type": "string", "minLength": 3}
                }
            }
        }
    }));
    let valid = parse("!app.Record { name: 1 }").expect("valid DAML input");
    schemas
        .resolve(&valid)
        .expect("property name meets its schema");

    let invalid = parse("!app.Record { x: 1 }").expect("valid DAML input");
    assert!(schemas.resolve(&invalid).is_err());
}

#[test]
fn data_equality_uses_exact_numbers_and_ignores_tag_metadata() {
    let schemas = schemas(json!({"types": {
        "Root": {"schema": {"const": {"values": [1, {"enabled": true}]}}},
        "List": {"schema": {"type": "array"}},
        "Flag": {"schema": {"type": "boolean"}}
    }}));
    let document = parse("!Root { values: !List [1.0\n{ enabled: !Flag true }] }").unwrap();
    schemas.resolve(&document).unwrap();
    let changed = parse("!Root { values: !List [1.01\n{ enabled: !Flag true }] }").unwrap();
    assert_eq!(
        schemas.resolve(&changed).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}
