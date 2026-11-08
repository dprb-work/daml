//! Public consumer contracts. Acceptance/spelling fixtures were observed with Spring 7.0.10.
use daml::{
    AdmissionStage, Bundle, CopyEdge, FormatOptions, NodeSummary, Origin, ResolvedNode,
    ResolvedValue, SchemaModule, Schemas, SemanticNode, SemanticValue, StageEdit, StageOutcome,
    SummaryValue, TemporalKind, TemporalValue, Value, format, format_semantic, parse,
};
use serde_json::json;

#[test]
fn spring_reference_acceptance_and_retained_spelling() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/spring-cron.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let expression = case["expression"].as_str().unwrap();
        let admitted = TemporalValue::parse(TemporalKind::Cron, expression);
        let source = format!("!Cron {}", serde_json::to_string(expression).unwrap());
        let parsed = parse(&source);
        if case["accepted"] == true {
            let expected = case["retained"].as_str().unwrap();
            let admitted = admitted.unwrap_or_else(|error| panic!("{expression:?}: {error:?}"));
            assert_eq!(admitted.kind(), TemporalKind::Cron);
            assert_eq!(admitted.as_str(), expected, "{expression:?}");
            assert_eq!(admitted.offset_minutes(), None);
            assert_eq!(admitted.as_borrowed().as_str(), expected);
            assert_eq!(admitted.into_owned().as_str(), expected);
            let document = parsed.unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
            assert!(matches!(document.root().value, Value::Temporal(_)));
            assert_eq!(
                document.temporal(document.root()).unwrap().as_str(),
                expected
            );
        } else {
            assert!(admitted.is_err(), "accepted direct {expression:?}");
            assert!(parsed.is_err(), "accepted intrinsic {expression:?}");
        }
    }
}

