//! Optional synchronous protocol adapter; native/core owners retain language policy.
//!
//! Documents remain syntax-only until a caller explicitly selects their context.
//! Session versions and context generations own diagnostics and watch metadata.

#[cfg(test)]
mod lifecycle_tests;

mod positions;
mod semantic;
mod uri;

use crate::Span;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response, ResponseError};
use lsp_types::{
    CompletionOptions, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    ExecuteCommandOptions, ExecuteCommandParams, HoverProviderCapability, InitializeError,
    InitializeParams, InitializeResult, OneOf, PublishDiagnosticsParams, SemanticTokensParams,
    ServerCapabilities, ServerInfo, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextEdit, Uri,
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Initialized,
        Notification as _, PublishDiagnostics,
    },
    request::{Formatting, Initialize, Request as _},
};
use positions::Encoding;
use semantic::{Context, Text};
use serde::de::DeserializeOwned;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    time::{Duration, Instant},
};

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
/// Accepting shutdown drops session state and permanently closes ordinary
/// dispatch. Until exit, every request receives InvalidRequest and other messages
/// are ignored. Terminal channel receives and sends share one 30-second deadline;
/// this does not interrupt a transport thread blocked in an OS stdout write.
///
/// # Errors
///
/// Invalid initialization parameters return a retryable protocol error and leave
/// initialization pending for the client. Returns a [`ServerError`] for a broken
/// initialization handshake, premature exit, transport failure, or a broken
/// shutdown handshake. The stdio executable terminates on such failures rather
/// than waiting indefinitely on an open input reader.
pub fn serve(connection: Connection) -> Result<(), ServerError> {
    // Upstream initialization helpers unwrap failed response sends. Keep this
    // phase fallible, and do not terminate before an InvalidParams reply flushes.
    let (id, params) =
        loop {
            let message = connection.receiver.recv().map_err(|error| {
                ServerError::caused("LSP initialization did not complete", error)
            })?;
            match message {
                Message::Request(request) if request.method == Initialize::METHOD => {
                    match serde_json::from_value::<InitializeParams>(request.params) {
                        Ok(params) => break (request.id, params),
                        Err(_) => {
                            let data = serde_json::to_value(InitializeError { retry: true })
                                .map_err(|error| {
                                    ServerError::caused(
                                        "LSP initialization error could not be encoded",
                                        error,
                                    )
                                })?;
                            send(
                                &connection,
                                Response {
                                    id: request.id,
                                    response_result: Err(ResponseError {
                                        code: ErrorCode::InvalidParams as i32,
                                        message: "invalid initialization parameters".to_owned(),
                                        data: Some(data),
                                    }),
                                }
                                .into(),
                            )?;
                        }
                    }
                }
                Message::Request(request) => send(
                    &connection,
                    Response::new_err(
                        request.id,
                        ErrorCode::ServerNotInitialized as i32,
                        "initialize request is required".to_owned(),
                    )
                    .into(),
                )?,
                Message::Notification(notification) if notification.method != "exit" => {}
                _ => return Err(ServerError::protocol("LSP initialization did not complete")),
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
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(vec!["!".into(), ".".into(), ":".into()]),
                resolve_provider: Some(false),
                ..Default::default()
            }),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            semantic_tokens_provider: Some(semantic::token_options().into()),
            execute_command_provider: Some(ExecuteCommandOptions {
                commands: vec![semantic::SET_CONTEXT.into()],
                ..Default::default()
            }),
            ..ServerCapabilities::default()
        },
        server_info: Some(ServerInfo {
            name: "daml-lsp".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    };
    let result = serde_json::to_value(result)
        .map_err(|error| ServerError::caused("LSP capabilities could not be encoded", error))?;
    send(&connection, Response::new_ok(id, result).into())?;
    match connection.receiver.recv() {
        Ok(Message::Notification(notification)) if notification.method == Initialized::METHOD => {}
        Ok(_) => {
            return Err(ServerError::protocol(
                "LSP initialized notification was not received",
            ));
        }
        Err(error) => {
            return Err(ServerError::caused(
                "LSP initialized notification was not received",
                error,
            ));
        }
    }
    let mut session = Session {
        encoding,
        documents: HashMap::new(),
        semantic_uris: HashSet::new(),
        next_generation: 0,
    };
    let (shutdown_id, deadline) = loop {
        let message = connection
            .receiver
            .recv()
            .map_err(|error| ServerError::caused("LSP input ended before shutdown", error))?;
        match message {
            Message::Request(request) if request.method == "shutdown" => {
                break (request.id, Instant::now() + Duration::from_secs(30));
            }
            Message::Request(request) => session.request(&connection, request)?,
            Message::Notification(notification) if notification.method == "exit" => {
                return Err(ServerError::protocol("LSP exit received before shutdown"));
            }
            Message::Notification(notification) => {
                session.notification(&connection, notification)?
            }
            Message::Response(_) => {} // This server issues no requests to the client.
        }
    };
    // Synchronous work before shutdown has finished; no session owner survives
    // into the terminal phase to dispatch late messages or publish language work.
    drop(session);
    send_terminal(
        &connection,
        Response::new_ok(shutdown_id, serde_json::Value::Null).into(),
        deadline,
    )?;
    await_exit(&connection, deadline)
}

