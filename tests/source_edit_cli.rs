//! Acceptance checks through the installed CLI consumer, independent of the source-edit Rust API.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "daml-source-edit-cli-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn bundle(&self) -> Vec<PathBuf> {
        vec![
            self.file("daml.bundle", "{}"),
            self.file(
                "Config.daml",
                "closed !Object {\n endpoint: !String\n timeout: !Int\n} = { timeout: 30 }",
            ),
            self.file(
                "Other.daml",
                "closed !Object {\n name: !String\n enabled: !Bool\n} = { enabled: true }",
            ),
        ]
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn run(args: &[&str], input: Option<&[u8]>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_daml"));
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        use std::io::Write;
        if let Err(error) = child.stdin.take().unwrap().write_all(input) {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        }
    }
    child.wait_with_output().unwrap()
}

fn status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn failure(output: &Output, expected: i32) {
    status(output, expected);
    assert!(
        output.stdout.is_empty(),
        "failed edits must not emit partial source"
    );
}

fn replace(path: &str, value: &str) -> String {
    format!("[{{ op: \"replace\"\n path: {path:?}\n value: {value} }}]")
}

fn module(temp: &TempDir) -> PathBuf {
    temp.file(
        "schema.json",
        r#"{"types":{"app.Config":{"schema":{"type":"object","properties":{"endpoint":{"type":"string"},"timeout":{"type":"integer","default":30}},"required":["endpoint","timeout"],"additionalProperties":false}}}}"#,
    )
}

#[test]
fn edit_returns_authored_source_not_completed_data_and_never_writes_any_input() {
    let temp = TempDir::new();
    let mut inputs = temp.bundle();
    let source = "# keep header\r\n!self.Config {\r\n endpoint: \"old\" # endpoint comment\r\n timeout: 3.0e1\r\n}";
    let config = temp.file("input.daml", source);
    let edits = temp.file("edits.daml", &replace("/endpoint", "\"new\""));
    inputs.extend([config.clone(), edits.clone()]);
    let before: Vec<_> = inputs.iter().map(|path| fs::read(path).unwrap()).collect();
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--bundle",
            text(&temp.0),
            text(&config),
        ],
        None,
    );
    status(&output, 0);
    assert_eq!(
        output.stdout,
        source.replacen("\"old\"", "\"new\"", 1).as_bytes()
    );
    for (path, bytes) in inputs.iter().zip(before) {
        assert_eq!(
            fs::read(path).unwrap(),
            bytes,
            "modified {}",
            path.display()
        );
    }

    // Discovery must use the original configuration path, not the edits file.
    let elsewhere = TempDir::new();
    let remote_edits = elsewhere.file("edits.daml", &replace("/endpoint", "\"discovered\""));
    let discovered = run(
        &["edit", "--edits", text(&remote_edits), text(&config)],
        None,
    );
    status(&discovered, 0);
    assert_eq!(
        discovered.stdout,
        source.replacen("\"old\"", "\"discovered\"", 1).as_bytes()
    );

    let omitted = temp.file("omitted.daml", "!self.Config { endpoint: \"old\" }");
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--bundle",
            text(&temp.0),
            text(&omitted),
        ],
        None,
    );
    status(&output, 0);
    assert_eq!(output.stdout, b"!self.Config { endpoint: \"new\" }");
}

#[test]
fn edit_can_repair_schema_invalid_configuration_before_validation() {
    let temp = TempDir::new();
    temp.bundle();
    let config = temp.file("input.daml", "!self.Config { endpoint: 7 }");
    let edits = temp.file("edits.daml", &replace("/endpoint", "\"repaired\""));
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--bundle",
            text(&temp.0),
            text(&config),
        ],
        None,
    );
    status(&output, 0);
    assert_eq!(output.stdout, b"!self.Config { endpoint: \"repaired\" }");
    assert_eq!(
        fs::read_to_string(config).unwrap(),
        "!self.Config { endpoint: 7 }"
    );
}

