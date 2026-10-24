use crate::{Document, FormatOptions, ResolvedNode, ResolvedValue, names};
use pretty::DocAllocator;
use serde_json::Number;
use std::{collections::BTreeMap, error::Error, fmt, fmt::Write as _};

/// An owned tag assertion. `binding` retains schema identity when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTag {
    /// Tag name without its leading `!`.
    pub name: String,
    /// Stable schema binding identifier when supplied by resolution.
    pub binding: Option<String>,
}

/// An owned, editable semantic value.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticNode {
    /// Optional tag assertion attached to this value.
    pub tag: Option<SemanticTag>,
    /// The owned semantic payload.
    pub value: SemanticValue,
}

/// Supported DAML data values, preserving exact decimal numbers and literal keys.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticValue {
    /// The explicit null value.
    Null,
    /// A boolean value.
    Bool(bool),
    /// An exact decimal number, without floating-point conversion.
    Number(Number),
    /// String data.
    String(String),
    /// Ordered sequence entries.
    Sequence(Vec<SemanticNode>),
    /// Literal mapping keys and their values.
    Mapping(BTreeMap<String, SemanticNode>),
}

impl SemanticNode {
    /// Takes an owned snapshot of parsed data and unqualified source assertions.
    pub fn from_document(document: &Document) -> Self {
        fn convert(document: &Document, node: &crate::Node) -> SemanticNode {
            let tag = document.tag(node).map(|name| SemanticTag {
                name: name.to_owned(),
                binding: None,
            });
            let value = match &node.value {
                crate::Value::Null => SemanticValue::Null,
                crate::Value::Bool(value) => SemanticValue::Bool(*value),
                crate::Value::Number(span) => SemanticValue::Number(
                    document.source()[span.start..span.end]
                        .parse()
                        .expect("parsed document contains a valid number"),
                ),
                crate::Value::String(_) => SemanticValue::String(
                    document
                        .string(node)
                        .expect("parsed string has valid data")
                        .to_owned(),
                ),
                crate::Value::Sequence(items) => SemanticValue::Sequence(
                    items.iter().map(|item| convert(document, item)).collect(),
                ),
                crate::Value::Mapping(entries) => SemanticValue::Mapping(
                    entries
                        .iter()
                        .map(|entry| {
                            (
                                document
                                    .slice(entry.key)
                                    .expect("parsed key has valid span")
                                    .to_owned(),
                                convert(document, &entry.value),
                            )
                        })
                        .collect(),
                ),
            };
            SemanticNode { tag, value }
        }
        convert(document, document.root())
    }

    /// Takes an owned snapshot of completed data, including retained schema bindings.
    pub fn from_resolved(node: &ResolvedNode<'_>) -> Self {
        let tag = node.tag().map(|name| SemanticTag {
            name: name.to_owned(),
            binding: node.tag_binding().map(str::to_owned),
        });
        let value = match node.value() {
            ResolvedValue::Null => SemanticValue::Null,
            ResolvedValue::Bool(value) => SemanticValue::Bool(*value),
            ResolvedValue::Number(value) => SemanticValue::Number(value.as_ref().clone()),
            ResolvedValue::String(value) => SemanticValue::String(value.to_string()),
            ResolvedValue::Sequence(items) => {
                SemanticValue::Sequence(items.iter().map(Self::from_resolved).collect())
            }
            ResolvedValue::Mapping(fields) => SemanticValue::Mapping(
                fields
                    .iter()
                    .map(|(key, value)| (key.to_string(), Self::from_resolved(value)))
                    .collect(),
            ),
        };
        Self { tag, value }
    }
}

/// Stable category of a semantic serialization failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticErrorKind {
    /// A mapping key cannot be written using the unquoted key grammar.
    InvalidKey,
    /// A tag name cannot be written using the tag grammar.
    InvalidTag,
    /// No visible source tag expression preserves the required binding.
    UnrepresentableTag,
    /// A string value cannot be represented in DAML source.
    UnrepresentableString,
    /// The requested formatting width is zero.
    InvalidLineWidth,
    /// A semantic value exceeds a serialization resource bound.
    LimitExceeded,
}

/// A structured semantic serialization error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    /// Stable semantic failure category.
    pub kind: SemanticErrorKind,
    /// RFC 6901 data pointer; empty denotes the root.
    pub pointer: String,
    /// Human-readable explanation of the failure.
    pub message: String,
}

impl fmt::Display for SemanticError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pointer = if self.pointer.is_empty() {
            "<root>"
        } else {
            &self.pointer
        };
        write!(out, "{} at {pointer}", self.message)
    }
}
impl Error for SemanticError {}

const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;
pub(crate) const MAX_SCALAR_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

fn error(kind: SemanticErrorKind, path: &[String], message: impl Into<String>) -> SemanticError {
    SemanticError {
        kind,
        pointer: pointer(path),
        message: message.into(),
    }
}

