//! Consumer checks for direct admission of editable, binding-aware semantic values.

use daml::{
    Bundle, ErrorOrigin, FormatOptions, Origin, ResolutionLimits, ResolveErrorKind, SchemaModule,
    Schemas, SemanticErrorKind, SemanticNode, SemanticTag, SemanticValue, parse,
};
use serde_json::{Number, json};
use std::{collections::BTreeMap, str::FromStr};

fn node(value: SemanticValue) -> SemanticNode {
    SemanticNode { tag: None, value }
}

fn number(raw: &str) -> SemanticNode {
    node(SemanticValue::Number(Number::from_str(raw).unwrap()))
}

fn tagged(mut value: SemanticNode, name: &str) -> SemanticNode {
    value.tag = Some(SemanticTag {
        name: name.to_owned(),
        binding: None,
    });
    value
}

fn mapping(entries: impl IntoIterator<Item = (&'static str, SemanticNode)>) -> SemanticNode {
    node(SemanticValue::Mapping(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    ))
}

fn schemas(types: serde_json::Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({"types": types})).unwrap()]).unwrap()
}

#[test]
fn semantic_completion_preserves_supplied_and_real_default_origins() {
    let registry = schemas(json!({"Config": {"schema": {
        "$id": "urn:semantic:config", "type": "object",
        "properties": {
            "name": {"type": "string", "maxLength": 4},
            "count": {"type": "integer", "default": 7}
        }, "required": ["name", "count"]
    }}}));
    let supplied = tagged(
        mapping([("name", node(SemanticValue::String("safe".into())))]),
        "Config",
    );
    let before = supplied.clone();
    let resolved = registry.resolve_semantic(&supplied, None).unwrap();
    assert_eq!(resolved.to_json(), json!({"name": "safe", "count": 7}));
    assert_eq!(resolved.origin(), Origin::Semantic);
    assert_eq!(resolved.tag_origin(), Some(Origin::Semantic));
    assert_eq!(resolved.get("name").unwrap().origin(), Origin::Semantic);
    let Origin::Default(location) = resolved.get("count").unwrap().origin() else {
        panic!("completed count must retain its actual schema default provenance");
    };
    assert_eq!(location.resource, "urn:semantic:config");
    assert_eq!(location.pointer, "/properties/count/default");
    assert_eq!(
        supplied, before,
        "admission must not mutate the editable input"
    );

    let invalid = mapping([("name", node(SemanticValue::String("edited".into())))]);
    let error = registry
        .resolve_semantic(&invalid, Some("Config"))
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/name");
    assert_eq!(error.origin, ErrorOrigin::Semantic);

    let invalid_defaults = schemas(json!({"Bad": {"schema": {
        "$id": "urn:semantic:bad", "properties": {
            "count": {"type": "integer", "minimum": 1, "default": 0}
        }
    }}}));
    let empty = mapping([]);
    let error = invalid_defaults
        .resolve_semantic(&empty, Some("Bad"))
        .unwrap_err();
    let ErrorOrigin::Default(location) = error.origin else {
        panic!("invalid defaults are not supplied semantic input");
    };
    assert_eq!(location.resource, "urn:semantic:bad");
    assert_eq!(location.pointer, "/properties/count/default");
}

