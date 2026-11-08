//! The native definition frontend must reach the same structural recursion contract.
use daml::{Bundle, NativeErrorKind, ResolveErrorKind, SemanticNode, parse};
use serde_json::json;

fn bundle(entries: &[(&str, &str)]) -> Bundle {
    Bundle::from_sources(
        ".",
        entries
            .iter()
            .map(|(path, text)| (path.to_string(), text.to_string())),
    )
    .unwrap()
}

#[test]
fn native_recursive_list_reference_completes_finite_values() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "closed !Object {\nvalue: !Int\nchildren: !List { items: !self.Node } = []\n}",
        ),
    ]);
    let supplied = parse("!self.Node {\nvalue: 1\nchildren: [{value: 2}]\n}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    let resolved = schemas.resolve(&supplied).unwrap();
    assert_eq!(
        resolved.to_json(),
        json!({"value": 1, "children": [{"value": 2, "children": []}]})
    );
    schemas
        .validate_semantic(&SemanticNode::from_resolved(resolved.root()), None)
        .unwrap();
    let invalid = parse("!self.Node {\nvalue: 1\nchildren: [{value: \"bad\"}]\n}").unwrap();
    assert_eq!(
        schemas.resolve(&invalid).unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn mutually_recursive_optional_members_preserve_their_own_constraints() {
    let bundle = bundle(&[
        ("daml.bundle", "{ other: \"./other\" }"),
        (
            "A.daml",
            "closed !Object {\nvalue: !Int\nnext?: !other.B\n}",
        ),
        ("other/daml.bundle", "{ parent: \"..\" }"),
        (
            "other/B.daml",
            "closed !Object {\nlabel: !String\nnext?: !parent.A\n}",
        ),
    ]);
    let supplied =
        parse("!self.A {\nvalue: 1\nnext: {\nlabel: \"child\"\nnext: {value: 2}\n}\n}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"value":1,"next":{"label":"child","next":{"value":2}}})
    );
    let invalid = parse("!self.A {\nvalue: 1\nnext: {label: 2}\n}").unwrap();
    assert_eq!(schemas.resolve(&invalid).unwrap_err().path, "/next/label");
}

#[test]
fn native_non_progressing_alias_loop_stays_invalid() {
    for source in [
        "!self.B",
        "!self.B { choice: !Int | !String }",
        "!self.B | !Int",
        "!self.B & !Object {}",
    ] {
        let bundle = bundle(&[
            ("daml.bundle", "{}"),
            ("A.daml", source),
            ("B.daml", "!self.A"),
        ]);
        let supplied = parse("!self.A {}").unwrap();
        assert_eq!(
            bundle.compile(&supplied, None).err().unwrap().kind,
            NativeErrorKind::ReferenceCycle
        );
    }
}

#[test]
fn recursive_member_default_overrides_preserve_supplied_values_and_constraints() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "closed !Object {\nsettings: !Object { value: !Int } = { value: 1 }\nnext?: !self.Node {\nsettings: !Object {} = { value: 2 }\n}\n}",
        ),
    ]);
    let supplied = parse("!self.Node {\nnext: {\nnext: {settings: {value: 9}}\n}\n}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"settings": {"value": 1}, "next": {"settings": {"value": 2}, "next": {"settings": {"value": 9}}}})
    );
    let invalid = parse("!self.Node {next: {settings: {value: \"bad\"}}}").unwrap();
    let error = schemas.resolve(&invalid).unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "/next/settings/value");
}

#[test]
fn recursive_conditional_presets_keep_ambiguity_and_supplied_value_precedence() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "closed !Object {\nsettings: !Object { value: !Int } = { value: 1 }\nnext?: !self.Node {\nsettings: (!Object { value: !Int } = { value: 2 }) | !Object {}\n}\n}",
        ),
    ]);
    let explicit = parse("!self.Node {next: {settings: {value: 9}}}").unwrap();
    let schemas = bundle.compile(&explicit, None).unwrap();
    assert_eq!(
        schemas.resolve(&explicit).unwrap().to_json(),
        json!({"settings": {"value": 1}, "next": {"settings": {"value": 9}}})
    );
    let partial = parse("!self.Node {next: {}}").unwrap();
    assert_eq!(
        schemas.resolve(&partial).unwrap_err().kind,
        ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn recursive_alternative_without_defaults_has_a_finite_compile_state() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        ("Node.daml", "!Object { next?: !self.Node } | !Int"),
    ]);
    let supplied = parse("!self.Node {next: 1}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"next": 1})
    );
}

#[test]
fn unset_container_proof_does_not_hide_a_non_progressing_alias_cycle() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        ("Root.daml", "!Object { child: !self.A = unset }"),
        ("A.daml", "!self.B"),
        ("B.daml", "!self.A"),
    ]);
    let supplied = parse("!self.Root {}").unwrap();
    assert_eq!(
        bundle.compile(&supplied, None).err().unwrap().kind,
        NativeErrorKind::ReferenceCycle
    );
}

