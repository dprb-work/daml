//! Native bundle scoping, constraint compilation, defaults, and admission limits.

use daml::{Bundle, Span, parse};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("daml-native-{stamp}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn put(&self, path: &str, contents: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn compiles_and_resolves_a_real_file_bundle_with_defaults() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "app/Config.daml",
        "!Object {\n  name: !String\n  retries: !Int\n} = { retries: 3 }",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let config = parse("!self.app.Config { name: \"worker\" }").unwrap();
    let schemas = bundle.compile(&config, None).unwrap();
    assert_eq!(
        schemas.resolve(&config).unwrap().to_json(),
        json!({"name":"worker","retries":3})
    );
}

#[test]
fn native_local_facets_validate_strings_lists_and_maps() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Facets.daml",
        r#"!Object {
  code: !String {
    min: 2
    max: 4
    pattern: !Regex "\d+"
  }
  values: !List {
    items: !Number
    min: 2
    max: 3
    unique: true
  }
  labels: !Map {
    values: !String
    min: 1
    max: 2
  }
}"#,
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let config = |code: &str, values: &str, labels: &str| {
        format!(
            "{{\n  code: \"{code}\"\n  values: [\n    {values}\n  ]\n  labels: {{\n    {labels}\n  }}\n}}"
        )
    };
    let valid_source = config("١٢", "1\n    2", "one: \"x\"");
    let valid = parse(&valid_source).unwrap();
    let schemas = bundle.compile(&valid, Some("self.Facets")).unwrap();
    assert!(schemas.resolve_as(&valid, "self.Facets").is_ok());

    for (source, path) in [
        (config("a1", "1\n    2", "one: \"x\""), "/code"),
        (config("١", "1\n    2", "one: \"x\""), "/code"),
        (config("١٢٣٤٥", "1\n    2", "one: \"x\""), "/code"),
        (config("١٢", "1\n    1.0", "one: \"x\""), "/values"),
        (config("١٢", "1", "one: \"x\""), "/values"),
        (
            config("١٢", "1\n    2\n    3\n    4", "one: \"x\""),
            "/values",
        ),
        (config("١٢", "1\n    2", ""), "/labels"),
        (
            config(
                "١٢",
                "1\n    2",
                "one: \"x\"\n    two: \"y\"\n    three: \"z\"",
            ),
            "/labels",
        ),
    ] {
        let input = parse(&source).unwrap();
        let error = schemas.resolve_as(&input, "self.Facets").unwrap_err();
        assert_eq!(error.path, path, "{source}");
    }
}

#[test]
fn map_count_facets_compose_with_closed_records() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Counted.daml",
        "closed !Object {\n  first?: !String\n  second?: !String\n} & !Map {\n  min: 1\n  max: 1\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let valid = parse("{ first: \"x\" }").unwrap();
    let schemas = bundle.compile(&valid, Some("self.Counted")).unwrap();
    assert!(schemas.resolve_as(&valid, "self.Counted").is_ok());

    for source in [
        "{}",
        "{\n  first: \"x\"\n  second: \"y\"\n}",
        "{ other: \"x\" }",
    ] {
        let input = parse(source).unwrap();
        let error = schemas.resolve_as(&input, "self.Counted").unwrap_err();
        assert_eq!(error.path, "", "{source}");
    }
}

#[test]
fn regex_constraints_survive_default_check_schema_copies() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Defaulted.daml",
        "!Object {\n  code: !String { pattern: !Regex \"a\" }\n} = {\n  code: \"a\"\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("{}").unwrap();
    let schemas = bundle.compile(&input, Some("self.Defaulted")).unwrap();
    assert_eq!(
        schemas
            .resolve_as(&input, "self.Defaulted")
            .unwrap()
            .to_json(),
        json!({"code": "a"})
    );
}

#[test]
fn native_local_facets_reject_invalid_definitions() {
    for (source, expected) in [
        ("!String { min: -1 }", "non-negative integer"),
        ("!List { unique: false }", "must be true"),
        ("!Map { max: 1.5 }", "non-negative integer"),
    ] {
        let tree = Tree::new();
        tree.put("daml.bundle", "{}");
        tree.put("Invalid.daml", source);
        let input = parse("\"value\"").unwrap();
        let bundle = Bundle::open(tree.path()).unwrap();
        let error = match bundle.compile(&input, Some("self.Invalid")) {
            Err(error) => error,
            Ok(_) => panic!("invalid native facet must fail compilation"),
        };
        assert!(
            error.message.contains(expected),
            "{source}: {}",
            error.message
        );
    }
}

