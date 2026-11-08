//! Public-consumer checks for temporal values and their integration paths.
use daml::{FormatOptions, TemporalKind, TemporalValue, Value, format, parse};

#[test]
fn intrinsic_values_are_distinct_from_strings_and_keep_authored_format() {
    let source = "{\n  date: !Date \"2024-02-29\"\n  time: !Time \"T09:30\"\n  dt: !DateTime \"2026-10-29T09\"\n  plain: \"2024-02-29\"\n}";
    let document = parse(source).unwrap();
    let date = document.get(document.root(), "date").unwrap();
    let time = document.get(document.root(), "time").unwrap();
    let datetime = document.get(document.root(), "dt").unwrap();
    let plain = document.get(document.root(), "plain").unwrap();

    assert!(matches!(date.value, Value::Temporal(_)));
    assert!(matches!(time.value, Value::Temporal(_)));
    assert!(matches!(datetime.value, Value::Temporal(_)));
    assert!(matches!(plain.value, Value::String(_)));
    assert_eq!(document.temporal(date).unwrap().kind(), TemporalKind::Date);
    assert_eq!(document.temporal(date).unwrap().as_str(), "2024-02-29");
    assert_eq!(document.temporal(time).unwrap().as_str(), "09:30:00");
    assert_eq!(
        document.temporal(datetime).unwrap().as_str(),
        "2026-10-29T09:00:00"
    );
    assert_eq!(document.string(plain), Some("2024-02-29"));
    let formatted = format(&document, FormatOptions { line_width: 80 }).unwrap();
    assert!(formatted.contains("!Time \"T09:30\""));
    assert!(formatted.contains("!DateTime \"2026-10-29T09\""));
}