#[test]
fn recursive_alternative_keeps_applicable_defaults_without_materializing_scalar_leaves() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "closed !Object {\nsettings: !Object { value: !Int } = { value: 1 }\nnext?: !self.Node\n} | !Int",
        ),
    ]);
    let supplied = parse("!self.Node {next: 7}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"settings": {"value": 1}, "next": 7})
    );
    assert_eq!(
        schemas
            .resolve(&parse("!self.Node 9").unwrap())
            .unwrap()
            .to_json(),
        json!(9)
    );
}

#[test]
fn mixed_recursive_conjunction_and_alternative_states_are_finite() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "A.daml",
            "!Object { next?: !self.A } | !Object { next?: !self.B }",
        ),
        (
            "B.daml",
            "!Object { next?: !self.A } & !Object { next?: !self.B }",
        ),
    ]);
    let supplied = parse("!self.A {next: {}}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"next": {}})
    );
}

#[test]
fn projecting_a_recursive_member_does_not_require_its_active_source_to_finish() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "!Object {\nnext?: !self.Node {\nnext: !Object {} = {}\n}\n}",
        ),
    ]);
    let supplied = parse("!self.Node {}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(schemas.resolve(&supplied).unwrap().to_json(), json!({}));
}

#[test]
fn recursive_conditional_projection_preserves_optional_finite_leaves() {
    for (member, source, expected) in [
        ("next", "!self.Node {}", json!({})),
        ("next?", "!self.Node {}", json!({})),
        ("next?", "!self.Node {next: {}}", json!({"next": {}})),
    ] {
        let definition = format!(
            "!Object {{\nnext?: !self.Node {{\n{member}: !Object {{ next: !Object {{}} = {{}} }} | !Object {{}}\n}}\n}}"
        );
        let bundle = bundle(&[("daml.bundle", "{}"), ("Node.daml", &definition)]);
        let supplied = parse(source).unwrap();
        let schemas = bundle.compile(&supplied, None).unwrap();
        let resolved = schemas.resolve(&supplied).unwrap();
        assert_eq!(resolved.to_json(), expected);
    }
}

#[test]
fn recursive_conditional_projection_preserves_selected_private_defaults() {
    let node = "!Object {\nsettings: !Object {\nkind: \"preset\"\npayload: !Object {} = !provider.Adapter {}\n} | closed !Object {\nkind: \"manual\"\npayload?: !Object {}\n}\nnext?: !self.Node {\nnext?: !Object { next: !Object {} = {} } | !Object {}\n}\n}";
    let bundle = bundle(&[
        ("daml.bundle", "{app: \"./app\"\nprovider: \"./decoy\"}"),
        ("app/daml.bundle", "{provider: \"./provider\"}"),
        ("app/provider/daml.bundle", "{}"),
        ("app/provider/Adapter.daml", "!Object {}"),
        ("decoy/daml.bundle", "{}"),
        ("decoy/Adapter.daml", "!Int"),
        ("app/Node.daml", node),
    ]);
    let supplied = parse(
        "{node: !app.Node {\nsettings: {kind: \"preset\"}\nnext: {\nsettings: {kind: \"preset\"\npayload: {marker: 7}}\n}\n}\ndecoy: !provider.Adapter 7}",
    )
    .unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    let resolved = schemas.resolve(&supplied).unwrap();
    assert_eq!(
        resolved.to_json(),
        json!({
            "node": {
                "settings": {"kind": "preset", "payload": {}},
                "next": {"settings": {"kind": "preset", "payload": {"marker": 7}}}
            },
            "decoy": 7
        })
    );
    let node = resolved.root().get("node").unwrap();
    let generated = node.get("settings").unwrap().get("payload").unwrap();
    let supplied = node
        .get("next")
        .unwrap()
        .get("settings")
        .unwrap()
        .get("payload")
        .unwrap();
    let decoy = resolved.root().get("decoy").unwrap();
    assert_eq!(generated.tag(), Some("provider.Adapter"));
    assert_eq!(supplied.tag(), Some("provider.Adapter"));
    assert!(generated.tag_binding().is_some());
    assert_eq!(generated.tag_binding(), supplied.tag_binding());
    assert_ne!(generated.tag_binding(), decoy.tag_binding());
    assert!(matches!(generated.origin(), daml::Origin::Default(_)));
    assert!(matches!(
        generated.tag_origin(),
        Some(daml::Origin::Default(_))
    ));
    assert!(matches!(supplied.origin(), daml::Origin::Input(_)));
    assert!(matches!(
        supplied.tag_origin(),
        Some(daml::Origin::Default(_))
    ));
    assert!(matches!(
        supplied.get("marker").unwrap().origin(),
        daml::Origin::Input(_)
    ));
}

