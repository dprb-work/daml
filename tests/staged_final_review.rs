//! Changed ancestor summaries and completion eligibility at supported edit paths.

use daml::{
    AdmissionStage, CopyEdge, NodeSummary, Origin, ResolveErrorKind, ResolvedNode, SchemaModule,
    Schemas, SemanticNode, StageEdit, StageError, StageOutcome, SummaryValue, parse,
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
        Ok(node
            .fields()
            .any(|(key, value)| key == "ref" && matches!(value, SummaryValue::String(_))))
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

fn contextual_root() -> Schemas {
    schemas(json!({"Root": {"schema": {
        "$id": "urn:final-review:root", "type": "object",
        "properties": {
            "source": {"type": "integer"},
            "target": {"default": {"context": 1}}
        }, "required": ["source", "target"]
    }}}))
}

fn replace_leaf(raw: &str) -> StageEdit {
    StageEdit::Replace {
        path: "/target/ref".into(),
        value: SemanticNode::from_document(&parse(raw).unwrap()),
    }
}

fn assert_atomic_reference_copy(source: &str, initial: &[StageEdit]) {
    let registry = contextual_root();
    let document = parse(source).unwrap();
    let mut stage = visitor(|root, _, round| {
        if round == 0 {
            assert_eq!(
                root.get("target").unwrap().to_json(),
                json!({"ref": "/source"})
            );
            Ok(StageOutcome::Edits(vec![StageEdit::Copy {
                path: "/target".into(),
                from: "/source".into(),
            }]))
        } else {
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, initial, &mut stage)
        .unwrap();
    assert_eq!(resolved.root().to_json(), json!({"source": 7, "target": 7}));
}

#[test]
fn original_input_reference_is_atomic_before_contextual_defaults_control() {
    assert_atomic_reference_copy("!Root {source: 7\ntarget: {ref: \"/source\"}}", &[]);
}

#[test]
fn initial_leaf_insertion_reclassifies_its_existing_record_before_defaults() {
    let initial = [replace_leaf("\"/source\"")];
    assert_atomic_reference_copy("!Root {source: 7\ntarget: {}}", &initial);
}

#[test]
fn callback_undefer_recovers_ancestor_preset_without_replaying_into_copied_record() {
    let registry = schemas(json!({"Root": {"schema": {
        "$id": "urn:final-review:combined", "type": "object",
        "properties": {"source": {"type": "object"}, "copy": {}, "target": {}},
        "default": {
            "target": {"context": 1},
            "copy": {"contamination": true}
        }
    }}}));
    let document =
        parse("!Root {source: {own: 9}\ncopy: {ref: \"/source\"}\ntarget: {ref: \"/source\"}}")
            .unwrap();
    let mut stage = visitor(|root, _, round| {
        if round == 0 {
            assert_eq!(
                root.get("copy").unwrap().to_json(),
                json!({"ref": "/source"})
            );
            assert_eq!(
                root.get("target").unwrap().to_json(),
                json!({"ref": "/source"})
            );
            Ok(StageOutcome::Edits(vec![
                StageEdit::Copy {
                    path: "/copy".into(),
                    from: "/source".into(),
                },
                replace_leaf("null"),
            ]))
        } else {
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({
            "source": {"own": 9},
            "copy": {"own": 9},
            "target": {"ref": null, "context": 1}
        })
    );
    assert_eq!(
        resolved.root().get("copy").unwrap().origin(),
        resolved.root().get("source").unwrap().origin()
    );
    let target = resolved.root().get("target").unwrap();
    assert_eq!(target.get("ref").unwrap().origin(), Origin::Semantic);
    let Origin::Default(location) = target.get("context").unwrap().origin() else {
        panic!("the newly eligible contextual field retains its real ancestor default origin");
    };
    assert_eq!(location.resource, "urn:final-review:combined");
    assert_eq!(location.pointer, "/default");
}

#[test]
fn copy_cannot_claim_a_descendant_hidden_behind_an_opaque_ancestor_is_completed() {
    let registry = schemas(json!({"Provider": {"schema": {
        "type": "object", "properties": {"count": {"type": "integer"}},
        "default": {"count": 1}
    }}}));
    // Count is deliberately optional: the old incomplete Copy could otherwise
    // be rejected incidentally by strict validation rather than eligibility.
    let document = parse("{source: {ref: \"runtime\"\nchild: !Provider {}}\ntarget: {}}").unwrap();
    let mut stage = visitor(|_, _, round| {
        Ok(if round == 0 {
            StageOutcome::Edits(vec![StageEdit::Copy {
                path: "/target".into(),
                from: "/source/child".into(),
            }])
        } else {
            StageOutcome::Ready { meaning: None }
        })
    });
    let StageError::Resolve(error) = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap_err()
    else {
        panic!("ineligible Copy source is a core stage contract error");
    };
    assert_eq!(error.kind, ResolveErrorKind::InvalidStage);
}
