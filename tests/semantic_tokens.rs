//! Consumer-visible semantic token checks through the actual stdio server.
#![cfg(feature = "lsp")]

mod support;

use lsp_server::{Message, Notification, Request, RequestId, Response, ResponseError};
use serde_json::{Value, json};
use std::{
    io::{self, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_TOKENS: usize = 100_000;

struct Server {
    child: Child,
    outgoing: Option<Sender<(Message, Sender<io::Result<()>>)>>,
    writer: Option<JoinHandle<()>>,
    incoming: Receiver<Message>,
    reader: Option<JoinHandle<()>>,
    next_id: i32,
    types: Vec<String>,
    modifiers: Vec<String>,
    encoding: &'static str,
}

impl Server {
    fn start(encoding: &'static str) -> Self {
        let mut child = Command::new(support::binary("daml-lsp", env!("CARGO_BIN_EXE_daml-lsp")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start actual daml-lsp");
        let mut stdin = child.stdin.take().expect("server stdin");
        let stdout = child.stdout.take().expect("server stdout");
        let (outgoing, messages) = mpsc::channel::<(Message, Sender<io::Result<()>>)>();
        let writer = thread::spawn(move || {
            while let Ok((message, completed)) = messages.recv() {
                let result = message.write(&mut stdin);
                let failed = result.is_err();
                if completed.send(result).is_err() || failed {
                    break;
                }
            }
        });
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
            outgoing: Some(outgoing),
            writer: Some(writer),
            incoming,
            reader: Some(reader),
            next_id: 1,
            types: Vec::new(),
            modifiers: Vec::new(),
            encoding,
        };
        let initialized = server.request(
            "initialize",
            json!({"processId":null,"capabilities":{"general":{"positionEncodings":[encoding]}}}),
        );
        let caps = initialized.response_result.expect("initialize succeeds");
        assert_eq!(caps["capabilities"]["positionEncoding"], encoding);
        let provider = &caps["capabilities"]["semanticTokensProvider"];
        assert_eq!(provider["full"], true);
        assert!(provider["range"].is_null() || provider["range"] == false);
        let legend = &provider["legend"];
        server.types = legend["tokenTypes"]
            .as_array()
            .expect("token type legend")
            .iter()
            .map(|entry| entry.as_str().expect("string token type").to_owned())
            .collect();
        server.modifiers = legend["tokenModifiers"]
            .as_array()
            .expect("modifier legend")
            .iter()
            .map(|entry| entry.as_str().expect("string modifier").to_owned())
            .collect();
        for token_type in [
            "property",
            "namespace",
            "type",
            "string",
            "number",
            "keyword",
            "comment",
            "regexp",
            "operator",
        ] {
            assert!(
                server.types.iter().any(|value| value == token_type),
                "missing {token_type} in {:?}",
                server.types
            );
        }
        assert!(server.types.iter().all(|value| {
            [
                "property",
                "namespace",
                "type",
                "string",
                "number",
                "keyword",
                "comment",
                "regexp",
                "operator",
            ]
            .contains(&value.as_str())
        }));
        assert_eq!(
            server.types.len(),
            9,
            "no editor-specific token types: {:?}",
            server.types
        );
        assert_eq!(server.modifiers, ["defaultLibrary"]);
        server.notify("initialized", json!({}));
        server
    }

    fn send(&mut self, message: Message) {
        let (completed, result) = mpsc::channel();
        self.outgoing
            .as_ref()
            .expect("open client transport")
            .send((message, completed))
            .expect("running client writer");
        result
            .recv_timeout(TIMEOUT)
            .expect("write LSP message before deadline")
            .expect("write LSP message");
    }

    fn receive(&self, deadline: Instant) -> Message {
        self.incoming
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("server response before deadline")
    }

    fn request(&mut self, method: &str, params: Value) -> Response {
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
                Message::Notification(_) => {}
                other => panic!("unexpected server message: {other:?}"),
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(Notification::new(method.to_owned(), params).into());
    }

    fn open(&mut self, uri: &str, version: i32, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"daml","version":version,"text":text}}),
        );
    }

    fn change(&mut self, uri: &str, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":text}]}),
        );
    }

    fn tokens(&mut self, uri: &str) -> Result<Value, ResponseError> {
        self.request(
            "textDocument/semanticTokens/full",
            json!({"textDocument":{"uri":uri}}),
        )
        .response_result
    }

    fn finish(mut self) {
        let response = self.request("shutdown", Value::Null);
        assert!(
            response.response_result.is_ok(),
            "shutdown: {:?}",
            response.response_result
        );
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
                Err(mpsc::RecvTimeoutError::Timeout) => panic!("server did not close stdout"),
            }
        }
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("reap server") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit before deadline"
            );
            thread::yield_now();
        };
        assert!(status.success(), "server exited with {status}");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.outgoing.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[derive(Debug)]
