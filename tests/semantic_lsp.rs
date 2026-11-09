//! Independent consumer checks through the real stdio server, not private LSP internals.
#![cfg(feature = "lsp")]

mod support;

use lsp_server::{Message, Notification, Request, RequestId};
use serde_json::{Value, json};
use std::{
    fs,
    io::BufReader,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CONFIG: &str = include_str!("fixtures/semantic-editor/workspace/config.daml");
const HTTP: &str = include_str!("fixtures/semantic-editor/providers/Http.daml");
const TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "daml-semantic-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create exclusively owned fixture directory");
        let fixture = Self(root);
        for (path, text) in [
            (
                "workspace/daml.bundle",
                include_str!("fixtures/semantic-editor/workspace/daml.bundle"),
            ),
            (
                "workspace/app/daml.bundle",
                include_str!("fixtures/semantic-editor/workspace/app/daml.bundle"),
            ),
            (
                "workspace/app/Service.daml",
                include_str!("fixtures/semantic-editor/workspace/app/Service.daml"),
            ),
            (
                "workspace/app/Worker.daml",
                include_str!("fixtures/semantic-editor/workspace/app/Worker.daml"),
            ),
            (
                "workspace/app/Choice.daml",
                include_str!("fixtures/semantic-editor/workspace/app/Choice.daml"),
            ),
            (
                "workspace/app/NestedChoice.daml",
                include_str!("fixtures/semantic-editor/workspace/app/NestedChoice.daml"),
            ),
            (
                "workspace/app/ChoiceFieldDetail.daml",
                include_str!("fixtures/semantic-editor/workspace/app/ChoiceFieldDetail.daml"),
            ),
            ("workspace/config.daml", CONFIG),
            (
                "providers/daml.bundle",
                include_str!("fixtures/semantic-editor/providers/daml.bundle"),
            ),
            ("providers/Http.daml", HTTP),
            (
                "providers/Unloaded.daml",
                include_str!("fixtures/semantic-editor/providers/Unloaded.daml"),
            ),
            (
                "contracts/daml.bundle",
                include_str!("fixtures/semantic-editor/contracts/daml.bundle"),
            ),
            (
                "contracts/Adapter.daml",
                include_str!("fixtures/semantic-editor/contracts/Adapter.daml"),
            ),
        ] {
            let target = fixture.0.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, text).unwrap();
        }
        fixture
    }
    fn uri(&self, path: impl AsRef<std::path::Path>) -> String {
        // The fixed fixture components are ASCII; encode the platform temp directory bytes.
        let path = self.0.join(path);
        #[cfg(windows)]
        let normalized = path
            .to_str()
            .expect("Windows fixture directory must have a Unicode path")
            .replace('\\', "/");
        #[cfg(windows)]
        let normalized = normalized.strip_prefix("//?/").unwrap_or(&normalized);
        #[cfg(windows)]
        let bytes = normalized.as_bytes();
        #[cfg(not(windows))]
        let bytes = path.as_os_str().as_encoded_bytes();
        let mut uri = String::from(if cfg!(windows) { "file:///" } else { "file://" });
        for &byte in bytes {
            if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
                uri.push(char::from(byte));
            } else {
                use std::fmt::Write;
                write!(uri, "%{byte:02X}").unwrap();
            }
        }
        // Use the actual protocol URI parser, rather than a native schema identity.
        let _: lsp_types::Uri = uri.parse().unwrap();
        uri
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    incoming: Receiver<Message>,
    reader: Option<JoinHandle<()>>,
    notices: Vec<Notification>,
    next_id: i32,
    last_generation: u32,
    generations: std::collections::BTreeMap<String, u32>,
}
impl Server {
    fn start(encoding: &str) -> Self {
        let mut child = Command::new(support::binary("daml-lsp", env!("CARGO_BIN_EXE_daml-lsp")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start actual daml-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, incoming) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            while let Ok(Some(message)) = Message::read(&mut stdout) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            stdin: Some(stdin),
            incoming,
            reader: Some(reader),
            notices: Vec::new(),
            next_id: 1,
            last_generation: 0,
            generations: std::collections::BTreeMap::new(),
        };
        let result = server.ok(
            "initialize",
            json!({"processId":null,"capabilities":{"general":{"positionEncodings":[encoding]}}}),
        );
        assert_eq!(result["capabilities"]["positionEncoding"], encoding);
        let caps = &result["capabilities"];
        assert!(
            caps["completionProvider"].is_object(),
            "completion capability: {caps}"
        );
        assert_ne!(caps["hoverProvider"], Value::Null);
        assert_ne!(caps["hoverProvider"], json!(false));
        assert_ne!(caps["definitionProvider"], Value::Null);
        assert_ne!(caps["definitionProvider"], json!(false));
        assert!(
            caps["executeCommandProvider"]["commands"]
                .as_array()
                .unwrap()
                .contains(&json!("daml.setContext"))
        );
        server.notify("initialized", json!({}));
        server
    }
    fn send(&mut self, message: Message) {
        message.write(self.stdin.as_mut().unwrap()).unwrap();
    }
    fn receive(&self, deadline: Instant) -> Message {
        self.incoming
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("server must respond before deadline")
    }
    fn request(&mut self, method: &str, params: Value) -> lsp_server::Response {
        let id = self.next_id;
        self.next_id += 1;
        self.send(Request::new(id.into(), method.to_owned(), params).into());
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.receive(deadline) {
                Message::Response(response) => {
                    assert_eq!(response.id, RequestId::from(id));
                    return response;
                }
                Message::Notification(notice) => self.notices.push(notice),
                other => panic!("unexpected server request: {other:?}"),
            }
        }
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        self.request(method, params).response_result.unwrap()
    }
    fn notify(&mut self, method: &str, params: Value) {
        self.send(Notification::new(method.to_owned(), params).into());
    }
    fn open(&mut self, uri: &str, text: &str) {
        self.notices.clear();
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"daml","version":1,"text":text}}),
        );
    }
    fn change(&mut self, uri: &str, version: i32, text: &str) {
        self.notices.clear();
        self.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":text}]}),
        );
    }
    fn context(&mut self, uri: &str, kind: &str, root: Option<&str>, expected: Option<&str>) {
        self.notices.clear();
        let mut arg = json!({"uri":uri,"kind":kind});
        if let Some(root) = root {
            arg["bundleRoot"] = json!(root);
        }
        if let Some(expected) = expected {
            arg["expected"] = json!(expected);
        }
        let result = self.ok(
            "workspace/executeCommand",
            json!({"command":"daml.setContext","arguments":[arg]}),
        );
        let generation = u32::try_from(
            result["generation"]
                .as_u64()
                .expect("context result generation"),
        )
        .unwrap();
        assert!(
            generation > self.last_generation,
            "context generations must advance monotonically: {result}"
        );
        self.last_generation = generation;
        self.generations.insert(uri.to_owned(), generation);
        // A serial protocol barrier drains analysis notifications emitted by the
        // context change, including earlier same-version context publications.
        let _ = self.request(
            "textDocument/formatting",
            json!({"textDocument":{"uri":uri},"options":{"tabSize":2,"insertSpaces":true}}),
        );
    }
    fn notice(&mut self, method: &str, uri: &str, version: Option<i32>) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(index) = self.notices.iter().rposition(|n| {
                n.method == method
                    && n.params["uri"] == uri
                    && version.is_none_or(|v| n.params["version"] == v)
                    && (method != "daml/dependencies"
                        || n.params["generation"] == json!(self.generations[uri]))
            }) {
                let params = self.notices.remove(index).params;
                if method == "daml/dependencies" {
                    assert_eq!(
                        params["generation"],
                        json!(self.generations[uri]),
                        "dependency membership must identify current context generation"
                    );
                }
                return params;
            }
            match self.receive(deadline) {
                Message::Notification(n) => self.notices.push(n),
                other => panic!("unexpected message waiting for {method}: {other:?}"),
            }
        }
    }
    fn diagnostics(&mut self, uri: &str, version: i32, clean: bool) -> Value {
        let params = self.notice("textDocument/publishDiagnostics", uri, Some(version));
        let items = params["diagnostics"].as_array().unwrap();
        assert_eq!(
            items.is_empty(),
            clean,
            "diagnostics for version {version}: {params}"
        );
        params["diagnostics"].clone()
    }
    fn at(&mut self, method: &str, uri: &str, line: u32, character: u32) -> Value {
        self.ok(
            method,
            json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character}}),
        )
    }
    fn completion(&mut self, uri: &str, line: u32, character: u32) -> Vec<Value> {
        let result = self.at("textDocument/completion", uri, line, character);
        if result.is_null() {
            Vec::new()
        } else if let Some(items) = result.as_array() {
            items.clone()
        } else {
            result["items"].as_array().unwrap().clone()
        }
    }
    fn finish(mut self) {
        self.ok("shutdown", Value::Null);
        self.notify("exit", json!({}));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self
                .incoming
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Message::Notification(_)) => {}
                Ok(message) => panic!("unexpected shutdown message: {message:?}"),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("server did not close stdout after graceful shutdown")
                }
            }
        }
        let status = self.child.wait().expect("reap gracefully exited server");
        assert!(status.success(), "server exited with {status}");
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn item<'a>(items: &'a [Value], label: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["label"] == label)
        .unwrap_or_else(|| panic!("missing {label}: {items:?}"))
}
fn replacement(item: &Value) -> &Value {
    let edit = &item["textEdit"];
    if edit.get("range").is_some() {
        &edit["range"]
    } else {
        &edit["replace"]
    }
}
fn range(line: u32, start: u32, end: u32) -> Value {
    json!({"start":{"line":line,"character":start},"end":{"line":line,"character":end}})
}
fn locations(value: &Value) -> Vec<(&Value, &Value)> {
    let values: Vec<&Value> = if value.is_null() {
        vec![]
    } else if let Some(items) = value.as_array() {
        items.iter().collect()
    } else {
        vec![value]
    };
    values
        .into_iter()
        .map(|value| {
            if value.get("targetUri").is_some() {
                (&value["targetUri"], &value["targetSelectionRange"])
            } else {
                (&value["uri"], &value["range"])
            }
        })
        .collect()
}
fn assert_location(value: &Value, uri: &str, expected: Value) {
    assert!(
        locations(value)
            .iter()
            .any(|(u, r)| *u == uri && **r == expected),
        "missing actual source {uri} {expected}: {value}"
    );
}

