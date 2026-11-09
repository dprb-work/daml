//! Presentation/projection of admitted native contracts, not another validator.

use super::{Analysis, RequestFailure, Session, failed};
use crate::{
    Definition, Limits, Node, Origin, ResolvedNode, ResolvedValue, Span, Value,
    native::{DefaultValue, Expression, ExpressionKind},
    parser::editor::{
        CompletionContext, CompletionKind, completion_context, native_completion_context,
    },
    schema::{SchemaId, SchemaView},
};
use lsp_server::{Connection, ErrorCode};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionTextEdit, Documentation, Hover,
    HoverContents, Location, MarkupContent, MarkupKind, TextDocumentPositionParams, TextEdit,
};
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, HashSet},
    io::{self, Write},
};

const MAX_WORK: usize = 10_000;
const MAX_ITEMS: usize = 256;
const MAX_LOCATIONS: usize = 256;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
const MAX_HOVER_BYTES: usize = 64 * 1024;

impl Session {
    pub(in crate::lsp) fn provider(
        &mut self,
        connection: &Connection,
        method: &str,
        params: TextDocumentPositionParams,
    ) -> Result<Result<Json, RequestFailure>, super::ServerError> {
        let uri = &params.text_document.uri;
        let Some(document) = self.documents.get(uri) else {
            return Ok(Err(super::invalid("document is not open")));
        };
        if document.context.is_none() {
            return Ok(Ok(if method == "textDocument/completion" {
                json!([])
            } else {
                Json::Null
            }));
        }
        let Some(source) = document.text.source() else {
            return Ok(Err(failed(
                "document source exceeds the editor input limit",
            )));
        };
        let Some(offset) = self.encoding.offset(source, params.position) else {
            return Ok(Err(super::invalid(
                "position is not a valid negotiated source boundary",
            )));
        };
        let cursor = if method == "textDocument/completion" {
            let parsed = if document
                .context
                .as_ref()
                .expect("selected provider context")
                .definition
            {
                if let Some(tag) = document
                    .text
                    .native()
                    .and_then(|native| native_target(native, offset))
                    .and_then(|target| target.tag)
                {
                    Ok(Some(CompletionContext {
                        kind: CompletionKind::Tag,
                        path: String::new(),
                        prefix: Span {
                            start: tag.start,
                            end: offset,
                        },
                        replace: tag,
                        tags: Vec::new(),
                        fields: Vec::new(),
                    }))
                } else {
                    native_completion_context(source, offset, Limits::default())
                }
            } else {
                completion_context(source, offset, Limits::default())
            };
            match parsed.map_err(|error| super::display(&error)) {
                Ok(cursor) => cursor,
                Err(message) => return Ok(Err(failed(message))),
            }
        } else {
            None
        };
        let extra: Vec<&str> = cursor.as_ref().map_or_else(Vec::new, |cursor| {
            cursor
                .tags
                .iter()
                .map(|(_, span)| &source[span.start..span.end])
                .collect()
        });
        let (analysis, current) = self.analyze_contexts(connection, None, Some((uri, &extra)))?;
        self.semantic_uris = current;
        let Some(analysis) = analysis else {
            return Ok(Ok(Json::Null));
        };
        let document = &self.documents[uri];
        let result = match method {
            "textDocument/completion" => completion(self, document, &analysis, cursor.as_ref()),
            "textDocument/hover" => hover(self, document, &analysis, offset),
            "textDocument/definition" => definition(self, uri, document, &analysis, offset),
            _ => unreachable!("only advertised semantic providers reach this dispatch"),
        };
        Ok(result)
    }
}

