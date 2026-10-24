use crate::{Document, Span, pointer};
use serde_json::Number;
use std::{borrow::Cow, collections::BTreeMap, error::Error, fmt};

/// A JSON Schema resource identifier and RFC 6901 pointer within it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SchemaLocation {
    /// Explicit in-memory resource URI; never fetched automatically.
    pub resource: String,
    /// Pointer to the schema/default, with an empty string denoting its root.
    pub pointer: String,
}

/// Where a completed value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin<'a> {
    /// Bytes in the original supplied DAML document.
    Input(Span),
    /// Declared schema default; no input byte span is fabricated for it.
    Default(&'a SchemaLocation),
}

/// A scalar or container after default completion.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedValue<'a> {
    /// Explicit null, which never means an omitted value.
    Null,
    /// Boolean data without coercion.
    Bool(bool),
    /// A bounded arbitrary-precision decimal value.
    Number(Cow<'a, Number>),
    /// Text borrowed from supplied input or schema defaults when possible.
    String(Cow<'a, str>),
    /// Ordered array entries.
    Sequence(Vec<ResolvedNode<'a>>),
    /// Literal object keys, without implicit dotted/slashed traversal.
    Mapping(BTreeMap<Cow<'a, str>, ResolvedNode<'a>>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedTag<'a> {
    pub(crate) name: Cow<'a, str>,
    pub(crate) binding: Option<Cow<'a, str>>,
    pub(crate) origin: Origin<'a>,
}

/// A completed value with retained tag and origin metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode<'a> {
    pub(crate) value: ResolvedValue<'a>,
    pub(crate) tag: Option<ResolvedTag<'a>>,
    pub(crate) origin: Origin<'a>,
}

impl<'a> ResolvedNode<'a> {
    /// Returns the completed value without permitting mutation of checked data.
    pub fn value(&self) -> &ResolvedValue<'a> {
        &self.value
    }

    /// Returns the literal asserted tag, without `!`.
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_ref().map(|tag| tag.name.as_ref())
    }
    /// Returns the stable schema binding identifier for an asserted tag, when known.
    pub fn tag_binding(&self) -> Option<&str> {
        self.tag.as_ref()?.binding.as_deref()
    }

    /// Returns the input/default provenance of the asserted tag, when present.
    pub fn tag_origin(&self) -> Option<Origin<'a>> {
        self.tag.as_ref().map(|tag| tag.origin)
    }

    /// Returns supplied-input or declared-default provenance.
    pub fn origin(&self) -> Origin<'a> {
        self.origin
    }

    /// Finds a literal field name in an object.
    pub fn get(&self, key: &str) -> Option<&Self> {
        match &self.value {
            ResolvedValue::Mapping(fields) => fields.get(key),
            _ => None,
        }
    }

    /// Returns array entries, or `None` for another value kind.
    pub fn items(&self) -> Option<&[Self]> {
        match &self.value {
            ResolvedValue::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// Returns string data, or `None` for another value kind.
    pub fn string(&self) -> Option<&str> {
        match &self.value {
            ResolvedValue::String(value) => Some(value),
            _ => None,
        }
    }

    /// Exports plain JSON-compatible data, deliberately excluding tag metadata.
    ///
    /// This allocates an owned output; inspect borrowed values through the other
    /// accessors when serialization is not needed.
    pub fn to_json(&self) -> serde_json::Value {
        match &self.value {
            ResolvedValue::Null => serde_json::Value::Null,
            ResolvedValue::Bool(value) => serde_json::Value::Bool(*value),
            ResolvedValue::Number(value) => serde_json::Value::Number(value.as_ref().clone()),
            ResolvedValue::String(value) => serde_json::Value::String(value.to_string()),
            ResolvedValue::Sequence(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            ResolvedValue::Mapping(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_json()))
                    .collect(),
            ),
        }
    }

    pub(crate) fn at_pointer(&self, pointer: &str) -> Option<&Self> {
        if pointer.is_empty() {
            return Some(self);
        }
        let mut node = self;
        for part in pointer.strip_prefix('/')?.split('/') {
            let key = pointer::decode_segment(part)?;
            node = match &node.value {
                ResolvedValue::Mapping(fields) => fields.get(key.as_ref())?,
                ResolvedValue::Sequence(items) => items.get(part.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(node)
    }
}

/// Completed and schema-checked data, borrowing its input and schema registry.
#[derive(Debug)]
pub struct ResolvedDocument<'a> {
    pub(crate) document: &'a Document,
    pub(crate) root: ResolvedNode<'a>,
}

impl<'a> ResolvedDocument<'a> {
    /// Returns the checked root, including completed defaults and tag metadata.
    pub fn root(&self) -> &ResolvedNode<'a> {
        &self.root
    }

    /// Returns the original, unmodified supplied source.
    pub fn source(&self) -> &str {
        self.document.source()
    }

    /// Exports the completed data without tag metadata.
    pub fn to_json(&self) -> serde_json::Value {
        self.root.to_json()
    }
}

/// Bounds for completion and validation work, including candidate union branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionLimits {
    /// Maximum materialized value nodes across the resolution work.
    pub max_nodes: usize,
    /// Maximum materialized key/string/number bytes across the resolution work.
    pub max_bytes: usize,
    /// Maximum schema/node processing steps.
    pub max_steps: usize,
}