struct Token {
    line: usize,
    start: usize,
    end: usize,
    kind: String,
    modifiers: Vec<String>,
    text: String,
}

fn byte_offset(line: &str, units: usize, encoding: &str) -> Option<usize> {
    let mut offset = 0;
    if units == 0 {
        return Some(0);
    }
    for (byte, character) in line.char_indices() {
        offset += match encoding {
            "utf-8" => character.len_utf8(),
            "utf-16" => character.len_utf16(),
            "utf-32" => 1,
            _ => unreachable!(),
        };
        if offset == units {
            return Some(byte + character.len_utf8());
        }
        if offset > units {
            return None;
        }
    }
    (offset == units).then_some(line.len())
}

fn decode(server: &Server, source: &str, value: &Value) -> Vec<Token> {
    let data: Vec<u32> = value["data"]
        .as_array()
        .expect("semantic token data array")
        .iter()
        .map(|item| {
            item.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .expect("u32 token field")
        })
        .collect();
    assert_eq!(data.len() % 5, 0, "five integers per token");
    let mut physical_lines = Vec::new();
    let bytes = source.as_bytes();
    let mut line_start = 0;
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\r' || bytes[cursor] == b'\n' {
            physical_lines.push(&source[line_start..cursor]);
            if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                cursor += 1;
            }
            cursor += 1;
            line_start = cursor;
        } else {
            cursor += 1;
        }
    }
    if line_start < source.len() || physical_lines.is_empty() {
        physical_lines.push(&source[line_start..]);
    }
    let mut tokens = Vec::new();
    let (mut line, mut start) = (0usize, 0usize);
    for item in data.chunks_exact(5) {
        line += item[0] as usize;
        start = if item[0] == 0 {
            start + item[1] as usize
        } else {
            item[1] as usize
        };
        let end = start + item[2] as usize;
        let kind = server
            .types
            .get(item[3] as usize)
            .expect("advertised type index")
            .clone();
        let modifiers = server
            .modifiers
            .iter()
            .enumerate()
            .filter_map(|(i, name)| ((item[4] & (1 << i)) != 0).then_some(name.clone()))
            .collect();
        assert!(item[2] > 0, "positive token length");
        assert!(line < physical_lines.len(), "token line exists: {line}");
        let width = match server.encoding {
            "utf-8" => physical_lines[line].len(),
            "utf-16" => physical_lines[line].encode_utf16().count(),
            "utf-32" => physical_lines[line].chars().count(),
            _ => unreachable!(),
        };
        assert!(
            end <= width,
            "token does not include newline: {line}:{start}..{end} width {width}"
        );
        if let Some(previous) = tokens.last() {
            let previous: &Token = previous;
            assert!(
                previous.line < line || (previous.line == line && previous.end <= start),
                "sorted, non-overlapping: {previous:?} then {line}:{start}..{end}"
            );
        }
        let line_text = physical_lines[line];
        let byte_start = byte_offset(line_text, start, server.encoding)
            .expect("token begins on encoding boundary");
        let byte_end =
            byte_offset(line_text, end, server.encoding).expect("token ends on encoding boundary");
        let text = line_text[byte_start..byte_end].to_owned();
        tokens.push(Token {
            line,
            start,
            end,
            kind,
            modifiers,
            text,
        });
    }
    tokens
}