#[test]
fn explicit_context_defaults_hints_and_real_source_navigation() {
    let fixture = Fixture::new();
    let uri = fixture.uri("workspace/config.daml");
    let root = fixture.uri("workspace");
    let mut server = Server::start("utf-16");
    server.open(&uri, CONFIG);
    server.diagnostics(&uri, 1, true);
    assert!(
        server.completion(&uri, 4, 0).is_empty(),
        "unselected context is syntax only"
    );
    assert!(server.at("textDocument/hover", &uri, 3, 6).is_null());
    server.context(&uri, "configuration", Some(&root), Some("app.Service"));
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    for path in [
        "workspace/daml.bundle",
        "workspace/app/Service.daml",
        "providers/Http.daml",
        "contracts/Adapter.daml",
    ] {
        assert!(
            files.contains(&json!(fixture.uri(path))),
            "missing dependency {path}: {dependencies}"
        );
    }
    assert_eq!(
        files
            .iter()
            .map(|file| file.as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        files.len()
    );
    let endpoint_hover = server
        .at("textDocument/hover", &uri, 3, 6)
        .to_string()
        .to_lowercase();
    assert!(
        endpoint_hover.contains("string"),
        "endpoint contract: {endpoint_hover}"
    );
    let adapter_hover = server
        .at("textDocument/hover", &uri, 2, 4)
        .to_string()
        .to_lowercase();
    assert!(
        adapter_hover.contains("30") && adapter_hover.contains("default"),
        "checked adapter must expose timeout default: {adapter_hover}"
    );
    let provider_tag = server.at("textDocument/definition", &uri, 2, 23);
    let provider_uri = fixture.uri("providers/Http.daml");
    assert!(
        locations(&provider_tag)
            .iter()
            .any(|(u, r)| u.as_str() == Some(provider_uri.as_str())
                && r["start"] == json!({"line":0,"character":0})),
        "{provider_tag}"
    );
    // Root type targets may cover the complete native expression; assert physical URI and start.
    let tag = server.at("textDocument/definition", &uri, 0, 5);
    let service_uri = fixture.uri("workspace/app/Service.daml");
    assert!(
        locations(&tag)
            .iter()
            .any(|(u, r)| u.as_str() == Some(service_uri.as_str())
                && r["start"] == json!({"line":0,"character":0})),
        "{tag}"
    );
    let endpoint = server.at("textDocument/definition", &uri, 3, 6);
    assert_location(
        &endpoint,
        &fixture.uri("providers/Http.daml"),
        range(1, 2, 10),
    );

    let prefix = "!app.Service {\n  name: \"demo\"\n  adapter: !providers.Http {\n    endpoint: \"https://example.com\"\n    time";
    server.change(&uri, 2, prefix);
    server.diagnostics(&uri, 2, false);
    let items = server.completion(&uri, 4, 8);
    let timeout = item(&items, "timeout");
    assert_eq!(replacement(timeout), &range(4, 4, 8));
    assert!(
        timeout.to_string().contains("30"),
        "completion must carry declared default: {timeout}"
    );
    assert!(
        !items.iter().any(|i| i["label"] == "endpoint"),
        "already supplied field must not recur"
    );
    server.change(&uri, 3, "!providers.");
    server.diagnostics(&uri, 3, false);
    let names = server.completion(&uri, 0, 11);
    assert_eq!(
        replacement(item(&names, "providers.Http")),
        &range(0, 1, 11)
    );
    item(&names, "providers.Unloaded"); // Listing must not parse an unused invalid body.
    assert!(!names.iter().any(|i| {
        i["label"]
            .as_str()
            .is_some_and(|s| s.starts_with("contracts."))
    }));
    server.change(&uri, 4, "!");
    server.diagnostics(&uri, 4, false);
    let visible = server.completion(&uri, 0, 1);
    item(&visible, "app.Service");
    assert!(!visible.iter().any(|i| {
        i["label"]
            .as_str()
            .is_some_and(|name| name.starts_with("contracts."))
    }));
    server.context(&uri, "configuration", Some(&root), None);
    server.change(&uri, 5, "!app.Worker {\n");
    server.diagnostics(&uri, 5, false);
    let fields = server.completion(&uri, 1, 0);
    assert!(
        item(&fields, "name")
            .to_string()
            .to_lowercase()
            .contains("required")
    );
    assert!(
        item(&fields, "note")
            .to_string()
            .to_lowercase()
            .contains("optional")
    );
    assert!(item(&fields, "retries").to_string().contains('3'));
    assert!(!fields.iter().any(|i| i["label"] == "endpoint"));
    for (version, source, line, character) in [
        (6, "!app.Worker {\n  name: \"ret", 1, 12),
        (7, "!app.Worker {\n  # ret", 1, 7),
        (8, "!app.Worker {\n  name: @\n  ret", 2, 5),
    ] {
        server.change(&uri, version, source);
        server.diagnostics(&uri, version, false);
        assert!(
            server.completion(&uri, line, character).is_empty(),
            "no structural suggestions inside unreliable context: {source}"
        );
    }
    server.change(&uri, 9, "!I");
    server.diagnostics(&uri, 9, false);
    let builtins = server.completion(&uri, 0, 2);
    assert_eq!(replacement(item(&builtins, "Int")), &range(0, 1, 2));
    server.change(&uri, 10, "!Int 3");
    server.diagnostics(&uri, 10, true);
    assert!(
        locations(&server.at("textDocument/definition", &uri, 0, 2)).is_empty(),
        "built-ins have no invented source"
    );
    server.finish();
}

#[test]
fn semantic_value_ranges_versions_context_admission_and_close_reopen() {
    for encoding in ["utf-8", "utf-16", "utf-32"] {
        let fixture = Fixture::new();
        let uri = fixture.uri("workspace/config.daml");
        let mut server = Server::start(encoding);
        let bad = "!providers.Http { endpoint: \"ok\"\n timeout: \"😀é\"\n}";
        server.open(&uri, bad);
        server.diagnostics(&uri, 1, true);
        // A data type error is not a syntax error until context is explicitly selected.
        server.context(&uri, "configuration", None, None);
        let errors = server.diagnostics(&uri, 1, false);
        let value_end = match encoding {
            "utf-8" => 18,
            "utf-16" => 15,
            _ => 14,
        };
        assert!(
            errors
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["range"] == range(1, 10, value_end)),
            "wrong Unicode timeout value span ({encoding}): {errors}"
        );
        let unicode_bad = "!providers.Http { endpoint: 42\n timeout: 30\n}";
        server.change(&uri, 2, unicode_bad);
        let errors = server.diagnostics(&uri, 2, false);
        assert!(
            errors
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["range"] == range(0, 28, 30)),
            "{errors}"
        );
        // A closed record must reject extra fields, even with a valid Unicode value.
        let unicode_bad = "!providers.Http { endpoint: \"😀é\"\n timeout: 30\n extra: 42\n}";
        server.change(&uri, 3, unicode_bad);
        server.diagnostics(&uri, 3, false); // closed record rejects unknown fields
        let source = "!app.Service { name: \"😀é\"\n adapter: !providers.Http { endpoint: 42 }\n}";
        server.change(&uri, 4, source);
        let errors = server.diagnostics(&uri, 4, false);
        assert!(
            errors
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["range"] == range(1, 38, 40)),
            "{errors}"
        );
        let source = "!providers.Http { endpoint: \"😀é\"\n timeout: 30\n}";
        server.change(&uri, 5, source);
        server.diagnostics(&uri, 5, true);
        // Inbound provider position after Unicode must use the negotiated units, too.
        let one_line = "!providers.Http { endpoint: \"😀é\" }";
        server.change(&uri, 6, one_line);
        server.diagnostics(&uri, 6, true);
        let end_char = match encoding {
            "utf-8" => one_line.len(),
            "utf-16" => one_line.encode_utf16().count(),
            _ => one_line.chars().count(),
        } as u32;
        let hover = server.at("textDocument/hover", &uri, 0, end_char - 3);
        assert!(
            !hover.is_null(),
            "hover after Unicode must map to the string: {encoding}"
        );
        // Old notifications cannot restore a rejected value or lower the accepted version.
        server.change(&uri, 5, bad);
        assert!(
            !server
                .at("textDocument/hover", &uri, 0, end_char - 3)
                .is_null()
        );
        let edits = server.ok(
            "textDocument/formatting",
            json!({"textDocument":{"uri":uri},"options":{"tabSize":2,"insertSpaces":true}}),
        );
        assert!(
            edits[0]["newText"]
                .as_str()
                .unwrap()
                .contains("endpoint: \"😀é\""),
            "old version must not replace current source: {edits}"
        );
        server.context(&uri, "syntax", None, None);
        server.diagnostics(&uri, 6, true);
        assert!(
            server.notice("daml/dependencies", &uri, Some(6))["files"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(server.at("textDocument/hover", &uri, 0, 2).is_null());
        server.notices.clear();
        server.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
        assert!(
            server.notice("daml/dependencies", &uri, None)["files"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        server.open(&uri, bad);
        server.diagnostics(&uri, 1, true);
        assert!(
            server.at("textDocument/hover", &uri, 0, 2).is_null(),
            "reopen must not retain context"
        );
        server.context(&uri, "configuration", None, None);
        server.diagnostics(&uri, 1, false);
        let reopened = server.notice("daml/dependencies", &uri, Some(1));
        assert!(!reopened["files"].as_array().unwrap().is_empty());
        server.finish();
    }
    let mut server = Server::start("utf-16");
    let uri = "untitled:semantic-no-filesystem";
    server.open(uri, "{ endpoint: 42 }");
    server.diagnostics(uri, 1, true);
    for arg in [
        json!({"uri":uri,"kind":"configuration"}),
        json!({"uri":uri,"kind":"definition"}),
        json!({"uri":uri,"kind":"unknown"}),
        json!({"uri":uri,"kind":"syntax","expected":"app.Service"}),
        json!({"uri":uri,"kind":"syntax","expected":null}),
        json!({"uri":uri,"kind":"syntax","bundleRoot":null}),
        json!({"uri":uri,"kind":"configuration","bundleRoot":"https://example.invalid/bundle"}),
        json!({"uri":uri,"kind":"configuration","bundleRoot":"file://remote/tmp/bundle"}),
        json!({"uri":uri,"kind":"configuration","extra":true}),
        json!({"uri":"untitled:not-open","kind":"syntax"}),
    ] {
        let response = server.request(
            "workspace/executeCommand",
            json!({"command":"daml.setContext","arguments":[arg]}),
        );
        assert_eq!(response.response_result.unwrap_err().code, -32602);
        assert!(
            server.at("textDocument/hover", uri, 0, 3).is_null(),
            "invalid context cannot install fallback semantics"
        );
    }
    let fixture = Fixture::new();
    server.context(
        uri,
        "configuration",
        Some(&fixture.uri("workspace")),
        Some("providers.Http"),
    );
    server.diagnostics(uri, 1, false);
    server.change(uri, 2, "{ endpoint: \"ok\" }");
    server.diagnostics(uri, 2, true);
    let generation = server.generations[uri];
    for (offset, argument) in [
        json!({"uri":uri,"kind":"syntax","expected":"providers.Http"}),
        json!({"uri":uri,"kind":"syntax","expected":null}),
        json!({"uri":uri,"kind":"syntax","bundleRoot":null}),
        json!({"uri":uri,"kind":"configuration","bundleRoot":fixture.uri("workspace"),"expected":null}),
    ]
    .into_iter()
    .enumerate()
    {
        let rejected = server.request(
            "workspace/executeCommand",
            json!({"command":"daml.setContext","arguments":[argument]}),
        );
        assert_eq!(rejected.response_result.unwrap_err().code, -32602);
        let version = 3 + i32::try_from(offset).unwrap();
        // A fresh version forces new analysis metadata, rather than accepting a
        // buffered dependency snapshot from before the rejected replacement.
        server.change(uri, version, "{ endpoint: \"still selected\" }");
        server.diagnostics(uri, version, true);
        assert!(
            !server.at("textDocument/hover", uri, 0, 4).is_null(),
            "rejected context must preserve selected semantics"
        );
        assert_eq!(
            server.notice("daml/dependencies", uri, Some(version))["generation"],
            generation
        );
    }
    let format = server.ok(
        "textDocument/formatting",
        json!({"textDocument":{"uri":uri},"options":{"tabSize":2,"insertSpaces":true}}),
    );
    assert!(
        format.is_array(),
        "syntax functionality remains available with explicit untitled context"
    );
    server.finish();
}

#[test]
fn unsaved_definition_manifest_and_standard_watch_invalidate_dependents() {
    let fixture = Fixture::new();
    let uri = fixture.uri("workspace/config.daml");
    let definition = fixture.uri("providers/Http.daml");
    let manifest = fixture.uri("workspace/daml.bundle");
    let mut server = Server::start("utf-16");
    server.open(&uri, CONFIG);
    server.diagnostics(&uri, 1, true);
    server.context(&uri, "configuration", None, None);
    server.diagnostics(&uri, 1, true);
    // An unselected open dependency is still an authoritative source overlay.
    server.open(
        &definition,
        &HTTP.replace("endpoint: !String", "endpoint: !Int"),
    );
    server.diagnostics(&uri, 1, false);
    server.change(&definition, 2, HTTP);
    server.diagnostics(&uri, 1, true);
    // Definition context validates native expressions, not a data-parser interpretation.
    server.context(&definition, "definition", None, None);
    server.diagnostics(&definition, 2, true);
    server.change(
        &definition,
        3,
        &HTTP.replace("!String", "!NoSuchNativeType"),
    );
    let definition_errors = server.diagnostics(&definition, 3, false);
    assert!(
        definition_errors
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["range"] == range(1, 13, 29)),
        "real failing reference name span: {definition_errors}"
    );
    let dependent = server.diagnostics(&uri, 1, false);
    assert!(
        dependent
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["relatedInformation"]
                .as_array()
                .is_some_and(|related| related.iter().any(|r| r["location"]["uri"] == definition))),
        "dependent error must identify actual definition source: {dependent}"
    );
    let oversized = "#".repeat(daml::Limits::default().max_input_bytes + 1);
    server.change(&definition, 4, &oversized);
    server.diagnostics(&definition, 4, false);
    server.diagnostics(&uri, 1, false); // Rejected open source must block the valid disk fallback.
    server.change(&definition, 5, HTTP);
    server.diagnostics(&uri, 1, true);
    server.notices.clear();
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":definition}}),
    );
    server.diagnostics(&uri, 1, true);
    server.open(&manifest, "{ app: \"./app\"\n providers: \"../absent\"\n}");
    server.diagnostics(&uri, 1, false);
    server.change(
        &manifest,
        2,
        include_str!("fixtures/semantic-editor/workspace/daml.bundle"),
    );
    server.diagnostics(&uri, 1, true);
    server.change(&manifest, 3, "{ broken:");
    server.diagnostics(&uri, 1, false); // No stale-disk fallback on malformed open manifests.
    server.notices.clear();
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":manifest}}),
    );
    server.diagnostics(&uri, 1, true);
    fs::write(
        fixture.0.join("providers/Http.daml"),
        HTTP.replace("endpoint: !String", "endpoint: !Int"),
    )
    .unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":definition,"type":2}]}),
    );
    server.diagnostics(&uri, 1, false);
    fs::write(fixture.0.join("providers/Http.daml"), HTTP).unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":definition,"type":2}]}),
    );
    server.diagnostics(&uri, 1, true);
    // A semantic request also refreshes disk state for clients without watcher support.
    fs::write(
        fixture.0.join("providers/Http.daml"),
        HTTP.replace("endpoint: !String", "endpoint: !Int"),
    )
    .unwrap();
    let hints = server
        .at("textDocument/hover", &uri, 3, 6)
        .to_string()
        .to_lowercase();
    assert!(
        hints.contains("int") || hints.contains("integer"),
        "request must not reuse stale disk constraints: {hints}"
    );
    server.finish();
}

