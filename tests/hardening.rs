//! Retained corpus behavior and cumulative resource limits across completion plans.

use daml::{ErrorKind, ResolutionLimits, ResolveErrorKind, SchemaModule, Schemas, parse};
use serde_json::json;

#[test]
fn yaml_implicit_mapping_does_not_enter_a_guessed_daml_mode() {
    let source = include_str!("fixtures/yaml/original/3UYS.yaml");
    let error = parse(source).unwrap_err();
    assert_eq!(error.kind, ErrorKind::ExpectedValue);
    assert!(source.get(error.span.start..error.span.end).is_some());
}

#[test]
fn complete_mapping_does_not_hide_invalid_trailing_input() {
    let source = include_str!("fixtures/yaml/daml/236B.daml");
    let error = parse(source).unwrap_err();
    assert_eq!(error.kind, ErrorKind::TrailingInput);
    assert_eq!(error.span.start, source.find("invalid").unwrap());
    assert!(source.get(error.span.start..error.span.end).is_some());
}

#[test]
fn adapted_yaml_escape_retains_the_decoded_string() {
    let document = parse(include_str!("fixtures/yaml/daml/3UYS.daml")).unwrap();
    let value = document.get(document.root(), "escaped_slash").unwrap();
    assert_eq!(document.string(value), Some("a/b"));
}

#[test]
fn punctuation_payload_is_data_not_structure_or_comment() {
    let document = parse(include_str!("fixtures/yaml/daml/2EBW.daml")).unwrap();
    let value = document.get(document.root(), "characters").unwrap();
    assert_eq!(
        document.string(value),
        Some(r##"a!"#$%&'()*+,-./09:;<=>?@AZ[\]^_`az{|}~"##)
    );
}

#[test]
fn alternative_completions_share_a_budget_without_poisoning_the_next_call() {
    let build = |branches: usize| {
        let branches = (1..=branches)
            .map(|maximum| {
                json!({
                    "type": "object",
                    "properties": {"chosen": {"type": "integer", "default": 1}},
                    "required": ["chosen"],
                    "maxProperties": maximum
                })
            })
            .collect::<Vec<_>>();
        let module = SchemaModule::from_value(json!({"types": {
            "Config": {"schema": {"anyOf": branches}}
        }}))
        .unwrap();
        Schemas::compose([module]).unwrap()
    };
    let input = parse("!Config").unwrap();
    let limits = ResolutionLimits {
        max_nodes: 8,
        ..Default::default()
    };
    assert_eq!(
        build(1)
            .resolve_with_limits(&input, None, limits)
            .unwrap()
            .to_json(),
        json!({"chosen": 1})
    );
    let schemas = build(16);
    assert_eq!(
        schemas
            .resolve_with_limits(&input, None, limits)
            .unwrap_err()
            .kind,
        ResolveErrorKind::LimitExceeded
    );
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"chosen": 1})
    );
}