#[test]
fn cron_source_identity_formatting_and_semantic_roundtrip() {
    for source in [
        r#"!Cron "0 7 11 * * *""#,
        r#"!Cron "  @daily  ""#,
        r#"!Cron "0\u00207 11 * * *""#,
    ] {
        let document = parse(source).unwrap();
        let same_source = parse(source).unwrap();
        let unrelated = parse(r#""xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx""#).unwrap();
        assert!(same_source.temporal(document.root()).is_none());
        assert!(unrelated.temporal(document.root()).is_none());
        assert_eq!(
            format(&document, FormatOptions { line_width: 80 })
                .unwrap()
                .trim(),
            source
        );
        let semantic = SemanticNode::from_document(&document);
        assert!(matches!(semantic.value, SemanticValue::Temporal(_)));
        let written = format_semantic(&semantic, FormatOptions { line_width: 80 }).unwrap();
        assert_eq!(
            SemanticNode::from_document(&parse(written).unwrap()),
            semantic
        );
        let moved = document;
        assert_eq!(
            moved.temporal(moved.root()).unwrap().kind(),
            TemporalKind::Cron
        );
    }
    let macro_value = TemporalValue::parse(TemporalKind::Cron, "@daily").unwrap();
    assert_eq!(
        macro_value,
        TemporalValue::parse(TemporalKind::Cron, "0 0 0 * * *").unwrap()
    );
    // Equality is retained text, not occurrence equivalence or ISO fraction equality.
    assert_ne!(
        macro_value,
        TemporalValue::parse(TemporalKind::Cron, "0 0 0 ? * *").unwrap()
    );
    assert_ne!(
        macro_value,
        TemporalValue::parse(TemporalKind::Cron, "00 0 0 * * *").unwrap()
    );
    let asserted = parse(r#"!app.Schedule !Cron "@daily""#).unwrap();
    assert_eq!(asserted.tag(asserted.root()), Some("app.Schedule"));
    assert!(matches!(asserted.root().value, Value::Temporal(_)));
    assert!(parse(r#"!app.A !app.B !Cron "@daily""#).is_err());
}

#[test]
fn native_cron_kind_literal_defaults_and_string_boundary() {
    let bundle = Bundle::from_sources(
        ".",
        [
            ("daml.bundle".to_owned(), "{}".to_owned()),
            (
                "Schedule.daml".to_owned(),
                r#"closed !Object {
  when: !Cron
  daily: !Cron "@daily"
  text: !String
} = {
  when: !Cron "@weekly"
  daily: !Cron "0 0 0 * * *"
  text: "ordinary"
}"#
                .to_owned(),
            ),
        ],
    )
    .unwrap();
    let definition = parse("!self.Schedule").unwrap();
    let schemas = bundle.compile(&definition, None).unwrap();
    let completed = schemas.resolve(&definition).unwrap();
    let when = completed.root().get("when").unwrap();
    assert!(matches!(when.origin(), Origin::Default(_)));
    let ResolvedValue::Temporal(value) = when.value() else {
        panic!("default lost Cron");
    };
    assert_eq!(value.kind(), TemporalKind::Cron);
    assert_eq!(value.as_str(), "0 0 0 * * 0");
    assert_eq!(completed.to_json()["when"], json!("0 0 0 * * 0"));
    let semantic = SemanticNode::from_resolved(completed.root());
    let readmitted = schemas.resolve_semantic(&semantic, None).unwrap();
    assert!(matches!(
        readmitted.get("when").unwrap().value(),
        ResolvedValue::Temporal(_)
    ));
    for source in [
        r#"!self.Schedule { when: "0 0 0 * * 0" }"#,
        r#"!self.Schedule { when: !Date "2024-02-29" }"#,
        r#"!self.Schedule { daily: !Cron "@hourly" }"#,
        r#"!self.Schedule { text: !Cron "@daily" }"#,
    ] {
        assert!(
            schemas.resolve(&parse(source).unwrap()).is_err(),
            "{source}"
        );
    }
    for offset in ["required", "forbidden", "optional"] {
        let bundle = Bundle::from_sources(
            ".",
            [
                ("daml.bundle".into(), "{}".into()),
                ("Bad.daml".into(), format!("!Cron {{ offset: {offset:?} }}")),
            ],
        )
        .unwrap();
        assert!(bundle.compile(&parse("!self.Bad").unwrap(), None).is_err());
    }
}

#[test]
fn json_schema_cron_defaults_and_constants_do_not_downgrade() {
    let module = SchemaModule::from_value(json!({
        "types": {"Root": {"schema": {
            "type": "object",
            "properties": {"when": {"x-daml-scalar": {"kind": "cron", "const": "@daily"}}},
            "default": {"when": "@daily"},
            "x-daml-default-temporals": [{"path": "/when", "kind": "cron"}]
        }}}
    }))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let document = parse("!Root").unwrap();
    let result = schemas.resolve(&document).unwrap();
    let ResolvedValue::Temporal(value) = result.root().get("when").unwrap().value() else {
        panic!("annotated default lost Cron");
    };
    assert_eq!(value.as_str(), "0 0 0 * * *");
    for source in [
        r#"!Root { when: "@daily" }"#,
        r#"!Root { when: !Cron "@weekly" }"#,
    ] {
        assert!(
            schemas.resolve(&parse(source).unwrap()).is_err(),
            "{source}"
        );
    }
    assert!(
        schemas
            .resolve(&parse(r#"!Root { when: !Cron "0 0 0 * * *" }"#).unwrap())
            .is_ok()
    );
    for expression in ["not a cron", "0 0 24 * * *"] {
        let invalid = SchemaModule::from_value(json!({
            "types": {"Root": {"schema": {
                "default": expression,
                "x-daml-default-temporals": [{"path": "", "kind": "cron"}]
            }}}
        }));
        assert!(match invalid {
            Err(_) => true,
            Ok(module) => Schemas::compose([module]).is_err(),
        });
    }
    // Intrinsic values in raw JSON-compatible module envelopes must not become strings.
    let envelope = r#"{ types: { Root: { schema: { default: !Cron "@daily" } } } }"#;
    assert!(SchemaModule::from_daml(envelope).is_err());
}

#[test]
fn staged_summaries_copy_and_replacement_keep_cron_identity() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({
        "types": {"Root": {"schema": {"type": "object"}}}
    }))
    .unwrap()])
    .unwrap();
    let source = parse(r#"!Root { source: !Cron "@weekly" }"#).unwrap();
    struct Edit {
        saw_summary: bool,
    }
    impl AdmissionStage for Edit {
        type Error = &'static str;
        fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> Result<bool, Self::Error> {
            if path == "/source" {
                assert_eq!(
                    node.value(),
                    SummaryValue::Temporal {
                        kind: TemporalKind::Cron,
                        value: "0 0 0 * * 0",
                    }
                );
                self.saw_summary = true;
            }
            Ok(false)
        }
        fn materialize(
            &mut self,
            root: &ResolvedNode<'_>,
            _copies: &[CopyEdge],
            round: usize,
        ) -> Result<StageOutcome<Self::Error>, Self::Error> {
            assert!(self.saw_summary);
            assert!(matches!(
                root.get("source").unwrap().value(),
                ResolvedValue::Temporal(_)
            ));
            Ok(if round == 0 {
                StageOutcome::Edits(vec![StageEdit::Copy {
                    path: "/target".into(),
                    from: "/source".into(),
                }])
            } else {
                StageOutcome::Ready { meaning: None }
            })
        }
    }
    let mut stage = Edit { saw_summary: false };
    let result = schemas
        .resolve_staged(&source, None, &[], &mut stage)
        .unwrap();
    let ResolvedValue::Temporal(value) = result.root().get("target").unwrap().value() else {
        panic!("copied Cron downgraded");
    };
    assert_eq!(value.as_str(), "0 0 0 * * 0");
    struct Replace;
    impl AdmissionStage for Replace {
        type Error = &'static str;
        fn defer(&mut self, _node: NodeSummary<'_>, _path: &str) -> Result<bool, Self::Error> {
            Ok(false)
        }
        fn materialize(
            &mut self,
            _root: &ResolvedNode<'_>,
            _copies: &[CopyEdge],
            round: usize,
        ) -> Result<StageOutcome<Self::Error>, Self::Error> {
            Ok(if round == 0 {
                StageOutcome::Edits(vec![StageEdit::Replace {
                    path: "/source".into(),
                    value: SemanticNode::from_document(&parse(r#"!Cron "0 0 0 * * 7""#).unwrap()),
                }])
            } else {
                StageOutcome::Ready { meaning: None }
            })
        }
    }
    let replaced = schemas
        .resolve_staged(&source, None, &[], &mut Replace)
        .unwrap();
    let ResolvedValue::Temporal(value) = replaced.root().get("source").unwrap().value() else {
        panic!("replacement Cron downgraded");
    };
    assert_eq!(value.as_str(), "0 0 0 * * 7");
}

#[test]
fn cron_parser_obeys_existing_scalar_and_node_limits() {
    let limits = daml::Limits {
        max_nodes: 1,
        ..daml::Limits::default()
    };
    assert!(daml::parse_with_limits(r#"!Cron "@hourly""#, limits).is_ok());
    assert!(daml::parse_with_limits(r#"[!Cron "@hourly"]"#, limits).is_err());
    let limits = daml::Limits {
        max_scalar_bytes: 3,
        ..daml::Limits::default()
    };
    assert!(daml::parse_with_limits(r#"!Cron "@hourly""#, limits).is_err());
}