#[test]
fn native_definition_reference_prefixes_use_native_scope_and_original_ranges() {
    let fixture = Fixture::new();
    let uri = fixture.uri("workspace/app/Service.daml");
    let mut server = Server::start("utf-16");
    server.open(&uri, "closed !Object {\n endpoint: !Str\n}");
    server.context(&uri, "definition", None, None);
    server.diagnostics(&uri, 1, false);
    let partial_builtin = server.completion(&uri, 1, 15);
    assert_eq!(
        replacement(item(&partial_builtin, "String")),
        &range(1, 12, 15)
    );

    server.change(&uri, 2, "closed !Object {\n endpoint: !\n}");
    server.diagnostics(&uri, 2, false);
    let empty_reference = server.completion(&uri, 1, 12);
    assert_eq!(
        replacement(item(&empty_reference, "String")),
        &range(1, 12, 12)
    );
    assert!(!empty_reference.iter().any(|candidate| {
        candidate["label"]
            .as_str()
            .is_some_and(|name| name.starts_with("providers."))
    }));

    server.change(&uri, 3, "closed !Object {\n endpoint: !contracts.\n}");
    server.diagnostics(&uri, 3, false);
    let scoped_reference = server.completion(&uri, 1, 22);
    assert_eq!(
        replacement(item(&scoped_reference, "contracts.Adapter")),
        &range(1, 12, 22)
    );

    server.change(&uri, 4, "closed !Object {\n endpoint: \"!Str\n}");
    server.diagnostics(&uri, 4, false);
    assert!(server.completion(&uri, 1, 16).is_empty());
    server.change(&uri, 5, "closed !Object {\n # !contracts.\n}");
    server.diagnostics(&uri, 5, true);
    assert!(server.completion(&uri, 1, 14).is_empty());
    for (version, source, character, label) in [
        (6, "closed !Object {\n endpoint: !Str", 15, "String"),
        (7, "closed !Object {\n endpoint: !", 12, "String"),
        (
            8,
            "closed !Object {\n endpoint: !contracts.",
            22,
            "contracts.Adapter",
        ),
    ] {
        server.change(&uri, version, source);
        server.diagnostics(&uri, version, false);
        let items = server.completion(&uri, 1, character);
        assert_eq!(replacement(item(&items, label)), &range(1, 12, character));
    }
    for (version, source, character) in [
        (9, "closed !Object {\n endpoint: \"!Str", 16),
        (10, "closed !Object {\n # !contracts.", 14),
    ] {
        server.change(&uri, version, source);
        server.diagnostics(&uri, version, false);
        assert!(server.completion(&uri, 1, character).is_empty());
    }
    server.finish();
}

