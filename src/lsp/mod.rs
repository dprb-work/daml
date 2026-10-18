//! Optional synchronous LSP adapter around DAML's checked parser and formatter.
//!
//! This module is enabled by the `lsp` feature. It owns protocol/session state,
//! not source discovery: every document comes from a client notification. Schema
//! folder roots and relative-path policy are deliberately outside this adapter.

mod positions;

use crate::{Document, FormatOptions, Limits, Span, format, parser};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, InitializeParams, InitializeResult, OneOf,
    PublishDiagnosticsParams, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextEdit, Uri,
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
        PublishDiagnostics,
    },
    request::{Formatting, Request as _},
};
use positions::Encoding;
use serde::de::DeserializeOwned;
use std::{collections::HashMap, error::Error, fmt};

type Cause = Box<dyn Error + Send + Sync>;

/// A failed protocol/session operation with a safe display context.
///
/// The underlying cause is available through [`Error::source`]. Display does not
/// echo incoming messages or document payloads into editor logs.
#[derive(Debug)]
pub struct ServerError {
    context: &'static str,
    cause: Option<Cause>,
}

impl ServerError {
    fn caused(context: &'static str, cause: impl Into<Cause>) -> Self {
        Self {
            context,
            cause: Some(cause.into()),
        }
    }

    fn protocol(context: &'static str) -> Self {
        Self {
            context,
            cause: None,
        }
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.context)
    }
}

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn Error + 'static))
    }
}

/// Serve one LSP connection until a standard shutdown/exit handshake completes.
///
/// The caller owns transport threads and process termination. This function
/// creates no threads or filesystem watchers. Updates and requests are handled
/// in transport order by one owner; diagnostics carry accepted document versions.
/// Formatting responses describe the latest processed revision. Clients must
/// discard edits if their buffer changes after requesting formatting.
///
/// # Errors
///
/// Returns a [`ServerError`] for failed initialization, premature exit, transport
/// failure, or a broken shutdown handshake. The stdio executable terminates on
/// such failures rather than waiting indefinitely on an open input reader.
pub fn serve(connection: Connection) -> Result<(), ServerError> {
    let (id, params) = connection
        .initialize_start()
        .map_err(|error| ServerError::caused("LSP initialization did not complete", error))?;
    let params: InitializeParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(error) => {
            send(
                &connection,
                Response::new_err(
                    id,
                    ErrorCode::InvalidParams as i32,
                    "invalid initialization parameters".to_owned(),
                )
                .into(),
            )?;
            return Err(ServerError::caused(
                "invalid LSP initialization parameters",
                error,
            ));
        }
    };
    let encoding = Encoding::negotiate(
        params
            .capabilities
            .general
            .as_ref()
            .and_then(|general| general.position_encodings.as_deref()),
    );
    drop(params); // No workspace roots or initialization settings are retained.
    let result = InitializeResult {
        capabilities: ServerCapabilities {
            position_encoding: Some(encoding.kind()),
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::FULL),
                    ..TextDocumentSyncOptions::default()
                },
            )),
            document_formatting_provider: Some(OneOf::Left(true)),
            ..ServerCapabilities::default()
        },
        server_info: Some(ServerInfo {
            name: "daml-lsp".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    };
    let result = serde_json::to_value(result)
        .map_err(|error| ServerError::caused("LSP capabilities could not be encoded", error))?;
    connection.initialize_finish(id, result).map_err(|error| {
        ServerError::caused("LSP initialized notification was not received", error)
    })?;
    let mut session = Session {
        encoding,
        documents: HashMap::new(),
    };
    loop {
        let message = connection
            .receiver
            .recv()
            .map_err(|error| ServerError::caused("LSP input ended before shutdown", error))?;
        match message {
            Message::Request(request) => {
                if connection
                    .handle_shutdown(&request)
                    .map_err(|error| ServerError::caused("LSP shutdown handshake failed", error))?
                {
                    return Ok(());
                }
                session.request(&connection, request)?;
            }
            Message::Notification(notification) if notification.method == "exit" => {
                return Err(ServerError::protocol("LSP exit received before shutdown"));
            }
            Message::Notification(notification) => {
                session.notification(&connection, notification)?
            }
            Message::Response(_) => {} // This server issues no requests to the client.
        }
    }
}

fn send(connection: &Connection, message: Message) -> Result<(), ServerError> {
    connection
        .sender
        .send(message)
        .map_err(|error| ServerError::caused("LSP response could not be sent", error))
}

struct OpenDocument {
    version: i32,
    // A rejected version retains no source buffer. Its diagnostic has been published.
    parsed: Option<Document>,
}

struct Session {
    encoding: Encoding,
    documents: HashMap<Uri, OpenDocument>,
}