fn completion(
    session: &Session,
    document: &super::super::OpenDocument,
    analysis: &Analysis,
    cursor: Option<&CompletionContext>,
) -> Result<Json, RequestFailure> {
    let Some(cursor) = cursor else {
        return Ok(json!([]));
    };
    let source = document.text.source().expect("cursor retains source");
    let prefix = &source[cursor.prefix.start..cursor.prefix.end];
    let range = session.encoding.range(source, cursor.replace);
    let mut items = Vec::new();
    let mut bytes = 0usize;
    let mut incomplete = false;
    match cursor.kind {
        CompletionKind::Tag => {
            let mut names = analysis
                .bundle
                .editor_visible_names(prefix)
                .map_err(|error| failed(super::display(&error)))?;
            names.extend(
                analysis
                    .compilation
                    .index
                    .loaded_names()
                    .filter(|name| name.starts_with(prefix))
                    .map(str::to_owned),
            );
            names.sort();
            names.dedup();
            for name in names {
                if !name.starts_with(prefix) {
                    continue;
                }
                if items.len() == MAX_ITEMS
                    || !reserve(
                        &mut bytes,
                        name.len().saturating_mul(12).saturating_add(512),
                    )
                {
                    incomplete = true;
                    break;
                }
                items.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some("visible native assertion/reference".into()),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: name,
                    })),
                    ..Default::default()
                });
            }
        }
        CompletionKind::Field => {
            if analysis.compilation.schemas.is_err() {
                return Ok(json!([]));
            }
            let roots = roots(document, analysis, Some(cursor), &cursor.path)?;
            let scopes = project(&roots, &cursor.path)?;
            let candidates = fields(&roots, &cursor.path, &scopes)?;
            let has_separator = crate::parser::editor::field_has_separator(source, cursor.replace);
            let mut present: HashSet<&str> = cursor
                .fields
                .iter()
                .map(|span| &source[span.start..span.end])
                .collect();
            if let Some(data) = document.text.data()
                && let Some(node) = node_at_path(data, &cursor.path)
                && let Value::Mapping(entries) = &node.value
            {
                present.extend(
                    entries
                        .iter()
                        .map(|entry| data.slice(entry.key).expect("parser-owned key span")),
                );
            }
            for (name, field) in candidates {
                if !name.starts_with(prefix) || present.contains(name) {
                    continue;
                }
                let detail = field_detail(&field);
                if items.len() == MAX_ITEMS
                    || !reserve(
                        &mut bytes,
                        name.len()
                            .saturating_mul(12)
                            .saturating_add(detail.len().saturating_mul(6))
                            .saturating_add(1024),
                    )
                {
                    incomplete = true;
                    break;
                }
                items.push(CompletionItem {
                    label: name.to_owned(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some(detail.clone()),
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: code_block("text", &detail),
                    })),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: if has_separator {
                            name.to_owned()
                        } else {
                            format!("{name}: ")
                        },
                    })),
                    ..Default::default()
                });
            }
        }
        CompletionKind::Value => {}
    }
    encode(CompletionList {
        is_incomplete: incomplete,
        items,
    })
}

