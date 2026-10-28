//! Ordinary staged SDK regressions retained by the independent review audit.

use daml::{
    AdmissionStage, CopyEdge, NodeSummary, Origin, ResolveErrorKind, ResolvedNode, SchemaModule,
    Schemas, SemanticNode, SemanticValue, StageEdit, StageError, StageOutcome, parse,
};
use serde_json::json;

fn schemas(types: serde_json::Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({"types": types})).unwrap()]).unwrap()
}

struct Visitor<F>(F);

impl<F> AdmissionStage for Visitor<F>
where
    F: FnMut(
        &ResolvedNode<'_>,
        &[CopyEdge],
        usize,
    ) -> Result<StageOutcome<&'static str>, &'static str>,
{
    type Error = &'static str;

    fn defer(&mut self, node: NodeSummary<'_>, _path: &str) -> Result<bool, Self::Error> {
        Ok(node.fields().any(|(key, _)| key == "ref"))
    }

    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error> {
        (self.0)(root, copies, round)
    }
}

fn visitor<F>(callback: F) -> Visitor<F>
where
    F: FnMut(
        &ResolvedNode<'_>,
        &[CopyEdge],
        usize,
    ) -> Result<StageOutcome<&'static str>, &'static str>,
{
    Visitor(callback)
}

fn copy_target(round: usize) -> StageOutcome<&'static str> {
    if round == 0 {
        StageOutcome::Edits(vec![StageEdit::Copy {
            path: "/target".into(),
            from: "/source".into(),
        }])
    } else {
        StageOutcome::Ready { meaning: None }
    }
}

#[test]
fn copied_record_still_completes_a_first_time_nested_provider_default() {
    let registry = schemas(json!({
        "Root": {"schema": {"properties": {
            "source": {"type": "object", "default": {}},
            "target": {"type": "object", "properties": {"child": {
                "type": "object", "default": {"nested": {}},
                "properties": {"nested": {}},
                "x-daml-default-tags": [{"path": "/nested", "tag": "Provider"}]
            }}, "required": ["child"]}
        }}},
        "Provider": {"schema": {
            "$id": "urn:review:provider", "type": "object",
            "properties": {"count": {"type": "integer"}},
            "required": ["count"], "default": {"count": 1}
        }}
    }));
    let document = parse("!Root {target: {ref: \"/source\"}}").unwrap();
    let mut stage = visitor(|_, _, round| Ok(copy_target(round)));
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({
            "source": {}, "target": {"child": {"nested": {"count": 1}}}
        })
    );
    let nested = resolved
        .root()
        .get("target")
        .unwrap()
        .get("child")
        .unwrap()
        .get("nested")
        .unwrap();
    assert_eq!(nested.tag(), Some("Provider"));
    let Origin::Default(location) = nested.get("count").unwrap().origin() else {
        panic!("first-time provider completion retains its real default origin");
    };
    assert_eq!(location.resource, "urn:review:provider");
    assert_eq!(location.pointer, "/default");
}

