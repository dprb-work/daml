//! Consumer-visible behavior of the `daml` configuration CLI.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/configuration-cli"
);
const TUTORIAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/tutorial");

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "daml-cli-{}-{}",
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
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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



#[test]
fn version_reports_embedded_package_version_without_reading_stdin() {
    let output = run(&["--version"], None);
    status(&output, 0);
    assert_eq!(
        output.stdout,
        format!("daml {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(output.stderr.is_empty());
}

fn status(out: &Output, expected: i32) {
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn report(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn fixture(name: &str) -> String {
    Path::new(FIXTURES).join(name).to_str().unwrap().to_owned()
}

#[test]
fn tutorial_provider_resolves_actual_defaults_and_explains_provenance_and_constraints() {
    let input = format!("{TUTORIAL}/data/provider.daml");
    let out = run(&["resolve", "--bundle", TUTORIAL, &input], None);
    status(&out, 0);
    let resolved = String::from_utf8(out.stdout).unwrap();
    assert!(resolved.contains("timeout: 30"));
    assert!(resolved.contains("retries: 3"));

    let endpoint = report(&run(
        &[
            "explain",
            "--bundle",
            TUTORIAL,
            "--path",
            "/adapter/endpoint",
            &input,
        ],
        None,
    ));
    assert_eq!(endpoint["path"], "/adapter/endpoint");
    assert_eq!(endpoint["origin"]["kind"], "input");
    assert!(endpoint["constraints"].as_array().unwrap().iter().any(|c| {
        c["schemaLocation"]["pointer"]
            .as_str()
            .is_some_and(|pointer| pointer.ends_with("/properties/endpoint"))
    }));

    let timeout = report(&run(
        &[
            "explain",
            "--bundle",
            TUTORIAL,
            "--path",
            "/adapter/timeout",
            &input,
        ],
        None,
    ));
    assert_eq!(timeout["origin"]["kind"], "default");
    assert!(
        timeout["origin"]["pointer"]
            .as_str()
            .is_some_and(|pointer| pointer.ends_with("/default"))
    );
    assert!(timeout["constraints"].as_array().unwrap().iter().any(|c| {
        c["schemaLocation"]["pointer"]
            .as_str()
            .is_some_and(|pointer| pointer.ends_with("/properties/timeout"))
    }));
}

#[test]
fn admission_requires_context_and_root_contract_and_stdin_requires_explicit_context() {
    let temp = TempDir::new();
    let file = temp.file("plain.daml", "{ value: 1 }");
    status(&run(&["check", file.to_str().unwrap()], None), 1);

    let root = fixture("recursive-root.json");
    let resource = fixture("recursive-resource.json");
    status(
        &run(
            &["resolve", "--module", &root, "--module", &resource, "-"],
            Some(b"{value:1}"),
        ),
        1,
    );
    status(
        &run(
            &[
                "resolve",
                "--module",
                &root,
                "--module",
                &resource,
                "--expected",
                "app.Node",
                "-",
            ],
            Some(b"{value:1}"),
        ),
        0,
    );
    status(
        &run(
            &[
                "resolve",
                "--module",
                &root,
                "--module",
                &resource,
                "--expected",
                "app.Node",
                "-",
            ],
            Some(b"!app.Other { value: 1 }"),
        ),
        1,
    );
}

#[test]
fn explain_root_pointer_escaped_segments_and_array_items_are_exact() {
    let temp = TempDir::new();
    let input = temp.file("value.daml", "{ a/b: [{value: 7}] }");
    let module = fixture("pointer-root.json");
    let root = report(&run(
        &[
            "explain",
            "--module",
            &module,
            "--expected",
            "app.Pointer",
            "--path",
            "",
            input.to_str().unwrap(),
        ],
        None,
    ));
    assert_eq!(root["path"], "");
    let child = report(&run(
        &[
            "explain",
            "--module",
            &module,
            "--expected",
            "app.Pointer",
            "--path",
            "/a~1b/0/value",
            input.to_str().unwrap(),
        ],
        None,
    ));
    assert_eq!(child["path"], "/a~1b/0/value");
    assert_eq!(child["value"]["value"], "7");
    for pointer in ["/missing", "/a~2b", "/a~1b/00/value"] {
        status(
            &run(
                &[
                    "explain",
                    "--module",
                    &module,
                    "--expected",
                    "app.Pointer",
                    "--path",
                    pointer,
                    input.to_str().unwrap(),
                ],
                None,
            ),
            1,
        );
    }
}

#[test]
fn exact_json_projection_and_temporal_cron_values_do_not_round() {
    let temp = TempDir::new();
    let exact = temp.file(
        "exact.daml",
        "{ amount: 9007199254740993123456789.123456789 }",
    );
    let exact_out = run(
        &[
            "resolve",
            "--module",
            &fixture("exact-root.json"),
            "--expected",
            "app.Exact",
            "--format",
            "json",
            exact.to_str().unwrap(),
        ],
        None,
    );
    status(&exact_out, 0);
    assert!(
        String::from_utf8(exact_out.stdout)
            .unwrap()
            .contains("9007199254740993123456789.123456789")
    );

    let special = temp.file(
        "special.daml",
        "{\n  date: !Date \"2024-01-02\"\n  cron: !Cron \"0 0 9 * * *\"\n}",
    );
    let json = run(
        &[
            "resolve",
            "--module",
            &fixture("special-root.json"),
            "--expected",
            "app.Special",
            "--format",
            "json",
            special.to_str().unwrap(),
        ],
        None,
    );
    status(&json, 0);
    let data: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(data["date"], "2024-01-02");
    assert_eq!(data["cron"], "0 0 9 * * *");
    let canonical = run(
        &[
            "resolve",
            "--module",
            &fixture("special-root.json"),
            "--expected",
            "app.Special",
            special.to_str().unwrap(),
        ],
        None,
    );
    status(&canonical, 0);
    let canonical = String::from_utf8(canonical.stdout).unwrap();
    assert!(canonical.contains("!Date"));
    assert!(canonical.contains("!Cron"));
}
#[test]
fn explanation_preserves_validation_only_branches_and_enclosing_constraints() {
    let temp = TempDir::new();
    let input = temp.file("trace.daml", "{ value: 12 }");
    let report = report(&run(
        &[
            "explain",
            "--module",
            &fixture("trace-root.json"),
            "--expected",
            "app.Trace",
            "--path",
            "/value",
            input.to_str().unwrap(),
        ],
        None,
    ));
    let constraints = report["constraints"].as_array().unwrap();
    let locations: Vec<&str> = constraints
        .iter()
        .filter_map(|constraint| constraint["schemaLocation"]["pointer"].as_str())
        .collect();
    assert!(
        locations
            .iter()
            .any(|location| { location.ends_with("/allOf/0/properties/value") })
    );
    assert!(
        locations
            .iter()
            .any(|location| { location.ends_with("/allOf/1/properties/value/anyOf/0") })
    );
    assert!(
        !locations
            .iter()
            .any(|location| location.ends_with("/allOf/1/properties/value/anyOf/1"))
    );
}

#[test]
fn recursive_descendant_explanation_has_same_admission_as_other_operations() {
    let temp = TempDir::new();
    let root = fixture("recursive-root.json");
    let resource = fixture("recursive-resource.json");
    let invalid = temp.file("invalid.daml", "{\n  value: 1\n  next: { value: -1 }\n}");
    for operation in ["check", "resolve", "explain"] {
        let mut args = vec![
            operation,
            "--module",
            &root,
            "--module",
            &resource,
            "--expected",
            "app.Node",
        ];
        if operation == "explain" {
            args.extend(["--path", "/next/value"]);
        }
        args.push(invalid.to_str().unwrap());
        let out = run(&args, None);
        status(&out, 1);
        assert!(out.stdout.is_empty());
    }

    let valid = temp.file(
        "valid-recursive.daml",
        "{\n  value: 1\n  next: { value: 2 }\n}",
    );
    let explained = report(&run(
        &[
            "explain",
            "--module",
            &root,
            "--module",
            &resource,
            "--expected",
            "app.Node",
            "--path",
            "/next/value",
            valid.to_str().unwrap(),
        ],
        None,
    ));
    assert_eq!(explained["path"], "/next/value");
    assert_eq!(explained["origin"]["kind"], "input");
    assert!(
        explained["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|constraint| constraint["schemaLocation"]["resource"] == "urn:recursive:node")
    );
}

#[test]
fn invalid_tags_values_ambiguous_defaults_and_unrepresentable_data_fail_without_stdout() {
    let temp = TempDir::new();
    let root = fixture("recursive-root.json");
    let resource = fixture("recursive-resource.json");
    let invalid = temp.file("invalid.daml", "!app.Node { value: -1 }");
    let out = run(
        &[
            "resolve",
            "--module",
            &root,
            "--module",
            &resource,
            invalid.to_str().unwrap(),
        ],
        None,
    );
    status(&out, 1);
    assert!(out.stdout.is_empty());
    let unknown = temp.file("unknown.daml", "!app.Unknown { value: 1 }");
    let out = run(
        &[
            "check",
            "--module",
            &root,
            "--module",
            &resource,
            unknown.to_str().unwrap(),
        ],
        None,
    );
    status(&out, 1);
    assert!(out.stdout.is_empty());
    let ambiguous = temp.file("ambiguous.daml", "{}");
    let out = run(
        &[
            "resolve",
            "--module",
            &fixture("ambiguous.json"),
            "--expected",
            "app.Ambiguous",
            ambiguous.to_str().unwrap(),
        ],
        None,
    );
    status(&out, 1);
    assert!(out.stdout.is_empty());
}

#[test]
fn finite_recursive_module_validates_descendants_and_rejects_cycles_and_bounds() {
    let temp = TempDir::new();
    let root = fixture("recursive-root.json");
    let resource = fixture("recursive-resource.json");
    let valid = temp.file("valid.daml", "{\n  value: 1\n  next: { value: 2 }\n}");
    status(
        &run(
            &[
                "check",
                "--module",
                &root,
                "--module",
                &resource,
                "--expected",
                "app.Node",
                valid.to_str().unwrap(),
            ],
            None,
        ),
        0,
    );
    let invalid = temp.file(
        "invalid-descendant.daml",
        "{\n  value: 1\n  next: { value: -1 }\n}",
    );
    let out = run(
        &[
            "check",
            "--module",
            &root,
            "--module",
            &resource,
            "--expected",
            "app.Node",
            invalid.to_str().unwrap(),
        ],
        None,
    );
    status(&out, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("/next/value"));

    let empty = temp.file("empty.daml", "{}");
    for name in ["nonprogressing.json", "resource-heavy.json"] {
        let module = fixture(name);
        status(
            &run(
                &[
                    "resolve",
                    "--module",
                    &module,
                    "--expected",
                    "app.Loop",
                    empty.to_str().unwrap(),
                ],
                None,
            ),
            1,
        );
    }
}

#[test]
fn malformed_invocation_and_missing_file_are_usage_or_io_errors() {
    status(&run(&["resolve", "--unknown-option"], None), 2);
    status(&run(&["resolve", "--module"], None), 2);
    status(
        &run(&["check", "/a/path/that/does/not/exist.daml"], None),
        2,
    );
}

#[test]
fn check_is_silent_and_inputs_are_read_only() {
    let temp = TempDir::new();
    let input = temp.file(
        "provider.daml",
        "!providers.Http { endpoint: \"https://example.com\" }",
    );
    let before = fs::read(&input).unwrap();
    let out = run(
        &["check", "--bundle", TUTORIAL, input.to_str().unwrap()],
        None,
    );
    status(&out, 0);
    assert!(out.stdout.is_empty());
    assert_eq!(fs::read(input).unwrap(), before);
}
