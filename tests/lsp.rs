//! Actual stdio LSP lifecycle, Unicode diagnostics, and client-applied formatting.

#![cfg(feature = "lsp")]

use std::{
    io::BufReader,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);

struct LanguageServer {
    child: Child,
    stdin: Option<ChildStdin>,
    incoming: Receiver<Message>,
    reader: Option<JoinHandle<()>>,
}

impl LanguageServer {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_daml-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start daml-lsp process");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = child.stdout.take().expect("child stdout");
        let (sender, incoming) = mpsc::channel();
        let reader = thread::spawn(move || read_messages(stdout, sender));
        Self {
            child,
            stdin: Some(stdin),
            incoming,
            reader: Some(reader),
        }
    }

    fn send(&mut self, message: Message) {
        message
            .write(self.stdin.as_mut().expect("open client stdin"))
            .expect("write LSP message");
    }

    fn request(&mut self, id: i32, method: &str, params: Value) -> Response {
        self.send(Request::new(id.into(), method.to_owned(), params).into());
        match self.receive() {
            Message::Response(response) if response.id == RequestId::from(id) => response,
            message => panic!("unexpected server message while waiting for {method}: {message:?}"),
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(Notification::new(method.to_owned(), params).into());
    }

    fn receive(&self) -> Message {
        match self.incoming.recv_timeout(RECEIVE_TIMEOUT) {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for LSP message"),
            Err(RecvTimeoutError::Disconnected) => panic!("LSP process output closed"),
        }
    }

    fn diagnostic(&self) -> Value {
        loop {
            if let Message::Notification(notification) = self.receive()
                && notification.method == "textDocument/publishDiagnostics"
            {
                return notification.params;
            }
        }
    }
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn read_messages(stdout: impl std::io::Read, sender: Sender<Message>) {
    let mut stdout = BufReader::new(stdout);
    loop {
        match Message::read(&mut stdout) {
            Ok(Some(message)) => {
                if sender.send(message).is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + RECEIVE_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("check server process") {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for server process exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn initialize(server: &mut LanguageServer, encodings: Option<Value>, expected_encoding: &str) {
    let capabilities = encodings.map_or_else(
        || json!({}),
        |position_encodings| json!({"general": {"positionEncodings": position_encodings}}),
    );
    let response = server.request(
        1,
        "initialize",
        json!({"processId": null, "capabilities": capabilities}),
    );
    let result = response.response_result.expect("initialize succeeds");
    assert_eq!(
        result["capabilities"]["positionEncoding"],
        expected_encoding
    );
    assert_eq!(
        result["capabilities"]["textDocumentSync"]["openClose"],
        true
    );
    assert_eq!(result["capabilities"]["textDocumentSync"]["change"], 1);
    assert_eq!(result["capabilities"]["documentFormattingProvider"], true);
    server.notify("initialized", json!({}));
}

fn open(server: &mut LanguageServer, uri: &str, version: i32, text: &str) {
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": uri, "languageId": "daml", "version": version, "text": text}}),
    );
}

fn diagnostics(server: &LanguageServer, uri: &str, version: i32, expected: usize) -> Value {
    let diagnostics = server.diagnostic();
    assert_eq!(diagnostics["uri"], uri);
    assert_eq!(diagnostics["version"], version);
    assert_eq!(
        diagnostics["diagnostics"].as_array().unwrap().len(),
        expected
    );
    diagnostics
}

fn format_request(server: &mut LanguageServer, id: i32, uri: &str) -> Response {
    server.request(
        id,
        "textDocument/formatting",
        json!({"textDocument": {"uri": uri}, "options": {"tabSize": 2, "insertSpaces": true}}),
    )
}

#[test]
fn stdio_lifecycle_diagnostics_formatting_and_document_isolation() {
    let mut server = LanguageServer::start();
    initialize(&mut server, None, "utf-16");

    let first = "untitled:daml-lsp-first";
    let second = "file:///definitely-not-a-real-daml-lsp-document.daml";
    let invalid = "\"😀é\" ?";
    open(&mut server, first, 1, invalid);
    let initial = diagnostics(&server, first, 1, 1);
    assert_eq!(
        initial["diagnostics"][0]["range"]["start"],
        json!({"line": 0, "character": 6})
    );
    assert_eq!(
        initial["diagnostics"][0]["range"]["end"],
        json!({"line": 0, "character": 7})
    );
    assert_eq!(
        format_request(&mut server, 10, first)
            .response_result
            .unwrap_err()
            .code,
        -32803
    );

    server.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": first, "version": 2}, "contentChanges": [{"text": "{name: \"Mira\"\n values: [1\n2]}"}]}),
    );
    diagnostics(&server, first, 2, 0);
    server.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": first, "version": 1}, "contentChanges": [{"text": "[still invalid,]"}]}),
    );
    let edits = format_request(&mut server, 2, first)
        .response_result
        .unwrap();
    assert_eq!(
        edits,
        json!([{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 3}},
            "newText": "{\n  name: \"Mira\"\n  values: [\n    1\n    2\n  ]\n}\n"
        }])
    );
    let formatted = edits[0]["newText"].as_str().unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": first, "version": 3}, "contentChanges": [{"text": formatted}]}),
    );
    diagnostics(&server, first, 3, 0);
    assert_eq!(
        format_request(&mut server, 3, first)
            .response_result
            .unwrap(),
        json!([])
    );

    open(&mut server, second, 4, "{other: \"😀\"}");
    diagnostics(&server, second, 4, 0);
    assert_eq!(
        format_request(&mut server, 4, second)
            .response_result
            .unwrap(),
        json!([{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 13}},
            "newText": "{ other: \"😀\" }\n"
        }])
    );
    assert_eq!(
        format_request(&mut server, 5, "file:///not-open.daml")
            .response_result
            .unwrap_err()
            .code,
        -32602
    );

    server.notify(
        "textDocument/didClose",
        json!({"textDocument": {"uri": first}}),
    );
    let cleared = server.diagnostic();
    assert_eq!(cleared["uri"], first);
    assert!(cleared["version"].is_null());
    assert_eq!(cleared["diagnostics"], json!([]));
    assert_eq!(
        format_request(&mut server, 6, first)
            .response_result
            .unwrap_err()
            .code,
        -32602
    );

    let unknown = server.request(7, "workspace/unknown", json!({}));
    assert_eq!(unknown.response_result.unwrap_err().code, -32601);
    let invalid_params = server.request(8, "textDocument/formatting", json!({}));
    assert_eq!(invalid_params.response_result.unwrap_err().code, -32602);

    let shutdown = server.request(9, "shutdown", Value::Null);
    assert!(shutdown.response_result.is_ok());
    server.notify("exit", json!({}));
    let status = wait_for_exit(&mut server.child);
    assert!(status.success(), "server exited with {status}");
    // The client keeps stdin open until after the server exits; drop cleans up the reader.
}

