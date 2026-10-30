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
    for source in ["!self.B", "!self.B { choice: !Int | !String }"] {
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