fn expected(tokens: &[Token], source: &str, spelling: &str, kind: &str, default_library: bool) {
    let candidates: Vec<_> = source
        .lines()
        .enumerate()
        .filter_map(|(line, text)| {
            text.find(spelling)
                .map(|start| (line, start, start + spelling.len()))
        })
        .collect();
    assert_eq!(
        candidates.len(),
        1,
        "fixture spelling must identify one source range: {spelling:?}"
    );
    let (line, start, end) = candidates[0];
    let token = tokens
        .iter()
        .find(|token| token.line == line && token.start <= start && token.end >= end)
        .unwrap_or_else(|| panic!("no token covering {kind} source {spelling:?}; got {tokens:?}"));
    assert_eq!(token.kind, kind, "role of {spelling:?}");
    assert_eq!(
        token.modifiers.contains(&"defaultLibrary".to_owned()),
        default_library,
        "defaultLibrary modifier of {spelling:?}"
    );
}

fn assert_error_code(error: ResponseError, code: i32) {
    assert_eq!(error.code, code, "error response: {error:?}");
}

#[test]
fn standard_legend_parser_roles_and_opaque_literals() {
    let source = "\u{feff}{\n  value: !app.String\n  count: 42\n  enabled: true\n  schedule: \"0 12 * * *\"\n  dotted.name: \"a\\\" # : { opaque }\"\n  pattern: \"\\\\d+\"\n}\n# note \" : { opaque }\n";
    daml::parse(source).expect("standard role fixture is admitted data syntax");
    let mut server = Server::start("utf-16");
    let uri = "file:///semantic-tokens.daml";
    server.open(uri, 1, source);
    let result = server.tokens(uri).expect("tokens for open source");
    let tokens = decode(&server, source, &result);
    expected(&tokens, source, "value", "property", false);
    expected(&tokens, source, "app", "namespace", false);
    expected(&tokens, source, "String", "type", false);
    expected(&tokens, source, "count", "property", false);
    expected(&tokens, source, "42", "number", false);
    expected(&tokens, source, "enabled", "property", false);
    expected(&tokens, source, "true", "keyword", false);
    expected(&tokens, source, "schedule", "property", false);
    expected(&tokens, source, "0 12 * * *", "string", false);
    expected(&tokens, source, "dotted.name", "property", false);
    expected(&tokens, source, "a\\\" # : { opaque }", "string", false);
    expected(&tokens, source, "\\\\d+", "string", false);
    expected(&tokens, source, "# note \" : { opaque }", "comment", false);
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == "operator" && token.text == "!"),
        "assertion sigil remains separate from names"
    );
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == "operator" && token.line == 1 && token.text == "."),
        "qualified name separator remains an operator"
    );
    let literal_line = source.lines().nth(5).unwrap();
    let literal_start = literal_line.find("a\\\" # : { opaque }").unwrap();
    let literal_end = literal_start + "a\\\" # : { opaque }".len();
    assert!(
        !tokens.iter().any(|token| token.line == 5
            && token.kind == "operator"
            && token.start < literal_end
            && token.end > literal_start),
        "literal punctuation remains opaque: {tokens:?}"
    );
    server.finish();
}