impl Default for ResolutionLimits {
    fn default() -> Self {
        Self {
            max_nodes: 100_000,
            max_bytes: 8 * 1024 * 1024,
            max_steps: 1_000_000,
        }
    }
}

/// A machine-readable resolution/validation failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveErrorKind {
    /// A supplied/defaulted tag has no explicitly composed binding.
    UnknownTag,
    /// A requested root schema was not composed.
    UnknownExpectedType,
    /// Completed data violates a schema constraint.
    SchemaViolation,
    /// Applicable defaults lead to more than one different completed value.
    AmbiguousDefault,
    /// A resource budget was exhausted.
    LimitExceeded,
    /// A number exceeds the documented exact-validation digit/exponent bounds.
    NumericLimit,
}

/// Owned provenance suitable for retaining an error after input/registry drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorOrigin {
    /// Original supplied-source bytes.
    Input(Span),
    /// A schema default rather than a fabricated supplied-source position.
    Default(SchemaLocation),
}

impl From<Origin<'_>> for ErrorOrigin {
    fn from(origin: Origin<'_>) -> Self {
        match origin {
            Origin::Input(span) => Self::Input(span),
            Origin::Default(location) => Self::Default(location.clone()),
        }
    }
}

/// A structured failure in completed data, distinct from syntax/schema errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    /// Failure category, without parsing a human message.
    pub kind: ResolveErrorKind,
    /// RFC 6901 instance pointer; empty denotes the root.
    pub path: String,
    /// Actual supplied/default origin of the failing value.
    pub origin: ErrorOrigin,
    /// Schema keyword location when available.
    pub schema: Option<Box<SchemaLocation>>,
    /// Human-readable context, not an execution recipe.
    pub message: String,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = if self.path.is_empty() {
            "<root>"
        } else {
            &self.path
        };
        write!(out, "{} at {path}", self.message)?;
        match &self.origin {
            ErrorOrigin::Input(span) => write!(out, " (bytes {}..{})", span.start, span.end),
            ErrorOrigin::Default(location) => {
                write!(out, " (default {}#{})", location.resource, location.pointer)
            }
        }
    }
}

impl Error for ResolveError {}

pub(crate) fn checked_number(raw: &str) -> Result<Number, &'static str> {
    if raw.bytes().filter(u8::is_ascii_digit).count() > 512 {
        return Err("number exceeds the exact-validation limit of 512 digits");
    }
    if let Some((_, exponent)) = raw.split_once(['e', 'E']) {
        let unsigned = exponent.trim_start_matches(['+', '-']);
        if unsigned.parse::<u16>().map_or(true, |value| value > 512) {
            return Err("number exponent exceeds the exact-validation bound of 512");
        }
    }
    raw.parse().map_err(|_| "invalid decimal number")
}