#[test]
fn recursive_child_defaults_do_not_inherit_the_parent_branch_discriminator() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "!Object {\nnext?: !self.Node {\nnext?: !Object {\nkind: \"preset\"\nnext: !Object {} = {kind: \"manual\"}\n} | !Object {kind: \"manual\"}\n}\n}",
        ),
    ]);
    let root = parse("!self.Node {}").unwrap();
    let schemas = bundle.compile(&root, None).unwrap();
    for (source, expected) in [
        ("!self.Node {}", json!({})),
        ("!self.Node {next: {}}", json!({"next": {}})),
    ] {
        let supplied = parse(source).unwrap();
        assert_eq!(schemas.resolve(&supplied).unwrap().to_json(), expected);
    }
    let complete =
        parse("!self.Node {next: {next: {kind: \"preset\"\nnext: {kind: \"manual\"}}}}").unwrap();
    schemas
        .validate_semantic(&daml::SemanticNode::from_document(&complete), None)
        .unwrap();
    let invalid = parse("!self.Node {next: {next: {kind: \"invalid\"}}}").unwrap();
    assert_eq!(
        schemas
            .validate_semantic(&daml::SemanticNode::from_document(&invalid), None)
            .unwrap_err()
            .kind,
        daml::ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn repeated_reference_alternatives_are_not_same_instance_cycles() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        ("A.daml", "!Object {next?: !self.B | !self.B}"),
        ("B.daml", "!self.A"),
    ]);
    let root = parse("!self.A {}").unwrap();
    let schemas = bundle.compile(&root, None).unwrap();
    for (source, expected) in [
        ("!self.A {}", json!({})),
        ("!self.A {next: {}}", json!({"next": {}})),
    ] {
        assert_eq!(
            schemas.resolve(&parse(source).unwrap()).unwrap().to_json(),
            expected
        );
    }
}

#[test]
fn recursive_noop_projections_do_not_erase_effectful_default_overrides() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Node.daml",
            "!Object {\nsettings: !Object {value: !Int} = {value: 1}\nnext?: !self.Node {\nsettings: (!Object {value: !Int} = {value: 2}) | !Object {}\nnext?: !Object {next: !Object {} = {}} | !Object {}\n}\n}",
        ),
    ]);
    let root = parse("!self.Node {}").unwrap();
    let schemas = bundle.compile(&root, None).unwrap();
    assert_eq!(
        schemas.resolve(&root).unwrap().to_json(),
        json!({"settings": {"value": 1}})
    );
    let supplied = parse("!self.Node {next: {settings: {value: 9}}}").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"settings": {"value": 1}, "next": {"settings": {"value": 9}}})
    );
    let partial = parse("!self.Node {next: {}}").unwrap();
    assert_eq!(
        schemas.resolve(&partial).unwrap_err().kind,
        ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn child_defaults_cannot_borrow_another_parent_union_arms_constraints() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Base.daml",
            "!Object { settings: !Object { value: !Int { min: 5 } } }",
        ),
        (
            "Root.daml",
            "!self.Base { settings: !Object { value: !Int } = { value: 1 } } | !Object { settings: !Object { value: !Int } }",
        ),
    ]);
    let root = parse("!self.Root {}").unwrap();
    assert_eq!(
        bundle.compile(&root, None).err().unwrap().kind,
        NativeErrorKind::InvalidDefault
    );
}

#[test]
fn distinct_source_unions_retain_independent_default_choices() {
    let union = "(!Object {} = { x: 1 }) | (!Object {} = { y: 2 })";
    for other in ["U", "Shared", "V", "NoOp"] {
        let definition =
            format!("(!Tuple [!self.U & !self.{other}] & [{{ x: 1\ny: 2 }}]) = [{{}}]");
        let bundle = bundle(&[
            ("daml.bundle", "{}"),
            ("U.daml", union),
            ("V.daml", union),
            ("Shared.daml", "!self.U & !Any"),
            (
                "NoOp.daml",
                "!self.U { spare?: !Object { inner: !Object {} = unset } }",
            ),
            ("Root.daml", &definition),
        ]);
        let supplied = parse("!self.Root [{}]").unwrap();
        let compiled = bundle.compile(&supplied, None);
        if other == "U" || other == "Shared" {
            assert_eq!(
                compiled.err().unwrap().kind,
                NativeErrorKind::InvalidDefault
            );
        } else {
            assert_eq!(
                compiled.unwrap().resolve(&supplied).unwrap().to_json(),
                json!([{"x": 1, "y": 2}])
            );
        }
    }
}

#[test]
fn partial_default_validation_checks_completed_container_upper_bounds() {
    let bundle = bundle(&[
        ("daml.bundle", "{}"),
        (
            "Root.daml",
            "(!Object { child: !Object {} = {} } & !Map { max: 0 }) = {}",
        ),
    ]);
    let supplied = parse("!self.Root {}").unwrap();
    assert_eq!(
        bundle.compile(&supplied, None).err().unwrap().kind,
        NativeErrorKind::InvalidDefault
    );
}