#[test]
fn module_expected_and_stdin_validate_final_root_without_materializing_defaults() {
    let temp = TempDir::new();
    let schema = module(&temp);
    let edits = temp.file("edits.daml", &replace("/endpoint", "\"new\""));
    let before = fs::read(&schema).unwrap();
    let args = [
        "edit",
        "--edits",
        text(&edits),
        "--module",
        text(&schema),
        "--expected",
        "app.Config",
        "-",
    ];
    let output = run(&args, Some(b"{ endpoint: 7 }"));
    status(&output, 0);
    assert_eq!(output.stdout, b"{ endpoint: \"new\" }");
    assert_eq!(fs::read(&schema).unwrap(), before);

    let untyped = temp.file("untyped.daml", "{ endpoint: 7 }");
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--module",
            text(&schema),
            text(&untyped),
        ],
        None,
    );
    failure(&output, 1);
    failure(
        &run(
            &["edit", "--edits", text(&edits), "-"],
            Some(b"{ endpoint: 7 }"),
        ),
        1,
    );
    failure(&run(&args, Some(b"!app.Other { endpoint: 7 }")), 1);
}

#[test]
fn bundle_compilation_sees_new_assertions_and_checks_the_edited_expected_contract() {
    let temp = TempDir::new();
    temp.bundle();
    let config = temp.file("input.daml", "!self.Config { endpoint: \"old\" }");
    let edits = temp.file(
        "edits.daml",
        &replace("", "!self.Other { name: \"new root\" }"),
    );
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--bundle",
            text(&temp.0),
            text(&config),
        ],
        None,
    );
    status(&output, 0);
    let document = daml::parse(String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(document.tag(document.root()), Some("self.Other"));
    assert_eq!(
        document.string(document.get(document.root(), "name").unwrap()),
        Some("new root")
    );
    assert!(document.get(document.root(), "enabled").is_none());
    failure(
        &run(
            &[
                "edit",
                "--edits",
                text(&edits),
                "--bundle",
                text(&temp.0),
                "--expected",
                "self.Config",
                text(&config),
            ],
            None,
        ),
        1,
    );

    // Invalid supplied data is admission failure; an unavailable native
    // definition requires a filesystem read and preserves the CLI I/O status.
    for (value, expected_status) in [("!self.Other { name: 7 }", 1), ("!self.Missing {}", 2)] {
        let invalid = temp.file("invalid-edits.daml", &replace("", value));
        failure(
            &run(
                &[
                    "edit",
                    "--edits",
                    text(&invalid),
                    "--bundle",
                    text(&temp.0),
                    text(&config),
                ],
                None,
            ),
            expected_status,
        );
    }
}

#[test]
fn supplied_exact_temporal_cron_and_tagged_values_retain_kinds_without_defaults() {
    let temp = TempDir::new();
    let schema = temp.file("schema.json", r#"{"types":{"app.Any":{"schema":{}},"app.Child":{"schema":{"type":"object","properties":{"omitted":{"default":true}}}}}}"#);
    let config = temp.file("input.daml", "!app.Any { value: null }");
    for value in [
        "9007199254740993123456789.123456789",
        "!Date \"2024-01-02\"",
        "!Time \"09:30:00.123456789123\"",
        "!Cron \"0 0 9 * * *\"",
        "!app.Child {}",
    ] {
        let edits = temp.file("edits.daml", &replace("/value", value));
        let output = run(
            &[
                "edit",
                "--edits",
                text(&edits),
                "--module",
                text(&schema),
                text(&config),
            ],
            None,
        );
        status(&output, 0);
        let document = daml::parse(String::from_utf8(output.stdout).unwrap()).unwrap();
        let supplied = daml::SemanticNode::from_document(&daml::parse(value).unwrap());
        let actual = daml::SemanticNode::from_document(&document);
        let daml::SemanticValue::Mapping(fields) = actual.value else {
            panic!("edited root is a map")
        };
        assert_eq!(
            fields["value"], supplied,
            "changed value kind or materialized defaults for {value}"
        );
    }
}

#[test]
fn daml_batch_shape_is_strict_and_failure_has_no_stdout() {
    let temp = TempDir::new();
    let schema = module(&temp);
    let config = temp.file("input.daml", "!app.Config { endpoint: \"old\" }");
    let invalid_batches = [
        "{}",
        "!app.Batch []",
        "[true]",
        "[!app.Operation { op: \"remove\"\n path: \"/endpoint\" }]",
        "[{ op: \"replace\"\n path: \"/endpoint\" }]",
        "[{ op: \"insert\"\n value: 1 }]",
        "[{ path: \"/endpoint\"\n value: 1 }]",
        "[{ op: 7\n path: \"/endpoint\" }]",
        "[{ op: \"remove\"\n path: 7 }]",
        "[{ op: !app.Text \"remove\"\n path: \"/endpoint\" }]",
        "[{ op: \"remove\"\n path: !app.Text \"/endpoint\" }]",
        "[{ op: \"copy\"\n path: \"/endpoint\" }]",
        "[{ op: \"remove\"\n path: \"/endpoint\"\n value: null }]",
        "[{ op: \"remove\"\n path: \"/endpoint\"\n extra: true }]",
        "[{ op: \"remove\"\n op: \"remove\"\n path: \"/endpoint\" }]",
        "[{ op: \"remove\"\n path: \"/endpoint\"\n path: \"/endpoint\" }]",
        "[",
    ];
    for batch in invalid_batches {
        let edits = temp.file("edits.daml", batch);
        let output = run(
            &[
                "edit",
                "--edits",
                text(&edits),
                "--module",
                text(&schema),
                text(&config),
            ],
            None,
        );
        failure(&output, 1);
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            "!app.Config { endpoint: \"old\" }"
        );
        assert_eq!(fs::read_to_string(edits).unwrap(), batch);
    }
}