fn await_exit(connection: &Connection, deadline: Instant) -> Result<(), ServerError> {
    loop {
        check_shutdown_deadline(deadline)?;
        let message = connection
            .receiver
            .recv_deadline(deadline)
            .map_err(|error| ServerError::caused("LSP shutdown handshake failed", error))?;
        // Ready traffic can succeed even with an expired channel deadline.
        check_shutdown_deadline(deadline)?;
        match message {
            Message::Request(request) => send_terminal(
                connection,
                Response::new_err(
                    request.id,
                    ErrorCode::InvalidRequest as i32,
                    "LSP session has shut down".to_owned(),
                )
                .into(),
                deadline,
            )?,
            Message::Notification(notification) if notification.method == "exit" => return Ok(()),
            Message::Notification(_) | Message::Response(_) => {}
        }
    }
}

fn send(connection: &Connection, message: Message) -> Result<(), ServerError> {
    connection
        .sender
        .send(message)
        .map_err(|error| ServerError::caused("LSP response could not be sent", error))
}

fn check_shutdown_deadline(deadline: Instant) -> Result<(), ServerError> {
    if Instant::now() >= deadline {
        return Err(ServerError::protocol("LSP shutdown handshake timed out"));
    }
    Ok(())
}

fn send_terminal(
    connection: &Connection,
    message: Message,
    deadline: Instant,
) -> Result<(), ServerError> {
    check_shutdown_deadline(deadline)?;
    connection
        .sender
        .send_deadline(message, deadline)
        .map_err(|error| ServerError::caused("LSP shutdown response could not be sent", error))?;
    check_shutdown_deadline(deadline)
}

struct OpenDocument {
    version: i32,
    text: Text,
    context: Option<Context>,
    generation: u32,
}