#[test]
fn native_exact_numeric_facets_validate_without_float_rounding() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Fraction.daml",
        "!Number {\n  exclusiveMin: 0\n  exclusiveMax: 1\n  multipleOf: 0.1\n}",
    );
    tree.put("Even.daml", "!Int { multipleOf: 2 }");
    tree.put("Hundredth.daml", "!Number { multipleOf: 1e-2 }");
    tree.put(
        "SmallPositive.daml",
        "!Int {\n  exclusiveMin: 0\n  exclusiveMax: 3\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let valid_fraction = parse("0.3").unwrap();
    let schemas = bundle
        .compile(&valid_fraction, Some("self.Fraction"))
        .unwrap();
    assert!(schemas.resolve_as(&valid_fraction, "self.Fraction").is_ok());
    for source in ["0", "1", "0.31"] {
        assert!(
            schemas
                .resolve_as(&parse(source).unwrap(), "self.Fraction")
                .is_err(),
            "{source}"
        );
    }

    let even = parse("4").unwrap();
    let schemas = bundle.compile(&even, Some("self.Even")).unwrap();
    assert!(schemas.resolve_as(&even, "self.Even").is_ok());
    assert!(
        schemas
            .resolve_as(&parse("3").unwrap(), "self.Even")
            .is_err()
    );

    let hundredth = parse("0.03").unwrap();
    let schemas = bundle.compile(&hundredth, Some("self.Hundredth")).unwrap();
    assert!(schemas.resolve_as(&hundredth, "self.Hundredth").is_ok());
    assert!(
        schemas
            .resolve_as(&parse("0.031").unwrap(), "self.Hundredth")
            .is_err()
    );

    let small_positive = parse("1").unwrap();
    let schemas = bundle
        .compile(&small_positive, Some("self.SmallPositive"))
        .unwrap();
    assert!(
        schemas
            .resolve_as(&small_positive, "self.SmallPositive")
            .is_ok()
    );
    for source in ["0", "3"] {
        assert!(
            schemas
                .resolve_as(&parse(source).unwrap(), "self.SmallPositive")
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn native_exact_numeric_facets_reject_conflicts_and_invalid_divisors() {
    for (source, expected) in [
        (
            "!Number {\n  min: 0\n  exclusiveMin: 0\n}",
            "both \"min\" and \"exclusiveMin\"",
        ),
        (
            "!Number {\n  max: 1\n  exclusiveMax: 1\n}",
            "both \"max\" and \"exclusiveMax\"",
        ),
        (
            "!Number { multipleOf: 0 }",
            "\"multipleOf\" requires a positive number",
        ),
        (
            "!Number { multipleOf: 0e1 }",
            "\"multipleOf\" requires a positive number",
        ),
        (
            "!Int { multipleOf: -2 }",
            "\"multipleOf\" requires a positive number",
        ),
    ] {
        let tree = Tree::new();
        tree.put("daml.bundle", "{}");
        tree.put("Invalid.daml", source);
        let input = parse("1").unwrap();
        let bundle = Bundle::open(tree.path()).unwrap();
        let error = match bundle.compile(&input, Some("self.Invalid")) {
            Err(error) => error,
            Ok(_) => panic!("invalid numeric facets must fail compilation"),
        };
        assert!(
            error.message.contains(expected),
            "{source}: {}",
            error.message
        );
    }
}

#[test]
fn native_regex_preserves_verbose_comments_and_reports_source_errors() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Verbose.daml",
        r#"!String { pattern: !Regex "(?x)a|abc # explanation" }"#,
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let valid = parse("\"abc\"").unwrap();
    let schemas = bundle.compile(&valid, Some("self.Verbose")).unwrap();
    assert!(schemas.resolve_as(&valid, "self.Verbose").is_ok());
    assert!(
        schemas
            .resolve_as(&parse("\"zabc\"").unwrap(), "self.Verbose")
            .is_err()
    );

    let invalid_source = r#"!String { pattern: !Regex "(?=prefix).*" }"#;
    tree.put("InvalidRegex.daml", invalid_source);
    tree.put("Host.daml", "!self.InvalidRegex");
    let error = match bundle.compile(&valid, Some("self.Host")) {
        Err(error) => error,
        Ok(_) => panic!("invalid Regex must fail compilation"),
    };
    let pattern_start = invalid_source.find("(?=prefix)").unwrap();
    assert_eq!(
        error.path.as_deref(),
        Some(tree.path().join("InvalidRegex.daml").as_path()),
        "{error:?}"
    );
    assert_eq!(
        error.span,
        Some(Span {
            start: pattern_start,
            end: pattern_start + "(?=prefix).*".len(),
        })
    );
    assert!(error.message.contains("look-around"), "{}", error.message);
}

#[test]
fn discovery_uses_nearest_manifest_and_open_requires_manifest() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("nested/daml.bundle", "{}");
    tree.put("Local.daml", "!Int");
    tree.put("nested/Local.daml", "!String");
    let found = Bundle::discover(tree.path().join("nested/input.daml"))
        .unwrap()
        .unwrap();
    let input = parse("\"nearest\"").unwrap();
    let schemas = found.compile(&input, Some("self.Local")).unwrap();
    assert_eq!(
        schemas.resolve_as(&input, "self.Local").unwrap().to_json(),
        json!("nearest")
    );
    let no_manifest = tree.path().join("empty");
    fs::create_dir(&no_manifest).unwrap();
    let error = match Bundle::open(&no_manifest) {
        Err(error) => error,
        Ok(_) => panic!("explicitly opened bundle must require its manifest"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::Io);
}

#[test]
fn compilation_checks_contradictory_defaults_but_allows_partial_record_defaults() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Bad.daml", "!Int { min: 3 } = 2");
    tree.put(
        "Partial.daml",
        "!Object {\n  required: !String\n  optional?: !Bool\n} = {}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();

    let invalid = parse("!self.Bad").unwrap();
    let error = match bundle.compile(&invalid, None) {
        Err(error) => error,
        Ok(_) => panic!("contradictory default must fail compilation"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);

    let partial = parse("!self.Partial").unwrap();
    let schemas = bundle.compile(&partial, None).unwrap();
    let error = schemas.resolve(&partial).unwrap_err();
    assert_eq!(error.kind, daml::ResolveErrorKind::SchemaViolation);
    assert_eq!(error.path, "");
}
#[test]
fn partial_inherited_default_still_checks_narrowed_fields() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object {\n  name: !String\n  timeout: !Int\n} = { timeout: 30 }",
    );
    tree.put(
        "Narrow.daml",
        "!self.Base {\n  timeout: !Int { max: 10 }\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("!self.Narrow").unwrap();
    let error = match bundle.compile(&input, None) {
        Err(error) => error,
        Ok(_) => panic!("partial inherited defaults must still validate present fields"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}
#[test]
fn reachable_definition_source_aggregate_is_bounded() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{\n  dep: \"./dep\"\n}");
    let comment = "x".repeat(4_300_000);
    tree.put("A.daml", &format!("!dep.B\n#{comment}\n"));
    tree.put("dep/daml.bundle", "{}");
    tree.put("dep/B.daml", &format!("!String\n#{comment}\n"));
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("null").unwrap();
    let error = match bundle.compile(&input, Some("self.A")) {
        Err(error) => error,
        Ok(_) => panic!("reachable source aggregate must remain bounded"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::LimitExceeded);
}

#[test]
fn replacement_and_unset_remove_inherited_presets_without_erasing_requirements() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "closed !Object {\n endpoint: !String\n retries: !Int\n} = {\n endpoint: \"base\"\n retries: 3\n}",
    );
    tree.put("Replacement.daml", "!self.Base = { endpoint: \"child\" }");
    tree.put("Unset.daml", "!self.Base = unset");
    let bundle = Bundle::open(tree.path()).unwrap();

    let partial = parse("!self.Replacement").unwrap();
    let schemas = bundle.compile(&partial, None).unwrap();
    assert_eq!(
        schemas.resolve(&partial).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let supplied = parse("!self.Replacement { retries: 7 }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"endpoint": "child", "retries": 7})
    );
    let unset = parse("!self.Unset { retries: 7 }").unwrap();
    let schemas = bundle.compile(&unset, None).unwrap();
    assert_eq!(
        schemas.resolve(&unset).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let supplied = parse("!self.Unset {\n endpoint: \"explicit\"\n retries: 7\n}").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"endpoint": "explicit", "retries": 7})
    );
}

#[test]
fn inherited_invalid_container_preset_fails_even_when_configuration_supplies_value() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { timeout: !Object { value: !Int } = { value: 30 } }",
    );
    tree.put(
        "Narrow.daml",
        "!self.Base { timeout: !Object { value: !Int { max: 10 } } }",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let supplied = parse("!self.Narrow { timeout: { value: 5 } }").unwrap();
    let error = match bundle.compile(&supplied, None) {
        Err(error) => error,
        Ok(_) => panic!("inherited invalid default must fail compilation"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}
#[test]
fn union_branch_defaults_are_checked_without_rejecting_valid_ambiguity() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Invalid.daml", "(!Int = \"bad\") | !String");
    tree.put("ValidUnion.daml", "(!Int = 1) | (!String = \"text\")");
    tree.put("Valid.daml", "!Object { choice: !self.ValidUnion }");
    let bundle = Bundle::open(tree.path()).unwrap();

    let invalid = parse("!self.Invalid").unwrap();
    let error = match bundle.compile(&invalid, None) {
        Err(error) => error,
        Ok(_) => panic!("invalid explicit union-branch default must fail compilation"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);

    let valid = parse("!self.Valid").unwrap();
    let schemas = bundle.compile(&valid, None).unwrap();
    assert_eq!(
        schemas.resolve(&valid).unwrap_err().kind,
        daml::ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn optional_references_stay_absent_while_required_and_supplied_objects_complete() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "ObjectPreset.daml",
        "!Object {\n value: !Int\n origin: !String\n} = { value: 1\n origin: \"preset\" }",
    );
    tree.put("ScalarPreset.daml", "!Int = 3");
    tree.put(
        "Host.daml",
        "!Object {\n  object?: !self.ObjectPreset\n  scalar?: !self.ScalarPreset\n  required_object: !self.ObjectPreset\n  required_scalar: !self.ScalarPreset\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();

    let omitted = parse("!self.Host").unwrap();
    let schemas = bundle.compile(&omitted, None).unwrap();
    assert_eq!(
        schemas.resolve(&omitted).unwrap().to_json(),
        json!({"required_object": {"value": 1, "origin": "preset"}, "required_scalar": 3})
    );
    let partial = parse("!self.Host { object: {} }").unwrap();
    assert_eq!(
        schemas.resolve(&partial).unwrap().to_json(),
        json!({"object": {"value": 1, "origin": "preset"}, "required_object": {"value": 1, "origin": "preset"}, "required_scalar": 3})
    );
}

#[test]
fn unknown_definition_alias_reports_declaring_file_span_and_reference() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Config.daml", "!missing.Record");
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("!self.Config").unwrap();
    let error = match bundle.compile(&input, None) {
        Err(error) => error,
        Ok(_) => panic!("unknown alias in loaded definition must fail compilation"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::UnknownReference);
    assert_eq!(error.path, Some(tree.path().join("Config.daml")));
    assert_eq!(error.reference.as_deref(), Some("missing.Record"));
    assert_eq!(error.span, Some(daml::Span { start: 1, end: 15 }));
}

#[test]
fn closing_an_extended_open_base_preserves_fields_but_cannot_reopen_a_closed_base() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Open.daml", "!Object { base: !Int }");
    tree.put("Extended.daml", "closed !self.Open { extra: !Bool }");
    tree.put("Closed.daml", "closed !self.Open");
    tree.put("ForbiddenExtension.daml", "!self.Closed { extra: !Bool }");
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("!self.Extended {\n base: 1\n extra: true\n}").unwrap();
    let schemas = bundle.compile(&input, None).unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"base": 1, "extra": true})
    );
    let unknown = parse("!self.Extended {\n base: 1\n extra: true\n unknown: 0\n}").unwrap();
    assert_eq!(
        schemas.resolve(&unknown).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let forbidden = parse("!self.ForbiddenExtension {\n base: 1\n extra: true\n}").unwrap();
    let schemas = bundle.compile(&forbidden, None).unwrap();
    assert_eq!(
        schemas.resolve(&forbidden).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn explicit_numeric_constraints_and_optional_containers_stay_stable() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Numbers.daml",
        "!Object {\n integral: !Int\n decimal: !Number\n exponent: !Number\n optional?: !Object { value: !String }\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let supplied =
        parse("!self.Numbers {\n integral: 3.0\n decimal: 3.5\n exponent: 4.5\n}").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    let resolved = schemas.resolve(&supplied).unwrap();
    assert_eq!(
        resolved.to_json(),
        json!({"integral": 3.0, "decimal": 3.5, "exponent": 4.5})
    );
    let fraction = parse("!self.Numbers { integral: 3.5\n decimal: 3.5\n exponent: 4.5 }").unwrap();
    assert_eq!(schemas.resolve(&fraction).unwrap_err().path, "/integral");
}

#[test]
fn bundle_import_cycles_and_recursive_definitions_preserve_scope_and_bounds() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{ left: \"./a\"\nagain: \"./a\" }");
    tree.put("a/daml.bundle", "{ dep: \"../b\" }");
    tree.put("b/daml.bundle", "{ dep: \"../a\" }");
    tree.put("a/Root.daml", "!Object { value: !dep.Item }");
    tree.put("b/Item.daml", "!Int = 9");
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("{ first: !left.Root\n second: !again.Root }").unwrap();
    let schemas = bundle.compile(&input, None).unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"first": {"value": 9}, "second": {"value": 9}})
    );
    tree.put("b/Item.daml", "!dep.Root");
    let schemas = bundle.compile(&input, None).unwrap();
    let error = schemas.resolve(&input).unwrap_err();
    assert_eq!(error.kind, daml::ResolveErrorKind::LimitExceeded);
}