#[test]
fn batches_are_atomic_for_paths_conflicts_final_schema_and_parse_failures() {
    let temp = TempDir::new();
    let schema = module(&temp);
    let original = "!app.Config { endpoint: \"old\" }";
    let config = temp.file("input.daml", original);
    let first = "{ op: \"replace\"\n path: \"/endpoint\"\n value: \"new\" }";
    for last in [
        "{ op: \"remove\"\n path: \"/missing\" }",
        "{ op: \"replace\"\n path: \"/missing/~2\"\n value: 1 }",
        "{ op: \"replace\"\n path: \"/endpoint\"\n value: \"again\" }",
        "{ op: \"insert\"\n path: \"/timeout\"\n value: \"not an integer\" }",
    ] {
        let edits = temp.file("edits.daml", &format!("[{first}\n{last}]"));
        failure(
            &run(
                &[
                    "edit",
                    "--edits",
                    text(&edits),
                    "--module",
                    text(&schema),
                    text(&config),
                ],
                None,
            ),
            1,
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
    }
    let invalid_config = temp.file("broken.daml", "{");
    let empty = temp.file("empty.daml", "[]");
    failure(
        &run(
            &[
                "edit",
                "--edits",
                text(&empty),
                "--module",
                text(&schema),
                text(&invalid_config),
            ],
            None,
        ),
        1,
    );
    let unchanged = run(
        &[
            "edit",
            "--edits",
            text(&empty),
            "--module",
            text(&schema),
            text(&config),
        ],
        None,
    );
    status(&unchanged, 0);
    assert_eq!(unchanged.stdout, original.as_bytes());
    let invalid = temp.file("invalid.daml", "!app.Config { endpoint: 7 }");
    failure(
        &run(
            &[
                "edit",
                "--edits",
                text(&empty),
                "--module",
                text(&schema),
                text(&invalid),
            ],
            None,
        ),
        1,
    );
}

#[test]
fn edit_flag_admission_and_io_are_exit_two_not_edit_failures() {
    let temp = TempDir::new();
    let edits = temp.file("edits.daml", "[]");
    let config = temp.file("input.daml", "{}");
    let missing = temp.0.join("missing.daml");
    let cases = vec![
        vec!["edit", text(&config)],
        vec!["edit", "--edits"],
        vec!["edit", "--edits", "-", text(&config)],
        vec![
            "edit",
            "--edits",
            text(&edits),
            "--edits",
            text(&edits),
            text(&config),
        ],
        vec![
            "edit",
            "--edits",
            text(&edits),
            "--format",
            "json",
            text(&config),
        ],
        vec!["edit", "--edits", text(&edits), "--path", "", text(&config)],
        vec!["edit", "--edits", text(&edits), "--in-place", text(&config)],
        vec!["check", "--edits", text(&edits), text(&config)],
        vec!["resolve", "--edits", text(&edits), text(&config)],
        vec!["explain", "--edits", text(&edits), text(&config)],
        vec!["edit", "--edits", text(&missing), text(&config)],
        vec!["edit", "--edits", text(&edits), text(&missing)],
    ];
    for args in cases {
        failure(&run(&args, None), 2);
    }
}

#[test]
fn successful_daml_batch_removes_and_inserts_using_original_array_indices() {
    let temp = TempDir::new();
    let schema = temp.file("schema.json", r#"{"types":{"app.Any":{"schema":{}}}}"#);
    let config = temp.file("input.daml", "!app.Any {\n items:[{name:\"zero\"}\n{name:\"one\"}\n{name:\"two\"}]\n obsolete:true # retain\n}");
    let edits = temp.file(
        "edits.daml",
        r#"[
{ op:"remove"
  path:"/items/0" }
{ op:"replace"
  path:"/items/1/name"
  value:"changed" }
{ op:"insert"
  path:"/items/3"
  value:{name:"appended"} }
{ op:"remove"
  path:"/obsolete" }
{ op:"insert"
  path:"/new.field"
  value:9007199254740993123456789 }
]"#,
    );
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--module",
            text(&schema),
            text(&config),
        ],
        None,
    );
    status(&output, 0);
    let source = String::from_utf8(output.stdout).unwrap();
    assert!(source.contains("# retain\n"));
    let actual = daml::SemanticNode::from_document(&daml::parse(source).unwrap());
    let expected = daml::SemanticNode::from_document(&daml::parse(
        "!app.Any {items:[{name:\"changed\"}\n{name:\"two\"}\n{name:\"appended\"}]\nnew.field:9007199254740993123456789}",
    ).unwrap());
    assert_eq!(actual, expected);
}

