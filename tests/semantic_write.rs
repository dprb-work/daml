//! Semantic source writing preserves edited values without float or tag loss.

use daml::{
    FormatOptions, SchemaModule, Schemas, SemanticErrorKind, SemanticNode, SemanticTag,
    SemanticValue, format_semantic, parse,
};
use serde_json::Number;
use std::{collections::BTreeMap, str::FromStr};
fn options(line_width: usize) -> FormatOptions {
    FormatOptions { line_width }
}

fn mapping(entries: impl IntoIterator<Item = (String, SemanticNode)>) -> SemanticNode {
    SemanticNode {
        tag: None,
        value: SemanticValue::Mapping(entries.into_iter().collect::<BTreeMap<_, _>>()),
    }
}

fn scalar_number(value: &str) -> SemanticNode {
    SemanticNode {
        tag: None,
        value: SemanticValue::Number(Number::from_str(value).unwrap()),
    }
}

#[test]
fn edited_exact_numbers_and_unicode_strings_roundtrip_as_typed_values() {
    let edited = mapping([
        (
            "count".to_owned(),
            scalar_number("900719925474099312345678901234567890"),
        ),
        (
            "ratio".to_owned(),
            scalar_number("1234567890.01234567890123456789"),
        ),
        (
            "text".to_owned(),
            SemanticNode {
                tag: None,
                value: SemanticValue::String("line\\break\n雪 🌍".to_owned()),
            },
        ),
    ]);
    let source = format_semantic(&edited, options(80)).unwrap();
    assert!(source.ends_with('\n'));
    let reparsed = parse(&source).unwrap();
    assert_eq!(SemanticNode::from_document(&reparsed), edited);
}

#[test]
fn raw_regex_tags_keep_pattern_data_and_tag_assertions() {
    let regex = SemanticNode {
        tag: Some(SemanticTag {
            name: "Regex".to_owned(),
            binding: None,
        }),
        value: SemanticValue::String(r"\d+\s#literal".to_owned()),
    };
    let source = format_semantic(&regex, options(80)).unwrap();
    let reparsed = parse(&source).unwrap();
    assert_eq!(SemanticNode::from_document(&reparsed), regex);
}

#[test]
fn invalid_mapping_keys_and_tags_are_rejected_with_their_data_pointer() {
    let invalid_key = mapping([("key with space".to_owned(), scalar_number("1"))]);
    let error = format_semantic(&invalid_key, options(80)).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::InvalidKey);
    assert_eq!(error.pointer, "/key with space");

    let invalid_tag = SemanticNode {
        tag: Some(SemanticTag {
            name: "not/a/tag".to_owned(),
            binding: None,
        }),
        value: SemanticValue::Null,
    };
    let error = format_semantic(&invalid_tag, options(80)).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::InvalidTag);
    assert_eq!(error.pointer, "");
}

#[test]
fn qualified_tags_need_a_registry_and_invalid_width_is_rejected() {
    let qualified = SemanticNode {
        tag: Some(SemanticTag {
            name: "provider.Http".to_owned(),
            binding: Some("private.Http".to_owned()),
        }),
        value: SemanticValue::Mapping(BTreeMap::new()),
    };
    let error = format_semantic(&qualified, options(80)).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::UnrepresentableTag);
    assert_eq!(error.pointer, "");

    let error = format_semantic(
        &SemanticNode {
            tag: None,
            value: SemanticValue::String("value".to_owned()),
        },
        options(0),
    )
    .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::InvalidLineWidth);
}

#[test]
fn schema_writer_rejects_a_structurally_matching_but_unavailable_binding() {
    let schemas = Schemas::compose([SchemaModule::from_value(serde_json::json!({
        "types": {
            "Root": {"schema": {"type": "string"}}
        }
    }))
    .unwrap()])
    .unwrap();
    let qualified = SemanticNode {
        tag: Some(SemanticTag {
            name: "Root".to_owned(),
            binding: Some("urn:private:hidden".to_owned()),
        }),
        value: SemanticValue::String("same-shape value".to_owned()),
    };
    let error = schemas
        .format_semantic(&qualified, options(80))
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::UnrepresentableTag);
    assert_eq!(error.pointer, "");
}

