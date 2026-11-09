//! Explicit document semantics; language rules remain in the native/core owners.

// lsp-types 0.97 Uri's Eq/Hash use only immutable as_str(), not fluent-uri's
// interior authority cache. Keep parsed protocol identities, not duplicate keys.
#![expect(
    clippy::mutable_key_type,
    reason = "Uri's cached metadata cannot change its string identity"
)]

mod providers;

use super::{
    OpenDocument, ServerError, Session,
    positions::Encoding,
    send,
    uri::{file_path, path_uri},
};
use crate::{
    Bundle, Definition, Document, ErrorOrigin, FormatOptions, Limits, NativeError, ParseError,
    ResolveError,
    native::tooling::{EditorCompilation, EditorIndex},
    parser,
};
use lsp_server::{Connection, ErrorCode, Notification};
use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location,
    PublishDiagnosticsParams, Range, Uri,
    notification::{Notification as _, PublishDiagnostics},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
};

pub(super) const SET_CONTEXT: &str = "daml.setContext";
pub(super) const DEPENDENCIES: &str = "daml/dependencies";
pub(super) const MAX_CONTEXTS: usize = 64;
const MAX_FILES: usize = 10_000;
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;
pub(super) type RequestFailure = (ErrorCode, String);

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Syntax,
    Configuration,
    Definition,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextArgument {
    uri: Uri,
    kind: Kind,
    #[serde(default, deserialize_with = "present_option")]
    bundle_root: Option<Uri>,
    #[serde(default, deserialize_with = "present_option")]
    expected: Option<String>,
}

// Missing options are distinct from present null/non-value options.
fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub(super) struct Context {
    definition: bool,
    bundle_root: Option<PathBuf>,
    expected: Option<String>,
}

impl Context {
    pub(super) fn parse(
        value: serde_json::Value,
        documents: &HashMap<Uri, OpenDocument>,
    ) -> Result<(Uri, Option<Self>), RequestFailure> {
        let argument: ContextArgument =
            serde_json::from_value(value).map_err(|_| invalid("invalid DAML context object"))?;
        if argument.uri.as_str().len() > super::uri::MAX_URI_BYTES {
            return Err(invalid("document URI exceeds the editor limit"));
        }
        if !documents.contains_key(&argument.uri) {
            return Err(invalid("document is not open"));
        }
        let context = match argument.kind {
            Kind::Syntax => {
                if argument.bundle_root.is_some() || argument.expected.is_some() {
                    return Err(invalid(
                        "syntax context does not accept bundleRoot or expected",
                    ));
                }
                None
            }
            Kind::Definition | Kind::Configuration => {
                let definition = argument.kind == Kind::Definition;
                if definition && argument.expected.is_some() {
                    return Err(invalid("definition context does not accept expected"));
                }
                if definition && file_path(&argument.uri).is_err() {
                    return Err(invalid("definition context needs a local file identity"));
                }
                let bundle_root = argument
                    .bundle_root
                    .as_ref()
                    .map(file_path)
                    .transpose()
                    .map_err(invalid)?;
                if bundle_root.is_none() && file_path(&argument.uri).is_err() {
                    return Err(invalid(
                        "virtual configuration needs an explicit local bundleRoot",
                    ));
                }
                if let Some(expected) = &argument.expected
                    && (expected.len() > Limits::default().max_input_bytes
                        || !crate::names::valid_tag(expected))
                {
                    return Err(invalid("expected must be a nonempty native type name"));
                }
                Some(Self {
                    definition,
                    bundle_root,
                    expected: argument.expected,
                })
            }
        };
        Ok((argument.uri, context))
    }

    pub(super) fn data(&self) -> bool {
        !self.definition
    }
}

pub(super) fn invalid(message: impl Into<String>) -> RequestFailure {
    (ErrorCode::InvalidParams, message.into())
}

pub(super) fn failed(message: impl Into<String>) -> RequestFailure {
    (ErrorCode::RequestFailed, message.into())
}

pub(super) fn token_options() -> lsp_types::SemanticTokensOptions {
    use lsp_types::{
        SemanticTokenModifier as Modifier, SemanticTokenType as Type, SemanticTokensFullOptions,
        SemanticTokensLegend, SemanticTokensOptions,
    };
    SemanticTokensOptions {
        legend: SemanticTokensLegend {
            // Same order as parser::tokens::Kind.
            token_types: vec![
                Type::PROPERTY,
                Type::NAMESPACE,
                Type::TYPE,
                Type::STRING,
                Type::NUMBER,
                Type::KEYWORD,
                Type::COMMENT,
                Type::REGEXP,
                Type::OPERATOR,
            ],
            token_modifiers: vec![Modifier::DEFAULT_LIBRARY],
        },
        full: Some(SemanticTokensFullOptions::Bool(true)),
        ..Default::default()
    }
}

