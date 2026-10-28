//! Staged completion keeps opaque payloads, assertion views, and work budgets intact.

use daml::{
    AdmissionStage, CopyEdge, ErrorOrigin, NodeSummary, Origin, ResolutionLimits, ResolveErrorKind,
    ResolvedNode, SchemaModule, Schemas, SemanticNode, SemanticValue, StageEdit, StageError,
    StageOutcome, SummaryValue, parse,
};
use serde_json::json;

fn schemas(types: serde_json::Value) -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({"types": types})).unwrap()]).unwrap()
}

struct Visitor<F> {
    materialize: F,
    defer_refs: bool,
    classify_numbers: bool,
}

impl<F> Visitor<F>
where
    F: FnMut(
        &ResolvedNode<'_>,
        &[CopyEdge],
        usize,
    ) -> Result<StageOutcome<&'static str>, &'static str>,
{
    fn new(materialize: F) -> Self {
        Self {
            materialize,
            defer_refs: false,
            classify_numbers: false,
        }
    }
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
        Ok(self.defer_refs && node.fields().any(|(key, _)| key == "ref"))
    }

    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error> {
        (self.materialize)(root, copies, round)
    }

    fn number_class(&mut self, lexeme: &str) -> Result<u32, Self::Error> {
        Ok(u32::from(
            self.classify_numbers
                && lexeme
                    .bytes()
                    .any(|byte| matches!(byte, b'.' | b'e' | b'E')),
        ))
    }
}

fn replace(path: &str, raw: &str) -> StageEdit {
    StageEdit::Replace {
        path: path.to_owned(),
        value: SemanticNode::from_document(&parse(raw).unwrap()),
    }
}

#[test]
fn source_and_semantic_stages_complete_before_copying_and_keep_real_origins() {
    let registry = schemas(json!({"Root": {"schema": {
        "type": "object", "default": {"target": {"context": 1}},
        "properties": {
            "source": {"type": "integer", "default": 7},
            "target": {"type": "integer"}
        }, "required": ["source", "target"]
    }}}));
    let document = parse("!Root { target: { ref: \"/source\" } }").unwrap();
    let semantic = SemanticNode::from_document(&document);
    for semantic_input in [false, true] {
        let mut stage = Visitor::new(|root, copies, round| {
            assert_eq!(root.get("source").unwrap().to_json(), json!(7));
            if round == 0 {
                assert_eq!(
                    root.get("target").unwrap().to_json(),
                    json!({"ref": "/source"})
                );
                assert!(copies.is_empty());
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
                    root.get("target").unwrap().origin(),
                    root.get("source").unwrap().origin()
                );
                Ok(StageOutcome::Ready {
                    meaning: Some(b"copy-plan".to_vec()),
                })
            }
        });
        stage.defer_refs = true;
        let resolved = if semantic_input {
            registry.resolve_semantic_staged(&semantic, None, &[], &mut stage)
        } else {
            registry.resolve_staged(&document, None, &[], &mut stage)
        }
        .unwrap();
        assert_eq!(resolved.root().to_json(), json!({"source": 7, "target": 7}));
        assert_eq!(resolved.meaning(), Some(b"copy-plan".as_slice()));
        assert!(matches!(
            resolved.root().get("target").unwrap().origin(),
            Origin::Default(_)
        ));
        assert_eq!(
            resolved.root().origin(),
            if semantic_input {
                Origin::Semantic
            } else {
                Origin::Input(document.root().span)
            }
        );
    }
}

