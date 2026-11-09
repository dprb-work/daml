//! Selected structural alternatives, cumulative work, and scaling diagnostics.
use daml::{ResolutionLimits, ResolveErrorKind, SchemaModule, Schemas, SemanticNode, parse};
use serde_json::{Value, json};

fn choices(alternatives: usize, overlapping: bool) -> Schemas {
    let mut resources = vec![];
    let mut arms = vec![];
    for index in 0..alternatives {
        let uri = format!("urn:work:arm:{index}");
        arms.push(json!({"$ref": uri}));
        resources.push(json!({"uri": uri, "schema": {
            "type": "object",
            "properties": {
                "kind": {"enum": [if overlapping {"option-0".to_owned()} else {format!("option-{index}")}],
                         "default": if overlapping {"option-0".to_owned()} else {format!("option-{index}")}},
                "children": {"type": "array", "items": {"$ref": "urn:work:choice"}, "default": []}
            },
            "required": ["kind", "children"], "additionalProperties": false
        }}));
    }
    resources.push(json!({"uri": "urn:work:choice", "schema": {"oneOf": arms}}));
    let module = SchemaModule::from_value(json!({
        "types": {
            "app.Root": {"schema": {"type": "object", "properties": {"child": {"$ref": "urn:work:choice"}}, "required": ["child"]}},
            // The binding name intentionally has no relationship to the enum.
            "unrelated.Selected": {"schema": {"$ref": "urn:work:arm:0"}},
            "app.Alias": {"schema": {"$ref": "urn:work:arm:0"}}
        }, "resources": resources
    })).unwrap();
    Schemas::compose([module]).unwrap()
}

fn input(depth: usize, tag: Option<&str>, supplied_kind: bool) -> String {
    let mut node = String::new();
    for _ in 0..depth {
        node = format!(
            "{}{{\n{}children: [{}]\n}}",
            tag.map_or(String::new(), |tag| format!("!{tag} ")),
            if supplied_kind {
                "kind: \"option-0\"\n"
            } else {
                ""
            },
            node
        );
    }
    format!("!app.Root {{child: {node}}}")
}

#[test]
fn selected_nested_defaults_do_not_enumerate_the_catalog_cartesian_product() {
    let schemas = choices(8, false);
    let tagged = parse(input(4, Some("unrelated.Selected"), false)).unwrap();
    let root_only = parse(input(4, None, true)).unwrap();
    let expected = schemas.resolve(&root_only).unwrap().to_json();
    let resolved = schemas.resolve(&tagged).unwrap();
    assert_eq!(resolved.to_json(), expected);
    let semantic = SemanticNode::from_document(&tagged);
    assert_eq!(
        schemas
            .resolve_semantic(&semantic, Some("app.Root"))
            .unwrap()
            .to_json(),
        expected
    );
    let alias = parse(input(4, Some("app.Alias"), false)).unwrap();
    assert_eq!(schemas.resolve(&alias).unwrap().to_json(), expected);
}

struct ReplaceChild;
impl daml::AdmissionStage for ReplaceChild {
    type Error = &'static str;
    fn defer(&mut self, _: daml::NodeSummary<'_>, _: &str) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn materialize(
        &mut self,
        _: &daml::ResolvedNode<'_>,
        _: &[daml::CopyEdge],
        round: usize,
    ) -> Result<daml::StageOutcome<Self::Error>, Self::Error> {
        Ok(if round == 0 {
            daml::StageOutcome::Edits(vec![daml::StageEdit::Replace {
                path: "/child".into(),
                value: SemanticNode::from_document(
                    &parse("{kind: \"option-1\"\nchildren: []}").unwrap(),
                ),
            }])
        } else {
            daml::StageOutcome::Ready { meaning: None }
        })
    }
}

#[test]
fn staged_selection_uses_retained_assertions_not_mutable_values() {
    let schemas = choices(4, false);
    let untagged = parse(input(1, None, true)).unwrap();
    let resolved = schemas
        .resolve_staged(&untagged, None, &[], &mut ReplaceChild)
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({"child": {"kind": "option-1", "children": []}})
    );
    let tagged = parse(input(1, Some("unrelated.Selected"), false)).unwrap();
    // Replacing a value does not waive its original supplied assertion.
    let error = schemas
        .resolve_staged(&tagged, None, &[], &mut ReplaceChild)
        .unwrap_err();
    assert!(
        matches!(error, daml::StageError::Resolve(error) if error.kind == ResolveErrorKind::SchemaViolation)
    );
}

