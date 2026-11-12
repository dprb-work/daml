//! Eager, snapshot-bound default materialization through the existing source editor.

use super::{AnalysisFailure, RequestFailure, Session, failed, invalid};
use crate::{
    Document, NativeErrorKind, Node, Origin, ResolveErrorKind, ResolvedNode, ResolvedValue,
    Schemas, SemanticErrorKind, SemanticNode, SourceEdit, SourceEditError, Value,
};
use lsp_types::CodeActionParams;
use serde_json::{Value as Json, json};
use std::collections::HashSet;

use std::io::{self, Write};
const MAX_ACTIONS: usize = 256;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

// Count the actual JSON encoding without allocating a second response payload.
struct ResultBytes(usize);

impl Write for ResultBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_RESULT_BYTES)
            .ok_or_else(|| io::Error::other("code action response limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Session {
    pub(in crate::lsp) fn code_actions(
        &self,
        params: CodeActionParams,
    ) -> Result<Json, RequestFailure> {
        // Parameter admission precedes eligibility, including unsupported clients.
        let uri = &params.text_document.uri;
        let Some(document) = self.documents.get(uri) else {
            return Err(invalid("document is not open"));
        };
        let Some(source) = document.text.source() else {
            return Err(failed("document source exceeds the editor input limit"));
        };
        let (Some(start), Some(end)) = (
            self.encoding.offset(source, params.range.start),
            self.encoding.offset(source, params.range.end),
        ) else {
            return Err(invalid("range is not a valid negotiated source boundary"));
        };
        if start > end {
            return Err(invalid("range endpoints are reversed"));
        }
        let empty = || Ok(json!([]));
        if !self.code_actions
            || params.context.only.as_ref().is_some_and(|kinds| {
                !kinds.iter().any(|kind| {
                    let kind = kind.as_str();
                    kind.is_empty()
                        || kind == "refactor.rewrite"
                        || "refactor.rewrite"
                            .strip_prefix(kind)
                            .is_some_and(|tail| tail.starts_with('.'))
                })
            })
        {
            return empty();
        }
        let Some(context) = document.context.as_ref().filter(|context| context.data()) else {
            return empty();
        };
        if let Some(error) = document.text.error()
            && matches!(
                error.kind,
                crate::ErrorKind::LimitExceeded | crate::ErrorKind::InvalidLimits
            )
        {
            return Err(failed(super::display(error)));
        }
        let Some(data) = document.text.data() else {
            return empty();
        };
        if context.expected.is_none() && data.tag(data.root()).is_none() {
            return empty();
        }
        let mut path = String::new();
        let Some(target) = object_at(data, data.root(), start, &mut path)? else {
            return empty();
        };
        let analysis = match self.analysis(uri, &[], &mut Vec::new()) {
            Ok(analysis) => analysis,
            Err(AnalysisFailure::Native(error)) if error.kind == NativeErrorKind::LimitExceeded => {
                return Err(failed(super::display(&error)));
            }
            Err(AnalysisFailure::Message("editor source overlay limit exceeded")) => {
                return Err(failed("editor source overlay limit exceeded"));
            }
            Err(_) => return empty(),
        };
        let schemas = match &analysis.compilation.schemas {
            Ok(schemas) => schemas,
            Err(error) if error.kind == NativeErrorKind::LimitExceeded => {
                return Err(failed(super::display(error)));
            }
            Err(_) => return empty(),
        };
        let expected = context.expected.as_deref();
        let resolved = match schemas.resolve_with_limits(data, expected, Default::default()) {
            Ok(resolved) => resolved,
            Err(error)
                if matches!(
                    error.kind,
                    ResolveErrorKind::LimitExceeded | ResolveErrorKind::NumericLimit
                ) =>
            {
                return Err(failed(super::display(&error)));
            }
            Err(_) => return empty(),
        };
        default_actions(
            self,
            uri,
            document.version,
            data,
            target,
            &path,
            schemas,
            expected,
            resolved.root(),
        )
    }
}

// Parsed node spans include tags and closing braces, but exclude trailing trivia.
// Reuse one bounded pointer buffer along the selected branch, rather than
// retaining a copy of every ancestor's growing path.
fn object_at<'a>(
    data: &Document,
    node: &'a Node,
    offset: usize,
    path: &mut String,
) -> Result<Option<&'a Node>, RequestFailure> {
    if offset < node.span.start || offset >= node.span.end {
        return Ok(None);
    }
    let parent_len = path.len();
    match &node.value {
        Value::Mapping(entries) => {
            for entry in entries {
                if offset >= entry.value.span.start && offset < entry.value.span.end {
                    append_path(path, data.slice(entry.key).expect("parsed key"))?;
                    if let Some(target) = object_at(data, &entry.value, offset, path)? {
                        return Ok(Some(target));
                    }
                    path.truncate(parent_len);
                    break;
                }
            }
            Ok(Some(node))
        }
        Value::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                if offset >= item.span.start && offset < item.span.end {
                    append_path(path, &index.to_string())?;
                    let target = object_at(data, item, offset, path)?;
                    if target.is_none() {
                        path.truncate(parent_len);
                    }
                    return Ok(target);
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn append_path(path: &mut String, segment: &str) -> Result<(), RequestFailure> {
    let size = path
        .len()
        .checked_add(crate::pointer::segment_len(segment))
        .filter(|size| *size <= super::MAX_METADATA_BYTES)
        .ok_or_else(|| failed("code action traversal path limit exceeded"))?;
    path.reserve(size - path.len());
    crate::pointer::append(path, segment);
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "borrowed request snapshot, no retained action state"
)]
fn default_actions(
    session: &Session,
    uri: &lsp_types::Uri,
    version: i32,
    data: &Document,
    target: &Node,
    path: &str,
    schemas: &Schemas,
    expected: Option<&str>,
    root: &ResolvedNode<'_>,
) -> Result<Json, RequestFailure> {
    let Some(completed) = root.at_pointer(path) else {
        return Err(failed("authored object missing from checked result"));
    };
    let ResolvedValue::Mapping(fields) = completed.value() else {
        return Err(failed("authored object changed kind during resolution"));
    };
    // Required-container synthesis uses a schema location, not a declared default
    // location. Check the compiled provenance, never a hint or the Origin variant alone.
    let explicit: HashSet<_> = schemas.explicit_default_origins().collect();
    let Value::Mapping(entries) = &target.value else {
        unreachable!("object selection returns mappings")
    };
    let present: HashSet<_> = entries
        .iter()
        .map(|entry| data.slice(entry.key).expect("parsed key"))
        .collect();
    let mut actions = Vec::new();
    let mut bytes = ResultBytes(2); // JSON array brackets.
    for (name, node) in fields {
        if present.contains(name.as_ref())
            || !matches!(node.origin(), Origin::Default(location) if explicit.contains(location))
        {
            continue;
        }
        let value = SemanticNode::from_resolved(node);
        let field_path = crate::pointer::join(path, name);
        let patch = match schemas.prepare_source_edits(
            data,
            expected,
            &[SourceEdit::Insert {
                path: &field_path,
                value: &value,
            }],
            Default::default(),
            Default::default(),
        ) {
            Ok(patch) => patch,
            Err(SourceEditError::Semantic(error))
                if matches!(
                    error.kind,
                    SemanticErrorKind::InvalidKey
                        | SemanticErrorKind::InvalidTag
                        | SemanticErrorKind::UnrepresentableTag
                        | SemanticErrorKind::UnrepresentableString
                ) =>
            {
                continue;
            }
            Err(error) => return Err(failed(super::display(&error))),
        };
        let edited = patch
            .apply_syntax(data.source())
            .map_err(|error| failed(super::display(&error)))?;
        let checked = schemas
            .resolve_with_limits(&edited, expected, Default::default())
            .map_err(|error| failed(super::display(&error)))?;
        if !same_completed(schemas, root, checked.root()) {
            continue;
        }
        let [change] = patch.changes() else {
            return Err(failed(
                "default insertion did not produce one source change",
            ));
        };
        let action = json!({
            "title": format!("Insert this default: {name}"),
            "kind": "refactor.rewrite",
            "edit": {"documentChanges": [{
                "textDocument": {"uri": uri, "version": version},
                "edits": [{"range": session.encoding.range(data.source(), change.span),
                           "newText": change.replacement}]
            }]}
        });
        if !actions.is_empty() {
            bytes
                .write_all(b",")
                .map_err(|error| failed(error.to_string()))?;
        }
        serde_json::to_writer(&mut bytes, &action)
            .map_err(|_| failed("code action response limit exceeded"))?;
        if actions.len() == MAX_ACTIONS {
            return Err(failed("code action response limit exceeded"));
        }
        actions.push(action);
    }
    Ok(Json::Array(actions))
}

fn same_completed(schemas: &Schemas, left: &ResolvedNode<'_>, right: &ResolvedNode<'_>) -> bool {
    if !crate::json_backend::equal_resolved(left, right, false) {
        return false;
    }
    fn tags(schemas: &Schemas, left: &ResolvedNode<'_>, right: &ResolvedNode<'_>) -> bool {
        let identity = |node: &ResolvedNode<'_>| {
            node.tag_binding()
                .or_else(|| node.tag())
                .and_then(|name| schemas.binding_identity(name))
        };
        if left.tag().is_some() != right.tag().is_some() || identity(left) != identity(right) {
            return false;
        }
        match (left.value(), right.value()) {
            (ResolvedValue::Mapping(a), ResolvedValue::Mapping(b)) => a
                .iter()
                .all(|(key, node)| tags(schemas, node, &b[key.as_ref()])),
            (ResolvedValue::Sequence(a), ResolvedValue::Sequence(b)) => {
                a.iter().zip(b).all(|(a, b)| tags(schemas, a, b))
            }
            _ => true,
        }
    }
    tags(schemas, left, right)
}