fn hover(
    session: &Session,
    document: &super::super::OpenDocument,
    analysis: &Analysis,
    offset: usize,
) -> Result<Json, RequestFailure> {
    let source = document.text.source().expect("provider has source");
    let Some(context) = &document.context else {
        return Ok(Json::Null);
    };
    if context.definition {
        let Some(native) = document.text.native() else {
            return Ok(Json::Null);
        };
        let Some(target) = native_target(native, offset) else {
            return Ok(Json::Null);
        };
        let text = source.get(target.span.start..target.span.end).unwrap_or("");
        if text.len() > MAX_HOVER_BYTES / 2 {
            return Err(failed("native hover display limit exceeded"));
        }
        return encode(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("**Native definition**\n\n{}", code_block("daml", text)),
            }),
            range: Some(session.encoding.range(source, target.span)),
        });
    }
    let Some(data) = document.text.data() else {
        return Ok(Json::Null);
    };
    let Some(target) = data_target(data.root(), data.source(), offset)? else {
        return Ok(Json::Null);
    };
    let Ok(schemas) = &analysis.compilation.schemas else {
        return Ok(Json::Null);
    };
    let roots = roots(document, analysis, None, &target.path)?;
    let scopes = project(&roots, &target.path)?;
    let mut text = String::new();
    match schemas.explain(data, context.expected.as_deref(), &target.path) {
        Ok(explanation) => {
            text.push_str("**Checked value**\n\n");
            text.push_str(&code_block("text", &value_summary(explanation.node())));
            text.push_str("\n\n**Origin:** ");
            match explanation.node().origin() {
                Origin::Input(span) => text.push_str(&format!(
                    "supplied input, UTF-8 bytes {}..{}",
                    span.start, span.end
                )),
                Origin::Default(location) => {
                    text.push_str("declared default\n\n");
                    text.push_str(&code_block(
                        "text",
                        &format!("{}#{}", location.resource, location.pointer),
                    ));
                }
                Origin::Semantic => text.push_str("semantic input (no invented source span)"),
            }
            text.push_str("\n\n**Applicable checked constraints**\n\n");
            append_schemas(
                &mut text,
                explanation
                    .constraints()
                    .iter()
                    .filter(|constraint| constraint.instance_path == target.path)
                    .map(|constraint| (constraint.schema, false)),
            )?;
        }
        Err(error) => {
            text.push_str("**Schema hints — configuration not checked**\n\n");
            text.push_str(&code_block("text", &super::display(&error)));
            text.push_str("\n\n**Constraints and declared defaults**\n\n");
            append_schemas(
                &mut text,
                scopes
                    .iter()
                    .map(|scope| (scope.view.value(), scope.alternative)),
            )?;
        }
    }
    // Container defaults are declarations even when the defaulted child has no
    // authored source span to hover. Keep those values visible without inventing one.
    if scopes
        .iter()
        .any(|scope| scope.view.container_kind() == Some("object"))
    {
        let candidates = fields(&roots, &target.path, &scopes)?;
        for (name, field) in candidates {
            if !field.defaults.is_empty() {
                let detail = format!("{name}: {}", field_detail(&field));
                if text.len().saturating_add(detail.len().saturating_mul(3)) > MAX_HOVER_BYTES {
                    return Err(failed("hover display limit exceeded"));
                }
                text.push_str("\n\n");
                text.push_str(&code_block("text", &detail));
            }
        }
    }
    if text.len() > MAX_HOVER_BYTES {
        return Err(failed("hover display limit exceeded"));
    }
    encode(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: text,
        }),
        range: Some(session.encoding.range(source, target.span)),
    })
}

fn definition(
    session: &Session,
    uri: &lsp_types::Uri,
    document: &super::super::OpenDocument,
    analysis: &Analysis,
    offset: usize,
) -> Result<Json, RequestFailure> {
    let source = document.text.source().expect("provider has source");
    let from = super::file_path(uri).ok();
    let mut targets = Vec::new();
    if let Some(native) = document.text.native() {
        if let Some(target) = native_target(native, offset)
            && let Some(tag) = target.tag
        {
            targets.extend(
                analysis
                    .compilation
                    .index
                    .reference_locations(from.as_deref(), &source[tag.start..tag.end]),
            );
        }
    } else if let Some(data) = document.text.data()
        && let Some(target) = data_target(data.root(), source, offset)?
    {
        if let Some(tag) = target.tag {
            targets.extend(
                analysis
                    .compilation
                    .index
                    .reference_locations(from.as_deref(), &source[tag.start..tag.end]),
            );
        } else if analysis.compilation.schemas.is_ok() {
            for scope in project(
                &roots(document, analysis, None, &target.path)?,
                &target.path,
            )? {
                targets.extend(
                    analysis
                        .compilation
                        .index
                        .locations(scope.view.explanation_location()),
                );
            }
        }
    }
    let mut result: Vec<Location> = Vec::new();
    let mut bytes = 0;
    for target in targets {
        if result.len() == MAX_LOCATIONS {
            return Err(failed("definition result exceeds editor location limit"));
        }
        let uri = super::path_uri(target.path).map_err(failed)?;
        if !reserve(
            &mut bytes,
            uri.as_str().len().saturating_mul(6).saturating_add(256),
        ) {
            return Err(failed("definition result exceeds editor byte limit"));
        }
        let location = Location {
            uri,
            range: session.encoding.range(target.source, target.span),
        };
        if !result.contains(&location) {
            result.push(location);
        }
    }
    encode(result)
}

#[derive(Clone, Copy)]
struct Hint<'a> {
    view: SchemaView<'a>,
    alternative: bool,
}

