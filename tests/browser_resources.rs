//! In-memory native source admission preserves the compiler's bundle semantics.

use daml::{Bundle, NativeErrorKind, ResolveErrorKind, parse};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("daml-browser-resources-{stamp}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sources(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(path, source)| ((*path).to_owned(), (*source).to_owned()))
        .collect()
}

#[test]
fn in_memory_bundle_matches_filesystem_defaults_and_tag_metadata() {
    let manifest = "{}";
    let definition = "!Object { child: (!Object = !self.Provider) }";
    let provider = "!Object { value: !Int } = { value: 3 }";
    let tree = TempTree::new();
    fs::write(tree.0.join("daml.bundle"), manifest).unwrap();
    fs::write(tree.0.join("Config.daml"), definition).unwrap();
    fs::write(tree.0.join("Provider.daml"), provider).unwrap();
    let disk = Bundle::open(&tree.0).unwrap();
    let memory = Bundle::from_sources(
        ".",
        sources(&[
            ("daml.bundle", manifest),
            ("Config.daml", definition),
            ("Provider.daml", provider),
        ]),
    )
    .unwrap();
    let input = parse("!self.Config").unwrap();
    let disk_schemas = disk.compile(&input, None).unwrap();
    let memory_schemas = memory.compile(&input, None).unwrap();
    let disk_value = disk_schemas.resolve(&input).unwrap();
    let memory_value = memory_schemas.resolve(&input).unwrap();
    assert_eq!(memory_value.to_json(), json!({"child":{"value":3}}));
    assert_eq!(memory_value.to_json(), disk_value.to_json());
    assert_eq!(
        memory_value.root().get("child").unwrap().tag(),
        Some("self.Provider")
    );
}

#[test]
fn in_memory_manifest_normalizes_current_and_dot_prefixed_sibling_locators() {
    let manifest = r#"{
  current: "."
  sibling: "./dep/"
}"#;
    let current_definition = "!Object { value: !String } = { value: \"root\" }";
    let sibling_definition = "!Object { value: !String } = { value: \"sibling\" }";
    let entries = [
        ("daml.bundle", manifest),
        ("Config.daml", current_definition),
        ("dep/daml.bundle", "{}"),
        ("dep/Config.daml", sibling_definition),
    ];
    let tree = TempTree::new();
    fs::write(tree.0.join("daml.bundle"), manifest).unwrap();
    fs::write(tree.0.join("Config.daml"), current_definition).unwrap();
    fs::create_dir_all(tree.0.join("dep")).unwrap();
    fs::write(tree.0.join("dep/daml.bundle"), "{}").unwrap();
    fs::write(tree.0.join("dep/Config.daml"), sibling_definition).unwrap();

    let disk = Bundle::open(&tree.0).unwrap();
    let memory = Bundle::from_sources(".", sources(&entries)).unwrap();
    for (tag, expected) in [
        ("current.Config", json!({"value":"root"})),
        ("sibling.Config", json!({"value":"sibling"})),
    ] {
        let input = parse(format!("!{tag}")).unwrap();
        let disk_schemas = disk.compile(&input, None).unwrap();
        let memory_schemas = memory.compile(&input, None).unwrap();
        let disk_value = disk_schemas.resolve(&input).unwrap();
        let memory_value = memory_schemas.resolve(&input).unwrap();
        assert_eq!(memory_value.to_json(), expected, "{tag}");
        assert_eq!(memory_value.to_json(), disk_value.to_json(), "{tag}");
    }
}

#[test]
fn explicit_expected_definition_is_used_for_compilation() {
    let bundle = Bundle::from_sources(
        ".",
        sources(&[
            ("daml.bundle", "{}"),
            ("A.daml", "!String"),
            ("B.daml", "!Int"),
        ]),
    )
    .unwrap();
    let input = parse("\"5\"").unwrap();
    let schemas = bundle.compile(&input, Some("self.B")).unwrap();
    assert_eq!(
        schemas.resolve_as(&input, "self.B").unwrap_err().kind,
        ResolveErrorKind::SchemaViolation
    );
}

#[test]
fn missing_references_and_definition_cycles_stay_inside_the_admitted_table() {
    let missing = Bundle::from_sources(
        ".",
        sources(&[("daml.bundle", "{}"), ("A.daml", "!self.Absent")]),
    )
    .unwrap();
    let input = parse("null").unwrap();
    assert_eq!(
        missing
            .compile(&input, Some("self.A"))
            .err()
            .expect("missing definition must fail")
            .kind,
        NativeErrorKind::MissingDefinition
    );

    let cyclic = Bundle::from_sources(
        ".",
        sources(&[
            ("daml.bundle", "{}"),
            ("A.daml", "!self.B"),
            ("B.daml", "!self.A"),
        ]),
    )
    .unwrap();
    assert_eq!(
        cyclic
            .compile(&input, Some("self.A"))
            .err()
            .expect("definition cycle must fail")
            .kind,
        NativeErrorKind::ReferenceCycle
    );
}