#[test]
fn applying_field_completion_before_existing_colon_preserves_supplied_value() {
    let fixture = Fixture::new();
    let uri = fixture.uri("workspace/config.daml");
    let source = "!providers.Http {\n endpoint: \"ok\"\n time: 30\n}";
    let mut server = Server::start("utf-16");
    server.open(&uri, source);
    server.context(&uri, "configuration", None, None);
    server.diagnostics(&uri, 1, false);
    let items = server.completion(&uri, 2, 5);
    let timeout = item(&items, "timeout");
    assert_eq!(replacement(timeout), &range(2, 1, 5));

    // The asserted wire range selects exactly this ASCII key. Apply the actual
    // server edit, not a synthesized correction or only the completion label.
    let key_start = source.find("time: 30").unwrap();
    let mut applied = source.to_owned();
    applied.replace_range(
        key_start..key_start + "time".len(),
        timeout["textEdit"]["newText"].as_str().unwrap(),
    );
    let document = daml::parse(&applied).expect("completion must produce valid source");
    let bundle = daml::Bundle::open(fixture.0.join("workspace")).unwrap();
    let schemas = bundle.compile(&document, None).unwrap();
    assert_eq!(
        schemas.resolve(&document).unwrap().to_json(),
        json!({"endpoint": "ok", "timeout": 30})
    );
    server.change(&uri, 2, &applied);
    server.diagnostics(&uri, 2, true);
    server.finish();
}