#[test]
fn member_object_preset_replacement_applies_to_supplied_partial_objects() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Base.daml", "!Object { x: !Object {} = { a: 1\n b: 1 } }");
    tree.put("Child.daml", "!self.Base { x: !Object {} = { a: 2 } }");
    tree.put("Third.daml", "!self.Child { x: !Object {} = { a: 4 } }");
    tree.put("Unset.daml", "!self.Base { x: !Object = unset }");
    let bundle = Bundle::open(tree.path()).unwrap();
    for (tag, expected) in [
        ("Child", json!({"x": {"a": 2, "c": 3}})),
        ("Third", json!({"x": {"a": 4, "c": 3}})),
        ("Unset", json!({"x": {"c": 3}})),
    ] {
        let input = parse(format!("!self.{tag} {{ x: {{ c: 3 }} }}")).unwrap();
        let schemas = bundle.compile(&input, None).unwrap();
        assert_eq!(
            schemas.resolve(&input).unwrap().to_json(),
            expected,
            "{tag}"
        );
    }
}

#[test]
fn nested_member_replacement_keeps_other_member_defaults() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { x: !Object {\n a: !Object { value: !Int } = { value: 1 }\n b: !Object { value: !Int } = { value: 2 }\n} }",
    );
    tree.put(
        "Child.daml",
        "!self.Base { x: !Object { a: !Object { value: !Int } = { value: 3 }\n b: !Object { value: !Int } } }",
    );
    let input = parse("!self.Child").unwrap();
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"x": {"a": {"value": 3}, "b": {"value": 2}}})
    );
}