struct Session {
    encoding: Encoding,
    documents: HashMap<Uri, OpenDocument>,
    semantic_uris: HashSet<Uri>,
    next_generation: u32,
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
                    if let Some(document) = self.documents.remove(&uri) {
                        send(
                            connection,
                            Notification::new(
                                PublishDiagnostics::METHOD.into(),
                                PublishDiagnosticsParams::new(uri.clone(), Vec::new(), None),
                            )
                            .into(),
                        )?;
                        if document.generation != 0 {
                            semantic::dependencies(
                                connection,
                                uri,
                                document.version,
                                document.generation,
                                Vec::new(),
                            )?;
                        }
                        self.refresh(connection, None, None)?;
                    }
                }
            }
            "workspace/didChangeWatchedFiles" => {
                if self
                    .documents
                    .values()
                    .any(|document| document.context.is_some())
                    && let Some(params) =
                        notification_params::<DidChangeWatchedFilesParams>(notification)
                {
                    let bytes = params.changes.iter().try_fold(0usize, |sum, change| {
                        sum.checked_add(change.uri.as_str().len())
                    });
                    if params.changes.len() <= 10_000
                        && bytes.is_some_and(|bytes| bytes <= 8 * 1024 * 1024)
                        && params
                            .changes
                            .iter()
                            .any(|change| uri::file_path(&change.uri).is_ok())
                    {
                        self.refresh(connection, None, None)?;
                    }
                }
            }
            _ => {} // Unsupported configuration/cancellation notifications do not choose context.
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
            // Preserve syntax-only empty-update behavior; selected semantic peers
            // still need current version stamps for their authoritative overlays.
            if self
                .documents
                .values()
                .any(|document| document.context.is_some())
            {
                self.refresh(connection, Some(&text_document.uri), None)?;
            }
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
        let previous = self.documents.remove(&uri);
        let (context, generation) = previous.map_or((None, 0), |document| {
            (document.context, document.generation)
        });
        let text = Text::parse(source, context.as_ref().is_some_and(Context::data));
        self.documents.insert(
            uri.clone(),
            OpenDocument {
                version,
                text,
                context,
                generation,
            },
        );
        self.refresh(connection, Some(&uri), None)?;
        Ok(())
    }

    fn select_context(
        &mut self,
        params: ExecuteCommandParams,
    ) -> Result<(Uri, u32), semantic::RequestFailure> {
        if params.command != semantic::SET_CONTEXT || params.arguments.len() != 1 {
            return Err(semantic::invalid(
                "daml.setContext requires exactly one context object",
            ));
        }
        let argument = params
            .arguments
            .into_iter()
            .next()
            .expect("one checked argument");
        let (uri, context) = Context::parse(argument, &self.documents)?;
        if context.is_some()
            && self.documents[&uri].context.is_none()
            && self
                .documents
                .values()
                .filter(|document| document.context.is_some())
                .count()
                >= semantic::MAX_CONTEXTS
        {
            return Err(semantic::failed("active semantic context limit exceeded"));
        }
        let generation = self.next_generation.checked_add(1).ok_or_else(|| {
            semantic::failed("context generation space exhausted; restart the connection")
        })?;
        let mut document = self
            .documents
            .remove(&uri)
            .expect("context admission established open document");
        document.text = document
            .text
            .reparse(context.as_ref().is_some_and(Context::data));
        document.context = context;
        document.generation = generation;
        self.documents.insert(uri.clone(), document);
        self.next_generation = generation;
        Ok((uri, generation))
    }

    fn request(&mut self, connection: &Connection, request: Request) -> Result<(), ServerError> {
        if request.method == "workspace/executeCommand" {
            let result = serde_json::from_value::<ExecuteCommandParams>(request.params)
                .map_err(|_| semantic::invalid("invalid execute-command parameters"))
                .and_then(|params| self.select_context(params));
            let selected = result.as_ref().ok().map(|(uri, _)| uri.clone());
            let response = match result {
                Ok((_, generation)) => {
                    Response::new_ok(request.id, serde_json::json!({"generation":generation}))
                }
                Err((code, message)) => Response::new_err(request.id, code as i32, message),
            };
            // Acknowledgment precedes notifications; generation also protects
            // clients whose asynchronous handlers observe them before continuation.
            send(connection, response.into())?;
            if let Some(uri) = selected {
                self.refresh(connection, Some(&uri), None)?;
                let document = &self.documents[&uri];
                if document.context.is_none() {
                    semantic::dependencies(
                        connection,
                        uri.clone(),
                        document.version,
                        document.generation,
                        Vec::new(),
                    )?;
                }
            }
            return Ok(());
        }
        let result = match request.method.as_str() {
            Formatting::METHOD => {
                serde_json::from_value::<DocumentFormattingParams>(request.params)
                    .map_err(|_| semantic::invalid("invalid document formatting parameters"))
                    .and_then(|params| self.formatting(params))
                    .and_then(|edits| {
                        serde_json::to_value(edits).map_err(|_| {
                            (
                                ErrorCode::InternalError,
                                "formatting result could not be encoded".into(),
                            )
                        })
                    })
            }
            "textDocument/semanticTokens/full" => {
                serde_json::from_value::<SemanticTokensParams>(request.params)
                    .map_err(|_| semantic::invalid("invalid semantic token parameters"))
                    .and_then(|params| {
                        self.documents
                            .get(&params.text_document.uri)
                            .ok_or_else(|| semantic::invalid("document is not open"))?
                            .text
                            .tokens(self.encoding)
                    })
                    .and_then(|tokens| {
                        serde_json::to_value(tokens).map_err(|_| {
                            (
                                ErrorCode::InternalError,
                                "semantic token result could not be encoded".into(),
                            )
                        })
                    })
            }
            "textDocument/completion" | "textDocument/hover" | "textDocument/definition" => {
                match serde_json::from_value::<TextDocumentPositionParams>(request.params) {
                    Ok(params) => self.provider(connection, &request.method, params)?,
                    Err(_) => Err(semantic::invalid("invalid semantic position parameters")),
                }
            }
            _ => Err((
                ErrorCode::MethodNotFound,
                "request method is not supported".to_owned(),
            )),
        };
        let response = match result {
            Ok(value) => Response::new_ok(request.id, value),
            Err((code, message)) => Response::new_err(request.id, code as i32, message),
        };
        send(connection, response.into())
    }

    fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Vec<TextEdit>, semantic::RequestFailure> {
        let document = self
            .documents
            .get(&params.text_document.uri)
            .ok_or_else(|| semantic::invalid("document is not open"))?;
        let source = document
            .text
            .source()
            .ok_or_else(|| semantic::failed("cannot format a document with parser diagnostics"))?;
        let output = document.text.format()?;
        if output == source {
            return Ok(Vec::new());
        }
        Ok(vec![TextEdit {
            range: self.encoding.range(
                source,
                Span {
                    start: 0,
                    end: source.len(),
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
