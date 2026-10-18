//! Deep assertions, module ownership, representation equivalence, and provenance.

use daml::{ErrorOrigin, ResolveErrorKind, SchemaModule, Schemas, parse};
use serde_json::json;

fn application_modules() -> Vec<SchemaModule> {
    vec![
        SchemaModule::from_daml(include_str!("../examples/schemas/application.daml")).unwrap(),
        SchemaModule::from_json(include_str!("../examples/schemas/network.json")).unwrap(),
        SchemaModule::from_daml(include_str!("../examples/schemas/text.daml")).unwrap(),
    ]
}

#[test]
fn complete_application_checks_deep_tags_and_completes_defaults() {
    let schemas = Schemas::compose(application_modules()).unwrap();
    let document = parse(include_str!("../examples/application.daml")).unwrap();
    let resolved = schemas.resolve(&document).unwrap();
    assert_eq!(resolved.root().tag(), Some("demo.Application"));
    let adapter = resolved.root().get("adapter").unwrap();
    assert_eq!(adapter.tag(), Some("network.Adapter"));
    assert_eq!(adapter.get("timeout").unwrap().to_json(), json!(30));
    assert!(matches!(
        adapter.get("timeout").unwrap().origin(),
        daml::Origin::Default(_)
    ));
    let task = &resolved.root().get("tasks").unwrap().items().unwrap()[0];
    let request = task.get("request").unwrap();
    assert_eq!(request.tag(), Some("text.Request"));
    assert_eq!(
        request.get("instruction").unwrap().string(),
        Some("Extract the release name. Return JSON only.")
    );
    assert_eq!(
        request.get("context").unwrap().string(),
        Some("release:\n  name: daml")
    );
    assert_eq!(request.get("request_id").unwrap().string(), Some("local"));
    assert_eq!(
        resolved.root().get("exact").unwrap().to_json().to_string(),
        "9007199254740993"
    );
    let original_adapter = document.get(document.root(), "adapter").unwrap();
    assert!(
        document.get(original_adapter, "timeout").is_none(),
        "completion must not mutate supplied syntax"
    );
}