#[test]
fn inherited_nested_container_presets_are_checked_against_narrowing() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { x: !Object { timeout: !Int } = { timeout: 30 } }",
    );
    tree.put(
        "Child.daml",
        "!self.Base { x: !Object { timeout: !Int { max: 10 } } }",
    );
    let input = parse("!self.Child { x: { timeout: 5 } }").unwrap();
    let error = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .err()
        .unwrap();
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn replacement_container_preset_must_satisfy_inherited_constraints() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { value: !Object { count: !Int { min: 10 } } }",
    );
    tree.put(
        "Child.daml",
        "!self.Base { value: !Object { count: !Int { min: 10 } } = { count: 3 } }",
    );
    let input = parse("!self.Child { value: { count: 20 } }").unwrap();
    let error = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .err()
        .unwrap();
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn conditional_member_overrides_do_not_erase_other_union_branches_inherited_defaults() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Base.daml", "!Object { x: !Object { a: !Int } = { a: 1 } }");
    tree.put(
        "Child.daml",
        "!self.Base { x: (!Object { a: !Int } = { a: 2 }) | !Object {} }",
    );
    tree.put("Wrapper.daml", "!self.Child = {}");
    let bundle = Bundle::open(tree.path()).unwrap();
    for tag in ["Child", "Wrapper"] {
        let input = parse(format!("!self.{tag}")).unwrap();
        let schemas = bundle.compile(&input, None).unwrap();
        assert_eq!(
            schemas.resolve(&input).unwrap_err().kind,
            daml::ResolveErrorKind::AmbiguousDefault,
            "{tag}"
        );
    }
}

#[test]
fn manifest_admission_errors_identify_the_invalid_alias_or_locator() {
    for (source, target, kind) in [
        (
            "{ self: \"./child\" }",
            "self",
            daml::NativeErrorKind::InvalidName,
        ),
        (
            "{ provider: 3 }",
            "3",
            daml::NativeErrorKind::InvalidManifest,
        ),
    ] {
        let tree = Tree::new();
        tree.put("daml.bundle", source);
        let error = Bundle::open(tree.path()).unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.path, Some(tree.path().join("daml.bundle")));
        let span = error.span.unwrap();
        assert_eq!(&source[span.start..span.end], target);
    }
}
#[test]
fn inherited_union_member_presets_are_replaced_and_unset_atomically() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object {\n  x: (\n    !Object = {\n      a: 1\n      b: 1\n    }\n  ) | (\n    !Object = {\n      a: 2\n      b: 1\n    }\n  )\n}",
    );
    tree.put(
        "Child.daml",
        "!self.Base {\n  x: !Object = {\n    a: 3\n  }\n}",
    );
    tree.put("Unset.daml", "!self.Base {\n  x: !Object = unset\n}");
    let bundle = Bundle::open(tree.path()).unwrap();

    let child = parse("!self.Child").unwrap();
    let schemas = bundle.compile(&child, None).unwrap();
    assert_eq!(
        schemas.resolve(&child).unwrap().to_json(),
        json!({"x": {"a": 3}})
    );
    let supplied = parse("!self.Child { x: {} }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"x": {"a": 3}})
    );

    let unset = parse("!self.Unset").unwrap();
    let schemas = bundle.compile(&unset, None).unwrap();
    assert_eq!(schemas.resolve(&unset).unwrap().to_json(), json!({"x": {}}));
}