#[test]
fn repeated_module_options_make_edit_value_assertions_available() {
    let temp = TempDir::new();
    let root = temp.file(
        "root.json",
        r#"{"types":{"app.Root":{"schema":{"type":"object","properties":{"child":{}}}}}}"#,
    );
    let child = temp.file("child.json", r#"{"types":{"app.Child":{"schema":{"type":"object","properties":{"name":{"type":"string"},"omitted":{"default":true}},"required":["name"]}}}}"#);
    let config = temp.file("input.daml", "{child:null}");
    let edits = temp.file(
        "edits.daml",
        &replace("/child", "!app.Child {name:\"new\"}"),
    );
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--module",
            text(&root),
            "--module",
            text(&child),
            "--expected",
            "app.Root",
            text(&config),
        ],
        None,
    );
    status(&output, 0);
    let actual = daml::SemanticNode::from_document(
        &daml::parse(String::from_utf8(output.stdout).unwrap()).unwrap(),
    );
    let expected = daml::SemanticNode::from_document(
        &daml::parse("{child:!app.Child {name:\"new\"}}").unwrap(),
    );
    assert_eq!(actual, expected);
    assert_eq!(
        fs::read_to_string(&child).unwrap(),
        r#"{"types":{"app.Child":{"schema":{"type":"object","properties":{"name":{"type":"string"},"omitted":{"default":true}},"required":["name"]}}}}"#
    );
}

#[test]
fn oversized_edits_source_is_an_atomic_data_failure_not_an_io_failure() {
    let temp = TempDir::new();
    let schema = module(&temp);
    let config = temp.file("input.daml", "!app.Config {endpoint:\"old\"}");
    // The edit-source cap is a CLI contract, not a configurable core default.
    let source = format!("#{}\n[]", "x".repeat(8 * 1024 * 1024));
    let edits = temp.file("edits.daml", &source);
    let output = run(
        &[
            "edit",
            "--edits",
            text(&edits),
            "--module",
            text(&schema),
            text(&config),
        ],
        None,
    );
    failure(&output, 1);
    assert_eq!(
        fs::read_to_string(config).unwrap(),
        "!app.Config {endpoint:\"old\"}"
    );
}