#[test]
fn expected_root_and_explicit_assertion_are_both_active() {
    let registry = schemas(json!({
        "AtLeastTen": {"schema": {"type": "integer", "minimum": 10}},
        "AtMostFifteen": {"schema": {"type": "integer", "maximum": 15}}
    }));
    let valid = tagged(number("12"), "AtMostFifteen");
    assert_eq!(
        registry
            .resolve_semantic(&valid, Some("AtLeastTen"))
            .unwrap()
            .to_json(),
        json!(12)
    );
    for raw in ["3", "20"] {
        let invalid = tagged(number(raw), "AtMostFifteen");
        let error = registry
            .resolve_semantic(&invalid, Some("AtLeastTen"))
            .unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation, "{raw}");
        assert_eq!(error.origin, ErrorOrigin::Semantic);
        assert_eq!(error.path, "");
    }
    assert_eq!(
        registry
            .resolve_semantic(&valid, Some("Absent"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::UnknownExpectedType
    );
}

#[test]
fn explicit_semantic_tag_replaces_contextual_preset_but_keeps_own_defaults_and_constraints() {
    let registry = schemas(json!({
        "Host": {"schema": {"type": "object", "properties": {"child": {
            "type": "object", "default": {"context": true, "own": 1},
            "properties": {"own": {"type": "integer", "minimum": 2}},
            "required": ["own"]
        }}}},
        "Provider": {"schema": {"type": "object", "default": {"own": 2}}}
    }));
    let supplied = tagged(
        mapping([("child", tagged(mapping([]), "Provider"))]),
        "Host",
    );
    let resolved = registry.resolve_semantic(&supplied, None).unwrap();
    assert_eq!(resolved.to_json(), json!({"child": {"own": 2}}));
    assert_eq!(
        resolved.get("child").unwrap().tag_origin(),
        Some(Origin::Semantic)
    );
    let invalid = tagged(
        mapping([("child", tagged(mapping([("own", number("0"))]), "Provider"))]),
        "Host",
    );
    let error = registry.resolve_semantic(&invalid, None).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/child/own");
    assert_eq!(error.origin, ErrorOrigin::Semantic);
}

fn collision_bundle() -> Bundle {
    let sources = [
        ("daml.bundle", "{app: \"./app\"\nprovider: \"./decoy\"}"),
        ("app/daml.bundle", "{provider: \"./provider\"}"),
        (
            "app/Host.daml",
            "!Object { child: (!Object = !provider.Adapter) }",
        ),
        ("app/provider/daml.bundle", "{}"),
        (
            "app/provider/Adapter.daml",
            "!Object { value: !String { min: 1 } } = { value: \"private\" }",
        ),
        ("decoy/daml.bundle", "{}"),
        ("decoy/Adapter.daml", "!Int"),
    ];
    Bundle::from_sources(
        ".",
        sources
            .into_iter()
            .map(|(path, source)| (path.to_owned(), source.to_owned())),
    )
    .unwrap()
}

#[test]
fn private_native_binding_survives_a_colliding_entry_alias_without_source_roundtrip() {
    let bundle = collision_bundle();
    let document = parse("{host: !app.Host\ndecoy: !provider.Adapter 7}").unwrap();
    let registry = bundle.compile(&document, None).unwrap();
    let original = registry.resolve(&document).unwrap();
    let exported = SemanticNode::from_resolved(original.root());
    let private_tag = exported_child(&exported).tag.as_ref().unwrap().clone();
    assert_eq!(private_tag.name, "provider.Adapter");
    assert!(private_tag.binding.is_some());
    let error = registry
        .format_semantic(&exported, FormatOptions { line_width: 80 })
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::UnrepresentableTag);
    assert_eq!(error.pointer, "/host/child");
    let admitted = registry.resolve_semantic(&exported, None).unwrap();
    assert_eq!(
        admitted.to_json(),
        json!({"host": {"child": {"value": "private"}}, "decoy": 7})
    );
    assert_eq!(
        admitted
            .get("host")
            .unwrap()
            .get("child")
            .unwrap()
            .tag_binding(),
        private_tag.binding.as_deref()
    );

    let mut edited = exported.clone();
    let SemanticValue::Mapping(fields) = &mut exported_child_mut(&mut edited).value else {
        panic!("private adapter is an object");
    };
    fields.insert("value".into(), node(SemanticValue::String(String::new())));
    let error = registry.resolve_semantic(&edited, None).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/host/child/value");
    assert_eq!(error.origin, ErrorOrigin::Semantic);

    // A real binding from another registry and an invented binding cannot fall
    // back to the valid entry-visible integer alias with the same literal name.
    let other = schemas(json!({"provider.Adapter": {"schema": {"type": "integer"}}}));
    let other_input = parse("!provider.Adapter 7").unwrap();
    let foreign = SemanticNode::from_resolved(other.resolve(&other_input).unwrap().root());
    for bound in [
        foreign,
        SemanticNode {
            tag: Some(SemanticTag {
                name: "provider.Adapter".into(),
                binding: Some("urn:absent:binding".into()),
            }),
            value: number("7").value,
        },
    ] {
        let error = registry.resolve_semantic(&bound, None).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::UnknownTag);
        assert_eq!(error.origin, ErrorOrigin::Semantic);
    }
}

fn exported_child(value: &SemanticNode) -> &SemanticNode {
    let SemanticValue::Mapping(root) = &value.value else {
        panic!("root object")
    };
    let SemanticValue::Mapping(host) = &root["host"].value else {
        panic!("host object")
    };
    &host["child"]
}

fn exported_child_mut(value: &mut SemanticNode) -> &mut SemanticNode {
    let SemanticValue::Mapping(root) = &mut value.value else {
        panic!("root object")
    };
    let SemanticValue::Mapping(host) = &mut root.get_mut("host").unwrap().value else {
        panic!("host object")
    };
    host.get_mut("child").unwrap()
}