#[test]
fn inherited_object_preset_remains_available_to_untouched_union_fallback() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object {\n  x: !Object {\n    a: !Object { value: !Int }\n  }\n} = {\n  x: {\n    a: { value: 1 }\n  }\n}",
    );
    tree.put(
        "Child.daml",
        "!self.Base {\n  x: (\n    !Object {\n      a: !Object { value: !Int } = { value: 2 }\n    } = {\n      a: { value: 2 }\n    }\n  ) | !Object {}\n}",
    );
    tree.put("Wrapper.daml", "!self.Child = {}");
    let bundle = Bundle::open(tree.path()).unwrap();

    let child = parse("!self.Child").unwrap();
    let schemas = bundle.compile(&child, None).unwrap();
    assert_eq!(
        schemas.resolve(&child).unwrap_err().kind,
        daml::ResolveErrorKind::AmbiguousDefault
    );

    let wrapper = parse("!self.Wrapper").unwrap();
    let schemas = bundle.compile(&wrapper, None).unwrap();
    assert_eq!(
        schemas.resolve(&wrapper).unwrap_err().kind,
        daml::ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn inherited_container_presets_are_checked_across_explicit_conjunctions() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { timeout: !Object { value: !Int } = { value: 30 } }",
    );
    tree.put(
        "Narrow.daml",
        "!self.Base & !Object { timeout: !Object { value: !Int { max: 10 } } }",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let supplied = parse("!self.Narrow { timeout: { value: 5 } }").unwrap();
    let error = match bundle.compile(&supplied, None) {
        Err(error) => error,
        Ok(_) => panic!("inherited invalid default in conjunction must fail compilation"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn grouped_container_presets_preserve_replacement_and_unset() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { x: !Object { value: !Int } = { value: 1 } }",
    );
    tree.put(
        "Replacement.daml",
        "!self.Base { x: (!Object { value: !Int } = { value: 2 }) }",
    );
    tree.put("Unset.daml", "!self.Base { x: (!Object = unset) }");
    let bundle = Bundle::open(tree.path()).unwrap();
    let replacement = parse("!self.Replacement").unwrap();
    let schemas = bundle.compile(&replacement, None).unwrap();
    assert_eq!(
        schemas.resolve(&replacement).unwrap().to_json(),
        json!({"x": {"value": 2}})
    );
    let unset = parse("!self.Unset").unwrap();
    let schemas = bundle.compile(&unset, None).unwrap();
    assert_eq!(
        schemas.resolve(&unset).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn reference_and_expression_depth_share_one_compilation_bound() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("D20.daml", "!String");
    for index in 0..20 {
        let source = format!(
            "{}!self.D{}{}",
            "!Object { child: ".repeat(8),
            index + 1,
            " }".repeat(8)
        );
        tree.put(&format!("D{index}.daml"), &source);
    }
    let input = parse("!self.D0").unwrap();
    let error = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .err()
        .unwrap();
    assert_eq!(error.kind, daml::NativeErrorKind::LimitExceeded);
}
#[test]
fn grouped_nested_default_assertions_fail_compilation_even_when_not_used() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Host.daml",
        "!Object {\n  value: (!Object = {\n    child: !Int \"bad\"\n  })\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let supplied = parse("!self.Host { value: {} }").unwrap();
    let error = match bundle.compile(&supplied, None) {
        Err(error) => error,
        Ok(_) => panic!("contradictory nested assertion in grouped default must fail compilation"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn inherited_list_item_defaults_are_checked_against_positional_conjunctions() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!List {\n  items: !Object { timeout: !Int } = { timeout: 30 }\n}",
    );
    tree.put(
        "Narrow.daml",
        "!self.Base & !Tuple [\n  !Object {\n    timeout: !Int { max: 10 }\n  }\n]",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let supplied = parse("!self.Narrow [{ timeout: 5 }]").unwrap();
    let error = match bundle.compile(&supplied, None) {
        Err(error) => error,
        Ok(_) => panic!("inherited list-item default excluded by positional constraint"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn inherited_map_value_defaults_are_checked_against_named_object_conjunctions() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Base.daml", "!Map {\n  values: !Int = 30\n}");
    tree.put(
        "Narrow.daml",
        "!self.Base & !Object {\n  timeout: !Int {\n    max: 10\n  }\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let supplied = parse("!self.Narrow { timeout: 5 }").unwrap();
    let error = match bundle.compile(&supplied, None) {
        Err(error) => error,
        Ok(_) => panic!("inherited map-value default excluded by named-object constraint"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn long_warmed_definition_chain_is_rejected_by_native_depth_limit() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("D0.daml", "!Object {}");
    let mut tags = Vec::with_capacity(141);
    tags.push("!self.D0".to_owned());
    for index in 1..=140 {
        tree.put(&format!("D{index}.daml"), &format!("!self.D{}", index - 1));
        tags.push(format!("!self.D{index}"));
    }
    let input = parse(format!("[{}]", tags.join("\n"))).unwrap();
    let bundle = Bundle::open(tree.path()).unwrap();
    let error = match bundle.compile(&input, None) {
        Err(error) => error,
        Ok(_) => panic!("definition chain beyond native schema depth must be rejected"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::LimitExceeded);
}

#[test]
fn repeated_large_defaults_exceed_bounded_compiler_materialization() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    let default = "x".repeat(1_000_000);
    tree.put("Base.daml", &format!("!String = \"{default}\""));
    let mut tags = Vec::with_capacity(80);
    for index in 0..80 {
        tree.put(&format!("A{index}.daml"), "!self.Base & !String");
        tags.push(format!("!self.A{index} \"\""));
    }
    let input = parse(format!("[{}]", tags.join("\n"))).unwrap();
    let bundle = Bundle::open(tree.path()).unwrap();
    let error = match bundle.compile(&input, None) {
        Err(error) => error,
        Ok(_) => panic!("repeated large defaults must exceed bounded compiler materialization"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::LimitExceeded);
}

#[test]
fn named_member_defaults_replace_and_unset_inherited_map_value_defaults() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Map { values: !Object { a: !Int } = { a: 1 } }",
    );
    tree.put("Child.daml", "!self.Base { x: !Object = { a: 2 } }");
    tree.put("Unset.daml", "!self.Base { x: !Object = unset }");
    let bundle = Bundle::open(tree.path()).unwrap();
    let child = parse("!self.Child").unwrap();
    let schemas = bundle.compile(&child, None).unwrap();
    assert_eq!(
        schemas.resolve(&child).unwrap().to_json(),
        json!({"x": {"a": 2}})
    );
    let unset = parse("!self.Unset").unwrap();
    let schemas = bundle.compile(&unset, None).unwrap();
    assert_eq!(
        schemas.resolve(&unset).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let supplied = parse("!self.Unset { x: { a: 7 }\n other: {} }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"x": {"a": 7}, "other": {"a": 1}})
    );
}

#[test]
fn partial_counted_defaults_are_checked_after_member_completion() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Counted.daml",
        "(!Object { a: !Int } & !Map { min: 1 }) = { a: 1 }",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("!self.Counted").unwrap();
    let schemas = bundle.compile(&input, None).unwrap();
    assert_eq!(schemas.resolve(&input).unwrap().to_json(), json!({"a": 1}));
    let invalid = parse("!self.Counted { a: \"wrong\" }").unwrap();
    assert_eq!(
        schemas.resolve(&invalid).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn partial_object_counts_defer_missing_fields_but_check_completed_upper_bounds() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Counted.daml",
        "(!Object { child: !Object {} } & !Map { min: 1 }) = {}",
    );
    let input = parse("!self.Counted").unwrap();
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"child": {}})
    );
    tree.put(
        "Counted.daml",
        "(!Object { a: !Int\n b: !Int } & !Map { max: 1 }) = { a: 1\n b: 2 }",
    );
    let error = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .err()
        .unwrap();
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn enclosing_default_override_is_not_restored_by_union_normalization() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Object { x: !Object { a: !Object { value: !Int } } = { a: { value: 1 } } }",
    );
    for assignment in ["{}", "unset"] {
        tree.put(
            "Child.daml",
            &format!(
                "!self.Base {{ x: (!Object {{ a: !Object {{ value: !Int }} = {{ value: 2 }} }} | !Object {{}}) = {assignment} }}"
            ),
        );
        let bundle = Bundle::open(tree.path()).unwrap();
        let input = parse("!self.Child").unwrap();
        let schemas = bundle.compile(&input, None).unwrap();
        assert_eq!(
            schemas.resolve(&input).unwrap().to_json(),
            json!({"x": {"a": {"value": 2}}}),
            "{assignment}"
        );
    }
}

#[test]
fn tuple_constraints_enforce_length_order_nesting_and_allow_duplicate_values() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Pair.daml", "!Tuple [\n  !Int\n  !String\n]");
    tree.put(
        "Nested.daml",
        "!Tuple [\n  !Tuple [\n    !Int\n    !String\n  ]\n  !Int\n]",
    );
    tree.put("Duplicates.daml", "!Tuple [\n  !Int\n  !Int\n]");
    tree.put("Empty.daml", "!Tuple []");
    let bundle = Bundle::open(tree.path()).unwrap();

    for (tag, source, expected) in [
        ("Pair", "!self.Pair [1\n\"x\"]", Some(json!([1, "x"]))),
        (
            "Nested",
            "!self.Nested [[1\n\"x\"]\n2]",
            Some(json!([[1, "x"], 2])),
        ),
        ("Duplicates", "!self.Duplicates [1\n1]", Some(json!([1, 1]))),
        ("Empty", "!self.Empty []", Some(json!([]))),
        ("Pair", "!self.Pair [1]", None),
        ("Pair", "!self.Pair [\"x\"\n1]", None),
        ("Nested", "!self.Nested [[1]\n2]", None),
        ("Empty", "!self.Empty [1]", None),
    ] {
        let input = parse(source).unwrap();
        let schemas = bundle.compile(&input, None).unwrap();
        match expected {
            Some(value) => assert_eq!(schemas.resolve(&input).unwrap().to_json(), value, "{tag}"),
            None => assert_eq!(
                schemas.resolve(&input).unwrap_err().kind,
                daml::ResolveErrorKind::SchemaViolation,
                "{tag}"
            ),
        }
    }
}

#[test]
fn bare_array_definition_does_not_retain_positional_constraint_semantics() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Legacy.daml", "[!Int]");
    let input = parse("!self.Legacy [1]").unwrap();
    match Bundle::open(tree.path()).unwrap().compile(&input, None) {
        Err(_) => {}
        Ok(schemas) => {
            assert_eq!(
                schemas.resolve(&input).err().unwrap().kind,
                daml::ResolveErrorKind::SchemaViolation
            );
        }
    }
}

#[test]
fn whole_tuple_defaults_replace_and_unset_without_merging_positions() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Tuple [\n  !Object { x: !Int }\n] = [{ x: 1 }]",
    );
    tree.put("Replacement.daml", "!self.Base = [{ x: 2 }]");
    tree.put("Unset.daml", "!self.Base = unset");
    tree.put("Host.daml", "!Object { value: !self.Replacement }");
    tree.put("UnsetHost.daml", "!Object { value: !self.Unset }");
    let bundle = Bundle::open(tree.path()).unwrap();

    let omitted = parse("!self.Host {}").unwrap();
    let schemas = bundle.compile(&omitted, None).unwrap();
    assert_eq!(
        schemas.resolve(&omitted).unwrap().to_json(),
        json!({"value": [{"x": 2}]})
    );
    let supplied = parse("!self.Host { value: [{ x: 3 }] }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"value": [{"x": 3}]})
    );

    let omitted = parse("!self.UnsetHost {}").unwrap();
    let schemas = bundle.compile(&omitted, None).unwrap();
    assert_eq!(
        schemas.resolve(&omitted).err().unwrap().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let supplied = parse("!self.UnsetHost { value: [{ x: 4 }] }").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"value": [{"x": 4}]})
    );
    let missing_position = parse("!self.UnsetHost { value: [] }").unwrap();
    assert_eq!(
        schemas.resolve(&missing_position).err().unwrap().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn tuple_member_container_presets_project_through_grouping_and_references() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Tuple [!Object { x: !Object { value: !Int } = { value: 1 } }]",
    );
    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }]",
    );
    let input = parse("!self.Child [{}]").unwrap();
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!([{"x": {"value": 2}}])
    );
    let supplied = parse("!self.Child [{ x: { value: 3 } }]").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!([{"x": {"value": 3}}])
    );

    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object = unset }]",
    );
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).err().unwrap().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let supplied = parse("!self.Child [{ x: { value: 3 } }]").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!([{"x": {"value": 3}}])
    );
    let missing = parse("!self.Child []").unwrap();
    assert_eq!(
        schemas.resolve(&missing).err().unwrap().kind,
        daml::ResolveErrorKind::SchemaViolation
    );

    for (index, (case, child)) in [
        (
            "grouped-tuple",
            "!self.Base & (!Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }])",
        ),
        (
            "grouped-reference",
            "(!self.Base) & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }]",
        ),
        (
            "nested-groups",
            "((!self.Base)) & ((!Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }]))",
        ),
        (
            "left-associated-conjunction",
            "(!self.Base & !List { items: !Object { x: !Object { value: !Int } } }) & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }]",
        ),
        (
            "right-associated-conjunction",
            "!self.Base & (!List { items: !Object { x: !Object { value: !Int } } } & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }])",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("Grouped{index}-{case}");
        tree.put(&format!("{name}.daml"), child);
        let omitted = parse(format!("!self.{name} [{{}}]")).unwrap();
        let schemas = Bundle::open(tree.path())
            .unwrap()
            .compile(&omitted, None)
            .unwrap();
        assert_eq!(
            schemas.resolve(&omitted).unwrap().to_json(),
            json!([{"x": {"value": 2}}]),
            "{child}"
        );
        let supplied =
            parse(format!("!self.{name} [{{ x: {{ value: 3 }} }}]")).unwrap();
        assert_eq!(
            schemas.resolve(&supplied).unwrap().to_json(),
            json!([{"x": {"value": 3}}]),
            "{child}"
        );
        let wrong_arity = parse(format!("!self.{name} []")).unwrap();
        assert_eq!(
            schemas.resolve(&wrong_arity).err().unwrap().kind,
            daml::ResolveErrorKind::SchemaViolation,
            "{child}"
        );

        let unset = child.replace("= { value: 2 }", "= unset");
        tree.put(&format!("{name}.daml"), &unset);
        let omitted = parse(format!("!self.{name} [{{}}]")).unwrap();
        let schemas = Bundle::open(tree.path())
            .unwrap()
            .compile(&omitted, None)
            .unwrap();
        assert_eq!(
            schemas.resolve(&omitted).err().unwrap().kind,
            daml::ResolveErrorKind::SchemaViolation,
            "{unset}"
        );
        let supplied =
            parse(format!("!self.{name} [{{ x: {{ value: 3 }} }}]")).unwrap();
        assert_eq!(
            schemas.resolve(&supplied).unwrap().to_json(),
            json!([{"x": {"value": 3}}]),
            "{unset}"
        );
        let wrong_arity = parse(format!("!self.{name} []")).unwrap();
        assert_eq!(
            schemas.resolve(&wrong_arity).err().unwrap().kind,
            daml::ResolveErrorKind::SchemaViolation,
            "{unset}"
        );
    }
}