#[test]
fn closed_union_field_hints_preserve_each_branch_at_root_and_nested_property() {
    let fixture = Fixture::new();
    let bundle = daml::Bundle::open(fixture.0.join("workspace")).unwrap();
    for (root, nested) in [("app.Choice", false), ("app.NestedChoice", true)] {
        let supplied = |key| {
            if nested {
                format!("!{root} {{\n choice: {{\n  {key}: \"kept\"\n }}\n}}")
            } else {
                format!("!{root} {{ {key}: \"kept\" }}")
            }
        };
        // Ground the expected hints in the ordinary native compiler/resolver:
        // a and c are usable in separate branches, while b is rejected by every
        // branch even though one open conjunct declares it.
        for field in ["a", "c"] {
            let source = supplied(field);
            let document = daml::parse(&source).unwrap();
            let schemas = bundle.compile(&document, None).unwrap();
            let resolved = schemas.resolve(&document).unwrap().to_json();
            let path = if nested {
                format!("/choice/{field}")
            } else {
                format!("/{field}")
            };
            assert_eq!(resolved.pointer(&path), Some(&json!("kept")));
        }
        let invalid_source = supplied("b");
        let invalid = daml::parse(&invalid_source).unwrap();
        let schemas = bundle.compile(&invalid, None).unwrap();
        assert!(
            schemas.resolve(&invalid).is_err(),
            "ordinary resolution must reject forbidden b: {invalid_source}"
        );

        let uri = fixture.uri("workspace/config.daml");
        let mut server = Server::start("utf-16");
        let (source, line) = if nested {
            (format!("!{root} {{\n choice: {{\n"), 2)
        } else {
            (format!("!{root} {{\n"), 1)
        };
        server.open(&uri, &source);
        server.context(&uri, "configuration", None, None);
        server.diagnostics(&uri, 1, false);
        let fields = server.completion(&uri, line, 0);
        item(&fields, "a");
        item(&fields, "c");
        assert!(
            !fields.iter().any(|candidate| candidate["label"] == "b"),
            "closed constraints cannot be flattened across union branches: {fields:?}"
        );
        server.finish();
    }
}

