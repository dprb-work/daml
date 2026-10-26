//! Regression checks for staged continuation transitions after opacity and Copy.

use daml::{
    AdmissionStage, CopyEdge, NodeSummary, Origin, ResolvedNode, SchemaModule, Schemas,
    SemanticNode, StageEdit, StageOutcome, SummaryValue, parse,
};
use serde_json::json;

fn schemas(types: serde_json::Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({"types": types})).unwrap()]).unwrap()
}

struct Visitor<F> {
    callback: F,
    defer_hold_tag: bool,
}

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
        Ok((self.defer_hold_tag && node.tag() == Some("Hold"))
            || node
                .fields()
                .any(|(key, value)| key == "ref" && matches!(value, SummaryValue::String(_))))
    }

    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error> {
        (self.callback)(root, copies, round)
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
    Visitor {
        callback,
        defer_hold_tag: false,
    }
}

fn hold_visitor<F>(callback: F) -> Visitor<F>
where
    F: FnMut(
        &ResolvedNode<'_>,
        &[CopyEdge],
        usize,
    ) -> Result<StageOutcome<&'static str>, &'static str>,
{
    Visitor {
        callback,
        defer_hold_tag: true,
    }
}

struct SourcePathVisitor<F>(F);

impl<F> AdmissionStage for SourcePathVisitor<F>
where
    F: FnMut(
        &ResolvedNode<'_>,
        &[CopyEdge],
        usize,
    ) -> Result<StageOutcome<&'static str>, &'static str>,
{
    type Error = &'static str;

    fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> Result<bool, Self::Error> {
        Ok(path.starts_with("/source/")
            && node
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

#[test]
fn replacing_an_opaque_subtree_restores_ancestor_preset_without_contaminating_copy() {
    let registry = schemas(json!({"Root": {"schema": {
        "$id": "urn:transition:root", "type": "object",
        "properties": {"source": {"type": "object"}, "copy": {}, "target": {}},
        "default": {
            "copy": {"contamination": true},
            "target": {"context": 1}
        }
    }}}));
    let document =
        parse("!Root {source: {own: 9}\ncopy: {ref: \"/source\"}\ntarget: {ref: \"runtime\"}}")
            .unwrap();
    let mut stage = visitor(|root, copies, round| {
        if round == 0 {
            assert_eq!(
                root.get("copy").unwrap().to_json(),
                json!({"ref": "/source"})
            );
            assert_eq!(
                root.get("target").unwrap().to_json(),
                json!({"ref": "runtime"})
            );
            Ok(StageOutcome::Edits(vec![
                StageEdit::Copy {
                    path: "/copy".into(),
                    from: "/source".into(),
                },
                StageEdit::Replace {
                    path: "/target".into(),
                    value: SemanticNode::from_document(&parse("{}").unwrap()),
                },
            ]))
        } else {
            assert_eq!(round, 1);
            assert_eq!(copies.len(), 1);
            assert_eq!(copies[0].path, "/copy");
            assert_eq!(copies[0].from, "/source");
            assert_eq!(root.get("copy").unwrap().to_json(), json!({"own": 9}));
            assert_eq!(root.get("target").unwrap().to_json(), json!({"context": 1}));
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({"source": {"own": 9}, "copy": {"own": 9}, "target": {"context": 1}})
    );
    assert_eq!(
        resolved.root().get("copy").unwrap().origin(),
        resolved.root().get("source").unwrap().origin()
    );
    let target = resolved.root().get("target").unwrap();
    let Origin::Default(location) = target.get("context").unwrap().origin() else {
        panic!("the resumed target preset retains its real root-default origin");
    };
    assert_eq!(location.resource, "urn:transition:root");
    assert_eq!(location.pointer, "/default");
}

#[test]
fn unblocking_a_deferred_descendant_inside_a_copy_applies_its_first_preset_only() {
    let registry = schemas(json!({
        "Root": {"schema": {
            "$id": "urn:transition:copy-root", "type": "object",
            "properties": {
                "source": {"type": "object"},
                "target": {"type": "object"}
            },
            "default": {"target": {"context": true}}
        }},
        "Provider": {"schema": {
            "$id": "urn:transition:provider", "type": "object",
            "properties": {
                "ref": {}, "count": {"type": "integer"}
            },
            "default": {"count": 1}
        }}
    }));
    let document = parse(
        "!Root {source: {child: !Provider {ref: \"runtime\"}\nsibling: !Provider {count: 9}}\ntarget: {}}",
    )
    .unwrap();
    // The first callback after Copy unblocks the child. A further snapshot
    // proves its own default resumes without replaying ancestor context into
    // the completed copied sibling or changing that sibling's provenance.
    let mut snapshots = 0;
    let mut stage = visitor(|root, copies, round| {
        snapshots += 1;
        if round == 0 {
            assert!(copies.is_empty());
            Ok(StageOutcome::Edits(vec![StageEdit::Copy {
                path: "/target".into(),
                from: "/source".into(),
            }]))
        } else if round == 1 {
            assert_eq!(copies.len(), 1);
            assert_eq!(root.get("target").unwrap().to_json()["sibling"]["count"], 9);
            Ok(StageOutcome::Edits(vec![StageEdit::Replace {
                path: "/target/child/ref".into(),
                value: SemanticNode::from_document(&parse("null").unwrap()),
            }]))
        } else {
            assert_eq!(round, 2);
            assert_eq!(copies.len(), 1);
            assert_eq!(
                root.get("target").unwrap().to_json(),
                json!({"child": {"ref": null, "count": 1}, "sibling": {"count": 9}})
            );
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(snapshots, 3);
    assert_eq!(
        resolved.root().to_json(),
        json!({
            "source": {"child": {"ref": "runtime"}, "sibling": {"count": 9}},
            "target": {"child": {"ref": null, "count": 1}, "sibling": {"count": 9}}
        })
    );
    let source_sibling = resolved
        .root()
        .get("source")
        .unwrap()
        .get("sibling")
        .unwrap();
    let copied_sibling = resolved
        .root()
        .get("target")
        .unwrap()
        .get("sibling")
        .unwrap();
    assert_eq!(copied_sibling.origin(), source_sibling.origin());
    let resumed_count = resolved
        .root()
        .get("target")
        .unwrap()
        .get("child")
        .unwrap()
        .get("count")
        .unwrap();
    let Origin::Default(location) = resumed_count.origin() else {
        panic!("the newly unblocked provider receives its first default");
    };
    assert_eq!(location.resource, "urn:transition:provider");
    assert_eq!(location.pointer, "/default");
}

#[test]
fn copied_subtree_reclassifies_opaque_descendants_at_destination_paths() {
    let registry = schemas(json!({
        "Root": {"schema": {
            "type": "object",
            "properties": {
                "source": {"type": "object"},
                "target": {"type": "object"}
            }
        }},
        "Provider": {"schema": {
            "$id": "urn:transition:path-provider", "type": "object",
            "properties": {
                "ref": {}, "count": {"type": "integer"}
            },
            "default": {"count": 1}
        }}
    }));
    let document = parse(
        "!Root {source: {child: !Provider {ref: \"runtime\"}\nsibling: !Provider {count: 9}}\ntarget: {}}",
    )
    .unwrap();
    let mut stage = SourcePathVisitor(
        |root: &ResolvedNode<'_>, copies: &[CopyEdge], round: usize| {
            if round == 0 {
                assert!(copies.is_empty());
                assert_eq!(
                    root.get("source").unwrap().to_json(),
                    json!({"child": {"ref": "runtime"}, "sibling": {"count": 9}})
                );
                Ok(StageOutcome::Edits(vec![StageEdit::Copy {
                    path: "/target".into(),
                    from: "/source".into(),
                }]))
            } else {
                assert_eq!(round, 1);
                assert_eq!(copies.len(), 1);
                assert_eq!(copies[0].path, "/target");
                assert_eq!(copies[0].from, "/source");
                assert_eq!(
                    root.get("target").unwrap().to_json(),
                    json!({"child": {"ref": "runtime", "count": 1}, "sibling": {"count": 9}})
                );
                Ok(StageOutcome::Ready { meaning: None })
            }
        },
    );
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({
            "source": {"child": {"ref": "runtime"}, "sibling": {"count": 9}},
            "target": {"child": {"ref": "runtime", "count": 1}, "sibling": {"count": 9}}
        })
    );
    let source = resolved.root().get("source").unwrap();
    let target = resolved.root().get("target").unwrap();
    let source_child = source.get("child").unwrap();
    let copied_child = target.get("child").unwrap();
    assert_eq!(source_child.to_json(), json!({"ref": "runtime"}));
    assert_eq!(source_child.tag(), Some("Provider"));
    assert_eq!(copied_child.tag(), Some("Provider"));
    assert_eq!(copied_child.origin(), source_child.origin());
    assert_eq!(
        target.get("sibling").unwrap().origin(),
        source.get("sibling").unwrap().origin()
    );
    let Origin::Default(location) = copied_child.get("count").unwrap().origin() else {
        panic!("the destination-only admission receives the provider's first default");
    };
    assert_eq!(location.resource, "urn:transition:path-provider");
    assert_eq!(location.pointer, "/default");
}

#[test]
fn duplicate_expect_does_not_replay_contextual_defaults_into_a_copy() {
    let registry = schemas(json!({
        "Root": {"schema": {
            "$id": "urn:transition:duplicate-expect-root",
            "type": "object",
            "properties": {
                "source": {"type": "object"},
                "target": {
                    "type": "object",
                    "properties": {"context": {"type": "integer"}}
                }
            },
            "default": {"target": {"context": 1}}
        }},
        "Provider": {"schema": {"type": "object"}}
    }));
    let document = parse("!Root {source: {}\ntarget: {}}").unwrap();
    let initial = [StageEdit::Expect {
        path: "/target".into(),
        name: "Provider".into(),
    }];
    let mut stage = visitor(|root, copies, round| {
        if round == 0 {
            assert!(copies.is_empty());
            Ok(StageOutcome::Edits(vec![
                StageEdit::Copy {
                    path: "/target".into(),
                    from: "/source".into(),
                },
                StageEdit::Expect {
                    path: "/target".into(),
                    name: "Provider".into(),
                },
            ]))
        } else {
            assert_eq!(round, 1);
            assert_eq!(copies.len(), 1);
            assert_eq!(copies[0].path, "/target");
            assert_eq!(copies[0].from, "/source");
            assert_eq!(root.get("target").unwrap().to_json(), json!({}));
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, &initial, &mut stage)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({"source": {}, "target": {}})
    );
    assert_eq!(
        resolved.root().get("target").unwrap().origin(),
        resolved.root().get("source").unwrap().origin()
    );
}

#[test]
fn new_expectation_activates_after_copy_in_either_edit_order() {
    let registry = schemas(json!({
        "A": {"schema": {"type": "object"}},
        "B": {"schema": {
            "$id": "urn:transition:order-b",
            "type": "object",
            "default": {"context": 1}
        }}
    }));
    for expect_first in [false, true] {
        let document = parse("{source: !A {}\ntarget: !B {}}").unwrap();
        let mut stage = visitor(|root, copies, round| {
            if round == 0 {
                assert!(copies.is_empty());
                let copy = StageEdit::Copy {
                    path: "/target".into(),
                    from: "/source".into(),
                };
                let expect = StageEdit::Expect {
                    path: "/target".into(),
                    name: "A".into(),
                };
                let edits = if expect_first {
                    vec![expect, copy]
                } else {
                    vec![copy, expect]
                };
                Ok(StageOutcome::Edits(edits))
            } else {
                assert_eq!(round, 1);
                assert_eq!(copies.len(), 1);
                assert_eq!(copies[0].path, "/target");
                assert_eq!(copies[0].from, "/source");
                assert_eq!(root.get("target").unwrap().to_json(), json!({"context": 1}));
                Ok(StageOutcome::Ready { meaning: None })
            }
        });
        let resolved = registry
            .resolve_staged(&document, None, &[], &mut stage)
            .unwrap();
        let source = resolved.root().get("source").unwrap();
        let target = resolved.root().get("target").unwrap();
        assert_eq!(source.to_json(), json!({}));
        assert_eq!(source.tag(), Some("A"));
        assert_eq!(target.to_json(), json!({"context": 1}));
        assert_eq!(target.tag(), Some("B"));
        assert_eq!(target.origin(), source.origin());
        let Origin::Default(location) = target.get("context").unwrap().origin() else {
            panic!("the new conjunctive expectation resumes B's first default");
        };
        assert_eq!(location.resource, "urn:transition:order-b");
        assert_eq!(location.pointer, "/default");
    }
}

#[test]
fn replacement_root_uses_restored_supplied_tag_for_opacity() {
    let registry = schemas(json!({"Hold": {"schema": {
        "$id": "urn:transition:hold", "type": "object",
        "properties": {"value": {"type": "integer"}, "context": {"type": "integer"}},
        "default": {"context": 1}
    }}}));
    let mut supplied_binding = None;
    let document = parse("!Hold {value: 0}").unwrap();
    let mut stage = hold_visitor(|root, _, round| {
        if round == 0 {
            supplied_binding = Some(root.tag_binding().unwrap().to_owned());
            assert_eq!(root.to_json(), json!({"value": 0}));
            Ok(StageOutcome::Edits(vec![StageEdit::Replace {
                path: "".into(),
                value: SemanticNode::from_document(&parse("{value: 1}").unwrap()),
            }]))
        } else {
            assert_eq!(root.tag_binding(), supplied_binding.as_deref());
            assert_eq!(root.to_json(), json!({"value": 1}));
            Ok(StageOutcome::Ready { meaning: None })
        }
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(resolved.root().tag_binding(), supplied_binding.as_deref());
    assert_eq!(resolved.root().to_json(), json!({"value": 1}));
    assert_eq!(resolved.root().tag(), Some("Hold"));
    assert_eq!(resolved.root().get("context"), None);
}