#[test]
fn positional_overrides_specialize_homogeneous_item_container_presets() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!List { items: !Object { x: !Object { value: !Int } = { value: 1 } } }",
    );
    let input = parse("!self.Child [{}\n{}]").unwrap();
    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }\n!Object {}]",
    );
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!([{"x": {"value": 2}}, {"x": {"value": 1}}])
    );

    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object = unset }\n!Object {}]",
    );
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).err().unwrap().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
    let supplied = parse("!self.Child [{ x: { value: 3 } }\n{}]").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!([{"x": {"value": 3}}, {"x": {"value": 1}}])
    );
}

#[test]
fn positional_union_keeps_inherited_container_presets_in_untouched_alternatives() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Tuple [!Object { x: !Object { value: !Int } = { value: 1 } }]",
    );
    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } } | !Object {}]",
    );
    let input = parse("!self.Child [{}]").unwrap();
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).err().unwrap().kind,
        daml::ResolveErrorKind::AmbiguousDefault
    );
    let supplied = parse("!self.Child [{ x: { value: 3 } }]").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!([{"x": {"value": 3}}])
    );
}

#[test]
fn tuple_member_container_override_projects_through_union_alternatives() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Tuple [!Object { x: !Object { value: !Int } = { value: 1 } }]",
    );
    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } }] & (!Tuple [!Object { y: !Object { value: !Int } = { value: 3 } }] | !Tuple [!Object {}])",
    );
    let supplied = parse("!self.Child [{ y: { value: 5 } }]").unwrap();
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&supplied, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!([{"x": {"value": 2}, "y": {"value": 5}}])
    );

    let omitted = parse("!self.Child [{}]").unwrap();
    assert_eq!(
        schemas.resolve(&omitted).err().unwrap().kind,
        daml::ResolveErrorKind::AmbiguousDefault
    );
}