#[test]
fn discovery_watches_missing_nearer_manifest_and_initially_absent_bundle() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("workspace/nested")).unwrap();
    let uri = fixture.uri("workspace/nested/config.daml");
    let nearer = fixture.uri("workspace/nested/daml.bundle");
    let outer = fixture.uri("workspace/daml.bundle");
    let source = "!providers.Http { endpoint: \"ok\" }";
    let mut server = Server::start("utf-16");
    server.open(&uri, source);
    server.context(&uri, "configuration", None, None);
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    assert!(
        files.contains(&json!(nearer)),
        "missing nearer discovery probe: {dependencies}"
    );
    assert!(
        files.contains(&json!(outer)),
        "missing discovered outer bundle: {dependencies}"
    );

    // A newly created nearer bundle shadows the outer aliases. No source edit or
    // semantic provider request may be the trigger for this diagnostic change.
    fs::write(fixture.0.join("workspace/nested/daml.bundle"), "{}\n").unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":nearer,"type":1}]}),
    );
    server.diagnostics(&uri, 1, false);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    assert!(
        dependencies["files"]
            .as_array()
            .unwrap()
            .contains(&json!(nearer))
    );

    fs::remove_file(fixture.0.join("workspace/nested/daml.bundle")).unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":nearer,"type":3}]}),
    );
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    assert!(
        dependencies["files"]
            .as_array()
            .unwrap()
            .contains(&json!(nearer))
    );
    server.finish();

    fs::create_dir(fixture.0.join("orphan")).unwrap();
    let uri = fixture.uri("orphan/config.daml");
    let manifest = fixture.uri("orphan/daml.bundle");
    let definition = fixture.uri("orphan/Config.daml");
    let mut server = Server::start("utf-16");
    server.open(&uri, "!self.Config { endpoint: \"ok\" }");
    server.context(&uri, "configuration", None, None);
    server.diagnostics(&uri, 1, false);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    assert!(
        dependencies["files"]
            .as_array()
            .unwrap()
            .contains(&json!(manifest)),
        "missing-bundle analysis must retain attempted manifest candidates: {dependencies}"
    );
    fs::write(
        fixture.0.join("orphan/Config.daml"),
        "closed !Object {\n endpoint: !String\n}\n",
    )
    .unwrap();
    fs::write(fixture.0.join("orphan/daml.bundle"), "{}\n").unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":manifest,"type":1}]}),
    );
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    assert!(files.contains(&json!(manifest)));
    assert!(files.contains(&json!(definition)));
    server.finish();
}

#[test]
fn closed_union_field_detail_and_checked_hover_exclude_ineligible_string_branch() {
    let fixture = Fixture::new();
    let bundle = daml::Bundle::open(fixture.0.join("workspace")).unwrap();
    let supplied = "!app.ChoiceFieldDetail {\n b: 1\n}";
    let document = daml::parse(supplied).unwrap();
    let schemas = bundle.compile(&document, None).unwrap();
    assert_eq!(
        schemas.resolve(&document).unwrap().to_json(),
        json!({"b": 1})
    );
    let invalid = daml::parse("!app.ChoiceFieldDetail {\n b: \"wrong\"\n}").unwrap();
    let invalid_schemas = bundle.compile(&invalid, None).unwrap();
    assert!(
        invalid_schemas.resolve(&invalid).is_err(),
        "the open String declaration is ineligible under its closed conjunct"
    );
    let explanation = schemas.explain(&document, None, "/b").unwrap();
    assert_eq!(explanation.node().to_json(), json!(1));
    let value_start = supplied.find('1').unwrap();
    let origin = daml::Span {
        start: value_start,
        end: value_start + 1,
    };
    assert_eq!(explanation.node().origin(), daml::Origin::Input(origin));
    assert!(explanation.constraints().iter().any(|constraint| {
        constraint.instance_path == "/b" && constraint.schema["type"] == "integer"
    }));
    assert!(!explanation.constraints().iter().any(|constraint| {
        constraint.instance_path == "/b" && constraint.schema["type"] == "string"
    }));

    let uri = fixture.uri("workspace/config.daml");
    let mut server = Server::start("utf-16");
    server.open(&uri, "!app.ChoiceFieldDetail {\n");
    server.context(&uri, "configuration", None, None);
    server.diagnostics(&uri, 1, false);
    let fields = server.completion(&uri, 1, 0);
    let detail = item(&fields, "b")["detail"]
        .as_str()
        .unwrap()
        .to_lowercase();
    assert!(
        detail.contains("integer") || detail.contains("int"),
        "b must describe the eligible integer declaration: {detail}"
    );
    assert!(
        !detail.contains("string"),
        "b must not describe the ineligible String declaration: {detail}"
    );

    server.change(&uri, 2, supplied);
    server.diagnostics(&uri, 2, true);
    let hover = server.at("textDocument/hover", &uri, 1, 4);
    assert_eq!(hover["range"], range(1, 4, 5));
    let text = hover.to_string().to_lowercase();
    assert!(
        text.contains("integer"),
        "checked b has integer constraints: {hover}"
    );
    assert!(
        !text.contains("string"),
        "checked b must not include impossible String constraints: {hover}"
    );
    assert!(
        text.contains(&format!("{}..{}", origin.start, origin.end)),
        "hover must retain the actual supplied-value origin: {hover}"
    );
    server.finish();
}

#[test]
fn explicit_definition_root_retains_nested_manifest_watch_after_boundary_failure() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("workspace/owned/sub")).unwrap();
    fs::write(fixture.0.join("workspace/owned/daml.bundle"), "{}\n").unwrap();
    let source = "closed !Object {\n endpoint: !String\n}\n";
    fs::write(fixture.0.join("workspace/owned/sub/Foo.daml"), source).unwrap();
    let root = fixture.uri("workspace/owned");
    let uri = fixture.uri("workspace/owned/sub/Foo.daml");
    let manifest = fixture.uri("workspace/owned/sub/daml.bundle");
    let directory = fixture.uri("workspace/owned/sub");
    let mut server = Server::start("utf-16");
    server.open(&uri, source);
    server.context(&uri, "definition", Some(&root), None);
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    assert!(
        dependencies["files"]
            .as_array()
            .unwrap()
            .contains(&json!(manifest)),
        "explicit-root admission must retain missing nested-manifest probes: {dependencies}"
    );

    fs::write(fixture.0.join("workspace/owned/sub/daml.bundle"), "{}\n").unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":manifest,"type":1}]}),
    );
    server.diagnostics(&uri, 1, false);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    assert!(
        files.contains(&json!(manifest)),
        "early boundary failure must retain the actual manifest URI: {dependencies}"
    );
    assert!(
        !files.contains(&json!(directory)),
        "a directory URI cannot stand in for the manifest watcher: {dependencies}"
    );

    fs::remove_file(fixture.0.join("workspace/owned/sub/daml.bundle")).unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":manifest,"type":3}]}),
    );
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    assert!(
        dependencies["files"]
            .as_array()
            .unwrap()
            .contains(&json!(manifest)),
        "recovered explicit root must continue watching the missing boundary: {dependencies}"
    );
    server.finish();
}