#[test]
fn shallow_summary_exposes_exact_own_scalar_and_only_direct_container_lengths() {
    struct Inspect;
    impl AdmissionStage for Inspect {
        type Error = &'static str;
        fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> Result<bool, Self::Error> {
            if path.is_empty() {
                assert_eq!(node.value(), SummaryValue::Mapping(3));
                assert_eq!(node.tag(), Some("Root"));
                let fields = node.fields().collect::<std::collections::BTreeMap<_, _>>();
                assert_eq!(
                    fields["big"],
                    SummaryValue::Number("900719925474099312345678901")
                );
                assert_eq!(fields["nested"], SummaryValue::Mapping(1));
                assert_eq!(fields["list"], SummaryValue::Sequence(2));
            }
            if path == "/big" {
                assert_eq!(
                    node.value(),
                    SummaryValue::Number("900719925474099312345678901")
                );
                assert_eq!(node.fields().count(), 0);
            }
            Ok(false)
        }
        fn materialize(
            &mut self,
            _root: &ResolvedNode<'_>,
            _copies: &[CopyEdge],
            _round: usize,
        ) -> Result<StageOutcome<Self::Error>, Self::Error> {
            Ok(StageOutcome::Ready { meaning: None })
        }
    }
    let registry = schemas(json!({"Root": {"schema": {"type": "object"}}}));
    let document =
        parse("!Root { big: 900719925474099312345678901\nnested: {leaf: true}\nlist: [1\n2] }")
            .unwrap();
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut Inspect)
        .unwrap();
    assert_eq!(
        resolved.root().get("nested").unwrap().to_json(),
        json!({"leaf": true})
    );
}