impl Session {
    fn notification(
        &mut self,
        connection: &Connection,
        notification: Notification,
    ) -> Result<(), ServerError> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Some(params) = notification_params::<DidOpenTextDocumentParams>(notification)
                {
                    let document = params.text_document;
                    if !self.documents.contains_key(&document.uri) {
                        self.replace(connection, document.uri, document.version, document.text)?;
                    }
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Some(params) =
                    notification_params::<DidChangeTextDocumentParams>(notification)
                {
                    self.change(connection, params)?;
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Some(params) =
                    notification_params::<DidCloseTextDocumentParams>(notification)
                {
                    let uri = params.text_document.uri;
                    if self.documents.remove(&uri).is_some() {
                        let params = PublishDiagnosticsParams::new(uri, Vec::new(), None);
                        send(
                            connection,
                            Notification::new(PublishDiagnostics::METHOD.to_owned(), params).into(),
                        )?;
                    }
                }
            }
            _ => {} // Includes unsupported workspace/configuration and cancellation notifications.
        }
        Ok(())
    }

    fn change(
        &mut self,
        connection: &Connection,
        params: DidChangeTextDocumentParams,
    ) -> Result<(), ServerError> {
        let DidChangeTextDocumentParams {
            text_document,
            mut content_changes,
        } = params;
        let Some(current) = self.documents.get(&text_document.uri) else {
            return Ok(());
        };
        if text_document.version <= current.version {
            return Ok(());
        }
        if content_changes.iter().any(|change| change.range.is_some()) {
            eprintln!("daml-lsp: ignoring ranged changes; full synchronization is required");
            return Ok(());
        }
        if let Some(change) = content_changes.pop() {
            // Each full replacement supersedes the preceding replacements. Deprecated
            // rangeLength has no meaning without a range and does not alter the text.
            self.replace(
                connection,
                text_document.uri,
                text_document.version,
                change.text,
            )
        } else {
            self.documents
                .get_mut(&text_document.uri)
                .expect("document remains open")
                .version = text_document.version;
            Ok(())
        }
    }

    fn replace(
        &mut self,
        connection: &Connection,
        uri: Uri,
        version: i32,
        source: String,
    ) -> Result<(), ServerError> {
        let limits = Limits::default();
        let oversized = source.len() > limits.max_input_bytes;
        let (parsed, diagnostics) = match parser::parse(&source, limits) {
            Ok(root) => (Some(Document { source, root }), Vec::new()),
            Err(error) => {
                // Over-limit text is discarded; its document-level diagnostic does
                // not require a position table for arbitrarily large client input.
                let range = if oversized {
                    Range::default()
                } else {
                    self.encoding.range(&source, error.span)
                };
                let diagnostic = Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("daml".to_owned()),
                    message: error.to_string(),
                    ..Diagnostic::default()
                };
                (None, vec![diagnostic])
            }
        };
        let params = PublishDiagnosticsParams::new(uri, diagnostics, Some(version));
        let notification = Notification::new(PublishDiagnostics::METHOD.to_owned(), &params);
        self.documents
            .insert(params.uri, OpenDocument { version, parsed });
        send(connection, notification.into())
    }

    fn request(&self, connection: &Connection, request: Request) -> Result<(), ServerError> {
        let response = if request.method == Formatting::METHOD {
            match serde_json::from_value::<DocumentFormattingParams>(request.params) {
                Ok(params) => match self.formatting(params) {
                    Ok(edits) => Response::new_ok(request.id, edits),
                    Err((code, message)) => Response::new_err(request.id, code as i32, message),
                },
                Err(_) => Response::new_err(
                    request.id,
                    ErrorCode::InvalidParams as i32,
                    "invalid document formatting parameters".to_owned(),
                ),
            }
        } else {
            Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                "request method is not supported".to_owned(),
            )
        };
        send(connection, response.into())
    }

    fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Vec<TextEdit>, (ErrorCode, String)> {
        let state = self
            .documents
            .get(&params.text_document.uri)
            .ok_or_else(|| (ErrorCode::InvalidParams, "document is not open".to_owned()))?;
        let document = state.parsed.as_ref().ok_or_else(|| {
            (
                ErrorCode::RequestFailed,
                "cannot format a document with parser diagnostics".to_owned(),
            )
        })?;
        let output = format(document, FormatOptions::default())
            .map_err(|error| (ErrorCode::RequestFailed, error.to_string()))?;
        if output == document.source() {
            return Ok(Vec::new());
        }
        // Layout can grow scalar source spans as well as total bytes. Admit the
        // proposed text through the same parser before asking the client to apply it.
        parser::parse(&output, Limits::default()).map_err(|error| {
            (
                ErrorCode::RequestFailed,
                format!("formatted document rejected by parser: {error}"),
            )
        })?;
        Ok(vec![TextEdit {
            range: self.encoding.range(
                document.source(),
                Span {
                    start: 0,
                    end: document.source().len(),
                },
            ),
            new_text: output,
        }])
    }
}

fn notification_params<T: DeserializeOwned>(notification: Notification) -> Option<T> {
    match serde_json::from_value(notification.params) {
        Ok(params) => Some(params),
        Err(_) => {
            eprintln!(
                "daml-lsp: ignoring malformed {} notification",
                notification.method
            );
            None
        }
    }
}