#[test]
fn schema_writer_rejects_a_constrained_alias_but_uses_its_visible_target() {
    let module = SchemaModule::from_value(serde_json::json!({
        "types": {
            "Host": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "payload": {
                            "default": "safe",
                            "x-daml-default-tags": [{
                                "path": "",
                                "tag": "provider.Text",
                                "binding": "private.Text"
                            }]
                        }
                    }
                }
            },
            "private.Text": {
                "schema": {
                    "$id": "urn:private-text",
                    "type": "string"
                }
            },
            "provider.Text": {
                "schema": {
                    "$ref": "urn:private-text",
                    "maxLength": 4
                }
            }
        }
    }))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let document = parse("!Host").unwrap();
    let resolved = schemas.resolve(&document).unwrap();
    let mut semantic = SemanticNode::from_resolved(resolved.root());
    let SemanticValue::Mapping(entries) = &mut semantic.value else {
        panic!("Host resolution must be a mapping");
    };
    let payload = entries.get_mut("payload").expect("defaulted payload");
    assert_eq!(
        payload.tag.as_ref().map(|tag| tag.name.as_str()),
        Some("provider.Text")
    );
    let binding = payload
        .tag
        .as_ref()
        .and_then(|tag| tag.binding.as_deref())
        .expect("default keeps native binding")
        .to_owned();
    assert_eq!(binding, "private.Text");
    payload.value = SemanticValue::String("too-long".to_owned());

    let source = schemas.format_semantic(&semantic, options(80)).unwrap();
    let emitted = parse(&source).unwrap();
    let emitted_semantic = SemanticNode::from_document(&emitted);
    let SemanticValue::Mapping(emitted_entries) = &emitted_semantic.value else {
        panic!("writer output must preserve the Host mapping");
    };
    let emitted_payload = emitted_entries.get("payload").expect("written payload");
    assert_eq!(
        emitted_payload.tag.as_ref().map(|tag| tag.name.as_str()),
        Some("private.Text")
    );
    assert_eq!(
        emitted_payload.value,
        SemanticValue::String("too-long".to_owned())
    );

    let rebound = schemas.resolve(&emitted).unwrap();
    let rebound_semantic = SemanticNode::from_resolved(rebound.root());
    let SemanticValue::Mapping(rebound_entries) = &rebound_semantic.value else {
        panic!("re-resolved Host must be a mapping");
    };
    let rebound_payload = rebound_entries.get("payload").expect("re-resolved payload");
    assert_eq!(
        rebound_payload.tag.as_ref().map(|tag| tag.name.as_str()),
        Some("private.Text")
    );
    assert_eq!(
        rebound_payload.value,
        SemanticValue::String("too-long".to_owned())
    );
}

#[test]
fn semantic_writer_rejects_serialized_scalar_tokens_over_the_parser_bound() {
    let string = "\n".repeat(600_000);
    let value = SemanticNode {
        tag: None,
        value: SemanticValue::String(string),
    };
    let error = format_semantic(&value, options(80)).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::LimitExceeded);
    assert_eq!(error.pointer, "");
}

#[test]
fn semantic_writer_accepts_depth_128_and_rejects_depth_129() {
    fn nested_empty_arrays(depth: usize) -> SemanticNode {
        let mut value = SemanticNode {
            tag: None,
            value: SemanticValue::Sequence(Vec::new()),
        };
        for _ in 1..depth {
            value = SemanticNode {
                tag: None,
                value: SemanticValue::Sequence(vec![value]),
            };
        }
        value
    }

    let at_limit = format_semantic(&nested_empty_arrays(128), options(80)).unwrap();
    assert!(
        parse(&at_limit).is_ok(),
        "the accepted depth boundary must be readable"
    );
    let error = format_semantic(&nested_empty_arrays(129), options(80)).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::LimitExceeded);
}

#[test]
fn semantic_writer_rejects_a_sequence_over_the_node_budget() {
    let children = (0..100_000)
        .map(|_| SemanticNode {
            tag: None,
            value: SemanticValue::Null,
        })
        .collect();
    let value = SemanticNode {
        tag: None,
        value: SemanticValue::Sequence(children),
    };
    let error = format_semantic(&value, options(80)).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::LimitExceeded);
}