#[test]
fn exact_numeric_values_and_real_spelling_survive_direct_readmission() {
    let tokens = [
        "900719925474099312345678901234567890",
        "1234567890.01234567890123456789",
        "-0.0",
        "1.2300e-40",
        "1.0",
    ];
    let exact = tokens
        .iter()
        .map(|raw| serde_json::from_str::<serde_json::Value>(raw).unwrap())
        .collect::<Vec<_>>();
    let registry = schemas(json!({"Exact": {"schema": {"const": exact}}}));
    let value = node(SemanticValue::Sequence(
        tokens.iter().map(|raw| number(raw)).collect(),
    ));
    let admitted = registry.resolve_semantic(&value, Some("Exact")).unwrap();
    let exported = SemanticNode::from_resolved(&admitted);
    let readmitted = registry.resolve_semantic(&exported, Some("Exact")).unwrap();
    for (item, token) in readmitted.items().unwrap().iter().zip(tokens) {
        let raw = item.to_json().to_string();
        let is_real = |text: &str| text.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E'));
        assert_eq!(is_real(&raw), is_real(token));
        if token == "-0.0" {
            assert!(raw.starts_with('-'), "negative zero must keep its sign");
        }
    }
    let limited =
        schemas(json!({"Big": {"schema": {"type": "integer", "maximum": 9007199254740993_i64}}}));
    let excess = number("9007199254740994");
    assert_eq!(
        limited
            .resolve_semantic(&excess, Some("Big"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn native_int_remains_mathematical_not_an_olaf_spelling_policy() {
    let bundle = Bundle::from_sources(
        ".",
        vec![
            ("daml.bundle".into(), "{}".into()),
            ("Count.daml".into(), "!Int".into()),
        ],
    )
    .unwrap();
    let input = parse("1").unwrap();
    let registry = bundle.compile(&input, Some("self.Count")).unwrap();
    let real_spelling = number("1.0");
    assert_eq!(
        registry
            .resolve_semantic(&real_spelling, Some("self.Count"))
            .unwrap()
            .to_json()
            .to_string(),
        "1.0"
    );
}

#[test]
fn numeric_profile_is_checked_even_without_a_schema() {
    let registry = schemas(json!({}));
    for raw in ["9".repeat(512), "1e+512".to_owned()] {
        let value = number(&raw);
        let admitted = registry.resolve_semantic(&value, None).unwrap();
        assert_eq!(admitted.origin(), Origin::Semantic);
    }
    for raw in ["9".repeat(513), "1e+513".to_owned()] {
        let value = number(&raw);
        let error = registry.resolve_semantic(&value, None).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::NumericLimit);
        assert_eq!(error.origin, ErrorOrigin::Semantic);
    }
}

#[test]
fn admission_enforces_grammar_and_resource_bounds_before_schema_work() {
    let registry = schemas(json!({}));
    for (value, path) in [
        (
            mapping([("bad key", node(SemanticValue::Null))]),
            "/bad key",
        ),
        (tagged(node(SemanticValue::Null), "bad/tag"), ""),
    ] {
        let error = registry.resolve_semantic(&value, None).unwrap_err();
        assert_eq!(error.path, path);
        assert_eq!(error.origin, ErrorOrigin::Semantic);
    }
    let values_and_limits = [
        (
            node(SemanticValue::Sequence(vec![node(SemanticValue::Null)])),
            ResolutionLimits {
                max_nodes: 1,
                ..Default::default()
            },
        ),
        (
            node(SemanticValue::String("東京".into())),
            ResolutionLimits {
                max_bytes: 5,
                ..Default::default()
            },
        ),
        (
            node(SemanticValue::Null),
            ResolutionLimits {
                max_steps: 0,
                ..Default::default()
            },
        ),
    ];
    for (value, limits) in values_and_limits {
        let error = registry
            .resolve_semantic_with_limits(&value, None, limits)
            .unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
        assert_eq!(error.origin, ErrorOrigin::Semantic);
    }
    let mut deep = node(SemanticValue::Sequence(Vec::new()));
    for _ in 1..128 {
        deep = node(SemanticValue::Sequence(vec![deep]));
    }
    assert_eq!(
        SemanticNode::from_resolved(&registry.resolve_semantic(&deep, None).unwrap()),
        deep
    );
    let too_deep = node(SemanticValue::Sequence(vec![deep]));
    assert_eq!(
        registry.resolve_semantic(&too_deep, None).unwrap_err().kind,
        ResolveErrorKind::LimitExceeded
    );
    let too_long = node(SemanticValue::String("x".repeat(1_048_577)));
    assert_eq!(
        registry.resolve_semantic(&too_long, None).unwrap_err().kind,
        ResolveErrorKind::LimitExceeded
    );
}

#[test]
fn completion_ambiguity_and_default_budgets_use_existing_resolution_rules() {
    let registry = schemas(json!({
        "Choice": {"schema": {"type": "object", "anyOf": [
            {"properties": {"n": {"default": 1}}},
            {"properties": {"n": {"default": 2}}}
        ]}},
        "Text": {"schema": {"properties": {"text": {"default": "東京"}}}}
    }));
    let empty = node(SemanticValue::Mapping(BTreeMap::new()));
    assert_eq!(
        registry
            .resolve_semantic(&empty, Some("Choice"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::AmbiguousDefault
    );
    let chosen = mapping([("n", number("3"))]);
    assert_eq!(
        registry
            .resolve_semantic(&chosen, Some("Choice"))
            .unwrap()
            .to_json(),
        json!({"n": 3})
    );
    let limits = ResolutionLimits {
        max_bytes: 5,
        ..Default::default()
    };
    assert_eq!(
        registry
            .resolve_semantic_with_limits(&empty, Some("Text"), limits)
            .unwrap_err()
            .kind,
        ResolveErrorKind::LimitExceeded
    );
}

fn alias_candidate_registry(second_binding: &str) -> Schemas {
    let candidate = |binding: &str| {
        json!({"properties": {"value": {
            "default": "ok",
            "x-daml-default-tags": [{
                "path": "", "tag": "Display", "binding": binding
            }]
        }}})
    };
    schemas(json!({
        "Base": {"schema": {"$id": "urn:canonical:text", "type": "string"}},
        "AliasA": {"schema": {"$ref": "urn:canonical:text"}},
        "AliasB": {"schema": {"$ref": "urn:canonical:text"}},
        "Display": {"schema": {"$ref": "urn:canonical:text"}},
        "Constrained": {"schema": {"$ref": "urn:canonical:text", "maxLength": 3}},
        "Choice": {"schema": {
            "type": "object",
            "anyOf": [candidate("AliasA"), candidate(second_binding)]
        }}
    }))
}

#[test]
fn pure_alias_default_candidates_coalesce_under_their_shared_binding() {
    let registry = alias_candidate_registry("AliasB");
    let input = parse("!Choice").unwrap();
    let resolved = registry.resolve(&input).unwrap();
    assert_eq!(resolved.to_json(), json!({"value": "ok"}));
    let value = resolved.root().get("value").unwrap();
    assert_eq!(value.tag(), Some("Display"));

    let alias_input = parse("!AliasA \"ok\"").unwrap();
    let alias = registry.resolve(&alias_input).unwrap();
    let binding = alias.root().tag_binding().expect("resolved alias binding");
    assert_eq!(value.tag_binding(), Some(binding));

    let exported = SemanticNode::from_resolved(resolved.root());
    let readmitted = registry.resolve_semantic(&exported, None).unwrap();
    assert_eq!(readmitted.to_json(), json!({"value": "ok"}));
    assert_eq!(
        readmitted.get("value").unwrap().tag_binding(),
        Some(binding)
    );
}

#[test]
fn constrained_alias_default_candidates_stay_ambiguous_even_when_both_accept() {
    let registry = alias_candidate_registry("Constrained");
    let accepted = parse("!Constrained \"ok\"").unwrap();
    assert_eq!(registry.resolve(&accepted).unwrap().to_json(), json!("ok"));

    let input = parse("!Choice").unwrap();
    let error = registry.resolve(&input).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::AmbiguousDefault);
    let empty = mapping([]);
    let error = registry
        .resolve_semantic(&empty, Some("Choice"))
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::AmbiguousDefault);
}

#[test]
fn canonical_binding_metadata_counts_toward_resolution_bytes() {
    let registry = schemas(json!({"X": {"schema": {
        "$id": format!("urn:bound:{}", "x".repeat(128)),
        "type": "integer"
    }}}));
    let input = parse("[\n!X 1\n!X 1\n!X 1\n!X 1\n]").unwrap();
    let limited = ResolutionLimits {
        max_bytes: 256,
        ..Default::default()
    };
    let error = registry
        .resolve_with_limits(&input, None, limited)
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
    assert!(matches!(error.origin, ErrorOrigin::Input(_)));

    let semantic = SemanticNode::from_document(&input);
    let error = registry
        .resolve_semantic_with_limits(&semantic, None, limited)
        .unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
    assert_eq!(error.origin, ErrorOrigin::Semantic);

    let sufficient = ResolutionLimits {
        max_bytes: 1024,
        ..Default::default()
    };
    let resolved = registry
        .resolve_with_limits(&input, None, sufficient)
        .unwrap();
    assert_eq!(resolved.to_json(), json!([1, 1, 1, 1]));
    let exported = SemanticNode::from_resolved(resolved.root());
    let readmitted = registry
        .resolve_semantic_with_limits(&exported, None, sufficient)
        .unwrap();
    assert_eq!(SemanticNode::from_resolved(&readmitted), exported);
}
