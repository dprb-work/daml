//! Frozen native bundle cases exercise real source loading and completed data.

use daml::{
    Bundle, FormatOptions, NativeErrorKind, Origin, ResolveErrorKind, format_definition, parse,
    parse_definition,
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

fn assert_corpus(root: &Path) {
    let cases: Value = serde_json::from_str(include_str!("../examples/native/cases.json")).unwrap();
    let bundle = Bundle::open(root).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["input"].as_str().unwrap();
        let document = parse(fs::read_to_string(root.join("data").join(name)).unwrap()).unwrap();
        let compiled = bundle.compile(&document, None);
        if let Some(kind) = case.get("native_error").and_then(Value::as_str) {
            let expected = match kind {
                "UnknownReference" => NativeErrorKind::UnknownReference,
                "NestedBundleBoundary" => NativeErrorKind::NestedBundleBoundary,
                "InvalidDefault" => NativeErrorKind::InvalidDefault,
                _ => panic!("unknown native corpus error category {kind}"),
            };
            let error = compiled.err().expect(name);
            assert_eq!(error.kind, expected, "{name}: {error}");
            continue;
        }
        let schemas = compiled.unwrap_or_else(|error| panic!("{name}: {error}"));
        let result = schemas.resolve(&document);
        if case.get("resolve_error").is_some() {
            let error = result.expect_err(name);
            assert_eq!(
                error.kind,
                ResolveErrorKind::SchemaViolation,
                "{name}: {error}"
            );
            assert_eq!(
                error.path,
                case["path"].as_str().unwrap(),
                "{name}: {error}"
            );
            continue;
        }
        let resolved = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(resolved.to_json(), case["expected"], "{name}");
        let adapter = resolved.root().get("adapter").unwrap();
        assert_eq!(adapter.tag(), case["adapter_tag"].as_str(), "{name}");
        assert!(
            matches!(adapter.get("timeout").unwrap().origin(), Origin::Default(_)),
            "{name}"
        );
        let supplied_address = matches!(adapter.get("addr").unwrap().origin(), Origin::Input(_));
        assert_eq!(
            supplied_address,
            case["address_origin"] == "input",
            "{name}"
        );
    }
}

#[test]
fn native_bundle_acceptance_corpus() {
    assert_corpus(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/native"));
}

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "daml-native-format-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_reflowed(source: &Path, target: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir(&destination).unwrap();
            copy_reflowed(&path, &destination);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "daml")
            || entry.file_name() == "daml.bundle"
        {
            let definition = parse_definition(fs::read_to_string(&path).unwrap()).unwrap();
            let options = FormatOptions { line_width: 28 };
            let output = format_definition(&definition, options).unwrap();
            let reparsed = parse_definition(&output).unwrap();
            assert_eq!(format_definition(&reparsed, options).unwrap(), output);
            fs::write(destination, output).unwrap();
        } else {
            fs::copy(path, destination).unwrap();
        }
    }
}

#[test]
fn reflowed_bundle_and_data_preserve_completed_values_assertions_and_errors() {
    let root = TempDir::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/native");
    copy_reflowed(&source, &root.0);
    assert_corpus(&root.0);
}

#[test]
fn reflow_keeps_parentheses_that_change_constraint_acceptance() {
    let root = TempDir::new();
    fs::write(root.0.join("daml.bundle"), "{}").unwrap();
    let path = root.0.join("Grouped.daml");
    let source = "!Object { value: (!Int | !String) & !Never }";
    let formatted = format_definition(
        &parse_definition(source).unwrap(),
        FormatOptions { line_width: 18 },
    )
    .unwrap();
    let document = parse("!self.Grouped { value: 3 }").unwrap();
    for text in [source, formatted.as_str()] {
        fs::write(&path, text).unwrap();
        let schemas = Bundle::open(&root.0)
            .unwrap()
            .compile(&document, None)
            .unwrap();
        assert_eq!(
            schemas.resolve(&document).err().unwrap().kind,
            ResolveErrorKind::SchemaViolation
        );
    }
}

