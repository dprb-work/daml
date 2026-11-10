//! Compatibility proof work and default-bearing semantic boundaries.

use daml::{ResolveErrorKind, SchemaModule, Schemas, parse};
use serde_json::{Value, json};

fn schemas(schema: Value) -> Schemas {
    let module = SchemaModule::from_value(json!({
        "types": {"app.Config": {"schema": schema}}
    }))
    .expect("witness module must be admitted");
    Schemas::compose([module])
        .expect("witness schema must compile within existing admission bounds")
}

#[test]
fn large_object_enum_keeps_validation_only_and_default_bearing_choices_affordable() {
    for default_bearing in [false, true] {
        let mut branches = vec![json!({"const": {"a": 999}}), json!({"const": {"a": 998}})];
        if default_bearing {
            for branch in &mut branches {
                branch["default"] = json!({});
            }
        }
        let registry = schemas(json!({
            "type": "object",
            "enum": (0..1000).map(|a| json!({"a": a})).collect::<Vec<_>>(),
            "anyOf": branches
        }));
        let document = parse("!app.Config {\na: 999\n}").unwrap();
        assert_eq!(
            registry.resolve(&document).unwrap().to_json(),
            json!({"a": 999})
        );
    }
}

#[test]
fn common_default_does_not_activate_validation_only_enum_proofs() {
    let first = (0..1000).collect::<Vec<_>>();
    let second = (1000..2000).collect::<Vec<_>>();
    let registry = schemas(json!({
        "type": "object",
        "properties": {"kind": {"enum": first, "default": 0}},
        "required": ["kind"],
        "anyOf": [
            {"properties": {"kind": {"enum": first}}},
            {"properties": {"kind": {"enum": second}}}
        ]
    }));
    let document = parse("!app.Config {\n}\n").expect("valid DAML input");
    let result = registry.resolve(&document);
    assert!(
        result.is_ok(),
        "F2 must preserve common completion within the default budget: {result:?}"
    );
    assert_eq!(result.unwrap().to_json(), json!({"kind": 0}));
}

#[test]
fn optional_disjoint_kind_constraints_keep_distinct_default_completions_ambiguous() {
    let registry = schemas(json!({
        "type": "object",
        "properties": {"kind": {"enum": ["a"]}},
        "anyOf": [
            {
                "properties": {
                    "kind": {"enum": ["a"]},
                    "value": {"const": "left", "default": "left"}
                },
                "required": ["value"]
            },
            {
                "properties": {
                    "kind": {"enum": ["b"]},
                    "value": {"const": "right", "default": "right"}
                },
                "required": ["value"]
            }
        ]
    }));
    let document = parse("!app.Config {\n}\n").expect("valid DAML input");
    // kind is optional everywhere: both arms remain possible when it is absent.
    // Unlike a validation-only oneOf overlap, wrongly pruning the second arm
    // changes this result from AmbiguousDefault to successful completion.
    let error = registry
        .resolve(&document)
        .expect_err("distinct valid defaults are ambiguous");
    assert_eq!(error.kind, ResolveErrorKind::AmbiguousDefault);
}

#[test]
fn numeric_required_kind_overlap_keeps_branch_owned_value_completion() {
    let registry = schemas(json!({
        "type": "object",
        "properties": {"kind": {"const": 1, "default": 1}},
        "required": ["kind"],
        "anyOf": [
            {
                "properties": {
                    "kind": {"const": 1.0},
                    "value": {"const": "numeric-match", "default": "numeric-match"}
                },
                "required": ["value"]
            },
            {
                "properties": {
                    "kind": {"const": 2.0},
                    "value": {"const": "disjoint", "default": "disjoint"}
                },
                "required": ["value"]
            }
        ]
    }));
    let document = parse("!app.Config {\n}\n").expect("valid DAML input");
    // kind is missing during discovery. Requiredness makes its finite-domain
    // intersection relevant; JSON numeric equality must keep 1 versus 1.0.
    // The branch supplies value, so merely validating a common-only result or
    // skipping every union cannot accidentally satisfy this assertion.
    let result = registry
        .resolve(&document)
        .expect("1 and 1.0 have overlapping numeric domains");
    assert_eq!(
        result.to_json(),
        json!({"kind": 1, "value": "numeric-match"})
    );
}