#[test]
fn temporal_access_rejects_nodes_from_other_documents() {
    let source = parse("!Date \"2024-02-29\"").unwrap();
    let unrelated = parse("\"xxxxxxxxxxxxxxxxxxxx\"").unwrap();
    assert_eq!(
        source.temporal(source.root()).unwrap().as_str(),
        "2024-02-29"
    );
    assert!(unrelated.temporal(source.root()).is_none());

    let canonicalized = parse("!Time \"09:30\"").unwrap();
    assert_eq!(
        canonicalized
            .temporal(canonicalized.root())
            .unwrap()
            .as_str(),
        "09:30:00"
    );
    assert!(unrelated.temporal(canonicalized.root()).is_none());

    let decoded = parse(r#"!Time "09\u003A30""#).unwrap();
    assert_eq!(
        decoded.temporal(decoded.root()).unwrap().as_str(),
        "09:30:00"
    );
    assert!(unrelated.temporal(decoded.root()).is_none());

    let equal_source = parse("!Date \"2024-02-29\"").unwrap();
    assert_eq!(source, equal_source);
    assert_eq!(
        equal_source.temporal(equal_source.root()).unwrap().as_str(),
        "2024-02-29"
    );
    assert!(equal_source.temporal(source.root()).is_none());
    let nested = parse("{\n  value: !Time \"09:30:00.123456789123\"\n}").unwrap();
    let nested_value = nested.get(nested.root(), "value").unwrap();
    assert_eq!(
        nested.temporal(nested_value).unwrap().as_str(),
        "09:30:00.123456789123"
    );

    let document = parse("!DateTime \"2026-10-29T09:30Z\"").unwrap();
    let moved_document = document;
    assert_eq!(
        moved_document
            .temporal(moved_document.root())
            .unwrap()
            .as_str(),
        "2026-10-29T09:30:00Z"
    );
}

#[test]
fn profile_preserves_fraction_offset_presence_and_exceptional_labels() {
    let precision = TemporalValue::parse(TemporalKind::Time, "09:30:00,123456789123").unwrap();
    assert_eq!(precision.as_str(), "09:30:00.123456789123");
    let local = TemporalValue::parse(TemporalKind::DateTime, "2026-10-29T09Z").unwrap();
    let absent = TemporalValue::parse(TemporalKind::DateTime, "2026-10-29T09").unwrap();
    assert_eq!(local.as_str(), "2026-10-29T09:00:00Z");
    assert_eq!(local.offset_minutes(), Some(0));
    assert_eq!(absent.as_str(), "2026-10-29T09:00:00");
    assert_eq!(absent.offset_minutes(), None);
    assert_ne!(local, absent);

    let end = TemporalValue::parse(TemporalKind::DateTime, "2026-10-29T24:00:00.000").unwrap();
    assert_eq!(end.as_str(), "2026-10-29T24:00:00.000");
    let leap = TemporalValue::parse(TemporalKind::DateTime, "2016-12-31T23:59:60.5Z").unwrap();
    assert_eq!(leap.as_str(), "2016-12-31T23:59:60.5Z");
}

#[test]
fn invalid_temporal_literals_are_rejected_at_admission() {
    for (kind, value) in [
        (TemporalKind::Date, "2023-02-29"),
        (TemporalKind::Date, "2024-13-01"),
        (TemporalKind::Time, "12:60"),
        (TemporalKind::Time, "12:30:00.١"),
        (TemporalKind::Time, "24:00:01"),
        (TemporalKind::Time, "24:00:00.001"),
        (TemporalKind::DateTime, "2026-10-29t09:00"),
        (TemporalKind::DateTime, "2026-10-29T09:00-00:00"),
        (TemporalKind::DateTime, "2026-10-29T09:00+23:60"),
        (TemporalKind::DateTime, "2026-10-29T23:59:60+01:00"),
    ] {
        assert!(
            TemporalValue::parse(kind, value).is_err(),
            "accepted {value}"
        );
    }
}

#[test]
fn constructors_and_offsets_are_explicit() {
    assert!(TemporalValue::parse(TemporalKind::Date, "2026-10-29T09:00").is_err());
    assert!(TemporalValue::parse(TemporalKind::Time, "2026-10-29").is_err());
    assert_eq!(
        TemporalValue::parse(TemporalKind::Time, "T09")
            .unwrap()
            .as_str(),
        "09:00:00"
    );
    assert_eq!(
        TemporalValue::parse(TemporalKind::Time, "09:30:00+01")
            .unwrap()
            .as_str(),
        "09:30:00+01:00"
    );
    assert_eq!(
        TemporalValue::parse(TemporalKind::Time, "09:30:00-01:30")
            .unwrap()
            .offset_minutes(),
        Some(-90)
    );
}

#[test]
fn native_temporal_constraints_distinguish_intrinsic_kinds_and_offset_presence() {
    use daml::{Bundle, Origin, ResolveErrorKind, ResolvedValue};

    let bundle = Bundle::from_sources(
        ".",
        [
            ("daml.bundle".to_owned(), "{}".to_owned()),
            (
                "Schedule.daml".to_owned(),
                r#"closed !Object {
  day: !Date
  deadline: !DateTime { offset: "required" }
  local: !Time { offset: "forbidden" }
  text: !String
} = {
  day: !Date "2026-10-29"
  deadline: !DateTime "2026-10-29T09:30Z"
  local: !Time "09:30"
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
    for (field, expected, kind) in [
        ("day", "2026-10-29", TemporalKind::Date),
        ("deadline", "2026-10-29T09:30:00Z", TemporalKind::DateTime),
        ("local", "09:30:00", TemporalKind::Time),
    ] {
        let value = completed.root().get(field).unwrap();
        assert!(matches!(value.origin(), Origin::Default(_)));
        let ResolvedValue::Temporal(temporal) = value.value() else {
            panic!("{field} default lost intrinsic temporal identity");
        };
        assert_eq!(temporal.kind(), kind);
        assert_eq!(temporal.as_str(), expected);
    }
    let semantic = daml::SemanticNode::from_resolved(completed.root());
    let semantic_re_admitted = schemas.resolve_semantic(&semantic, None).unwrap();
    let semantic_deadline = semantic_re_admitted.get("deadline").unwrap();
    let daml::ResolvedValue::Temporal(temporal) = semantic_deadline.value() else {
        panic!("semantic re-admission downgraded the datetime default");
    };
    assert_eq!(temporal.as_str(), "2026-10-29T09:30:00Z");

    struct DefaultInspect {
        saw_default: bool,
    }
    impl daml::AdmissionStage for DefaultInspect {
        type Error = &'static str;
        fn defer(&mut self, node: daml::NodeSummary<'_>, path: &str) -> Result<bool, Self::Error> {
            if path == "/deadline" {
                assert_eq!(
                    node.value(),
                    daml::SummaryValue::Temporal {
                        kind: TemporalKind::DateTime,
                        value: "2026-10-29T09:30:00Z",
                    }
                );
                self.saw_default = true;
            }
            Ok(false)
        }
        fn materialize(
            &mut self,
            root: &daml::ResolvedNode<'_>,
            _copies: &[daml::CopyEdge],
            _round: usize,
        ) -> Result<daml::StageOutcome<Self::Error>, Self::Error> {
            assert!(self.saw_default);
            assert!(matches!(
                root.get("deadline").unwrap().value(),
                daml::ResolvedValue::Temporal(_)
            ));
            Ok(daml::StageOutcome::Ready { meaning: None })
        }
    }
    let mut default_inspect = DefaultInspect { saw_default: false };
    let default_stage = schemas
        .resolve_staged(&definition, None, &[], &mut default_inspect)
        .unwrap();
    assert!(default_inspect.saw_default);
    assert!(matches!(
        default_stage.root().get("deadline").unwrap().value(),
        daml::ResolvedValue::Temporal(_)
    ));
    let valid = parse(
        "!self.Schedule {\n  day: !Date \"2026-10-29\"\n  deadline: !DateTime \"2026-10-29T09:30+01\"\n  local: !Time \"09:30\"\n  text: \"ordinary\"\n}",
    )
    .unwrap();
    struct Inspect {
        saw_supplied: bool,
    }
    impl daml::AdmissionStage for Inspect {
        type Error = &'static str;
        fn defer(&mut self, node: daml::NodeSummary<'_>, path: &str) -> Result<bool, Self::Error> {
            if path == "/deadline" {
                match node.value() {
                    daml::SummaryValue::Temporal {
                        kind: TemporalKind::DateTime,
                        value: "2026-10-29T09:30:00+01:00",
                    } => self.saw_supplied = true,
                    daml::SummaryValue::Temporal {
                        kind: TemporalKind::DateTime,
                        value: "2026-10-29T09:30:00Z",
                    } => {}
                    summary => panic!("unexpected deadline summary: {summary:?}"),
                }
            }
            Ok(false)
        }
        fn materialize(
            &mut self,
            _root: &daml::ResolvedNode<'_>,
            _copies: &[daml::CopyEdge],
            _round: usize,
        ) -> Result<daml::StageOutcome<Self::Error>, Self::Error> {
            assert!(self.saw_supplied);
            Ok(daml::StageOutcome::Ready { meaning: None })
        }
    }
    let mut inspect = Inspect {
        saw_supplied: false,
    };
    let staged = schemas
        .resolve_staged(&valid, None, &[], &mut inspect)
        .unwrap();
    assert!(inspect.saw_supplied);
    assert_eq!(
        staged.root().get("deadline").unwrap().to_json(),
        serde_json::json!("2026-10-29T09:30:00+01:00")
    );
    assert!(schemas.resolve(&valid).is_ok());

    for invalid in [
        "!self.Schedule {\n  day: \"2026-10-29\"\n  deadline: !DateTime \"2026-10-29T09:30Z\"\n  local: !Time \"09:30\"\n}",
        "!self.Schedule {\n  day: !Date \"2026-10-29\"\n  deadline: !DateTime \"2026-10-29T09:30\"\n  local: !Time \"09:30\"\n}",
        "!self.Schedule {\n  day: !Date \"2026-10-29\"\n  deadline: !DateTime \"2026-10-29T09:30Z\"\n  local: !Time \"09:30Z\"\n}",
    ] {
        assert_eq!(
            schemas.resolve(&parse(invalid).unwrap()).unwrap_err().kind,
            ResolveErrorKind::SchemaViolation
        );
    }
}

#[test]
fn semantic_write_and_read_retain_temporal_kind_and_fraction_digits() {
    use daml::{FormatOptions, SemanticNode, SemanticValue, format_semantic};

    let parsed = parse("!Time \"09:30:00,123456789123\"").unwrap();
    let semantic = SemanticNode::from_document(&parsed);
    assert!(matches!(semantic.value, SemanticValue::Temporal(_)));
    let written = format_semantic(&semantic, FormatOptions { line_width: 80 }).unwrap();
    assert_eq!(written, "!Time \"09:30:00.123456789123\"\n");
    let re_admitted = SemanticNode::from_document(&parse(&written).unwrap());
    assert_eq!(re_admitted, semantic);
}

#[test]
fn outer_assertion_applies_without_permitting_generic_tag_stacking() {
    let asserted = parse("!app.Deadline !DateTime \"2026-10-29T09:30Z\"").unwrap();
    assert_eq!(asserted.tag(asserted.root()), Some("app.Deadline"));
    assert!(matches!(asserted.root().value, Value::Temporal(_)));
    assert_eq!(
        asserted.temporal(asserted.root()).unwrap().as_str(),
        "2026-10-29T09:30:00Z"
    );
    assert!(parse("!app.First !app.Second !DateTime \"2026-10-29T09:30Z\"").is_err());
}

#[test]
fn temporal_values_count_as_one_node_at_parser_budget_boundary() {
    use daml::{Limits, parse_with_limits};

    let limits = Limits {
        max_nodes: 1,
        ..Limits::default()
    };
    assert!(parse_with_limits("!Date \"2024-02-29\"", limits).is_ok());
    assert!(parse_with_limits("[!Date \"2024-02-29\"]", limits).is_err());
}

#[test]
fn native_string_rejects_temporal_values_and_invalid_offset_facets_fail() {
    use daml::{Bundle, ResolveErrorKind};

    let bundle = Bundle::from_sources(
        ".",
        [
            ("daml.bundle".to_owned(), "{}".to_owned()),
            (
                "Schedule.daml".to_owned(),
                "closed !Object {\n  day: !Date\n  text: !String\n}".to_owned(),
            ),
        ],
    )
    .unwrap();
    let definition = parse("!self.Schedule").unwrap();
    let schemas = bundle.compile(&definition, None).unwrap();
    let temporal_string =
        parse("!self.Schedule {\n  day: !Date \"2024-02-29\"\n  text: !Date \"2024-02-29\"\n}")
            .unwrap();
    assert_eq!(
        schemas.resolve(&temporal_string).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );

    for (file, native) in [
        ("BadDate.daml", r#"!Date { offset: "required" }"#),
        ("BadTime.daml", r#"!Time { offset: "unknown" }"#),
        (
            "BadFacet.daml",
            r#"!DateTime { min: "2024-01-01T00:00:00" }"#,
        ),
    ] {
        let bundle = Bundle::from_sources(
            ".",
            [
                ("daml.bundle".to_owned(), "{}".to_owned()),
                (file.to_owned(), native.to_owned()),
            ],
        )
        .unwrap();
        let definition = parse(format!("!self.{}", file.trim_end_matches(".daml"))).unwrap();
        assert!(bundle.compile(&definition, None).is_err(), "{file}");
    }
}

#[test]
fn malformed_temporal_default_annotations_fail_and_data_lookalikes_stay_inert() {
    use daml::{SchemaModule, Schemas};
    use serde_json::{Value as JsonValue, json};

    let invalid = [
        (json!({}), "/value", "date"),
        (json!({"value": 42}), "/value", "date"),
        (json!({"value": "2024-02-29"}), "/value", "unknown"),
        (json!({"value": "2023-02-29"}), "/value", "date"),
    ];
    for (default, path, kind) in invalid {
        let module = SchemaModule::from_value(json!({
            "types": {"app.Config": {"schema": {
                "type": "object",
                "default": default,
                "x-daml-default-temporals": [{"path": path, "kind": kind}]
            }}}
        }));
        assert!(
            match module {
                Err(_) => true,
                Ok(module) => Schemas::compose([module]).is_err(),
            },
            "accepted malformed temporal default annotation for {path} ({kind})"
        );
    }

    let module = SchemaModule::from_value(json!({
        "types": {"app.Config": {"schema": {"type": "object"}}}
    }))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let data = parse(
        "{\n  x-daml-default-temporals: [\n    {\n      path: \"/value\"\n      kind: \"date\"\n    }\n  ]\n  value: \"2024-02-29\"\n}",
    )
    .unwrap();
    let actual: JsonValue = schemas.resolve(&data).unwrap().to_json();
    assert_eq!(actual["x-daml-default-temporals"][0]["kind"], json!("date"));
    assert_eq!(actual["value"], json!("2024-02-29"));
}

#[test]
fn temporal_copy_and_exact_stage_replacement_preserve_representation() {
    use daml::{
        AdmissionStage, CopyEdge, ResolvedNode, SchemaModule, Schemas, SemanticNode, StageEdit,
        StageOutcome,
    };
    use serde_json::json;

    let module = SchemaModule::from_value(json!({
        "types": {"Root": {"schema": {"type": "object"}}}
    }))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let source = parse("!Root {\n  source: !Time \"09:30:00.123456789123\"\n}").unwrap();
    struct Copy;
    impl AdmissionStage for Copy {
        type Error = &'static str;
        fn defer(
            &mut self,
            _node: daml::NodeSummary<'_>,
            _path: &str,
        ) -> Result<bool, Self::Error> {
            Ok(false)
        }
        fn materialize(
            &mut self,
            _root: &ResolvedNode<'_>,
            _copies: &[CopyEdge],
            round: usize,
        ) -> Result<StageOutcome<Self::Error>, Self::Error> {
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
    let copied = schemas
        .resolve_staged(&source, None, &[], &mut Copy)
        .unwrap();
    let target = copied.root().get("target").unwrap();
    let daml::ResolvedValue::Temporal(copied_time) = target.value() else {
        panic!("stage copy downgraded the temporal kind");
    };
    assert_eq!(copied_time.as_str(), "09:30:00.123456789123");

    let precision_source = parse("!Root {\n  source: !Time \"09:30:00.5\"\n}").unwrap();
    let precise = TemporalValue::parse(TemporalKind::Time, "09:30:00.5").unwrap();
    let same_numeric_value = TemporalValue::parse(TemporalKind::Time, "09:30:00.500").unwrap();
    assert_eq!(precise, same_numeric_value);

    struct ExactReplacement;
    impl AdmissionStage for ExactReplacement {
        type Error = &'static str;
        fn defer(
            &mut self,
            _node: daml::NodeSummary<'_>,
            _path: &str,
        ) -> Result<bool, Self::Error> {
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
                    value: SemanticNode::from_document(&parse("!Time \"09:30:00.500\"").unwrap()),
                }])
            } else {
                StageOutcome::Ready { meaning: None }
            })
        }
    }
    let exact = schemas
        .resolve_staged(&precision_source, None, &[], &mut ExactReplacement)
        .unwrap();
    let value = exact.root().get("source").unwrap();
    let daml::ResolvedValue::Temporal(exact_time) = value.value() else {
        panic!("stage replacement downgraded the temporal kind");
    };
    assert_eq!(exact_time.as_str(), "09:30:00.500");
}
