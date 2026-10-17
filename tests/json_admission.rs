//! Strict JSON admission preserves data objects and rejects ambiguous members.

use daml::{ResolveErrorKind, SchemaErrorKind, SchemaModule, Schemas, parse};
use serde_json::{Value, json};

fn module_from_value(schema: Value) -> SchemaModule {
    SchemaModule::from_value(json!({"types": {"Config": {"schema": schema}}}))
        .expect("module from Value")
}

fn schemas(module: SchemaModule) -> Schemas {
    Schemas::compose([module]).expect("schemas")
}

#[test]
fn private_number_looking_defaults_remain_ordinary_objects() {
    let module_json = r#"{"types":{"Config":{"schema":{"type":"object","properties":{"plain":{"type":"object","default":{"$serde_json::private::Number":"not numeric"}},"extra":{"type":"object","default":{"$serde_json::private::Number":7,"extra":true}}},"required":["plain","extra"]}}}}"#;
    let loaded = SchemaModule::from_json(module_json).expect("JSON module");
    let from_value = module_from_value(json!({
        "type": "object",
        "properties": {
            "plain": {"type": "object", "default": {
                "$serde_json::private::Number": "not numeric"
            }},
            "extra": {"type": "object", "default": {
                "$serde_json::private::Number": 7,
                "extra": true
            }}
        },
        "required": ["plain", "extra"]
    }));

    for module in [loaded, from_value] {
        let schemas = schemas(module);
        let input = parse("!Config").expect("input");
        let resolved = schemas.resolve(&input).expect("default resolves");
        assert_eq!(
            resolved.to_json(),
            json!({
                "plain": {"$serde_json::private::Number": "not numeric"},
                "extra": {"$serde_json::private::Number": 7, "extra": true}
            })
        );
    }
}

#[test]
fn private_number_looking_const_does_not_match_numeric_value() {
    let json_module = SchemaModule::from_json(
        r#"{"types":{"Config":{"schema":{"const":{"$serde_json::private::Number":7}}}}}"#,
    )
    .expect("JSON module");
    let value_module = module_from_value(json!({
        "const": {"$serde_json::private::Number": 7}
    }));

    for module in [json_module, value_module] {
        let error = schemas(module)
            .resolve(&parse("!Config 7").expect("numeric input"))
            .expect_err("ordinary object const must not match a number");
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    }
}

#[test]
fn escaped_duplicate_member_names_are_rejected_after_decoding() {
    let error = SchemaModule::from_json(
        r#"{"types":{"Config":{"schema":{"properties":{"a/b":{"type":"string"},"a\u002fb":{"type":"number"}}}}}}"#,
    )
    .err()
    .expect("escaped keys decode to the same member name");
    assert_eq!(error.kind, SchemaErrorKind::Parse);
}

#[test]
fn arbitrary_precision_schema_numbers_are_preserved() {
    for number in ["1e400", "1234567890123456789012345678901234567890"] {
        let text = [r#"{"types":{"Config":{"schema":{"const":"#, number, "}}}}"].concat();
        let module = SchemaModule::from_json(&text).expect("arbitrary-precision JSON number");
        let schemas = schemas(module);
        let input = parse(format!("!Config {number}")).expect("arbitrary-precision DAML number");
        assert!(
            schemas.resolve(&input).is_ok(),
            "const should preserve {number}"
        );
    }
}

#[test]
fn malformed_json_is_a_parse_error_and_admission_bounds_are_structured() {
    let malformed = SchemaModule::from_json(r#"{"types":{"Config":}}"#)
        .err()
        .expect("malformed JSON");
    assert_eq!(malformed.kind, SchemaErrorKind::Parse);

    let oversized = format!(r#"{{"padding":"{}"}}"#, "x".repeat(8 * 1024 * 1024));
    let oversized = SchemaModule::from_json(&oversized)
        .err()
        .expect("source byte limit");
    assert_eq!(oversized.kind, SchemaErrorKind::LimitExceeded);

    let many_nodes = format!("[{}]", vec!["null"; 10_001].join(","));
    let many_nodes = SchemaModule::from_json(&many_nodes)
        .err()
        .expect("node limit");
    assert_eq!(many_nodes.kind, SchemaErrorKind::LimitExceeded);

    let deep = format!("{}null{}", "[".repeat(129), "]".repeat(129));
    let deep = SchemaModule::from_json(&deep).err().expect("depth limit");
    assert_eq!(deep.kind, SchemaErrorKind::LimitExceeded);

    let nested_rescan = format!(
        "{}\"{}\"{}",
        "[".repeat(16),
        "x".repeat(4 * 1024 * 1024),
        "]".repeat(16)
    );
    let nested_rescan = SchemaModule::from_json(&nested_rescan)
        .err()
        .expect("raw rescan work limit");
    assert_eq!(nested_rescan.kind, SchemaErrorKind::LimitExceeded);
}