#[test]
fn negotiation_selects_first_supported_encoding_and_utf32_positions() {
    let mut server = LanguageServer::start();
    initialize(&mut server, Some(json!(["utf-8", "utf-32"])), "utf-8");
    let uri = "untitled:encoding";
    open(&mut server, uri, 1, "\"😀é\" ?");
    let diag = server.diagnostic();
    assert_eq!(
        diag["diagnostics"][0]["range"]["start"],
        json!({"line": 0, "character": 9})
    );
    assert_eq!(
        diag["diagnostics"][0]["range"]["end"],
        json!({"line": 0, "character": 10})
    );
    let shutdown = server.request(2, "shutdown", Value::Null);
    assert!(shutdown.response_result.is_ok());
    server.notify("exit", json!({}));
    let status = wait_for_exit(&mut server.child);
    assert!(status.success(), "server exited with {status}");

    let mut server = LanguageServer::start();
    initialize(&mut server, Some(json!(["utf-32"])), "utf-32");
    open(&mut server, uri, 1, "\"😀é\" ?");
    let diag = server.diagnostic();
    assert_eq!(
        diag["diagnostics"][0]["range"]["start"],
        json!({"line": 0, "character": 5})
    );
    assert_eq!(
        diag["diagnostics"][0]["range"]["end"],
        json!({"line": 0, "character": 6})
    );
    let shutdown = server.request(3, "shutdown", Value::Null);
    assert!(shutdown.response_result.is_ok());
    server.notify("exit", json!({}));
    let status = wait_for_exit(&mut server.child);
    assert!(status.success(), "server exited with {status}");
}

#[test]
fn formatting_cannot_push_a_valid_literal_past_native_scalar_limits() {
    let mut server = LanguageServer::start();
    initialize(&mut server, None, "utf-16");
    let uri = "untitled:scalar-resource-boundary";
    // Three opening quotes, LF, body, LF, and three closing quotes exactly
    // fill the native scalar limit. Reflow adds closing-line indentation.
    let body = "a".repeat(daml::Limits::default().max_scalar_bytes - 8);
    let source = format!("{{outer:{{literal:\"\"\"\n{body}\n\"\"\"\n}}}}");
    open(&mut server, uri, 1, &source);
    diagnostics(&server, uri, 1, 0);
    let response = format_request(&mut server, 2, uri);
    assert!(
        matches!(&response.response_result, Err(error) if error.code == -32803),
        "formatting must not propose text rejected by this server's parser limits"
    );

    server.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "{ok:1}"}]}),
    );
    diagnostics(&server, uri, 2, 0);
    let recovered = format_request(&mut server, 3, uri).response_result.unwrap();
    assert_eq!(recovered[0]["newText"], "{ ok: 1 }\n");
    server
        .request(4, "shutdown", Value::Null)
        .response_result
        .unwrap();
    server.notify("exit", json!({}));
    assert!(wait_for_exit(&mut server.child).success());
}

#[test]
fn invalid_initialization_delivers_error_and_allows_client_correction() {
    let mut server = LanguageServer::start();
    let before_init = format_request(&mut server, 1, "untitled:not-initialized");
    assert_eq!(before_init.response_result.unwrap_err().code, -32002);
    for capabilities in [json!([]), json!("invalid")] {
        let response = server.request(2, "initialize", json!({"capabilities": capabilities}));
        let error = response.response_result.unwrap_err();
        assert_eq!(error.code, -32602);
        assert_eq!(error.data, Some(json!({"retry": true})));
    }
    initialize(&mut server, None, "utf-16");
    open(
        &mut server,
        "untitled:corrected-initialization",
        1,
        "{ok:1}",
    );
    diagnostics(&server, "untitled:corrected-initialization", 1, 0);
    server
        .request(3, "shutdown", Value::Null)
        .response_result
        .unwrap();
    server.notify("exit", json!({}));
    assert!(wait_for_exit(&mut server.child).success());
}

#[test]
fn disconnected_initialization_returns_a_transport_error() {
    for method in ["initialize", "textDocument/formatting"] {
        let (server, client) = lsp_server::Connection::memory();
        client
            .sender
            .send(Request::new(1.into(), method.to_owned(), json!({"capabilities": {}})).into())
            .unwrap();
        drop(client);
        let error = daml::lsp::serve(server).unwrap_err();
        assert!(std::error::Error::source(&error).is_some());
    }
}