#[test]
fn oversized_manifest_fails_at_source_admission() {
    let root = TempDir::new();
    let manifest = root.0.join("daml.bundle");
    let file = fs::File::create(&manifest).unwrap();
    file.set_len(daml::Limits::default().max_input_bytes as u64 + 1)
        .unwrap();
    let error = Bundle::open(&root.0).unwrap_err();
    assert_eq!(error.kind, NativeErrorKind::LimitExceeded);
    assert_eq!(error.path.as_deref(), Some(manifest.as_path()));
}

#[cfg(target_os = "linux")]
#[test]
fn distinct_non_utf8_bundle_roots_do_not_alias_after_lossy_display() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };

    let root = TempDir::new();
    let first = root.0.join(OsString::from_vec(b"bundle-\xff".to_vec()));
    let second = root.0.join("bundle-�");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    for directory in [&first, &second] {
        fs::write(directory.join("daml.bundle"), "{}").unwrap();
    }
    fs::write(first.join("Item.daml"), "!Int").unwrap();
    fs::write(second.join("Item.daml"), "!String").unwrap();
    symlink(&first, root.0.join("first")).unwrap();
    symlink(&second, root.0.join("second")).unwrap();
    fs::write(
        root.0.join("daml.bundle"),
        "{first: \"./first\"\nsecond: \"./second\"}",
    )
    .unwrap();
    let document = parse("{first: !first.Item 3\nsecond: !second.Item \"value\"}").unwrap();
    let schemas = Bundle::open(&root.0)
        .unwrap()
        .compile(&document, None)
        .unwrap();
    assert_eq!(
        schemas.resolve(&document).unwrap().to_json(),
        serde_json::json!({"first": 3, "second": "value"})
    );
}

#[test]
fn grouped_bare_tagged_default_keeps_empty_payload_and_completion_after_reflow() {
    let root = TempDir::new();
    fs::write(root.0.join("daml.bundle"), "{}").unwrap();
    fs::write(
        root.0.join("Provider.daml"),
        "!Object { value: !Int } = { value: 3 }",
    )
    .unwrap();
    let source = "!Object { child: (!Object = !self.Provider) }";
    let formatted = format_definition(
        &parse_definition(source).unwrap(),
        FormatOptions { line_width: 20 },
    )
    .unwrap();
    let input = parse("!self.Holder").unwrap();
    for text in [source, formatted.as_str()] {
        fs::write(root.0.join("Holder.daml"), text).unwrap();
        let schemas = Bundle::open(&root.0)
            .unwrap()
            .compile(&input, None)
            .unwrap();
        let resolved = schemas.resolve(&input).unwrap();
        let child = resolved.root().get("child").unwrap();
        assert_eq!(child.to_json(), serde_json::json!({"value": 3}));
        assert_eq!(child.tag(), Some("self.Provider"));
    }
}

#[test]
fn grouped_multiline_default_reflow_preserves_its_value_and_delimiter_framing() {
    let root = TempDir::new();
    fs::write(root.0.join("daml.bundle"), "{}").unwrap();
    let source = "(!String = \"\"\"\n    first\n      second\n \"\"\"\n)";
    let formatted = format_definition(
        &parse_definition(source).unwrap(),
        FormatOptions { line_width: 20 },
    )
    .unwrap();
    let input = parse("!self.Holder").unwrap();
    for text in [source, formatted.as_str()] {
        fs::write(root.0.join("Text.daml"), text).unwrap();
        fs::write(root.0.join("Holder.daml"), "!Object { text: !self.Text }").unwrap();
        let schemas = Bundle::open(&root.0)
            .unwrap()
            .compile(&input, None)
            .unwrap();
        assert_eq!(
            schemas.resolve(&input).unwrap().to_json(),
            serde_json::json!({"text": "first\n  second"})
        );
    }
}