// One source owner per accepted version. Incomplete text remains useful for
// cursor hints; rejected oversized text is discarded and blocks disk fallback.
pub(super) enum Text {
    Data(Document),
    Native(Definition),
    Incomplete {
        source: String,
        error: ParseError,
        data: bool,
    },
    Rejected(ParseError),
}

impl Text {
    pub(super) fn parse(source: String, data: bool) -> Self {
        let limits = Limits::default();
        if let Err(error) = parser::check_input(&source, limits) {
            return Self::Rejected(error);
        }
        if data {
            match parser::parse(&source, limits) {
                Ok(root) => Self::Data(Document { source, root }),
                Err(error) => Self::Incomplete {
                    source,
                    error,
                    data,
                },
            }
        } else {
            match parser::native::parse_definition(&source, limits) {
                Ok(root) => Self::Native(Definition { source, root }),
                Err(error) => Self::Incomplete {
                    source,
                    error,
                    data,
                },
            }
        }
    }

    pub(super) fn reparse(self, data: bool) -> Self {
        let same = match &self {
            Self::Data(_) => data,
            Self::Native(_) => !data,
            Self::Incomplete { data: previous, .. } => *previous == data,
            Self::Rejected(_) => true,
        };
        if same {
            return self;
        }
        let source = match self {
            Self::Data(document) => document.source,
            Self::Native(definition) => definition.source,
            Self::Incomplete { source, .. } => source,
            Self::Rejected(_) => unreachable!("rejected source is retained without reparsing"),
        };
        Self::parse(source, data)
    }

    pub(super) fn source(&self) -> Option<&str> {
        match self {
            Self::Data(document) => Some(document.source()),
            Self::Native(definition) => Some(definition.source()),
            Self::Incomplete { source, .. } => Some(source),
            Self::Rejected(_) => None,
        }
    }

    fn error(&self) -> Option<&ParseError> {
        match self {
            Self::Incomplete { error, .. } | Self::Rejected(error) => Some(error),
            _ => None,
        }
    }

    pub(super) fn data(&self) -> Option<&Document> {
        match self {
            Self::Data(document) => Some(document),
            _ => None,
        }
    }

    pub(super) fn native(&self) -> Option<&Definition> {
        match self {
            Self::Native(definition) => Some(definition),
            _ => None,
        }
    }

    pub(super) fn tokens(
        &self,
        encoding: Encoding,
    ) -> Result<lsp_types::SemanticTokens, RequestFailure> {
        if let Some(error) = self.error()
            && matches!(
                error.kind,
                crate::ErrorKind::LimitExceeded | crate::ErrorKind::InvalidLimits
            )
        {
            return Err(failed(display(error)));
        }
        let source = self
            .source()
            .ok_or_else(|| failed("semantic tokens require admitted document source"))?;
        let data = matches!(self, Self::Data(_) | Self::Incomplete { data: true, .. });
        let tokens =
            parser::tokens::collect(source, data).map_err(|error| failed(display(&error)))?;
        Ok(lsp_types::SemanticTokens {
            result_id: None,
            data: encoding.tokens(source, tokens),
        })
    }

    pub(super) fn format(&self) -> Result<String, RequestFailure> {
        let options = FormatOptions::default();
        let output = match self {
            Self::Data(document) => crate::format(document, options),
            Self::Native(definition) => crate::format_definition(definition, options),
            _ => return Err(failed("cannot format a document with parser diagnostics")),
        }
        .map_err(|error| failed(display(&error)))?;
        if matches!(self, Self::Data(_)) {
            parser::check_input(&output, Limits::default())
                .and_then(|()| parser::parse(&output, Limits::default()))
                .map(|_| ())
        } else {
            parser::native::parse_definition(&output, Limits::default()).map(|_| ())
        }
        .map_err(|error| {
            failed(format!(
                "formatted document rejected by parser: {}",
                display(&error)
            ))
        })?;
        Ok(output)
    }

    pub(super) fn diagnostics(&self, encoding: Encoding) -> Vec<Diagnostic> {
        self.error().map_or_else(Vec::new, |error| {
            vec![diagnostic(
                self.source()
                    .map_or_else(Range::default, |source| encoding.range(source, error.span)),
                display(error),
            )]
        })
    }
}

