//! Explicit extra roots share the native loader and never widen alias visibility.

use daml::{Bundle, NativeErrorKind, ResolveErrorKind, parse};
use serde_json::json;

fn target_bundle() -> Bundle {
    let sources = [
        ("daml.bundle", "{dep: \"./dep\"}"),
        ("Config.daml", "!Object { name: !String }"),
        (
            "Target.daml",
            "closed !Object { count: !Int { min: 1 } } = { count: 4 }",
        ),
        ("dep/daml.bundle", "{private: \"./private\"}"),
        ("dep/Host.daml", "!Object { value: !private.Value }"),
        ("dep/private/daml.bundle", "{}"),
        ("dep/private/Value.daml", "!String"),
        ("Unused.daml", "!missing.Invalid"),
    ];
    Bundle::from_sources(
        ".",
        sources
            .into_iter()
            .map(|(name, source)| (name.to_owned(), source.to_owned())),
    )
    .unwrap()
}

#[test]
fn explicitly_loaded_target_types_supply_native_defaults_without_sticky_root_policy() {
    let bundle = target_bundle();
    let document = parse("!self.Config { name: \"worker\" }").unwrap();
    let ordinary = bundle.compile(&document, None).unwrap();
    let empty = parse("{}").unwrap();
    assert_eq!(
        ordinary.resolve_as(&empty, "self.Target").unwrap_err().kind,
        ResolveErrorKind::UnknownExpectedType
    );
    let schemas = bundle
        .compile_with_types(&document, None, ["self.Target"])
        .unwrap();
    assert_eq!(
        schemas.resolve(&document).unwrap().to_json(),
        json!({"name": "worker"})
    );
    assert_eq!(schemas.resolve(&empty).unwrap().to_json(), json!({}));
    assert_eq!(
        schemas.resolve_as(&empty, "self.Target").unwrap().to_json(),
        json!({"count": 4})
    );
    let invalid = parse("{ count: 0 }").unwrap();
    assert_eq!(
        schemas
            .resolve_as(&invalid, "self.Target")
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
    let unknown = parse("{ extra: true }").unwrap();
    assert_eq!(
        schemas
            .resolve_as(&unknown, "self.Target")
            .unwrap_err()
            .kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn additional_native_roots_are_entry_visible_names_with_honest_caller_diagnostics() {
    let bundle = target_bundle();
    let document = parse("!self.Config { name: \"worker\" }").unwrap();
    for (name, kind) in [
        ("self.Absent", NativeErrorKind::MissingDefinition),
        ("private.Value", NativeErrorKind::UnknownReference),
    ] {
        let error = bundle
            .compile_with_types(&document, None, [name])
            .err()
            .expect("unknown explicit root");
        assert_eq!(error.kind, kind);
        assert_eq!(error.reference.as_deref(), Some(name));
        assert_eq!(
            error.span, None,
            "caller-supplied names have no source byte span"
        );
    }
    let error = bundle
        .compile_with_types(&document, None, ["self.Unused"])
        .err()
        .expect("reachable invalid extra root");
    assert_eq!(error.kind, NativeErrorKind::UnknownReference);
    assert_eq!(error.reference.as_deref(), Some("missing.Invalid"));
    assert!(
        error.span.is_some(),
        "definition references keep their actual source span"
    );
}

#[test]
fn oversized_additional_root_is_bounded_before_lookup_or_composition() {
    let bundle = target_bundle();
    let document = parse("{}").unwrap();
    let oversized = "X".repeat(1_048_577);
    let error = bundle
        .compile_with_types(&document, None, [oversized.as_str()])
        .err()
        .expect("bounded caller root");
    assert_eq!(error.kind, NativeErrorKind::LimitExceeded);
    assert_eq!(error.span, None);
}

#[test]
fn duplicate_additional_roots_still_consume_the_input_work_budget() {
    let bundle = target_bundle();
    let document = parse("{}").unwrap();
    let names = std::iter::repeat_n("self.Target", 100_001);
    let error = bundle
        .compile_with_types(&document, None, names)
        .err()
        .expect("bounded duplicate inputs");
    assert_eq!(error.kind, NativeErrorKind::LimitExceeded);
    assert_eq!(error.span, None);
}