#[test]
fn supplied_tuple_does_not_receive_fields_from_replaced_whole_array_preset() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put(
        "Base.daml",
        "!Tuple [!Object { x: !Object { value: !Int } }] = [{ x: { value: 1 } }]",
    );
    tree.put(
        "Child.daml",
        "!self.Base & !Tuple [!Object { x: !Object { value: !Int } = { value: 2 } } | !Object {}]",
    );
    let input = parse("!self.Child [{}]").unwrap();
    let schemas = Bundle::open(tree.path())
        .unwrap()
        .compile(&input, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!([{"x": {"value": 2}}])
    );
    let supplied = parse("!self.Child [{ x: { value: 3 } }]").unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!([{"x": {"value": 3}}])
    );
}

#[test]
fn tuple_position_defaults_are_rejected_even_when_grouped_or_combined() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    for (index, position) in [
        "!Int = 1",
        "(!Int = 1)",
        "(!Int = 1) | !String",
        "(!Int = 1) & !Number",
        "(!Int | !String) = 1",
        "(!Int & !Number) = 1",
        "!Int = unset",
    ]
    .into_iter()
    .enumerate()
    {
        tree.put(
            &format!("Invalid{index}.daml"),
            &format!("!Tuple [{position}]"),
        );
    }
    let bundle = Bundle::open(tree.path()).unwrap();
    for index in 0..7 {
        let input = parse(format!("!self.Invalid{index} [1]")).unwrap();
        assert!(
            bundle.compile(&input, None).is_err(),
            "position {index} must not admit a scalar fallback or unset"
        );
    }
}