#[test]
fn full_result_uses_current_document_across_changes_and_reopen() {
    let mut server = Server::start("utf-8");
    let uri = "file:///semantic-lifecycle.daml";
    for source in [
        "{name: 1}\n",
        "{fresh: 2}\n",
        "{obsolete: 3}\n",
        "{again: 4}\n",
    ] {
        daml::parse(source).expect("lifecycle fixture is admitted data syntax");
    }
    server.open(uri, 1, "{name: 1}\n");
    let old = server.tokens(uri).expect("initial tokens");
    let decoded = decode(&server, "{name: 1}\n", &old);
    expected(&decoded, "{name: 1}\n", "name", "property", false);
    expected(&decoded, "{name: 1}\n", ":", "operator", false);
    expected(&decoded, "{name: 1}\n", "1", "number", false);
    server.change(uri, 3, "{fresh: 2}\n");
    server.change(uri, 2, "{obsolete: 3}\n");
    let current = server
        .tokens(uri)
        .expect("tokens after newer then stale change");
    let decoded = decode(&server, "{fresh: 2}\n", &current);
    expected(&decoded, "{fresh: 2}\n", "fresh", "property", false);
    expected(&decoded, "{fresh: 2}\n", "2", "number", false);
    assert!(
        !decoded.iter().any(|t| t.start == 1 && t.end == 9),
        "stale content not used"
    );
    server.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
    assert_error_code(server.tokens(uri).unwrap_err(), -32602);
    server.open(uri, 4, "{again: 4}\n");
    let reopened = server.tokens(uri).expect("reopened document");
    expected(
        &decode(&server, "{again: 4}\n", &reopened),
        "{again: 4}\n",
        "again",
        "property",
        false,
    );
    assert_error_code(server.tokens("file:///unknown.daml").unwrap_err(), -32602);
    assert_error_code(
        server
            .request(
                "textDocument/semanticTokens/full",
                json!({"textDocument":{"uri":7}}),
            )
            .response_result
            .unwrap_err(),
        -32602,
    );
    server.finish();
}

#[test]
fn encodings_multiline_boundaries_and_bom_coordinates() {
    for encoding in ["utf-8", "utf-16", "utf-32"] {
        let source = "\u{feff}!app.𐐀Service {\r\n\tname: \"\"\"\r\n😀a\r\nb\rc\r\n\"\"\"\r  count: 7\t# tail\r}";
        daml::parse(source).expect("mixed-line-ending triple fixture is admitted data syntax");
        let mut server = Server::start(encoding);
        let uri = "file:///semantic-unicode.daml";
        server.open(uri, 1, source);
        let result = server.tokens(uri).expect("unicode tokens");
        let tokens = decode(&server, source, &result);
        let name = tokens
            .iter()
            .find(|token| token.kind == "type" && token.text == "𐐀Service")
            .expect("type token selects original supplementary name");
        assert_eq!(
            (name.line, name.start, name.end),
            match encoding {
                "utf-8" => (0, 8, 19),
                "utf-16" => (0, 6, 15),
                "utf-32" => (0, 6, 14),
                _ => unreachable!(),
            },
            "BOM and supplementary name use negotiated character units"
        );
        assert!(name.modifiers.is_empty(), "qualified name is not a builtin");
        let emoji = tokens
            .iter()
            .find(|token| token.kind == "string" && token.text.contains("😀a"))
            .expect("string token selects original supplementary source");
        assert_eq!(emoji.text, "😀a");
        assert_eq!(emoji.line, 2);
        assert_eq!(emoji.start, 0);
        assert_eq!(
            emoji.end,
            match encoding {
                "utf-8" => 5,
                "utf-16" => 3,
                "utf-32" => 2,
                _ => unreachable!(),
            }
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == "string" && token.line == 3 && token.text == "b")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == "string" && token.line == 4 && token.text == "c")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == "number" && token.text == "7")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == "comment" && token.text == "# tail")
        );
        assert!(
            tokens
                .iter()
                .all(|token| !token.text.contains('\r') && !token.text.contains('\n')),
            "single-line token source slices only"
        );
        server.finish();
    }
}