fn roots<'a, 'p>(
    document: &super::super::OpenDocument,
    analysis: &'a Analysis,
    cursor: Option<&'p CompletionContext>,
    target: &'p str,
) -> Result<Vec<(&'p str, Hint<'a>)>, RequestFailure> {
    let Ok(schemas) = &analysis.compilation.schemas else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    let context = document
        .context
        .as_ref()
        .expect("selected provider context");
    if let Some(expected) = &context.expected
        && let Some(view) = schemas.type_schema(expected)
    {
        result.push((
            "",
            Hint {
                view,
                alternative: false,
            },
        ));
    }
    if let Some(data) = document.text.data() {
        let mut node = data.root();
        let mut end = 0;
        if let Some(name) = data.tag(node)
            && let Some(view) = schemas.type_schema(name)
        {
            result.push((
                "",
                Hint {
                    view,
                    alternative: false,
                },
            ));
        }
        // Only ancestors of the target contribute assertions. Borrow prefixes
        // from its one bounded path instead of copying every path in the tree.
        for raw in raw_parts(target) {
            end += 1 + raw.len();
            let part = crate::pointer::decode_segment(raw).expect("internally encoded pointer");
            node = match &node.value {
                Value::Mapping(entries) => match entries.iter().find(|entry| {
                    data.slice(entry.key).expect("parser-owned key span") == part.as_ref()
                }) {
                    Some(entry) => &entry.value,
                    None => break,
                },
                Value::Sequence(items) => match part
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| items.get(index))
                {
                    Some(node) => node,
                    None => break,
                },
                _ => break,
            };
            if let Some(name) = data.tag(node)
                && let Some(view) = schemas.type_schema(name)
            {
                result.push((
                    &target[..end],
                    Hint {
                        view,
                        alternative: false,
                    },
                ));
            }
        }
    } else if let Some(cursor) = cursor {
        let source = document.text.source().expect("cursor has source");
        for (path, span) in &cursor.tags {
            if let Some(view) = schemas.type_schema(&source[span.start..span.end]) {
                result.push((
                    path.as_str(),
                    Hint {
                        view,
                        alternative: false,
                    },
                ));
            }
        }
        if let Ok(Some(span)) = crate::parser::editor::root_tag(source, Limits::default())
            && let Some(view) = schemas.type_schema(&source[span.start..span.end])
        {
            result.push((
                "",
                Hint {
                    view,
                    alternative: false,
                },
            ));
        }
    }
    Ok(result)
}

fn raw_parts(path: &str) -> impl Iterator<Item = &str> {
    let suffix = if path.is_empty() {
        None
    } else {
        Some(
            path.strip_prefix('/')
                .expect("internal instance paths are RFC 6901 pointers"),
        )
    };
    suffix.into_iter().flat_map(|suffix| suffix.split('/'))
}

fn path_parts(path: &str) -> impl Iterator<Item = std::borrow::Cow<'_, str>> {
    raw_parts(path)
        .map(|part| crate::pointer::decode_segment(part).expect("internally encoded pointer"))
}

fn project<'a>(roots: &[(&str, Hint<'a>)], path: &str) -> Result<Vec<Hint<'a>>, RequestFailure> {
    let mut result = Vec::new();
    let mut work = 0usize;
    for &(at, root) in roots {
        let Some(suffix) = path
            .strip_prefix(at)
            .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
        else {
            continue;
        };
        let mut current = expand(vec![root], &mut work)?;
        for part in path_parts(suffix) {
            let mut children = Vec::new();
            for scope in &current {
                let child = if scope.view.container_kind() == Some("array") {
                    part.parse::<usize>()
                        .ok()
                        .and_then(|index| scope.view.item(index))
                } else {
                    scope.view.property(&part)
                };
                if let Some(view) = child {
                    children.push(Hint {
                        view,
                        alternative: scope.alternative,
                    });
                }
            }
            current = expand(children, &mut work)?;
        }
        result.extend(current);
        if result.len() > MAX_WORK {
            return Err(failed("schema hint result limit exceeded"));
        }
    }
    let mut seen = HashSet::new();
    result.retain(|scope| seen.insert((scope.view.id(), scope.alternative)));
    Ok(result)
}