#[test]
fn unused_invalid_definition_is_lazy_but_reachable_invalid_definition_is_rejected() {
    let bundle = Bundle::from_sources(
        ".",
        sources(&[
            ("daml.bundle", "{}"),
            ("Good.daml", "!String"),
            ("Unused.daml", "!Object { malformed definition body"),
        ]),
    )
    .unwrap();
    let good = parse("\"ok\"").unwrap();
    assert!(bundle.compile(&good, Some("self.Good")).is_ok());
    let unused = parse("null").unwrap();
    assert!(bundle.compile(&unused, Some("self.Unused")).is_err());
}

#[test]
fn source_table_rejects_escape_paths_and_does_not_discover_host_files() {
    for path in [
        "../outside.daml",
        "nested/../../escape.daml",
        "a\\b.daml",
        "C:/outside.daml",
    ] {
        let error = Bundle::from_sources(".", sources(&[("daml.bundle", "{}"), (path, "!String")]))
            .err()
            .unwrap();
        assert!(
            matches!(
                error.kind,
                NativeErrorKind::PathEscape | NativeErrorKind::InvalidManifest
            ),
            "{path}: {:?}",
            error.kind
        );
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tree = TempTree(
        std::env::current_dir()
            .unwrap()
            .join(format!("daml-memory-{stamp}")),
    );
    fs::create_dir_all(&tree.0).unwrap();
    fs::write(tree.0.join("daml.bundle"), "{}").unwrap();
    fs::write(tree.0.join("NotOnDisk.daml"), "!String").unwrap();
    let root = tree.0.file_name().unwrap().to_str().unwrap().to_owned();
    let manifest = format!("{root}/daml.bundle");
    let bundle = Bundle::from_sources(&root, sources(&[(&manifest, "{}")])).unwrap();
    let input = parse("null").unwrap();
    assert_eq!(
        bundle
            .compile(&input, Some("self.NotOnDisk"))
            .err()
            .expect("host-only definition must not be discovered")
            .kind,
        NativeErrorKind::MissingDefinition
    );
}

#[test]
fn nested_bundle_boundaries_and_source_table_byte_limits_are_enforced() {
    let nested = Bundle::from_sources(
        ".",
        sources(&[
            ("daml.bundle", "{}"),
            ("outer/Entry.daml", "!self.Config"),
            ("outer/Config.daml", "!String"),
            ("outer/daml.bundle", "{}"),
        ]),
    )
    .unwrap();
    let input = parse("null").unwrap();
    assert_eq!(
        nested
            .compile(&input, Some("self.outer.Entry"))
            .err()
            .expect("nested bundle boundary must be refused")
            .kind,
        NativeErrorKind::NestedBundleBoundary
    );

    let oversized = "x".repeat(8 * 1024 * 1024 + 1);
    let error = Bundle::from_sources(
        ".",
        sources(&[("daml.bundle", "{}"), ("Unused.daml", &oversized)]),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind, NativeErrorKind::LimitExceeded);
}
#[test]
fn source_table_count_and_logical_path_bounds_have_exact_edges() {
    fn table_with_files(file_count: usize) -> Vec<(String, String)> {
        let mut entries = Vec::with_capacity(file_count + 1);
        entries.push(("daml.bundle".to_owned(), "{}".to_owned()));
        entries
            .extend((0..file_count).map(|index| (format!("unused-{index}.daml"), String::new())));
        entries
    }

    assert!(
        Bundle::from_sources(".", table_with_files(9_999)).is_ok(),
        "10,000 total table entries, including the manifest, are admitted"
    );
    let count_error = Bundle::from_sources(".", table_with_files(10_000))
        .err()
        .unwrap();
    assert_eq!(count_error.kind, NativeErrorKind::LimitExceeded);

    let at_limit = format!("{}.daml", "a".repeat(4_091));
    let over_limit = format!("{}.daml", "a".repeat(4_092));
    assert_eq!(at_limit.len(), 4_096);
    assert_eq!(over_limit.len(), 4_097);
    assert!(
        Bundle::from_sources(
            ".",
            vec![
                ("daml.bundle".to_owned(), "{}".to_owned()),
                (at_limit, String::new()),
            ],
        )
        .is_ok()
    );
    let path_error = Bundle::from_sources(
        ".",
        vec![
            ("daml.bundle".to_owned(), "{}".to_owned()),
            (over_limit, String::new()),
        ],
    )
    .err()
    .unwrap();
    assert_eq!(path_error.kind, NativeErrorKind::LimitExceeded);
}