pub(super) struct Analysis {
    bundle: Bundle,
    compilation: EditorCompilation,
}

impl Analysis {
    fn dependencies(&self) -> &[PathBuf] {
        self.compilation.index.dependencies()
    }
}

enum AnalysisFailure {
    Native(NativeError),
    Message(&'static str),
}

impl Session {
    // The immutable native snapshot is admitted before its source strings are
    // copied. Neither failed/incomplete source nor a virtual URI becomes disk data.
    fn analysis(
        &self,
        uri: &Uri,
        extra: &[&str],
        discovery: &mut Vec<PathBuf>,
    ) -> Result<Analysis, AnalysisFailure> {
        let document = &self.documents[uri];
        let context = document
            .context
            .as_ref()
            .expect("analysis has selected context");
        let mut overlays = Vec::new();
        for (uri, document) in &self.documents {
            if let Ok(path) = file_path(uri) {
                if overlays.len() == MAX_FILES {
                    return Err(AnalysisFailure::Message(
                        "editor source overlay limit exceeded",
                    ));
                }
                overlays.push((path, document.text.source()));
            }
        }
        let bundle = match &context.bundle_root {
            Some(root) => Bundle::open_editor(root, overlays).map_err(AnalysisFailure::Native)?,
            None => {
                let path = file_path(uri).map_err(AnalysisFailure::Message)?;
                Bundle::discover_editor(&path, overlays, discovery)
                    .map_err(AnalysisFailure::Native)?
                    .ok_or(AnalysisFailure::Message(
                        "no ancestor daml.bundle; select bundleRoot",
                    ))?
            }
        };
        let mut roots = Vec::new();
        let definition_name;
        let expected;
        let data;
        if context.definition {
            let path = file_path(uri).map_err(AnalysisFailure::Message)?;
            definition_name = bundle
                .editor_definition_name(&path)
                .map_err(AnalysisFailure::Native)?;
            roots.push(definition_name.as_str());
            expected = None;
            data = None;
        } else {
            expected = context.expected.as_deref();
            data = document.text.data();
            if data.is_none()
                && let Some(source) = document.text.source()
                && let Ok(Some(span)) = parser::editor::root_tag(source, Limits::default())
            {
                roots.push(&source[span.start..span.end]);
            }
        }
        roots.extend_from_slice(extra);
        let compilation = bundle.compile_editor(data, expected, &roots);
        Ok(Analysis {
            bundle,
            compilation,
        })
    }

    pub(super) fn refresh(
        &mut self,
        connection: &Connection,
        changed: Option<&Uri>,
        requested: Option<(&Uri, &[&str])>,
    ) -> Result<Option<Analysis>, ServerError> {
        let (analysis, current) = self.analyze_contexts(connection, changed, requested)?;
        self.semantic_uris = current;
        Ok(analysis)
    }

