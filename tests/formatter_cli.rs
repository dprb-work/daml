//! Actual formatter command behavior, including check status and input preservation.

mod support;

use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "daml-fmt-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(support::binary("daml-fmt", env!("CARGO_BIN_EXE_daml-fmt")))
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    if let Err(error) = child.stdin.take().unwrap().write_all(input) {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
    child.wait_with_output().unwrap()
}

#[test]
fn stdin_width_changes_layout_and_check_is_idempotent() {
    let source = b"{\n  alpha: 1\n  beta: 2\n}\n";
    let narrow = cli(&["--width", "8"], source);
    assert!(
        narrow.status.success(),
        "{}",
        String::from_utf8_lossy(&narrow.stderr)
    );
    let wide = cli(&["--width", "80"], source);
    assert!(wide.status.success());
    assert_ne!(
        narrow.stdout, wide.stdout,
        "width must affect the rendered layout"
    );
    let checked = cli(&["--check", "--width", "8"], &narrow.stdout);
    assert_eq!(checked.status.code(), Some(0));
    assert!(checked.stdout.is_empty());
    let changed = cli(&["--check", "--width", "8"], source);
    assert_eq!(changed.status.code(), Some(1));
    assert!(changed.stdout.is_empty());
}

#[test]
fn file_input_is_formatted_without_changing_source() {
    let dir = TempDir::new();
    let path = dir.0.join("input.daml");
    let source = b"{alpha: 1}\n";
    fs::write(&path, source).unwrap();
    let output = Command::new(support::binary("daml-fmt", env!("CARGO_BIN_EXE_daml-fmt")))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(output.stdout, b"{ alpha: 1 }\n");
}

#[test]
fn invalid_source_and_cli_errors_exit_two() {
    let invalid = cli(&[], b"{broken:");
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    for args in [
        &["--width", "0"][..],
        &["--width"][..],
        &["--unknown"][..],
        &["--check", "--check"][..],
        &["one", "two"][..],
    ] {
        let output = cli(args, b"{}");
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty());
    }
}