#[test]
fn alternative_required_hints_keep_eligible_branch_obligations() {
    let fixture = Fixture::new();
    for (name, definition, empty_valid, string_valid, conditional_required) in [
        (
            "RequiredChoice",
            "!Object {\n b: !String\n} | !Object {\n b: !Int\n}",
            false,
            true,
            true,
        ),
        (
            "EligibleOptional",
            "(closed !Object {\n a?: !String\n} & !Object {\n b: !String\n}) | closed !Object {\n b?: !Int\n}",
            true,
            false,
            false,
        ),
    ] {
        fs::write(
            fixture.0.join(format!("workspace/app/{name}.daml")),
            definition,
        )
        .unwrap();
        let bundle = daml::Bundle::open(fixture.0.join("workspace")).unwrap();
        for (body, valid) in [
            ("{}", empty_valid),
            ("{ b: 1 }", true),
            ("{ b: \"kept\" }", string_valid),
        ] {
            let source = format!("!app.{name} {body}");
            let document = daml::parse(&source).unwrap();
            let schemas = bundle.compile(&document, None).unwrap();
            assert_eq!(
                schemas.resolve(&document).is_ok(),
                valid,
                "ordinary core admission establishes the branch obligation: {source}"
            );
        }
        let uri = fixture.uri("workspace/config.daml");
        let mut server = Server::start("utf-16");
        server.open(&uri, &format!("!app.{name} {{\n"));
        server.context(&uri, "configuration", None, None);
        server.diagnostics(&uri, 1, false);
        let fields = server.completion(&uri, 1, 0);
        let detail = item(&fields, "b")["detail"]
            .as_str()
            .unwrap()
            .to_lowercase();
        if conditional_required {
            assert!(
                detail.contains("required") && detail.contains("alternative"),
                "required branch obligations need conditional required information: {detail}"
            );
            assert!(
                !detail.contains("optional"),
                "b is required in both admitted alternatives, not optional: {detail}"
            );
        } else {
            assert!(
                detail.contains("optional") && !detail.contains("required"),
                "an ineligible closed branch must not leak its required b declaration: {detail}"
            );
        }
        server.finish();
    }
}

#[test]
fn dependency_uris_are_physical_consulted_files_without_dot_alias_duplicates() {
    let fixture = Fixture::new();
    let uri = fixture.uri("workspace/config.daml");
    let mut server = Server::start("utf-16");
    server.open(&uri, CONFIG);
    server.context(&uri, "configuration", None, None);
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    // The real compiler consults these sources through ./app, ../providers,
    // and app's ../../contracts import. Derive identities from the filesystem,
    // not from copies of the aliases or the wire notification's own spellings.
    for source in [
        "workspace/daml.bundle",
        "workspace/app/daml.bundle",
        "workspace/app/Service.daml",
        "providers/daml.bundle",
        "providers/Http.daml",
        "contracts/daml.bundle",
        "contracts/Adapter.daml",
    ] {
        let physical = fs::canonicalize(fixture.0.join(source)).unwrap();
        assert!(
            files.contains(&json!(fixture.uri(&physical))),
            "missing physically consulted source {physical:?}: {dependencies}"
        );
    }
    let mut physical_files = std::collections::BTreeSet::new();
    let mut wire_files = std::collections::BTreeSet::new();
    for file in files {
        let spelling = file.as_str().unwrap();
        let uri: lsp_types::Uri = spelling.parse().unwrap();
        assert!(uri.scheme().is_some_and(|scheme| scheme.as_str() == "file"));
        assert!(
            wire_files.insert(spelling),
            "duplicate dependency URI: {spelling}"
        );
        let component = uri.path();
        let bytes = component.as_estr().decode().into_bytes();
        assert!(
            !bytes
                .split(|byte| *byte == b'/')
                .any(|segment| segment == b"." || segment == b".."),
            "watcher identity must not retain lexical dot segments: {spelling}"
        );
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(bytes.into_owned()))
        };
        #[cfg(not(unix))]
        let path = {
            let mut text = String::from_utf8(bytes.into_owned()).unwrap();
            #[cfg(windows)]
            if text.starts_with('/') && text.as_bytes().get(2) == Some(&b':') {
                text.remove(0);
            }
            PathBuf::from(text)
        };
        if let Ok(physical) = fs::canonicalize(path) {
            assert!(
                physical.is_file(),
                "watch metadata must identify files, not directories: {physical:?}"
            );
            assert!(
                physical_files.insert(physical.clone()),
                "two watcher URIs alias the same physical source {physical:?}: {dependencies}"
            );
            assert_eq!(
                spelling,
                fixture.uri(physical),
                "existing dependencies must use their physical file URI"
            );
        }
        // Missing candidates remain legitimate dependencies; the discovery and
        // explicit-root create/delete scenarios separately prove their recovery.
    }
    server.finish();
}

#[cfg(unix)]
#[test]
fn final_definition_symlink_entry_and_physical_target_remain_watch_dependencies() {
    use std::os::unix::fs::symlink;

    fn require_snapshot(server: &mut Server, uri: &str, links: &[String], target: &str) {
        let dependencies = server.notice("daml/dependencies", uri, Some(1));
        let files = dependencies["files"].as_array().unwrap();
        for expected in links.iter().map(String::as_str).chain([target]) {
            assert!(
                files.contains(&json!(expected)),
                "every file-leaf link and physical source must remain watched ({expected}): {dependencies}"
            );
        }
    }

    for (name, chained) in [("file-retarget", false), ("file-retarget-chain", true)] {
        let fixture = Fixture::new();
        let bundle = fixture.0.join(name);
        fs::create_dir(&bundle).unwrap();
        fs::write(bundle.join("daml.bundle"), "{}\n").unwrap();
        let string_source = "!String\n";
        let int_source = "!Int\n";
        fs::write(bundle.join("StringDef.daml"), string_source).unwrap();
        fs::write(bundle.join("IntDef.daml"), int_source).unwrap();
        if chained {
            symlink("StringDef.daml", bundle.join("Middle.daml")).unwrap();
        }
        let alias_target = if chained {
            "Middle.daml"
        } else {
            "StringDef.daml"
        };
        symlink(alias_target, bundle.join("Alias.daml")).unwrap();
        let physical_parent = fs::canonicalize(&bundle).unwrap();
        let alias = fixture.uri(physical_parent.join("Alias.daml"));
        let entry_name = if chained { "Middle.daml" } else { "Alias.daml" };
        let entry = fixture.uri(physical_parent.join(entry_name));
        let mut links = vec![alias];
        if chained {
            links.push(entry.clone());
        }
        let string_target = fixture.uri(fs::canonicalize(bundle.join("StringDef.daml")).unwrap());
        assert_ne!(links[0], string_target);
        let uri = fixture.uri("workspace/file-retarget.daml");
        let mut server = Server::start("utf-16");
        server.open(&uri, "!self.Alias \"kept\"\n");
        server.context(
            &uri,
            "configuration",
            Some(&fixture.uri(&bundle)),
            Some("self.Alias"),
        );
        server.diagnostics(&uri, 1, true);
        require_snapshot(&mut server, &uri, &links, &string_target);

        for (target, clean) in [("IntDef.daml", false), ("StringDef.daml", true)] {
            symlink(target, bundle.join("Retarget.pending")).unwrap();
            fs::rename(bundle.join("Retarget.pending"), bundle.join(entry_name)).unwrap();
            server.notices.clear();
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":entry,"type":2}]}),
            );
            server.diagnostics(&uri, 1, clean);
            let target_uri = fixture.uri(fs::canonicalize(bundle.join(target)).unwrap());
            require_snapshot(&mut server, &uri, &links, &target_uri);
            if chained {
                assert_eq!(
                    fs::read_link(bundle.join("Alias.daml")).unwrap(),
                    PathBuf::from("Middle.daml")
                );
            }
        }
        assert_eq!(
            fs::read_link(bundle.join("Alias.daml")).unwrap(),
            PathBuf::from(alias_target)
        );
        assert_eq!(
            fs::read_to_string(bundle.join("StringDef.daml")).unwrap(),
            string_source
        );
        assert_eq!(
            fs::read_to_string(bundle.join("IntDef.daml")).unwrap(),
            int_source
        );
        server.finish();
    }
}