pub(crate) fn pointer(path: &[String]) -> String {
    path.iter().fold(String::new(), |pointer, segment| {
        crate::pointer::join(&pointer, segment)
    })
}

pub(crate) fn unrepresentable_tag(path: &[String]) -> SemanticError {
    error(
        SemanticErrorKind::UnrepresentableTag,
        path,
        "tag binding has no equivalent visible source name",
    )
}

pub(crate) fn tag_limit_error(path: &[String]) -> SemanticError {
    error(
        SemanticErrorKind::LimitExceeded,
        path,
        "tag exceeds scalar byte bound",
    )
}

/// Writes an unqualified semantic value as canonical DAML source.
/// Qualified tag bindings require a `Schemas` context and are rejected here.
pub fn format_semantic(
    node: &SemanticNode,
    options: FormatOptions,
) -> Result<String, SemanticError> {
    format_with_tags(node, options, |tag, path| {
        if tag.binding.is_some() {
            Err(unrepresentable_tag(path))
        } else {
            if tag.name.len() > MAX_SCALAR_BYTES {
                return Err(error(
                    SemanticErrorKind::LimitExceeded,
                    path,
                    "tag exceeds scalar byte bound",
                ));
            }
            validate_tag(&tag.name, path)?;
            Ok(tag.name.clone())
        }
    })
}

pub(crate) fn format_with_tags(
    node: &SemanticNode,
    options: FormatOptions,
    mut tag_name: impl FnMut(&SemanticTag, &[String]) -> Result<String, SemanticError>,
) -> Result<String, SemanticError> {
    if options.line_width == 0 {
        return Err(error(
            SemanticErrorKind::InvalidLineWidth,
            &[],
            "line width must be positive",
        ));
    }
    let arena = pretty::Arena::new();
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    let doc = build_doc(
        node,
        &arena,
        &mut Vec::new(),
        0,
        &mut nodes,
        &mut bytes,
        &mut tag_name,
    )?;
    let mut output = BoundedOutput::new(bytes.saturating_add(256).min(MAX_OUTPUT_BYTES));
    if doc.render_fmt(options.line_width, &mut output).is_err() || output.write_char('\n').is_err()
    {
        return Err(error(
            SemanticErrorKind::LimitExceeded,
            &[],
            "semantic output exceeds 8 MiB bound",
        ));
    }
    Ok(output.output)
}

struct BoundedOutput {
    output: String,
    limit: usize,
}

impl BoundedOutput {
    fn new(capacity: usize) -> Self {
        Self {
            output: String::with_capacity(capacity),
            limit: MAX_OUTPUT_BYTES,
        }
    }
}

impl fmt::Write for BoundedOutput {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.output.len().saturating_add(value.len()) > self.limit {
            return Err(fmt::Error);
        }
        self.output.push_str(value);
        Ok(())
    }
}

fn validate_tag(name: &str, path: &[String]) -> Result<(), SemanticError> {
    if names::valid_tag(name) {
        Ok(())
    } else {
        Err(error(
            SemanticErrorKind::InvalidTag,
            path,
            format!("tag {name:?} is not representable"),
        ))
    }
}

fn escaped_len(value: &str) -> usize {
    let mut size = 2usize;
    for ch in value.chars() {
        size = size.saturating_add(match ch {
            '"' | '\\' | '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
            '\u{00}'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        });
        if size > MAX_SCALAR_BYTES {
            return size;
        }
    }
    size
}

fn quote(value: &str, path: &[String]) -> Result<String, SemanticError> {
    serde_json::to_string(value).map_err(|_| {
        error(
            SemanticErrorKind::UnrepresentableString,
            path,
            "string cannot be represented in DAML source",
        )
    })
}