    /// Refresh all opted-in contexts in transport order, retaining only the
    /// requested analysis. No cross-version cache or self-referential result.
    fn analyze_contexts(
        &self,
        connection: &Connection,
        changed: Option<&Uri>,
        requested: Option<(&Uri, &[&str])>,
    ) -> Result<(Option<Analysis>, HashSet<Uri>), ServerError> {
        let active: Vec<_> = self
            .documents
            .iter()
            .filter(|(_, document)| document.context.is_some())
            .map(|(uri, _)| uri.clone())
            .collect();
        if active.is_empty() && self.semantic_uris.is_empty() {
            if let Some(uri) = changed
                && let Some(document) = self.documents.get(uri)
            {
                publish(
                    connection,
                    uri.clone(),
                    Some(document.version),
                    document.text.diagnostics(self.encoding),
                )?;
            }
            return Ok((None, HashSet::new()));
        }
        let mut errors: HashMap<Uri, Vec<Diagnostic>> = HashMap::new();
        let mut notices = Vec::new();
        let mut retained = None;
        for uri in active {
            let document = &self.documents[&uri];
            let extra = requested
                .filter(|(target, _)| *target == &uri)
                .map_or(&[][..], |(_, extra)| extra);
            let mut discovery = Vec::new();
            let analysis = self.analysis(&uri, extra, &mut discovery);
            let compiled = analysis
                .as_ref()
                .ok()
                .map_or(&[][..], Analysis::dependencies);
            let failure_path = match &analysis {
                Err(AnalysisFailure::Native(error)) => error.path.as_ref(),
                _ => None,
            };
            let files = match dependency_uris(discovery.iter().chain(compiled).chain(failure_path))
            {
                Ok(uris) => uris,
                Err(message) => {
                    insert(
                        &mut errors,
                        uri.clone(),
                        diagnostic(Range::default(), message),
                    );
                    Vec::new()
                }
            };
            match &analysis {
                Ok(analysis) => {
                    if document.text.error().is_none() {
                        match &analysis.compilation.schemas {
                            Err(error) => self.native_diagnostics(
                                &uri,
                                error,
                                Some(&analysis.compilation.index),
                                &mut errors,
                            ),
                            Ok(schemas) if document.context.as_ref().is_some_and(Context::data) => {
                                if let Some(data) = document.text.data() {
                                    if data.tag(data.root()).is_none()
                                        && document
                                            .context
                                            .as_ref()
                                            .and_then(|c| c.expected.as_ref())
                                            .is_none()
                                    {
                                        insert(&mut errors, uri.clone(), diagnostic(self.encoding.range(data.source(), data.root().span),
                                            "configuration needs a root assertion or expected contract".into()));
                                    } else if let Err(error) = schemas.resolve_with_limits(
                                        data,
                                        document
                                            .context
                                            .as_ref()
                                            .and_then(|context| context.expected.as_deref()),
                                        Default::default(),
                                    ) {
                                        self.resolve_diagnostics(
                                            &uri,
                                            &error,
                                            &analysis.compilation.index,
                                            &mut errors,
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(AnalysisFailure::Native(error)) => {
                    self.native_diagnostics(&uri, error, None, &mut errors);
                }
                Err(AnalysisFailure::Message(message)) => insert(
                    &mut errors,
                    uri.clone(),
                    diagnostic(Range::default(), (*message).into()),
                ),
            }
            notices.push((uri.clone(), document.version, document.generation, files));
            if requested.is_some_and(|(target, _)| target == &uri) {
                retained = analysis.ok();
            }
        }
        let current: HashSet<_> = errors.keys().cloned().collect();
        let mut affected: HashSet<_> = self.documents.keys().cloned().collect();
        affected.extend(self.semantic_uris.iter().cloned());
        affected.extend(current.iter().cloned());
        for uri in affected {
            let mut diagnostics = errors.remove(&uri).unwrap_or_default();
            let version = self.documents.get(&uri).map(|document| {
                let mut syntax = document.text.diagnostics(self.encoding);
                for error in diagnostics.drain(..) {
                    if !syntax.contains(&error) {
                        syntax.push(error);
                    }
                }
                diagnostics = syntax;
                document.version
            });
            publish(connection, uri, version, diagnostics)?;
        }
        for (uri, version, generation, files) in notices {
            dependencies(connection, uri, version, generation, files)?;
        }
        Ok((retained, current))
    }

    fn native_diagnostics(
        &self,
        uri: &Uri,
        error: &NativeError,
        index: Option<&EditorIndex>,
        errors: &mut HashMap<Uri, Vec<Diagnostic>>,
    ) {
        let document = &self.documents[uri];
        let mut primary = diagnostic(Range::default(), display(error));
        if let Some(path) = &error.path {
            let target = self.source_uri(path);
            let source = index
                .and_then(|index| index.source(path))
                .or_else(|| self.source_for_path(path));
            let range = error
                .span
                .zip(source)
                .map_or_else(Range::default, |(span, source)| {
                    self.encoding.range(source, span)
                });
            if let Some(target) = target {
                if &target == uri {
                    primary.range = range;
                } else {
                    primary.related_information = Some(vec![DiagnosticRelatedInformation {
                        location: Location {
                            uri: target.clone(),
                            range,
                        },
                        message: "native definition/context failure".into(),
                    }]);
                    insert(errors, target, diagnostic(range, display(error)));
                }
            }
        } else if let (Some(source), Some(span)) = (document.text.source(), error.span) {
            primary.range = self.encoding.range(source, span);
        }
        insert(errors, uri.clone(), primary);
    }

    fn resolve_diagnostics(
        &self,
        uri: &Uri,
        error: &ResolveError,
        index: &EditorIndex,
        errors: &mut HashMap<Uri, Vec<Diagnostic>>,
    ) {
        let document = &self.documents[uri];
        let mut result = diagnostic(Range::default(), display(error));
        if let (Some(source), ErrorOrigin::Input(span)) = (document.text.source(), &error.origin) {
            result.range = self.encoding.range(source, *span);
        }
        let mut related = Vec::new();
        let location = match &error.origin {
            ErrorOrigin::Default(location) => Some(location),
            _ => error.schema.as_deref(),
        };
        if let Some(location) = location {
            for (number, target) in index.locations(location).into_iter().enumerate() {
                if number == 32 {
                    result.message.push_str(
                        " [additional source references exceed the editor display limit]",
                    );
                    break;
                }
                if let Ok(uri) = path_uri(target.path) {
                    related.push(DiagnosticRelatedInformation {
                        location: Location {
                            uri,
                            range: self.encoding.range(target.source, target.span),
                        },
                        message: "applicable native constraint/default".into(),
                    });
                }
            }
        }
        if !related.is_empty() {
            result.related_information = Some(related);
        }
        insert(errors, uri.clone(), result);
    }

    fn source_uri(&self, path: &Path) -> Option<Uri> {
        self.documents
            .keys()
            .find(|uri| {
                file_path(uri).is_ok_and(|candidate| {
                    candidate == path
                        || candidate
                            .canonicalize()
                            .is_ok_and(|candidate| candidate == path)
                })
            })
            .cloned()
            .or_else(|| path_uri(path).ok())
    }

    fn source_for_path(&self, path: &Path) -> Option<&str> {
        let uri = self.documents.keys().find(|uri| {
            file_path(uri).is_ok_and(|candidate| {
                candidate == path
                    || candidate
                        .canonicalize()
                        .is_ok_and(|candidate| candidate == path)
            })
        })?;
        self.documents[uri].text.source()
    }
}

#[derive(Serialize)]
struct Dependencies {
    uri: Uri,
    version: i32,
    generation: u32,
    files: Vec<Uri>,
}

pub(super) fn dependencies(
    connection: &Connection,
    uri: Uri,
    version: i32,
    generation: u32,
    files: Vec<Uri>,
) -> Result<(), ServerError> {
    send(
        connection,
        Notification::new(
            DEPENDENCIES.into(),
            Dependencies {
                uri,
                version,
                generation,
                files,
            },
        )
        .into(),
    )
}

fn dependency_uris<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> Result<Vec<Uri>, String> {
    let mut seen = HashSet::new();
    let mut bytes = 0usize;
    let mut output = Vec::new();
    for path in paths {
        // Resolve watcher parents and retain every file-link entry, not merely
        // the ultimate target. Native admission preserves symlink/.. order.
        for entry in crate::native::tooling::editor_watch_paths(path) {
            let physical = entry.map_err(|error| display(&error))?;
            if seen.contains(&physical) {
                break;
            }
            if output.len() == MAX_FILES {
                return Err("editor dependency count limit exceeded".into());
            }
            let uri = path_uri(&physical).map_err(str::to_owned)?;
            bytes = bytes
                .checked_add(uri.as_str().len())
                .ok_or_else(|| "editor dependency byte limit exceeded".to_owned())?;
            if bytes > MAX_METADATA_BYTES {
                return Err("editor dependency byte limit exceeded".into());
            }
            output.push(uri);
            seen.insert(physical);
        }
    }
    Ok(output)
}

fn publish(
    connection: &Connection,
    uri: Uri,
    version: Option<i32>,
    diagnostics: Vec<Diagnostic>,
) -> Result<(), ServerError> {
    send(
        connection,
        Notification::new(
            PublishDiagnostics::METHOD.into(),
            PublishDiagnosticsParams::new(uri, diagnostics, version),
        )
        .into(),
    )
}

fn diagnostic(range: Range, message: String) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("daml".into()),
        message,
        ..Default::default()
    }
}

fn insert(errors: &mut HashMap<Uri, Vec<Diagnostic>>, uri: Uri, error: Diagnostic) {
    let items = errors.entry(uri).or_default();
    if !items.contains(&error) {
        items.push(error);
    }
}

// Error display is presentation, not admission. Stop boundedly without copying
// arbitrarily long source names/patterns into a diagnostic or exposing raw input.
fn display(value: &impl fmt::Display) -> String {
    use fmt::Write;
    let mut output = DisplayText {
        text: String::new(),
        remaining: 8 * 1024,
    };
    if write!(&mut output, "{value}").is_err() {
        output.text.push_str(" … [diagnostic display limit]");
    }
    output.text
}

struct DisplayText {
    text: String,
    remaining: usize,
}
impl fmt::Write for DisplayText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() > self.remaining {
            let mut end = self.remaining;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&text[..end]);
            self.remaining = 0;
            return Err(fmt::Error);
        }
        self.text.push_str(text);
        self.remaining -= text.len();
        Ok(())
    }
}