#[cfg(unix)]
#[test]
fn file_symlink_chain_retains_missing_terminal_creation_dependency() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let bundle = fixture.0.join("missing-file-target");
    fs::create_dir(&bundle).unwrap();
    fs::write(bundle.join("daml.bundle"), "{}\n").unwrap();
    symlink("Middle.daml", bundle.join("Alias.daml")).unwrap();
    symlink("MissingDef.daml", bundle.join("Middle.daml")).unwrap();
    let physical_parent = fs::canonicalize(&bundle).unwrap();
    let terminal = fixture.uri(physical_parent.join("MissingDef.daml"));
    let uri = fixture.uri("workspace/missing-file-target.daml");
    let mut server = Server::start("utf-16");
    server.open(&uri, "!self.Alias \"kept\"\n");
    server.context(
        &uri,
        "configuration",
        Some(&fixture.uri(&bundle)),
        Some("self.Alias"),
    );
    for ready in [false, true] {
        if ready {
            // The chain is fixed: only its previously absent terminal appears.
            fs::write(bundle.join("MissingDef.daml"), "!String\n").unwrap();
            server.notices.clear();
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":terminal,"type":1}]}),
            );
        }
        server.diagnostics(&uri, 1, ready);
        let dependencies = server.notice("daml/dependencies", &uri, Some(1));
        let files = dependencies["files"].as_array().unwrap();
        for entry in ["Alias.daml", "Middle.daml", "MissingDef.daml"] {
            let expected = fixture.uri(physical_parent.join(entry));
            assert!(
                files.contains(&json!(expected)),
                "missing-target analysis must retain the entire leaf-link chain ({entry}): {dependencies}"
            );
        }
    }
    assert_eq!(
        fs::read_link(bundle.join("Alias.daml")).unwrap(),
        PathBuf::from("Middle.daml")
    );
    assert_eq!(
        fs::read_link(bundle.join("Middle.daml")).unwrap(),
        PathBuf::from("MissingDef.daml")
    );
    server.finish();
}

#[cfg(unix)]
#[test]
fn filesystem_symlink_cycle_stays_invalid_but_retains_repair_dependencies() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let bundle = fixture.0.join("cyclic-file-links");
    fs::create_dir(&bundle).unwrap();
    fs::write(bundle.join("daml.bundle"), "{}\n").unwrap();
    fs::write(bundle.join("StringDef.daml"), "!String\n").unwrap();
    symlink("Middle.daml", bundle.join("Alias.daml")).unwrap();
    symlink("Alias.daml", bundle.join("Middle.daml")).unwrap();
    let source = "!self.Alias \"kept\"\n";
    let document = daml::parse(source).unwrap();
    let ordinary = daml::Bundle::open(&bundle).unwrap();
    let error = ordinary
        .compile(&document, None)
        .err()
        .expect("a filesystem link cycle cannot certify valid data");
    // Ordinary filesystem canonical_file admission classifies failed definition
    // canonicalization as MissingDefinition and retains the underlying I/O cause.
    assert_eq!(error.kind, daml::NativeErrorKind::MissingDefinition);
    assert!(
        std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .is_some(),
        "the filesystem failure cause must remain available"
    );

    let parent = fs::canonicalize(&bundle).unwrap();
    let alias = fixture.uri(parent.join("Alias.daml"));
    let middle = fixture.uri(parent.join("Middle.daml"));
    let manifest = fixture.uri(parent.join("daml.bundle"));
    let uri = fixture.uri("workspace/cyclic-file-links.daml");
    let mut server = Server::start("utf-16");
    server.open(&uri, source);
    server.context(
        &uri,
        "configuration",
        Some(&fixture.uri(&bundle)),
        Some("self.Alias"),
    );
    server.diagnostics(&uri, 1, false);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    for expected in [&alias, &middle, &manifest] {
        assert!(
            files.contains(&json!(expected)),
            "filesystem cycle failure must retain each finite repair entry: {dependencies}"
        );
    }
    assert_eq!(
        files
            .iter()
            .map(|file| file.as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        files.len(),
        "cycle traversal must not repeat dependency entries"
    );

    // Only Middle is retargeted; Alias, the completed data, and the preexisting
    // physical source remain untouched. No provider request triggers recovery.
    symlink("StringDef.daml", bundle.join("Retarget.pending")).unwrap();
    fs::rename(bundle.join("Retarget.pending"), bundle.join("Middle.daml")).unwrap();
    server.notices.clear();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":middle,"type":2}]}),
    );
    server.diagnostics(&uri, 1, true);
    let dependencies = server.notice("daml/dependencies", &uri, Some(1));
    let files = dependencies["files"].as_array().unwrap();
    let physical = fixture.uri(fs::canonicalize(bundle.join("StringDef.daml")).unwrap());
    for expected in [&alias, &middle, &manifest, &physical] {
        assert!(files.contains(&json!(expected)), "{dependencies}");
    }
    assert_eq!(
        fs::read_link(bundle.join("Alias.daml")).unwrap(),
        PathBuf::from("Middle.daml")
    );
    assert_eq!(
        fs::read_to_string(bundle.join("StringDef.daml")).unwrap(),
        "!String\n"
    );
    server.finish();
}