#[test]
fn native_regex_payloads_are_regexp_not_strings() {
    let source = r#"!String { pattern: !Regex """
    (?x)a|abc # raw comment
    """
}
"#;
    daml::parse_definition(source).expect("raw Regex fixture is admitted native syntax");
    let mut server = Server::start("utf-8");
    let uri = "file:///semantic-native.daml";
    server.open(uri, 1, source);
    let result = server.tokens(uri).expect("native tokens");
    let tokens = decode(&server, source, &result);
    expected(&tokens, source, "String", "type", true);
    expected(&tokens, source, "Regex", "type", true);
    expected(&tokens, source, "(?x)a|abc # raw comment", "regexp", false);
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == "operator" && token.text == "!"),
        "native constructor sigils remain operators"
    );
    server.finish();
}

#[test]
fn malformed_prefix_limits_and_recovery_never_return_cached_success() {
    let mut server = Server::start("utf-16");
    let uri = "file:///semantic-errors.daml";
    for source in ["{valid: 1}", "{recovered: 9}"] {
        daml::parse(source).expect("recovery fixture is admitted data syntax");
    }
    server.open(uri, 1, "{valid: 1}");
    let rejected = "x".repeat(MAX_INPUT + 1);
    server.change(uri, 2, &rejected);
    assert_error_code(server.tokens(uri).unwrap_err(), -32803);
    let recovered = "{recovered: 9}";
    server.change(uri, 3, recovered);
    let recovered_tokens = server.tokens(uri).expect("recovered current source");
    expected(
        &decode(&server, recovered, &recovered_tokens),
        recovered,
        "recovered",
        "property",
        false,
    );
    let incomplete = "{\nprefix: 12\ntrailing: \"unterminated";
    server.change(uri, 4, incomplete);
    let partial = server
        .tokens(uri)
        .expect("grounded parser prefix and unfinished string");
    let tokens = decode(&server, incomplete, &partial);
    expected(&tokens, incomplete, "prefix", "property", false);
    expected(&tokens, incomplete, "12", "number", false);
    expected(&tokens, incomplete, "unterminated", "string", false);
    let unparsed_suffix = "{\nprefix: 12\n?\nnotParsed: 99\n}";
    server.change(uri, 5, unparsed_suffix);
    let partial = server
        .tokens(uri)
        .expect("grounded prefix before syntax error");
    let tokens = decode(&server, unparsed_suffix, &partial);
    expected(&tokens, unparsed_suffix, "prefix", "property", false);
    expected(&tokens, unparsed_suffix, "12", "number", false);
    assert!(
        tokens.iter().all(|token| token.line < 3),
        "unparsed suffix receives no invented roles: {tokens:?}"
    );
    server.finish();
}

#[test]
fn token_exhaustion_is_request_failed_not_partial_result() {
    let mut server = Server::start("utf-8");
    let uri = "file:///semantic-exhaustion.daml";
    // Comments do not consume AST nodes: these fixtures isolate the output-token
    // budget from the independent parser-node budget.
    let mut source = "#\n".repeat(MAX_TOKENS - 1);
    source.push_str("null");
    daml::parse_definition(&source)
        .expect("exact token-boundary fixture is admitted native syntax");
    server.open(uri, 1, &source);
    let boundary = server
        .tokens(uri)
        .expect("exactly 100,000 tokens are allowed");
    let data = boundary["data"].as_array().expect("token-boundary data");
    assert_eq!(
        data.len(),
        MAX_TOKENS * 5,
        "boundary output is not truncated"
    );
    let comment = server
        .types
        .iter()
        .position(|kind| kind == "comment")
        .unwrap() as u64;
    let keyword = server
        .types
        .iter()
        .position(|kind| kind == "keyword")
        .unwrap() as u64;
    for (index, token) in data.chunks_exact(5).enumerate() {
        let is_null = index == MAX_TOKENS - 1;
        assert_eq!(token[0].as_u64(), Some(u64::from(index != 0)));
        assert_eq!(token[1].as_u64(), Some(0));
        assert_eq!(token[2].as_u64(), Some(if is_null { 4 } else { 1 }));
        assert_eq!(
            token[3].as_u64(),
            Some(if is_null { keyword } else { comment })
        );
        assert_eq!(token[4].as_u64(), Some(0));
    }
    source.push_str("\n#");
    daml::parse_definition(&source).expect("100,001-token fixture is still admitted native syntax");
    server.change(uri, 2, &source);
    assert_error_code(server.tokens(uri).unwrap_err(), -32803);
    server.change(uri, 3, "null");
    let recovered = server.tokens(uri).expect("recover after output exhaustion");
    expected(
        &decode(&server, "null", &recovered),
        "null",
        "null",
        "keyword",
        false,
    );
    server.finish();
}