fn build_doc<'a>(
    node: &'a SemanticNode,
    arena: &'a pretty::Arena<'a>,
    path: &mut Vec<String>,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
    tag_name: &mut impl FnMut(&SemanticTag, &[String]) -> Result<String, SemanticError>,
) -> Result<pretty::DocBuilder<'a, pretty::Arena<'a>>, SemanticError> {
    *nodes = nodes.saturating_add(1);
    *bytes = bytes.saturating_add(2usize.saturating_mul(depth).saturating_add(4));
    if *nodes > MAX_NODES {
        return Err(error(
            SemanticErrorKind::LimitExceeded,
            path,
            "semantic value exceeds node bound",
        ));
    }
    if depth >= MAX_DEPTH
        && matches!(
            &node.value,
            SemanticValue::Sequence(_) | SemanticValue::Mapping(_)
        )
    {
        return Err(error(
            SemanticErrorKind::LimitExceeded,
            path,
            "semantic value exceeds depth bound",
        ));
    }
    let tag = if let Some(tag) = &node.tag {
        let name = tag_name(tag, path)?;
        if name.len() > MAX_SCALAR_BYTES {
            return Err(error(
                SemanticErrorKind::LimitExceeded,
                path,
                "tag exceeds scalar byte bound",
            ));
        }
        validate_tag(&name, path)?;
        *bytes = bytes.saturating_add(name.len() + 2);
        Some(name)
    } else {
        None
    };
    let value = match &node.value {
        SemanticValue::Null => {
            *bytes = bytes.saturating_add(4);
            arena.text("null")
        }
        SemanticValue::Bool(value) => {
            *bytes = bytes.saturating_add(if *value { 4 } else { 5 });
            arena.text(if *value { "true" } else { "false" })
        }
        SemanticValue::Number(value) => {
            let raw = value.as_str();
            if raw.len() > MAX_SCALAR_BYTES {
                return Err(error(
                    SemanticErrorKind::LimitExceeded,
                    path,
                    "number exceeds scalar byte bound",
                ));
            }
            *bytes = bytes.saturating_add(raw.len());
            arena.text(raw)
        }
        SemanticValue::String(value) => {
            let encoded_len = escaped_len(value);
            if encoded_len > MAX_SCALAR_BYTES {
                return Err(error(
                    SemanticErrorKind::LimitExceeded,
                    path,
                    "encoded string exceeds scalar byte bound",
                ));
            }
            *bytes = bytes.saturating_add(encoded_len);
            arena.text(quote(value, path)?)
        }
        SemanticValue::Sequence(items) => {
            if items.len() > MAX_NODES.saturating_sub(*nodes) {
                return Err(error(
                    SemanticErrorKind::LimitExceeded,
                    path,
                    "semantic value exceeds node bound",
                ));
            }
            *bytes = bytes.saturating_add(items.len().saturating_add(1));
            let mut docs = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                let doc = build_doc(item, arena, path, depth + 1, nodes, bytes, tag_name);
                path.pop();
                docs.push(doc?);
            }
            collection(arena, "[", "]", docs, false)
        }
        SemanticValue::Mapping(fields) => {
            if fields.len() > MAX_NODES.saturating_sub(*nodes) {
                return Err(error(
                    SemanticErrorKind::LimitExceeded,
                    path,
                    "semantic value exceeds node bound",
                ));
            }
            *bytes = bytes.saturating_add(fields.len().saturating_add(1));
            let mut docs = Vec::with_capacity(fields.len());
            for (key, child) in fields {
                if key.len() > MAX_SCALAR_BYTES {
                    return Err(error(
                        SemanticErrorKind::LimitExceeded,
                        path,
                        "mapping key exceeds scalar byte bound",
                    ));
                }
                if !names::valid_key(key) {
                    path.push(key.clone());
                    let failure = error(
                        SemanticErrorKind::InvalidKey,
                        path,
                        "mapping key is not representable",
                    );
                    path.pop();
                    return Err(failure);
                }
                *bytes = bytes.saturating_add(key.len() + 1);
                if *bytes > MAX_OUTPUT_BYTES {
                    return Err(error(
                        SemanticErrorKind::LimitExceeded,
                        path,
                        "semantic output exceeds 8 MiB bound",
                    ));
                }
                path.push(key.clone());
                let child_doc = build_doc(child, arena, path, depth + 1, nodes, bytes, tag_name);
                path.pop();
                let child_doc = child_doc?;
                docs.push(
                    arena
                        .text(key.as_str())
                        .append(":")
                        .append(arena.line().append(child_doc).nest(2))
                        .group(),
                );
            }
            collection(arena, "{", "}", docs, true)
        }
    };
    if *bytes > MAX_OUTPUT_BYTES {
        return Err(error(
            SemanticErrorKind::LimitExceeded,
            path,
            "semantic output exceeds 8 MiB bound",
        ));
    }
    if let Some(name) = tag {
        Ok(arena
            .text("!")
            .append(name)
            .append(arena.space())
            .append(value))
    } else {
        Ok(value)
    }
}

fn collection<'a>(
    arena: &'a pretty::Arena<'a>,
    open: &'static str,
    close: &'static str,
    items: Vec<pretty::DocBuilder<'a, pretty::Arena<'a>>>,
    mapping: bool,
) -> pretty::DocBuilder<'a, pretty::Arena<'a>> {
    let count = items.len();
    if count == 0 {
        return arena.text(open).append(close);
    }
    let border = if count > 1 {
        arena.hardline()
    } else if mapping {
        arena.line()
    } else {
        arena.line_()
    };
    let mut inside = arena.nil();
    for (index, item) in items.into_iter().enumerate() {
        inside = inside.append(if index == 0 {
            border.clone()
        } else {
            arena.hardline()
        });
        inside = inside.append(item);
    }
    arena
        .text(open)
        .append(inside.nest(2))
        .append(border)
        .append(close)
        .group()
}