#[test]
fn unknown_tag_is_checked_beneath_open_records_and_sequences() {
    let module = SchemaModule::from_value(json!({"types": {
        "Root": {"schema": {"type": "object", "additionalProperties": true}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let source = "!Root { extra: [{ message: \"é\"\n value: !Missing null }] }";
    let document = parse(source).unwrap();
    let error = schemas.resolve(&document).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::UnknownTag);
    assert_eq!(error.path, "/extra/0/value");
    let ErrorOrigin::Input(span) = error.origin else {
        panic!("input tag")
    };
    assert_eq!(document.slice(span), Some("Missing"));
}

#[test]
fn parent_constraint_and_explicit_tag_are_both_enforced() {
    let module = SchemaModule::from_value(json!({"types": {
        "Root": {"schema": {
            "type": "object",
            "properties": {"value": {"type": "integer", "minimum": 10}},
            "required": ["value"]
        }},
        "Small": {"schema": {"type": "integer", "maximum": 5}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    for value in [3, 20] {
        let document = parse(format!("!Root {{ value: !Small {value} }}")).unwrap();
        let error = schemas.resolve(&document).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert_eq!(error.path, "/value");
    }
}

#[test]
fn same_schema_in_json_and_daml_accepts_and_rejects_the_same_data() {
    let json_module = SchemaModule::from_value(json!({"types": {"Config": {"schema": {
        "type": "object",
        "properties": {"timeout": {"type": "integer", "minimum": 1, "default": 30}},
        "required": ["timeout"],
        "additionalProperties": false
    }}}}))
    .unwrap();
    let daml_module = SchemaModule::from_daml(
        r#"{
      types: {
        Config: {
          schema: {
            type: "object"
            properties: {
              timeout: {
                type: "integer"
                minimum: 1
                default: 30
              }
            }
            required: ["timeout"]
            additionalProperties: false
          }
        }
      }
    }"#,
    )
    .unwrap();
    for module in [json_module, daml_module] {
        let schemas = Schemas::compose([module]).unwrap();
        let partial = parse("!Config").unwrap();
        assert_eq!(
            schemas.resolve(&partial).unwrap().to_json(),
            json!({"timeout": 30})
        );
        let invalid = parse("!Config { timeout: 0 }").unwrap();
        let error = schemas.resolve(&invalid).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert_eq!(error.path, "/timeout");
    }
}

#[test]
fn partial_object_defaults_merge_without_waiving_required_fields() {
    let module = SchemaModule::from_value(json!({"types": {"Request": {"schema": {
        "type": "object",
        "properties": {"params": {
            "type": "object",
            "default": {"timeout": 30, "retries": 2},
            "properties": {
                "timeout": {"type": "integer"},
                "retries": {"type": "integer"},
                "endpoint": {"type": "string"}
            },
            "required": ["timeout", "retries", "endpoint"],
            "additionalProperties": false
        }},
        "required": ["params"]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let partial = parse("!Request { params: { endpoint: \"/v1\"\n timeout: 5 } }").unwrap();
    assert_eq!(
        schemas.resolve(&partial).unwrap().to_json(),
        json!({"params": {
            "timeout": 5, "retries": 2, "endpoint": "/v1"
        }})
    );
    let incomplete = parse("!Request").unwrap();
    let error = schemas.resolve(&incomplete).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert!(error.path.starts_with("/params"));
}

#[test]
fn invalid_default_reports_schema_origin_not_a_fake_input_span() {
    let module = SchemaModule::from_value(json!({"types": {"Config": {"schema": {
        "$id": "urn:example:Config",
        "type": "object",
        "properties": {"port": {"type": "integer", "minimum": 1, "default": 0}},
        "required": ["port"]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let document = parse("!Config").unwrap();
    let error = schemas.resolve(&document).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/port");
    let ErrorOrigin::Default(location) = error.origin else {
        panic!("must identify the actual invalid default")
    };
    assert_eq!(location.resource, "urn:example:Config");
    assert_eq!(location.pointer, "/properties/port/default");
}

#[test]
fn numeric_constraints_are_exact_and_out_of_profile_numbers_fail_closed() {
    let module = SchemaModule::from_json(
        r#"{"types":{
      "Big":{"schema":{"type":"integer","maximum":184467440737095516170}},
      "Decimal":{"schema":{"type":"number","multipleOf":0.1}},
      "Exponent":{"schema":{"type":"number","maximum":1e399}}
    }}"#,
    )
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let exact = parse("!Big 184467440737095516170").unwrap();
    assert_eq!(
        schemas.resolve(&exact).unwrap().to_json().to_string(),
        "184467440737095516170"
    );
    for source in [
        "!Big 184467440737095516171",
        "!Decimal 0.31",
        "!Exponent 1e400",
    ] {
        let document = parse(source).unwrap();
        assert_eq!(
            schemas.resolve(&document).unwrap_err().kind,
            ResolveErrorKind::SchemaViolation,
            "{source}"
        );
    }
    let decimal = parse("!Decimal 0.3").unwrap();
    assert_eq!(
        schemas.resolve(&decimal).unwrap().to_json().to_string(),
        "0.3"
    );
    let excessive = parse("!Exponent 1e10000").unwrap();
    assert_eq!(
        schemas.resolve(&excessive).unwrap_err().kind,
        ResolveErrorKind::NumericLimit
    );
}

#[test]
fn unicode_diagnostics_retain_original_bytes_and_literal_pointer_segments() {
    let module = SchemaModule::from_value(json!({"types": {"Config": {"schema": {
        "type": "object",
        "properties": {"payload": {"type": "object", "properties": {
            "a/b~c": {"type": "string", "maxLength": 1}
        }}}
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let source = "!Config { label: \"é\"\n payload: { a/b~c: \"東京\" } }";
    let document = parse(source).unwrap();
    let error = schemas.resolve(&document).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/payload/a~1b~0c");
    let ErrorOrigin::Input(span) = error.origin else {
        panic!("supplied value")
    };
    assert_eq!(document.slice(span), Some("\"東京\""));
    assert_eq!(span.end - span.start, 8);
}

#[test]
fn completion_accounts_for_default_bytes_and_validation_work() {
    let module = SchemaModule::from_value(json!({"types": {"Config": {"schema": {
        "type": "object",
        "properties": {"text": {"type": "string", "default": "東京"}},
        "required": ["text"]
    }}}}))
    .unwrap();
    let config_schemas = Schemas::compose([module]).unwrap();
    let document = parse("!Config").unwrap();
    let byte_limit = daml::ResolutionLimits {
        max_bytes: 10, // Tag + multibyte default exceeds this, even before its key.
        ..Default::default()
    };
    let error = config_schemas
        .resolve_with_limits(&document, None, byte_limit)
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);

    let long_tag = "provider.with.a.generated.identity.longer.than.the.byte.limit";
    let native = SchemaModule::from_value(json!({"types": {"Host": {"schema": {
        "type": "object",
        "default": {},
        "x-daml-default-tags": [{"path": "", "tag": long_tag}]
    }}}}))
    .unwrap();
    let native_schemas = Schemas::compose([native]).unwrap();
    let input = parse("{}").unwrap();
    let limits = daml::ResolutionLimits {
        max_bytes: 8,
        ..Default::default()
    };
    assert_eq!(
        native_schemas
            .resolve_with_limits(&input, Some("Host"), limits)
            .unwrap_err()
            .kind,
        ResolveErrorKind::LimitExceeded
    );
    let work_limit = daml::ResolutionLimits {
        max_steps: 1,
        ..Default::default()
    };
    let error = config_schemas
        .resolve_with_limits(&document, None, work_limit)
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
}

#[test]
fn compatible_providers_and_untagged_values_satisfy_the_same_structural_requirement() {
    let host = SchemaModule::from_value(json!({
        "types": {"Host": {"schema": {
            "type": "object",
            "properties": {"adapter": {"$ref": "urn:example:Adapter"}},
            "required": ["adapter"]
        }}},
        "resources": [{"uri": "urn:example:Adapter", "schema": {
            "type": "object",
            "properties": {
                "addr": {"const": "localhost"},
                "timeout": {"type": "integer", "default": 30}
            },
            "required": ["addr", "timeout"]
        }}]
    }))
    .unwrap();
    let providers = SchemaModule::from_value(json!({"types": {
        "network.Provider": {"schema": {
            "type": "object",
            "properties": {"timeout": {"type": "integer", "maximum": 60}}
        }},
        "independent.Name": {"schema": {
            "type": "object",
            "additionalProperties": {"type": ["string", "integer"]}
        }}
    }}))
    .unwrap();
    let schemas = Schemas::compose([host, providers]).unwrap();
    for adapter in [
        "!network.Provider { addr: \"localhost\" }",
        "!independent.Name { addr: \"localhost\" }",
        "{ addr: \"localhost\" }",
    ] {
        let document = parse(format!("!Host {{ adapter: {adapter} }}")).unwrap();
        assert_eq!(
            schemas.resolve(&document).unwrap().to_json(),
            json!({"adapter": {"addr": "localhost", "timeout": 30}})
        );
    }
    for (adapter, path) in [
        ("!network.Provider { addr: \"remote\" }", "/adapter/addr"),
        (
            "!network.Provider { addr: \"localhost\"\n timeout: 61 }",
            "/adapter/timeout",
        ),
    ] {
        let document = parse(format!("!Host {{ adapter: {adapter} }}")).unwrap();
        let error = schemas.resolve(&document).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
        assert_eq!(error.path, path);
    }
}

#[test]
fn exact_large_integer_constraints_do_not_round_through_float() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({
        "types": {"Count": {"schema": {
            "type": "integer",
            "minimum": 9007199254740993_i64
        }}}
    }))
    .unwrap()])
    .unwrap();
    let minimum = parse("!Count 9007199254740993").unwrap();
    schemas.resolve(&minimum).unwrap();
    let below = parse("!Count 9007199254740992").unwrap();
    assert_eq!(
        schemas.resolve(&below).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn property_name_constraints_validate_borrowed_keys() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({
        "types": {"Record": {"schema": {
            "type": "object",
            "propertyNames": {"type": "string", "minLength": 3}
        }}}
    }))
    .unwrap()])
    .unwrap();
    let valid = parse("!Record { name: 1 }").unwrap();
    schemas.resolve(&valid).unwrap();
    let invalid = parse("!Record { x: 1 }").unwrap();
    assert_eq!(
        schemas.resolve(&invalid).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn constant_equality_preserves_exact_numbers_and_ignores_data_tag_names() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({"types": {
        "Root": {"schema": {"const": {"values": [1, {"enabled": true}]}}},
        "List": {"schema": {"type": "array"}},
        "Flag": {"schema": {"type": "boolean"}}
    }}))
    .unwrap()])
    .unwrap();
    let equivalent = parse("!Root { values: !List [1.0\n{ enabled: !Flag true }] }").unwrap();
    schemas.resolve(&equivalent).unwrap();
    let different = parse("!Root { values: !List [1.01\n{ enabled: !Flag true }] }").unwrap();
    assert_eq!(
        schemas.resolve(&different).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn expected_union_does_not_waive_a_known_tags_own_schema() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({"types": {
        "Expected": {"schema": {"anyOf": [
            {"type": "string"},
            {"type": "integer"}
        ]}},
        "Text": {"schema": {"type": "string"}}
    }}))
    .unwrap()])
    .unwrap();
    for source in ["!Text \"hello\"", "7"] {
        let document = parse(source).unwrap();
        schemas.resolve_as(&document, "Expected").unwrap();
    }
    let invalid_assertion = parse("!Text 7").unwrap();
    let error = schemas
        .resolve_as(&invalid_assertion, "Expected")
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "");
}