#[test]
fn structural_selection_preserves_ambiguity_and_supplied_values() {
    let overlapping = choices(2, true);
    let document = parse(input(1, Some("unrelated.Selected"), false)).unwrap();
    assert_eq!(
        overlapping.resolve(&document).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
    let schemas = choices(4, false);
    for kind in ["null", "\"option-1\"", "42"] {
        let document = parse(format!(
            "!app.Root {{child: !unrelated.Selected {{\nkind: {kind}\nchildren: []\n}}}}"
        ))
        .unwrap();
        assert_eq!(
            schemas.resolve(&document).unwrap_err().kind,
            ResolveErrorKind::SchemaViolation
        );
    }
    let optional = SchemaModule::from_value(json!({"types": {
        "app.Optional": {"schema": {"properties": {"kind": {"enum": ["a"]}}}},
        "app.Consumer": {"schema": {"oneOf": [
            {"properties": {"kind": {"enum": ["b"]}}},
            {"properties": {"kind": {"enum": ["c"]}}}
        ]}}
    }}))
    .unwrap();
    let optional = Schemas::compose([optional]).unwrap();
    let document = parse("!app.Optional {}").unwrap();
    // Omission satisfies every optional property constraint; it cannot prune arms.
    assert_eq!(
        optional
            .resolve_as(&document, "app.Consumer")
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn numeric_domains_preserve_json_equality_and_one_of_overlap() {
    let module = SchemaModule::from_value(json!({"types": {
        "app.Number": {"schema": {"const": 1}},
        "app.Consumer": {"schema": {"oneOf": [
            {"const": 1.0}, {"enum": [1]}
        ]}}
    }}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let document = parse("!app.Number 1").unwrap();
    assert_eq!(
        schemas
            .resolve_as(&document, "app.Consumer")
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn individually_admitted_modules_still_exceed_the_aggregate_node_bound() {
    let modules = (0..2)
        .map(|index| {
            SchemaModule::from_value(json!({"resources": [{
                "uri": format!("urn:work:large:{index}"),
                "schema": {"const": vec![0; 6000]}
            }]}))
            .unwrap()
        })
        .collect::<Vec<_>>();
    let error = Schemas::compose(modules)
        .err()
        .expect("aggregate schema admission must reject");
    assert_eq!(error.kind, daml::SchemaErrorKind::LimitExceeded);
}

#[test]
fn genuinely_repeated_recursive_obligations_remain_bounded() {
    let module = SchemaModule::from_value(json!({"types": {"app.Repeated": {"schema": {
        "type": "object", "allOf": [
            {"properties": {"child": {"$ref": "#"}}},
            {"properties": {"child": {"$ref": "#"}}}
        ]
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let mut source = "{}".to_owned();
    for _ in 0..30 {
        source = format!("{{child: {source}}}");
    }
    let document = parse(source).unwrap();
    assert_eq!(
        schemas
            .resolve_as(&document, "app.Repeated")
            .unwrap_err()
            .kind,
        ResolveErrorKind::LimitExceeded
    );
}

#[test]
fn unused_definitions_do_not_multiply_instance_validation_work() {
    let definitions: serde_json::Map<String, Value> = (0..1024)
        .map(|index| {
            (
                format!("Unused{index}"),
                json!({"type":"null", "description":"not applied"}),
            )
        })
        .collect();
    let module = SchemaModule::from_value(json!({"types": {"app.Config": {"schema": {
        "$defs": definitions, "type": "array", "items": {"type": "integer"}
    }}}}))
    .unwrap();
    let schemas = Schemas::compose([module]).unwrap();
    let source = format!(
        "[{}]",
        (0..1024)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let document = parse(source).unwrap();
    assert_eq!(
        schemas
            .resolve_as(&document, "app.Config")
            .unwrap()
            .to_json(),
        json!((0..1024).collect::<Vec<_>>())
    );
}

fn minimum_steps(schemas: &Schemas, document: &daml::Document) -> Option<usize> {
    let (mut low, mut high) = (0, ResolutionLimits::default().max_steps);
    let run = |max_steps| {
        schemas.resolve_with_limits(
            document,
            None,
            ResolutionLimits {
                max_steps,
                ..Default::default()
            },
        )
    };
    match run(high) {
        Ok(_) => (),
        Err(error) if error.kind == ResolveErrorKind::LimitExceeded => return None,
        Err(error) => panic!("not a work-limit reproduction: {error:?}"),
    }
    while low < high {
        let middle = low + (high - low) / 2;
        match run(middle) {
            Ok(_) => high = middle,
            Err(error) if error.kind == ResolveErrorKind::LimitExceeded => low = middle + 1,
            Err(error) => panic!("not a work-limit reproduction: {error:?}"),
        }
    }
    Some(low)
}

#[test]
#[ignore = "diagnostic: minimum cumulative work budget, not an exact-count contract"]
fn report_validation_work_scaling_for_depth_alternatives_and_tags() {
    for (depth, alternatives) in [
        (1, 4),
        (2, 4),
        (3, 4),
        (4, 4),
        (3, 1),
        (3, 2),
        (3, 8),
        (3, 16),
    ] {
        let schemas = choices(alternatives, false);
        for (label, tag, supplied_kind) in [
            ("root-only", None, true),
            ("tagged", Some("unrelated.Selected"), false),
            ("tagged-supplied", Some("unrelated.Selected"), true),
        ] {
            let document = parse(input(depth, tag, supplied_kind)).unwrap();
            eprintln!(
                "depth={depth} alternatives={alternatives} {label}: min_steps={:?}",
                minimum_steps(&schemas, &document)
            );
        }
    }
}