fn expand<'a>(
    mut pending: Vec<Hint<'a>>,
    work: &mut usize,
) -> Result<Vec<Hint<'a>>, RequestFailure> {
    let mut seen: HashSet<(SchemaId, bool)> = HashSet::new();
    let mut output = Vec::new();
    while let Some(scope) = pending.pop() {
        if !seen.insert((scope.view.id(), scope.alternative)) {
            continue;
        }
        *work = work.saturating_add(1);
        if *work > MAX_WORK {
            return Err(failed("schema hint work limit exceeded"));
        }
        output.push(scope);
        if let Some(view) = scope.view.referenced() {
            pending.push(Hint { view, ..scope });
        }
        for (keyword, alternative) in [
            ("allOf", scope.alternative),
            ("anyOf", true),
            ("oneOf", true),
        ] {
            for view in scope.view.array_children(keyword) {
                if pending.len().saturating_add(*work) >= MAX_WORK {
                    return Err(failed("schema hint work limit exceeded"));
                }
                pending.push(Hint { view, alternative });
            }
        }
    }
    Ok(output)
}

struct Field<'a> {
    required: bool,
    alternative_required: bool,
    schemas: Vec<Hint<'a>>,
    defaults: Vec<&'a Json>,
}
// Candidate names come from display projection; declarations and closed
// eligibility must keep the original conjunction/alternative grouping.
fn fields<'a>(
    roots: &[(&str, Hint<'a>)],
    path: &str,
    scopes: &[Hint<'a>],
) -> Result<BTreeMap<&'a str, Field<'a>>, RequestFailure> {
    let mut result = BTreeMap::new();
    let mut work = 0;
    for scope in scopes {
        for name in scope.view.property_names() {
            work += 1;
            if work > MAX_WORK {
                return Err(failed("field hint work limit exceeded"));
            }
            result.entry(name).or_insert_with(|| Field {
                required: false,
                alternative_required: false,
                schemas: Vec::new(),
                defaults: Vec::new(),
            });
        }
    }
    let mut active = HashSet::new();
    for &(at, root) in roots {
        let Some(suffix) = path
            .strip_prefix(at)
            .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
        else {
            continue;
        };
        let mut keys: Vec<_> = path_parts(suffix).collect();
        let mut excluded = Vec::new();
        for (&name, field) in &mut result {
            keys.push(std::borrow::Cow::Borrowed(name));
            if !field_permission(root, &keys, 0, &mut work, &mut active, field)?.allowed {
                excluded.push(name);
            }
            keys.pop();
        }
        for name in excluded {
            result.remove(name);
        }
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct FieldPermission {
    allowed: bool,
    evaluated: bool,
}

fn field_permission<'a>(
    scope: Hint<'a>,
    keys: &[std::borrow::Cow<'_, str>],
    depth: usize,
    work: &mut usize,
    active: &mut HashSet<(SchemaId, usize)>,
    field: &mut Field<'a>,
) -> Result<FieldPermission, RequestFailure> {
    *work = work.saturating_add(1);
    if *work > MAX_WORK || depth > crate::schema::profile::MAX_SCHEMA_DEPTH {
        return Err(failed("closed-field projection work/depth limit exceeded"));
    }
    let view = scope.view;
    let mut result = FieldPermission {
        allowed: view.value() != &Json::Bool(false),
        evaluated: false,
    };
    let identity = (view.id(), keys.len());
    if !active.insert(identity) {
        return Ok(result);
    }
    let checkpoint = (
        field.schemas.len(),
        field.defaults.len(),
        field.required,
        field.alternative_required,
    );
    if keys.is_empty()
        && !field.schemas.iter().any(|existing| {
            existing.view.id() == view.id() && existing.alternative == scope.alternative
        })
    {
        field.schemas.push(scope);
    }
    let default = if let Some((key, rest)) = keys.split_first() {
        let child = if view.container_kind() == Some("array") {
            key.parse::<usize>().ok().and_then(|index| view.item(index))
        } else {
            view.property(key)
        };
        if let Some(view) = child {
            result.allowed &=
                field_permission(Hint { view, ..scope }, rest, depth + 1, work, active, field)?
                    .allowed;
            result.evaluated = true;
        } else if let Some(additional) = view.value().get("additionalProperties") {
            result.allowed &= additional != &Json::Bool(false);
            result.evaluated = true;
        }
        if rest.is_empty() {
            if view
                .value()
                .get("required")
                .and_then(Json::as_array)
                .is_some_and(|names| names.iter().any(|name| name.as_str() == Some(key)))
            {
                if scope.alternative {
                    field.alternative_required = true;
                } else {
                    field.required = true;
                }
            }
            view.value()
                .get("default")
                .and_then(Json::as_object)
                .and_then(|values| values.get(key.as_ref()))
        } else {
            None
        }
    } else {
        view.value().get("default")
    };
    if let Some(value) = default
        && !field
            .defaults
            .iter()
            .any(|existing| std::ptr::eq(*existing, value))
    {
        field.defaults.push(value);
    }
    for view in view
        .referenced()
        .into_iter()
        .chain(view.array_children("allOf"))
    {
        let permission =
            field_permission(Hint { view, ..scope }, keys, depth + 1, work, active, field)?;
        result.allowed &= permission.allowed;
        result.evaluated |= permission.evaluated;
    }
    for keyword in ["anyOf", "oneOf"] {
        if view.value().get(keyword).is_none() {
            continue;
        }
        let mut alternative = FieldPermission {
            allowed: false,
            evaluated: false,
        };
        for view in view.array_children(keyword) {
            let permission = field_permission(
                Hint {
                    view,
                    alternative: true,
                },
                keys,
                depth + 1,
                work,
                active,
                field,
            )?;
            if permission.allowed {
                alternative.allowed = true;
                alternative.evaluated |= permission.evaluated;
            }
        }
        result.allowed &= alternative.allowed;
        result.evaluated |= alternative.evaluated;
    }
    if !keys.is_empty()
        && view.value().get("unevaluatedProperties") == Some(&Json::Bool(false))
        && !result.evaluated
    {
        result.allowed = false;
    }
    if !result.allowed {
        field.schemas.truncate(checkpoint.0);
        field.defaults.truncate(checkpoint.1);
        field.required = checkpoint.2;
        field.alternative_required = checkpoint.3;
    }
    active.remove(&identity);
    Ok(result)
}

fn field_detail(field: &Field<'_>) -> String {
    let mut result = if field.required {
        "required"
    } else if field.alternative_required {
        "required in an alternative"
    } else {
        "optional"
    }
    .to_owned();
    if field.schemas.iter().any(|scope| scope.alternative) {
        result.push_str("; alternative contract");
    }
    let mut types = HashSet::new();
    for scope in &field.schemas {
        if let Some(kind) = scope
            .view
            .value()
            .get("x-daml-scalar")
            .and_then(|scalar| scalar.get("kind"))
            .and_then(Json::as_str)
            .or_else(|| scope.view.value().get("type").and_then(Json::as_str))
            && types.insert(kind)
        {
            result.push_str("; ");
            result.push_str(kind);
        }
    }
    match field.defaults.as_slice() {
        [] => {}
        [value] => {
            result.push_str("; declared default ");
            result.push_str(
                &json_text(*value, 2048)
                    .unwrap_or_else(|| "[default exceeds display limit]".into()),
            );
        }
        _ => result.push_str("; multiple declared defaults (not a selected completed value)"),
    }
    result
}

fn append_schemas<'a>(
    text: &mut String,
    scopes: impl IntoIterator<Item = (&'a Json, bool)>,
) -> Result<(), RequestFailure> {
    let facets = [
        "type",
        "const",
        "enum",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
        "required",
        "additionalProperties",
        "unevaluatedProperties",
        "default",
        "x-daml-scalar",
        "x-daml-regex",
    ];
    for (schema, alternative) in scopes {
        let mut details = String::new();
        for key in facets {
            if let Some(value) = schema.get(key) {
                details.push_str(key);
                details.push_str(": ");
                details.push_str(
                    &json_text(value, 4096)
                        .unwrap_or_else(|| "[constraint exceeds display limit]".into()),
                );
                details.push('\n');
                if details.len() > MAX_HOVER_BYTES / 2 {
                    return Err(failed("hover display limit exceeded"));
                }
            }
        }
        if !details.is_empty() {
            if alternative {
                text.push_str("Alternative declaration (not an admission certificate):\n\n");
            }
            let block = code_block("text", &details);
            if text.len().saturating_add(block.len()) > MAX_HOVER_BYTES {
                return Err(failed("hover display limit exceeded"));
            }
            text.push_str(&block);
            text.push_str("\n\n");
        }
    }
    Ok(())
}

struct Target {
    path: String,
    span: Span,
    tag: Option<Span>,
}
fn data_target(node: &Node, source: &str, offset: usize) -> Result<Option<Target>, RequestFailure> {
    fn walk(
        node: &Node,
        source: &str,
        offset: usize,
        path: &mut String,
    ) -> Result<Option<Target>, RequestFailure> {
        if let Some(tag) = node.tag
            && tag.start <= offset
            && offset <= tag.end
        {
            return Ok(Some(Target {
                path: path.clone(),
                span: tag,
                tag: Some(tag),
            }));
        }
        match &node.value {
            Value::Mapping(entries) => {
                for entry in entries {
                    let key = entry.key.start <= offset && offset <= entry.key.end;
                    if key || contains(&entry.value, offset) {
                        let parent = path.len();
                        push_path(path, &source[entry.key.start..entry.key.end])?;
                        let result = if key {
                            Ok(Some(Target {
                                path: path.clone(),
                                span: entry.key,
                                tag: None,
                            }))
                        } else {
                            walk(&entry.value, source, offset, path)
                        };
                        path.truncate(parent);
                        return result;
                    }
                }
            }
            Value::Sequence(items) => {
                for (index, item) in items.iter().enumerate() {
                    if contains(item, offset) {
                        use std::fmt::Write;
                        let digits = index.checked_ilog10().map_or(1, |log| log as usize + 1);
                        let parent = path.len();
                        if parent.saturating_add(1 + digits) > Limits::default().max_input_bytes {
                            return Err(failed("instance path exceeds editor limit"));
                        }
                        path.reserve(1 + digits);
                        write!(path, "/{index}")
                            .expect("writing an admitted index into a String cannot fail");
                        let result = walk(item, source, offset, path);
                        path.truncate(parent);
                        return result;
                    }
                }
            }
            _ => {}
        }
        Ok(contains(node, offset).then(|| Target {
            path: path.clone(),
            span: node.span,
            tag: None,
        }))
    }
    walk(node, source, offset, &mut String::new())
}

fn contains(node: &Node, offset: usize) -> bool {
    (node.span.start <= offset && offset <= node.span.end)
        || node
            .tag
            .is_some_and(|tag| tag.start <= offset && offset <= tag.end)
}

fn push_path(path: &mut String, segment: &str) -> Result<(), RequestFailure> {
    let bytes = crate::pointer::segment_len(segment);
    if path.len().saturating_add(bytes) > Limits::default().max_input_bytes {
        return Err(failed("instance path exceeds editor limit"));
    }
    path.reserve(bytes);
    crate::pointer::append(path, segment);
    Ok(())
}

fn node_at_path<'a>(data: &'a crate::Document, path: &str) -> Option<&'a Node> {
    let mut node = data.root();
    for part in path_parts(path) {
        node = match &node.value {
            Value::Mapping(entries) => {
                &entries
                    .iter()
                    .find(|entry| {
                        data.slice(entry.key).expect("parser-owned key span") == part.as_ref()
                    })?
                    .value
            }
            Value::Sequence(items) => items.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(node)
}

struct NativeTarget {
    span: Span,
    tag: Option<Span>,
}
fn native_target(definition: &Definition, offset: usize) -> Option<NativeTarget> {
    expression_target(&definition.root, offset)
}
fn expression_target(expression: &Expression, offset: usize) -> Option<NativeTarget> {
    if offset < expression.span.start || offset > expression.span.end {
        return None;
    }
    match &expression.kind {
        ExpressionKind::Tagged { name, body } => {
            if name.start <= offset && offset <= name.end {
                return Some(NativeTarget {
                    span: expression.span,
                    tag: Some(*name),
                });
            }
            if let Some(body) = body
                && let Some(found) = expression_target(body, offset)
            {
                return Some(found);
            }
        }
        ExpressionKind::Record(members) => {
            for member in members {
                if member.key.start <= offset && offset <= member.key.end {
                    return Some(NativeTarget {
                        span: member.constraint.span,
                        tag: None,
                    });
                }
                if let Some(found) = expression_target(&member.constraint, offset) {
                    return Some(found);
                }
            }
        }
        ExpressionKind::Tuple(expressions)
        | ExpressionKind::Union(expressions)
        | ExpressionKind::Intersection(expressions) => {
            for expression in expressions {
                if let Some(found) = expression_target(expression, offset) {
                    return Some(found);
                }
            }
        }
        ExpressionKind::Group(inner) | ExpressionKind::Closed(inner) => {
            if let Some(found) = expression_target(inner, offset) {
                return Some(found);
            }
        }
        ExpressionKind::Literal(node) => {
            if let Some(tag) = tag_target(node, offset) {
                return Some(NativeTarget {
                    span: node.span,
                    tag: Some(tag),
                });
            }
        }
        ExpressionKind::Regex { .. } => {}
    }
    if let DefaultValue::Value(node) = &expression.default
        && let Some(tag) = tag_target(node, offset)
    {
        return Some(NativeTarget {
            span: tag,
            tag: Some(tag),
        });
    }
    Some(NativeTarget {
        span: expression.span,
        tag: None,
    })
}
fn tag_target(node: &Node, offset: usize) -> Option<Span> {
    if let Some(tag) = node.tag
        && tag.start <= offset
        && offset <= tag.end
    {
        return Some(tag);
    }
    match &node.value {
        Value::Mapping(entries) => entries
            .iter()
            .find_map(|entry| tag_target(&entry.value, offset)),
        Value::Sequence(items) => items.iter().find_map(|node| tag_target(node, offset)),
        _ => None,
    }
}

fn value_summary(node: &ResolvedNode<'_>) -> String {
    match node.value() {
        ResolvedValue::Null => "null".into(),
        ResolvedValue::Bool(value) => value.to_string(),
        ResolvedValue::Number(value) => value.as_ref().as_str().to_owned(),
        ResolvedValue::String(value) => json_text(value.as_ref(), 4096)
            .unwrap_or_else(|| "[string exceeds display limit]".into()),
        ResolvedValue::Temporal(value) => format!("{} {}", value.kind().as_str(), value.as_str()),
        ResolvedValue::Sequence(items) => format!("list with {} completed items", items.len()),
        ResolvedValue::Mapping(fields) => format!("object with {} completed fields", fields.len()),
    }
}

fn code_block(language: &str, text: &str) -> String {
    let mut longest = 0;
    let mut current = 0;
    for byte in text.bytes() {
        if byte == b'`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    let fence = "`".repeat(3.max(longest + 1));
    format!("{fence}{language}\n{text}\n{fence}")
}

struct JsonText {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for JsonText {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("display limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn json_text(value: &(impl Serialize + ?Sized), limit: usize) -> Option<String> {
    let mut output = JsonText {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).ok()?;
    Some(String::from_utf8(output.bytes).expect("JSON serialization emits UTF-8"))
}
fn encode(value: impl Serialize) -> Result<Json, RequestFailure> {
    serde_json::to_value(value).map_err(|_| {
        (
            ErrorCode::InternalError,
            "semantic result could not be encoded".into(),
        )
    })
}
fn reserve(bytes: &mut usize, amount: usize) -> bool {
    match bytes
        .checked_add(amount)
        .filter(|bytes| *bytes <= MAX_RESULT_BYTES)
    {
        Some(next) => {
            *bytes = next;
            true
        }
        None => false,
    }
}