#[test]
fn copied_default_tag_is_supplied_for_precedence_without_fabricating_origin() {
    let registry = schemas(json!({
        "Root": {"schema": {"properties": {
            "source": {"default": {"own": 2}, "x-daml-default-tags": [{"path": "", "tag": "Provider"}]},
            "target": {"default": {"context": true, "own": 1}}
        }}},
        "Provider": {"schema": {"type": "object"}}
    }));
    let document = parse("!Root").unwrap();
    let mut stage = Visitor::new(|_, _, round| {
        Ok(if round == 0 {
            StageOutcome::Edits(vec![StageEdit::Copy {
                path: "/target".into(),
                from: "/source".into(),
            }])
        } else {
            StageOutcome::Ready { meaning: None }
        })
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    let source = resolved.root().get("source").unwrap();
    let target = resolved.root().get("target").unwrap();
    assert_eq!(target.to_json(), json!({"own": 2}));
    assert_eq!(target.tag(), Some("Provider"));
    assert_eq!(target.origin(), source.origin());
    assert_eq!(target.tag_origin(), source.tag_origin());
    assert!(matches!(target.origin(), Origin::Default(_)));
    assert!(matches!(target.tag_origin(), Some(Origin::Default(_))));
}

#[test]
fn copy_obligations_survive_transitive_copy_and_later_replacement() {
    let registry = schemas(json!({
        "Source": {"schema": {"type": "integer", "minimum": 5}},
        "Destination": {"schema": {"type": "integer", "maximum": 10}},
        "Final": {"schema": {"type": "integer", "minimum": 1}}
    }));
    let document = parse("{ source: !Source 7\ndest: !Destination 8\nfinal: !Final 9 }").unwrap();
    let original = registry.resolve(&document).unwrap();
    for violate in [false, true] {
        let mut stage = Visitor::new(|root, _, round| {
            Ok(match round {
                0 => StageOutcome::Edits(vec![StageEdit::Copy {
                    path: "/dest".into(),
                    from: "/source".into(),
                }]),
                1 => StageOutcome::Edits(vec![StageEdit::Copy {
                    path: "/final".into(),
                    from: "/dest".into(),
                }]),
                2 if violate => StageOutcome::Edits(vec![replace("/final", "3")]),
                _ => {
                    assert_eq!(root.get("dest").unwrap().tag(), Some("Destination"));
                    assert_eq!(root.get("final").unwrap().tag(), Some("Final"));
                    StageOutcome::Ready { meaning: None }
                }
            })
        });
        let result = registry.resolve_staged(&document, None, &[], &mut stage);
        if violate {
            let StageError::Resolve(error) = result.unwrap_err() else {
                panic!("native assertion violation")
            };
            assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
            assert_eq!(error.path, "/final");
            assert_eq!(error.origin, ErrorOrigin::Semantic);
        } else {
            let result = result.unwrap();
            assert_eq!(
                result.root().to_json(),
                json!({"source": 7, "dest": 7, "final": 7})
            );
            assert_eq!(
                result.root().get("dest").unwrap().origin(),
                original.root().get("source").unwrap().origin()
            );
            assert_eq!(
                result.root().get("dest").unwrap().tag_origin(),
                original.root().get("dest").unwrap().tag_origin()
            );
            assert_eq!(
                result.root().get("final").unwrap().tag_origin(),
                original.root().get("final").unwrap().tag_origin()
            );
        }
    }
}

#[test]
fn number_class_refines_recursive_defaults_and_final_candidates_only() {
    for keyword in ["allOf", "anyOf"] {
        let mut schema = json!({"type": "object"});
        schema[keyword] = serde_json::from_str(
            r#"[{"properties":{"n":{"default":1}}},{"properties":{"n":{"default":1.0}}}]"#,
        )
        .unwrap();
        let registry = schemas(json!({"Choice": {"schema": schema}}));
        let document = parse("!Choice").unwrap();
        let mut ordinary = Visitor::new(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
        let ordinary = registry
            .resolve_staged(&document, None, &[], &mut ordinary)
            .unwrap();
        let number = ordinary.root().get("n").unwrap().to_json();
        assert!(number == json!(1) || number == json!(1.0));
        let mut classified = Visitor::new(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
        classified.classify_numbers = true;
        let StageError::Resolve(error) = registry
            .resolve_staged(&document, None, &[], &mut classified)
            .unwrap_err()
        else {
            panic!("class-aware ambiguity")
        };
        assert_eq!(error.kind, ResolveErrorKind::AmbiguousDefault);
    }
    let registry = schemas(json!({"Unique": {"schema": {"type": "array", "uniqueItems": true}}}));
    let document = parse("[1\n1.0]").unwrap();
    let mut stage = Visitor::new(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
    stage.classify_numbers = true;
    let StageError::Resolve(error) = registry
        .resolve_staged(&document, Some("Unique"), &[], &mut stage)
        .unwrap_err()
    else {
        panic!("native mathematical uniqueItems")
    };
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
}

#[test]
fn selected_meaning_excludes_ready_keys_from_native_invalid_candidates() {
    let registry = schemas(json!({"Choice": {"schema": {"anyOf": [
        {"properties": {"n": {"default": 0, "minimum": 1}}},
        {"properties": {"n": {"default": 2, "minimum": 1}}}
    ]}}}));
    let document = parse("!Choice").unwrap();
    let mut keys = Vec::new();
    let mut stage = Visitor::new(|root, _, _| {
        let key = root.get("n").unwrap().to_json().to_string().into_bytes();
        keys.push(key.clone());
        Ok(StageOutcome::Ready { meaning: Some(key) })
    });
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap();
    assert_eq!(resolved.root().to_json(), json!({"n": 2}));
    assert_eq!(resolved.meaning(), Some(b"2".as_slice()));
    assert!(keys.contains(&b"0".to_vec()) && keys.contains(&b"2".to_vec()));
}

#[test]
fn callback_error_is_fatal_but_explicit_rejection_is_preserved() {
    let registry = schemas(json!({}));
    let document = parse("null").unwrap();
    let mut fatal = Visitor::new(|_, _, _| Err("original callback error"));
    assert!(matches!(
        registry.resolve_staged(&document, None, &[], &mut fatal),
        Err(StageError::Callback("original callback error"))
    ));
    let mut rejected = Visitor::new(|_, _, _| Ok(StageOutcome::Reject("original rejection")));
    assert!(matches!(
        registry.resolve_staged(&document, None, &[], &mut rejected),
        Err(StageError::Rejected("original rejection"))
    ));
}

#[test]
fn oscillating_edits_and_ready_meanings_share_the_cumulative_work_budget() {
    let registry = schemas(json!({}));
    let document = parse("{flag: false}").unwrap();
    let mut stage = Visitor::new(|root, _, round| {
        assert!(round < 128, "work must not reset on each continuation");
        let raw = if root.get("flag").unwrap().to_json() == json!(false) {
            "true"
        } else {
            "false"
        };
        Ok(StageOutcome::Edits(vec![replace("/flag", raw)]))
    });
    let limits = ResolutionLimits {
        max_steps: 64,
        ..Default::default()
    };
    let StageError::Resolve(error) = registry
        .resolve_staged_with_limits(&document, None, &[], limits, &mut stage)
        .unwrap_err()
    else {
        panic!("bounded staged work")
    };
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);

    let semantic = SemanticNode {
        tag: None,
        value: SemanticValue::Null,
    };
    let mut stage = Visitor::new(|_, _, _| {
        Ok(StageOutcome::Ready {
            meaning: Some(vec![0; 256]),
        })
    });
    let limits = ResolutionLimits {
        max_bytes: 128,
        ..Default::default()
    };
    let StageError::Resolve(error) = registry
        .resolve_semantic_staged_with_limits(&semantic, None, &[], limits, &mut stage)
        .unwrap_err()
    else {
        panic!("bounded meaning retention")
    };
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
    assert_eq!(error.origin, ErrorOrigin::Semantic);
}

#[test]
fn initial_replace_and_expect_are_conjunctive_and_precede_completion() {
    let registry = schemas(json!({
        "Host": {"schema": {"properties": {
            "target": {"type": "object", "properties": {"count": {"maximum": 10}}}
        }}},
        "Actual": {"schema": {
            "type": "object", "properties": {"count": {"type": "integer", "default": 4}},
            "required": ["count"], "additionalProperties": false
        }}
    }));
    let document = parse("!Host {target: {authored: true}}").unwrap();
    let initial = [
        replace("/target", "{}"),
        StageEdit::Expect {
            path: "/target".into(),
            name: "Actual".into(),
        },
    ];
    let mut stage = Visitor::new(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
    let resolved = registry
        .resolve_staged(&document, None, &initial, &mut stage)
        .unwrap();
    assert_eq!(resolved.root().to_json(), json!({"target": {"count": 4}}));
    let mut stage = Visitor::new(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
    let StageError::Resolve(error) = registry
        .resolve_staged(&document, None, &initial[1..], &mut stage)
        .unwrap_err()
    else {
        panic!("actual target rejects unknown arguments")
    };
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
}

#[test]
fn caller_edit_contract_errors_are_not_schema_failures_or_implicit_parent_creation() {
    let registry = schemas(json!({}));
    let document = parse("{}").unwrap();
    let edits = [replace("/missing/leaf", "1")];
    let mut stage = Visitor::new(|_, _, _| Ok(StageOutcome::Ready { meaning: None }));
    let StageError::Resolve(error) = registry
        .resolve_staged(&document, None, &edits, &mut stage)
        .unwrap_err()
    else {
        panic!("invalid caller edit");
    };
    assert_eq!(error.kind, ResolveErrorKind::InvalidStage);
}

#[test]
fn repeated_copies_charge_imports_and_history_without_resetting_bytes() {
    let registry = schemas(json!({}));
    let document = parse(format!(
        "{{a: \"{}\"\nb: \"{}\"\ntarget: null}}",
        "a".repeat(256),
        "b".repeat(256)
    ))
    .unwrap();
    let mut stage = Visitor::new(|_, _, round| {
        assert!(
            round < 32,
            "copy continuation must retain cumulative budget"
        );
        Ok(StageOutcome::Edits(vec![StageEdit::Copy {
            path: "/target".into(),
            from: if round % 2 == 0 { "/a" } else { "/b" }.into(),
        }]))
    });
    let limits = ResolutionLimits {
        max_bytes: 1024,
        ..Default::default()
    };
    let StageError::Resolve(error) = registry
        .resolve_staged_with_limits(&document, None, &[], limits, &mut stage)
        .unwrap_err()
    else {
        panic!("bounded copy/import/history work");
    };
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
}

#[test]
fn initial_replacement_cannot_bypass_input_numeric_and_tag_admission() {
    let registry = schemas(json!({}));
    let initial = [replace("", "0")];
    let excessive = parse("1e513").unwrap();
    let semantic = SemanticNode::from_document(&excessive);
    let mut stage = Visitor::new(|_, _, _| Err("unadmitted input reached materializer"));
    let StageError::Resolve(error) = registry
        .resolve_semantic_staged(&semantic, None, &initial, &mut stage)
        .unwrap_err()
    else {
        panic!("numeric profile admission precedes edits");
    };
    assert_eq!(error.kind, ResolveErrorKind::NumericLimit);
    assert_eq!(error.origin, ErrorOrigin::Semantic);

    let unknown = parse("!Missing 1").unwrap();
    let mut stage = Visitor::new(|_, _, _| Err("unadmitted input reached materializer"));
    let StageError::Resolve(error) = registry
        .resolve_staged(&unknown, None, &initial, &mut stage)
        .unwrap_err()
    else {
        panic!("literal binding admission precedes edits");
    };
    assert_eq!(error.kind, ResolveErrorKind::UnknownTag);
    assert!(matches!(error.origin, ErrorOrigin::Input(_)));
}