#[test]
fn callback_binding_matches_checked_resolution_and_survives_snapshot_replacement() {
    let registry = schemas(json!({
        "Base": {"schema": {"$id": "urn:review:text", "type": "string"}},
        "AliasA": {"schema": {"$ref": "urn:review:text"}},
        "Display": {"schema": {"$ref": "urn:review:text"}},
        "Root": {"schema": {"type": "object", "properties": {"value": {
            "default": "ok", "x-daml-default-tags": [{
                "path": "", "tag": "Display", "binding": "AliasA"
            }]
        }}}}
    }));
    let document = parse("!Root").unwrap();
    let checked = registry.resolve(&document).unwrap();
    let checked_value = checked.root().get("value").unwrap();
    let binding = checked_value.tag_binding().unwrap().to_owned();
    let mut stage = visitor(|root, _, round| {
        if round == 0 {
            let value = root.get("value").unwrap();
            assert_eq!(value.tag(), Some("Display"));
            assert_eq!(value.tag_binding(), Some(binding.as_str()));
            assert_eq!(value.origin(), checked_value.origin());
            let mut snapshot = SemanticNode::from_resolved(root);
            let SemanticValue::Mapping(fields) = &mut snapshot.value else {
                panic!("root record")
            };
            fields.insert(
                "extra".into(),
                SemanticNode {
                    tag: None,
                    value: SemanticValue::Bool(true),
                },
            );
            Ok(StageOutcome::Edits(vec![StageEdit::Replace {
                path: "".into(),
                value: snapshot,
            }]))
        } else {
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({"value": "ok", "extra": true})
    );
    assert_eq!(
        resolved.root().get("value").unwrap().tag_binding(),
        Some(binding.as_str())
    );
}

#[test]
fn ignored_opaque_preset_does_not_conflict_with_an_ordinary_record_preset() {
    for reverse in [false, true] {
        let mut presets = vec![
            json!({"default": {"a": 1}}),
            json!({"default": {"ref": "/source"}}),
        ];
        if reverse {
            presets.reverse();
        }
        let registry = schemas(json!({"Root": {"schema": {"properties": {
            "source": {"default": 3},
            "slot": {"type": "object", "allOf": presets}
        }}}}));
        let document = parse("!Root {slot: {}}").unwrap();
        let mut stage = visitor(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
        let resolved = registry
            .resolve_staged(&document, None, &[], &mut stage)
            .unwrap();
        assert_eq!(
            resolved.root().to_json(),
            json!({"source": 3, "slot": {"a": 1}})
        );
    }
}

#[test]
fn exact_number_or_origin_only_copy_is_progress_with_default_number_class() {
    let registry = schemas(json!({}));
    for original_target in ["1", "1.0"] {
        let document = parse(format!("{{source: 1.0\ntarget: {original_target}}}")).unwrap();
        let checked = registry.resolve(&document).unwrap();
        let source = checked.root().get("source").unwrap();
        assert_ne!(
            source.origin(),
            checked.root().get("target").unwrap().origin()
        );
        let mut stage = visitor(|_, copies, round| {
            if round > 0 {
                assert_eq!(copies.len(), 1);
                assert_eq!(copies[0].path, "/target");
                assert_eq!(copies[0].from, "/source");
            }
            Ok(copy_target(round))
        });
        let resolved = registry
            .resolve_staged(&document, None, &[], &mut stage)
            .unwrap();
        let target = resolved.root().get("target").unwrap();
        assert_eq!(target.to_json().to_string(), source.to_json().to_string());
        assert_eq!(target.origin(), source.origin());
    }
}

#[test]
fn a_new_compatible_constraint_is_progress_and_survives_later_edits() {
    let registry = schemas(json!({
        "A": {"schema": {"type": "integer", "minimum": 1}},
        "B": {"schema": {"type": "integer", "maximum": 10}}
    }));
    let document = parse("!A 1").unwrap();
    for continuation in ["ready", "bad-value", "repeat-constraint"] {
        let mut stage = visitor(|_, _, round| {
            if round == 0 || (round == 1 && continuation == "repeat-constraint") {
                let value = SemanticNode::from_document(&parse("!B 1").unwrap());
                return Ok(StageOutcome::Edits(vec![StageEdit::Replace {
                    path: "".into(),
                    value,
                }]));
            }
            if round == 1 && continuation == "bad-value" {
                let value = SemanticNode::from_document(&parse("11").unwrap());
                return Ok(StageOutcome::Edits(vec![StageEdit::Replace {
                    path: "".into(),
                    value,
                }]));
            }
            Ok(StageOutcome::Ready { meaning: None })
        });
        let result = registry.resolve_staged(&document, None, &[], &mut stage);
        if continuation == "ready" {
            let resolved = result.unwrap();
            assert_eq!(resolved.root().to_json(), json!(1));
            assert_eq!(resolved.root().tag(), Some("A"));
        } else {
            let StageError::Resolve(error) = result.unwrap_err() else {
                panic!("stage constraint outcome")
            };
            let expected = if continuation == "bad-value" {
                ResolveErrorKind::SchemaViolation
            } else {
                ResolveErrorKind::InvalidStage
            };
            assert_eq!(error.kind, expected);
        }
    }
}