#[test]
fn tuple_metadata_is_rejected_during_definition_compilation() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Invalid.daml", "!Tuple { min: 1 }");
    let input = parse("!self.Invalid [1]").unwrap();
    assert!(
        Bundle::open(tree.path())
            .unwrap()
            .compile(&input, None)
            .is_err()
    );
}

#[test]
fn scalar_reference_field_unset_defaults_are_rejected() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Scalar.daml", "!Int");
    tree.put("Host.daml", "!Object { value: (!self.Scalar = unset) }");
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("!self.Host").unwrap();
    let error = match bundle.compile(&input, None) {
        Err(error) => error,
        Ok(_) => panic!("a scalar reference cannot provide a field unset default"),
    };
    assert_eq!(error.kind, daml::NativeErrorKind::InvalidDefault);
}

#[test]
fn literal_array_unset_preserves_its_container_constraint() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("ConstContainer.daml", "!Object { fixed: [] = unset }");
    let bundle = Bundle::open(tree.path()).unwrap();
    let input = parse("!self.ConstContainer { fixed: [] }").unwrap();
    let schemas = bundle.compile(&input, None).unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({"fixed": []})
    );

    let invalid = parse("!self.ConstContainer { fixed: [1] }").unwrap();
    assert_eq!(
        schemas.resolve(&invalid).unwrap_err().kind,
        daml::ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn retained_root_nested_reference_and_metadata_defaults_resolve() {
    let tree = Tree::new();
    tree.put("daml.bundle", "{}");
    tree.put("Root.daml", "!Int = 3");
    tree.put("Nested.daml", "!Object { value: !Int } = { value: 4 }");
    tree.put("NestedReference.daml", "!self.Nested = { value: 5 }");
    tree.put(
        "NestedUnset.daml",
        "!Object { child: !self.Nested = unset }",
    );
    tree.put(
        "Host.daml",
        "!Object {\n  root: !self.Root\n  nested: !Object { value: !Int } = { value: 6 }\n  referenced: !self.Nested = { value: 12 }\n  items: !List { items: !Int = 7 } = [8]\n  mapped: !Map { values: !Int = 9 } = { entry: 10 }\n}",
    );
    let bundle = Bundle::open(tree.path()).unwrap();
    for (tag, expected) in [
        ("Nested", json!({"value": 4})),
        ("NestedReference", json!({"value": 5})),
        (
            "Host",
            json!({
                "root": 3,
                "nested": {"value": 6},
                "referenced": {"value": 12},
                "items": [8],
                "mapped": {"entry": 10}
            }),
        ),
    ] {
        let input = parse(format!("!self.{tag}")).unwrap();
        let schemas = bundle.compile(&input, None).unwrap();
        assert_eq!(
            schemas.resolve(&input).unwrap().to_json(),
            expected,
            "{tag}"
        );
    }
    let supplied = parse("!self.NestedUnset { child: { value: 13 } }").unwrap();
    let schemas = bundle.compile(&supplied, None).unwrap();
    assert_eq!(
        schemas.resolve(&supplied).unwrap().to_json(),
        json!({"child": {"value": 13}})
    );
}
